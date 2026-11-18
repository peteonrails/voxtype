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

/// The pinned model as it sits in the source tree. `voxtype setup lard`
/// downloads the same bytes from the mirror; tests load this copy, and
/// `--source` installs from it without a network.
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
    /// The dictation is not known to be English, for the reason given.
    NotEnglish(NotEnglish),
    /// No file at the model path. `custom` says the path came from
    /// `text.lard_model_path` rather than the default install location.
    ModelMissing { path: PathBuf, custom: bool },
    /// Something is at the model path but it does not load as a VXDF model.
    ModelInvalid {
        path: PathBuf,
        error: String,
        custom: bool,
    },
}

/// Why a configuration's dictation does not count as English, which decides
/// what the user is told to change. See [`Config::text_language`] for the
/// rule these mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotEnglish {
    /// Nothing says what language the text is in: a multilingual or unknown
    /// model on automatic detection, and no `text.language`. Declaring the
    /// language is the fix.
    Undeclared { engine: &'static str, model: String },
    /// The engine's own language key names another language.
    Engine { key: &'static str, language: String },
    /// The engine's detection is limited to a set without English, so a
    /// declared `en` would contradict it.
    EngineSet {
        key: &'static str,
        languages: Vec<String>,
    },
    /// The selected model only writes another language.
    Model { model: String, language: String },
    /// `text.language` declares another language.
    Declared { language: String },
    /// `text.language` holds something that is not a language code.
    Malformed { value: String },
    /// No configuration to explain it with: only the language a text
    /// processor was built for.
    Unexplained { language: Option<String> },
}

const ONLY_ENGLISH: &str =
    "LARD disfluency cleanup is on but is being skipped: it only runs on English, and";
const DECLARE: &str = "voxtype config set text.language en";

impl NotEnglish {
    /// Classify a configuration whose [`Config::text_language`] is not
    /// English.
    fn of(config: &Config) -> Self {
        use crate::config::EngineLanguage;
        let key = config.engine_language_key();
        match config.engine_language() {
            EngineLanguage::Fixed(language) => {
                return NotEnglish::Engine {
                    key,
                    language: language.to_string(),
                }
            }
            EngineLanguage::OneOf(languages) if !languages.iter().any(|l| is_english(Some(l))) => {
                return NotEnglish::EngineSet {
                    key,
                    languages: languages.into_iter().map(str::to_string).collect(),
                }
            }
            EngineLanguage::OneOf(_) | EngineLanguage::Open => {}
        }
        if let Some(language) = config.model_language() {
            return NotEnglish::Model {
                model: config.model_name().to_string(),
                language: language.to_string(),
            };
        }
        if config.text.language_is_malformed() {
            return NotEnglish::Malformed {
                value: config.text.language.clone(),
            };
        }
        match config.text.declared_language() {
            Some(language) => NotEnglish::Declared {
                language: language.to_string(),
            },
            None => NotEnglish::Undeclared {
                engine: config.engine.name(),
                model: config.model_name().to_string(),
            },
        }
    }

    fn message(&self) -> String {
        match self {
            NotEnglish::Undeclared { engine, model } => {
                format!("{ONLY_ENGLISH} nothing says {engine} dictation with {model} is English")
            }
            NotEnglish::Engine { key, language } => {
                format!("{ONLY_ENGLISH} {key} is {language}")
            }
            NotEnglish::EngineSet { key, languages } => format!(
                "{ONLY_ENGLISH} {key} is limited to {}",
                languages.join(", ")
            ),
            NotEnglish::Model { model, language } => {
                format!("{ONLY_ENGLISH} {model} only writes {language}")
            }
            NotEnglish::Declared { language } => {
                format!("{ONLY_ENGLISH} text.language is {language}")
            }
            NotEnglish::Malformed { value } => {
                format!("{ONLY_ENGLISH} text.language is \"{value}\", which is not a language code")
            }
            NotEnglish::Unexplained {
                language: Some(language),
            } => format!("{ONLY_ENGLISH} the dictation language is {language}"),
            NotEnglish::Unexplained { language: None } => {
                format!("{ONLY_ENGLISH} the dictation language is not known")
            }
        }
    }

    /// The change that gets the stage running, for someone who does dictate
    /// in English. The off switch is appended by [`Problem::fix`].
    fn fix(&self) -> String {
        match self {
            NotEnglish::Undeclared { .. } | NotEnglish::Unexplained { .. } => {
                format!("If you dictate in English, declare it: {DECLARE}")
            }
            NotEnglish::Engine { key, .. } => {
                format!("If you dictate in English, change it: voxtype config set {key} en")
            }
            NotEnglish::EngineSet { key, .. } => {
                format!("If you dictate in English, add en to {key}")
            }
            NotEnglish::Model { .. } => {
                "Choose a model that writes English to use this stage".to_string()
            }
            NotEnglish::Declared { .. } => {
                format!("If you dictate in English, change it: {DECLARE}")
            }
            NotEnglish::Malformed { .. } => format!("Use a language code: {DECLARE}"),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            NotEnglish::Undeclared { .. } | NotEnglish::Unexplained { language: None } => {
                "language not declared"
            }
            NotEnglish::Malformed { .. } => "text.language is not a code",
            _ => "language is not English",
        }
    }
}

impl Problem {
    /// What is wrong, as one sentence without a trailing period.
    pub fn message(&self) -> String {
        match self {
            Problem::NotEnglish(why) => why.message(),
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
            Problem::ModelInvalid { path, error, .. } => format!(
                "LARD disfluency cleanup is on but its model does not load ({}: {error})",
                path.display()
            ),
        }
    }

    /// What to do about it: the change that gets the stage running (the
    /// exact command where there is one), and always the switch that turns
    /// the stage off. A path the user set is theirs to correct, because
    /// `voxtype setup lard` installs to the default one.
    pub fn fix(&self) -> String {
        match self {
            Problem::NotEnglish(why) => format!("{}, {TURN_OFF}", why.fix()),
            Problem::ModelMissing { custom: true, .. }
            | Problem::ModelInvalid { custom: true, .. } => format!(
                "Correct [text] lard_model_path, or remove it to use the model \
                 installed by voxtype setup lard, {TURN_OFF}"
            ),
            Problem::ModelMissing { custom: false, .. } => {
                format!("Run: voxtype setup lard, {TURN_OFF}")
            }
            Problem::ModelInvalid { custom: false, .. } => {
                format!("Replace it: voxtype setup lard, {TURN_OFF}")
            }
        }
    }

    /// A few words for a table cell, where the full message does not fit.
    pub fn label(&self) -> &'static str {
        match self {
            Problem::NotEnglish(why) => why.label(),
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
pub fn is_english(language: Option<&str>) -> bool {
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
/// on. `language` is what [`Config::text_language`] resolved to. `Err` says
/// why the stage would be skipped; [`problem`] says it in terms of the whole
/// configuration.
pub fn load_tagger(text: &TextConfig, language: Option<&str>) -> Result<DisfluencyTagger, Problem> {
    if !is_english(language) {
        return Err(Problem::NotEnglish(NotEnglish::Unexplained {
            language: language.map(str::to_string),
        }));
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
                custom,
            });
        }
    };
    DisfluencyTagger::from_bytes(&bytes).map_err(|error| Problem::ModelInvalid {
        path,
        error: error.to_string(),
        custom,
    })
}

/// Why the stage is not running for `text` when the dictation language is
/// `language`, or `None` when it is off or working.
pub fn problem_for(text: &TextConfig, language: Option<&str>) -> Option<Problem> {
    if !text.lard_disfluency {
        return None;
    }
    load_tagger(text, language).err()
}

/// Why the stage is not running for a whole configuration, or `None` when it
/// is off or working. A language problem names what in this configuration
/// makes the dictation not English, so the fix is the user's own.
pub fn problem(config: &Config) -> Option<Problem> {
    match problem_for(&config.text, config.text_language())? {
        Problem::NotEnglish(_) => Some(Problem::NotEnglish(NotEnglish::of(config))),
        other => Some(other),
    }
}

fn validate(bytes: &[u8]) -> Result<()> {
    let digest = format!("{:x}", Sha256::digest(bytes));
    if bytes.len() as u64 != MODEL_SIZE || digest != MODEL_SHA256 {
        bail!(
            "LARD model checksum mismatch: got {} bytes with SHA-256 {digest}, \
             expected {MODEL_SIZE} bytes with SHA-256 {MODEL_SHA256}",
            bytes.len()
        );
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

/// Where `voxtype setup lard` downloads the model from: the same mirror
/// every other model comes from.
pub fn model_url() -> String {
    format!(
        "{}/text/lard-disfluency-v1.vxdf",
        super::manifest::MODELS_BASE_URL
    )
}

/// The model card published beside the model: provenance, attribution and
/// the CC BY-SA 4.0 terms.
pub fn model_card_url() -> String {
    format!(
        "{}/text/lard-disfluency-v1.MODEL_CARD.md",
        super::manifest::MODELS_BASE_URL
    )
}

/// What every failed install ends with, so nobody has to wonder whether a
/// working model was damaged.
const UNTOUCHED: &str = "Nothing was installed and any existing model is untouched";

/// Fetch the model bytes. Each failure says what happened: an HTTP status,
/// an unreachable host, or a body that stopped early.
fn download(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => {
            bail!("LARD model download failed: {url} answered HTTP {code}. {UNTOUCHED}.")
        }
        Err(error) => bail!(
            "LARD model download failed: could not reach {url} ({error}). {UNTOUCHED}. \
             Check your connection and run voxtype setup lard again."
        ),
    };
    let mut bytes = Vec::new();
    // One byte past the pinned size is enough to know a body is too long.
    response
        .into_reader()
        .take(MODEL_SIZE + 1)
        .read_to_end(&mut bytes)
        .with_context(|| {
            format!("LARD model download from {url} stopped before the end. {UNTOUCHED}.")
        })?;
    Ok(bytes)
}

/// Download from `url` and install at `destination`, replacing it only with
/// bytes that match the pinned checksum.
fn install_from_url(url: &str, destination: &Path) -> Result<()> {
    let bytes = download(url)?;
    install(&bytes, destination)
        .with_context(|| format!("{url} did not serve the pinned LARD model. {UNTOUCHED}"))
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
                "LARD model unavailable: {error}. Download it with: voxtype setup lard"
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

/// `voxtype setup lard`: download the model from the mirror, or install it
/// from a file or another URL. Every route installs only the pinned bytes.
/// Nothing here runs at daemon start: installing is always this command.
pub fn setup(
    source: Option<&Path>,
    url: Option<&str>,
    status: bool,
    config: &Config,
) -> Result<()> {
    let destination = model_path();
    let installed = || {
        std::fs::read(&destination)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| validate(&bytes))
    };
    if status {
        let installed = installed().map_err(|error| error.to_string());
        for line in status_lines(installed, &destination, config) {
            println!("{line}");
        }
        return Ok(());
    }
    if let Some(source) = source {
        let bytes = std::fs::read(source).context("Cannot read LARD model source")?;
        install(&bytes, &destination).with_context(|| {
            format!(
                "{} is not the pinned LARD model. {UNTOUCHED}",
                source.display()
            )
        })?;
    } else if let Some(url) = url {
        if !url.starts_with("https://") {
            bail!("LARD model downloads require an HTTPS URL");
        }
        install_from_url(url, &destination)?;
    } else if installed().is_ok() {
        println!(
            "LARD model already installed and verified: {}",
            destination.display()
        );
        for line in next_step_lines(config) {
            println!("{line}");
        }
        return Ok(());
    } else {
        let url = model_url();
        println!("Downloading the LARD disfluency model (64 KB) from {url}");
        install_from_url(&url, &destination)?;
    }
    println!("LARD model installed: {}.", destination.display());
    println!(
        "Model license: CC BY-SA 4.0, separate from Voxtype's MIT code. Model card: {}",
        model_card_url()
    );
    for line in next_step_lines(config) {
        println!("{line}");
    }
    Ok(())
}

/// What to do once the model is in place, for this configuration.
fn next_step_lines(config: &Config) -> Vec<String> {
    if !config.text.lard_disfluency {
        return vec![
            "Enable with voxtype config set text.lard_disfluency true (English only).".to_string(),
        ];
    }
    if config.text.lard_model_path.is_some() {
        return vec![
            "text.lard_model_path is set, so the daemon reads that file, not this one. Remove it to use the installed model: voxtype config unset text.lard_model_path".to_string(),
        ];
    }
    match problem(config) {
        None => vec![
            "text.lard_disfluency is already on. Restart voxtype to load the model: systemctl --user restart voxtype".to_string(),
        ],
        Some(problem) => vec![
            format!("{}.", problem.message()),
            format!("  Fix: {}", problem.fix()),
        ],
    }
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

    /// Answer one HTTP request on loopback with `status` and `body`, then
    /// close. Returns the URL to fetch. The test sandbox has no network, and a
    /// download test must never depend on the real mirror being up.
    fn serve_once(status: &'static str, body: Vec<u8>) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/text/lard-disfluency-v1.vxdf",
            listener.local_addr().unwrap()
        );
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Read the request head so the client is not reset mid-send.
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        });
        url
    }

    /// A models directory that already holds a good model, to prove a failed
    /// download leaves it alone.
    fn installed_model() -> (tempfile::TempDir, PathBuf, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("lard-disfluency-v1.vxdf");
        let model = std::fs::read(repo_model()).unwrap();
        install(&model, &destination).unwrap();
        (dir, destination, model)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn default_download_comes_from_the_model_mirror() {
        assert_eq!(
            model_url(),
            "https://models.voxtype.io/text/lard-disfluency-v1.vxdf"
        );
        assert!(model_url().starts_with(crate::setup::manifest::MODELS_BASE_URL));
        assert!(model_card_url().starts_with(crate::setup::manifest::MODELS_BASE_URL));
    }

    #[test]
    fn download_installs_the_pinned_model() {
        let dir = tempfile::tempdir().unwrap();
        // The models directory may not exist yet on a fresh install.
        let destination = dir.path().join("models/lard-disfluency-v1.vxdf");
        let model = std::fs::read(repo_model()).unwrap();
        let url = serve_once("200 OK", model.clone());

        install_from_url(&url, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), model);
    }

    #[test]
    fn download_with_the_wrong_bytes_is_refused_and_named() {
        let (dir, destination, model) = installed_model();
        // Right size, wrong content: only the checksum can tell.
        let mut tampered = model.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xff;
        let url = serve_once("200 OK", tampered);

        let error = install_from_url(&url, &destination).unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("did not serve the pinned LARD model"),
            "{text}"
        );
        assert!(text.contains("checksum mismatch"), "{text}");
        assert!(text.contains(MODEL_SHA256), "{text}");
        assert!(text.contains("any existing model is untouched"), "{text}");
        assert_eq!(std::fs::read(&destination).unwrap(), model);
        assert_eq!(files_in(dir.path()), ["lard-disfluency-v1.vxdf"]);
    }

    #[test]
    fn download_of_an_error_page_is_refused_whatever_its_size() {
        let (dir, destination, model) = installed_model();
        for body in [b"<html>captive portal</html>".to_vec(), vec![0u8; 200_000]] {
            let url = serve_once("200 OK", body);
            let error = install_from_url(&url, &destination).unwrap_err();
            assert!(format!("{error:#}").contains("checksum mismatch"));
            assert_eq!(std::fs::read(&destination).unwrap(), model);
        }
        assert_eq!(files_in(dir.path()), ["lard-disfluency-v1.vxdf"]);
    }

    #[test]
    fn http_error_is_reported_with_its_status() {
        let (dir, destination, model) = installed_model();
        let url = serve_once("404 Not Found", b"not here".to_vec());

        let error = install_from_url(&url, &destination).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("answered HTTP 404"), "{text}");
        assert!(text.contains(&url), "{text}");
        assert!(text.contains("any existing model is untouched"), "{text}");
        assert_eq!(std::fs::read(&destination).unwrap(), model);
        assert_eq!(files_in(dir.path()), ["lard-disfluency-v1.vxdf"]);

        // And with nothing installed before, nothing is left behind.
        let empty = tempfile::tempdir().unwrap();
        let fresh = empty.path().join("lard-disfluency-v1.vxdf");
        let url = serve_once("503 Service Unavailable", Vec::new());
        let error = install_from_url(&url, &fresh).unwrap_err();
        assert!(format!("{error:#}").contains("answered HTTP 503"));
        assert!(files_in(empty.path()).is_empty());
    }

    #[test]
    fn unreachable_host_is_reported_as_a_network_error() {
        let (_dir, destination, model) = installed_model();
        // Bind, note the port, and close it: nothing is listening there.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.vxdf", closed.local_addr().unwrap());
        drop(closed);

        let error = install_from_url(&url, &destination).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("could not reach"), "{text}");
        assert!(text.contains("any existing model is untouched"), "{text}");
        assert_eq!(std::fs::read(&destination).unwrap(), model);
    }

    #[test]
    fn a_local_file_that_is_not_the_model_names_the_mismatch() {
        let error = validate(b"corrupt").unwrap_err().to_string();
        assert!(error.contains("got 7 bytes"), "{error}");
        assert!(error.contains(MODEL_SHA256), "{error}");
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
        let missing = Path::new("/nonexistent/voxtype-test/lard.vxdf");
        for model in [repo_model().as_path(), missing] {
            for language in [None, Some("fr"), Some("auto")] {
                assert_eq!(
                    problem_for(&text_config(true, Some(model)), language),
                    Some(Problem::NotEnglish(NotEnglish::Unexplained {
                        language: language.map(str::to_string),
                    })),
                    "{language:?}"
                );
            }
        }
    }

    /// Every fix is a command the user can run as written, and every one
    /// offers the way out that always works.
    #[test]
    fn fixes_name_the_setup_command_and_the_off_switch() {
        let path = PathBuf::from("/data/voxtype/models/lard-disfluency-v1.vxdf");
        let invalid = |custom| Problem::ModelInvalid {
            path: path.clone(),
            error: "unsupported disfluency model format".into(),
            custom,
        };
        let missing = |custom| Problem::ModelMissing {
            path: path.clone(),
            custom,
        };
        let off = "or turn it off: voxtype config set text.lard_disfluency false";

        let not_english = [
            NotEnglish::Undeclared {
                engine: "parakeet",
                model: "parakeet-tdt-0.6b-v3".into(),
            },
            NotEnglish::Engine {
                key: "whisper.language",
                language: "fr".into(),
            },
            NotEnglish::EngineSet {
                key: "whisper.language",
                languages: vec!["fr".into(), "de".into()],
            },
            NotEnglish::Model {
                model: "base-ja".into(),
                language: "ja".into(),
            },
            NotEnglish::Declared {
                language: "fr".into(),
            },
            NotEnglish::Malformed {
                value: "english".into(),
            },
            NotEnglish::Unexplained { language: None },
            NotEnglish::Unexplained {
                language: Some("fr".into()),
            },
        ]
        .map(Problem::NotEnglish);

        for problem in not_english.into_iter().chain([
            missing(false),
            missing(true),
            invalid(false),
            invalid(true),
        ]) {
            let fix = problem.fix();
            assert!(fix.ends_with(off), "{fix}");
            // The command is the fix. A URL in a notification is not.
            assert!(!fix.contains("http"), "{fix}");
            assert!(problem
                .message()
                .starts_with("LARD disfluency cleanup is on"));
            assert!(problem.summary().contains(&fix));
            // One line each: these go into a notification and a QML label.
            assert!(!problem.summary().contains("  "), "{}", problem.summary());
            assert!(!problem.summary().contains('\n'));
        }

        assert!(missing(false).message().contains("not installed"));
        assert!(missing(false).message().contains(path.to_str().unwrap()));
        assert_eq!(
            missing(false).fix(),
            format!("Run: voxtype setup lard, {off}")
        );
        assert_eq!(
            invalid(false).fix(),
            format!("Replace it: voxtype setup lard, {off}")
        );

        // A path the user set is theirs to correct: setup lard installs to
        // the default location, which that config does not read.
        for custom in [missing(true), invalid(true)] {
            assert!(custom.fix().starts_with("Correct [text] lard_model_path"));
            assert!(!custom.fix().contains("Run: voxtype setup lard"));
        }
    }

    #[test]
    fn status_reports_the_setting_as_well_as_the_file() {
        let destination = Path::new("/data/voxtype/models/lard-disfluency-v1.vxdf");
        let missing = Path::new("/nonexistent/voxtype-test/lard.vxdf");

        let mut config = Config::default();
        let off = status_lines(Err("No such file".into()), destination, &config).join("\n");
        assert!(off.contains(
            "LARD model unavailable: No such file. Download it with: voxtype setup lard"
        ));
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

        config.whisper.model = "large-v3".into();
        config.whisper.language = crate::config::LanguageConfig::Single("auto".into());
        let skipped = status_lines(Ok(()), destination, &config).join("\n");
        assert!(skipped.contains("LARD model installed and verified"));
        assert!(skipped.contains("it only runs on English"));
        assert!(skipped.contains(
            "Fix: If you dictate in English, declare it: voxtype config set text.language en"
        ));
    }
    fn lard_on(mut config: Config) -> Config {
        config.text = TextConfig {
            language: config.text.language.clone(),
            ..text_config(true, Some(&repo_model()))
        };
        config
    }

    fn engine_config(engine: crate::config::TranscriptionEngine, model: &str) -> Config {
        let mut config = Config::default();
        config.set_model_for(engine, model);
        lard_on(config)
    }

    /// What doctor, the startup notification, `config set`, the panel and
    /// `voxtype configure` all print: the reason in this configuration, and
    /// the change that fixes it, ending with the off switch.
    #[test]
    fn a_language_problem_names_the_cause_and_the_exact_fix() {
        use crate::config::TranscriptionEngine::*;
        const SKIPPED: &str =
            "LARD disfluency cleanup is on but is being skipped: it only runs on English, and";
        const OFF: &str = "or turn it off: voxtype config set text.lard_disfluency false";
        let report = |config: &Config| {
            let problem = problem(config).expect("a problem");
            assert!(matches!(problem, Problem::NotEnglish(_)), "{problem:?}");
            (problem.message(), problem.fix())
        };

        // A multilingual model on automatic detection: declare the language.
        for (engine, model) in [
            (Parakeet, "parakeet-tdt-0.6b-v3"),
            (Dolphin, "dolphin-base"),
            (Omnilingual, "omnilingual-300m"),
            (Paraformer, "paraformer-zh"),
            (Soniox, "stt-rt-v4"),
        ] {
            let (message, fix) = report(&engine_config(engine, model));
            assert_eq!(
                message,
                format!(
                    "{SKIPPED} nothing says {} dictation with {model} is English",
                    engine.name()
                )
            );
            assert_eq!(
                fix,
                format!(
                    "If you dictate in English, declare it: \
                     voxtype config set text.language en, {OFF}"
                )
            );
        }
        // Engines with a language key on automatic get the same one answer.
        let mut whisper = engine_config(Whisper, "large-v3-turbo");
        whisper.whisper.language = crate::config::LanguageConfig::Single("auto".into());
        assert!(report(&whisper).1.starts_with(
            "If you dictate in English, declare it: voxtype config set text.language en,"
        ));

        // An English-only model: nothing to do, so nothing is reported.
        for (engine, model) in [
            (Parakeet, "parakeet-tdt-0.6b-v2"),
            (Parakeet, "parakeet-unified-en-0.6b"),
            (Moonshine, "base"),
            (Paraformer, "paraformer-en"),
        ] {
            assert_eq!(problem(&engine_config(engine, model)), None, "{model}");
        }
        // And following the advice clears it.
        let mut declared = engine_config(Parakeet, "parakeet-tdt-0.6b-v3");
        declared.text.language = "en".into();
        assert_eq!(problem(&declared), None);

        // The engine was told another language: that key is the one to change.
        let mut french = engine_config(Whisper, "large-v3-turbo");
        french.whisper.language = crate::config::LanguageConfig::Single("fr".into());
        french.text.language = "en".into();
        assert_eq!(
            report(&french),
            (
                format!("{SKIPPED} whisper.language is fr"),
                format!(
                    "If you dictate in English, change it: \
                     voxtype config set whisper.language en, {OFF}"
                )
            )
        );

        // Detection limited to a set without English.
        let mut limited = engine_config(Whisper, "large-v3-turbo");
        limited.whisper.language =
            crate::config::LanguageConfig::Multiple(vec!["fr".into(), "de".into()]);
        limited.text.language = "en".into();
        assert_eq!(
            report(&limited),
            (
                format!("{SKIPPED} whisper.language is limited to fr, de"),
                format!("If you dictate in English, add en to whisper.language, {OFF}")
            )
        );

        // A model that only writes another language: no setting helps.
        let mut japanese = engine_config(Moonshine, "base-ja");
        japanese.text.language = "en".into();
        assert_eq!(
            report(&japanese),
            (
                format!("{SKIPPED} base-ja only writes ja"),
                format!("Choose a model that writes English to use this stage, {OFF}")
            )
        );

        // Another language declared, and a value that is not a code.
        let mut portuguese = engine_config(Parakeet, "parakeet-tdt-0.6b-v3");
        portuguese.text.language = "pt-BR".into();
        assert_eq!(
            report(&portuguese),
            (
                format!("{SKIPPED} text.language is pt-BR"),
                format!(
                    "If you dictate in English, change it: \
                     voxtype config set text.language en, {OFF}"
                )
            )
        );
        let mut typo = engine_config(Parakeet, "parakeet-tdt-0.6b-v3");
        typo.text.language = "english".into();
        assert_eq!(
            report(&typo),
            (
                format!("{SKIPPED} text.language is \"english\", which is not a language code"),
                format!("Use a language code: voxtype config set text.language en, {OFF}")
            )
        );

        // The model is still checked once the language is right.
        let mut no_model = engine_config(Parakeet, "parakeet-tdt-0.6b-v2");
        no_model.text.lard_model_path = Some("/nonexistent/voxtype-test/lard.vxdf".into());
        assert!(matches!(
            problem(&no_model),
            Some(Problem::ModelMissing { custom: true, .. })
        ));
    }
}
