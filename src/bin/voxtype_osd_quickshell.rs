//! `voxtype-osd-quickshell` — a tiny launcher that finds voxtype's Quickshell
//! OSD entry point (`shell.qml`) and execs `qs -p <path>`.
//!
//! Quickshell (`qs`) is the runtime; voxtype ships its QML shell tree
//! under one of the standard data directories. The launcher resolves
//! the shell path in this order:
//!
//! 1. `--qml-path <PATH>` on the command line
//! 2. `VOXTYPE_OSD_QML_PATH` env var
//! 3. `$XDG_DATA_HOME/voxtype/quickshell/voxtype-osd/shell.qml`
//! 4. `~/.local/share/voxtype/quickshell/voxtype-osd/shell.qml`
//! 5. `/usr/share/voxtype/quickshell/voxtype-osd/shell.qml`
//! 6. `quickshell/voxtype-osd/shell.qml` relative to the current directory
//!    (development convenience when running from the repo root)
//!
//! All other CLI arguments pass through to `qs` unchanged.
//!
//! Exit codes:
//! - 2: Quickshell (`qs`) not found on PATH.
//! - 3: No shell.qml found at any of the resolved paths.
//! - 1: exec of `qs` failed for some other reason.

use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const QS_BIN: &str = "qs";
const SHELL_FILE: &str = "shell.qml";
const SHELL_SUBPATH: &str = "voxtype/quickshell/voxtype-osd/shell.qml";

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

    let (cli_qml_path, rest) = parse_qml_path(&raw_args);

    let qml_path = match resolve_qml_path(cli_qml_path) {
        Some(p) => p,
        None => {
            eprintln!(
                "voxtype-osd-quickshell: could not find '{SHELL_FILE}' for the Quickshell OSD.\n\
                 \n\
                 Searched:\n    \
                     --qml-path <PATH>\n    \
                     $VOXTYPE_OSD_QML_PATH\n    \
                     $XDG_DATA_HOME/{SHELL_SUBPATH}\n    \
                     ~/.local/share/{SHELL_SUBPATH}\n    \
                     /usr/share/{SHELL_SUBPATH}\n    \
                     ./quickshell/voxtype-osd/{SHELL_FILE}\n\
                 \n\
                 Install voxtype's Quickshell files (e.g. `voxtype setup quickshell`)\n\
                 or pass `--qml-path /path/to/shell.qml` explicitly."
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
        qml = %qml_path.display(),
        "launching Quickshell OSD"
    );

    let mut cmd = Command::new(QS_BIN);
    cmd.arg("-p").arg(&qml_path).args(&rest);
    let err = cmd.exec();
    eprintln!(
        "voxtype-osd-quickshell: failed to exec '{QS_BIN}' with shell '{}': {err}",
        qml_path.display()
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
             --qml-path <PATH>    Override the shell.qml path. Default is to\n\
                                  search standard data dirs.\n    \
             -h, --help           Show this message.\n    \
             -V, --version        Show version.\n\
         \n\
         All other arguments are forwarded to `qs` after `-p <shell.qml>`.\n\
         \n\
         ENV:\n    \
             VOXTYPE_OSD_QML_PATH   Same as --qml-path.\n",
        env!("CARGO_PKG_VERSION"),
    );
}

/// Strip `--qml-path X`/`--qml-path=X` out of `args`. Anything left over
/// is passed through to `qs` unchanged.
fn parse_qml_path(args: &[String]) -> (Option<PathBuf>, Vec<String>) {
    let mut qml: Option<PathBuf> = None;
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
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }
    (qml, rest)
}

fn resolve_qml_path(cli: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = cli.filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Ok(env_path) = env::var("VOXTYPE_OSD_QML_PATH") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Some(p);
        }
    }
    for base in candidate_data_dirs() {
        let candidate = base.join(SHELL_SUBPATH);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Development convenience: running `cargo run --bin voxtype-osd-quickshell`
    // from the repo root should find the QML tree without installing.
    let dev_candidate = Path::new("quickshell/voxtype-osd").join(SHELL_FILE);
    if dev_candidate.is_file() {
        return Some(dev_candidate);
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

    #[test]
    fn parse_qml_path_space_form() {
        let args = vec!["--qml-path".into(), "/tmp/x.qml".into(), "extra".into()];
        let (q, rest) = parse_qml_path(&args);
        assert_eq!(q.as_deref(), Some(Path::new("/tmp/x.qml")));
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_qml_path_equals_form() {
        let args = vec!["--qml-path=/tmp/y.qml".into(), "extra".into()];
        let (q, rest) = parse_qml_path(&args);
        assert_eq!(q.as_deref(), Some(Path::new("/tmp/y.qml")));
        assert_eq!(rest, vec!["extra".to_string()]);
    }

    #[test]
    fn parse_qml_path_absent() {
        let args = vec!["--width-px".into(), "400".into()];
        let (q, rest) = parse_qml_path(&args);
        assert!(q.is_none());
        assert_eq!(rest, vec!["--width-px".to_string(), "400".to_string()]);
    }

    #[test]
    fn parse_qml_path_dangling_flag() {
        let args = vec!["--qml-path".into()];
        let (q, rest) = parse_qml_path(&args);
        // Dangling `--qml-path` with no value is passed through so the
        // child (which won't recognise it) errors out clearly rather than
        // being silently dropped.
        assert!(q.is_none());
        assert_eq!(rest, vec!["--qml-path".to_string()]);
    }
}
