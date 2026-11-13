//! `voxtype setup migrate`: print, accept, decline or clean up model upgrade
//! offers. The decisions live in `voxtype::model_upgrade`; this file only
//! resolves paths, asks for confirmation and prints.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use voxtype::config::{self, Config};
use voxtype::model_upgrade::migrate::{self, MigrateError, MigratePaths, SystemEnv};
use voxtype::model_upgrade::{self, UpgradeStatus};
use voxtype::setup::progress;

pub(crate) struct MigrateArgs {
    pub json: bool,
    pub accept: Option<String>,
    /// `Some("")` declines every offer.
    pub decline: Option<String>,
    pub cleanup: bool,
    pub yes: bool,
    /// stdout carries NDJSON download events only.
    pub json_progress: bool,
}

fn size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn format_label(format: &str) -> &str {
    match format {
        "onnx" => "ONNX",
        "gguf" => "GGUF",
        "ggml" => "Whisper",
        "openvino" => "OpenVINO",
        _ => "remote",
    }
}

fn print_status_json(status: &UpgradeStatus) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(status)?);
    Ok(())
}

fn print_summary(status: &UpgradeStatus) {
    println!(
        "Current model: {} / {} ({})",
        status.current.engine,
        status.current.model,
        format_label(&status.current.format)
    );
    if status.offers.is_empty() {
        println!("\nNo model upgrade is available for this model.");
    } else {
        println!("\nOffers:");
        for offer in &status.offers {
            let mut tags = Vec::new();
            if offer.recommended {
                tags.push("recommended");
            }
            if offer.declined {
                tags.push("declined");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", tags.join(", "))
            };
            println!("  {}{tags}: {}", offer.id, offer.title);
            println!("      {}", offer.detail);
            if offer.download_bytes > 0 {
                println!(
                    "      Switches to {}, a {} download.",
                    offer.model,
                    size(offer.download_bytes)
                );
            } else {
                println!(
                    "      Switches to {}, which is already downloaded.",
                    offer.model
                );
            }
        }
        println!("\nAccept an offer:     voxtype setup migrate --accept <OFFER>");
        println!("Keep current model:  voxtype setup migrate --decline");
    }
    if let Some(notice) = &status.removal_notice {
        println!("\n{}", notice.message);
    }
    if let Some(cleanup) = &status.cleanup {
        println!(
            "\n{} no longer used ({}): {}. Remove with: voxtype setup migrate --cleanup",
            old_models_label(cleanup),
            size(cleanup.bytes),
            cleanup_names(cleanup)
        );
    }
}

fn fail(error: MigrateError, model: &str) -> ! {
    progress::error(model, &error.message);
    eprintln!("error: {}", error.message);
    std::process::exit(error.code);
}

/// Recompute after a change, from the config file as it now is on disk.
fn reload_status(path: &std::path::Path, fallback: &Config) -> UpgradeStatus {
    let config = config::load_config(Some(path)).unwrap_or_else(|_| fallback.clone());
    model_upgrade::status_for_config(&config)
}

fn confirm(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub(crate) fn run(
    config: &Config,
    cli_config: Option<PathBuf>,
    args: MigrateArgs,
) -> anyhow::Result<()> {
    let status = model_upgrade::status_for_config(config);
    let paths = MigratePaths {
        config: super::config_set::resolve_config_path_for_write(cli_config.clone())?,
        models_dir: Config::models_dir(),
        state: model_upgrade::state_path(),
    };
    let human = !args.json_progress;

    if let Some(id) = args.accept.as_deref() {
        let offer = match migrate::find_offer(&status, id) {
            Ok(offer) => offer.clone(),
            Err(error) => fail(error, id),
        };
        if human {
            println!("{}", offer.title);
            println!("{}", offer.detail);
            if offer.download_bytes > 0 {
                println!("Download: {} ({})", offer.model, size(offer.download_bytes));
            } else {
                println!("{} is already downloaded.", offer.model);
            }
        }
        if !args.yes {
            if !std::io::stdin().is_terminal() {
                fail(
                    MigrateError {
                        message: "Confirmation required. Pass --yes to accept without a prompt. Config unchanged.".into(),
                        code: 2,
                    },
                    &offer.model,
                );
            }
            let question = format!("Switch from {} to {}?", status.current.model, offer.model);
            if !confirm(&question) {
                fail(
                    MigrateError {
                        message: "Cancelled. Config unchanged; the offer stays available.".into(),
                        code: 1,
                    },
                    &offer.model,
                );
            }
        }

        let mut env = SystemEnv::new(config.clone());
        let outcome = match migrate::accept(&mut env, &paths, &status, id) {
            Ok(outcome) => outcome,
            Err(error) => fail(error, &offer.model),
        };
        progress::done(&outcome.model);
        if human {
            println!(
                "\nSwitched to {} / {} in {}.",
                outcome.engine,
                outcome.model,
                paths.config.display()
            );
            if outcome.restarted {
                println!("voxtype restarted and is ready.");
            } else if voxtype::daemon_status::is_daemon_running() {
                println!("Restart voxtype to use the new model.");
            }
            println!(
                "Your old models are kept. After your first dictation with the new model, remove them with: voxtype setup migrate --cleanup"
            );
        }
        if args.json && human {
            print_status_json(&reload_status(&paths.config, config))?;
        }
        return Ok(());
    }

    if let Some(id) = args.decline.as_deref() {
        let id = (!id.is_empty()).then_some(id);
        let declined = match migrate::decline(&paths, &status, id) {
            Ok(declined) => declined,
            Err(error) => fail(error, id.unwrap_or_default()),
        };
        if args.json {
            print_status_json(&model_upgrade::status_for_config(config))?;
            return Ok(());
        }
        if declined.is_empty() {
            println!("No offer to decline. Keeping {}.", status.current.model);
        } else {
            println!(
                "Keeping {}. Declined: {}. The offer stays listed if you change your mind.",
                status.current.model,
                declined.join(", ")
            );
        }
        if let Some(notice) = &status.removal_notice {
            println!("\n{}", notice.message);
        }
        return Ok(());
    }

    if args.cleanup {
        let (model, bytes) = match migrate::cleanup(&paths, &status) {
            Ok(done) => done,
            Err(error) => fail(error, ""),
        };
        if args.json {
            print_status_json(&model_upgrade::status_for_config(config))?;
        } else {
            println!("Removed {model} ({}).", size(bytes));
        }
        return Ok(());
    }

    if args.json {
        print_status_json(&status)?;
        return Ok(());
    }
    print_summary(&status);
    // In a terminal, offer the choices right away: the tray and TUI open one
    // for this. Scripts and pipes get the summary only.
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        if let Some(next) = choose(&status) {
            return run(config, cli_config, next);
        }
    }
    Ok(())
}

/// Interactive menu after the summary. `None` means decide later.
fn choose(status: &UpgradeStatus) -> Option<MigrateArgs> {
    if status.offers.is_empty() && status.cleanup.is_none() {
        return None;
    }
    println!("\nWhat would you like to do?");
    for (index, offer) in status.offers.iter().enumerate() {
        let size = if offer.download_bytes > 0 {
            format!(", {} download", size(offer.download_bytes))
        } else {
            String::new()
        };
        println!("  {}) Switch to {}{size}", index + 1, offer.model);
    }
    if !status.offers.is_empty() {
        println!("  k) Keep {}", status.current.model);
    }
    if let Some(cleanup) = &status.cleanup {
        println!(
            "  c) Remove {} ({})",
            old_models_label(cleanup).to_lowercase(),
            size(cleanup.bytes)
        );
    }
    print!("  Enter) Decide later\n\nChoice: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok()?;
    let answer = answer.trim().to_ascii_lowercase();
    let base = MigrateArgs {
        json: false,
        accept: None,
        decline: None,
        cleanup: false,
        // Choosing an offer from this menu is the confirmation.
        yes: true,
        json_progress: false,
    };
    match answer.as_str() {
        "k" if !status.offers.is_empty() => Some(MigrateArgs {
            decline: Some(String::new()),
            ..base
        }),
        "c" if status.cleanup.is_some() => Some(MigrateArgs {
            cleanup: true,
            ..base
        }),
        number => {
            let index = number.parse::<usize>().ok()?.checked_sub(1)?;
            let offer = status.offers.get(index)?;
            Some(MigrateArgs {
                accept: Some(offer.id.clone()),
                ..base
            })
        }
    }
}

/// "1 old model" / "5 old models" for the cleanup lines.
fn old_models_label(cleanup: &model_upgrade::Cleanup) -> String {
    match cleanup_count(cleanup) {
        1 => "1 old model".to_string(),
        n => format!("{n} old models"),
    }
}

fn cleanup_count(cleanup: &model_upgrade::Cleanup) -> usize {
    cleanup.models.len().max(1)
}

/// Older status objects carry only `model`; newer ones list every model.
fn cleanup_names(cleanup: &model_upgrade::Cleanup) -> String {
    if cleanup.models.is_empty() {
        cleanup.model.clone()
    } else {
        cleanup.models.join(", ")
    }
}
