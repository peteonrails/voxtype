//! MPRIS media player control via playerctl
//!
//! Pauses playing media players when recording starts and resumes
//! only the players that were actually playing when recording stops.

use tokio::process::Command;
use tracing::{debug, warn};

/// Pause all currently playing MPRIS media players.
/// Returns the names of players that were paused, so they can be resumed later.
pub async fn pause_playing_players() -> Vec<String> {
    // List players that are currently in "Playing" status
    let output = match Command::new("playerctl")
        .args(["--all-players", "-f", "{{playerName}}", "status"])
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            warn!("playerctl not found or failed to run: {}", e);
            return Vec::new();
        }
    };

    if !output.status.success() {
        // playerctl exits non-zero when no players are running
        debug!("No MPRIS players found");
        return Vec::new();
    }

    // playerctl with --all-players returns one status per line, but we need
    // the player names paired with their statuses. Use a separate call to get
    // player names and statuses together.
    let players_output = match Command::new("playerctl")
        .args([
            "--all-players",
            "-f",
            "{{playerName}}\t{{status}}",
            "status",
        ])
        .output()
        .await
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&players_output.stdout);
    let mut paused_players = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, status)) = line.split_once('\t') {
            if status == "Playing" {
                debug!(player = %name, "Pausing media player");
                let result = Command::new("playerctl")
                    .args(["--player", name, "pause"])
                    .output()
                    .await;
                if let Err(e) = result {
                    warn!(player = %name, "Failed to pause player: {}", e);
                } else {
                    paused_players.push(name.to_string());
                }
            }
        }
    }

    if !paused_players.is_empty() {
        debug!("Paused {} media player(s)", paused_players.len());
    }

    paused_players
}

/// Resume the specified MPRIS media players.
pub async fn resume_players(players: Vec<String>) {
    for name in &players {
        debug!(player = %name, "Resuming media player");
        let result = Command::new("playerctl")
            .args(["--player", name, "play"])
            .output()
            .await;
        if let Err(e) = result {
            warn!(player = %name, "Failed to resume player: {}", e);
        }
    }

    if !players.is_empty() {
        debug!("Resumed {} media player(s)", players.len());
    }
}
