//! Explicit installation of the separately licensed LARD model, and the one
//! place that decides whether the stage can run for a configuration.
//!
//! The daemon, `voxtype doctor`, `voxtype config set`, the settings schema
//! and the TUI all ask [`problem`] (or [`load_tagger`]) so they give the same
//! answer and the same fix. Before this, a config with `text.lard_disfluency`
//! on and no model skipped the stage with one line in the journal (#696).

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::config::{Config, TextConfig};
use crate::text::disfluency::DisfluencyTagger;

pub const MODEL_SHA256: &str = "f11b45d644b2d829349df2ac2ad3d69c9cf19cd3fae888c69268bddf64cfb354";
const MODEL_SIZE: u64 = 65_560;

/// Where the model can be had today. Publication on the model host is still
/// pending approval, so this names the copy in the voxtype source tree. It is
/// a path, not a URL: nothing here may point at a download that does not
/// exist.
pub const SOURCE_TREE_PATH: &str = "assets/models/lard-disfluency-v1.vxdf";

const TURN_OFF: &str = "or turn it off: voxtype config set text.lard_disfluency false";

pub fn model_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voxtype/models/lard-disfluency-v1.vxdf")
}

/// Why the LARD stage cannot run although `text.lard_disfluency` is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The active engine's language is not an explicit English one.
    NotEnglish,
    /// No file at the model path. `custom` says the path came from
    /// `text.lard_model_path` rather than the default install location.
    ModelMissing { path: PathBuf, custom: bool },
    /// Something is at the model path but it does not load as a VXDF model.
    ModelInvalid { path: PathBuf, error: String },
}

impl Problem {
    /// What is wrong, as one sentence without a trailing period.
    pub fn message(&self) -> String {
        match self {
            Problem::NotEnglish => "LARD disfluency cleanup is on but is being skipped: it only runs when the engine language is set to English"
                .to_string(),
            Problem::ModelMissing {
                path,
                custom: false,
            } => format!(
                "LARD disfluency cleanup is on but its model is not installed ({})",
                path.display()
            ),
            Problem::ModelMissing { path, custom: true } => format!(
                "LARD disfluency cleanup is on but text.lard_model_path names a missing file ({})",
                path.display()
            ),
            Problem::ModelInvalid { path, error } => format!(
                "LARD disfluency cleanup is on but its model does not load ({}: {error})",
                path.display()
            ),
        }
    }

    /// What to do about it. The model is not downloadable, so the fix never
    /// promises a download: it names the source-tree copy and the off switch.
    pub fn fix(&self) -> String {
        match self {
            Problem::NotEnglish => format!(
                "Set the active engine's language to en instead of auto (engines \
                 without a language setting cannot run this stage), {TURN_OFF}"
            ),
            Problem::ModelMissing { custom: true, .. } => format!(
                "Correct [text] lard_model_path, or remove it to use the model installed by voxtype setup lard, {TURN_OFF}"
            ),
            Problem::ModelMissing { custom: false, .. } | Problem::ModelInvalid { .. } => {
                format!(
                    "The model cannot be downloaded yet. Take {SOURCE_TREE_PATH} from the voxtype source and run: voxtype setup lard --source <that file>, {TURN_OFF}"
                )
            }
        }
    }

    /// A few words for a table cell, where the full message does not fit.
    pub fn label(&self) -> &'static str {
        match self {
            Problem::NotEnglish => "needs an English language",
            Problem::ModelMissing { .. } => "model not installed",
            Problem::ModelInvalid { .. } => "model does not load",
        }
    }

    /// Message and fix as one line, for places with a single text slot.
    pub fn summary(&self) -> String {
        format!("{}. {}", self.message(), self.fix())
    }
}

/// Whether `language` is an explicit English tag (`en`, `en-US`, `en_GB`).
fn is_english(language: Option<&str>) -> bool {
    language.and_then(|l| l.split(['-', '_']).next()) == Some("en")
}

/// The model file a text config resolves to and whether the user chose it.
fn resolved_model_path(text: &TextConfig) -> (PathBuf, bool) {
    match &text.lard_model_path {
        Some(path) => (path.clone(), true),
        None => (model_path(), false),
    }
}

/// Load the tagger the way the daemon will, for a config that has the stage
/// on. `Err` says exactly why the stage would be skipped.
pub fn load_tagger(text: &TextConfig, language: Option<&str>) -> Result<DisfluencyTagger, Problem> {
    if !is_english(language) {
        return Err(Problem::NotEnglish);
    }
    let (path, custom) = resolved_model_path(text);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Problem::ModelMissing { path, custom });
        }
        Err(error) => {
            return Err(Problem::ModelInvalid {
                path,
                error: error.to_string(),
            });
        }
    };
    DisfluencyTagger::from_bytes(&bytes).map_err(|error| Problem::ModelInvalid {
        path,
        error: error.to_string(),
    })
}

/// Why the stage is not running for `text` with the given engine language,
/// or `None` when it is off or working.
pub fn problem_for(text: &TextConfig, language: Option<&str>) -> Option<Problem> {
    if !text.lard_disfluency {
        return None;
    }
    load_tagger(text, language).err()
}

/// [`problem_for`] for a whole configuration.
pub fn problem(config: &Config) -> Option<Problem> {
    problem_for(&config.text, config.active_language())
}

fn validate(bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != MODEL_SIZE || format!("{:x}", Sha256::digest(bytes)) != MODEL_SHA256 {
        bail!("LARD model checksum mismatch; use the pinned lard-disfluency-v1.vxdf artifact");
    }
    crate::text::disfluency::DisfluencyTagger::from_bytes(bytes)?;
    Ok(())
}

fn install(bytes: &[u8], destination: &Path) -> Result<()> {
    validate(bytes)?;
    let parent = destination
        .parent()
        .context("LARD model path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.persist(destination)
        .context("Cannot install LARD model")?;
    Ok(())
}

/// What `voxtype setup lard --status` prints: the installed model, then
/// whether the stage is actually running for this configuration.
fn status_lines(installed: Result<(), String>, destination: &Path, config: &Config) -> Vec<String> {
    let mut lines = Vec::new();
    match installed {
        Ok(()) => lines.push(format!(
            "LARD model installed and verified: {}",
            destination.display()
        )),
        Err(error) => {
            lines.push(format!(
                "LARD model unavailable: {error}. Install with voxtype setup lard --source <model.vxdf>"
            ));
            lines.push(format!(
                "The model cannot be downloaded yet; it ships in the voxtype source as {SOURCE_TREE_PATH}."
            ));
        }
    }
    match (config.text.lard_disfluency, problem(config)) {
        (false, _) => lines.push(
            "text.lard_disfluency is off. Enable with: voxtype config set text.lard_disfluency true"
                .to_string(),
        ),
        (true, None) => {
            lines.push("text.lard_disfluency is on and the cleanup stage is running.".to_string())
        }
        (true, Some(problem)) => {
            lines.push(format!("{}.", problem.message()));
            lines.push(format!("  Fix: {}", problem.fix()));
        }
    }
    lines
}

/// No URL is enabled implicitly until model publication has been approved.
/// An explicit URL is still pinned to the exact release bytes, not arbitrary weights.
pub fn setup(
    source: Option<&Path>,
    url: Option<&str>,
    status: bool,
    config: &Config,
) -> Result<()> {
    let destination = model_path();
    if status {
        let installed = std::fs::read(&destination)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| validate(&bytes))
            .map_err(|error| error.to_string());
        for line in status_lines(installed, &destination, config) {
            println!("{line}");
        }
        return Ok(());
    }
    let bytes = if let Some(source) = source {
        std::fs::read(source).context("Cannot read LARD model source")?
    } else if let Some(url) = url {
        if !url.starts_with("https://") {
            bail!("LARD model downloads require an explicit HTTPS URL");
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(60))
            .build();
        let response = agent
            .get(url)
            .call()
            .context("LARD download failed; local cleanup remains available")?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MODEL_SIZE + 1)
            .read_to_end(&mut bytes)?;
        bytes
    } else {
        bail!(
            "The LARD model cannot be downloaded yet: publication is pending approval.\n  \
             It ships in the voxtype source as {SOURCE_TREE_PATH}. Install it with:\n    \
             voxtype setup lard --source <path to lard-disfluency-v1.vxdf>\n  \
             (or --url <approved-https-url>). Pinned SHA-256: {MODEL_SHA256}\n  \
             Until it is installed, text.lard_disfluency = true has no effect."
        );
    };
    install(&bytes, &destination)?;
    println!("LARD model installed: {}.", destination.display());
    if config.text.lard_disfluency && config.text.lard_model_path.is_none() {
        match problem(config) {
            None => println!(
                "text.lard_disfluency is already on. Restart voxtype to load the model: systemctl --user restart voxtype"
            ),
            Some(problem) => println!("{}.\n  Fix: {}", problem.message(), problem.fix()),
        }
    } else if config.text.lard_disfluency {
        println!(
            "text.lard_model_path is set, so the daemon reads that file, not this one. Remove it to use the installed model: voxtype config unset text.lard_model_path"
        );
    } else {
        println!("Enable with voxtype config set text.lard_disfluency true (English only).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_model() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(SOURCE_TREE_PATH)
    }

    fn text_config(enabled: bool, model: Option<&Path>) -> TextConfig {
        TextConfig {
            lard_disfluency: enabled,
            lard_model_path: model.map(Path::to_path_buf),
            ..Default::default()
        }
    }

    #[test]
    fn pinned_install_rejects_corrupt_model_without_replacing_good_copy() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.vxdf");
        let model = include_bytes!("../../assets/models/lard-disfluency-v1.vxdf");
        install(model, &destination).unwrap();
        assert!(install(b"corrupt", &destination).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), model);
    }

    /// The fix text sends people to this path, so it has to be the pinned
    /// artifact and not just a file that happens to exist.
    #[test]
    fn source_tree_path_is_the_pinned_model() {
        let bytes = std::fs::read(repo_model()).unwrap();
        validate(&bytes).unwrap();
    }

    #[test]
    fn no_problem_when_the_stage_is_off_or_working() {
        let missing = Path::new("/nonexistent/voxtype-test/lard.vxdf");
        assert_eq!(problem_for(&text_config(false, Some(missing)), None), None);
        assert_eq!(
            problem_for(&text_config(true, Some(&repo_model())), Some("en-US")),
            None
        );
    }

    #[test]
    fn missing_model_is_reported_with_its_path() {
        let missing = Path::new("/nonexistent/voxtype-test/lard.vxdf");
        assert_eq!(
            problem_for(&text_config(true, Some(missing)), Some("en")),
            Some(Problem::ModelMissing {
                path: missing.to_path_buf(),
                custom: true,
            })
        );
    }

    #[test]
    fn unloadable_model_is_reported_as_invalid() {
        let corrupt = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(corrupt.path(), b"not a model").unwrap();
        assert!(matches!(
            problem_for(&text_config(true, Some(corrupt.path())), Some("en")),
            Some(Problem::ModelInvalid { .. })
        ));
    }

    #[test]
    fn language_is_checked_before_the_model() {
        for language in [None, Some("fr"), Some("auto")] {
            assert_eq!(
                problem_for(&text_config(true, Some(&repo_model())), language),
                Some(Problem::NotEnglish),
                "{language:?}"
            );
        }
    }

    /// The model is not published. Whatever the fix says, it must not point
    /// at a download, and it must offer the way out that always works.
    #[test]
    fn fixes_never_promise_a_download() {
        let path = PathBuf::from("/data/voxtype/models/lard-disfluency-v1.vxdf");
        let problems = [
            Problem::NotEnglish,
            Problem::ModelMissing {
                path: path.clone(),
                custom: false,
            },
            Problem::ModelMissing {
                path: path.clone(),
                custom: true,
            },
            Problem::ModelInvalid {
                path: path.clone(),
                error: "unsupported disfluency model format".into(),
            },
        ];
        for problem in &problems {
            let fix = problem.fix();
            assert!(!fix.contains("http"), "{fix}");
            assert!(
                fix.contains("voxtype config set text.lard_disfluency false"),
                "{fix}"
            );
            assert!(problem
                .message()
                .starts_with("LARD disfluency cleanup is on"));
            assert!(problem.summary().contains(&fix));
            // One line each: these go into a notification and a QML label.
            assert!(!problem.summary().contains("  "), "{}", problem.summary());
            assert!(!problem.summary().contains('\n'));
        }

        let default_missing = &problems[1];
        assert!(default_missing.message().contains("not installed"));
        assert!(default_missing.message().contains(path.to_str().unwrap()));
        assert!(default_missing.fix().contains("cannot be downloaded yet"));
        assert!(default_missing.fix().contains(SOURCE_TREE_PATH));
        assert!(default_missing
            .fix()
            .contains("voxtype setup lard --source"));

        // A path the user set is theirs to correct, not ours to reinstall.
        assert!(problems[2].fix().contains("lard_model_path"));
        assert!(!problems[2].fix().contains("setup lard --source"));
    }

    #[test]
    fn status_reports_the_setting_as_well_as_the_file() {
        let destination = Path::new("/data/voxtype/models/lard-disfluency-v1.vxdf");
        let missing = Path::new("/nonexistent/voxtype-test/lard.vxdf");

        let mut config = Config::default();
        let off = status_lines(Err("No such file".into()), destination, &config).join("\n");
        assert!(off.contains("LARD model unavailable: No such file"));
        assert!(off.contains("cannot be downloaded yet"));
        assert!(off.contains("text.lard_disfluency is off"));

        // On with nothing to load: the status says so instead of only
        // describing the file.
        config.text = text_config(true, Some(missing));
        let broken = status_lines(Err("No such file".into()), destination, &config).join("\n");
        assert!(broken.contains("LARD disfluency cleanup is on but"));
        assert!(broken.contains("Fix: "));

        // Installed is not the same as running: the language still decides.
        config.text = text_config(true, Some(&repo_model()));
        config.whisper.language = crate::config::LanguageConfig::Single("en".into());
        let running = status_lines(Ok(()), destination, &config).join("\n");
        assert!(running.contains("LARD model installed and verified"));
        assert!(running.contains("the cleanup stage is running"));

        config.whisper.language = crate::config::LanguageConfig::Single("auto".into());
        let skipped = status_lines(Ok(()), destination, &config).join("\n");
        assert!(skipped.contains("LARD model installed and verified"));
        assert!(skipped.contains("only runs when the engine language is set to English"));
    }
}
