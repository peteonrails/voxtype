//! The meeting views' level meters, fed by a real daemon's sound check.
//!
//! The daemon runs with the audio tools faked as in `meeting_soundcheck.rs`:
//! `parec` plays a sine (0.1 on the echo canceller's mic source, 0.5 on the
//! loopback), so the levels on meeting.sock are -23.0 dBFS on the mic and
//! -9.0 on the call. `voxtype meeting soundcheck` asks the daemon to run the
//! check, and each view must show those levels:
//!
//! - the terminal view (`meeting watch`'s screen, drawn with ratatui's test
//!   backend from the socket's real lines): 3 of 5 cells for the mic, 4 for
//!   the call;
//! - the GTK4 window (with `--features meetings-gtk4`, under Broadway): its
//!   two level bars at 0.62 and 0.85.

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use voxtype::meeting::view::client::{spawn_feed, Feed};

struct World {
    dir: tempfile::TempDir,
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl World {
    /// `None` when python3, which plays the fake parec, is missing.
    fn new() -> Option<Self> {
        let python = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .any(|d| Path::new(&d).join("python3").is_file());
        if !python {
            return None;
        }
        let w = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        let bin = w.p("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("sine.py"),
            r#"import sys, math, struct, time
dev = sys.argv[sys.argv.index("--device") + 1] if "--device" in sys.argv else ""
amp = 0.1 if "aec" in dev else 0.5
out = sys.stdout.buffer
t = 0
while True:
    block = b"".join(struct.pack("<f", amp * math.sin(2 * math.pi * 440 * (t + i) / 16000)) for i in range(1600))
    t += 1600
    try:
        out.write(block); out.flush()
    except BrokenPipeError:
        break
    time.sleep(0.1)
"#,
        )
        .unwrap();
        common::write_script(
            &bin.join("parec"),
            &format!(
                "#!/bin/sh\nexec python3 -I '{}' \"$@\"\n",
                bin.join("sine.py").display()
            ),
        );
        common::write_script(
            &bin.join("pactl"),
            "#!/bin/sh\ncase \"$*\" in\n  \"list short sources\") printf '1\\tvoxtype_meeting_aec_source\\tPipeWire\\tfloat32le 1ch 48000Hz\\tRUNNING\\n' ;;\nesac\n",
        );
        common::write_script(&bin.join("pipewire"), "#!/bin/sh\nexec sleep 1000\n");
        let config = w.p("config/voxtype");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.toml"),
            "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n\n[whisper]\non_demand_loading = true\n\n\
             [meeting]\nenabled = true\nshow_view_on_start = false\n\n\
             [meeting.audio]\nloopback_device = \"fake.monitor\"\necho_cancel = \"webrtc\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(w.p("run/voxtype")).unwrap();
        Some(w)
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn env(&self, cmd: &mut Command) {
        let path = format!(
            "{}:{}",
            self.p("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        cmd.env_clear()
            .env("PATH", path)
            .env("HOME", self.dir.path())
            .env("XDG_RUNTIME_DIR", self.p("run"))
            .env("XDG_CONFIG_HOME", self.p("config"))
            .env("XDG_DATA_HOME", self.p("data"))
            .env("XDG_STATE_HOME", self.p("state"))
            .env("XDG_CACHE_HOME", self.p("cache"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::null());
    }

    fn voxtype(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        self.env(&mut cmd);
        cmd.args(args);
        cmd
    }

    fn socket(&self) -> PathBuf {
        self.p("run/voxtype/meeting.sock")
    }

    /// The daemon, once its meeting.sock answers.
    fn daemon(&self) -> Running {
        let daemon = Running(
            self.voxtype(&["daemon"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        while std::os::unix::net::UnixStream::connect(self.socket()).is_err() {
            assert!(started.elapsed() < Duration::from_secs(30), "no daemon");
            std::thread::sleep(Duration::from_millis(50));
        }
        daemon
    }

    /// `meeting soundcheck` for `seconds`, in the background.
    fn soundcheck(&self, seconds: u32) -> Running {
        Running(
            self.voxtype(&["meeting", "soundcheck", "--seconds", &seconds.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}

#[test]
fn the_terminal_view_meters_a_sound_check() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use voxtype::tui::meeting_watch::{draw, WatchApp};

    let Some(w) = World::new() else {
        eprintln!("skipping: python3 not found for the fake parec");
        return;
    };
    let _daemon = w.daemon();
    let (tx, rx) = mpsc::channel();
    let feed = spawn_feed(w.socket(), tx);
    let mut app = WatchApp::new();
    let take = |app: &mut WatchApp, rx: &mpsc::Receiver<Feed>| {
        while let Ok(f) = rx.try_recv() {
            app.on_feed(f);
        }
    };
    // Connected and idle before the check starts.
    let started = Instant::now();
    while !app.live.connected {
        assert!(started.elapsed() < Duration::from_secs(10), "no hello");
        std::thread::sleep(Duration::from_millis(50));
        take(&mut app, &rx);
    }
    let _check = w.soundcheck(5);

    let mut screen = String::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
        take(&mut app, &rx);
        let mut terminal = Terminal::new(TestBackend::new(100, 6)).unwrap();
        terminal
            .draw(|f| draw(f, &mut app, chrono::Utc::now()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        screen = (0..6)
            .map(|y| {
                (0..100)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The mic and call levels arrive as separate events, so wait for the
        // call meter too; breaking on the label alone raced the second event.
        if screen.contains("Sound check") && screen.contains("call ▮") {
            break;
        }
    }
    feed.stop();
    assert!(app.live.soundcheck, "the soundcheck event arrived");
    let first = screen.lines().next().unwrap_or_default();
    assert!(
        first.contains("Meetings  Sound check  mic ▮▮▮▯▯ call ▮▮▮▮▯"),
        "{screen}"
    );
    let l = app.live.levels.as_ref().unwrap();
    assert_eq!((l.mic, l.loopback), (Some(-23.0), Some(-9.0)));
}

#[cfg(feature = "meetings-gtk4")]
#[test]
fn the_gtk4_window_meters_a_sound_check() {
    // The mic and call levels the fake parec produces, as 0 to 1 meters.
    const MIC: f64 = (60.0 - 23.0) / 60.0;
    const CALL: f64 = (60.0 - 9.0) / 60.0;
    let Some(w) = World::new() else {
        eprintln!("skipping: python3 not found for the fake parec");
        return;
    };
    let mut broadway = Command::new("gtk4-broadwayd");
    broadway
        .arg(":44")
        .env("XDG_RUNTIME_DIR", w.p("run"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(broadway) = broadway.spawn() else {
        eprintln!("skipping: gtk4-broadwayd is not available");
        return;
    };
    let _broadway = Running(broadway);
    std::thread::sleep(Duration::from_millis(500));
    let _daemon = w.daemon();

    let mut window = Command::new(env!("CARGO_BIN_EXE_voxtype-meetings-gtk4"));
    w.env(&mut window);
    window
        .env("GDK_BACKEND", "broadway")
        .env("BROADWAY_DISPLAY", ":44")
        .env("VOXTYPE_MEETINGS_GTK4_SMOKE", "4")
        .envs(std::env::var_os("VOXTYPE_MEETINGS_GTK4_SHOT_DIR").map(|d| {
            (
                "VOXTYPE_MEETINGS_GTK4_SHOT",
                PathBuf::from(d).join("meetings-gtk4-soundcheck.png"),
            )
        }))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let window = window.spawn().unwrap();
    // Let it connect, then check for longer than it waits to report.
    std::thread::sleep(Duration::from_millis(1500));
    let _check = w.soundcheck(6);
    let out = window.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .rfind(|l| l.starts_with('{'))
            .unwrap_or_else(|| panic!("no report: {stdout}")),
    )
    .unwrap();
    assert_eq!(report["screen"], "list", "{report}");
    assert_eq!(report["state"], "Sound check", "{report}");
    assert_eq!(report["levels_visible"], true, "{report}");
    let mic = report["mic_level"].as_f64().unwrap();
    let call = report["call_level"].as_f64().unwrap();
    assert!((mic - MIC).abs() < 0.01, "mic {mic}, want {MIC}: {report}");
    assert!(
        (call - CALL).abs() < 0.01,
        "call {call}, want {CALL}: {report}"
    );
}
