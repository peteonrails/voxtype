//! `voxtype-osd-quickshell` — a tiny launcher that finds voxtype's Quickshell
//! shell directory (containing `shell.qml`) and execs `qs -d -p <dir>`.
//!
//! Quickshell (`qs`) treats the directory as a config root and loads
//! `shell.qml` from it. We pass the directory rather than the file so that
//! sibling QML imports (`import "voxtype-shared" as VT`) resolve through
//! Quickshell's virtual filesystem; passing the file directly traps `..`
//! traversals in `qrc:/qs-blackhole`.
//!
//! ## Daemonize by default
//!
//! The launcher passes `-d` to `qs` by default. Without `-d`, qs stays
//! attached to its controlling terminal and dies via SIGHUP when its
//! parent process exits — which is exactly what happens when users invoke
//! the launcher from a hotkey, a short-lived shell, or `voxtype setup
//! quickshell` smoke tests (see issue #395). Passing `-d` forks qs into
//! its own session so the OSD survives.
//!
//! The daemon's OSD supervisor needs the opposite behavior: it spawns
//! `voxtype-osd` (the dispatcher) via `tokio::process::Command` with
//! `kill_on_drop(true)` so the OSD child is killed on daemon shutdown. If
//! qs daemonizes, the supervisor's child slot exits immediately, the
//! supervisor thinks qs died, and it respawns in a loop. To opt out, the
//! supervisor sets `VOXTYPE_OSD_SUPERVISED=1` and the dispatcher then
//! passes `--no-daemonize` through to this launcher.
//!
//! The launcher resolves the shell directory in this order:
//!
//! 1. `--qml-path <PATH>` on the command line (accepts either the
//!    directory containing `shell.qml` or the `shell.qml` file itself —
//!    we resolve a file argument to its parent directory)
//! 2. `VOXTYPE_OSD_QML_PATH` env var (same accept-either rules)
//! 3. `$XDG_DATA_HOME/voxtype/quickshell/`
//! 4. `~/.local/share/voxtype/quickshell/`
//! 5. `/usr/share/voxtype/quickshell/`
//! 6. `quickshell/` relative to the current directory (development
//!    convenience when running from the repo root)
//!
//! All other CLI arguments pass through to `qs` unchanged.
//!
//! ## Omarchy theme following
//!
//! When the resolved palette source is Omarchy, theme switches apply live:
//! before exec'ing `qs`, the launcher spawns itself with the internal
//! `--theme-follow` flag. That side process watches the Omarchy theme via
//! `notify`, re-resolves the style on each change, and atomically rewrites
//! the runtime JSON — which Quickshell's `FileView` reloads. The launcher
//! can't run the watcher on a thread because `exec` replaces the process,
//! so the follower's lifetime is tied to `qs` through a pipe instead: `qs`
//! inherits the write end across exec, and the follower exits on EOF (i.e.
//! when `qs` exits). Every failure on this path logs and degrades to the
//! old restart-to-retheme behavior; it never blocks the OSD launch.
//!
//! Exit codes:
//! - 2: Quickshell (`qs`) not found on PATH.
//! - 3: No `shell.qml` found in any of the resolved directories.
//! - 1: exec of `qs` failed for some other reason.

use std::env;
use std::fs;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};

use voxtype::config;
use voxtype::osd::config::{OsdConfig, OsdPaletteSource};
use voxtype::osd::style::{self, RuntimeOsdStyle};
use voxtype::osd::theme;

const QS_BIN: &str = "qs";
const SHELL_FILE: &str = "shell.qml";
const SHELL_SUBDIR: &str = "voxtype/quickshell";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let raw_args: Vec<String> = env::args().skip(1).collect();
    if raw_args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if raw_args.iter().any(|a| a == "--version" || a == "-V") {
        println!("voxtype-osd-quickshell {}", voxtype::cli::VERSION);
        return ExitCode::SUCCESS;
    }

    let (cli_qml_path, cli_style, config_path, daemonize, rest) = parse_args(&raw_args);

    // Internal mode: run as the Omarchy theme follower for an already
    // launched qs (see the module docs). Never reached by user invocation.
    if raw_args.iter().any(|a| a == "--theme-follow") {
        return run_theme_follow(cli_style, config_path);
    }

    let shell_dir = match resolve_shell_dir(cli_qml_path) {
        Some(p) => p,
        None => {
            eprintln!(
                "voxtype-osd-quickshell: could not find '{SHELL_FILE}' for the Quickshell OSD.\n\
                 \n\
                 Searched:\n    \
                     --qml-path <PATH>\n    \
                     $VOXTYPE_OSD_QML_PATH\n    \
                     $XDG_DATA_HOME/{SHELL_SUBDIR}/\n    \
                     ~/.local/share/{SHELL_SUBDIR}/\n    \
                     /usr/share/{SHELL_SUBDIR}/\n    \
                     ./quickshell/\n\
                 \n\
                 Install voxtype's Quickshell files (e.g. `voxtype setup quickshell`)\n\
                 or pass `--qml-path /path/to/quickshell/` explicitly."
            );
            return ExitCode::from(3);
        }
    };

    if which::which(QS_BIN).is_err() {
        eprintln!(
            "voxtype-osd-quickshell: '{QS_BIN}' (Quickshell) is not installed on PATH.\n\
             \n\
             Install it from your distro's package manager:\n    \
                 sudo pacman -S quickshell        # Arch / Omarchy\n    \
                 nix profile install nixpkgs#quickshell  # NixOS\n\
             \n\
             Or switch to a different OSD frontend:\n    \
                 voxtype config set osd.frontend gtk4"
        );
        return ExitCode::from(2);
    }

    tracing::info!(
        shell_dir = %shell_dir.display(),
        daemonize,
        "launching Quickshell OSD"
    );

    let env_style = env::var("VOXTYPE_OSD_STYLE").ok();
    let style_override = cli_style.as_deref().or(env_style.as_deref());
    let config = match config::load_config(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("voxtype-osd-quickshell: failed to load config: {e}");
            return ExitCode::from(3);
        }
    };
    let runtime_style = match style::resolve_runtime_style(&config.osd, style_override) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("voxtype-osd-quickshell: failed to resolve OSD style: {e}");
            return ExitCode::from(3);
        }
    };
    let style_file = match style::write_runtime_style(&runtime_style) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("voxtype-osd-quickshell: failed to write OSD style runtime file: {e}");
            return ExitCode::from(3);
        }
    };

    spawn_theme_follower(&runtime_style, cli_style.as_deref(), config_path.as_deref());

    let mut cmd = Command::new(QS_BIN);
    if daemonize {
        cmd.arg("-d");
    }
    cmd.env("VOXTYPE_OSD_STYLE_FILE", &style_file);
    cmd.arg("-p").arg(&shell_dir).args(&rest);
    let err = cmd.exec();
    eprintln!(
        "voxtype-osd-quickshell: failed to exec '{QS_BIN}' with shell dir '{}': {err}",
        shell_dir.display()
    );
    ExitCode::from(1)
}

fn print_help() {
    println!(
        "voxtype-osd-quickshell {} — launcher for the Quickshell-based voxtype OSD\n\
         \n\
         USAGE:\n    \
             voxtype-osd-quickshell [--qml-path PATH] [QUICKSHELL ARGS...]\n\
         \n\
         OPTIONS:\n    \
             --qml-path <PATH>    Override the Quickshell config directory.\n\
                                  Accepts either the directory containing\n\
                                  shell.qml or the shell.qml file itself.\n    \
             --style <STYLE>      Override [osd] style. Accepts \"default\",\n\
                                  a package name, or a package path.\n    \
             --config <FILE>      Read voxtype config from FILE.\n    \
             --daemonize          Pass `-d` to qs (default). qs forks and\n\
                                  detaches from the controlling terminal so\n\
                                  the OSD survives a short-lived parent\n\
                                  shell or hotkey invocation.\n    \
             --no-daemonize       Do NOT pass `-d` to qs. Use this when a\n\
                                  supervisor wants to keep qs attached to a\n\
                                  child-process slot (e.g. the daemon's OSD\n\
                                  supervisor relies on this so kill_on_drop\n\
                                  reaches qs on shutdown).\n    \
             --theme-follow       Internal. Run as the Omarchy theme follower\n\
                                  the launcher spawns alongside qs.\n    \
             -h, --help           Show this message.\n    \
             -V, --version        Show version.\n\
         \n\
         If both --daemonize and --no-daemonize appear, the last one wins.\n\
         \n\
         All other arguments are forwarded to `qs` after `-d -p <dir>`.\n\
         \n\
         ENV:\n    \
             VOXTYPE_OSD_QML_PATH   Same as --qml-path.\n    \
             VOXTYPE_OSD_STYLE      Same as --style.\n    \
             VOXTYPE_CONFIG         Path to voxtype config.toml.\n",
        voxtype::cli::VERSION,
    );
}

/// Spawn the theme-follower side process and hand qs the write end of its
/// lifetime pipe. Best effort: every failure logs and returns, leaving the
/// OSD launch untouched.
fn spawn_theme_follower(
    runtime_style: &RuntimeOsdStyle,
    cli_style: Option<&str>,
    config_path: Option<&Path>,
) {
    if runtime_style.palette != OsdPaletteSource::Omarchy {
        return;
    }
    if theme::omarchy_current_dirs().is_empty() {
        tracing::debug!("no Omarchy install detected; skipping theme follower");
        return;
    }
    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            tracing::warn!(error = %e, "cannot resolve own executable; theme changes need an OSD restart");
            return;
        }
    };
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        let e = std::io::Error::last_os_error();
        tracing::warn!(error = %e, "pipe for theme follower failed; theme changes need an OSD restart");
        return;
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);

    let mut cmd = Command::new(exe);
    cmd.arg("--theme-follow");
    if let Some(s) = cli_style {
        cmd.arg("--style").arg(s);
    }
    if let Some(c) = config_path {
        cmd.arg("--config").arg(c);
    }
    // SAFETY: read_fd is a fresh pipe fd owned by nothing else; Stdio takes
    // ownership and closes it in this process after the spawn.
    cmd.stdin(unsafe { Stdio::from_raw_fd(read_fd) });
    cmd.stdout(Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            // Clear CLOEXEC so qs inherits the write end across exec; the
            // follower sees EOF (and exits) when qs closes it by exiting.
            unsafe { libc::fcntl(write_fd, libc::F_SETFD, 0) };
            tracing::info!(pid = child.id(), "started Omarchy theme follower");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to start theme follower; theme changes need an OSD restart");
            unsafe { libc::close(write_fd) };
        }
    }
}

/// Run as the theme follower (internal `--theme-follow` mode): watch the
/// Omarchy theme, re-resolve the style on each change, and rewrite the
/// runtime JSON when the result differs. Exits when stdin hits EOF, which
/// happens when the qs process holding our pipe's write end goes away.
fn run_theme_follow(cli_style: Option<String>, config_path: Option<PathBuf>) -> ExitCode {
    // Detach from the launcher's session: a hotkey terminal closing must
    // not HUP us. Lifetime comes solely from the stdin pipe.
    unsafe { libc::setsid() };

    let env_style = env::var("VOXTYPE_OSD_STYLE").ok();
    let style_override = cli_style.or(env_style);
    let config = match config::load_config(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "theme follower: failed to load config; exiting");
            return ExitCode::SUCCESS;
        }
    };
    let initial = match style::resolve_runtime_style(&config.osd, style_override.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "theme follower: failed to resolve OSD style; exiting");
            return ExitCode::SUCCESS;
        }
    };

    let path = style::runtime_style_path();
    let last_json = Arc::new(Mutex::new(fs::read_to_string(&path).unwrap_or_default()));

    let osd = config.osd.clone();
    let handle = {
        let (osd, style_override, path, last_json) = (
            osd.clone(),
            style_override.clone(),
            path.clone(),
            Arc::clone(&last_json),
        );
        style::follow_omarchy_theme(&initial, move || {
            refresh_style(&osd, style_override.as_deref(), &path, &last_json);
        })
    };
    let Some(_handle) = handle else {
        tracing::debug!("theme follower: nothing to watch; exiting");
        return ExitCode::SUCCESS;
    };

    // Catch a theme switch that slipped in between the launcher's initial
    // write and our watches being established.
    refresh_style(&osd, style_override.as_deref(), &path, &last_json);

    let mut buf = [0u8; 64];
    let mut stdin = std::io::stdin().lock();
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    tracing::debug!("theme follower: qs exited; shutting down");
    ExitCode::SUCCESS
}

/// Re-resolve the style with a fresh Omarchy theme load and rewrite the
/// runtime JSON when it changed. Errors log and leave the last good JSON
/// in place.
fn refresh_style(
    osd: &OsdConfig,
    style_override: Option<&str>,
    path: &Path,
    last_json: &Mutex<String>,
) {
    let resolved = match style::resolve_runtime_style(osd, style_override) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "theme follower: failed to re-resolve OSD style");
            return;
        }
    };
    let mut last = last_json
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match style::rewrite_runtime_style_if_changed(path, &resolved, &mut last) {
        Ok(true) => tracing::info!("theme follower: applied Omarchy theme change"),
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "theme follower: failed to rewrite OSD style"),
    }
}

/// Strip our own flags out of `args`:
///
/// - `--qml-path X` / `--qml-path=X`: resolve to a `PathBuf`.
/// - `--daemonize` / `--no-daemonize`: set the daemonize flag. Last one
///   wins so callers can override an upstream default by appending the
///   opposite flag at the end (the dispatcher relies on this when it
///   appends `--no-daemonize` to the user's argv).
///
/// Anything left over is passed through to `qs` unchanged. The returned
/// `daemonize` defaults to `true` (the bug-fix in #395).
fn parse_args(
    args: &[String],
) -> (
    Option<PathBuf>,
    Option<String>,
    Option<PathBuf>,
    bool,
    Vec<String>,
) {
    let mut qml: Option<PathBuf> = None;
    let mut style: Option<String> = None;
    let mut config: Option<PathBuf> = env::var("VOXTYPE_CONFIG").ok().map(PathBuf::from);
    let mut daemonize = true;
    let mut rest: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--qml-path" {
            if let Some(v) = args.get(i + 1) {
                qml = Some(PathBuf::from(v));
                i += 2;
                continue;
            }
            rest.push(a.clone());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--qml-path=") {
            qml = Some(PathBuf::from(v));
            i += 1;
        } else if a == "--style" {
            if let Some(v) = args.get(i + 1) {
                style = Some(v.clone());
                i += 2;
                continue;
            }
            rest.push(a.clone());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--style=") {
            style = Some(v.to_string());
            i += 1;
        } else if a == "--config" {
            if let Some(v) = args.get(i + 1) {
                config = Some(PathBuf::from(v));
                i += 2;
            } else {
                rest.push(a.clone());
                i += 1;
            }
        } else if let Some(v) = a.strip_prefix("--config=") {
            config = Some(PathBuf::from(v));
            i += 1;
        } else if a == "--daemonize" {
            daemonize = true;
            i += 1;
        } else if a == "--no-daemonize" {
            daemonize = false;
            i += 1;
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }
    (qml, style, config, daemonize, rest)
}

/// Normalize a user-supplied path into the directory containing
/// `shell.qml`, validating that the file exists. Accepts either the
/// directory itself or the `shell.qml` file inside it (in which case we
/// return its parent). Returns `None` if neither resolves to a real
/// `shell.qml`.
fn dir_with_shell(p: &Path) -> Option<PathBuf> {
    if p.is_dir() && p.join(SHELL_FILE).is_file() {
        return Some(p.to_path_buf());
    }
    if p.is_file() && p.file_name().map(|n| n == SHELL_FILE).unwrap_or(false) {
        if let Some(parent) = p.parent() {
            if parent.join(SHELL_FILE).is_file() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

fn resolve_shell_dir(cli: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = cli {
        if let Some(dir) = dir_with_shell(&p) {
            return Some(dir);
        }
    }
    if let Ok(env_path) = env::var("VOXTYPE_OSD_QML_PATH") {
        if let Some(dir) = dir_with_shell(Path::new(&env_path)) {
            return Some(dir);
        }
    }
    for base in candidate_data_dirs() {
        let candidate = base.join(SHELL_SUBDIR);
        if let Some(dir) = dir_with_shell(&candidate) {
            return Some(dir);
        }
    }
    // Development convenience: running `cargo run --bin voxtype-osd-quickshell`
    // from the repo root should find the QML tree without installing.
    let dev_candidate = Path::new("quickshell");
    if let Some(dir) = dir_with_shell(dev_candidate) {
        return Some(dir);
    }
    None
}

fn candidate_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            dirs.push(PathBuf::from(xdg));
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share"));
    }
    dirs.push(PathBuf::from("/usr/share"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_qml_path_space_form() {
        let args = vec!["--qml-path".into(), "/tmp/x".into(), "extra".into()];
        let (q, style, _, d, rest) = parse_args(&args);
        assert_eq!(q.as_deref(), Some(Path::new("/tmp/x")));
        assert!(style.is_none());
        assert!(d, "daemonize defaults to true");
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_qml_path_equals_form() {
        let args = vec!["--qml-path=/tmp/y".into(), "extra".into()];
        let (q, _, _, d, rest) = parse_args(&args);
        assert_eq!(q.as_deref(), Some(Path::new("/tmp/y")));
        assert!(d);
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_qml_path_absent() {
        let args = vec!["--width-px".into(), "400".into()];
        let (q, _, _, d, rest) = parse_args(&args);
        assert!(q.is_none());
        assert!(d, "daemonize defaults to true");
        assert_eq!(rest, vec!["--width-px".to_string(), "400".to_string()]);
    }

    #[test]
    fn parse_qml_path_dangling_flag() {
        let args = vec!["--qml-path".into()];
        let (q, _, _, d, rest) = parse_args(&args);
        // Dangling `--qml-path` with no value is passed through so the
        // child (which won't recognise it) errors out clearly rather than
        // being silently dropped.
        assert!(q.is_none());
        assert!(d);
        assert_eq!(rest, vec!["--qml-path".to_string()]);
    }

    #[test]
    fn parse_daemonize_default_true() {
        // No flag at all → daemonize stays true (the v0.7.3 default).
        let args: Vec<String> = vec!["--width-px".into(), "400".into()];
        let (_, _, _, d, rest) = parse_args(&args);
        assert!(d);
        // The pass-through arg is preserved verbatim for qs.
        assert_eq!(rest, vec!["--width-px".to_string(), "400".to_string()]);
    }

    #[test]
    fn parse_no_daemonize_strips_flag_and_clears_default() {
        let args = vec!["--no-daemonize".into(), "extra".into()];
        let (_, _, _, d, rest) = parse_args(&args);
        assert!(!d, "--no-daemonize must turn off daemonize");
        assert_eq!(
            rest,
            vec!["extra".to_string()],
            "--no-daemonize must be stripped from the pass-through args"
        );
    }

    #[test]
    fn parse_daemonize_explicit_flag_stripped() {
        let args = vec!["--daemonize".into(), "extra".into()];
        let (_, _, _, d, rest) = parse_args(&args);
        assert!(d);
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_daemonize_flags_last_wins() {
        // --daemonize then --no-daemonize → no-daemonize wins.
        let args = vec!["--daemonize".into(), "--no-daemonize".into()];
        let (_, _, _, d, rest) = parse_args(&args);
        assert!(!d, "last flag wins: --no-daemonize at the end");
        assert!(rest.is_empty());

        // Reverse: --no-daemonize then --daemonize → daemonize wins.
        let args = vec!["--no-daemonize".into(), "--daemonize".into()];
        let (_, _, _, d, rest) = parse_args(&args);
        assert!(d, "last flag wins: --daemonize at the end");
        assert!(rest.is_empty());
    }

    #[test]
    fn parse_style_strips_own_flag() {
        let args = vec!["--style".into(), "bars-plus".into(), "extra".into()];
        let (_, style, _, d, rest) = parse_args(&args);
        assert_eq!(style.as_deref(), Some("bars-plus"));
        assert!(d);
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_config_strips_own_arg() {
        let args = vec!["--config".into(), "/tmp/config.toml".into(), "extra".into()];
        let (_, _, config, _, rest) = parse_args(&args);
        assert_eq!(config.as_deref(), Some(Path::new("/tmp/config.toml")));
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn dir_with_shell_accepts_directory() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(SHELL_FILE), "").unwrap();
        let resolved = dir_with_shell(tmp.path()).unwrap();
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn dir_with_shell_accepts_file_and_returns_parent() {
        let tmp = tempdir().unwrap();
        let shell = tmp.path().join(SHELL_FILE);
        fs::write(&shell, "").unwrap();
        let resolved = dir_with_shell(&shell).unwrap();
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn dir_with_shell_rejects_missing() {
        let tmp = tempdir().unwrap();
        assert!(dir_with_shell(tmp.path()).is_none());
    }

    #[test]
    fn dir_with_shell_rejects_non_shell_qml_file() {
        let tmp = tempdir().unwrap();
        let other = tmp.path().join("other.qml");
        fs::write(&other, "").unwrap();
        assert!(dir_with_shell(&other).is_none());
    }
}
