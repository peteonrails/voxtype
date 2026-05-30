//! R2 model manifest schema and `ModelArtifact` trait.
//!
//! Every ONNX-engine model voxtype downloads is mirrored to Cloudflare R2 at
//! `https://models.voxtype.io/{engine_prefix}/{model_name}/`. Alongside the
//! model files sits a `manifest.json` describing the file list and per-file
//! sha256 hashes. The runtime downloader fetches the manifest first, then
//! validates each file as it lands on disk.
//!
//! The manifest is the source of truth for integrity. The struct-level
//! `expected_files()` list in `ModelArtifact` is a sanity check used by both
//! the mirror script (so it knows what to publish) and the runtime (so a
//! download that succeeds but doesn't match the publisher's expectation is
//! surfaced before transcription tries to load a broken model directory).
//!
//! See `roadmap` in `CLAUDE.md` ("voxtype-models CDN") and the matching
//! mirror script at `scripts/mirror-models-to-r2.sh`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Base URL for the Cloudflare R2 model mirror. Every model artifact lives at
/// `{MODELS_BASE_URL}/{engine_prefix}/{name}/{file}` and ships a sibling
/// `manifest.json` describing the files and their sha256 hashes.
pub const MODELS_BASE_URL: &str = "https://models.voxtype.io";

/// Schema version currently produced by the mirror script and consumed by the
/// runtime downloader. Bump this only with a coordinated runtime + mirror
/// rollout.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// One file's entry in `manifest.json`. `path` is relative to the model
/// directory on R2 (and to the local target directory on disk). `sha256` is
/// lowercase hex, matching the output of `sha256sum`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// `manifest.json` content shipped alongside every model on R2.
///
/// `model` and `engine` are checked against the requesting `ModelArtifact` so
/// a misrouted upload (e.g. parakeet manifest under moonshine prefix) fails
/// fast instead of silently corrupting a model directory.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub model: String,
    pub engine: String,
    pub files: Vec<ManifestFile>,
}

/// A file the local installer expects to see in the model directory once
/// download finishes. The runtime compares the manifest's `files` against the
/// artifact's `expected_files()` so a publisher who forgets to list a file
/// (and therefore wouldn't sha256-verify it) doesn't silently leave a hole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedFile {
    pub path: String,
    pub size: u64,
}

/// Common interface across every ONNX engine's model definition struct.
/// Implemented by `ParakeetModelInfo`, `MoonshineModelInfo`, etc. in
/// `super::model`. The unified `download_artifact()` consumes this trait so
/// the per-engine `download_*_model_by_info` duplicates can be deleted.
///
/// `engine_prefix` is the R2 sub-namespace and matches the manifest's
/// `engine` field. `upstream_repo` is only consumed by the mirror script
/// (`scripts/mirror-models-to-r2.sh`); the runtime never touches it.
pub trait ModelArtifact {
    /// Stable model identifier. Used as the URL segment under `engine_prefix`
    /// and as the on-disk directory name under `models_dir`.
    fn name(&self) -> &str;

    /// Engine sub-namespace under `MODELS_BASE_URL`. Static because every
    /// implementor knows its engine at compile time.
    fn engine_prefix(&self) -> &'static str;

    /// HuggingFace `owner/repo` of the upstream the model was mirrored from.
    /// The mirror script reads this; the runtime does not.
    fn upstream_repo(&self) -> &str;

    /// Files the publisher expects this model to ship, with their expected
    /// sizes. The manifest is authoritative for sha256s; this list catches
    /// "publisher forgot to enumerate a file in the manifest" cases.
    fn expected_files(&self) -> Vec<ExpectedFile>;
}

/// Validate that a manifest matches the artifact requesting it. Returns an
/// error if the version/model/engine don't line up, or if any file the
/// artifact expects is missing from the manifest.
///
/// Pulled out into a free function so unit tests can exercise it without
/// going near the network.
pub fn validate_manifest<T: ModelArtifact + ?Sized>(
    manifest: &Manifest,
    artifact: &T,
) -> anyhow::Result<()> {
    if manifest.version != MANIFEST_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported manifest version {} (expected {}) for model '{}'",
            manifest.version,
            MANIFEST_SCHEMA_VERSION,
            artifact.name()
        );
    }
    if manifest.model != artifact.name() {
        anyhow::bail!(
            "manifest model name mismatch: manifest says '{}', artifact says '{}'",
            manifest.model,
            artifact.name()
        );
    }
    if manifest.engine != artifact.engine_prefix() {
        anyhow::bail!(
            "manifest engine mismatch for '{}': manifest says '{}', artifact says '{}'",
            artifact.name(),
            manifest.engine,
            artifact.engine_prefix()
        );
    }
    let manifest_paths: std::collections::HashSet<&str> =
        manifest.files.iter().map(|f| f.path.as_str()).collect();
    let mut missing = Vec::new();
    for expected in artifact.expected_files() {
        if !manifest_paths.contains(expected.path.as_str()) {
            missing.push(expected.path);
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "manifest for '{}' is missing expected files: {}",
            artifact.name(),
            missing.join(", ")
        );
    }
    Ok(())
}

/// Compute the R2 URL for a model's manifest.
pub fn manifest_url<T: ModelArtifact + ?Sized>(artifact: &T) -> String {
    format!(
        "{}/{}/{}/manifest.json",
        MODELS_BASE_URL,
        artifact.engine_prefix(),
        artifact.name()
    )
}

/// Compute the R2 URL for one file within a model.
pub fn file_url<T: ModelArtifact + ?Sized>(artifact: &T, relative_path: &str) -> String {
    format!(
        "{}/{}/{}/{}",
        MODELS_BASE_URL,
        artifact.engine_prefix(),
        artifact.name(),
        relative_path
    )
}

/// Local on-disk path for one file within a model directory under
/// `models_dir`. Centralised so the downloader and the mirror script agree
/// on layout.
pub fn local_file_path(models_dir: &std::path::Path, name: &str, relative_path: &str) -> PathBuf {
    models_dir.join(name).join(relative_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeArtifact {
        name: &'static str,
        engine: &'static str,
        files: Vec<ExpectedFile>,
    }

    impl ModelArtifact for FakeArtifact {
        fn name(&self) -> &str {
            self.name
        }
        fn engine_prefix(&self) -> &'static str {
            self.engine
        }
        fn upstream_repo(&self) -> &str {
            "fake/repo"
        }
        fn expected_files(&self) -> Vec<ExpectedFile> {
            self.files.clone()
        }
    }

    fn good_manifest() -> Manifest {
        Manifest {
            version: 1,
            model: "fake".to_string(),
            engine: "parakeet".to_string(),
            files: vec![
                ManifestFile {
                    path: "encoder.onnx".to_string(),
                    size: 100,
                    sha256: "aa".to_string(),
                },
                ManifestFile {
                    path: "vocab.txt".to_string(),
                    size: 10,
                    sha256: "bb".to_string(),
                },
            ],
        }
    }

    fn good_artifact() -> FakeArtifact {
        FakeArtifact {
            name: "fake",
            engine: "parakeet",
            files: vec![ExpectedFile {
                path: "encoder.onnx".to_string(),
                size: 100,
            }],
        }
    }

    #[test]
    fn manifest_round_trip_json() {
        let m = good_manifest();
        let s = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn deserializes_sample_json() {
        let json = r#"{
            "version": 1,
            "model": "parakeet-unified-en-0.6b",
            "engine": "parakeet",
            "files": [
                {"path": "encoder.onnx", "size": 43878400, "sha256": "abcd"},
                {"path": "tokenizer.model", "size": 257024, "sha256": "ef01"}
            ]
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, 1);
        assert_eq!(m.model, "parakeet-unified-en-0.6b");
        assert_eq!(m.engine, "parakeet");
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[0].path, "encoder.onnx");
        assert_eq!(m.files[0].size, 43_878_400);
    }

    #[test]
    fn validate_manifest_happy_path() {
        validate_manifest(&good_manifest(), &good_artifact()).unwrap();
    }

    #[test]
    fn validate_manifest_rejects_wrong_version() {
        let mut m = good_manifest();
        m.version = 2;
        let err = validate_manifest(&m, &good_artifact()).unwrap_err();
        assert!(err.to_string().contains("unsupported manifest version"));
    }

    #[test]
    fn validate_manifest_rejects_model_mismatch() {
        let mut m = good_manifest();
        m.model = "different".to_string();
        let err = validate_manifest(&m, &good_artifact()).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("manifest model name mismatch"), "got: {}", s);
        assert!(s.contains("different"));
        assert!(s.contains("fake"));
    }

    #[test]
    fn validate_manifest_rejects_engine_mismatch() {
        let mut m = good_manifest();
        m.engine = "moonshine".to_string();
        let err = validate_manifest(&m, &good_artifact()).unwrap_err();
        assert!(err.to_string().contains("manifest engine mismatch"));
    }

    #[test]
    fn validate_manifest_flags_missing_expected_file() {
        let artifact = FakeArtifact {
            name: "fake",
            engine: "parakeet",
            files: vec![
                ExpectedFile {
                    path: "encoder.onnx".to_string(),
                    size: 100,
                },
                ExpectedFile {
                    path: "missing.bin".to_string(),
                    size: 50,
                },
            ],
        };
        let err = validate_manifest(&good_manifest(), &artifact).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("missing expected files"), "got: {}", s);
        assert!(s.contains("missing.bin"), "got: {}", s);
    }

    #[test]
    fn url_helpers_match_spec() {
        let a = good_artifact();
        assert_eq!(
            manifest_url(&a),
            "https://models.voxtype.io/parakeet/fake/manifest.json"
        );
        assert_eq!(
            file_url(&a, "encoder.onnx"),
            "https://models.voxtype.io/parakeet/fake/encoder.onnx"
        );
    }
}
