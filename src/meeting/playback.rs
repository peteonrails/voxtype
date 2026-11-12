//! `voxtype meeting play`: play a meeting's retained audio from a line or a
//! time, one playback at a time.
//!
//! Playback is `ffmpeg -ss <from> -i <track> -f wav - | <player> -`, where
//! the player is the first of `pw-play` (PipeWire), `paplay` (PulseAudio)
//! and `aplay` (ALSA) found. ffmpeg does the seeking and decoding, since
//! none of the players can seek and not all of them read Opus.
//!
//! The pipeline runs under `sh` in a session of its own, so it outlives the
//! command that started it. Its state lives in the runtime directory:
//!
//! - `meeting_playback.json`: the last playback started (meeting, track,
//!   start point, the process group's PID and a token naming this run)
//! - `meeting_playback.ended`: the token of a playback that ran to its end,
//!   written by the pipeline's shell as it finishes
//! - `meeting_playback`: a trigger telling the daemon to announce the
//!   change on meeting.sock
//!
//! A playback is playing while its process group is alive and has not
//! written its token to the ended file.

use crate::meeting::{retained_audio, MeetingId, MeetingStorage, StorageError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The last playback started, as `meeting play` records it.
pub const STATE_FILE: &str = "meeting_playback.json";
/// The token of a playback that ran to its end.
pub const ENDED_FILE: &str = "meeting_playback.ended";
/// The daemon trigger that announces a playback change on meeting.sock.
pub const TRIGGER: &str = "meeting_playback";

/// `$0` of the pipeline's shell, so `ps` shows what it is.
const SHELL_NAME: &str = "voxtype-meeting-play";

/// The players tried, in order. Each reads WAV on stdin.
const PLAYERS: &[(&str, &[&str])] = &[
    ("pw-play", &["-"]),
    ("paplay", &[]),
    ("aplay", &["-q", "-"]),
];

/// Which retained track to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    /// Both sides together
    Mixed,
    /// Your microphone
    Mic,
    /// The other side
    Loopback,
}

impl Track {
    fn stem(self) -> &'static str {
        match self {
            Track::Mixed => retained_audio::MIXED,
            Track::Mic => retained_audio::MIC,
            Track::Loopback => retained_audio::LOOPBACK,
        }
    }
}

/// The track names `--track` accepts.
pub const TRACK_NAMES: [&str; 3] = ["mixed", "mic", "loopback"];

impl std::str::FromStr for Track {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mixed" => Ok(Track::Mixed),
            "mic" => Ok(Track::Mic),
            "loopback" => Ok(Track::Loopback),
            other => Err(format!(
                "unknown track '{other}': use {}",
                TRACK_NAMES.join(", ")
            )),
        }
    }
}

impl std::fmt::Display for Track {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.stem())
    }
}

/// Why a playback could not start.
#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error(
        "This meeting was recorded without keeping audio. \
         Turn on meeting.retain_audio for future meetings."
    )]
    NoAudio,

    #[error("This meeting has no {track} track. It has: {}", .available.join(", "))]
    NoTrack {
        track: Track,
        available: Vec<String>,
    },

    #[error(
        "Playing meeting audio needs ffmpeg and one of pw-play (PipeWire), paplay \
         (PulseAudio) or aplay (ALSA). Missing: {0}"
    )]
    MissingTools(String),

    #[error("Invalid time '{0}': use seconds (95 or 95.5), mm:ss (1:35) or hh:mm:ss")]
    BadTime(String),

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error("Could not start playback: {0}")]
    Io(#[from] std::io::Error),
}

/// The programs a playback runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    /// The player and the arguments that make it read WAV on stdin
    pub player: Vec<String>,
}

impl Tools {
    /// Find ffmpeg and a player on the user's PATH.
    pub fn detect() -> Result<Self, PlaybackError> {
        Self::find_in(std::env::var_os("PATH").unwrap_or_default().as_os_str())
    }

    /// Find ffmpeg and a player in the directories of `path`, a PATH-style
    /// list.
    pub fn find_in(path: &OsStr) -> Result<Self, PlaybackError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let find = |name: &str| which::which_in(name, Some(path), &cwd).ok();
        let ffmpeg = find("ffmpeg");
        let player = PLAYERS.iter().find_map(|(name, args)| {
            find(name).map(|bin| {
                std::iter::once(bin.to_string_lossy().into_owned())
                    .chain(args.iter().map(|a| a.to_string()))
                    .collect::<Vec<_>>()
            })
        });
        match (ffmpeg, player) {
            (Some(ffmpeg), Some(player)) => Ok(Self { ffmpeg, player }),
            (ffmpeg, player) => {
                let mut missing = Vec::new();
                if ffmpeg.is_none() {
                    missing.push("ffmpeg");
                }
                if player.is_none() {
                    missing.push("a player (pw-play, paplay or aplay)");
                }
                Err(PlaybackError::MissingTools(missing.join(" and ")))
            }
        }
    }
}

/// Where to start playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAt {
    /// The start of this transcript segment
    Segment(u32),
    /// Milliseconds from the start of the meeting
    Time(u64),
}

/// Parse `95`, `95.5`, `1:35` or `1:01:35` into milliseconds.
pub fn parse_time(text: &str) -> Result<u64, PlaybackError> {
    let bad = || PlaybackError::BadTime(text.to_string());
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(bad());
    }
    let (last, whole) = parts.split_last().ok_or_else(bad)?;
    let secs: f64 = last.parse().map_err(|_| bad())?;
    if !secs.is_finite() || secs < 0.0 || (!whole.is_empty() && secs >= 60.0) {
        return Err(bad());
    }
    let mut total = 0u64;
    for (i, part) in whole.iter().enumerate() {
        let n: u64 = part.parse().map_err(|_| bad())?;
        if i > 0 && n >= 60 {
            return Err(bad());
        }
        total = total * 60 + n;
    }
    Ok(total * 60_000 + (secs * 1000.0).round() as u64)
}

/// A playback as `meeting play --status --json` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackStatus {
    /// `playing` or `stopped`
    pub state: String,
    pub meeting_id: Option<String>,
    pub track: Option<Track>,
    /// Where the playback started, in ms from the start of the meeting
    pub from_ms: Option<u64>,
    /// The playback's process group
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    /// Roughly where it is now: `from_ms` plus the time since it started.
    /// Only while playing.
    pub position_ms: Option<u64>,
}

/// What `meeting_playback.json` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlaybackRecord {
    meeting_id: String,
    track: Track,
    from_ms: u64,
    pid: u32,
    /// The shell's start time in clock ticks since boot, from
    /// /proc/<pid>/stat: with `pid`, names this process and no later one
    /// that reuses the PID.
    #[serde(default)]
    start_ticks: u64,
    started_at: DateTime<Utc>,
    token: String,
    file: PathBuf,
}

/// What `meeting play` prints with `--json` once it has started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Started {
    pub meeting_id: String,
    pub track: Track,
    pub from_ms: u64,
    pub pid: u32,
}

/// The retained audio file for `track` in a meeting.
pub fn track_file(
    storage: &MeetingStorage,
    meeting_id: &MeetingId,
    track: Track,
) -> Result<PathBuf, PlaybackError> {
    let dir = storage.get_meeting_path(meeting_id)?;
    let files = retained_audio::retained_files(&dir);
    if files.is_empty() {
        return Err(PlaybackError::NoAudio);
    }
    files
        .iter()
        .find(|(stem, _)| *stem == track.stem())
        .map(|(_, name)| dir.join(name))
        .ok_or_else(|| PlaybackError::NoTrack {
            track,
            available: files.iter().map(|(stem, _)| stem.to_string()).collect(),
        })
}

/// Milliseconds from the start of the meeting for `at`.
pub fn resolve_start(
    storage: &MeetingStorage,
    meeting_id: &MeetingId,
    at: Option<StartAt>,
) -> Result<u64, PlaybackError> {
    match at {
        None => Ok(0),
        Some(StartAt::Time(ms)) => Ok(ms),
        Some(StartAt::Segment(id)) => {
            let transcript = storage.load_transcript(meeting_id)?;
            transcript
                .segments
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.start_ms)
                .ok_or_else(|| {
                    StorageError::UnknownSegment {
                        segment: id,
                        first: transcript.segments.iter().map(|s| s.id).min(),
                        last: transcript.segments.iter().map(|s| s.id).max(),
                    }
                    .into()
                })
        }
    }
}

/// Start playing `file` from `from_ms`, stopping any playback already
/// running. Returns once the pipeline is running in the background.
pub fn start(
    runtime_dir: &Path,
    tools: &Tools,
    meeting_id: &MeetingId,
    track: Track,
    file: &Path,
    from_ms: u64,
) -> Result<Started, PlaybackError> {
    stop(runtime_dir);
    std::fs::create_dir_all(runtime_dir)?;

    let token = format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let log = std::fs::File::create(runtime_dir.join("meeting_playback.log"))?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(
            r#"ffmpeg=$1 from=$2 file=$3 runtime=$4 token=$5; shift 5
"$ffmpeg" -hide_banner -loglevel error -nostdin -ss "$from" -i "$file" -f wav pipe:1 | "$@"
printf %s "$token" > "$runtime/meeting_playback.ended"
: > "$runtime/meeting_playback""#,
        )
        .arg(SHELL_NAME)
        .arg(&tools.ffmpeg)
        .arg(format!("{:.3}", from_ms as f64 / 1000.0))
        .arg(file)
        .arg(runtime_dir)
        .arg(&token)
        .args(&tools.player)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // Its own session: the playback outlives this command and its
    // terminal, and the whole pipeline stops with one signal to the group.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    let pid = child.id();
    let start_ticks = proc_stat(pid).map(|(_, ticks)| ticks).unwrap_or(0);
    // Reaped by init once this command exits; the shell outlives it.
    drop(child);

    let record = PlaybackRecord {
        meeting_id: meeting_id.to_string(),
        track,
        from_ms,
        pid,
        start_ticks,
        started_at: Utc::now(),
        token,
        file: file.to_path_buf(),
    };
    write_record(runtime_dir, &record)?;
    poke(runtime_dir);
    Ok(Started {
        meeting_id: record.meeting_id,
        track,
        from_ms,
        pid,
    })
}

/// Stop the playback if one is running. Returns its status as it was.
pub fn stop(runtime_dir: &Path) -> PlaybackStatus {
    let before = status(runtime_dir);
    if before.state == "playing" {
        if let Some(pid) = before.pid {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
            // Recorded as ended so the status does not depend on how fast
            // the group dies.
            if let Some(record) = read_record(runtime_dir) {
                let _ = std::fs::write(runtime_dir.join(ENDED_FILE), &record.token);
            }
            poke(runtime_dir);
        }
    }
    before
}

/// The current or last playback.
pub fn status(runtime_dir: &Path) -> PlaybackStatus {
    let Some(record) = read_record(runtime_dir) else {
        return PlaybackStatus {
            state: "stopped".to_string(),
            meeting_id: None,
            track: None,
            from_ms: None,
            pid: None,
            started_at: None,
            position_ms: None,
        };
    };
    let ended = std::fs::read_to_string(runtime_dir.join(ENDED_FILE))
        .is_ok_and(|token| token == record.token);
    let playing = !ended && is_running(record.pid, record.start_ticks);
    let position_ms = playing.then(|| {
        let elapsed = (Utc::now() - record.started_at).num_milliseconds().max(0) as u64;
        record.from_ms + elapsed
    });
    PlaybackStatus {
        state: if playing { "playing" } else { "stopped" }.to_string(),
        meeting_id: Some(record.meeting_id),
        track: Some(record.track),
        from_ms: Some(record.from_ms),
        pid: Some(record.pid),
        started_at: Some(record.started_at),
        position_ms,
    }
}

/// Whether the playback shell started as `pid` at `start_ticks` is still
/// running. Matching the start time, not the command line, both rules out a
/// process that reused the PID and works in the moment after exec, before
/// the kernel has filled in the new command line.
fn is_running(pid: u32, start_ticks: u64) -> bool {
    proc_stat(pid).is_some_and(|(state, ticks)| state != 'Z' && ticks == start_ticks)
}

/// A process's state letter and start time (clock ticks since boot).
fn proc_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the parenthesised command name, which may hold spaces:
    // state is the first, starttime (field 22) the twentieth.
    let rest = stat.rsplit_once(')')?.1;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ticks = fields.nth(18)?.parse().ok()?;
    Some((state, ticks))
}

fn read_record(runtime_dir: &Path) -> Option<PlaybackRecord> {
    let json = std::fs::read_to_string(runtime_dir.join(STATE_FILE)).ok()?;
    serde_json::from_str(&json).ok()
}

fn write_record(runtime_dir: &Path, record: &PlaybackRecord) -> std::io::Result<()> {
    let staged = runtime_dir.join(format!(".{}.{}", STATE_FILE, std::process::id()));
    std::fs::write(
        &staged,
        serde_json::to_string(record).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&staged, runtime_dir.join(STATE_FILE))
}

/// Ask the daemon to announce the change on meeting.sock. Without a
/// daemon nothing reads it, and the next daemon start clears it.
fn poke(runtime_dir: &Path) {
    let _ = std::fs::write(runtime_dir.join(TRIGGER), "");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::{data::TranscriptSegment, MeetingMetadata, StorageConfig, Transcript};
    use std::time::{Duration, Instant};

    /// A directory of fake programs: none of them makes a sound.
    struct FakeBin {
        dir: tempfile::TempDir,
    }

    impl FakeBin {
        fn new() -> Self {
            Self {
                dir: tempfile::TempDir::new().unwrap(),
            }
        }

        /// Safe to run at once under parallel tests (no ETXTBSY).
        fn add(&self, name: &str, body: &str) {
            crate::test_support::write_script(
                &self.dir.path().join(name),
                &format!("#!/bin/sh\n{body}\n"),
            );
        }

        /// ffmpeg that records its arguments and writes `bytes` bytes,
        /// then waits `hold` seconds before closing its output.
        fn ffmpeg(&self, bytes: usize, hold: u32) {
            let log = self.log("ffmpeg");
            self.add(
                "ffmpeg",
                &format!(
                    r#"printf '%s\n' "$@" > {log}
head -c {bytes} /dev/zero
sleep {hold}"#
                ),
            );
        }

        /// A player that records its arguments and how many bytes it read.
        fn player(&self, name: &str) {
            let log = self.log(name);
            self.add(
                name,
                &format!(
                    r#"printf '%s\n' "$@" > {log}.args
wc -c | tr -d ' ' > {log}.tmp && mv {log}.tmp {log}.bytes"#
                ),
            );
        }

        fn log(&self, name: &str) -> String {
            self.dir
                .path()
                .join(format!("{name}.log"))
                .display()
                .to_string()
        }

        fn read(&self, file: &str) -> Option<String> {
            std::fs::read_to_string(self.dir.path().join(file)).ok()
        }

        fn tools(&self) -> Result<Tools, PlaybackError> {
            Tools::find_in(self.dir.path().as_os_str())
        }
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A stored meeting with two segments and the given audio files.
    fn meeting(files: &[&str]) -> (tempfile::TempDir, MeetingStorage, MeetingId) {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = MeetingStorage::open(StorageConfig {
            storage_path: dir.path().to_path_buf(),
            retain_audio: true,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        let metadata = MeetingMetadata::new(Some("Play".to_string()));
        let folder = storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        transcript.add_segment(TranscriptSegment::new(4, 1_000, 2_000, "a".into(), 0));
        transcript.add_segment(TranscriptSegment::new(9, 61_250, 63_000, "b".into(), 2));
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        for f in files {
            std::fs::write(folder.join(f), b"x").unwrap();
        }
        (dir, storage, metadata.id)
    }

    #[test]
    fn times_parse_as_seconds_or_clock() {
        assert_eq!(parse_time("95").unwrap(), 95_000);
        assert_eq!(parse_time("95.5").unwrap(), 95_500);
        assert_eq!(parse_time("1:35").unwrap(), 95_000);
        assert_eq!(parse_time("01:02:03.25").unwrap(), 3_723_250);
        for bad in ["", "-3", "1:60", "1:60:00", "a", "1::2", "1:2:3:4", "nan"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn start_points_resolve_from_segments_and_times() {
        let (_d, storage, id) = meeting(&["mixed.opus"]);
        assert_eq!(resolve_start(&storage, &id, None).unwrap(), 0);
        assert_eq!(
            resolve_start(&storage, &id, Some(StartAt::Segment(9))).unwrap(),
            61_250
        );
        assert_eq!(
            resolve_start(&storage, &id, Some(StartAt::Time(5_000))).unwrap(),
            5_000
        );
        let err = resolve_start(&storage, &id, Some(StartAt::Segment(5))).unwrap_err();
        assert!(err.to_string().contains("4 to 9"), "{err}");
    }

    #[test]
    fn a_meeting_without_audio_says_how_to_keep_it() {
        let (_d, storage, id) = meeting(&[]);
        let err = track_file(&storage, &id, Track::Mixed).unwrap_err();
        assert_eq!(
            err.to_string(),
            "This meeting was recorded without keeping audio. \
             Turn on meeting.retain_audio for future meetings."
        );
    }

    #[test]
    fn a_missing_track_names_the_ones_there() {
        let (_d, storage, id) = meeting(&["mic.opus", "mixed.opus"]);
        let err = track_file(&storage, &id, Track::Loopback).unwrap_err();
        assert_eq!(
            err.to_string(),
            "This meeting has no loopback track. It has: mic, mixed"
        );
        let wav = meeting(&["mic.wav", "mixed.wav"]);
        assert!(track_file(&wav.1, &wav.2, Track::Mixed)
            .unwrap()
            .ends_with("mixed.wav"));
    }

    #[test]
    fn missing_tools_are_named() {
        let bin = FakeBin::new();
        let err = bin.tools().unwrap_err().to_string();
        assert!(
            err.ends_with("Missing: ffmpeg and a player (pw-play, paplay or aplay)"),
            "{err}"
        );
        bin.ffmpeg(0, 0);
        let err = bin.tools().unwrap_err().to_string();
        assert!(
            err.ends_with("Missing: a player (pw-play, paplay or aplay)"),
            "{err}"
        );
        bin.player("aplay");
        assert!(bin.tools().unwrap().player[0].ends_with("/aplay"));
        bin.player("pw-play");
        assert_eq!(bin.tools().unwrap().player[1], "-", "pw-play reads stdin");
    }

    #[test]
    fn plays_the_track_from_the_segment_and_then_stops() {
        let bin = FakeBin::new();
        // Holds a second so the test sees the start's poke before the end's.
        bin.ffmpeg(32_044, 1);
        bin.player("pw-play");
        let runtime = tempfile::TempDir::new().unwrap();
        let (_d, storage, id) = meeting(&["mic.opus", "loopback.opus", "mixed.opus"]);
        let file = track_file(&storage, &id, Track::Mic).unwrap();
        let from = resolve_start(&storage, &id, Some(StartAt::Segment(9))).unwrap();

        let started = start(
            runtime.path(),
            &bin.tools().unwrap(),
            &id,
            Track::Mic,
            &file,
            from,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&started).unwrap(),
            serde_json::json!({
                "meeting_id": id.to_string(), "track": "mic",
                "from_ms": 61_250, "pid": started.pid,
            })
        );
        assert!(runtime.path().join(TRIGGER).exists(), "daemon poked");

        std::fs::remove_file(runtime.path().join(TRIGGER)).unwrap();
        wait_for("the player", || bin.read("pw-play.log.bytes").is_some());
        assert_eq!(bin.read("pw-play.log.bytes").unwrap().trim(), "32044");
        assert_eq!(bin.read("pw-play.log.args").unwrap(), "-\n");
        let ffmpeg_args: Vec<String> = bin
            .read("ffmpeg.log")
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        let after = |flag: &str| {
            let i = ffmpeg_args.iter().position(|a| a == flag).unwrap();
            ffmpeg_args[i + 1].clone()
        };
        assert_eq!(after("-ss"), "61.250");
        assert_eq!(after("-i"), file.display().to_string());
        assert_eq!(after("-f"), "wav");

        // At its end the pipeline records it and pokes the daemon again.
        wait_for("the end", || status(runtime.path()).state == "stopped");
        wait_for("the poke", || runtime.path().join(TRIGGER).exists());
        let s = status(runtime.path());
        assert_eq!(s.meeting_id, Some(id.to_string()));
        assert_eq!(
            (s.track, s.from_ms, s.position_ms),
            (Some(Track::Mic), Some(61_250), None)
        );
    }

    fn alive(pid: u32) -> bool {
        proc_stat(pid).is_some_and(|(state, _)| state != 'Z')
    }

    #[test]
    fn a_process_is_known_by_its_start_time() {
        let (_, ticks) = proc_stat(std::process::id()).unwrap();
        assert!(ticks > 0);
        assert!(is_running(std::process::id(), ticks));
        assert!(!is_running(std::process::id(), ticks + 1), "a reused PID");
        assert!(!is_running(u32::MAX, ticks));
    }

    fn diag(runtime: &Path, bin: &FakeBin, pid: u32) -> String {
        format!(
            "log={:?} ended={:?} stat={:?} cmdline={:?} ffmpeg={:?} paplay={:?}",
            std::fs::read_to_string(runtime.join("meeting_playback.log")),
            std::fs::read_to_string(runtime.join(ENDED_FILE)),
            std::fs::read_to_string(format!("/proc/{pid}/stat")),
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|b| String::from_utf8_lossy(&b).replace('\0', " ")),
            bin.read("ffmpeg.log"),
            bin.read("paplay.log.bytes"),
        )
    }

    #[test]
    fn a_new_playback_stops_the_last_and_stop_stops_it() {
        let bin = FakeBin::new();
        // Holds its output open, as a long track would.
        bin.ffmpeg(100, 30);
        bin.player("paplay");
        let tools = bin.tools().unwrap();
        let runtime = tempfile::TempDir::new().unwrap();
        let (_d, storage, id) = meeting(&["mixed.opus"]);
        let file = track_file(&storage, &id, Track::Mixed).unwrap();

        let first = start(runtime.path(), &tools, &id, Track::Mixed, &file, 0).unwrap();
        let s = status(runtime.path());
        assert_eq!(
            s.state,
            "playing",
            "{}",
            diag(runtime.path(), &bin, first.pid)
        );
        assert!(s.position_ms.unwrap() < 5_000);

        let second = start(runtime.path(), &tools, &id, Track::Mixed, &file, 2_000).unwrap();
        wait_for("the first to die", || !alive(first.pid));
        let s = status(runtime.path());
        assert_eq!(
            (s.state.as_str(), s.pid, s.from_ms),
            ("playing", Some(second.pid), Some(2_000)),
            "{}",
            diag(runtime.path(), &bin, second.pid)
        );

        let stopped = stop(runtime.path());
        assert_eq!(stopped.state, "playing", "reports what it stopped");
        assert_eq!(status(runtime.path()).state, "stopped");
        wait_for("the second to die", || !alive(second.pid));
        // Stopping again is harmless.
        assert_eq!(stop(runtime.path()).state, "stopped");
    }

    #[test]
    fn nothing_played_yet_is_stopped() {
        let runtime = tempfile::TempDir::new().unwrap();
        let s = status(runtime.path());
        assert_eq!(s.state, "stopped");
        assert_eq!(s.meeting_id, None);
        assert_eq!(stop(runtime.path()), s);
    }
}
