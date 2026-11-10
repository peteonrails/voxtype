//! Helper binary that dumps voxtype's model registries to stdout as JSON for
//! `scripts/mirror-models-to-r2.sh`, so the script doesn't have to parse Rust
//! source to know which models to mirror.
//!
//! Default: the ONNX-engine registry. Each entry includes the engine
//! sub-namespace, the model name (which doubles as the R2 directory under the
//! engine), the upstream HuggingFace repo, and the upstream->local file-path
//! mapping. The mirror script downloads upstream paths and uploads them to R2
//! under local paths so the runtime sees exactly what `download_artifact`
//! expects.
//!
//! `--gguf`: the curated single-file GGUF catalog with the URL the runtime
//! downloads from, pinned size and sha256, for the R2 audit.

fn main() {
    let json = if std::env::args().any(|arg| arg == "--gguf") {
        serde_json::to_string_pretty(&voxtype::setup::model::gguf_registry_snapshot())
    } else {
        serde_json::to_string_pretty(&voxtype::setup::model::registry_snapshot())
    }
    .expect("registry serializes to JSON");
    println!("{}", json);
}
