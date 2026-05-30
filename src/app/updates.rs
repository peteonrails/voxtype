//! `voxtype check-update` — compare current version against the latest
//! GitHub release. One-shot startup-time call, so we hit the API directly
//! rather than caching.

/// Check for updates by comparing version with GitHub releases
pub(crate) async fn check_for_updates() -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Voxtype Update Check\n");
    println!("====================\n");
    println!("Current version: {}", current);
    println!("Checking for updates...\n");

    // Fetch latest release from GitHub API (blocking call wrapped in spawn_blocking).
    // ureq::Error is ~272 bytes; boxing the closure's Result would require an extra
    // allocation just to satisfy clippy on a one-shot startup-time call.
    #[allow(clippy::result_large_err)]
    let result = tokio::task::spawn_blocking(|| {
        ureq::get("https://api.github.com/repos/peteonrails/voxtype/releases/latest")
            .set("User-Agent", "voxtype-update-checker")
            .call()
    })
    .await?;

    match result {
        Ok(resp) => {
            let release: serde_json::Value = resp.into_json()?;
            if let Some(tag) = release["tag_name"].as_str() {
                let latest = tag.trim_start_matches('v');

                // Compare versions using semver
                let current_ver = semver::Version::parse(current)
                    .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
                let latest_ver = semver::Version::parse(latest)
                    .unwrap_or_else(|_| semver::Version::new(0, 0, 0));

                if latest_ver > current_ver {
                    println!(
                        "\x1b[33m⚠ Update available: {} → {}\x1b[0m\n",
                        current, latest
                    );
                    println!(
                        "Download: https://github.com/peteonrails/voxtype/releases/tag/{}",
                        tag
                    );
                    println!("Website:  https://voxtype.io/download");

                    // Show release notes excerpt if available
                    if let Some(body) = release["body"].as_str() {
                        let summary: String = body.lines().take(5).collect::<Vec<_>>().join("\n");
                        if !summary.is_empty() {
                            println!("\nRelease notes:");
                            println!("{}", summary);
                            if body.lines().count() > 5 {
                                println!("...");
                            }
                        }
                    }
                } else {
                    println!(
                        "\x1b[32m✓ You're on the latest version ({}).\x1b[0m",
                        current
                    );
                }
            } else {
                println!("Could not parse latest version from GitHub.");
            }
        }
        Err(ureq::Error::Status(code, _)) => {
            eprintln!("GitHub API returned status: {}", code);
            eprintln!("Try again later or check manually: https://github.com/peteonrails/voxtype/releases");
        }
        Err(e) => {
            eprintln!("Failed to check for updates: {}", e);
            eprintln!("Check manually: https://github.com/peteonrails/voxtype/releases");
        }
    }

    Ok(())
}
