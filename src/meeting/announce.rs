//! The recording announcement: "This meeting is being recorded and
//! transcribed with Voxtype", played once when a meeting starts.
//!
//! Off by default (`[meeting] announce_recording`). It is a courtesy, not
//! consent: the laws on recording calls still apply to the user.
//!
//! The people on the call have to hear it, and playing it on the speakers
//! does not get it there: headphones keep it in, and the call app's echo
//! cancellation takes speaker sound out of the mic. So the clip goes into the
//! call app's own microphone stream. PipeWire mixes every link into an input
//! port, so a player node linked straight to the call app's capture stream
//! is heard by that app alongside the real mic, and by nothing else:
//!
//! 1. Find the call: the capture streams of the apps call detection
//!    confirms ([`crate::meeting::detect::call_streams`]), waiting up to
//!    [`WAIT_FOR_CALL`] for one to open the mic.
//! 2. Start `pw-play --target 0` on the clip. Unlinked, the node is never
//!    scheduled, so it holds the clip until it is linked.
//! 3. `pw-link` its output ports to the call stream's input ports, then
//!    check in `pw-dump` that the links exist. The first link starts it.
//! 4. Play the clip on the speakers too, with an ordinary `pw-play`, so the
//!    user hears it.
//!
//! Whatever fails, the clip still plays locally, the reason is logged, and
//! the [`Status`] says it only played locally. It never claims the call
//! heard it when no link to the call was made.
//!
//! The clip comes back on the loopback and maybe the mic, so the meeting
//! keeps it out of the transcript with a [`Window`]: lines in the time the
//! clip played that say the announcement's words lose them.
//!
//! The default clip ships in the binary ([`BUILTIN_CLIP`]), a Kokoro-82M
//! voice that is MIT like the rest of voxtype, so the announcement plays
//! offline. `scripts/render-meeting-announcement.sh` renders it again from
//! [`ANNOUNCEMENT_TEXT`]. The ElevenLabs voice "Jessica" is not in the
//! repository or the binary: it is downloaded once, on request, under its own
//! terms ([`jessica`]), and voxtype never calls a speech service.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{MeetingConfig, MeetingStartOptions};
use crate::meeting::data::TranscriptSegment;
use crate::meeting::detect::pipewire::{Direction, Port};
use crate::meeting::echo::{normalize_word, word_distance};

/// What the built-in clip says. Keep it in step with the clip: re-render
/// with `scripts/render-meeting-announcement.sh` after changing it.
pub const ANNOUNCEMENT_TEXT: &str = "This meeting is being recorded and transcribed with Voxtype.";

/// The built-in clip: [`ANNOUNCEMENT_TEXT`] in the Kokoro-82M voice
/// `af_heart`, 24 kHz mono 16-bit WAV, MIT like the rest of voxtype.
pub const BUILTIN_CLIP: &[u8] =
    include_bytes!("../../assets/sounds/meeting-announcement-kokoro.wav");

/// The file under `assets/sounds/` that [`BUILTIN_CLIP`] embeds.
pub const BUILTIN_CLIP_FILE: &str = "meeting-announcement-kokoro.wav";

pub mod jessica;

/// The built-in announcement's voices (`[meeting] announcement_voice`).
pub const VOICES: &[&str] = crate::cli::ANNOUNCEMENT_VOICES;

/// How long a meeting waits for a call app to open the mic before it plays
/// the announcement on the speakers only.
pub const WAIT_FOR_CALL: Duration = Duration::from_secs(120);

/// How long a call app has to hold the same microphone streams before the
/// announcement plays.
///
/// A call app opens the microphone on its pre-join screen, where nobody on
/// the call can hear it, and opens a new stream when the user joins. Played
/// the moment the first stream appeared, the announcement went into the
/// preview and was over before the user was in the meeting. Waiting for one
/// set of streams to last this long puts it after the join.
pub const SETTLE: Duration = Duration::from_secs(5);

/// How often the graph is looked at while a call is settling.
const SETTLE_POLL: Duration = Duration::from_secs(1);

/// Whether a call has held the same microphone streams for long enough.
#[derive(Debug)]
pub(crate) struct Settle {
    need: Duration,
    /// The streams last seen, and since when
    held: Option<(Vec<u32>, std::time::Instant)>,
}

impl Settle {
    pub(crate) fn new(need: Duration) -> Self {
        Self { need, held: None }
    }

    /// Whether a call is being timed.
    pub(crate) fn settling(&self) -> bool {
        self.held.is_some()
    }

    /// Note the call's capture streams as they are at `now` (`None` when no
    /// call has the microphone) and say whether to announce. A different set
    /// of streams, or none, starts the count again.
    pub(crate) fn observe(&mut self, streams: Option<&[u32]>, now: std::time::Instant) -> bool {
        let Some(streams) = streams else {
            self.held = None;
            return false;
        };
        let mut ids = streams.to_vec();
        ids.sort_unstable();
        match &self.held {
            Some((seen, since)) if *seen == ids => now.duration_since(*since) >= self.need,
            _ => {
                self.held = Some((ids, now));
                self.need.is_zero()
            }
        }
    }
}

/// How often it looks for the call while waiting.
const POLL: Duration = Duration::from_secs(2);

/// The built-in clip, written here in the runtime directory for `pw-play`.
const CLIP_FILE: &str = "meeting-announcement.wav";

/// The runtime file holding the running meeting's [`Status`] as JSON, for
/// `voxtype meeting status`.
pub const STATUS_FILE: &str = "meeting_announcement.json";

/// Which audio the announcement plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    /// [`BUILTIN_CLIP`], the Kokoro voice
    Builtin,
    /// The Jessica voice, downloaded to this path ([`jessica`]). Played
    /// only once [`Clip::ready`] has it, else the built-in voice plays.
    Jessica(PathBuf),
    /// The user's own recording (`announcement_file`)
    File(PathBuf),
}

impl Clip {
    /// The clip a meeting started with `options` announces with, or `None`
    /// when it does not announce. `meeting start` resolves both fields; a
    /// trigger without them (an older CLI, a script) falls back to config.
    pub fn for_meeting(config: &MeetingConfig, options: &MeetingStartOptions) -> Option<Self> {
        if !options.announce.unwrap_or(config.announce_recording) {
            return None;
        }
        let file = options
            .announcement_file
            .as_deref()
            .or(config.announcement_file.as_deref())
            .map(str::trim)
            .filter(|f| !f.is_empty());
        if let Some(f) = file {
            return Some(Clip::File(expand_home(f)));
        }
        let voice = options
            .announcement_voice
            .as_deref()
            .unwrap_or(&config.announcement_voice);
        Some(match voice.trim().to_lowercase().as_str() {
            "jessica" => Clip::Jessica(jessica::cache_path()),
            "kokoro" => Clip::Builtin,
            other => {
                tracing::warn!(
                    "Unknown announcement voice {:?}; using the built-in one",
                    other
                );
                Clip::Builtin
            }
        })
    }

    /// The clip ready to play, and why it is not the one asked for, when it
    /// isn't: the user's file and the built-in voice are always ready; the
    /// Jessica voice is the cached copy when it checks out, else a download
    /// through `download`, else the built-in voice.
    pub fn ready(
        self,
        download: impl FnOnce(&Path) -> Result<(), String>,
    ) -> (Clip, Option<String>) {
        self.ready_checked(jessica::SHA256, download)
    }

    /// [`Clip::ready`], checking the Jessica voice against `sha256`.
    pub(crate) fn ready_checked(
        self,
        sha256: &str,
        download: impl FnOnce(&Path) -> Result<(), String>,
    ) -> (Clip, Option<String>) {
        match self {
            Clip::Jessica(path) => match jessica::ensure_sha(&path, sha256, download) {
                Ok(()) => (Clip::Jessica(path), None),
                Err(why) => (
                    Clip::Builtin,
                    Some(format!(
                        "the Jessica voice could not be downloaded ({}), so the built-in voice played",
                        why
                    )),
                ),
            },
            other => (other, None),
        }
    }

    /// What the clip says, when voxtype knows: both voxtype voices say
    /// [`ANNOUNCEMENT_TEXT`]; a user's file is unknown.
    pub fn phrase(&self) -> Option<&'static str> {
        match self {
            Clip::Builtin | Clip::Jessica(_) => Some(ANNOUNCEMENT_TEXT),
            Clip::File(_) => None,
        }
    }

    /// The clip as a file `pw-play` can read: the user's, or the built-in
    /// one written to `runtime_dir`.
    pub fn materialize(&self, runtime_dir: &Path) -> std::io::Result<PathBuf> {
        match self {
            Clip::File(path) | Clip::Jessica(path) => {
                if path.is_file() {
                    Ok(path.clone())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("{} not found", path.display()),
                    ))
                }
            }
            Clip::Builtin => {
                std::fs::create_dir_all(runtime_dir)?;
                let path = runtime_dir.join(CLIP_FILE);
                if std::fs::read(&path).ok().as_deref() != Some(BUILTIN_CLIP) {
                    std::fs::write(&path, BUILTIN_CLIP)?;
                }
                Ok(path)
            }
        }
    }

    /// How long the clip plays, when it is a WAV file; else a guess the
    /// transcript window is corrected from once playback ends.
    pub fn expected_duration(&self) -> Duration {
        let reader = match self {
            Clip::Builtin => hound::WavReader::new(std::io::Cursor::new(BUILTIN_CLIP))
                .ok()
                .map(|r| (r.duration(), r.spec().sample_rate)),
            Clip::File(path) | Clip::Jessica(path) => hound::WavReader::open(path)
                .ok()
                .map(|r| (r.duration(), r.spec().sample_rate)),
        };
        match reader {
            Some((frames, rate)) if rate > 0 => {
                Duration::from_millis(u64::from(frames) * 1000 / u64::from(rate))
            }
            _ => Duration::from_secs(10),
        }
    }
}

/// `~/x` as a path under the home directory.
fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// `voxtype meeting start`'s announcement choices, resolved against the
/// config it read: `--no-announce` beats `--announce` beats the config,
/// `--announcement-file` beats `announcement_file`, and
/// `--announcement-voice` beats `announcement_voice`. The file is made
/// absolute, since the daemon does not share this command's directory, and
/// a missing one refuses the start rather than announcing nothing.
pub fn resolve_start_options(
    config: &MeetingConfig,
    mut options: MeetingStartOptions,
    announce: bool,
    no_announce: bool,
    file: Option<&Path>,
    voice: Option<&str>,
) -> Result<MeetingStartOptions, String> {
    let on = if no_announce {
        false
    } else {
        announce || config.announce_recording
    };
    options.announce = Some(on);
    if !on {
        return Ok(options);
    }
    options.announcement_voice = Some(
        voice
            .unwrap_or(&config.announcement_voice)
            .trim()
            .to_lowercase(),
    );
    let file = match file {
        Some(f) => Some(f.to_path_buf()),
        None => config
            .announcement_file
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(expand_home),
    };
    if let Some(file) = file {
        let file = std::path::absolute(&file).unwrap_or(file);
        if !file.is_file() {
            return Err(format!(
                "Announcement file {} not found. Fix announcement_file in [meeting], or start with --no-announce.",
                file.display()
            ));
        }
        options.announcement_file = Some(file.to_string_lossy().into_owned());
    }
    Ok(options)
}

/// Where the announcement got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting for a call app to open the mic
    Waiting,
    /// Played into the call app's mic stream, and on the speakers
    Call,
    /// Played on the speakers only: the call did not hear it
    Local,
    /// Not played at all
    Failed,
}

/// The announcement's outcome, for `meeting status` and the meeting views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    /// The call app it went to, for [`State::Call`]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Why it only played locally or not at all; empty otherwise
    #[serde(default)]
    pub detail: String,
    /// Why the voice that played is not the one asked for, when it isn't
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Status {
    pub fn waiting() -> Self {
        Self {
            state: State::Waiting,
            app: None,
            detail: String::new(),
            note: None,
        }
    }

    pub fn call(app: &str) -> Self {
        Self {
            state: State::Call,
            app: Some(app.to_string()),
            detail: String::new(),
            note: None,
        }
    }

    pub fn local(why: impl Into<String>) -> Self {
        Self {
            state: State::Local,
            app: None,
            detail: why.into(),
            note: None,
        }
    }

    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            state: State::Failed,
            app: None,
            detail: why.into(),
            note: None,
        }
    }

    /// The same status, noting why the voice that played isn't the one
    /// asked for.
    pub fn with_note(mut self, note: Option<String>) -> Self {
        self.note = note;
        self
    }

    /// One line for `meeting status` and the views.
    pub fn describe(&self) -> String {
        let line = self.describe_state();
        match &self.note {
            Some(note) => format!("{line}. Note: {note}"),
            None => line,
        }
    }

    fn describe_state(&self) -> String {
        match self.state {
            State::Waiting => {
                "waiting for a call app to open the microphone (up to 2 minutes)".to_string()
            }
            State::Call => format!(
                "played into {}'s microphone and on your speakers",
                self.app.as_deref().unwrap_or("the call app")
            ),
            State::Local => format!(
                "played on your speakers only; the call did not hear it ({})",
                self.detail
            ),
            State::Failed => format!("not played ({})", self.detail),
        }
    }

    /// The JSON written for `meeting status`, with [`Status::describe`]'s
    /// line as `text`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        value["text"] = self.describe().into();
        value
    }
}

/// Read the running meeting's announcement status from the runtime dir.
pub fn read_status(runtime_dir: &Path) -> Option<Status> {
    let text = std::fs::read_to_string(runtime_dir.join(STATUS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Which of our player's output ports feed which of the call stream's input
/// ports. A mono clip feeds every input. Otherwise channels pair by name,
/// and an input with no namesake (a mono capture fed a stereo clip) takes
/// our first output, so no input is left without the announcement.
pub fn link_pairs(ours: &[Port], theirs: &[Port]) -> Vec<(u32, u32)> {
    let outputs: Vec<&Port> = ours
        .iter()
        .filter(|p| p.direction == Direction::Out)
        .collect();
    let Some(first) = outputs.first() else {
        return Vec::new();
    };
    theirs
        .iter()
        .filter(|p| p.direction == Direction::In && !p.monitor)
        .map(|input| {
            let source = if outputs.len() == 1 {
                first
            } else {
                outputs
                    .iter()
                    .find(|o| !o.channel.is_empty() && o.channel == input.channel)
                    .unwrap_or(first)
            };
            (source.id, input.id)
        })
        .collect()
}

/// When the announcement played, on the meeting's timeline, and what it
/// said, so the meeting keeps it out of the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub start_ms: u64,
    pub end_ms: u64,
    /// The words it said; `None` for a user's clip, which is then judged by
    /// time alone
    pub phrase: Option<String>,
}

/// How far a line may sit outside the window and still be the
/// announcement: the loopback's and the speech engine's timing slack.
const PHRASE_SLACK_MS: u64 = 2000;
/// The tighter slack for a clip whose words are unknown, where time is
/// all there is to go on.
const TIME_SLACK_MS: u64 = 500;
/// The fewest words of the announcement in a row that are taken out of a
/// line that also holds other words.
const MIN_RUN_WORDS: usize = 3;

impl Window {
    /// Take the announcement out of one chunk's lines: a line that is only
    /// the announcement goes, one that also holds other speech keeps the
    /// rest. Returns how many lines changed or went.
    pub fn scrub(&self, segments: &mut Vec<TranscriptSegment>) -> usize {
        let mut changed = 0;
        segments.retain_mut(|seg| match self.judge(seg) {
            Verdict::Keep => true,
            Verdict::Drop => {
                changed += 1;
                false
            }
            Verdict::Rewrite(rest) => {
                changed += 1;
                seg.text = rest;
                true
            }
        });
        changed
    }

    fn judge(&self, seg: &TranscriptSegment) -> Verdict {
        match &self.phrase {
            Some(phrase) => {
                let near = seg.end_ms + PHRASE_SLACK_MS >= self.start_ms
                    && seg.start_ms <= self.end_ms + PHRASE_SLACK_MS;
                if !near {
                    return Verdict::Keep;
                }
                match strip_phrase(&seg.text, phrase) {
                    None => Verdict::Keep,
                    Some(rest) if rest.is_empty() => Verdict::Drop,
                    Some(rest) => Verdict::Rewrite(rest),
                }
            }
            None => {
                let from = self.start_ms.saturating_sub(TIME_SLACK_MS);
                let to = self.end_ms + TIME_SLACK_MS;
                let overlap = seg.end_ms.min(to).saturating_sub(seg.start_ms.max(from));
                let length = seg.end_ms.saturating_sub(seg.start_ms).max(1);
                if overlap * 2 >= length {
                    Verdict::Drop
                } else {
                    Verdict::Keep
                }
            }
        }
    }
}

enum Verdict {
    Keep,
    Drop,
    Rewrite(String),
}

/// `text` with the announcement's words taken out, `Some("")` when nothing
/// else is left, or `None` when it does not hold the announcement.
///
/// A word counts as the announcement's when it is one of `phrase`'s words,
/// spelled close enough to one to be it misheard, or joined with the next
/// word it makes one ("Vox type"). Runs of [`MIN_RUN_WORDS`] such words are
/// removed; a line made only of them goes whole, however short.
pub fn strip_phrase(text: &str, phrase: &str) -> Option<String> {
    let wanted: Vec<String> = phrase
        .split_whitespace()
        .map(normalize_word)
        .filter(|w| !w.is_empty())
        .collect();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| normalize_word(t)).collect();
    let is_wanted = |w: &str| {
        !w.is_empty()
            && wanted
                .iter()
                .any(|p| p == w || (p.len() >= 4 && w.len() >= 3 && word_distance(p, w) <= 0.34))
    };

    let mut marked = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        if is_wanted(&words[i]) {
            marked[i] = true;
        } else if i + 1 < tokens.len() && is_wanted(&format!("{}{}", words[i], words[i + 1])) {
            marked[i] = true;
            marked[i + 1] = true;
            i += 1;
        }
        i += 1;
    }

    let spoken: Vec<usize> = (0..tokens.len())
        .filter(|&i| !words[i].is_empty())
        .collect();
    if spoken.is_empty() || !spoken.iter().any(|&i| marked[i]) {
        return None;
    }
    if spoken.iter().all(|&i| marked[i]) {
        return Some(String::new());
    }

    // Runs of marked words; tokens with no letters (a lone dash) neither
    // break nor count toward a run.
    let mut remove = vec![false; tokens.len()];
    let mut run: Vec<usize> = Vec::new();
    let flush = |run: &mut Vec<usize>, remove: &mut Vec<bool>| {
        if run.iter().filter(|&&i| !words[i].is_empty()).count() >= MIN_RUN_WORDS {
            for &i in run.iter() {
                remove[i] = true;
            }
        }
        run.clear();
    };
    for i in 0..tokens.len() {
        if marked[i] || (words[i].is_empty() && !run.is_empty()) {
            run.push(i);
        } else {
            flush(&mut run, &mut remove);
        }
    }
    flush(&mut run, &mut remove);
    if !remove.iter().any(|&r| r) {
        return None;
    }
    let rest: Vec<&str> = tokens
        .iter()
        .zip(&remove)
        .filter(|(_, &r)| !r)
        .map(|(t, _)| *t)
        .collect();
    let rest = rest.join(" ");
    if rest.chars().any(char::is_alphanumeric) {
        Some(rest)
    } else {
        Some(String::new())
    }
}

pub use live::{Runner, Update};

/// The announcement as it runs. Off PipeWire (macOS) `pw-play` is missing
/// and it reports [`State::Failed`].
mod live {
    use super::*;
    use crate::config::MeetingDetectConfig;
    use crate::meeting::detect::{self, hyprland, pipewire, MicStream, Snapshot};
    use std::process::Stdio;
    use std::time::Instant;
    use tokio::process::{Child, Command};
    use tokio::sync::mpsc;

    /// How long the player node has to appear in the graph.
    const NODE_TIMEOUT: Duration = Duration::from_secs(3);

    /// Progress the daemon is told about.
    #[derive(Debug, Clone)]
    pub enum Update {
        /// No call yet; waiting for one
        Waiting,
        /// The clip started playing, at `at`, for about `expected`
        Started {
            at: Instant,
            expected: Duration,
            phrase: Option<String>,
        },
        /// Done, one way or another
        Finished { status: Status, ended_at: Instant },
    }

    /// One announcement, run as a task beside the meeting.
    #[derive(Debug, Clone)]
    pub struct Runner {
        pub clip: Clip,
        pub detect: MeetingDetectConfig,
        pub runtime_dir: PathBuf,
        /// How long to wait for a call ([`WAIT_FOR_CALL`])
        pub wait: Duration,
        /// How long the call has to hold its streams first ([`SETTLE`])
        pub settle: Duration,
        /// Where the local copy plays: `None` for the default output, as
        /// PipeWire routes it. Tests send it to a null sink.
        pub local_target: Option<String>,
        /// Where the Jessica voice is downloaded from ([`jessica::url`])
        pub jessica_url: String,
        /// How long that download may take ([`jessica::MEETING_TIMEOUT`])
        pub jessica_timeout: Duration,
    }

    /// The call's capture streams, found in one look at the graph.
    struct Call {
        app: String,
        streams: Vec<MicStream>,
    }

    impl Runner {
        pub fn new(clip: Clip, detect: MeetingDetectConfig, runtime_dir: PathBuf) -> Self {
            Self {
                clip,
                detect,
                runtime_dir,
                wait: WAIT_FOR_CALL,
                settle: SETTLE,
                local_target: None,
                jessica_url: jessica::url(),
                jessica_timeout: jessica::MEETING_TIMEOUT,
            }
        }

        /// Announce, telling `tx` how it goes. The last update is always
        /// [`Update::Finished`].
        pub async fn run(self, tx: mpsc::Sender<Update>) -> Status {
            let status = self.announce(&tx).await;
            match status.state {
                State::Call => tracing::info!("Recording announcement: {}", status.describe()),
                _ => tracing::warn!("Recording announcement: {}", status.describe()),
            }
            let _ = tx
                .send(Update::Finished {
                    status: status.clone(),
                    ended_at: Instant::now(),
                })
                .await;
            status
        }

        async fn announce(&self, tx: &mpsc::Sender<Update>) -> Status {
            let (clip, note) = self.prepared_clip().await;
            self.announce_clip(&clip, tx).await.with_note(note)
        }

        /// The clip to play, after fetching the Jessica voice when it is
        /// wanted and not cached yet. A failed fetch leaves the built-in
        /// voice, with the reason.
        async fn prepared_clip(&self) -> (Clip, Option<String>) {
            let clip = self.clip.clone();
            if !matches!(clip, Clip::Jessica(_)) {
                return (clip, None);
            }
            let url = self.jessica_url.clone();
            let timeout = self.jessica_timeout;
            let fallback = (
                Clip::Builtin,
                Some(
                    "the Jessica voice could not be prepared, so the built-in voice played".into(),
                ),
            );
            tokio::task::spawn_blocking(move || {
                clip.ready(|dest| {
                    tracing::info!(
                        "Recording announcement: downloading the Jessica voice from {}",
                        url
                    );
                    tracing::info!("{}", jessica::TERMS);
                    jessica::fetch(&url, dest, timeout)
                })
            })
            .await
            .unwrap_or(fallback)
        }

        async fn announce_clip(&self, clip: &Clip, tx: &mpsc::Sender<Update>) -> Status {
            let path = match clip.materialize(&self.runtime_dir) {
                Ok(p) => p,
                Err(e) => return Status::failed(format!("cannot read the clip: {}", e)),
            };
            if which::which("pw-play").is_err() {
                return Status::failed(
                    "pw-play is not installed; the announcement needs PipeWire's tools",
                );
            }

            let deadline = Instant::now() + self.wait;
            let mut told_waiting = false;
            let mut settle = Settle::new(self.settle);
            let call = loop {
                match find_call(&self.detect).await {
                    Ok(Some(call)) => {
                        let ids: Vec<u32> = call.streams.iter().map(|s| s.id).collect();
                        let was_settling = settle.settling();
                        if settle.observe(Some(&ids), Instant::now()) {
                            break Ok(call);
                        }
                        if !was_settling {
                            tracing::info!(
                                "Recording announcement: {} has the mic (streams {:?}); \
                                 announcing once it has kept them for {} s",
                                call.app,
                                ids,
                                self.settle.as_secs()
                            );
                        }
                    }
                    Ok(None) => {
                        settle.observe(None, Instant::now());
                    }
                    Err(e) => break Err(e),
                }
                // A call that is settling is not held to the deadline: it
                // is there, and the wait for it is short.
                if settle.settling() {
                    tokio::time::sleep(SETTLE_POLL.min(self.settle)).await;
                    continue;
                }
                if Instant::now() >= deadline {
                    break Err(format!(
                        "no call app opened the microphone within {} s",
                        self.wait.as_secs()
                    ));
                }
                if !told_waiting {
                    tracing::info!(
                        "Recording announcement: waiting for a call app to open the mic"
                    );
                    let _ = tx.send(Update::Waiting).await;
                    told_waiting = true;
                }
                tokio::time::sleep(POLL).await;
            };

            let injected = match call {
                Ok(call) => self.inject(&path, &call).await,
                Err(why) => Err(why),
            };
            let (player, status) = match injected {
                Ok((player, app)) => (Some(player), Status::call(&app)),
                Err(why) => (None, Status::local(why)),
            };

            let local = self.play_local(&path);
            let _ = tx
                .send(Update::Started {
                    at: Instant::now(),
                    expected: clip.expected_duration(),
                    phrase: clip.phrase().map(str::to_string),
                })
                .await;
            let limit = clip.expected_duration() + Duration::from_secs(10);
            let local_ok = match local {
                Ok(child) => finish(child, limit).await,
                Err(e) => {
                    tracing::warn!("Recording announcement: cannot play it locally: {}", e);
                    false
                }
            };
            if let Some(player) = player {
                finish(player, limit).await;
            }
            match (status.state, local_ok) {
                (State::Local, false) => Status::failed(format!(
                    "{}; and it could not play on the speakers",
                    status.detail
                )),
                _ => status,
            }
        }

        /// Start the held player, link it into the call's streams and
        /// confirm the links. Returns the player and the app's name.
        async fn inject(&self, clip: &Path, call: &Call) -> Result<(Child, String), String> {
            let node = format!("voxtype-announce-{}", std::process::id());
            // Full volume, whatever WirePlumber remembers for pw-play
            // streams: a restored volume (0.34 on the test machine) would
            // make the call hear it faint.
            let mut player = Command::new("pw-play")
                .arg("--target")
                .arg("0")
                .arg("--volume")
                .arg("1.0")
                .arg("-P")
                .arg(format!(
                    "{{ node.name = \"{node}\" node.description = \"Voxtype recording announcement\" state.restore-props = false }}"
                ))
                .arg(clip)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("cannot run pw-play: {}", e))?;

            // The node takes a moment to appear, with its ports.
            let started = Instant::now();
            let (graph, ours) = loop {
                let graph = dump().await?;
                if let Some(id) = graph.node_named(&node) {
                    let ours = graph.ports_of(id, Direction::Out);
                    if !ours.is_empty() {
                        break (graph, ours);
                    }
                }
                if started.elapsed() > NODE_TIMEOUT {
                    let _ = player.kill().await;
                    return Err("the announcement player did not appear in PipeWire".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };

            let mut pairs = Vec::new();
            for stream in &call.streams {
                pairs.extend(link_pairs(&ours, &graph.ports_of(stream.id, Direction::In)));
            }
            if pairs.is_empty() {
                let _ = player.kill().await;
                return Err(format!(
                    "{}'s mic stream has no input ports to link",
                    call.app
                ));
            }
            for (out, input) in &pairs {
                let linked = Command::new("pw-link")
                    .arg(out.to_string())
                    .arg(input.to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .output()
                    .await;
                match linked {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => tracing::warn!(
                        "Recording announcement: pw-link {} {} failed: {}",
                        out,
                        input,
                        String::from_utf8_lossy(&o.stderr).trim()
                    ),
                    Err(e) => tracing::warn!("Recording announcement: cannot run pw-link: {}", e),
                }
            }

            // Only links PipeWire reports count. Once linked, the clip
            // plays, and the links go when the player exits.
            let graph = dump().await?;
            let made = pairs
                .iter()
                .filter(|(o, i)| graph.link(*o, *i).is_some_and(|l| l.state != "error"))
                .count();
            if made == 0 {
                let _ = player.kill().await;
                return Err(format!("could not link into {}'s mic stream", call.app));
            }
            tracing::info!(
                "Recording announcement: linked into {} ({} of {} port links, streams {:?})",
                call.app,
                made,
                pairs.len(),
                call.streams.iter().map(|s| s.id).collect::<Vec<_>>()
            );
            Ok((player, call.app.clone()))
        }

        /// The local copy, routed like any other sound.
        fn play_local(&self, clip: &Path) -> std::io::Result<Child> {
            let mut cmd = Command::new("pw-play");
            if let Some(target) = &self.local_target {
                cmd.arg("--target").arg(target);
            }
            cmd.arg("-P")
                .arg(format!(
                    "{{ node.name = \"voxtype-announce-local-{}\" node.description = \"Voxtype recording announcement\" }}",
                    std::process::id()
                ))
                .arg(clip)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
        }
    }

    /// Wait for a player to finish, killing it after `limit`. True when it
    /// exited cleanly.
    async fn finish(mut child: Child, limit: Duration) -> bool {
        match tokio::time::timeout(limit, child.wait()).await {
            Ok(Ok(status)) => status.success(),
            Ok(Err(_)) => false,
            Err(_) => {
                let _ = child.kill().await;
                false
            }
        }
    }

    async fn dump() -> Result<pipewire::Graph, String> {
        let out = Command::new("pw-dump")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("cannot run pw-dump: {}", e))?;
        if !out.status.success() {
            return Err("pw-dump failed; is PipeWire running?".into());
        }
        pipewire::parse_graph(&String::from_utf8_lossy(&out.stdout))
            .map_err(|e| format!("cannot read pw-dump: {}", e))
    }

    /// The call in progress, if any: the first app detection confirms,
    /// with every capture stream of that app.
    async fn find_call(config: &MeetingDetectConfig) -> Result<Option<Call>, String> {
        let graph = dump().await?;
        let snapshot = Snapshot {
            streams: graph.streams,
            windows: hyprland::query_windows().await,
        };
        let found = detect::call_streams(&snapshot, config);
        let Some((first, _)) = found.first() else {
            return Ok(None);
        };
        let app = first.app.clone();
        let streams = found
            .into_iter()
            .filter(|(c, _)| c.app == app)
            .map(|(_, s)| s)
            .collect();
        Ok(Some(Call { app, streams }))
    }
}

#[cfg(test)]
mod tests;
