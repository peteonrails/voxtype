//! A track that carries sound but puts no lines in the transcript.
//!
//! Each chunk of a track passes the speech check
//! ([`VoiceActivityDetector::contains_speech`]) before it is transcribed:
//! 300 ms of 30 ms frames louder than `[meeting.audio] vad_threshold`. A
//! threshold above the track's level fails every chunk, and that whole side
//! of the meeting is missing from the transcript with nothing but a debug
//! line to say so. A 29-minute meeting recorded with `vad_threshold = 0.11`
//! and speech peaking at 0.06 came out with 139 lines from the call and
//! none from the microphone.
//!
//! A quiet track is normal: someone who only listens has a quiet
//! microphone. So this goes by evidence. [`read_chunk`] measures what the
//! speech check skipped, and a chunk counts only when it holds sound that
//! is clearly not the room:
//!
//! - above four times the track's noise floor (its quietest tenth, as the
//!   echo guard measures it) and above half the default threshold, the level
//!   under which [`utterance_regions`](super::chunk::utterance_regions)
//!   already calls a frame silence in a stock configuration;
//! - in runs of 150 ms or more, the echo guard's test for an onset rather
//!   than a click;
//! - on the microphone, away from sound on the call, which the microphone
//!   hears through speakers;
//! - 0.6 s of it in the chunk, the shortest piece the chunker treats as an
//!   utterance and twice what the speech check itself asks for.
//!
//! [`Monitor`] raises the condition when three such chunks fall within a
//! track's last ten with no line among them, and clears it when a line
//! arrives. Only the first raise in a meeting is announced, per track.
//!
//! The announcement is a desktop notification that `[meeting]
//! status_notifications` does not govern, like the other meeting error
//! notices: it reports transcript being lost, and a user who has turned
//! the Meeting Started and Ended notices off still has to hear that.

use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::chunk::{ChunkConfig, VoiceActivityDetector, MIN_SPEECH_MS, MIN_UTTERANCE_SECS};
use super::data::AudioSource;

/// The notification's title, and the heading of its troubleshooting entry.
pub const TITLE: &str = "One side of the meeting is not being transcribed";

/// The title when both sides are raised by the same pair of chunks.
pub const TITLE_BOTH: &str = "Neither side of the meeting is being transcribed";

/// The running meeting's untranscribed tracks, in the runtime directory,
/// for `voxtype meeting status`.
pub const STATUS_FILE: &str = "meeting_untranscribed_tracks.json";

/// `code` of the `error` event meeting.sock carries for this.
pub const EVENT_CODE: &str = "track_not_transcribed";

/// `[meeting.audio] vad_threshold` when it is not set.
pub const DEFAULT_VAD_THRESHOLD: f32 = 0.01;

/// The speech check's analysis frame.
const FRAME_MS: u32 = 30;
/// Sound stands this far above the track's noise floor (12 dB). The echo
/// guard calls anything under three times the floor silence; this is one
/// step clear of that.
const SOUND_OVER_NOISE: f32 = 4.0;
/// ... and above this, whatever the floor: half the default threshold,
/// where `utterance_regions` cuts silence in a stock configuration. Sound
/// that voxtype would call silence with no setting changed is not evidence.
const MIN_SOUND_RMS: f32 = DEFAULT_VAD_THRESHOLD * 0.5;
/// A run of sound this long (150 ms) is an onset, not a click or a key
/// press: the echo guard's `OWN_VOICE_FRAMES`.
const MIN_RUN_FRAMES: usize = 5;
/// The call counts as sounding above this (or four times its own floor):
/// a tenth of the default threshold, since the microphone's gain on what
/// the speakers play is unknown.
const CALL_QUIET_RMS: f32 = DEFAULT_VAD_THRESHOLD * 0.1;
/// How long after the call sounds the microphone may still be hearing it
/// (300 ms): the echo guard's longest echo path.
const ECHO_REACH_FRAMES: usize = 10;
/// Skipped chunks with sound it takes to raise the condition ...
const EVIDENCE_CHUNKS: usize = 3;
/// ... among this many of the track's latest chunks (five minutes at the
/// default chunk length), so three coughs an hour apart never add up.
const WINDOW_CHUNKS: usize = 10;

/// Frames the speech check needs above the threshold: 300 ms.
fn gate_frames() -> usize {
    (MIN_SPEECH_MS / FRAME_MS) as usize
}

/// Frames of sound that make a chunk evidence: one utterance.
fn evidence_frames() -> usize {
    (MIN_UTTERANCE_SECS * 1000.0 / FRAME_MS as f32).ceil() as usize
}

/// What one chunk of one track showed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkReading {
    /// The speech check found no speech, so the chunk was not transcribed
    pub skipped: bool,
    /// The level the chunk's sound reaches, when it has enough to be
    /// evidence: the highest threshold at which the speech check would
    /// still have passed that sound (its tenth-loudest frame)
    pub sound: Option<f32>,
    /// The track's noise floor in this chunk
    pub noise: f32,
}

fn frame_levels(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let frame = ((sample_rate * FRAME_MS / 1000) as usize).max(1);
    samples
        .chunks(frame)
        .map(VoiceActivityDetector::calculate_rms)
        .collect()
}

/// The quietest tenth of the frames: the track with nobody talking.
fn noise_floor(levels: &[f32]) -> f32 {
    let mut sorted = levels.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted.get(sorted.len() / 10).copied().unwrap_or(0.0)
}

/// Measure one chunk of a track the way the speech check sees it.
///
/// `call` is the loopback's chunk for the same stretch when `samples` is
/// the microphone's: microphone sound while the call sounds may be the call
/// heard through speakers, and is left out. Both chunks start together, as
/// [`MeetingDaemon::process_pair`](super::MeetingDaemon::process_pair)
/// feeds them. `None` for a chunk too short to be processed at all.
pub fn read_chunk(
    samples: &[f32],
    call: Option<&[f32]>,
    sample_rate: u32,
    vad_threshold: f32,
) -> Option<ChunkReading> {
    let min_samples =
        (ChunkConfig::default().min_chunk_duration_secs * sample_rate as f32) as usize;
    if samples.len() < min_samples {
        return None;
    }
    let skipped = !VoiceActivityDetector::new(vad_threshold, sample_rate).contains_speech(samples);
    let levels = frame_levels(samples, sample_rate);
    let noise = noise_floor(&levels);
    let floor = (SOUND_OVER_NOISE * noise).max(MIN_SOUND_RMS);

    let call_sounds: Vec<bool> = call
        .map(|call| {
            let levels = frame_levels(call, sample_rate);
            let floor = (SOUND_OVER_NOISE * noise_floor(&levels)).max(CALL_QUIET_RMS);
            levels.iter().map(|&level| level > floor).collect()
        })
        .unwrap_or_default();
    // The call sounding now, up to an echo path ago, or one frame ahead
    // (the two tracks are aligned to the frame at best).
    let heard_from_call = |i: usize| {
        let from = i.saturating_sub(ECHO_REACH_FRAMES);
        let to = (i + 2).min(call_sounds.len());
        from < to && call_sounds[from..to].iter().any(|&s| s)
    };

    // Frames of sound, kept only from runs long enough to be an onset.
    let mut sound: Vec<f32> = Vec::new();
    let mut run_start = 0;
    for (i, &level) in levels.iter().enumerate() {
        if level > floor && !heard_from_call(i) {
            continue;
        }
        if i - run_start >= MIN_RUN_FRAMES {
            sound.extend_from_slice(&levels[run_start..i]);
        }
        run_start = i + 1;
    }
    if levels.len() - run_start >= MIN_RUN_FRAMES {
        sound.extend_from_slice(&levels[run_start..]);
    }

    let sound = (sound.len() >= evidence_frames()).then(|| {
        sound.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        sound[gate_frames() - 1]
    });
    Some(ChunkReading {
        skipped,
        sound,
        noise,
    })
}

/// A track with sound and no lines, as it was when the condition was
/// raised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilentTrack {
    /// `microphone` or `loopback`
    pub track: AudioSource,
    /// The level its skipped sound reaches: the middle one of the chunks
    /// that raised the condition, on the scale of `vad_threshold`
    pub level: f32,
    /// `[meeting.audio] vad_threshold` in this meeting
    pub threshold: f32,
}

/// Two significant digits, no trailing zeros: 0.045, 0.11, 0.0072.
fn two_digits(level: f32) -> String {
    if level <= 0.0 {
        return "0".to_string();
    }
    let level = level as f64;
    let decimals = (1 - level.log10().floor() as i32).max(0) as usize;
    let text = format!("{level:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

impl SilentTrack {
    /// The threshold that lets this track through: half its level (6 dB of
    /// room), rounded down to one digit, and never above the default.
    pub fn suggested_threshold(&self) -> String {
        let half = self.level as f64 / 2.0;
        if half >= DEFAULT_VAD_THRESHOLD as f64 || half <= 0.0 {
            return format!("{DEFAULT_VAD_THRESHOLD}");
        }
        let exponent = half.log10().floor() as i32;
        let unit = 10f64.powi(exponent);
        // The epsilon keeps 0.003 from rounding down to 0.002 on 2.9999...
        let digit = (half / unit + 1e-6).floor().clamp(1.0, 9.0);
        format!("{:.*}", (-exponent).max(0) as usize, digit * unit)
    }

    /// Lowering the threshold to the default would not be enough, or it is
    /// there already.
    fn too_quiet_for_default(&self) -> bool {
        self.suggested_threshold() != format!("{DEFAULT_VAD_THRESHOLD}")
    }

    fn at_or_below_default(&self) -> bool {
        self.threshold <= DEFAULT_VAD_THRESHOLD
    }

    fn name(&self) -> &'static str {
        match self.track {
            AudioSource::Microphone => "Your microphone",
            AudioSource::Loopback => "The call",
            AudioSource::Unknown => "The recording",
        }
    }

    fn lost(&self) -> &'static str {
        match self.track {
            AudioSource::Microphone => "nothing you say",
            AudioSource::Loopback => "nothing the other side says",
            AudioSource::Unknown => "none of it",
        }
    }

    /// What to turn up when the threshold cannot be the cause.
    fn volume(&self) -> &'static str {
        match self.track {
            AudioSource::Microphone => "the microphone's input volume",
            AudioSource::Loopback => "the call's volume in the app that plays it",
            AudioSource::Unknown => "its volume",
        }
    }

    fn command(&self) -> String {
        format!(
            "voxtype config set meeting.audio.vad_threshold {}",
            self.suggested_threshold()
        )
    }

    /// One line: for `meeting status`, the meeting views' banner and the
    /// log.
    pub fn summary(&self) -> String {
        let what = format!(
            "{} has sound, but {} is transcribed: it reaches about {} and \
             [meeting.audio] vad_threshold is {}",
            self.name(),
            self.lost(),
            two_digits(self.level),
            self.threshold,
        );
        if self.at_or_below_default() {
            format!(
                "{what}, already {} the default. Raise {}, or run: {}",
                if self.threshold < DEFAULT_VAD_THRESHOLD {
                    "below"
                } else {
                    "at"
                },
                self.volume(),
                self.command()
            )
        } else {
            format!("{what}. Lower it: {}", self.command())
        }
    }

    /// The notification's opening: what is wrong with this side, and the
    /// numbers.
    fn opening(&self) -> String {
        let mut text = format!(
            "{} has sound, but the speech check skips it, so {} reaches the transcript. \
             It reaches about {} and [meeting.audio] vad_threshold is {}",
            self.name(),
            self.lost(),
            two_digits(self.level),
            self.threshold,
        );
        if self.at_or_below_default() {
            text.push_str(&format!(
                ", already {} the default ({DEFAULT_VAD_THRESHOLD})",
                if self.threshold < DEFAULT_VAD_THRESHOLD {
                    "below"
                } else {
                    "at"
                },
            ));
        }
        text.push('.');
        text
    }

    /// The fix, for this side and any others in `volumes` raised with it.
    fn fix(&self, volumes: &[&str]) -> String {
        let lead = if self.at_or_below_default() {
            format!(
                "Raise {}, or lower the threshold further:",
                volumes.join(" and ")
            )
        } else if self.too_quiet_for_default() {
            format!(
                "Lower the threshold. The sound is quiet, so go below the default \
                 ({DEFAULT_VAD_THRESHOLD}):"
            )
        } else {
            format!("Lower the threshold to the default, {DEFAULT_VAD_THRESHOLD}:")
        };
        format!(
            "{lead}\n{}\nIt applies once voxtype restarts, so restart it after this meeting: \
             systemctl --user restart voxtype",
            self.command()
        )
    }

    /// The desktop notification for this side alone: see [`notification`].
    pub fn notification(&self, audio_kept: bool, meeting: Option<&str>) -> (&'static str, String) {
        notification(std::slice::from_ref(self), audio_kept, meeting)
            .expect("one track is a notification")
    }

    /// For `meeting status --json`: the fields, the threshold to set, and
    /// [`SilentTrack::summary`]'s line as `text`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        value["suggested_threshold"] = self.suggested_threshold().into();
        value["text"] = self.summary().into();
        value
    }
}

/// Whether the lines this meeting missed can be had back.
fn recovery(audio_kept: bool, meeting: Option<&str>) -> String {
    if !audio_kept {
        return "This meeting does not keep its audio ([meeting] retain_audio), so the lines \
                missed so far cannot be recovered."
            .to_string();
    }
    // The first block of the UUID: `retranscribe` takes a unique start.
    let id = meeting
        .and_then(|id| id.split('-').next())
        .filter(|id| !id.is_empty())
        .unwrap_or("latest");
    format!(
        "This meeting keeps its audio. With the threshold lowered, get the missing lines \
         back after it ends:\nvoxtype meeting retranscribe {id}"
    )
}

/// The desktop notification for the sides raised by one pair of chunks:
/// what is wrong with each and its numbers, one fix (the threshold the
/// quietest of them needs), and whether this meeting's lines can be had
/// back. `meeting` is the meeting's ID, for the `retranscribe` command.
///
/// One notification for both: voxtype's notifications share a slot, so a
/// second one sent at once would replace the first unread. `None` for no
/// tracks.
pub fn notification(
    tracks: &[SilentTrack],
    audio_kept: bool,
    meeting: Option<&str>,
) -> Option<(&'static str, String)> {
    let quietest = tracks.iter().min_by(|a, b| {
        a.level
            .partial_cmp(&b.level)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let title = if tracks.len() > 1 { TITLE_BOTH } else { TITLE };
    let openings: Vec<String> = tracks.iter().map(SilentTrack::opening).collect();
    let volumes: Vec<&str> = tracks.iter().map(SilentTrack::volume).collect();
    Some((
        title,
        format!(
            "{}\n\n{}\n\n{}",
            openings.join("\n"),
            quietest.fix(&volumes),
            recovery(audio_kept, meeting)
        ),
    ))
}

/// What a chunk changed.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The track has sound and no lines. `announce` is true the first time
    /// in a meeting, and false when it comes back after lines had arrived.
    Raised { track: SilentTrack, announce: bool },
    /// Lines arrived from a track that was raised.
    Cleared(AudioSource),
}

#[derive(Debug, Default)]
struct Detector {
    /// The track's latest chunks since its last line: the level of the
    /// sound in a skipped one, `None` for any other.
    recent: VecDeque<Option<f32>>,
    active: Option<SilentTrack>,
    announced: bool,
}

impl Detector {
    fn observe(
        &mut self,
        track: AudioSource,
        threshold: f32,
        reading: ChunkReading,
        lines: bool,
    ) -> Option<Change> {
        if lines {
            self.recent.clear();
            return self.active.take().map(|_| Change::Cleared(track));
        }
        self.recent
            .push_back(reading.sound.filter(|_| reading.skipped));
        while self.recent.len() > WINDOW_CHUNKS {
            self.recent.pop_front();
        }
        if self.active.is_some() {
            return None;
        }
        let mut levels: Vec<f32> = self.recent.iter().flatten().copied().collect();
        if levels.len() < EVIDENCE_CHUNKS {
            return None;
        }
        levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let raised = SilentTrack {
            track,
            level: levels[levels.len() / 2],
            threshold,
        };
        self.active = Some(raised.clone());
        let announce = !std::mem::replace(&mut self.announced, true);
        Some(Change::Raised {
            track: raised,
            announce,
        })
    }
}

/// Watches both tracks of one meeting.
#[derive(Debug, Default)]
pub struct Monitor {
    threshold: f32,
    mic: Detector,
    loopback: Detector,
}

impl Monitor {
    /// For a meeting whose speech check uses `vad_threshold`.
    pub fn new(vad_threshold: f32) -> Self {
        Self {
            threshold: vad_threshold,
            ..Default::default()
        }
    }

    /// Take one chunk of `track`: what [`read_chunk`] measured, and whether
    /// the chunk put any line in the transcript.
    pub fn observe(
        &mut self,
        track: AudioSource,
        reading: ChunkReading,
        lines: bool,
    ) -> Option<Change> {
        let detector = match track {
            AudioSource::Microphone => &mut self.mic,
            AudioSource::Loopback => &mut self.loopback,
            AudioSource::Unknown => return None,
        };
        detector.observe(track, self.threshold, reading, lines)
    }

    /// The tracks that have sound and no lines right now.
    pub fn active(&self) -> Vec<SilentTrack> {
        [&self.mic, &self.loopback]
            .into_iter()
            .filter_map(|d| d.active.clone())
            .collect()
    }
}

/// Write the running meeting's untranscribed tracks for `meeting status`,
/// or remove the file when there are none.
pub fn write_status(runtime_dir: &Path, tracks: &[SilentTrack]) {
    let path = runtime_dir.join(STATUS_FILE);
    if tracks.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let json = serde_json::Value::Array(tracks.iter().map(SilentTrack::to_json).collect());
    if let Err(e) = std::fs::write(&path, json.to_string()) {
        tracing::warn!("Cannot write {}: {}", path.display(), e);
    }
}

/// The running meeting's untranscribed tracks, as the daemon last wrote
/// them. Empty when there are none.
pub fn read_status(runtime_dir: &Path) -> Vec<SilentTrack> {
    std::fs::read_to_string(runtime_dir.join(STATUS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod testing {
    /// Deterministic noise in -1.0..1.0.
    pub struct Noise(pub u32);

    impl Noise {
        pub fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    /// `secs` of a track at 16 kHz: room noise at `noise_rms` throughout,
    /// and over each `(start, end, rms)` span a 220 Hz voice whose level
    /// swells and dips four times a second, like syllables, with `rms` the
    /// level of its loudest 30 ms.
    pub fn track(secs: f32, noise_rms: f32, voice: &[(f32, f32, f32)], seed: u32) -> Vec<f32> {
        let mut noise = Noise(seed);
        // Uniform noise in -a..a has an RMS of a / sqrt(3).
        let amplitude = noise_rms * 3f32.sqrt();
        let mut samples: Vec<f32> = (0..(secs * 16000.0) as usize)
            .map(|_| amplitude * noise.next())
            .collect();
        for &(start, end, rms) in voice {
            let span = (start * 16000.0) as usize..(end * 16000.0) as usize;
            for i in span {
                let t = i as f32 / 16000.0;
                // 0.5..1.0 of the peak level.
                let syllable = 0.75 + 0.25 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
                samples[i] +=
                    rms * 2f32.sqrt() * syllable * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
            }
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::testing::track;
    use super::*;

    /// The incident: `vad_threshold = 0.11`, speech whose loudest 30 ms is
    /// 0.06 RMS, a room at 0.002.
    const INCIDENT_THRESHOLD: f32 = 0.11;
    const INCIDENT_SPEECH: f32 = 0.06;
    const ROOM: f32 = 0.002;

    fn read(samples: &[f32], threshold: f32) -> ChunkReading {
        read_chunk(samples, None, 16000, threshold).unwrap()
    }

    fn skipped_with(level: f32) -> ChunkReading {
        ChunkReading {
            skipped: true,
            sound: Some(level),
            noise: ROOM,
        }
    }

    fn quiet() -> ChunkReading {
        ChunkReading {
            skipped: true,
            sound: None,
            noise: ROOM,
        }
    }

    fn transcribed() -> ChunkReading {
        ChunkReading {
            skipped: false,
            sound: None,
            noise: ROOM,
        }
    }

    const MIC: AudioSource = AudioSource::Microphone;
    const CALL: AudioSource = AudioSource::Loopback;

    /// Feed `levels` (a skipped chunk's sound, or `None` for a quiet one)
    /// with no lines, and return what each chunk changed.
    fn feed(monitor: &mut Monitor, track: AudioSource, levels: &[Option<f32>]) -> Vec<Change> {
        levels
            .iter()
            .filter_map(|level| {
                let reading = level.map_or_else(quiet, skipped_with);
                monitor.observe(track, reading, false)
            })
            .collect()
    }

    #[test]
    fn the_numbers_come_from_the_speech_check_and_the_chunker() {
        assert_eq!(gate_frames(), 10, "300 ms of 30 ms frames");
        assert_eq!(evidence_frames(), 20, "one 0.6 s utterance");
        assert_eq!(
            DEFAULT_VAD_THRESHOLD,
            crate::config::MeetingAudioConfig::default().vad_threshold
        );
        assert_eq!(
            DEFAULT_VAD_THRESHOLD,
            crate::meeting::MeetingConfig::default().vad_threshold
        );
        assert_eq!(DEFAULT_VAD_THRESHOLD, ChunkConfig::default().vad_threshold);
    }

    #[test]
    fn the_incidents_speech_is_skipped_and_is_evidence() {
        // Three sentences in a 30 s chunk, the rest the room.
        let mic = track(
            30.0,
            ROOM,
            &[
                (2.0, 5.0, INCIDENT_SPEECH),
                (11.0, 13.5, INCIDENT_SPEECH),
                (20.0, 24.0, INCIDENT_SPEECH),
            ],
            1,
        );
        let reading = read(&mic, INCIDENT_THRESHOLD);
        assert!(reading.skipped, "no frame reaches 0.11");
        let level = reading.sound.expect("9.5 s of speech is evidence");
        assert!(
            (0.05..=0.062).contains(&level),
            "the level is the speech's, near its loudest 30 ms: {level}"
        );
        assert!((0.0015..0.0025).contains(&reading.noise), "{reading:?}");

        // The same chunk at the default threshold is transcribed.
        assert!(!read(&mic, DEFAULT_VAD_THRESHOLD).skipped);
    }

    #[test]
    fn a_listeners_room_is_not_evidence() {
        // Nothing but the room, at the incident's threshold and the default.
        let room = track(30.0, ROOM, &[], 2);
        for threshold in [INCIDENT_THRESHOLD, DEFAULT_VAD_THRESHOLD] {
            let reading = read(&room, threshold);
            assert!(reading.skipped);
            assert_eq!(reading.sound, None, "at {threshold}");
        }
        // A loud room is still only a room: steady noise never stands four
        // times above its own floor.
        let fan = track(30.0, 0.03, &[], 3);
        assert_eq!(read(&fan, INCIDENT_THRESHOLD).sound, None);
        // Digital silence, as from a muted or dead device.
        assert_eq!(read(&vec![0.0; 16000 * 30], INCIDENT_THRESHOLD).sound, None);
    }

    #[test]
    fn clicks_and_a_cough_are_not_evidence() {
        // Key presses: 30 ms each, three a second for ten seconds.
        let keys: Vec<(f32, f32, f32)> = (0..30)
            .map(|i| {
                let at = 5.0 + i as f32 / 3.0;
                (at, at + 0.03, 0.05)
            })
            .collect();
        let typing = track(30.0, ROOM, &keys, 4);
        let reading = read(&typing, INCIDENT_THRESHOLD);
        assert!(reading.skipped);
        assert_eq!(reading.sound, None, "no run reaches 150 ms");

        // One cough: sound, but less than an utterance of it.
        let cough = track(30.0, ROOM, &[(12.0, 12.4, 0.08)], 5);
        assert_eq!(read(&cough, INCIDENT_THRESHOLD).sound, None);
    }

    #[test]
    fn sound_under_half_the_default_threshold_is_not_evidence() {
        // A very quiet room and a faint voice in it: far above the floor,
        // but a stock voxtype calls it silence too.
        let faint = track(30.0, 0.0002, &[(3.0, 9.0, 0.004)], 6);
        let reading = read(&faint, DEFAULT_VAD_THRESHOLD);
        assert!(reading.skipped);
        assert_eq!(reading.sound, None);
    }

    #[test]
    fn the_call_heard_through_speakers_is_not_the_microphones_sound() {
        // The remote talks; the microphone hears it 90 ms later at a third
        // of the level. The user says nothing.
        let call = track(30.0, 0.0, &[(4.0, 12.0, 0.2), (18.0, 25.0, 0.2)], 7);
        let mut mic = track(30.0, ROOM, &[], 8);
        let delay = 16000 * 90 / 1000;
        for i in delay..mic.len() {
            mic[i] += call[i - delay] / 3.0;
        }
        let alone = read(&mic, INCIDENT_THRESHOLD);
        assert!(alone.skipped && alone.sound.is_some(), "it is sound");
        let reading = read_chunk(&mic, Some(&call), 16000, INCIDENT_THRESHOLD).unwrap();
        assert_eq!(reading.sound, None, "but the call explains all of it");

        // The user answers once the remote stops: that is theirs.
        let own = track(30.0, 0.0, &[(13.0, 16.0, INCIDENT_SPEECH)], 9);
        for (m, o) in mic.iter_mut().zip(own) {
            *m += o;
        }
        let reading = read_chunk(&mic, Some(&call), 16000, INCIDENT_THRESHOLD).unwrap();
        let level = reading.sound.expect("their own 3 s");
        assert!((0.05..=0.062).contains(&level), "{level}");
    }

    #[test]
    fn a_chunk_too_short_to_process_is_not_read() {
        assert_eq!(
            read_chunk(&[0.01; 4000], None, 16000, INCIDENT_THRESHOLD),
            None
        );
    }

    #[test]
    fn the_incident_raises_the_microphone_on_the_third_chunk_with_speech() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        // He speaks in some chunks and listens in others.
        let changes = feed(
            &mut monitor,
            MIC,
            &[Some(0.055), None, Some(0.048), None, None],
        );
        assert_eq!(changes, [], "two chunks are not a stretch");
        let changes = feed(&mut monitor, MIC, &[Some(0.052)]);
        let expected = SilentTrack {
            track: MIC,
            level: 0.052,
            threshold: INCIDENT_THRESHOLD,
        };
        assert_eq!(
            changes,
            [Change::Raised {
                track: expected.clone(),
                announce: true
            }]
        );
        assert_eq!(monitor.active(), [expected]);
    }

    #[test]
    fn a_quiet_listener_is_never_raised() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        // An hour of listening.
        assert_eq!(feed(&mut monitor, MIC, &[None; 120]), []);
        assert_eq!(monitor.active(), []);
    }

    #[test]
    fn stray_sounds_far_apart_do_not_add_up() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        // A laugh every five minutes.
        let mut levels = Vec::new();
        for _ in 0..6 {
            levels.push(Some(0.05));
            levels.extend([None; 9]);
        }
        assert_eq!(feed(&mut monitor, MIC, &levels), []);
    }

    #[test]
    fn chunks_that_were_transcribed_are_not_evidence() {
        let mut monitor = Monitor::new(DEFAULT_VAD_THRESHOLD);
        // Sound the speech check passed and the engine made nothing of.
        for _ in 0..20 {
            assert_eq!(monitor.observe(MIC, transcribed(), false), None);
        }
    }

    #[test]
    fn it_is_announced_once_however_long_it_lasts() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        let changes = feed(&mut monitor, MIC, &[Some(0.05); 58]);
        assert_eq!(changes.len(), 1, "29 minutes, one notice: {changes:?}");
    }

    #[test]
    fn a_track_that_recovers_is_cleared_and_never_announced_again() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        assert_eq!(feed(&mut monitor, MIC, &[Some(0.05); 3]).len(), 1);

        // A line arrives.
        assert_eq!(
            monitor.observe(MIC, transcribed(), true),
            Some(Change::Cleared(MIC))
        );
        assert_eq!(monitor.active(), []);
        // More lines change nothing.
        assert_eq!(monitor.observe(MIC, transcribed(), true), None);

        // Lines before the count is reached start it over.
        assert_eq!(feed(&mut monitor, MIC, &[Some(0.05); 2]), []);
        assert_eq!(monitor.observe(MIC, transcribed(), true), None);
        assert_eq!(feed(&mut monitor, MIC, &[Some(0.05); 2]), []);

        // It happens again: `meeting status` says so, no second notice.
        let changes = feed(&mut monitor, MIC, &[Some(0.05)]);
        assert!(
            matches!(
                changes.as_slice(),
                [Change::Raised {
                    announce: false,
                    ..
                }]
            ),
            "{changes:?}"
        );
        assert_eq!(monitor.active().len(), 1);
    }

    #[test]
    fn the_call_side_is_watched_on_its_own() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        // The call at a low volume; the microphone transcribes fine.
        let mut changes = Vec::new();
        for _ in 0..3 {
            assert_eq!(monitor.observe(MIC, transcribed(), true), None);
            changes.extend(monitor.observe(CALL, skipped_with(0.03), false));
        }
        assert_eq!(
            changes,
            [Change::Raised {
                track: SilentTrack {
                    track: CALL,
                    level: 0.03,
                    threshold: INCIDENT_THRESHOLD
                },
                announce: true
            }]
        );
        // Each track is announced once: the microphone still can be.
        let changes = feed(&mut monitor, MIC, &[Some(0.05); 3]);
        assert!(matches!(
            changes.as_slice(),
            [Change::Raised { announce: true, .. }]
        ));
        assert_eq!(monitor.active().len(), 2);
    }

    #[test]
    fn a_track_of_no_side_is_not_watched() {
        let mut monitor = Monitor::new(INCIDENT_THRESHOLD);
        for _ in 0..5 {
            assert_eq!(
                monitor.observe(AudioSource::Unknown, skipped_with(0.05), false),
                None
            );
        }
    }

    #[test]
    fn the_incidents_notification() {
        let incident = SilentTrack {
            track: MIC,
            level: 0.052,
            threshold: INCIDENT_THRESHOLD,
        };
        assert_eq!(incident.suggested_threshold(), "0.01");
        let (title, body) =
            incident.notification(true, Some("b6b2a0c4-1d2e-4f30-9a8b-7c6d5e4f3a2b"));
        assert_eq!(title, "One side of the meeting is not being transcribed");
        assert_eq!(
            body,
            "Your microphone has sound, but the speech check skips it, so nothing you say \
             reaches the transcript. It reaches about 0.052 and [meeting.audio] vad_threshold \
             is 0.11.\n\
             \n\
             Lower the threshold to the default, 0.01:\n\
             voxtype config set meeting.audio.vad_threshold 0.01\n\
             It applies once voxtype restarts, so restart it after this meeting: \
             systemctl --user restart voxtype\n\
             \n\
             This meeting keeps its audio. With the threshold lowered, get the missing lines \
             back after it ends:\n\
             voxtype meeting retranscribe b6b2a0c4"
        );
        assert_eq!(
            incident.summary(),
            "Your microphone has sound, but nothing you say is transcribed: it reaches about \
             0.052 and [meeting.audio] vad_threshold is 0.11. Lower it: voxtype config set \
             meeting.audio.vad_threshold 0.01"
        );
    }

    #[test]
    fn without_kept_audio_it_does_not_promise_the_lines_back() {
        let incident = SilentTrack {
            track: MIC,
            level: 0.052,
            threshold: INCIDENT_THRESHOLD,
        };
        let (_, body) = incident.notification(false, Some("b6b2a0c4-1d2e"));
        assert!(!body.contains("retranscribe"), "{body}");
        assert!(
            body.ends_with(
                "This meeting does not keep its audio ([meeting] retain_audio), so the lines \
                 missed so far cannot be recovered."
            ),
            "{body}"
        );
    }

    #[test]
    fn the_call_is_named_as_the_call() {
        let call = SilentTrack {
            track: CALL,
            level: 0.03,
            threshold: INCIDENT_THRESHOLD,
        };
        let (_, body) = call.notification(true, None);
        assert!(
            body.starts_with(
                "The call has sound, but the speech check skips it, so nothing the other side \
                 says reaches the transcript. It reaches about 0.03 and"
            ),
            "{body}"
        );
        assert!(
            body.ends_with("voxtype meeting retranscribe latest"),
            "{body}"
        );
    }

    #[test]
    fn both_sides_raised_together_share_one_notification() {
        let both = [
            SilentTrack {
                track: MIC,
                level: 0.052,
                threshold: 0.5,
            },
            SilentTrack {
                track: CALL,
                level: 0.012,
                threshold: 0.5,
            },
        ];
        let (title, body) = notification(&both, true, Some("b6b2a0c4-1d2e")).unwrap();
        assert_eq!(title, "Neither side of the meeting is being transcribed");
        assert_eq!(
            body,
            "Your microphone has sound, but the speech check skips it, so nothing you say \
             reaches the transcript. It reaches about 0.052 and [meeting.audio] vad_threshold \
             is 0.5.\n\
             The call has sound, but the speech check skips it, so nothing the other side says \
             reaches the transcript. It reaches about 0.012 and [meeting.audio] vad_threshold \
             is 0.5.\n\
             \n\
             Lower the threshold. The sound is quiet, so go below the default (0.01):\n\
             voxtype config set meeting.audio.vad_threshold 0.006\n\
             It applies once voxtype restarts, so restart it after this meeting: \
             systemctl --user restart voxtype\n\
             \n\
             This meeting keeps its audio. With the threshold lowered, get the missing lines \
             back after it ends:\n\
             voxtype meeting retranscribe b6b2a0c4",
            "one command, for the quieter side"
        );

        // At the default threshold: both volumes, one command.
        let both = both.map(|track| SilentTrack {
            level: track.level / 2.0,
            threshold: DEFAULT_VAD_THRESHOLD,
            ..track
        });
        let (_, body) = notification(&both, false, None).unwrap();
        assert!(
            body.contains(
                "Raise the microphone's input volume and the call's volume in the app that \
                 plays it, or lower the threshold further:\nvoxtype config set \
                 meeting.audio.vad_threshold 0.003\n"
            ),
            "{body}"
        );

        assert_eq!(notification(&[], true, None), None);
    }

    #[test]
    fn at_the_default_threshold_it_does_not_say_to_lower_it_to_the_default() {
        let quiet_mic = SilentTrack {
            track: MIC,
            level: 0.007,
            threshold: DEFAULT_VAD_THRESHOLD,
        };
        assert_eq!(quiet_mic.suggested_threshold(), "0.003");
        let (_, body) = quiet_mic.notification(false, None);
        assert!(
            body.contains(
                "It reaches about 0.007 and [meeting.audio] vad_threshold is 0.01, already at \
                 the default (0.01).\n\nRaise the microphone's input volume, or lower the \
                 threshold further:\nvoxtype config set meeting.audio.vad_threshold 0.003\n"
            ),
            "{body}"
        );
        assert!(!body.contains("to the default"), "{body}");
        assert_eq!(
            quiet_mic.summary(),
            "Your microphone has sound, but nothing you say is transcribed: it reaches about \
             0.007 and [meeting.audio] vad_threshold is 0.01, already at the default. Raise the \
             microphone's input volume, or run: voxtype config set meeting.audio.vad_threshold \
             0.003"
        );

        // Below the default, and the call side.
        let quiet_call = SilentTrack {
            track: CALL,
            level: 0.0062,
            threshold: 0.008,
        };
        let (_, body) = quiet_call.notification(false, None);
        assert!(
            body.contains(
                "vad_threshold is 0.008, already below the default (0.01).\n\nRaise the call's \
                 volume in the app that plays it, or lower the threshold further:\nvoxtype \
                 config set meeting.audio.vad_threshold 0.003\n"
            ),
            "{body}"
        );
    }

    #[test]
    fn a_quiet_side_under_a_raised_threshold_is_sent_below_the_default() {
        // The default would pass 0.012 with nothing to spare.
        let track = SilentTrack {
            track: MIC,
            level: 0.012,
            threshold: INCIDENT_THRESHOLD,
        };
        assert_eq!(track.suggested_threshold(), "0.006");
        let (_, body) = track.notification(false, None);
        assert!(
            body.contains(
                "Lower the threshold. The sound is quiet, so go below the default (0.01):\n\
                 voxtype config set meeting.audio.vad_threshold 0.006\n"
            ),
            "{body}"
        );
    }

    #[test]
    fn the_suggested_threshold_always_lets_the_track_through() {
        for level in [0.0051, 0.006, 0.0072, 0.01, 0.015, 0.0199, 0.02, 0.06, 0.4] {
            for threshold in [0.005, 0.01, 0.02, 0.11, 0.5] {
                if level > threshold {
                    continue;
                }
                let track = SilentTrack {
                    track: MIC,
                    level,
                    threshold,
                };
                let suggested: f32 = track.suggested_threshold().parse().unwrap();
                assert!(
                    suggested > 0.0 && suggested <= level / 2.0 + 1e-9,
                    "{level}: {suggested}"
                );
                assert!(suggested < threshold, "{level} at {threshold}: {suggested}");
                assert!(suggested <= DEFAULT_VAD_THRESHOLD);
            }
        }
    }

    #[test]
    fn levels_read_at_two_digits() {
        assert_eq!(two_digits(0.0523), "0.052");
        assert_eq!(two_digits(0.11), "0.11");
        assert_eq!(two_digits(0.06), "0.06");
        assert_eq!(two_digits(0.0072), "0.0072");
        assert_eq!(two_digits(0.01), "0.01");
        assert_eq!(two_digits(0.3), "0.3");
        assert_eq!(two_digits(0.0), "0");
    }

    #[test]
    fn status_file_round_trip_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_status(dir.path()), []);
        let tracks = [SilentTrack {
            track: MIC,
            level: 0.052,
            threshold: INCIDENT_THRESHOLD,
        }];
        write_status(dir.path(), &tracks);
        assert_eq!(read_status(dir.path()), tracks);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(STATUS_FILE)).unwrap())
                .unwrap();
        assert_eq!(saved[0]["track"], "microphone");
        assert_eq!(saved[0]["suggested_threshold"], "0.01");
        assert_eq!(saved[0]["text"], tracks[0].summary());

        write_status(dir.path(), &[]);
        assert!(!dir.path().join(STATUS_FILE).exists());
        assert_eq!(read_status(dir.path()), []);
    }
}
