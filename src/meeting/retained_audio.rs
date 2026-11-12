//! Retained meeting audio: `mic`, `loopback` and `mixed` tracks written into
//! the meeting folder as the meeting records.
//!
//! Each track is encoded to Ogg Opus by an `ffmpeg` process reading raw PCM
//! on stdin, so audio reaches the disk chunk by chunk rather than at stop.
//! Ogg is a stream of self-checking pages, which is what makes this crash
//! safe: if the daemon dies, ffmpeg sees its stdin close and finishes the
//! file properly; if ffmpeg itself dies, every page already written still
//! decodes. Without ffmpeg (or without its libopus encoder) the tracks are
//! WAV instead, with the header rewritten after every chunk so the file on
//! disk is always complete.
//!
//! The mic track holds the samples the transcript was made from, which are
//! the echo-cancelled ones when GTCRN runs. The raw mic carries the far
//! side again through the speakers, so a mix of it with the loopback track
//! plays every remote voice twice, a few milliseconds apart.
//!
//! Every chunk pair is padded to the longer track, so the three files stay
//! the same length and line up with the transcript's timestamps. Audio from
//! a paused meeting is not kept, matching the transcript.
//!
//! Writing happens on a thread of its own: a stalled encoder must not hold
//! up the meeting daemon. If that thread falls behind, the chunks it misses
//! become silence rather than a shift in the timeline.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Sample rate of every retained track (meeting audio is 16 kHz mono).
pub const SAMPLE_RATE: u32 = 16_000;

/// Opus bitrate. Speech at 16 kHz is near transparent here, and it keeps an
/// hour of one track to about 11 MB.
pub const OPUS_BITRATE_KBPS: u32 = 24;

/// The microphone track's file stem.
pub const MIC: &str = "mic";
/// The loopback (far side) track's file stem.
pub const LOOPBACK: &str = "loopback";
/// The mic and loopback tracks summed, for playback.
pub const MIXED: &str = "mixed";

/// How long `finish` waits for the encoders to close their files.
const FINISH_TIMEOUT: Duration = Duration::from_secs(20);

/// Chunks the writer thread may hold before new ones become silence.
const QUEUE_CHUNKS: usize = 8;

/// The container retained audio is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Opus,
    Wav,
}

impl AudioFormat {
    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Opus => "opus",
            AudioFormat::Wav => "wav",
        }
    }
}

/// The file name of a track in the given format, e.g. `mic.opus`.
pub fn track_file(stem: &str, format: AudioFormat) -> String {
    format!("{}.{}", stem, format.extension())
}

/// How retained audio is encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Encoder {
    /// Ogg Opus through this ffmpeg binary
    Ffmpeg(PathBuf),
    /// WAV, written in-process
    Wav,
}

impl Encoder {
    /// Opus through `ffmpeg` on PATH when it has libopus, otherwise WAV.
    /// Probed once per process.
    pub fn detect() -> Self {
        static DETECTED: OnceLock<Encoder> = OnceLock::new();
        DETECTED
            .get_or_init(|| match which::which("ffmpeg") {
                Ok(path) if ffmpeg_has_libopus(&path) => Encoder::Ffmpeg(path),
                Ok(path) => {
                    tracing::warn!(
                        "{} has no libopus encoder; keeping meeting audio as WAV \
                         (about 115 MB per hour per track instead of 11 MB)",
                        path.display()
                    );
                    Encoder::Wav
                }
                Err(_) => {
                    tracing::warn!(
                        "ffmpeg not found; keeping meeting audio as WAV (about 115 MB \
                         per hour per track instead of 11 MB). Install ffmpeg to keep \
                         it as Opus."
                    );
                    Encoder::Wav
                }
            })
            .clone()
    }

    /// The container this encoder writes.
    pub fn format(&self) -> AudioFormat {
        match self {
            Encoder::Ffmpeg(_) => AudioFormat::Opus,
            Encoder::Wav => AudioFormat::Wav,
        }
    }
}

fn ffmpeg_has_libopus(ffmpeg: &Path) -> bool {
    Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .any(|word| word == "libopus")
        })
        .unwrap_or(false)
}

/// Where one track's samples go.
enum Sink {
    Opus {
        child: Child,
        stdin: Option<ChildStdin>,
    },
    Wav(hound::WavWriter<std::io::BufWriter<std::fs::File>>),
    /// The encoder failed; the file keeps what it had.
    Failed,
}

/// One retained track and its file.
struct Track {
    file: String,
    sink: Sink,
}

impl Track {
    fn open(dir: &Path, stem: &str, encoder: &Encoder) -> std::io::Result<Self> {
        let file = track_file(stem, encoder.format());
        let path = dir.join(&file);
        let sink = match encoder {
            Encoder::Ffmpeg(ffmpeg) => {
                let mut child = ffmpeg_command(ffmpeg, &path).spawn()?;
                let stdin = child.stdin.take();
                Sink::Opus { child, stdin }
            }
            Encoder::Wav => {
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: SAMPLE_RATE,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut writer = hound::WavWriter::create(&path, spec)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                // A valid, empty file from the start.
                writer
                    .flush()
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                Sink::Wav(writer)
            }
        };
        Ok(Self { file, sink })
    }

    fn pid(&self) -> Option<u32> {
        match &self.sink {
            Sink::Opus { child, .. } => Some(child.id()),
            _ => None,
        }
    }

    fn write(&mut self, samples: &[f32]) {
        let result = match &mut self.sink {
            Sink::Opus {
                stdin: Some(stdin), ..
            } => {
                let mut bytes = Vec::with_capacity(samples.len() * 2);
                for &s in samples {
                    bytes.extend_from_slice(&to_i16(s).to_le_bytes());
                }
                stdin.write_all(&bytes).map_err(|e| e.to_string())
            }
            Sink::Wav(writer) => samples
                .iter()
                .try_for_each(|&s| writer.write_sample(to_i16(s)))
                // Rewrite the header so the file is complete as it stands.
                .and_then(|()| writer.flush())
                .map_err(|e| e.to_string()),
            _ => return,
        };
        if let Err(e) = result {
            tracing::error!(
                "Meeting audio for {} stopped: {}; the file keeps the audio up to here",
                self.file,
                e
            );
            self.close();
            self.sink = Sink::Failed;
        }
    }

    fn write_silence(&mut self, mut samples: usize) {
        let zeros = vec![0.0f32; SAMPLE_RATE as usize];
        while samples > 0 {
            let n = samples.min(zeros.len());
            self.write(&zeros[..n]);
            samples -= n;
        }
    }

    /// Close the file: let ffmpeg write its last pages and exit, or update
    /// the WAV header one final time.
    fn close(&mut self) {
        match std::mem::replace(&mut self.sink, Sink::Failed) {
            Sink::Opus { mut child, stdin } => {
                drop(stdin);
                let deadline = Instant::now() + FINISH_TIMEOUT;
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                let mut err = String::new();
                                if let Some(mut stderr) = child.stderr.take() {
                                    let _ = std::io::Read::read_to_string(&mut stderr, &mut err);
                                }
                                tracing::warn!(
                                    "ffmpeg for {} exited with {}: {}",
                                    self.file,
                                    status,
                                    err.trim()
                                );
                            }
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Ok(None) => {
                            tracing::warn!("ffmpeg for {} did not finish; killing it", self.file);
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("Could not wait for ffmpeg for {}: {}", self.file, e);
                            break;
                        }
                    }
                }
            }
            Sink::Wav(writer) => {
                if let Err(e) = writer.finalize() {
                    tracing::warn!("Failed to finalize {}: {}", self.file, e);
                }
            }
            Sink::Failed => {}
        }
    }
}

fn ffmpeg_command(ffmpeg: &Path, path: &Path) -> Command {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-f", "s16le", "-ar", &SAMPLE_RATE.to_string(), "-ac", "1"])
        .args(["-i", "pipe:0"])
        .args(encoder_args(AudioFormat::Opus))
        // Hand each Ogg page to the file as soon as it is complete, so a
        // crash loses at most the page in progress (about a second).
        .args(["-flush_packets", "1", "-y"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Its own process group, so Ctrl+C on a daemon in a terminal does not
    // stop the encoder before the daemon hands it the last chunk.
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd
}

/// ffmpeg output arguments that write a retained track in `format`:
/// 16 kHz mono Ogg Opus at [`OPUS_BITRATE_KBPS`], or 16-bit WAV. The
/// caller supplies the input, the sample rate and channel layout if it
/// needs converting, and the output path.
pub fn encoder_args(format: AudioFormat) -> Vec<String> {
    let args: Vec<String> = match format {
        AudioFormat::Opus => vec![
            "-c:a".into(),
            "libopus".into(),
            "-b:a".into(),
            format!("{OPUS_BITRATE_KBPS}k"),
            "-application".into(),
            "voip".into(),
            "-f".into(),
            "ogg".into(),
        ],
        AudioFormat::Wav => vec!["-c:a".into(), "pcm_s16le".into(), "-f".into(), "wav".into()],
    };
    args
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// Messages to the writer thread.
enum Msg {
    Audio { mic: Vec<f32>, loopback: Vec<f32> },
    Silence(usize),
}

/// The writer thread's tracks.
struct Writer {
    dir: PathBuf,
    encoder: Encoder,
    mic: Track,
    /// Opened at the first loopback audio, so a mic-only meeting has no
    /// loopback file full of silence
    loopback: Option<Track>,
    mixed: Track,
    /// Samples written to each track so far
    written: usize,
}

impl Writer {
    fn audio(&mut self, mic: Vec<f32>, loopback: Vec<f32>) {
        let span = mic.len().max(loopback.len());
        if span == 0 {
            return;
        }
        if !loopback.is_empty() && self.loopback.is_none() {
            match Track::open(&self.dir, LOOPBACK, &self.encoder) {
                Ok(mut track) => {
                    track.write_silence(self.written);
                    self.loopback = Some(track);
                }
                Err(e) => tracing::error!("Failed to start the loopback audio track: {}", e),
            }
        }

        let mut mic = mic;
        mic.resize(span, 0.0);
        let mut mixed = mic.clone();
        if let Some(ref mut track) = self.loopback {
            let mut loopback = loopback;
            loopback.resize(span, 0.0);
            for (m, l) in mixed.iter_mut().zip(&loopback) {
                *m = (*m + l).clamp(-1.0, 1.0);
            }
            track.write(&loopback);
        }
        self.mic.write(&mic);
        self.mixed.write(&mixed);
        self.written += span;
    }

    fn silence(&mut self, samples: usize) {
        self.mic.write_silence(samples);
        if let Some(ref mut track) = self.loopback {
            track.write_silence(samples);
        }
        self.mixed.write_silence(samples);
        self.written += samples;
    }

    fn run(mut self, rx: Receiver<Msg>) {
        // Ends when the retainer finishes or is dropped, including when
        // the daemon unwinds without stopping the meeting.
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Audio { mic, loopback } => self.audio(mic, loopback),
                Msg::Silence(samples) => self.silence(samples),
            }
        }
        self.mic.close();
        if let Some(ref mut track) = self.loopback {
            track.close();
        }
        self.mixed.close();
    }
}

/// Writes a meeting's audio tracks into its folder as chunks arrive.
pub struct AudioRetainer {
    tx: Option<SyncSender<Msg>>,
    thread: Option<JoinHandle<()>>,
    done: Receiver<()>,
    format: AudioFormat,
    /// Samples per track that could not be queued and are owed as silence
    owed_silence: usize,
    /// Encoder PIDs at start, for tests that kill one mid-stream
    #[cfg_attr(not(test), allow(dead_code))]
    pids: Vec<u32>,
}

impl AudioRetainer {
    /// Start the mic and mixed tracks in `dir`. When ffmpeg cannot be
    /// started, falls back to WAV.
    pub fn start(dir: &Path, encoder: Encoder) -> std::io::Result<Self> {
        let open = |encoder: &Encoder| -> std::io::Result<(Track, Track)> {
            Ok((
                Track::open(dir, MIC, encoder)?,
                Track::open(dir, MIXED, encoder)?,
            ))
        };
        let (encoder, (mic, mixed)) = match open(&encoder) {
            Ok(tracks) => (encoder, tracks),
            Err(e) if encoder != Encoder::Wav => {
                tracing::warn!(
                    "Could not start ffmpeg for meeting audio ({}); keeping it as WAV",
                    e
                );
                (Encoder::Wav, open(&Encoder::Wav)?)
            }
            Err(e) => return Err(e),
        };
        let format = encoder.format();
        let pids = [mic.pid(), mixed.pid()].into_iter().flatten().collect();

        let writer = Writer {
            dir: dir.to_path_buf(),
            encoder,
            mic,
            loopback: None,
            mixed,
            written: 0,
        };
        let (tx, rx) = mpsc::sync_channel(QUEUE_CHUNKS);
        let (done_tx, done) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("meeting-audio".to_string())
            .spawn(move || {
                writer.run(rx);
                let _ = done_tx.send(());
            })?;
        tracing::info!(
            "Keeping meeting audio as {} in {}",
            format.extension(),
            dir.display()
        );
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            done,
            format,
            owed_silence: 0,
            pids,
        })
    }

    /// The container the tracks are written in.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Queue one chunk of each track covering the same stretch of time.
    /// Never blocks: if the writer is behind, the chunk is kept as silence
    /// once it catches up.
    pub fn push(&mut self, mic: Vec<f32>, loopback: Vec<f32>) {
        let Some(ref tx) = self.tx else {
            return;
        };
        if self.owed_silence > 0 {
            match tx.try_send(Msg::Silence(self.owed_silence)) {
                Ok(()) => self.owed_silence = 0,
                Err(TrySendError::Full(_)) => {
                    self.owed_silence += mic.len().max(loopback.len());
                    return;
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        let span = mic.len().max(loopback.len());
        match tx.try_send(Msg::Audio { mic, loopback }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::warn!(
                    "Meeting audio writer is behind; keeping {:.1}s as silence",
                    span as f32 / SAMPLE_RATE as f32
                );
                self.owed_silence += span;
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Write what is queued and close the files. Waits up to
    /// [`FINISH_TIMEOUT`] for the encoders; returns whether they finished.
    pub fn finish(mut self) -> bool {
        if let Some(tx) = self.tx.take() {
            if self.owed_silence > 0 {
                let _ = tx.send(Msg::Silence(self.owed_silence));
            }
        }
        match self
            .done
            .recv_timeout(FINISH_TIMEOUT + Duration::from_secs(5))
        {
            Ok(()) => {
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
                true
            }
            Err(_) => {
                tracing::warn!("Meeting audio writer did not finish in time");
                false
            }
        }
    }

    #[cfg(test)]
    fn encoder_pids(&self) -> &[u32] {
        &self.pids
    }
}

/// The retained audio files in `dir`, as (track stem, file name) in
/// mic, loopback, mixed order.
pub fn retained_files(dir: &Path) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    for stem in [MIC, LOOPBACK, MIXED] {
        for format in [AudioFormat::Opus, AudioFormat::Wav] {
            let file = track_file(stem, format);
            if dir.join(&file).is_file() {
                found.push((stem, file));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, amplitude: f32) -> Vec<f32> {
        (0..(secs * SAMPLE_RATE as f32) as usize)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect()
    }

    fn ffmpeg() -> Option<Encoder> {
        match Encoder::detect() {
            Encoder::Ffmpeg(path) => Some(Encoder::Ffmpeg(path)),
            Encoder::Wav => {
                eprintln!("skipping: no ffmpeg with libopus");
                None
            }
        }
    }

    /// Decode a file with ffmpeg to 16 kHz mono samples.
    fn decode(path: &Path) -> Vec<i16> {
        let Encoder::Ffmpeg(ffmpeg) = Encoder::detect() else {
            panic!("decode needs ffmpeg");
        };
        let out = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-f", "s16le", "-ac", "1", "-ar", "16000", "pipe:1"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{} does not decode: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect()
    }

    fn secs(samples: usize) -> f32 {
        samples as f32 / SAMPLE_RATE as f32
    }

    fn rms(samples: &[i16]) -> f32 {
        let sum: f64 = samples.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
        (sum / samples.len().max(1) as f64).sqrt() as f32
    }

    #[test]
    fn opus_tracks_decode_to_the_audio_written() {
        let Some(encoder) = ffmpeg() else { return };
        let dir = tempfile::TempDir::new().unwrap();
        let mut audio = AudioRetainer::start(dir.path(), encoder).unwrap();
        assert_eq!(audio.format(), AudioFormat::Opus);
        // Mic speaks in the first chunk, the far side in the second.
        audio.push(tone(3.0, 0.5), vec![0.0; 3 * 16000]);
        audio.push(vec![0.0; 3 * 16000], tone(3.0, 0.5));
        assert!(audio.finish());

        let mic = decode(&dir.path().join("mic.opus"));
        let loopback = decode(&dir.path().join("loopback.opus"));
        let mixed = decode(&dir.path().join("mixed.opus"));
        for track in [&mic, &loopback, &mixed] {
            assert!(
                (secs(track.len()) - 6.0).abs() < 0.1,
                "{}",
                secs(track.len())
            );
        }
        let (first, second) = (0..2 * 16000, 4 * 16000..5 * 16000);
        assert!(rms(&mic[first.clone()]) > 0.2 && rms(&mic[second.clone()]) < 0.02);
        assert!(rms(&loopback[first.clone()]) < 0.02 && rms(&loopback[second.clone()]) > 0.2);
        assert!(rms(&mixed[first]) > 0.2 && rms(&mixed[second]) > 0.2);
    }

    #[test]
    fn loopback_that_starts_late_is_padded_to_the_timeline() {
        let Some(encoder) = ffmpeg() else { return };
        let dir = tempfile::TempDir::new().unwrap();
        let mut audio = AudioRetainer::start(dir.path(), encoder).unwrap();
        audio.push(tone(2.0, 0.5), Vec::new());
        assert!(!dir.path().join("loopback.opus").exists());
        // A short loopback chunk is padded to the mic's length.
        audio.push(tone(2.0, 0.5), tone(1.0, 0.5));
        assert!(audio.finish());

        let loopback = decode(&dir.path().join("loopback.opus"));
        assert!((secs(loopback.len()) - 4.0).abs() < 0.1);
        assert!(
            rms(&loopback[..32000 - 1600]) < 0.02,
            "silence before it began"
        );
        assert!(rms(&loopback[32000 + 1600..48000 - 1600]) > 0.2);
        assert!(rms(&loopback[48000 + 1600..]) < 0.02, "padding after it");
    }

    #[test]
    fn mic_only_meetings_have_no_loopback_file() {
        let Some(encoder) = ffmpeg() else { return };
        let dir = tempfile::TempDir::new().unwrap();
        let mut audio = AudioRetainer::start(dir.path(), encoder).unwrap();
        audio.push(tone(1.0, 0.5), Vec::new());
        assert!(audio.finish());
        let files: Vec<String> = retained_files(dir.path())
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        assert_eq!(files, ["mic.opus", "mixed.opus"]);
    }

    /// Seconds of audio in the complete Ogg pages of an Opus file: the last
    /// complete page's granule position (48 kHz samples) less the encoder's
    /// pre-skip. This is exactly what a decoder can recover from the bytes
    /// on disk, whatever the bitrate did.
    fn flushed_secs(path: &Path) -> f64 {
        let bytes = std::fs::read(path).unwrap_or_default();
        let (mut pos, mut pre_skip, mut granule) = (0, None, 0i64);
        while pos + 27 <= bytes.len() && &bytes[pos..pos + 4] == b"OggS" {
            let segments = bytes[pos + 26] as usize;
            let header = 27 + segments;
            if pos + header > bytes.len() {
                break;
            }
            let body: usize = bytes[pos + 27..pos + header]
                .iter()
                .map(|&b| b as usize)
                .sum();
            if pos + header + body > bytes.len() {
                break; // the page being written when it died
            }
            let page = &bytes[pos + header..pos + header + body];
            if pre_skip.is_none() && page.starts_with(b"OpusHead") {
                pre_skip = Some(u16::from_le_bytes([page[10], page[11]]) as i64);
            }
            let g = i64::from_le_bytes(bytes[pos + 6..pos + 14].try_into().unwrap());
            if g > 0 {
                granule = g;
            }
            pos += header + body;
        }
        (granule - pre_skip.unwrap_or(0)).max(0) as f64 / 48_000.0
    }

    /// The encoder dies mid-meeting: everything it had flushed still
    /// decodes, the other tracks are unaffected, and the meeting carries
    /// on without blocking.
    #[test]
    fn a_killed_encoder_leaves_the_audio_written_so_far() {
        let Some(encoder) = ffmpeg() else { return };
        let dir = tempfile::TempDir::new().unwrap();
        let mut audio = AudioRetainer::start(dir.path(), encoder).unwrap();
        for _ in 0..10 {
            audio.push(tone(1.0, 0.5), Vec::new());
        }
        // Kill ffmpeg unannounced once at least two seconds are on disk in
        // complete pages, however long encoding takes under load.
        let mic_path = dir.path().join("mic.opus");
        let deadline = Instant::now() + Duration::from_secs(30);
        while flushed_secs(&mic_path) < 2.0 {
            assert!(Instant::now() < deadline, "ffmpeg flushed nothing");
            std::thread::sleep(Duration::from_millis(20));
        }
        let mic_pid = audio.encoder_pids()[0];
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(mic_pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();
        // More audio after the crash goes nowhere, without an error.
        audio.push(tone(1.0, 0.5), Vec::new());
        assert!(audio.finish());

        // The file is final now. What its complete pages hold is what must
        // decode, no more and no less (to within one 20 ms Opus frame).
        let flushed = flushed_secs(&mic_path);
        assert!(flushed >= 2.0, "{flushed}s flushed");
        let recovered = secs(decode(&mic_path).len()) as f64;
        assert!(
            (recovered - flushed).abs() <= 0.021,
            "{recovered}s decoded of {flushed}s flushed"
        );
        assert!(rms(&decode(&mic_path)[16000..32000]) > 0.2);
        // The other track was not affected.
        assert!((secs(decode(&dir.path().join("mixed.opus")).len()) - 11.0).abs() < 0.1);
    }

    /// The daemon dies mid-meeting without stopping: ffmpeg sees its input
    /// close and finishes the file on its own.
    #[test]
    fn a_vanished_daemon_leaves_complete_files() {
        let Some(Encoder::Ffmpeg(ffmpeg)) = ffmpeg() else {
            return;
        };
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        let mut child = ffmpeg_command(&ffmpeg, &path).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let bytes: Vec<u8> = tone(5.0, 0.5)
            .iter()
            .flat_map(|&s| to_i16(s).to_le_bytes())
            .collect();
        stdin.write_all(&bytes).unwrap();
        // What the kernel does to the pipe when the writer's process dies.
        drop(stdin);
        assert!(child.wait().unwrap().success());
        assert!((secs(decode(&path).len()) - 5.0).abs() < 0.1);
    }

    #[test]
    fn wav_tracks_are_valid_after_every_chunk() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut audio = AudioRetainer::start(dir.path(), Encoder::Wav).unwrap();
        assert_eq!(audio.format(), AudioFormat::Wav);
        let read = |name: &str| -> Vec<i16> {
            hound::WavReader::open(dir.path().join(name))
                .unwrap()
                .into_samples::<i16>()
                .map(|s| s.unwrap())
                .collect()
        };
        audio.push(tone(1.0, 0.5), tone(1.0, 0.25));
        audio.push(tone(1.0, 0.5), tone(1.0, 0.25));
        // Read mid-meeting, as after a crash: complete up to the last chunk.
        let deadline = Instant::now() + Duration::from_secs(10);
        while read("mic.wav").len() < 32000 {
            assert!(Instant::now() < deadline, "writer never caught up");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(read("mic.wav").len(), 32000);
        assert!(audio.finish());
        assert_eq!(read("loopback.wav").len(), 32000);
        let mixed = read("mixed.wav");
        assert_eq!(mixed.len(), 32000);
        // 0.5 + 0.25 of the same tone at its peak.
        let peak = mixed.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((peak as f32 / 32767.0 - 0.75).abs() < 0.01, "{peak}");
    }

    #[test]
    fn missing_ffmpeg_falls_back_to_wav() {
        let dir = tempfile::TempDir::new().unwrap();
        let encoder = Encoder::Ffmpeg(PathBuf::from("/nonexistent/ffmpeg"));
        let mut audio = AudioRetainer::start(dir.path(), encoder).unwrap();
        assert_eq!(audio.format(), AudioFormat::Wav);
        audio.push(tone(0.5, 0.5), Vec::new());
        assert!(audio.finish());
        assert!(dir.path().join("mic.wav").is_file());
        assert!(!dir.path().join("mic.opus").exists());
    }
}
