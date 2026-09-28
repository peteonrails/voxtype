//! `voxtype learn` — teach `[text.replacements]` from an edited dictation.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

use voxtype::config_set;
use voxtype::notification;
use voxtype::text::learn::{self, LearnDiff};

use super::config_set::resolve_config_path_for_write;

#[derive(Clone, Copy)]
pub(crate) enum LearnSource {
    /// Wayland primary selection, falling back to the clipboard.
    Selection,
    Clipboard,
    Stdin,
}

pub(crate) async fn run_learn(
    cli_config: Option<PathBuf>,
    from_clipboard: bool,
    from_stdin: bool,
) -> anyhow::Result<()> {
    let source = if from_stdin {
        LearnSource::Stdin
    } else if from_clipboard {
        LearnSource::Clipboard
    } else {
        // Default, including an explicit --from-selection.
        LearnSource::Selection
    };

    let corrected = match read_corrected(source) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            eprintln!("error: corrected text is empty");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let Some(transcript) = learn::load_last_transcript() else {
        eprintln!(
            "error: no last transcript found.\n  \
             Dictate something first, or check that the voxtype daemon is logging."
        );
        std::process::exit(1);
    };

    match learn::diff_replacements(&transcript, &corrected) {
        LearnDiff::Identical => {
            println!("Nothing to learn: text matches the last transcript.");
            Ok(())
        }
        LearnDiff::TooDifferent { ratio } => {
            eprintln!(
                "error: corrected text is too different from the last transcript \
                 (similarity {ratio:.2} < {:.2}). Refusing to learn.",
                learn::MIN_SIMILARITY
            );
            std::process::exit(1);
        }
        LearnDiff::NoReplacements => {
            println!("Nothing to learn: no word replacements found (inserts/deletes only).");
            Ok(())
        }
        LearnDiff::Replacements(pairs) => {
            let path = resolve_config_path_for_write(cli_config)?;
            let written = match config_set::merge_replacements(path, &pairs) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };

            for (from, to) in &pairs {
                println!("Learned \"{from}\" = \"{to}\"");
            }
            println!("Wrote {} in {}", pairs.len(), written.display());

            restart_daemon();
            notify_learned(&pairs).await;
            Ok(())
        }
    }
}

fn read_corrected(source: LearnSource) -> anyhow::Result<String> {
    match source {
        LearnSource::Stdin => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
        LearnSource::Clipboard => read_clipboard().ok_or_else(|| {
            anyhow::anyhow!("could not read clipboard (need wl-paste, xclip, or pbpaste)")
        }),
        LearnSource::Selection => read_primary_then_clipboard().ok_or_else(|| {
            anyhow::anyhow!(
                "could not read primary selection or clipboard (need wl-paste or xclip)"
            )
        }),
    }
}

fn read_primary_then_clipboard() -> Option<String> {
    let primary = run_stdout("wl-paste", &["--primary"])
        .or_else(|| run_stdout("xclip", &["-selection", "primary", "-o"]));
    if let Some(text) = primary {
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    read_clipboard()
}

fn read_clipboard() -> Option<String> {
    run_stdout("wl-paste", &[])
        .or_else(|| run_stdout("xclip", &["-selection", "clipboard", "-o"]))
        .or_else(|| run_stdout("pbpaste", &[]))
}

fn run_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(bin).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn restart_daemon() {
    #[cfg(target_os = "linux")]
    {
        let active = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "voxtype"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !active {
            println!("Daemon is not running; replacements apply on next start.");
            return;
        }
        let restart = Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .status();
        match restart {
            Ok(s) if s.success() => println!("Restarted voxtype daemon."),
            _ => {
                eprintln!(
                    "warning: could not restart daemon. Restart manually: \
                     systemctl --user restart voxtype"
                );
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Restart voxtype to apply the new replacements.");
    }
}

async fn notify_learned(pairs: &[(String, String)]) {
    let body = pairs
        .iter()
        .map(|(from, to)| format!("\"{from}\" → \"{to}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let title = if pairs.len() == 1 {
        "Voxtype learned a replacement"
    } else {
        "Voxtype learned replacements"
    };
    notification::send(title, &body).await;
}
