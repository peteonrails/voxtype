//! Helper binary that dumps voxtype's ONNX-engine model registry to stdout
//! as JSON. Consumed by `scripts/mirror-models-to-r2.sh` so the script
//! doesn't have to parse Rust source to know which models to mirror.
//!
//! Each registry entry includes the engine sub-namespace, the model name
//! (which doubles as the R2 directory under the engine), the upstream
//! HuggingFace repo, and the upstream→local file-path mapping. The mirror
//! script downloads upstream paths and uploads them to R2 under local paths
//! so the runtime sees exactly what `download_artifact` expects.

fn main() {
    let registry = voxtype::setup::model::registry_snapshot();
    let json = serde_json::to_string_pretty(&registry).expect("registry serializes to JSON");
    println!("{}", json);
}
