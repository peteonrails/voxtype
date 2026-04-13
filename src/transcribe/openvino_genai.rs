//! OpenVINO GenAI speech-to-text transcription
//!
//! Uses Intel OpenVINO GenAI's WhisperPipeline for hardware-accelerated
//! transcription on CPU, GPU, or NPU (Intel AI Boost).
//!
//! Links directly to libopenvino_genai_c.so via FFI. The WhisperPipeline
//! runs the entire Whisper model (encoder + decoder + tokenizer) through
//! OpenVINO, enabling full NPU/GPU offload.

use super::Transcriber;
use crate::config::OpenvinoGenaiConfig;
use crate::error::TranscribeError;
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::ptr;

// ============================================================================
// FFI bindings to OpenVINO GenAI C API (libopenvino_genai_c.so)
// ============================================================================

#[allow(non_camel_case_types)]
type ov_status_e = i32;
const OV_STATUS_OK: ov_status_e = 0;

#[repr(C)]
struct ov_genai_whisper_pipeline_opaque {
    _private: [u8; 0],
}

#[repr(C)]
struct ov_genai_whisper_decoded_results_opaque {
    _private: [u8; 0],
}

#[repr(C)]
struct ov_genai_whisper_generation_config_opaque {
    _private: [u8; 0],
}

type WhisperPipelinePtr = *mut ov_genai_whisper_pipeline_opaque;
type DecodedResultsPtr = *mut ov_genai_whisper_decoded_results_opaque;
type GenerationConfigPtr = *mut ov_genai_whisper_generation_config_opaque;

#[link(name = "openvino_genai_c")]
extern "C" {
    fn ov_genai_whisper_pipeline_create(
        models_path: *const i8,
        device: *const i8,
        property_args_size: usize,
        pipeline: *mut WhisperPipelinePtr,
        ...
    ) -> ov_status_e;

    fn ov_genai_whisper_pipeline_free(pipeline: WhisperPipelinePtr);

    fn ov_genai_whisper_pipeline_generate(
        pipeline: WhisperPipelinePtr,
        raw_speech: *const f32,
        raw_speech_size: usize,
        config: GenerationConfigPtr,
        results: *mut DecodedResultsPtr,
    ) -> ov_status_e;

    fn ov_genai_whisper_pipeline_get_generation_config(
        pipeline: WhisperPipelinePtr,
        config: *mut GenerationConfigPtr,
    ) -> ov_status_e;

    fn ov_genai_whisper_generation_config_free(config: GenerationConfigPtr);

    fn ov_genai_whisper_generation_config_set_language(
        config: GenerationConfigPtr,
        language: *const i8,
    ) -> ov_status_e;

    fn ov_genai_whisper_generation_config_set_task(
        config: GenerationConfigPtr,
        task: *const i8,
    ) -> ov_status_e;

    fn ov_genai_whisper_decoded_results_get_texts_count(
        results: DecodedResultsPtr,
        count: *mut usize,
    ) -> ov_status_e;

    fn ov_genai_whisper_decoded_results_get_text_at(
        results: DecodedResultsPtr,
        index: usize,
        text: *mut i8,
        text_size: *mut usize,
    ) -> ov_status_e;

    fn ov_genai_whisper_decoded_results_free(results: DecodedResultsPtr);
}

// ============================================================================
// Safe Rust wrapper
// ============================================================================

struct Pipeline {
    ptr: WhisperPipelinePtr,
}

impl Pipeline {
    fn new(model_path: &str, device: &str) -> Result<Self, TranscribeError> {
        let model_c = CString::new(model_path)
            .map_err(|e| TranscribeError::InitFailed(format!("Invalid model path: {}", e)))?;
        let device_c = CString::new(device)
            .map_err(|e| TranscribeError::InitFailed(format!("Invalid device: {}", e)))?;

        let mut ptr: WhisperPipelinePtr = ptr::null_mut();
        let status = unsafe {
            ov_genai_whisper_pipeline_create(
                model_c.as_ptr(),
                device_c.as_ptr(),
                0,
                &mut ptr,
            )
        };

        if status != OV_STATUS_OK || ptr.is_null() {
            return Err(TranscribeError::InitFailed(format!(
                "Failed to create WhisperPipeline on {} (status={})",
                device, status
            )));
        }

        Ok(Self { ptr })
    }

    fn generate(
        &self,
        samples: &[f32],
        language: Option<&str>,
        task: &str,
    ) -> Result<String, TranscribeError> {
        // Get generation config
        let mut config_ptr: GenerationConfigPtr = ptr::null_mut();
        let status = unsafe {
            ov_genai_whisper_pipeline_get_generation_config(self.ptr, &mut config_ptr)
        };
        if status != OV_STATUS_OK || config_ptr.is_null() {
            return Err(TranscribeError::InferenceFailed(
                "Failed to get generation config".to_string(),
            ));
        }

        // Set language
        if let Some(lang) = language {
            let lang_token = if lang.starts_with("<|") {
                lang.to_string()
            } else {
                format!("<|{}|>", lang)
            };
            let lang_c = CString::new(lang_token).unwrap();
            unsafe {
                ov_genai_whisper_generation_config_set_language(config_ptr, lang_c.as_ptr());
            }
        }

        // Set task
        let task_c = CString::new(task).unwrap();
        unsafe {
            ov_genai_whisper_generation_config_set_task(config_ptr, task_c.as_ptr());
        }

        // Run inference
        let mut results_ptr: DecodedResultsPtr = ptr::null_mut();
        let status = unsafe {
            ov_genai_whisper_pipeline_generate(
                self.ptr,
                samples.as_ptr(),
                samples.len(),
                config_ptr,
                &mut results_ptr,
            )
        };

        unsafe {
            ov_genai_whisper_generation_config_free(config_ptr);
        }

        if status != OV_STATUS_OK || results_ptr.is_null() {
            return Err(TranscribeError::InferenceFailed(format!(
                "WhisperPipeline generate failed (status={})",
                status
            )));
        }

        // Get text count
        let mut count: usize = 0;
        unsafe {
            ov_genai_whisper_decoded_results_get_texts_count(results_ptr, &mut count);
        }

        let mut text = String::new();
        for i in 0..count {
            // Two-call pattern: get size, then get text
            let mut size: usize = 0;
            unsafe {
                ov_genai_whisper_decoded_results_get_text_at(
                    results_ptr, i, ptr::null_mut(), &mut size,
                );
            }
            let mut buf = vec![0u8; size + 1];
            unsafe {
                ov_genai_whisper_decoded_results_get_text_at(
                    results_ptr, i, buf.as_mut_ptr() as *mut i8, &mut size,
                );
            }
            if let Ok(s) = unsafe { CStr::from_ptr(buf.as_ptr() as *const i8) }.to_str() {
                text.push_str(s);
            }
        }

        unsafe {
            ov_genai_whisper_decoded_results_free(results_ptr);
        }

        Ok(text.trim().to_string())
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ov_genai_whisper_pipeline_free(self.ptr);
            }
        }
    }
}

unsafe impl Send for Pipeline {}
unsafe impl Sync for Pipeline {}

// ============================================================================
// Transcriber implementation
// ============================================================================

pub struct OpenvinoGenaiTranscriber {
    pipeline: Pipeline,
    language: Option<String>,
    translate: bool,
}

impl OpenvinoGenaiTranscriber {
    pub fn new(config: &OpenvinoGenaiConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;
        let device = std::env::var("OPENVINO_DEVICE")
            .ok()
            .or_else(|| config.device.clone())
            .unwrap_or_else(|| "CPU".to_string());
        let device = device.as_str();

        tracing::info!(
            "Loading OpenVINO GenAI WhisperPipeline from {:?} on {}",
            model_path, device
        );
        let start = std::time::Instant::now();

        let pipeline = Pipeline::new(
            model_path.to_str()
                .ok_or_else(|| TranscribeError::ModelNotFound("Invalid path".to_string()))?,
            device,
        )?;

        tracing::info!(
            "WhisperPipeline loaded in {:.2}s on {}",
            start.elapsed().as_secs_f32(), device
        );

        Ok(Self {
            pipeline,
            language: config.language.clone(),
            translate: config.translate.unwrap_or(false),
        })
    }
}

impl Transcriber for OpenvinoGenaiTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let task = if self.translate { "translate" } else { "transcribe" };
        self.pipeline.generate(samples, self.language.as_deref(), task)
    }
}

fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }
    if let Some(home) = std::env::var("HOME").ok().map(PathBuf::from) {
        let candidate = home.join(".local/share/voxtype/models").join(model);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    if path.exists() {
        return Ok(path);
    }
    Err(TranscribeError::ModelNotFound(format!(
        "OpenVINO model not found: {}. Export with: \
         optimum-cli export openvino --model openai/whisper-base ~/.local/share/voxtype/models/{}",
        model, model
    )))
}
