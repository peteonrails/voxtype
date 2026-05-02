//! Shared helper that registers ONNX Runtime execution providers for any
//! ONNX-backed engine in voxtype.
//!
//! Each engine's session builder calls [`register_gpu_eps`] to attach the
//! GPU EPs that were compiled into this binary. The compile-time gating
//! lives on three marker features in `Cargo.toml`:
//!
//! - `onnx-cuda-enabled`     — CUDA EP (NVIDIA)
//! - `onnx-migraphx-enabled` — MIGraphX EP (AMD)
//! - `onnx-tensorrt-enabled` — TensorRT EP (NVIDIA, optimized)
//!
//! Enabling any per-engine feature like `cohere-migraphx` or
//! `parakeet-cuda` transitively enables the matching marker, so this
//! helper sees the right EPs without each engine duplicating the cfg
//! plumbing.
//!
//! Order matters: ort tries EPs in sequence and falls through to the
//! next on registration failure. Specialized EPs (TensorRT) come before
//! their generic siblings (CUDA). The CPU EP is always implicit at the
//! bottom of the chain — even if every GPU EP fails to register at
//! runtime (no GPU, missing driver, missing companion .so files), ort
//! still runs the model on CPU.

#[cfg(feature = "onnx-common")]
use ort::execution_providers::ExecutionProviderDispatch;
#[cfg(feature = "onnx-common")]
use ort::session::builder::{BuilderResult, SessionBuilder};

/// Register GPU EPs onto a session builder.
///
/// `engine_label` and `session_label` are used only for logging
/// (`"Cohere encoder: registering execution providers [...]"`). Returns
/// the modified builder; if no EPs are compiled in or registration
/// fails, falls through unchanged and ort uses the CPU EP.
#[cfg(feature = "onnx-common")]
pub fn register_gpu_eps(
    builder: SessionBuilder,
    engine_label: &str,
    session_label: &str,
) -> BuilderResult {
    let providers = compiled_providers();
    if providers.is_empty() {
        return Ok(builder);
    }
    let names: Vec<&'static str> = providers.iter().map(|(n, _)| *n).collect();
    tracing::info!("{engine_label} {session_label}: registering execution providers {names:?}");
    let dispatches: Vec<_> = providers.into_iter().map(|(_, ep)| ep).collect();
    builder.with_execution_providers(dispatches)
}

#[cfg(feature = "onnx-common")]
fn compiled_providers() -> Vec<(&'static str, ExecutionProviderDispatch)> {
    #[allow(unused_mut)]
    let mut providers: Vec<(&'static str, ExecutionProviderDispatch)> = Vec::new();

    #[cfg(feature = "onnx-tensorrt-enabled")]
    {
        use ort::execution_providers::{ExecutionProvider, TensorRTExecutionProvider};
        providers.push(("TensorRT", TensorRTExecutionProvider::default().build()));
    }
    #[cfg(feature = "onnx-cuda-enabled")]
    {
        use ort::execution_providers::{CUDAExecutionProvider, ExecutionProvider};
        providers.push(("CUDA", CUDAExecutionProvider::default().build()));
    }
    #[cfg(feature = "onnx-migraphx-enabled")]
    {
        use ort::execution_providers::{ExecutionProvider, MIGraphXExecutionProvider};
        providers.push(("MIGraphX", MIGraphXExecutionProvider::default().build()));
    }

    providers
}
