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
//! Exit codes:
//! - 2: Quickshell (`qs`) not found on PATH.
//! - 3: No `shell.qml` found in any of the resolved directories.
//! - 1: exec of `qs` failed for some other reason.

use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use voxtype::config;
use voxtype::osd::style;

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
        println!("voxtype-osd-quickshell {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let (cli_qml_path, cli_style, config_path, daemonize, rest) = parse_args(&raw_args);

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
        env!("CARGO_PKG_VERSION"),
    );
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
