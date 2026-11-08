//! SenseVoice via the FunASR llama.cpp / ggml runtime
//!
//! Runs `llama-funasr-sensevoice` as a one-shot child process per
//! transcription. With `--backend vulkan` the ggml tensors are uploaded to the
//! GPU, so the weights live in VRAM for the duration of inference and never in
//! the daemon's heap: the child mmaps the GGUF, uploads it, transcribes, exits,
//! and both the host mapping and the VRAM are released with it.
//!
//! Why a child process instead of an in-process backend: voxtype links ggml
//! only through whisper-rs (whisper.cpp), which cannot execute SenseVoice's
//! SAN-M encoder. The FunASR runtime is the implementation of that
//! architecture, and the process boundary is exactly what keeps the model out
//! of the daemon's address space.
//!
//! Runtime contract, verified against FunASR `runtime-llamacpp-v0.2.6`:
//!
//! ```text
//! llama-funasr-sensevoice -m model.gguf -a audio.wav
//!     [--vad fsmn-vad.gguf] [--backend cpu|cuda|vulkan]
//! ```
//!
//! Transcription text goes to stdout, progress and diagnostics to stderr.
//! Without `--keep-tags` the runtime strips the language/emotion/event tags
//! itself, and inverse text normalization is always applied.

use super::Transcriber;
use crate::config::{GgmlBackend, SenseVoiceConfig};
use crate::error::TranscribeError;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Sample rate the runtime expects in its WAV input (mono, PCM16).
const SAMPLE_RATE: u32 = 16000;

/// How much of the runtime's stderr to keep in an error message.
const STDERR_TAIL_CHARS: usize = 400;

/// SenseVoice transcriber driven by the FunASR ggml runtime.
#[derive(Debug)]
pub struct SenseVoiceGgmlTranscriber {
    binary: PathBuf,
    model: PathBuf,
    vad: Option<PathBuf>,
    backend: GgmlBackend,
}

impl SenseVoiceGgmlTranscriber {
    /// Validate the configured paths and build the transcriber.
    ///
    /// Construction is cheap by design: all the expensive work (weight upload,
    /// inference) happens in the child process. This only checks that the
    /// files exist and reports settings the ggml runtime cannot honour.
    pub fn new(config: &SenseVoiceConfig) -> Result<Self, TranscribeError> {
        let binary = config.ggml_binary.clone().ok_or_else(|| {
            TranscribeError::InitFailed(
                "sensevoice.runtime = \"ggml\" requires sensevoice.ggml_binary.\n  \
                 Fetch llama-funasr-sensevoice from the FunASR runtime release:\n  \
                 https://github.com/modelscope/FunASR/releases/tag/runtime-llamacpp-v0.2.6"
                    .to_string(),
            )
        })?;
        if !binary.is_file() {
            return Err(TranscribeError::InitFailed(format!(
                "sensevoice.ggml_binary is not a file: {}",
                binary.display()
            )));
        }

        let model = config.ggml_model.clone().ok_or_else(|| {
            TranscribeError::InitFailed(
                "sensevoice.runtime = \"ggml\" requires sensevoice.ggml_model.\n  \
                 Fetch a SenseVoiceSmall GGUF (sensevoice-small-q8.gguf or\n  \
                 sensevoice-small-f16.gguf) from FunAudioLLM/SenseVoiceSmall-GGUF."
                    .to_string(),
            )
        })?;
        if !model.is_file() {
            return Err(TranscribeError::ModelNotFound(format!(
                "SenseVoice GGUF not found: {}",
                model.display()
            )));
        }

        if let Some(vad) = &config.ggml_vad {
            if !vad.is_file() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "FSMN-VAD GGUF not found: {}",
                    vad.display()
                )));
            }
        }

        // The ggml runtime always auto-detects the language and always applies
        // ITN. Say so once at construction rather than silently ignoring the
        // settings on every dictation.
        if config.language != "auto" {
            tracing::warn!(
                "sensevoice.language = {:?} is ignored by the ggml runtime, which always \
                 auto-detects; use runtime = \"onnx\" to force a language",
                config.language
            );
        }
        if !config.use_itn {
            tracing::warn!(
                "sensevoice.use_itn = false is ignored by the ggml runtime (ITN is always applied)"
            );
        }

        tracing::debug!(
            "SenseVoice ggml runtime: binary={} model={} backend={}",
            binary.display(),
            model.display(),
            config.ggml_backend.as_arg()
        );

        Ok(Self {
            binary,
            model,
            vad: config.ggml_vad.clone(),
            backend: config.ggml_backend,
        })
    }

    /// Command line for one transcription. Separate from `transcribe` so the
    /// argument contract can be asserted without running the runtime.
    fn build_command(&self, wav: &Path) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-a")
            .arg(wav)
            .stdin(Stdio::null());
        if let Some(vad) = &self.vad {
            cmd.arg("--vad").arg(vad);
        }
        cmd.arg("--backend").arg(self.backend.as_arg());
        cmd
    }

    /// Write 16 kHz mono samples as a PCM16 WAV for the runtime to read.
    fn write_temp_wav(&self, samples: &[f32]) -> Result<tempfile::NamedTempFile, TranscribeError> {
        let temp_file = tempfile::Builder::new()
            .prefix("voxtype_")
            .suffix(".wav")
            .tempfile()
            .map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to create temp file: {}", e))
            })?;

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(temp_file.path(), spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {}", e))
        })?;

        for &sample in samples {
            let scaled = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            writer.write_sample(scaled).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {}", e))
            })?;
        }

        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {}", e)))?;

        Ok(temp_file)
    }
}

impl Transcriber for SenseVoiceGgmlTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with SenseVoice ggml ({})",
            duration_secs,
            samples.len(),
            self.backend.as_arg()
        );

        let start = std::time::Instant::now();
        let temp_wav = self.write_temp_wav(samples)?;

        let output = self
            .build_command(temp_wav.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to run {}: {}",
                    self.binary.display(),
                    e
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TranscribeError::InferenceFailed(format!(
                "llama-funasr-sensevoice failed ({}):\n  {}",
                output.status,
                stderr_tail(&stderr)
            )));
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();

        tracing::info!(
            "SenseVoice (ggml/{}) transcription completed in {:.2}s: {:?}",
            self.backend.as_arg(),
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );

        Ok(text)
    }
}

/// Keep the end of the runtime's diagnostics: its stage lines fail in order, so
/// the last completed boundary identifies what went wrong.
fn stderr_tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    let count = trimmed.chars().count();
    if count <= STDERR_TAIL_CHARS {
        return trimmed.to_string();
    }
    let tail: String = trimmed.chars().skip(count - STDERR_TAIL_CHARS).collect();
    format!("...{}", tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config whose paths point at real (empty) files, so construction only
    /// exercises validation, not the runtime.
    fn config_in(dir: &Path, with_vad: bool) -> SenseVoiceConfig {
        let binary = dir.join("llama-funasr-sensevoice");
        let model = dir.join("sensevoice-small-q8.gguf");
        std::fs::write(&binary, b"fake").unwrap();
        std::fs::write(&model, b"fake").unwrap();

        let mut config = SenseVoiceConfig {
            ggml_binary: Some(binary),
            ggml_model: Some(model),
            ..Default::default()
        };
        if with_vad {
            let vad = dir.join("fsmn-vad.gguf");
            std::fs::write(&vad, b"fake").unwrap();
            config.ggml_vad = Some(vad);
        }
        config
    }

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn missing_binary_is_rejected_with_the_release_url() {
        let config = SenseVoiceConfig::default();
        let err = SenseVoiceGgmlTranscriber::new(&config).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("sensevoice.ggml_binary"), "{message}");
    }

    #[test]
    fn missing_model_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config_in(dir.path(), false);
        config.ggml_model = Some(dir.path().join("absent.gguf"));
        let err = SenseVoiceGgmlTranscriber::new(&config).unwrap_err();
        assert!(err.to_string().contains("SenseVoice GGUF not found"));
    }

    #[test]
    fn missing_vad_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config_in(dir.path(), false);
        config.ggml_vad = Some(dir.path().join("absent-vad.gguf"));
        let err = SenseVoiceGgmlTranscriber::new(&config).unwrap_err();
        assert!(err.to_string().contains("FSMN-VAD GGUF not found"));
    }

    #[test]
    fn command_line_matches_the_runtime_contract() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), true);
        let expected_model = config.ggml_model.clone().unwrap();
        let expected_vad = config.ggml_vad.clone().unwrap();

        let transcriber = SenseVoiceGgmlTranscriber::new(&config).unwrap();
        let cmd = transcriber.build_command(Path::new("/tmp/audio.wav"));

        assert_eq!(
            args_of(&cmd),
            vec![
                "-m",
                expected_model.to_str().unwrap(),
                "-a",
                "/tmp/audio.wav",
                "--vad",
                expected_vad.to_str().unwrap(),
                "--backend",
                "vulkan",
            ]
        );
    }

    #[test]
    fn command_line_omits_vad_when_unset_and_honours_the_backend() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config_in(dir.path(), false);
        config.ggml_backend = GgmlBackend::Cpu;

        let transcriber = SenseVoiceGgmlTranscriber::new(&config).unwrap();
        let args = args_of(&transcriber.build_command(Path::new("/tmp/audio.wav")));

        assert!(!args.iter().any(|a| a == "--vad"));
        assert_eq!(args.last().unwrap(), "cpu");
    }

    #[test]
    fn stderr_tail_keeps_the_end_of_the_diagnostics() {
        let short = "vulkan backend ready on Vulkan0";
        assert_eq!(stderr_tail(short), short);

        let long = format!("{}END", "x".repeat(STDERR_TAIL_CHARS * 2));
        let tail = stderr_tail(&long);
        assert!(tail.starts_with("..."));
        assert!(tail.ends_with("END"));
        assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS + 3);
    }
}
