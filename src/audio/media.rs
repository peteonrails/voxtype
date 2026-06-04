//! MPRIS media player control via direct D-Bus.
//!
//! Pauses playing players when recording starts and resumes only the
//! players we actually paused. The previous implementation shelled out
//! to `playerctl`, which had two failure modes that hit real users:
//!
//!   * `playerctl -l` silently filters out some MPRIS-compliant players
//!     (e.g. `cliamp`) even when they expose a complete MPRIS interface
//!     on the bus, so voxtype never tried to pause them.
//!   * The resume path called `playerctl --player <stored-name> play`
//!     and ignored the exit code. If the player's bus name had gone
//!     away during the dictation window (Chromium's PID-suffixed names
//!     are particularly fragile), the resume silently no-opped and the
//!     user's music stayed paused. See Omarchy issue #6029.
//!
//! Talking D-Bus directly via zbus fixes both: we enumerate all owned
//! names matching `org.mpris.MediaPlayer2.*` ourselves and surface real
//! errors on resume.

#[cfg(target_os = "linux")]
mod imp {
    use tracing::{debug, warn};
    use zbus::{fdo::DBusProxy, Connection, Proxy};

    const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
    const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
    const MPRIS_IFACE: &str = "org.mpris.MediaPlayer2.Player";

    /// Pause all currently playing MPRIS media players.
    /// Returns the bus names of players that were paused so they can be resumed.
    /// Suffixes in `ignored` are matched against the part after the MPRIS prefix
    /// either exactly or as a `<entry>.<instance>` prefix (so `"chromium"`
    /// matches `chromium.instance1872063`).
    pub async fn pause_playing_players(ignored: &[String]) -> Vec<String> {
        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to connect to session bus: {e}");
                return Vec::new();
            }
        };

        let players = match list_mpris_players(&conn, ignored).await {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to enumerate MPRIS players: {e}");
                return Vec::new();
            }
        };

        if players.is_empty() {
            debug!("No MPRIS players found");
            return Vec::new();
        }

        let mut paused = Vec::new();
        for bus_name in players {
            match player_status(&conn, &bus_name).await {
                Ok(status) if status == "Playing" => {
                    debug!(player = %bus_name, "Pausing media player");
                    match call_player(&conn, &bus_name, "Pause").await {
                        Ok(()) => paused.push(bus_name),
                        Err(e) => warn!(player = %bus_name, "Pause failed: {e}"),
                    }
                }
                Ok(status) => {
                    debug!(player = %bus_name, %status, "Skipping non-playing player")
                }
                Err(e) => debug!(player = %bus_name, "Status query failed: {e}"),
            }
        }

        if !paused.is_empty() {
            debug!("Paused {} media player(s)", paused.len());
        }
        paused
    }

    /// Resume previously-paused MPRIS media players.
    pub async fn resume_players(players: Vec<String>) {
        if players.is_empty() {
            return;
        }
        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to connect to session bus for resume: {e}");
                return;
            }
        };

        let mut resumed = 0usize;
        for bus_name in &players {
            debug!(player = %bus_name, "Resuming media player");
            match call_player(&conn, bus_name, "Play").await {
                Ok(()) => resumed += 1,
                Err(e) => warn!(
                    player = %bus_name,
                    "Resume failed (player may have exited during dictation): {e}"
                ),
            }
        }

        debug!("Resumed {}/{} media player(s)", resumed, players.len());
    }

    async fn list_mpris_players(
        conn: &Connection,
        ignored: &[String],
    ) -> zbus::Result<Vec<String>> {
        let dbus = DBusProxy::new(conn).await?;
        let names = dbus.list_names().await?;
        let mut out = Vec::new();
        for n in names {
            let s: &str = n.as_str();
            if !s.starts_with(MPRIS_PREFIX) {
                continue;
            }
            let suffix = &s[MPRIS_PREFIX.len()..];
            // Skip playerctld's aggregator: pausing it would double-fire
            // pause across every underlying player.
            if suffix == "playerctld" {
                continue;
            }
            if ignored.iter().any(|ig| {
                suffix == ig
                    || suffix.starts_with(ig) && suffix.as_bytes().get(ig.len()) == Some(&b'.')
            }) {
                debug!(player = %suffix, "Ignored by config");
                continue;
            }
            out.push(s.to_string());
        }
        Ok(out)
    }

    async fn player_status(conn: &Connection, bus_name: &str) -> zbus::Result<String> {
        let proxy = Proxy::new(conn, bus_name, MPRIS_PATH, MPRIS_IFACE).await?;
        proxy.get_property::<String>("PlaybackStatus").await
    }

    async fn call_player(
        conn: &Connection,
        bus_name: &str,
        method: &'static str,
    ) -> zbus::Result<()> {
        let proxy = Proxy::new(conn, bus_name, MPRIS_PATH, MPRIS_IFACE).await?;
        proxy.call::<_, _, ()>(method, &()).await
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn list_skips_non_mpris_and_playerctld() {
            // We can't easily fake a real session bus, but we can at least
            // exercise the filter logic by running against the live bus and
            // confirming playerctld and ignored entries never appear.
            let Ok(conn) = Connection::session().await else {
                eprintln!("skip: no session bus");
                return;
            };
            let players = list_mpris_players(&conn, &["chromium".to_string()])
                .await
                .unwrap_or_default();
            for p in players {
                assert!(p.starts_with(MPRIS_PREFIX));
                let suffix = &p[MPRIS_PREFIX.len()..];
                assert_ne!(suffix, "playerctld");
                assert!(
                    !(suffix == "chromium" || suffix.starts_with("chromium.")),
                    "ignored prefix leaked: {p}"
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::{pause_playing_players, resume_players};

// On non-Linux targets MPRIS doesn't apply. Keep the public API stable
// so the daemon doesn't need to cfg-gate every call site.
#[cfg(not(target_os = "linux"))]
pub async fn pause_playing_players(_ignored: &[String]) -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "linux"))]
pub async fn resume_players(_players: Vec<String>) {}
