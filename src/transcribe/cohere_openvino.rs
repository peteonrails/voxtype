//! Experimental native OpenVINO encoder for Cohere on Intel GPUs.
//!
//! Keep the original ONNX weights and the ONNX Runtime CPU decoder. Native
//! OpenVINO is used deliberately: the tested ORT OpenVINO EP bundled an older
//! GPU plugin that aborted during dynamic kernel compilation on Arc B390.

use crate::error::TranscribeError;
use openvino::{
    CompiledModel, Core, DeviceType, ElementType, InferRequest, RwPropertyKey, Shape, Tensor,
};
use ort::value::{DynValue, Tensor as OrtTensor};
use std::path::Path;
use std::sync::Mutex;

// Reuse the runtime/device context across model reloads. Creating a fresh Core
// each time grew Arc B390 DRM allocations by ~26 MiB/reload in OpenVINO 2026.4;
// reusing one Core avoided that growth in the native runtime reproducer.
// The Core must outlive compiled models. Its runtime caches live until exit,
// so dropping a model is not a promise of complete GPU memory reclamation.
static CORE: Mutex<Option<Core>> = Mutex::new(None);

pub(super) struct OpenVinoEncoder {
    // Drop requests before their compiled model; the shared Core outlives both.
    request: InferRequest,
    _compiled: CompiledModel,
    reported_inference: bool,
}

impl OpenVinoEncoder {
    pub(super) fn new(path: &Path) -> Result<Self, TranscribeError> {
        let init_error = |e: String| {
            TranscribeError::InitFailed(format!(
                "Cohere OpenVINO GPU encoder: {e}\n  \
                 Install a recent OpenVINO runtime (validated with 2026.4) and Intel compute drivers. \
                 Add the runtime library directory to LD_LIBRARY_PATH, or select \
                 cohere.encoder_backend = \"onnx\" to use ONNX Runtime."
            ))
        };
        // Never fall back to a shared /tmp cache: model caches belong to the user.
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| {
                init_error("cannot locate a user cache directory; set XDG_CACHE_HOME".into())
            })?
            .join("voxtype")
            .join("cohere-openvino");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| init_error(format!("cache {}: {e}", cache_dir.display())))?;
        let cache_path = cache_dir
            .to_str()
            .ok_or_else(|| init_error("cache path is not UTF-8".into()))?;
        // Serialize initialization/compilation, not inference. A failed first
        // initialization can be retried; a binding panic poisons the lock and
        // requires restarting the process rather than reusing uncertain state.
        let mut core_guard = CORE.lock().map_err(|_| {
            init_error("runtime initialization lock poisoned; restart voxtype".into())
        })?;
        if core_guard.is_none() {
            *core_guard = Some(Core::new().map_err(|e| init_error(e.to_string()))?);
        }
        let core = core_guard
            .as_mut()
            .ok_or_else(|| init_error("runtime initialization failed".into()))?;
        let device = DeviceType::GPU;
        core.set_property(&device, &RwPropertyKey::CacheDir, cache_path)
            .map_err(|e| init_error(e.to_string()))?;
        core.set_property(&device, &RwPropertyKey::HintPerformanceMode, "LATENCY")
            .map_err(|e| init_error(e.to_string()))?;
        // LATENCY controls scheduling, not precision. Prefer the original graph's
        // precision over PERFORMANCE mode's accuracy-reducing optimizations.
        core.set_property(&device, &RwPropertyKey::HintExecutionMode, "ACCURACY")
            .map_err(|e| init_error(e.to_string()))?;
        let path = path
            .to_str()
            .ok_or_else(|| init_error("model path is not UTF-8".into()))?;
        // Empty weights path lets the ONNX frontend resolve external-data files
        // relative to the ONNX graph, without converting or copying the model.
        let model = core
            .read_model_from_file(path, "")
            .map_err(|e| init_error(e.to_string()))?;
        // GPU, not AUTO/HETERO: never report a CPU fallback as acceleration.
        let mut compiled = core
            .compile_model(&model, device)
            .map_err(|e| init_error(e.to_string()))?;
        let request = compiled
            .create_infer_request()
            .map_err(|e| init_error(e.to_string()))?;
        // Avoid property getters in openvino-rs 0.11: they leak the C API's
        // allocated strings. version() uses the matching native free function.
        tracing::info!(
            runtime_version = %openvino::version().build_number,
            execution_mode = "ACCURACY",
            "Cohere OpenVINO GPU encoder ready; decoder remains on CPU"
        );
        Ok(Self {
            request,
            _compiled: compiled,
            reported_inference: false,
        })
    }

    pub(super) fn run(
        &mut self,
        frames: usize,
        features: &[f32],
    ) -> Result<DynValue, TranscribeError> {
        let fail =
            |e: String| TranscribeError::InferenceFailed(format!("Cohere OpenVINO encoder: {e}"));
        let frame_dim = validate_input_shape(frames, features.len()).map_err(fail)?;
        let shape = Shape::new(&[1, frame_dim, 128]).map_err(|e| fail(e.to_string()))?;
        let mut input = Tensor::new(ElementType::F32, &shape).map_err(|e| fail(e.to_string()))?;
        let data = input
            .get_data_mut::<f32>()
            .map_err(|e| fail(e.to_string()))?;
        if data.len() != features.len() {
            return Err(fail("input feature shape mismatch".into()));
        }
        data.copy_from_slice(features);
        self.request
            .set_tensor("input_features", &input)
            .map_err(|e| fail(e.to_string()))?;
        self.request.infer().map_err(|e| fail(e.to_string()))?;
        let output = self
            .request
            .get_tensor("last_hidden_state")
            .map_err(|e| fail(e.to_string()))?;
        if output.get_element_type().map_err(|e| fail(e.to_string()))? != ElementType::F32 {
            return Err(fail("expected Float32 encoder output".into()));
        }
        let shape = output.get_shape().map_err(|e| fail(e.to_string()))?;
        let dims = shape.get_dimensions();
        if dims.len() != 3 || dims[0] != 1 || dims[1] <= 0 || dims[2] != 1024 {
            return Err(fail(format!("unexpected encoder output shape {dims:?}")));
        }
        // Own the output in ORT memory: the OpenVINO request reuses its output
        // buffers on the next invocation, while the decoder borrows this tensor.
        let data = output
            .get_data::<f32>()
            .map_err(|e| fail(e.to_string()))?
            .to_vec();
        if data.iter().any(|v| !v.is_finite()) {
            return Err(fail("non-finite encoder output".into()));
        }
        let output = OrtTensor::from_array((dims.to_vec(), data))
            .map_err(|e| fail(e.to_string()))?
            .into_dyn();
        if !self.reported_inference {
            // info accel recognizes this only after successful GPU inference,
            // not device discovery or a compilation attempt.
            tracing::info!(
                "Cohere OpenVINO GPU encoder inference succeeded; decoder remains on CPU"
            );
            self.reported_inference = true;
        }
        Ok(output)
    }
}

fn validate_input_shape(frames: usize, values: usize) -> Result<i64, String> {
    if frames == 0 || frames.checked_mul(128) != Some(values) {
        return Err("input feature shape mismatch".into());
    }
    i64::try_from(frames).map_err(|_| "input frame count exceeds i64".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::cohere_fbank::CohereFbank;

    #[test]
    fn invalid_shapes_are_rejected_before_native_allocation() {
        assert_eq!(validate_input_shape(1, 128).unwrap(), 1);
        for (frames, values) in [(0, 0), (1, 0), (2, 128), (usize::MAX, 0)] {
            assert!(validate_input_shape(frames, values).is_err());
        }
    }

    /// Opt-in hardware regression: reuse one compiled encoder across changing
    /// recording lengths, then verify an earlier ORT-owned output is unchanged.
    #[test]
    #[ignore = "requires Intel GPU, OpenVINO, VOXTYPE_COHERE_MODEL_DIR and VOXTYPE_COHERE_TEST_AUDIO"]
    fn openvino_encoder_changes_length_without_reusing_output_storage() {
        let model = std::env::var("VOXTYPE_COHERE_MODEL_DIR").expect("set model directory");
        let audio = std::env::var("VOXTYPE_COHERE_TEST_AUDIO").expect("set PCM16 WAV fixture");
        let reader = hound::WavReader::open(audio).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().bits_per_sample, 16);
        let samples: Vec<f32> = reader
            .into_samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        assert!(
            samples.len() >= 64_000,
            "fixture must be at least four seconds"
        );
        let encoder_path = Path::new(&model).join("encoder_model.onnx");
        let mut encoder = OpenVinoEncoder::new(&encoder_path).unwrap();
        let mut cpu = ort::session::Session::builder()
            .unwrap()
            .with_intra_threads(4)
            .unwrap()
            .commit_from_file(&encoder_path)
            .unwrap();
        let fbank = CohereFbank::new();
        let features = fbank.extract(&samples);
        let first = encoder
            .run(features.nrows(), features.as_slice().unwrap())
            .unwrap();
        let (first_shape, data) = first.try_extract_tensor::<f32>().unwrap();
        let original = data.to_vec();
        let original_shape = first_shape.to_vec();
        // Both Rust-side validation and a recoverable native request error must
        // leave the encoder usable. Do not use Tensor::set_shape: in this binding
        // it creates a second owning wrapper for the same native pointer.
        assert!(encoder.run(usize::MAX, &[]).is_err());
        assert!(encoder.run(1, &[]).is_err());
        let invalid = Tensor::new(ElementType::F32, &Shape::new(&[1, 1, 128]).unwrap()).unwrap();
        assert!(encoder
            .request
            .set_tensor("not_an_input", &invalid)
            .is_err());
        let all_frames = features.nrows();
        for frames in [1, 2, 7, 8, 9, 15, 16, 17, 100, all_frames, 1, all_frames] {
            let values = &features.as_slice().unwrap()[..frames * 128];
            let output = encoder.run(frames, values).unwrap();
            let (shape, data) = output.try_extract_tensor::<f32>().unwrap();
            let cpu_input = OrtTensor::from_array(([1, frames, 128], values.to_vec())).unwrap();
            let cpu_outputs = cpu
                .run(ort::inputs!["input_features" => cpu_input])
                .unwrap();
            let (cpu_shape, cpu_data) = cpu_outputs["last_hidden_state"]
                .try_extract_tensor::<f32>()
                .unwrap();
            assert_eq!(
                shape, cpu_shape,
                "temporal shape mismatch for {frames} frames"
            );
            assert_eq!(data.len(), cpu_data.len());
            assert!(data.iter().all(|v| v.is_finite()));
            let error_squared: f64 = data
                .iter()
                .zip(cpu_data)
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum();
            let reference_squared: f64 = cpu_data.iter().map(|v| f64::from(*v).powi(2)).sum();
            let relative_l2 = (error_squared / reference_squared.max(f64::EPSILON)).sqrt();
            println!("frames={frames} relative_l2={relative_l2:.6}");
            assert!(relative_l2 < 0.05, "encoder numerical drift: {relative_l2}");
            if frames == all_frames {
                assert_eq!(shape.as_ref(), original_shape.as_slice());
                let max_difference = data
                    .iter()
                    .zip(&original)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    max_difference < 0.01,
                    "repeated encoder output drift: {max_difference}"
                );
            }
            assert_eq!(
                first.try_extract_tensor::<f32>().unwrap().1,
                original.as_slice()
            );
        }
        drop(encoder);
        let mut reloaded = OpenVinoEncoder::new(&encoder_path).unwrap();
        let output = reloaded
            .run(all_frames, features.as_slice().unwrap())
            .unwrap();
        assert_eq!(
            output.try_extract_tensor::<f32>().unwrap().0.as_ref(),
            original_shape.as_slice()
        );
        assert_eq!(
            first.try_extract_tensor::<f32>().unwrap().1,
            original.as_slice()
        );
    }
}
