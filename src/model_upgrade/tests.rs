use super::migrate::{self, MigrateEnv, MigratePaths};
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

const ALL: BackendCapabilities = BackendCapabilities {
    parakeet_onnx: true,
    parakeet_gguf: true,
    cohere_onnx: true,
    cohere_gguf: true,
    gguf: true,
};

const ONNX_ONLY: BackendCapabilities = BackendCapabilities {
    parakeet_onnx: true,
    parakeet_gguf: false,
    cohere_onnx: true,
    cohere_gguf: false,
    gguf: false,
};

const PIN: &str = "add IgnorePkg = voxtype-bin to /etc/pacman.conf";

fn status_with(
    engine: &str,
    model: &str,
    streaming: bool,
    capabilities: BackendCapabilities,
    models_dir: &Path,
    state: &UpgradeState,
) -> UpgradeStatus {
    compute(
        &Inputs {
            engine,
            model,
            streaming,
            capabilities,
            gguf_worker: true,
            models_dir,
            state,
        },
        || PIN.to_string(),
    )
}

fn status(engine: &str, model: &str) -> UpgradeStatus {
    let dir = tempfile::tempdir().unwrap();
    status_with(
        engine,
        model,
        false,
        ALL,
        dir.path(),
        &UpgradeState::default(),
    )
}

fn offer_ids(status: &UpgradeStatus) -> Vec<(&str, &str)> {
    status
        .offers
        .iter()
        .map(|offer| (offer.id.as_str(), offer.model.as_str()))
        .collect()
}

/// A file the cheap integrity check accepts as the catalog GGUF: right size,
/// GGUF magic. Sparse, so it costs no disk.
fn fake_gguf(models_dir: &Path, name: &str) {
    let entry = crate::setup::model::find_gguf_model(name).unwrap();
    let file = std::fs::File::create(models_dir.join(name)).unwrap();
    file.set_len(entry.size).unwrap();
    use std::io::Write;
    (&file).write_all(b"GGUF").unwrap();
}

// ------------------------------------------------------------ offer table

#[test]
fn parakeet_v3_onnx_gets_ultra_first_then_the_same_checkpoint_in_gguf() {
    for model in ["parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v3-int8"] {
        let status = status("parakeet", model);
        assert_eq!(
            offer_ids(&status),
            vec![
                (OFFER_ULTRA, "parakeet-ultra-Q8_0.gguf"),
                (OFFER_FORMAT, "parakeet-tdt-0.6b-v3-Q8_0.gguf"),
            ]
        );
        let ultra = &status.offers[0];
        assert!(ultra.recommended);
        assert_eq!(ultra.kind, "model");
        assert_eq!(ultra.title, "Parakeet Ultra is now available");
        assert_eq!(
            ultra.detail,
            "Parakeet Ultra covers the same 25 European languages as Parakeet v3."
        );
        assert_eq!(ultra.download_bytes, 740_363_168);
        let format = &status.offers[1];
        assert!(!format.recommended);
        assert_eq!(format.kind, "format");
        assert_eq!(format.title, "Your model format has been updated");
        assert_eq!(
            format.detail,
            format!("Redownload {model} in the new format for better performance.")
        );
        assert_eq!(format.download_bytes, 739_508_576);
        assert_eq!(status.current.format, "onnx");
    }
}

#[test]
fn parakeet_v3_gguf_gets_only_ultra() {
    for model in [
        "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        "parakeet-tdt-0.6b-v3-F16.gguf",
        "/opt/models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf",
    ] {
        let status = status("parakeet", model);
        assert_eq!(
            offer_ids(&status),
            vec![(OFFER_ULTRA, "parakeet-ultra-Q8_0.gguf")],
            "{model}"
        );
        assert_eq!(status.current.format, "gguf");
        assert!(status.removal_notice.is_none(), "{model}");
    }
}

#[test]
fn parakeet_v2_gets_only_ultra_until_a_v2_gguf_exists() {
    for model in ["parakeet-tdt-0.6b-v2", "parakeet-tdt-0.6b-v2-int8"] {
        let status = status("parakeet", model);
        assert_eq!(
            offer_ids(&status),
            vec![(OFFER_ULTRA, "parakeet-ultra-Q8_0.gguf")]
        );
        assert!(status.offers[0].detail.contains("English"));
    }
}

#[test]
fn streaming_configs_get_no_offer() {
    let dir = tempfile::tempdir().unwrap();
    let state = UpgradeState::default();
    let unified = status_with(
        "parakeet",
        "parakeet-unified-en-0.6b",
        true,
        ALL,
        dir.path(),
        &state,
    );
    assert!(unified.offers.is_empty());
    let unified_batch = status("parakeet", "parakeet-unified-en-0.6b");
    assert!(unified_batch.offers.is_empty());
    let v3_streaming = status_with(
        "parakeet",
        "parakeet-tdt-0.6b-v3",
        true,
        ALL,
        dir.path(),
        &state,
    );
    assert!(v3_streaming.offers.is_empty());
    // Still ONNX, so the 2.0 removal applies.
    assert!(unified.removal_notice.is_some());
}

#[test]
fn ultra_users_get_nothing() {
    for model in ["parakeet-ultra-Q8_0.gguf", "parakeet-ultra-F16.gguf"] {
        let status = status("parakeet", model);
        assert!(status.offers.is_empty());
        assert!(!status.has_anything());
    }
}

#[test]
fn cohere_onnx_gets_the_matching_gguf_quantization() {
    for (model, target) in [
        (
            "cohere-transcribe-q4f16",
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        ),
        (
            "cohere-transcribe-q4",
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        ),
        (
            "cohere-transcribe-int8",
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        ),
        (
            "cohere-transcribe-fp16",
            "cohere-transcribe-03-2026-Q8_0.gguf",
        ),
    ] {
        let status = status("cohere", model);
        assert_eq!(offer_ids(&status), vec![(OFFER_FORMAT, target)], "{model}");
        assert_eq!(status.offers[0].engine, "cohere");
        assert!(!status.offers[0].recommended);
    }
    assert!(status("cohere", "cohere-transcribe-03-2026-Q4_K_M.gguf")
        .offers
        .is_empty());
}

#[test]
fn families_without_a_gguf_get_no_offer_but_the_removal_notice() {
    for (engine, model) in [
        ("moonshine", "base"),
        ("sensevoice", "small"),
        ("paraformer", "paraformer-zh"),
        ("dolphin", "dolphin-base"),
        ("omnilingual", "omnilingual-300m"),
    ] {
        let status = status(engine, model);
        assert!(status.offers.is_empty(), "{engine}");
        assert_eq!(status.current.format, "onnx");
        assert!(status.removal_notice.is_some(), "{engine}");
    }
}

#[test]
fn non_onnx_engines_get_nothing() {
    for (engine, model, format) in [
        ("whisper", "base.en", "ggml"),
        ("gguf", "Qwen3-ASR-0.6B-Q8_0.gguf", "gguf"),
        ("openvino", "base.en-int8", "openvino"),
        ("soniox", "stt-rt-v3", "remote"),
    ] {
        let status = status(engine, model);
        assert_eq!(status.current.format, format);
        assert!(!status.has_anything(), "{engine}");
    }
}

#[test]
fn binaries_without_the_gguf_runtime_get_no_offer() {
    let dir = tempfile::tempdir().unwrap();
    let state = UpgradeState::default();
    for (engine, model) in [
        ("parakeet", "parakeet-tdt-0.6b-v3"),
        ("parakeet", "parakeet-tdt-0.6b-v2"),
        ("cohere", "cohere-transcribe-q4f16"),
    ] {
        let status = status_with(engine, model, false, ONNX_ONLY, dir.path(), &state);
        assert!(status.offers.is_empty(), "{model}");
        assert!(status.removal_notice.is_some(), "{model}");
    }
    // Each family is gated on its own feature.
    let parakeet_only = BackendCapabilities {
        parakeet_gguf: true,
        ..ONNX_ONLY
    };
    assert_eq!(
        status_with(
            "parakeet",
            "parakeet-tdt-0.6b-v3",
            false,
            parakeet_only,
            dir.path(),
            &state
        )
        .offers
        .len(),
        2
    );
    assert!(status_with(
        "cohere",
        "cohere-transcribe-q4f16",
        false,
        parakeet_only,
        dir.path(),
        &state
    )
    .offers
    .is_empty());
}

/// A package can compile the GGUF client and still leave the worker out (a
/// repackaging without the `.gguf-worker` files). Accepting an offer there
/// would fail, so none is made; the ONNX removal notice still shows.
#[test]
fn compiled_gguf_without_a_reachable_worker_gets_no_offer_but_the_removal_notice() {
    let dir = tempfile::tempdir().unwrap();
    let state = UpgradeState::default();
    for (engine, model) in [
        ("parakeet", "parakeet-tdt-0.6b-v3"),
        ("parakeet", "parakeet-tdt-0.6b-v2-int8"),
        ("parakeet", "parakeet-tdt-0.6b-v3-Q8_0.gguf"),
        ("cohere", "cohere-transcribe-q4f16"),
        ("cohere", "cohere-transcribe-fp16"),
    ] {
        let inputs = Inputs {
            engine,
            model,
            streaming: false,
            capabilities: ALL,
            gguf_worker: false,
            models_dir: dir.path(),
            state: &state,
        };
        let status = compute(&inputs, || PIN.to_string());
        assert!(status.offers.is_empty(), "{model}: {:?}", status.offers);
        assert_eq!(
            status.removal_notice.is_some(),
            model_format(engine, model) == "onnx",
            "{model}"
        );

        // The same inputs with the worker present do get offers.
        let with_worker = compute(
            &Inputs {
                gguf_worker: true,
                ..inputs
            },
            || PIN.to_string(),
        );
        assert!(!with_worker.offers.is_empty(), "{model}");
    }
}

/// A format target already on disk is a switch, not a redownload, and the
/// copy must not contradict the "already downloaded" line beside it.
#[test]
fn an_installed_format_target_reads_as_a_switch() {
    let dir = tempfile::tempdir().unwrap();
    fake_gguf(dir.path(), "parakeet-tdt-0.6b-v3-Q8_0.gguf");
    let status = status_with(
        "parakeet",
        "parakeet-tdt-0.6b-v3",
        false,
        ALL,
        dir.path(),
        &UpgradeState::default(),
    );
    let format = status.offer(OFFER_FORMAT).unwrap();
    assert_eq!(format.download_bytes, 0);
    assert_eq!(
        format.detail,
        "Switch parakeet-tdt-0.6b-v3 to the new format for better performance."
    );
}

#[test]
fn an_installed_target_needs_no_download() {
    let dir = tempfile::tempdir().unwrap();
    fake_gguf(dir.path(), "parakeet-ultra-Q8_0.gguf");
    let status = status_with(
        "parakeet",
        "parakeet-tdt-0.6b-v3",
        false,
        ALL,
        dir.path(),
        &UpgradeState::default(),
    );
    assert_eq!(status.offer(OFFER_ULTRA).unwrap().download_bytes, 0);
    assert_eq!(
        status.offer(OFFER_FORMAT).unwrap().download_bytes,
        739_508_576
    );
}

// --------------------------------------------------------- removal notice

#[test]
fn removal_notice_is_only_for_onnx_and_names_the_pin_command() {
    let notice = status("parakeet", "parakeet-tdt-0.6b-v3")
        .removal_notice
        .unwrap();
    assert_eq!(notice.version, "2.0");
    assert_eq!(notice.pin_command, PIN);
    assert_eq!(
        notice.message,
        format!("Support for parakeet-tdt-0.6b-v3 (ONNX) will be removed in voxtype 2.0. To keep using it, pin voxtype to this version: {PIN}")
    );
    assert!(!notice.message.contains('\u{2014}'));
    assert!(status("parakeet", "parakeet-tdt-0.6b-v3-Q8_0.gguf")
        .removal_notice
        .is_none());
    assert!(status("whisper", "base.en").removal_notice.is_none());
}

#[test]
fn the_pin_command_is_not_detected_for_gguf_models() {
    let dir = tempfile::tempdir().unwrap();
    let status = compute(
        &Inputs {
            engine: "parakeet",
            model: "parakeet-tdt-0.6b-v3-Q8_0.gguf",
            streaming: false,
            capabilities: ALL,
            gguf_worker: true,
            models_dir: dir.path(),
            state: &UpgradeState::default(),
        },
        || panic!("package manager queried for a GGUF model"),
    );
    assert!(status.removal_notice.is_none());
}

struct FakeRunner(HashMap<&'static str, &'static str>);

impl CommandRunner for FakeRunner {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        let path = args.last().copied().unwrap_or_default();
        self.0
            .get(format!("{program} {path}").as_str())
            .map(|out| out.to_string())
    }
}

fn runner(entries: &[(&'static str, &'static str)]) -> FakeRunner {
    FakeRunner(entries.iter().copied().collect())
}

#[test]
fn pin_command_per_install_method() {
    let exe = Path::new("/usr/lib/voxtype/voxtype-onnx-migraphx");
    let arch = runner(&[(
        "pacman /usr/lib/voxtype/voxtype-onnx-migraphx",
        "voxtype-bin\n",
    )]);
    assert_eq!(
        detect_install_method(exe, &arch),
        InstallMethod::Pacman("voxtype-bin".into())
    );
    assert_eq!(
        InstallMethod::Pacman("voxtype-bin".into()).pin_command(),
        "add IgnorePkg = voxtype-bin to /etc/pacman.conf"
    );

    // The executable is not owned, but /usr/bin/voxtype is.
    let debian = runner(&[("dpkg /usr/bin/voxtype", "voxtype: /usr/bin/voxtype\n")]);
    let method = detect_install_method(exe, &debian);
    assert_eq!(method, InstallMethod::Dpkg("voxtype".into()));
    assert_eq!(method.pin_command(), "sudo apt-mark hold voxtype");
    let multiarch = runner(&[(
        "dpkg /usr/lib/voxtype/voxtype-onnx-migraphx",
        "voxtype:amd64: /usr/lib/voxtype/voxtype-onnx-migraphx\n",
    )]);
    assert_eq!(
        detect_install_method(exe, &multiarch),
        InstallMethod::Dpkg("voxtype".into())
    );

    let fedora = runner(&[("rpm /usr/bin/voxtype", "voxtype")]);
    let method = detect_install_method(Path::new("/usr/bin/voxtype"), &fedora);
    assert_eq!(method, InstallMethod::Rpm("voxtype".into()));
    assert_eq!(method.pin_command(), "sudo dnf versionlock add voxtype");

    let nix = detect_install_method(
        Path::new("/nix/store/abc-voxtype-1.2.0/bin/voxtype"),
        &runner(&[]),
    );
    assert_eq!(nix, InstallMethod::Nix);
    assert_eq!(nix.pin_command(), "pin the voxtype flake input");

    let other = detect_install_method(Path::new("/home/u/.cargo/bin/voxtype"), &runner(&[]));
    assert_eq!(other, InstallMethod::Other);
    assert!(other.pin_command().contains(env!("CARGO_PKG_VERSION")));
}

// ---------------------------------------------------------------- decline

fn paths(dir: &Path) -> MigratePaths {
    let models_dir = dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    MigratePaths {
        config: dir.join("config.toml"),
        models_dir,
        state: dir.join("state/model_upgrade.json"),
    }
}

fn status_at(paths: &MigratePaths, engine: &str, model: &str) -> UpgradeStatus {
    let state = UpgradeState::load(&paths.state);
    status_with(engine, model, false, ALL, &paths.models_dir, &state)
}

#[test]
fn decline_persists_keeps_the_offer_listed_and_is_per_model() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let before = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    assert!(before.has_open_offer());

    let declined = migrate::decline(&paths, &before, Some(OFFER_ULTRA)).unwrap();
    assert_eq!(declined, vec![OFFER_ULTRA.to_string()]);
    // Reloaded from disk, as after a restart or upgrade.
    let after = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    assert!(after.offer(OFFER_ULTRA).unwrap().declined);
    assert!(!after.offer(OFFER_FORMAT).unwrap().declined);
    assert!(after.has_open_offer());
    let recorded = UpgradeState::load(&paths.state);
    assert_eq!(recorded.declined[0].version, env!("CARGO_PKG_VERSION"));

    // Declining everything leaves both listed, both declined.
    migrate::decline(&paths, &after, None).unwrap();
    let all = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    assert_eq!(all.offers.len(), 2);
    assert!(all.offers.iter().all(|offer| offer.declined));
    assert!(!all.has_open_offer());
    assert_eq!(UpgradeState::load(&paths.state).declined.len(), 2);

    // The same offer on another model is a new question.
    let other = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3-Q8_0.gguf");
    assert!(!other.offer(OFFER_ULTRA).unwrap().declined);
    assert!(other.removal_notice.is_none());
}

#[test]
fn decline_never_touches_config() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    std::fs::write(&paths.config, "engine = \"parakeet\"\n").unwrap();
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    migrate::decline(&paths, &status, None).unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths.config).unwrap(),
        "engine = \"parakeet\"\n"
    );
}

#[test]
fn unknown_offer_ids_are_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    let error = migrate::decline(&paths, &status, Some("bogus")).unwrap_err();
    assert_eq!(error.code, 2);
    assert!(error.message.contains("parakeet-ultra, format-gguf"));
    let none = status_at(&paths, "whisper", "base.en");
    let mut env = FakeEnv::default();
    let error = migrate::accept(&mut env, &paths, &none, OFFER_ULTRA).unwrap_err();
    assert_eq!(error.code, 2);
    assert!(env.calls.borrow().is_empty());
}

// ----------------------------------------------------------------- accept

const OLD_CONFIG: &str = "# my settings\nengine = \"parakeet\"\n\n[parakeet]\nmodel = \"parakeet-tdt-0.6b-v3\"  # keep\n";

#[derive(Default)]
struct FakeEnv {
    fail_download: Option<&'static str>,
    fail_verify: bool,
    running: bool,
    busy: bool,
    fail_activate: bool,
    fail_validate: bool,
    /// Results for successive restarts; missing entries succeed.
    restarts: Vec<Result<(), String>>,
    calls: RefCell<Vec<String>>,
    /// When set, each restart records the upgrade state file as the
    /// restarted daemon would read it at startup.
    watch_state: Option<PathBuf>,
    state_at_restart: RefCell<Vec<Option<String>>>,
}

impl FakeEnv {
    fn log(&self, call: impl Into<String>) {
        self.calls.borrow_mut().push(call.into());
    }
}

impl MigrateEnv for FakeEnv {
    fn download(&mut self, model: &str) -> anyhow::Result<()> {
        self.log(format!("download {model}"));
        match self.fail_download {
            Some(message) => anyhow::bail!("{message}"),
            None => Ok(()),
        }
    }
    fn verify(&mut self, model: &str) -> anyhow::Result<()> {
        self.log(format!("verify {model}"));
        if self.fail_verify {
            anyhow::bail!("sha256 mismatch");
        }
        Ok(())
    }
    fn begin_commit(&mut self) {
        self.log("begin_commit");
    }
    fn daemon_running(&mut self) -> bool {
        self.running
    }
    fn ensure_idle(&mut self) -> Result<(), String> {
        self.log("ensure_idle");
        if self.busy {
            Err("Daemon is busy.".into())
        } else {
            Ok(())
        }
    }
    fn activate(&mut self, config: &Path, engine: &str, model: &str) -> Result<(), String> {
        self.log(format!("activate {engine} {model}"));
        // Write first, then fail: rollback must undo even a partial write.
        std::fs::write(
            config,
            format!("engine = \"{engine}\"\n\n[{engine}]\nmodel = \"{model}\"\n"),
        )
        .unwrap();
        if self.fail_activate {
            Err("validation failed".into())
        } else {
            Ok(())
        }
    }
    fn validate_saved(&mut self, _config: &Path) -> Result<(), String> {
        self.log("validate_saved");
        if self.fail_validate {
            Err("worker missing".into())
        } else {
            Ok(())
        }
    }
    fn restart(&mut self) -> Result<(), String> {
        self.log("restart");
        if let Some(path) = &self.watch_state {
            self.state_at_restart
                .borrow_mut()
                .push(std::fs::read_to_string(path).ok());
        }
        if self.restarts.is_empty() {
            Ok(())
        } else {
            self.restarts.remove(0)
        }
    }
}

fn accept_case(env: &mut FakeEnv, config: Option<&str>) -> (MigratePaths, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    if let Some(config) = config {
        std::fs::write(&paths.config, config).unwrap();
    }
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    let result = migrate::accept(env, &paths, &status, OFFER_ULTRA);
    assert!(result.is_err(), "expected failure, got {result:?}");
    let error = result.unwrap_err();
    assert_eq!(error.code, 1);
    assert!(!error.message.contains('\u{2014}'), "{}", error.message);
    // The offer is still there and nothing was recorded.
    let after = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    assert!(after.offer(OFFER_ULTRA).is_some());
    assert!(UpgradeState::load(&paths.state).accepted.is_none());
    (paths, dir)
}

fn assert_config(paths: &MigratePaths, expected: Option<&str>) {
    match expected {
        Some(text) => assert_eq!(std::fs::read(&paths.config).unwrap(), text.as_bytes()),
        None => assert!(!paths.config.exists()),
    }
}

#[test]
fn failed_or_cancelled_download_leaves_config_byte_identical() {
    for message in ["curl exited with code 28", "cancelled"] {
        let mut env = FakeEnv {
            fail_download: Some(message),
            running: true,
            ..Default::default()
        };
        let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
        assert_config(&paths, Some(OLD_CONFIG));
        assert!(!env.calls.borrow().iter().any(|c| c.starts_with("activate")));
        // A failed or cancelled download never enters the commit phase.
        assert!(!env.calls.borrow().contains(&"begin_commit".to_string()));
    }
}

#[test]
fn checksum_failure_leaves_config_byte_identical() {
    let mut env = FakeEnv {
        fail_download: Some("sha256 mismatch for parakeet-ultra-Q8_0.gguf"),
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
    assert_config(&paths, Some(OLD_CONFIG));
}

#[test]
fn busy_daemon_refuses_before_writing() {
    let mut env = FakeEnv {
        running: true,
        busy: true,
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
    assert_config(&paths, Some(OLD_CONFIG));
    assert!(!env.calls.borrow().iter().any(|c| c.starts_with("activate")));
}

#[test]
fn failed_activation_is_rolled_back() {
    let mut env = FakeEnv {
        running: true,
        fail_activate: true,
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
    assert_config(&paths, Some(OLD_CONFIG));
    assert!(!env.calls.borrow().contains(&"restart".to_string()));

    // A config that did not exist before does not exist after.
    let mut env = FakeEnv {
        fail_activate: true,
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, None);
    assert_config(&paths, None);
}

#[test]
fn failed_restart_check_is_rolled_back() {
    let mut env = FakeEnv {
        running: true,
        fail_validate: true,
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
    assert_config(&paths, Some(OLD_CONFIG));
    assert!(!env.calls.borrow().contains(&"restart".to_string()));
}

#[test]
fn failed_restart_is_rolled_back_and_the_old_model_restarted() {
    let mut env = FakeEnv {
        running: true,
        restarts: vec![Err("service failed".into()), Ok(())],
        ..Default::default()
    };
    let (paths, _dir) = accept_case(&mut env, Some(OLD_CONFIG));
    assert_config(&paths, Some(OLD_CONFIG));
    let restarts = env
        .calls
        .borrow()
        .iter()
        .filter(|c| *c == "restart")
        .count();
    assert_eq!(restarts, 2);
}

#[test]
fn accept_switches_restarts_and_records_the_previous_model() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    std::fs::write(&paths.config, OLD_CONFIG).unwrap();
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    let mut env = FakeEnv {
        running: true,
        ..Default::default()
    };
    let outcome = migrate::accept(&mut env, &paths, &status, OFFER_FORMAT).unwrap();
    assert_eq!(outcome.model, "parakeet-tdt-0.6b-v3-Q8_0.gguf");
    assert!(outcome.downloaded && outcome.restarted);
    assert_eq!(
        *env.calls.borrow(),
        vec![
            "download parakeet-tdt-0.6b-v3-Q8_0.gguf",
            "begin_commit",
            "ensure_idle",
            "activate parakeet parakeet-tdt-0.6b-v3-Q8_0.gguf",
            "validate_saved",
            "restart",
        ]
    );
    let accepted = UpgradeState::load(&paths.state).accepted.unwrap();
    assert_eq!(accepted.previous_model, "parakeet-tdt-0.6b-v3");
    assert_eq!(accepted.previous_engine, "parakeet");
    assert!(!accepted.first_success);
}

/// The restarted daemon reads the accepted record once at startup to know
/// its first successful transcription unlocks cleanup, so the record must
/// already name the new model when the restart happens (Pete's rc4 test:
/// written afterwards, cleanup never appeared).
#[test]
fn the_accepted_record_exists_before_the_daemon_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    std::fs::write(&paths.config, OLD_CONFIG).unwrap();
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    let mut env = FakeEnv {
        running: true,
        watch_state: Some(paths.state.clone()),
        ..Default::default()
    };
    migrate::accept(&mut env, &paths, &status, OFFER_ULTRA).unwrap();
    let seen = env.state_at_restart.borrow();
    assert_eq!(seen.len(), 1);
    let at_restart = seen[0]
        .as_deref()
        .expect("state file written before restart");
    assert!(
        crate::model_upgrade::pending_first_success(&paths.state, "parakeet-ultra-Q8_0.gguf"),
        "the restarted daemon must see a pending first success"
    );
    assert!(
        at_restart.contains("parakeet-ultra-Q8_0.gguf"),
        "{at_restart}"
    );
}

#[test]
fn an_installed_target_is_verified_instead_of_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fake_gguf(&paths.models_dir, "parakeet-ultra-Q8_0.gguf");
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    let mut env = FakeEnv::default();
    let outcome = migrate::accept(&mut env, &paths, &status, OFFER_ULTRA).unwrap();
    assert!(!outcome.downloaded && !outcome.restarted);
    assert_eq!(env.calls.borrow()[0], "verify parakeet-ultra-Q8_0.gguf");
    assert!(!env.calls.borrow().iter().any(|c| c.starts_with("download")));

    // A damaged copy is replaced by a fresh download.
    let mut env = FakeEnv {
        fail_verify: true,
        ..Default::default()
    };
    migrate::accept(&mut env, &paths, &status, OFFER_ULTRA).unwrap();
    assert_eq!(env.calls.borrow()[1], "download parakeet-ultra-Q8_0.gguf");
}

// ---------------------------------------------------------------- cleanup

fn accepted_state(paths: &MigratePaths, first_success: bool) {
    UpgradeState {
        declined: Vec::new(),
        accepted: Some(Accepted {
            id: OFFER_FORMAT.into(),
            engine: "parakeet".into(),
            model: "parakeet-tdt-0.6b-v3-Q8_0.gguf".into(),
            previous_engine: "parakeet".into(),
            previous_model: "parakeet-tdt-0.6b-v3".into(),
            version: "1.2.0".into(),
            first_success,
            cleaned: false,
        }),
    }
    .save(&paths.state)
    .unwrap();
}

fn old_model_dir(paths: &MigratePaths) -> PathBuf {
    let old = paths.models_dir.join("parakeet-tdt-0.6b-v3");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("encoder-model.onnx"), vec![0u8; 1000]).unwrap();
    std::fs::write(old.join("vocab.txt"), vec![0u8; 24]).unwrap();
    old
}

#[test]
fn cleanup_waits_for_the_first_successful_transcription() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let old = old_model_dir(&paths);
    accepted_state(&paths, false);
    let current = "parakeet-tdt-0.6b-v3-Q8_0.gguf";
    let status = status_at(&paths, "parakeet", current);
    assert!(status.cleanup.is_none());
    let error = migrate::cleanup(&paths, &status).unwrap_err();
    assert!(error.message.contains("transcribed successfully"));
    assert!(old.exists());

    assert!(pending_first_success(&paths.state, current));
    assert!(!pending_first_success(&paths.state, "parakeet-tdt-0.6b-v3"));
    assert!(record_first_success(&paths.state, current).unwrap());
    assert!(!record_first_success(&paths.state, current).unwrap());
    assert!(!pending_first_success(&paths.state, current));

    let status = status_at(&paths, "parakeet", current);
    assert_eq!(
        status.cleanup,
        Some(Cleanup {
            model: "parakeet-tdt-0.6b-v3".into(),
            bytes: 1024,
            models: vec!["parakeet-tdt-0.6b-v3".into()],
        })
    );
    assert_eq!(
        migrate::cleanup(&paths, &status).unwrap(),
        ("parakeet-tdt-0.6b-v3".to_string(), 1024)
    );
    assert!(!old.exists());
    assert!(paths.models_dir.exists());
    let after = status_at(&paths, "parakeet", current);
    assert!(after.cleanup.is_none());
    assert!(migrate::cleanup(&paths, &after).is_err());
}

/// One-way door (Pete's rc4 test): after format-gguf and then Ultra, the
/// previous model is the intermediate GGUF. Cleanup removes every installed
/// ONNX model of the engine, which the model list no longer offers, and
/// keeps both GGUFs.
#[test]
fn cleanup_removes_every_legacy_onnx_model_and_keeps_gguf() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let onnx = [
        "parakeet-tdt-0.6b-v2",
        "parakeet-tdt-0.6b-v3",
        "parakeet-unified-en-0.6b",
    ];
    for name in onnx {
        let d = paths.models_dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("encoder-model.onnx"), vec![0u8; 100]).unwrap();
    }
    fake_gguf(&paths.models_dir, "parakeet-tdt-0.6b-v3-Q8_0.gguf");
    fake_gguf(&paths.models_dir, "parakeet-ultra-Q8_0.gguf");
    UpgradeState {
        declined: Vec::new(),
        accepted: Some(Accepted {
            id: OFFER_ULTRA.into(),
            engine: "parakeet".into(),
            model: "parakeet-ultra-Q8_0.gguf".into(),
            previous_engine: "parakeet".into(),
            previous_model: "parakeet-tdt-0.6b-v3-Q8_0.gguf".into(),
            version: "1.2.0".into(),
            first_success: true,
            cleaned: false,
        }),
    }
    .save(&paths.state)
    .unwrap();

    let status = status_at(&paths, "parakeet", "parakeet-ultra-Q8_0.gguf");
    let cleanup = status
        .cleanup
        .clone()
        .expect("legacy ONNX models offered for cleanup");
    let mut listed = cleanup.models.clone();
    listed.sort();
    assert_eq!(listed, onnx.map(String::from).to_vec());
    assert_eq!(cleanup.bytes, 300);

    let (removed, bytes) = migrate::cleanup(&paths, &status).unwrap();
    assert_eq!(bytes, 300);
    for name in onnx {
        assert!(removed.contains(name), "{removed}");
        assert!(!paths.models_dir.join(name).exists(), "{name} removed");
    }
    assert!(paths
        .models_dir
        .join("parakeet-tdt-0.6b-v3-Q8_0.gguf")
        .exists());
    assert!(paths.models_dir.join("parakeet-ultra-Q8_0.gguf").exists());
    assert!(status_at(&paths, "parakeet", "parakeet-ultra-Q8_0.gguf")
        .cleanup
        .is_none());
}

/// Only engines that list both ONNX and GGUF checkpoints treat ONNX as legacy;
/// ONNX-only engines keep their whole list.
#[test]
fn legacy_models_are_onnx_checkpoints_of_engines_with_gguf() {
    assert!(crate::model_upgrade::is_legacy_model(
        "parakeet",
        "parakeet-tdt-0.6b-v3"
    ));
    assert!(crate::model_upgrade::is_legacy_model(
        "cohere",
        "cohere-transcribe-q4f16"
    ));
    assert!(!crate::model_upgrade::is_legacy_model(
        "parakeet",
        "parakeet-ultra-Q8_0.gguf"
    ));
    assert!(!crate::model_upgrade::is_legacy_model("moonshine", "base"));
    assert!(!crate::model_upgrade::is_legacy_model(
        "sensevoice",
        "small"
    ));
    assert!(!crate::model_upgrade::is_legacy_model("whisper", "base.en"));
    assert!(!crate::model_upgrade::is_legacy_model(
        "gguf",
        "Qwen3-ASR-0.6B-Q8_0.gguf"
    ));
}

#[test]
fn cleanup_is_not_offered_after_switching_away_from_the_accepted_model() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let old = old_model_dir(&paths);
    accepted_state(&paths, true);
    let status = status_at(&paths, "parakeet", "parakeet-tdt-0.6b-v3");
    assert!(status.cleanup.is_none());
    assert!(migrate::cleanup(&paths, &status).is_err());
    assert!(old.exists());
}

#[cfg(unix)]
#[test]
fn cleanup_never_leaves_the_models_directory() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    accepted_state(&paths, true);
    let current = "parakeet-tdt-0.6b-v3-Q8_0.gguf";

    // A symlinked model directory pointing elsewhere is refused, and its
    // target survives.
    let outside = dir.path().join("precious");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, paths.models_dir.join("parakeet-tdt-0.6b-v3")).unwrap();
    let status = status_at(&paths, "parakeet", current);
    assert!(status.cleanup.is_none());
    let error = migrate::cleanup(&paths, &status).unwrap_err();
    assert!(error.message.contains("nothing to remove") || error.message.contains("symbolic"));
    assert!(outside.join("keep.txt").exists());

    // Path escapes in a recorded model name are refused too.
    assert!(contained_model_path(&paths.models_dir, &paths.models_dir.join("..")).is_err());
    assert!(contained_model_path(&paths.models_dir, &outside).is_err());
    assert!(contained_model_path(
        &paths.models_dir,
        &paths.models_dir.join("sub/../../precious")
    )
    .is_err());
}

// ------------------------------------------------------------ JSON shape

#[test]
fn status_json_matches_the_frontend_contract() {
    let status = status("parakeet", "parakeet-tdt-0.6b-v3");
    let text = serde_json::to_string(&status).unwrap();
    // serde_json::Value sorts keys, so pin the serialized order directly.
    assert!(text.starts_with(r#"{"schema":1,"current":{"engine":"parakeet","model":"parakeet-tdt-0.6b-v3","format":"onnx"},"offers":[{"id":"parakeet-ultra","kind":"model","engine":"parakeet","model":"parakeet-ultra-Q8_0.gguf","title":"Parakeet Ultra is now available","detail":"Parakeet Ultra covers the same 25 European languages as Parakeet v3.","download_bytes":740363168,"recommended":true,"declined":false},{"id":"format-gguf","kind":"format","#), "{text}");
    assert!(
        text.contains(r#""removal_notice":{"version":"2.0","message":"#),
        "{text}"
    );
    assert!(text.ends_with(r#"},"cleanup":null}"#), "{text}");
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["schema"], 1);
    assert_eq!(
        json["current"],
        serde_json::json!({"engine": "parakeet", "model": "parakeet-tdt-0.6b-v3", "format": "onnx"})
    );
    assert_eq!(
        json["offers"][0],
        serde_json::json!({
            "id": "parakeet-ultra", "kind": "model", "engine": "parakeet",
            "model": "parakeet-ultra-Q8_0.gguf",
            "title": "Parakeet Ultra is now available",
            "detail": "Parakeet Ultra covers the same 25 European languages as Parakeet v3.",
            "download_bytes": 740363168u64, "recommended": true, "declined": false
        })
    );
    assert!(json["cleanup"].is_null());

    let gguf = serde_json::to_value(status_for_gguf()).unwrap();
    assert!(gguf["removal_notice"].is_null());
}

fn status_for_gguf() -> UpgradeStatus {
    status("parakeet", "parakeet-tdt-0.6b-v3-Q8_0.gguf")
}

#[test]
fn a_damaged_state_file_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model_upgrade.json");
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(UpgradeState::load(&path), UpgradeState::default());
    assert_eq!(
        UpgradeState::load(&dir.path().join("missing.json")),
        UpgradeState::default()
    );
}
