//! Per-engine model catalogs and on-disk installation checks.
//!
//! Extracted from `src/tui/engine.rs` so `voxtype info models --json` and the
//! TUI's engine picker enumerate the same models. Whisper, Parakeet,
//! Moonshine and SenseVoice come from the central `setup::model` registry;
//! the remaining ONNX engines aren't registered there yet, so their
//! canonical names are listed here.

use std::path::Path;

use crate::config::Config;
use crate::setup::manifest::read_cached_manifest;
use crate::setup::model;

/// Every engine name that has a model catalog, in the order the TUI shows
/// them.
pub const CATALOG_ENGINES: &[&str] = &[
    "whisper",
    "parakeet",
    "moonshine",
    "sensevoice",
    "paraformer",
    "dolphin",
    "omnilingual",
    "cohere",
    "openvino",
];

/// Models voxtype knows how to download for `engine`.
pub fn model_catalog(engine: &str) -> Vec<&'static str> {
    match engine {
        "whisper" => model::valid_model_names(),
        "parakeet" => model::valid_parakeet_model_names(),
        "moonshine" => model::valid_moonshine_model_names(),
        "sensevoice" => model::valid_sensevoice_model_names(),
        "paraformer" => vec!["paraformer-zh", "paraformer-en"],
        "dolphin" => vec!["dolphin-base"],
        "omnilingual" => vec!["omnilingual-300m"],
        "cohere" => vec![
            "cohere-transcribe-q4f16",
            "cohere-transcribe-q4",
            "cohere-transcribe-int8",
            "cohere-transcribe-fp16",
        ],
        "openvino" => model::valid_openvino_model_names(),
        _ => Vec::new(),
    }
}

/// Default model name baked into voxtype for each engine. Used when a fresh
/// `[engine]` table has to be materialized — those structs require `model`
/// and the validator rejects a partial table.
pub const fn default_model(engine: &str) -> &'static str {
    match engine.as_bytes() {
        b"whisper" => "base.en",
        b"parakeet" => "parakeet-tdt-0.6b-v3",
        b"moonshine" => "base",
        b"sensevoice" => "sensevoice-small",
        b"paraformer" => "paraformer-zh",
        b"dolphin" => "dolphin-base",
        b"omnilingual" => "omnilingual-300m",
        b"cohere" => "cohere-transcribe-q4f16",
        b"openvino" => "base.en-int8",
        _ => "",
    }
}

/// Directory a catalog entry occupies under the models dir.
///
/// Most engines name the directory exactly as the catalog (and the config
/// field) does. Moonshine and SenseVoice don't: their config values are short
/// (`base`, `small`) while the directory carries the engine prefix
/// (`moonshine-base`, `sensevoice-small`). Anything that looks for files on
/// disk has to go through here, or it reports installed models as missing.
pub fn model_dir_name(engine: &str, model: &str) -> String {
    match engine {
        "moonshine" => model::moonshine_dir_name(model)
            .unwrap_or(model)
            .to_string(),
        "sensevoice" => model::sensevoice_dir_name(model)
            .unwrap_or(model)
            .to_string(),
        "openvino" => model::openvino_dir_name(model).unwrap_or(model).to_string(),
        _ => model.to_string(),
    }
}

/// The `--model` value that downloads this catalog entry.
///
/// `None` means `voxtype setup --download` can't fetch it: `run_setup` only
/// routes Whisper, Parakeet, SenseVoice, and OpenVINO names, so the other
/// ONNX engines
/// are reachable only through the interactive picker (`voxtype setup model`).
/// A UI should not offer a Download button for those.
///
/// SenseVoice returns its directory name rather than its config value,
/// because `small` is also a Whisper model name and Whisper wins that
/// collision in `setup --model`.
pub fn download_arg(engine: &str, model: &str) -> Option<String> {
    match engine {
        "whisper" | "parakeet" | "openvino" => Some(model.to_string()),
        "sensevoice" => Some(model_dir_name(engine, model)),
        _ => None,
    }
}

/// On-disk location of a model: a single `ggml-<name>.bin` file for whisper,
/// a directory for every ONNX engine.
fn model_path(models_dir: &Path, engine: &str, model: &str) -> std::path::PathBuf {
    if engine == "whisper" {
        models_dir.join(format!("ggml-{}.bin", model))
    } else {
        models_dir.join(model_dir_name(engine, model))
    }
}

/// What the cheap integrity check concluded about a model on disk.
///
/// "Cheap" is the whole point: this runs for every model on every listing, so
/// it may `stat` files and read a few bytes, but never hash them. See
/// [`verify_model`] for the thorough version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelHealth {
    /// Nothing on disk.
    Missing,
    /// On disk, and nothing checkable contradicts that.
    Present,
    /// On disk but demonstrably wrong. Each entry describes one problem in
    /// terms a user can act on.
    Corrupt(Vec<String>),
}

/// How much of a model's integrity could actually be established by hashing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelVerification {
    /// Every file matched the sha256 recorded when the model was downloaded.
    Ok,
    Corrupt(Vec<String>),
    /// There is nothing on record to compare against. The string says why, so
    /// a UI can distinguish "checked and fine" from "couldn't check".
    Unverifiable(&'static str),
}

/// Cheap integrity check for one model.
///
/// What can be checked depends on the engine, because the evidence available
/// differs (see the module docs on `crate::setup::manifest`):
///
/// - ONNX models downloaded since voxtype started caching manifests carry the
///   publisher's file list and sizes in their own directory, so a size
///   mismatch or a missing file is detectable by `stat` alone.
/// - Older ONNX installs have no cached manifest. Their expected file *names*
///   are still known, so a missing file is detectable; sizes are not, because
///   the compiled-in numbers have drifted from upstream.
/// - Whisper models have no manifest anywhere (they come from HuggingFace, not
///   the R2 mirror). The ggml file magic is the one exact check available, and
///   it catches the common failure of an error page saved under a `.bin` name.
///   A truncated whisper model with intact magic reads as healthy here.
pub fn model_health(engine: &str, model: &str) -> ModelHealth {
    model_health_in(&Config::models_dir(), engine, model)
}

pub(crate) fn model_health_in(models_dir: &Path, engine: &str, model: &str) -> ModelHealth {
    let path = model_path(models_dir, engine, model);
    if !path.exists() {
        return ModelHealth::Missing;
    }

    if engine == "whisper" {
        return match model::validate_download(&path, None, model::ContentCheck::Ggml) {
            Ok(()) => ModelHealth::Present,
            Err(e) => ModelHealth::Corrupt(vec![format!("{}: {}", display_name(&path), e)]),
        };
    }

    if engine == "openvino" {
        return match model::validate_openvino_model(&path) {
            Ok(()) => ModelHealth::Present,
            Err(e) => ModelHealth::Corrupt(vec![format!("{}: {}", display_name(&path), e)]),
        };
    }

    let mut problems = Vec::new();
    match read_cached_manifest(&path) {
        Some(manifest) => {
            for file in &manifest.files {
                let f = path.join(&file.path);
                match std::fs::metadata(&f) {
                    Ok(meta) if meta.len() == file.size => {}
                    Ok(meta) => problems.push(format!(
                        "{}: {} bytes on disk, manifest says {}",
                        file.path,
                        meta.len(),
                        file.size
                    )),
                    Err(_) => problems.push(format!("{}: missing", file.path)),
                }
            }
        }
        None => {
            // The registry keys ONNX models by directory name, which is not
            // the catalog name for moonshine and sensevoice.
            for name in model::expected_file_names(engine, &model_dir_name(engine, model)) {
                if !path.join(&name).exists() {
                    problems.push(format!("{}: missing", name));
                }
            }
        }
    }

    if problems.is_empty() {
        ModelHealth::Present
    } else {
        ModelHealth::Corrupt(problems)
    }
}

/// Hash every file of a model against the manifest recorded at download time.
///
/// Reads every byte, so this is only for `voxtype info models --verify`, never
/// the default listing.
pub fn verify_model(engine: &str, model: &str) -> ModelVerification {
    verify_model_in(&Config::models_dir(), engine, model)
}

pub(crate) fn verify_model_in(models_dir: &Path, engine: &str, model: &str) -> ModelVerification {
    let path = model_path(models_dir, engine, model);

    // Whatever the cheap check already found is still true, and costs nothing
    // to report here.
    if let ModelHealth::Corrupt(problems) = model_health_in(models_dir, engine, model) {
        return ModelVerification::Corrupt(problems);
    }

    if engine == "whisper" {
        return ModelVerification::Unverifiable(
            "whisper models are published without checksums, so only their \
             ggml header could be checked",
        );
    }

    let Some(manifest) = read_cached_manifest(&path) else {
        return ModelVerification::Unverifiable(
            "no manifest was recorded when this model was downloaded; \
             re-download it to enable verification",
        );
    };

    let mut problems = Vec::new();
    for file in &manifest.files {
        let f = path.join(&file.path);
        match model::sha256_file(&f) {
            Ok(hash) if hash == file.sha256.to_lowercase() => {}
            Ok(hash) => problems.push(format!(
                "{}: sha256 {} does not match the manifest's {}",
                file.path, hash, file.sha256
            )),
            Err(e) => problems.push(format!("{}: could not be read ({})", file.path, e)),
        }
    }

    if problems.is_empty() {
        ModelVerification::Ok
    } else {
        ModelVerification::Corrupt(problems)
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Is `model` usable from the models dir for `engine`?
///
/// False both for absent models and for present-but-damaged ones, so a caller
/// offering a picker never lists a model the daemon would fail to load.
pub fn model_installed(engine: &str, model: &str) -> bool {
    model_installed_in(&Config::models_dir(), engine, model)
}

pub(crate) fn model_installed_in(models_dir: &Path, engine: &str, model: &str) -> bool {
    model_health_in(models_dir, engine, model) == ModelHealth::Present
}

/// Catalog entries for `engine` that are actually on disk.
pub fn installed_models_for(engine: &str) -> Vec<String> {
    model_catalog(engine)
        .into_iter()
        .filter(|name| model_installed(engine, name))
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_set::ENGINE_NAMES;
    use crate::setup::manifest::{write_cached_manifest, Manifest, ManifestFile};

    /// Build a model directory and record a manifest describing exactly what
    /// was written, the way a real download leaves things.
    fn install_onnx_model(
        models_dir: &Path,
        engine: &str,
        model: &str,
        files: &[(&str, &[u8])],
    ) -> std::path::PathBuf {
        let dir = models_dir.join(model);
        std::fs::create_dir_all(&dir).unwrap();
        let mut entries = Vec::new();
        for (name, bytes) in files {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            entries.push(ManifestFile {
                path: (*name).to_string(),
                size: bytes.len() as u64,
                sha256: model::sha256_file(&path).unwrap(),
            });
        }
        write_cached_manifest(
            &dir,
            &Manifest {
                version: 1,
                model: model.to_string(),
                engine: engine.to_string(),
                files: entries,
            },
        );
        dir
    }

    /// The cheap tier: with a manifest on disk, a wrong size or a missing file
    /// is visible from `stat` alone.
    #[test]
    fn cached_manifest_sizes_catch_damage_without_hashing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = install_onnx_model(
            tmp.path(),
            "parakeet",
            "parakeet-tdt-0.6b-v3-int8",
            &[
                ("encoder-model.int8.onnx", b"encoder bytes"),
                ("vocab.txt", b"vocab"),
            ],
        );
        assert_eq!(
            model_health_in(tmp.path(), "parakeet", "parakeet-tdt-0.6b-v3-int8"),
            ModelHealth::Present
        );

        // A truncated file: same name, fewer bytes.
        std::fs::write(dir.join("encoder-model.int8.onnx"), b"enc").unwrap();
        let problems = match model_health_in(tmp.path(), "parakeet", "parakeet-tdt-0.6b-v3-int8") {
            ModelHealth::Corrupt(p) => p,
            other => panic!("expected corrupt, got {:?}", other),
        };
        assert_eq!(problems.len(), 1, "{:?}", problems);
        assert!(
            problems[0].contains("3 bytes on disk") && problems[0].contains("says 13"),
            "{:?}",
            problems
        );

        // A file that went away entirely.
        std::fs::remove_file(dir.join("vocab.txt")).unwrap();
        match model_health_in(tmp.path(), "parakeet", "parakeet-tdt-0.6b-v3-int8") {
            ModelHealth::Corrupt(p) => {
                assert!(p.iter().any(|s| s == "vocab.txt: missing"), "{:?}", p)
            }
            other => panic!("expected corrupt, got {:?}", other),
        }
    }

    /// Models installed before voxtype cached manifests must not be called
    /// corrupt on the strength of the compiled-in sizes, which have drifted
    /// from upstream. Only a missing file is provable there.
    #[test]
    fn without_a_manifest_sizes_are_not_evidence_but_missing_files_are() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("parakeet-tdt-0.6b-v3-int8");
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "encoder-model.int8.onnx",
            "decoder_joint-model.int8.onnx",
            "vocab.txt",
            "config.json",
        ] {
            // Sizes nowhere near the compiled-in expectations.
            std::fs::write(dir.join(name), b"stub").unwrap();
        }
        assert_eq!(
            model_health_in(tmp.path(), "parakeet", "parakeet-tdt-0.6b-v3-int8"),
            ModelHealth::Present,
            "compiled-in sizes must not be used as a corruption signal"
        );

        std::fs::remove_file(dir.join("vocab.txt")).unwrap();
        assert!(matches!(
            model_health_in(tmp.path(), "parakeet", "parakeet-tdt-0.6b-v3-int8"),
            ModelHealth::Corrupt(_)
        ));
    }

    /// Whisper has no manifest anywhere, so the ggml header is the only exact
    /// check. It catches an error page saved under a model name.
    #[test]
    fn whisper_health_rests_on_the_ggml_header() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            model_health_in(tmp.path(), "whisper", "tiny"),
            ModelHealth::Missing
        );

        std::fs::write(tmp.path().join("ggml-tiny.bin"), b"lmgg\x00\x01\x02\x03").unwrap();
        assert_eq!(
            model_health_in(tmp.path(), "whisper", "tiny"),
            ModelHealth::Present
        );

        std::fs::write(tmp.path().join("ggml-tiny.bin"), b"<!DOCTYPE html>").unwrap();
        match model_health_in(tmp.path(), "whisper", "tiny") {
            ModelHealth::Corrupt(p) => {
                assert!(p[0].contains("not a ggml model"), "{:?}", p)
            }
            other => panic!("expected corrupt, got {:?}", other),
        }
        assert!(!model_installed_in(tmp.path(), "whisper", "tiny"));
    }

    /// Why the slow tier exists: content can rot without the size changing,
    /// and only hashing sees that.
    #[test]
    fn verify_catches_tampering_that_stat_cannot_see() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = install_onnx_model(
            tmp.path(),
            "dolphin",
            "dolphin-base",
            &[("model.onnx", b"original contents")],
        );
        assert_eq!(
            verify_model_in(tmp.path(), "dolphin", "dolphin-base"),
            ModelVerification::Ok
        );

        // Same length, different bytes.
        std::fs::write(dir.join("model.onnx"), b"tampered contents").unwrap();
        assert_eq!(
            model_health_in(tmp.path(), "dolphin", "dolphin-base"),
            ModelHealth::Present,
            "the cheap check cannot see a same-size change"
        );
        match verify_model_in(tmp.path(), "dolphin", "dolphin-base") {
            ModelVerification::Corrupt(p) => {
                assert!(p[0].contains("sha256"), "{:?}", p)
            }
            other => panic!("expected corrupt, got {:?}", other),
        }
    }

    /// Verification has to distinguish "checked and fine" from "nothing to
    /// check against", or a UI would show the second as the first.
    #[test]
    fn verification_is_explicit_about_what_it_could_not_check() {
        let tmp = tempfile::tempdir().unwrap();

        // Every expected file present, but no manifest: nothing to hash
        // against, which is where every install predating manifest caching
        // lands.
        let dir = tmp.path().join("dolphin-base");
        std::fs::create_dir_all(&dir).unwrap();
        for name in model::expected_file_names("dolphin", "dolphin-base") {
            std::fs::write(dir.join(name), b"whatever").unwrap();
        }
        match verify_model_in(tmp.path(), "dolphin", "dolphin-base") {
            ModelVerification::Unverifiable(why) => assert!(why.contains("re-download"), "{}", why),
            other => panic!("expected unverifiable, got {:?}", other),
        }

        std::fs::write(tmp.path().join("ggml-tiny.bin"), b"lmgg\x00\x01\x02\x03").unwrap();
        match verify_model_in(tmp.path(), "whisper", "tiny") {
            ModelVerification::Unverifiable(why) => {
                assert!(why.contains("without checksums"), "{}", why)
            }
            other => panic!("expected unverifiable, got {:?}", other),
        }
    }

    #[test]
    fn every_catalog_engine_has_models_and_a_default() {
        for engine in CATALOG_ENGINES {
            assert!(
                !model_catalog(engine).is_empty(),
                "no catalog for '{}'",
                engine
            );
            assert!(
                !default_model(engine).is_empty(),
                "no default model for '{}'",
                engine
            );
        }
    }

    /// SenseVoice is the one engine whose registry uses short names while its
    /// config value is the directory name, so `default_model` deliberately
    /// does not appear in `model_catalog`. Pinned here so nobody "fixes" the
    /// mismatch by changing the default to a name the config field doesn't
    /// take.
    #[test]
    fn sensevoice_catalog_uses_short_names_while_the_default_is_a_dir_name() {
        let catalog = model_catalog("sensevoice");
        assert!(catalog.contains(&"small"), "got {:?}", catalog);
        assert_eq!(default_model("sensevoice"), "sensevoice-small");
        assert_eq!(
            model::sensevoice_dir_name("small"),
            Some("sensevoice-small"),
            "the short name must still map to the default dir name"
        );

        // Every other catalog engine names its default directly.
        for engine in CATALOG_ENGINES {
            if *engine == "sensevoice" {
                continue;
            }
            let default = default_model(engine);
            assert!(
                model_catalog(engine).contains(&default),
                "default model '{}' for '{}' is not in its own catalog",
                default,
                engine
            );
        }
    }

    /// Regression (#662): sensevoice's config value is a short name and the
    /// downloader writes a differently-named directory. A caller asking "is
    /// the configured model installed?" passes the config value, so the
    /// lookup has to resolve it. The configure TUI probed the raw value and
    /// told users a working model was not downloaded.
    ///
    /// Pinned end to end rather than on the name mapping alone, because the
    /// mapping was always correct — it was the lookup that skipped it.
    #[test]
    fn an_installed_sensevoice_model_is_found_by_its_short_config_name() {
        let tmp = tempfile::tempdir().unwrap();
        install_onnx_model(
            tmp.path(),
            "sensevoice",
            "sensevoice-small",
            &[("model.onnx", b"weights"), ("tokens.txt", b"tokens")],
        );

        assert!(
            model_installed_in(tmp.path(), "sensevoice", "small"),
            "the short config value must resolve to the directory on disk"
        );
        assert!(
            model_installed_in(tmp.path(), "sensevoice", "sensevoice-small"),
            "the directory name itself must keep working"
        );
        assert!(!model_installed_in(tmp.path(), "sensevoice", "medium"));
    }

    /// The catalog covers every engine `config set engine` accepts, minus
    /// cloud engines that have no downloadable model.
    #[test]
    fn catalog_covers_the_settable_engines() {
        for name in ENGINE_NAMES {
            assert!(
                CATALOG_ENGINES.contains(name),
                "engine '{}' is settable but has no model catalog",
                name
            );
        }
    }

    #[test]
    fn unknown_engine_has_an_empty_catalog() {
        assert!(model_catalog("nope").is_empty());
        assert_eq!(default_model("nope"), "");
    }
}
