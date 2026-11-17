//! Accept, decline and clean up model upgrade offers.
//!
//! Accepting is a transaction over `config.toml`: the file is snapshotted
//! before anything writes it and restored byte for byte if any later step
//! fails (activation, the restart check, or the restarted daemon not coming
//! up). Downloads and checksum checks run before the snapshot, so a failure
//! there never touches the config at all. The old model's files are kept.
//!
//! Side effects go through [`MigrateEnv`], so tests drive every failure path
//! with a fake downloader and daemon, without network or systemd.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{
    contained_model_path, model_location, Accepted, Decline, Offer, UpgradeState, UpgradeStatus,
};
use crate::config::Config;

/// A failed migrate action: the message to print and the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateError {
    pub message: String,
    /// 2 for a command the user can fix (unknown offer id), 1 otherwise.
    pub code: i32,
}

impl MigrateError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MigrateError {}

/// Where an action reads and writes.
#[derive(Debug, Clone)]
pub struct MigratePaths {
    pub config: PathBuf,
    pub models_dir: PathBuf,
    pub state: PathBuf,
}

/// Every side effect accepting an offer has.
pub trait MigrateEnv {
    /// Download a catalog GGUF into the models directory and verify its
    /// sha256 before it replaces anything.
    fn download(&mut self, model: &str) -> anyhow::Result<()>;
    /// Verify the sha256 of a catalog GGUF already on disk.
    fn verify(&mut self, model: &str) -> anyhow::Result<()>;
    /// Called once the download is done and before `config.toml` is touched.
    /// From here to the end the transaction must run to completion or roll
    /// back, so a cancel (SIGTERM from a GUI, Ctrl-C) cannot stop it halfway.
    fn begin_commit(&mut self) {}
    /// Whether a daemon this command can restart is running.
    fn daemon_running(&mut self) -> bool;
    /// Refuse while the daemon is recording, transcribing or in a meeting.
    fn ensure_idle(&mut self) -> Result<(), String>;
    /// Switch engine and model in one validated config write.
    fn activate(&mut self, config: &Path, engine: &str, model: &str) -> Result<(), String>;
    /// Check the written config is one the daemon can start with.
    fn validate_saved(&mut self, config: &Path) -> Result<(), String>;
    /// Restart the daemon and wait until it is ready again.
    fn restart(&mut self) -> Result<(), String>;
}

/// What a successful accept did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptOutcome {
    pub engine: String,
    pub model: String,
    pub previous_model: String,
    pub downloaded: bool,
    pub restarted: bool,
}

fn unknown_offer(status: &UpgradeStatus, id: &str) -> MigrateError {
    let known: Vec<&str> = status
        .offers
        .iter()
        .map(|offer| offer.id.as_str())
        .collect();
    if known.is_empty() {
        MigrateError::usage(format!(
            "No model upgrade is offered for {} / {}, so there is no offer '{id}'.",
            status.current.engine, status.current.model
        ))
    } else {
        MigrateError::usage(format!(
            "Unknown offer '{id}'. Offers for {}: {}",
            status.current.model,
            known.join(", ")
        ))
    }
}

/// Look up an offer by id, with a message naming the valid ids.
pub fn find_offer<'a>(status: &'a UpgradeStatus, id: &str) -> Result<&'a Offer, MigrateError> {
    status.offer(id).ok_or_else(|| unknown_offer(status, id))
}

/// Restore `config` to exactly `snapshot`. `None` means the file did not
/// exist before, so it is removed.
fn restore_config(config: &Path, snapshot: &Option<Vec<u8>>) -> std::io::Result<()> {
    match snapshot {
        Some(bytes) => {
            let tmp = config.with_extension("toml.migrate-restore");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, config)
        }
        None => match std::fs::remove_file(config) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

fn unchanged(offer: &Offer) -> String {
    format!(
        "Your config is unchanged and the offer stays available. Retry with: voxtype setup migrate --accept {}",
        offer.id
    )
}

/// Download (if needed), verify, switch, restart, record. Any failure leaves
/// `config.toml` byte-identical and the offer in place.
pub fn accept(
    env: &mut dyn MigrateEnv,
    paths: &MigratePaths,
    status: &UpgradeStatus,
    offer_id: &str,
) -> Result<AcceptOutcome, MigrateError> {
    let offer = find_offer(status, offer_id)?;

    let mut downloaded = false;
    let ready = offer.download_bytes == 0 && env.verify(&offer.model).is_ok();
    if !ready {
        env.download(&offer.model).map_err(|error| {
            let error = format!("{error:#}");
            MigrateError::failed(format!(
                "Download of {} failed: {}. A partial download is discarded. {}",
                offer.model,
                error.trim_end_matches('.'),
                unchanged(offer)
            ))
        })?;
        downloaded = true;
    }

    env.begin_commit();
    let was_running = env.daemon_running();
    if was_running {
        env.ensure_idle().map_err(|error| {
            MigrateError::failed(format!(
                "{error} The model is downloaded. {}",
                unchanged(offer)
            ))
        })?;
    }

    let snapshot = match std::fs::read(&paths.config) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(MigrateError::failed(format!(
                "Cannot read {}: {error}. {}",
                paths.config.display(),
                unchanged(offer)
            )))
        }
    };
    let rollback = |reason: String| -> MigrateError {
        match restore_config(&paths.config, &snapshot) {
            Ok(()) => MigrateError::failed(format!("{reason} {}", unchanged(offer))),
            Err(error) => MigrateError::failed(format!(
                "{reason} Restoring {} also failed ({error}); check that file before restarting voxtype.",
                paths.config.display()
            )),
        }
    };

    env.activate(&paths.config, &offer.engine, &offer.model)
        .map_err(|error| rollback(format!("Could not switch to {}: {error}", offer.model)))?;
    env.validate_saved(&paths.config)
        .map_err(|error| rollback(format!("The new selection is not ready to start: {error}")))?;

    // Record the acceptance before the restart: the restarted daemon reads it
    // once at startup to know its first successful transcription unlocks
    // cleanup. Written after the restart, the new daemon would only ever see
    // the previous record. A failed restart puts the old record back.
    let state_snapshot = std::fs::read(&paths.state).ok();
    let mut state = UpgradeState::load(&paths.state);
    state.accepted = Some(Accepted {
        id: offer.id.clone(),
        engine: offer.engine.clone(),
        model: offer.model.clone(),
        previous_engine: status.current.engine.clone(),
        previous_model: status.current.model.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        first_success: false,
        cleaned: false,
    });
    if let Err(error) = state.save(&paths.state) {
        // Not fatal: the switch can still go ahead; only the cleanup offer is lost.
        tracing::warn!(%error, path = %paths.state.display(), "Could not record the accepted model upgrade");
    }

    if was_running {
        if let Err(error) = env.restart() {
            let error = rollback(format!(
                "voxtype did not start with {}: {error}",
                offer.model
            ));
            restore_state(&paths.state, &state_snapshot);
            // Bring dictation back on the previous model.
            return Err(match env.restart() {
                Ok(()) => MigrateError::failed(format!(
                    "{} voxtype is running again with {}.",
                    error.message, status.current.model
                )),
                Err(again) => MigrateError::failed(format!(
                    "{} Restarting with {} also failed ({again}); run: systemctl --user restart voxtype",
                    error.message, status.current.model
                )),
            });
        }
    }

    Ok(AcceptOutcome {
        engine: offer.engine.clone(),
        model: offer.model.clone(),
        previous_model: status.current.model.clone(),
        downloaded,
        restarted: was_running,
    })
}

/// Put the upgrade state file back exactly as it was before an accept.
fn restore_state(path: &Path, snapshot: &Option<Vec<u8>>) {
    let result = match snapshot {
        Some(bytes) => std::fs::write(path, bytes),
        None => match std::fs::remove_file(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(error) = result {
        tracing::warn!(%error, path = %path.display(), "Could not restore the model upgrade state");
    }
}

/// Record "keep current model" for one offer, or every current offer when
/// `offer_id` is `None`. Returns the ids declined.
pub fn decline(
    paths: &MigratePaths,
    status: &UpgradeStatus,
    offer_id: Option<&str>,
) -> Result<Vec<String>, MigrateError> {
    let ids: Vec<String> = match offer_id {
        Some(id) => vec![find_offer(status, id)?.id.clone()],
        None => status.offers.iter().map(|offer| offer.id.clone()).collect(),
    };
    if ids.is_empty() {
        return Ok(ids);
    }
    let mut state = UpgradeState::load(&paths.state);
    let version = env!("CARGO_PKG_VERSION").to_string();
    for id in &ids {
        state
            .declined
            .retain(|decline| !(decline.id == *id && decline.from_model == status.current.model));
        state.declined.push(Decline {
            id: id.clone(),
            from_model: status.current.model.clone(),
            version: version.clone(),
        });
    }
    state.save(&paths.state).map_err(|error| {
        MigrateError::failed(format!(
            "Could not record the decline in {}: {error}",
            paths.state.display()
        ))
    })?;
    Ok(ids)
}

/// Delete the previous model's files, only inside the models directory and
/// only after the accepted model transcribed successfully. Returns the
/// model removed and the bytes freed.
pub fn cleanup(
    paths: &MigratePaths,
    status: &UpgradeStatus,
) -> Result<(String, u64), MigrateError> {
    let mut state = UpgradeState::load(&paths.state);
    let Some(accepted) = state.accepted.as_ref() else {
        return Err(MigrateError::failed(
            "Nothing to clean up: no model upgrade has been accepted.",
        ));
    };
    if accepted.cleaned {
        return Err(MigrateError::failed(format!(
            "{} was already removed.",
            accepted.previous_model
        )));
    }
    if accepted.model != status.current.model {
        return Err(MigrateError::failed(format!(
            "The configured model is {}, not the accepted {}. Keeping {}.",
            status.current.model, accepted.model, accepted.previous_model
        )));
    }
    if !accepted.first_success {
        return Err(MigrateError::failed(format!(
            "Keeping {} until voxtype has transcribed successfully with {}. Dictate once, then run this again.",
            accepted.previous_model, accepted.model
        )));
    }
    let Some(cleanup) = status.cleanup.as_ref() else {
        return Err(MigrateError::failed(format!(
            "No old models are left in {}, so there is nothing to remove.",
            paths.models_dir.display()
        )));
    };
    let engine_models = crate::model_catalog::model_catalog(&accepted.engine);
    let mut bytes = 0;
    let mut removed_models = Vec::new();
    for name in &cleanup.models {
        if *name == status.current.model {
            continue;
        }
        let engine = if engine_models.contains(&name.as_str()) {
            accepted.engine.as_str()
        } else {
            accepted.previous_engine.as_str()
        };
        if super::model_format(engine, name) != "onnx" && *name != accepted.previous_model {
            continue;
        }
        let location = model_location(&paths.models_dir, engine, name);
        let path = contained_model_path(&paths.models_dir, &location)
            .map_err(|error| MigrateError::failed(format!("Not removing {name}: {error}")))?;
        let size = super::disk_usage(&path);
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        removed.map_err(|error| {
            MigrateError::failed(format!("Could not remove {}: {error}", path.display()))
        })?;
        bytes += size;
        removed_models.push(name.clone());
    }
    let model = removed_models.join(", ");
    if let Some(accepted) = state.accepted.as_mut() {
        accepted.cleaned = true;
    }
    if let Err(error) = state.save(&paths.state) {
        tracing::warn!(%error, "Could not record the model cleanup");
    }
    Ok((model, bytes))
}

// ------------------------------------------------------------ real system

/// The real environment: catalog downloads with the same progress output as
/// `voxtype setup --download`, the `config select-model` transaction, and
/// `systemctl --user restart voxtype`, the restart Settings already uses.
pub struct SystemEnv {
    pub config: Config,
    pub models_dir: PathBuf,
    /// How long a restarted daemon may take to load its model.
    pub ready_timeout: Duration,
}

impl SystemEnv {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            models_dir: Config::models_dir(),
            ready_timeout: Duration::from_secs(180),
        }
    }

    fn systemctl(args: &[&str]) -> Option<String> {
        let output = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl MigrateEnv for SystemEnv {
    fn download(&mut self, model: &str) -> anyhow::Result<()> {
        crate::setup::model::download_gguf_model(model)
    }

    fn verify(&mut self, model: &str) -> anyhow::Result<()> {
        let entry = crate::setup::model::find_gguf_model(model)
            .ok_or_else(|| anyhow::anyhow!("{model} is not in the catalog"))?;
        crate::setup::model::verify_gguf_model(&self.models_dir.join(entry.name), entry)
    }

    fn begin_commit(&mut self) {
        // Cancelling during the download is safe (nothing has been written)
        // and stays possible. After it, a signal could land between the
        // config write and the rollback, so ignore the cancel signals for
        // the rest of this short-lived process.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }

    fn daemon_running(&mut self) -> bool {
        Self::systemctl(&["is-active", "voxtype"]).as_deref() == Some("active")
    }

    fn ensure_idle(&mut self) -> Result<(), String> {
        crate::model_selection::validate_idle_for_restart(&self.config)
    }

    fn activate(&mut self, config: &Path, engine: &str, model: &str) -> Result<(), String> {
        crate::config_set::select_model(config.to_path_buf(), engine, model, None)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn validate_saved(&mut self, config: &Path) -> Result<(), String> {
        crate::model_selection::validate_saved_for_restart(config)
    }

    fn restart(&mut self) -> Result<(), String> {
        let started = SystemTime::now();
        let status = std::process::Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|error| format!("systemctl --user restart voxtype: {error}"))?;
        if !status.success() {
            return Err(format!(
                "systemctl --user restart voxtype exited with {status}"
            ));
        }
        // The daemon writes "idle" only after its model has loaded, so a
        // fresh idle state is the readiness signal. Without a state file the
        // best available signal is the unit staying up.
        let state_file = self.config.resolve_state_file();
        let deadline = Instant::now() + self.ready_timeout;
        let mut active_since: Option<Instant> = None;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            let unit = Self::systemctl(&["is-active", "voxtype"]).unwrap_or_default();
            match unit.as_str() {
                "active" => {}
                "activating" | "reloading" => {
                    active_since = None;
                    continue;
                }
                other => {
                    return Err(format!(
                        "the voxtype service is {other} after the restart; see: journalctl --user -u voxtype -n 50"
                    ))
                }
            }
            let since = *active_since.get_or_insert_with(Instant::now);
            match &state_file {
                Some(path) => {
                    let fresh = std::fs::metadata(path)
                        .and_then(|meta| meta.modified())
                        .is_ok_and(|modified| modified >= started);
                    let idle =
                        std::fs::read_to_string(path).is_ok_and(|state| state.trim() == "idle");
                    if fresh && idle {
                        return Ok(());
                    }
                }
                None if since.elapsed() >= Duration::from_secs(5) => return Ok(()),
                None => {}
            }
        }
        Err(format!(
            "voxtype did not report ready within {}s; see: journalctl --user -u voxtype -n 50",
            self.ready_timeout.as_secs()
        ))
    }
}
