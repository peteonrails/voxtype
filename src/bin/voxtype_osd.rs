//! `voxtype-osd` — a tiny launcher that picks between the `voxtype-osd-gtk4`,
//! `voxtype-osd-native`, and `voxtype-osd-quickshell` frontends and execs
//! the chosen one.
//!
//! The user's preference comes from (in priority order):
//!
//! 1. `--frontend gtk4|native|quickshell` on the command line
//! 2. `VOXTYPE_OSD_FRONTEND=gtk4|native|quickshell` env var
//! 3. `[osd] frontend = "gtk4|native|quickshell"` in `~/.config/voxtype/config.toml`
//! 4. Default: `gtk4`
//!
//! That preference is then reconciled with what's actually installed: the
//! preferred frontend's binary is searched alongside this launcher and on
//! `$PATH`; if missing, the launcher tries the other installed frontends
//! in a fixed fallback order and warns about the substitution. If nothing
//! is installed at all the launcher exits with a clear error pointing to
//! the build feature flags.
//!
//! Source builders who only enabled one of `osd-gtk4`/`osd-native`/
//! `osd-quickshell` thus get a working `voxtype-osd` regardless of config
//! — the launcher adapts to what was actually built.
//!
//! All other CLI args + env vars pass through unchanged to the chosen
//! frontend (including `--config`, which the GTK4 and native frontends
//! consume on their own to read the rest of the `[osd]` section; the
//! Quickshell frontend reads the same config via its launcher).
//!
//! ## Supervisor opt-out for Quickshell daemonize
//!
//! `voxtype-osd-quickshell` passes `-d` to `qs` by default so that
//! hotkey/CLI invocations survive a short-lived parent shell (see issue
//! #395). The daemon's OSD supervisor in `src/osd/supervisor.rs` needs
//! the opposite behavior: it uses `kill_on_drop(true)` to kill the OSD
//! on shutdown, which only works if qs stays attached to its child slot.
//!
//! The supervisor sets `VOXTYPE_OSD_SUPERVISED=1` on the dispatcher's
//! environment. When this dispatcher sees that env var AND it's about to
//! exec the Quickshell launcher, it appends `--no-daemonize` to the
//! launcher's args so qs stays in the foreground.

use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use voxtype::config::Config as VoxtypeConfig;
use voxtype::osd::config::{OsdConfig, OsdFrontend};

const NATIVE_BIN: &str = "voxtype-osd-native";
const GTK4_BIN: &str = "voxtype-osd-gtk4";
const QUICKSHELL_BIN: &str = "voxtype-osd-quickshell";

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
        println!("voxtype-osd {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    // Strip `--frontend X` and `--frontend=X` from the args so the chosen
    // child doesn't choke on a flag it doesn't know. `--config` and
    // everything else passes through.
    let (cli_frontend, config_path, rest) = parse_frontend_and_config(&raw_args);

    // Resolve preference: CLI > env > config file > default.
    let preferred = cli_frontend
        .or_else(|| {
            env::var("VOXTYPE_OSD_FRONTEND")
                .ok()
                .and_then(|s| OsdFrontend::parse_str(&s))
        })
        .unwrap_or_else(|| load_frontend_from_config(config_path.as_deref()));

    let chosen = match resolve_installed(preferred) {
        Some(c) => c,
        None => {
            eprintln!(
                "voxtype-osd: none of '{GTK4_BIN}', '{NATIVE_BIN}', or '{QUICKSHELL_BIN}' \
                 was found on PATH or next to this binary.\n\
                 \n\
                 If you built from source, enable at least one OSD feature:\n\
                   cargo build --release --features osd-gtk4       # GTK4 frontend\n\
                   cargo build --release --features osd-native     # SCTK + wgpu + egui\n\
                   cargo build --release --bin voxtype-osd-quickshell  # Quickshell launcher\n\
                 \n\
                 If you installed a package, the OSD binaries may be a separate\n\
                 optional dependency."
            );
            return ExitCode::from(2);
        }
    };

    if chosen.frontend != preferred {
        tracing::warn!(
            "preferred frontend '{}' not installed; using '{}' instead",
            preferred.binary_name(),
            chosen.frontend.binary_name(),
        );
    }

    // Build the final argv. If the supervisor invoked us AND we're
    // dispatching to the Quickshell launcher, append `--no-daemonize` so
    // qs stays attached to its child slot for `kill_on_drop`.
    let supervised = env::var("VOXTYPE_OSD_SUPERVISED")
        .map(|v| v == "1")
        .unwrap_or(false);
    let final_args = build_child_args(chosen.frontend, supervised, &rest);

    // Hand off. exec replaces this process so the child inherits stdin,
    // stdout, stderr, signals, and process group cleanly. There's no return
    // path on success.
    let err = Command::new(&chosen.path).args(&final_args).exec();
    eprintln!(
        "voxtype-osd: failed to exec '{}': {err}",
        chosen.path.display()
    );
    ExitCode::from(1)
}

fn print_help() {
    println!(
        "voxtype-osd {} — launcher for the on-screen mic visualizer\n\
         \n\
         USAGE:\n    \
             voxtype-osd [--frontend gtk4|native|quickshell] [FRONTEND ARGS...]\n\
         \n\
         OPTIONS:\n    \
             --frontend <gtk4|native|quickshell>\n    \
                                          Which frontend to launch. Falls back to\n\
                                          whatever is installed if the preferred\n\
                                          frontend isn't found on PATH.\n    \
             -h, --help                   Show this message.\n    \
             -V, --version                Show version.\n\
         \n\
         All other arguments are passed through to the chosen frontend\n\
         (--config, --width-px, --waveform-gain, etc.). See the frontend's\n\
         own --help for the full list.\n\
         \n\
         CONFIG:\n    \
             [osd]\n    \
             frontend = \"gtk4\"  # or \"native\", \"quickshell\"\n\
         \n\
         ENV:\n    \
             VOXTYPE_OSD_FRONTEND   Same as --frontend.\n    \
             VOXTYPE_CONFIG         Path to the voxtype config file.\n",
        env!("CARGO_PKG_VERSION"),
    );
}

/// Strip `--frontend X`/`--frontend=X` out of `args`, returning the chosen
/// frontend (if any) and the remaining args to pass through to the child.
/// Also sniff `--config X`/`--config=X` so we know which file to read for
/// the `[osd]` section without consuming it from the pass-through args
/// (the child needs to see `--config` too).
fn parse_frontend_and_config(
    args: &[String],
) -> (Option<OsdFrontend>, Option<PathBuf>, Vec<String>) {
    let mut frontend: Option<OsdFrontend> = None;
    let mut config: Option<PathBuf> = None;
    let mut rest: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--frontend" {
            if let Some(v) = args.get(i + 1) {
                frontend = OsdFrontend::parse_str(v);
                i += 2;
                continue;
            }
            // `--frontend` with no value: pass through and let the child
            // (which doesn't know it) error out properly.
            rest.push(a.clone());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--frontend=") {
            frontend = OsdFrontend::parse_str(v);
            i += 1;
        } else if a == "--config" {
            rest.push(a.clone());
            if let Some(v) = args.get(i + 1) {
                config = Some(PathBuf::from(v));
                rest.push(v.clone());
                i += 2;
            } else {
                i += 1;
            }
        } else if let Some(v) = a.strip_prefix("--config=") {
            config = Some(PathBuf::from(v));
            rest.push(a.clone());
            i += 1;
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }
    (frontend, config, rest)
}

/// Build the argv we hand to the chosen frontend. Currently only the
/// Quickshell launcher needs special handling: when the daemon
/// supervisor invokes us we append `--no-daemonize` so qs stays attached
/// to its child slot for `kill_on_drop`. We append (rather than prepend)
/// so the user's own `--daemonize`/`--no-daemonize` in `rest` always
/// loses to the supervisor's intent. The Quickshell launcher uses
/// last-wins flag resolution.
fn build_child_args(frontend: OsdFrontend, supervised: bool, rest: &[String]) -> Vec<String> {
    let mut out: Vec<String> = rest.to_vec();
    if supervised && frontend == OsdFrontend::Quickshell {
        out.push("--no-daemonize".into());
    }
    out
}

/// Load the `[osd] frontend` value from the voxtype config file, falling
/// back to the default when the file is missing, unreadable, or doesn't
/// contain a usable value.
fn load_frontend_from_config(explicit: Option<&Path>) -> OsdFrontend {
    let path = explicit
        .map(Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path);
    let Some(path) = path else {
        return OsdFrontend::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return OsdFrontend::default();
    };

    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        osd: Option<OsdConfig>,
    }

    match toml::from_str::<PartialConfig>(&content) {
        Ok(p) => p.osd.map(|o| o.frontend).unwrap_or_default(),
        Err(_) => OsdFrontend::default(),
    }
}

struct ResolvedFrontend {
    frontend: OsdFrontend,
    path: PathBuf,
}

/// Find the binary for `preferred`; if missing, fall back to the next
/// available frontend in a fixed order. Quickshell sits at the bottom of
/// non-Quickshell chains because users who didn't ask for it usually
/// don't have `qs` installed; conversely a Quickshell user who's missing
/// their launcher falls back to Gtk4 (the default) before Native.
fn resolve_installed(preferred: OsdFrontend) -> Option<ResolvedFrontend> {
    let chain: [OsdFrontend; 3] = match preferred {
        OsdFrontend::Gtk4 => [
            OsdFrontend::Gtk4,
            OsdFrontend::Native,
            OsdFrontend::Quickshell,
        ],
        OsdFrontend::Native => [
            OsdFrontend::Native,
            OsdFrontend::Gtk4,
            OsdFrontend::Quickshell,
        ],
        OsdFrontend::Quickshell => [
            OsdFrontend::Quickshell,
            OsdFrontend::Gtk4,
            OsdFrontend::Native,
        ],
    };
    chain.into_iter().find_map(|f| {
        find_binary(f.binary_name()).map(|path| ResolvedFrontend { frontend: f, path })
    })
}

/// Locate a binary by name. First checks alongside `voxtype-osd` itself
/// (so `target/release/voxtype-osd` finds `target/release/voxtype-osd-gtk4`
/// during development) and then walks `$PATH`.
fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(self_exe) = env::current_exe() {
        if let Some(parent) = self_exe.parent() {
            let candidate = parent.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which::which(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_frontend_flag_space_form() {
        let args = vec![
            "--frontend".into(),
            "native".into(),
            "--width-px".into(),
            "400".into(),
        ];
        let (f, _, rest) = parse_frontend_and_config(&args);
        assert_eq!(f, Some(OsdFrontend::Native));
        assert_eq!(rest, vec!["--width-px".to_string(), "400".to_string()]);
    }

    #[test]
    fn parse_strips_frontend_flag_equals_form() {
        let args = vec!["--frontend=gtk4".into(), "--width-px".into(), "400".into()];
        let (f, _, rest) = parse_frontend_and_config(&args);
        assert_eq!(f, Some(OsdFrontend::Gtk4));
        assert_eq!(rest, vec!["--width-px".to_string(), "400".to_string()]);
    }

    #[test]
    fn parse_passes_config_through() {
        let args = vec![
            "--config".into(),
            "/tmp/foo.toml".into(),
            "--width-px".into(),
            "400".into(),
        ];
        let (_, cfg, rest) = parse_frontend_and_config(&args);
        assert_eq!(cfg.as_deref(), Some(Path::new("/tmp/foo.toml")));
        // --config + value still in rest so the child reads it too.
        assert_eq!(
            rest,
            vec![
                "--config".to_string(),
                "/tmp/foo.toml".to_string(),
                "--width-px".to_string(),
                "400".to_string(),
            ]
        );
    }

    #[test]
    fn build_child_args_appends_no_daemonize_when_supervised_quickshell() {
        let rest = vec!["--width-px".to_string(), "400".to_string()];
        let out = build_child_args(OsdFrontend::Quickshell, true, &rest);
        assert_eq!(
            out,
            vec![
                "--width-px".to_string(),
                "400".to_string(),
                "--no-daemonize".to_string(),
            ]
        );
    }

    #[test]
    fn build_child_args_no_op_for_quickshell_unsupervised() {
        let rest = vec!["--width-px".to_string(), "400".to_string()];
        let out = build_child_args(OsdFrontend::Quickshell, false, &rest);
        assert_eq!(out, rest);
    }

    #[test]
    fn build_child_args_no_op_for_gtk4_supervised() {
        // The supervisor flag must not bleed into the GTK4 or Native
        // frontends — they have no `--no-daemonize` flag and would
        // error out on an unknown arg.
        let rest = vec!["--width-px".to_string(), "400".to_string()];
        let out = build_child_args(OsdFrontend::Gtk4, true, &rest);
        assert_eq!(out, rest);
    }

    #[test]
    fn build_child_args_no_op_for_native_supervised() {
        let rest = vec!["--width-px".to_string(), "400".to_string()];
        let out = build_child_args(OsdFrontend::Native, true, &rest);
        assert_eq!(out, rest);
    }

    #[test]
    fn parse_unknown_frontend_value_drops_it() {
        // Bad value is a parse error: returns None for frontend, but doesn't
        // pass `--frontend nonsense` through to the child either.
        let args = vec!["--frontend".into(), "nonsense".into()];
        let (f, _, rest) = parse_frontend_and_config(&args);
        assert_eq!(f, None);
        assert!(rest.is_empty());
    }
}
