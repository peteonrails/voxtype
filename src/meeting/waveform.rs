//! `voxtype meeting waveform`: per-bucket peak levels of a meeting's kept
//! audio, for drawing a waveform under a player.
//!
//! The tracks are decoded with ffmpeg (as `meeting play` does) to 16 kHz mono
//! and folded into peaks as they stream, so an hour-long meeting is never
//! held in memory. The result is cached in the meeting folder as
//! `waveform.json`, one entry per bucket count, and recomputed when an audio
//! file changes (a meeting still recording keeps growing its files).

use crate::meeting::retained_audio;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A bucket with no sound in it, and the lowest level reported: the same
/// floor as the meeting levels.
pub const SILENCE_DB: f32 = -100.0;
/// Buckets when not told.
pub const DEFAULT_BUCKETS: usize = 1000;
/// The most buckets asked for at once.
pub const MAX_BUCKETS: usize = 100_000;
/// The cache file in the meeting folder.
pub const CACHE_FILE: &str = "waveform.json";

const SAMPLE_RATE: u64 = retained_audio::SAMPLE_RATE as u64;
const CACHE_VERSION: u32 = 1;

/// The peaks of a meeting's tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waveform {
    /// The longer track's length.
    pub duration_ms: u64,
    /// The time each value covers; the last bucket may be shorter.
    pub bucket_ms: u64,
    /// Peak dBFS per bucket of your microphone, [`SILENCE_DB`] for silence.
    pub mic: Vec<f32>,
    /// The same for the other side, `null` when the meeting has no loopback.
    pub loopback: Option<Vec<f32>>,
    /// `mic`, or `mixed` when the meeting kept only both sides together (an
    /// import without mapped channels): `mic` then holds the mixed track.
    pub mic_track: String,
}

/// Why a waveform could not be made.
#[derive(Debug, thiserror::Error)]
pub enum WaveformError {
    #[error(
        "This meeting was recorded without keeping audio, so it has no waveform. \
         Turn on meeting.retain_audio for future meetings."
    )]
    NoAudio,

    #[error("The waveform needs ffmpeg to decode the meeting's audio, and it is not installed")]
    NoFfmpeg,

    #[error("ffmpeg could not decode {path}: {message}")]
    Decode { path: String, message: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The files a waveform is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracks {
    pub mic: PathBuf,
    pub mic_track: &'static str,
    pub loopback: Option<PathBuf>,
}

/// The meeting folder's kept tracks: `mic` (or `mixed` without it) and
/// `loopback` when there is one.
pub fn tracks(dir: &Path) -> Result<Tracks, WaveformError> {
    let files = retained_audio::retained_files(dir);
    let find = |stem: &str| {
        files
            .iter()
            .find(|(s, _)| *s == stem)
            .map(|(_, name)| dir.join(name))
    };
    let (mic, mic_track) = match (find(retained_audio::MIC), find(retained_audio::MIXED)) {
        (Some(mic), _) => (mic, retained_audio::MIC),
        (None, Some(mixed)) => (mixed, retained_audio::MIXED),
        (None, None) => return Err(WaveformError::NoAudio),
    };
    Ok(Tracks {
        mic,
        mic_track,
        loopback: find(retained_audio::LOOPBACK),
    })
}

/// Folds samples into per-bucket peaks.
struct Peaks {
    bucket_samples: u64,
    filled: u64,
    peak: f32,
    total: u64,
    out: Vec<f32>,
}

impl Peaks {
    fn new(bucket_samples: u64) -> Self {
        Self {
            bucket_samples: bucket_samples.max(1),
            filled: 0,
            peak: 0.0,
            total: 0,
            out: Vec::new(),
        }
    }

    fn push(&mut self, sample: f32) {
        self.peak = self.peak.max(sample.abs());
        self.filled += 1;
        self.total += 1;
        if self.filled == self.bucket_samples {
            self.out.push(db(self.peak));
            self.filled = 0;
            self.peak = 0.0;
        }
    }

    /// The peaks, the last partial bucket included, padded with silence to
    /// `buckets`.
    fn finish(mut self, buckets: usize) -> Vec<f32> {
        if self.filled > 0 {
            self.out.push(db(self.peak));
        }
        self.out.resize(buckets, SILENCE_DB);
        self.out.truncate(buckets);
        self.out
    }
}

/// Peak in dBFS, one decimal, floored at [`SILENCE_DB`].
fn db(peak: f32) -> f32 {
    if peak <= 0.0 {
        return SILENCE_DB;
    }
    let level = (20.0 * peak.min(1.0).log10()).max(SILENCE_DB);
    (level * 10.0).round() / 10.0
}

/// Decode `path` to 16 kHz mono f32 and hand each sample to `each`.
fn decode(ffmpeg: &Path, path: &Path, mut each: impl FnMut(f32)) -> Result<(), WaveformError> {
    let mut child = Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "1", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..n]);
        let whole = carry.len() - carry.len() % 4;
        for c in carry[..whole].as_chunks::<4>().0 {
            each(f32::from_le_bytes(*c));
        }
        carry.drain(..whole);
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(WaveformError::Decode {
            path: path.display().to_string(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

/// Samples in `path`, read by decoding it once.
fn sample_count(ffmpeg: &Path, path: &Path) -> Result<u64, WaveformError> {
    let mut n = 0u64;
    decode(ffmpeg, path, |_| n += 1)?;
    Ok(n)
}

/// Compute the waveform of `tracks` in about `buckets` buckets. The bucket
/// length is a whole number of milliseconds, so there can be fewer.
pub fn compute(ffmpeg: &Path, tracks: &Tracks, buckets: usize) -> Result<Waveform, WaveformError> {
    let buckets = buckets.clamp(1, MAX_BUCKETS);
    // The length first, to size the buckets; a second pass folds the peaks.
    let mic_len = sample_count(ffmpeg, &tracks.mic)?;
    let loopback_len = match &tracks.loopback {
        Some(path) => sample_count(ffmpeg, path)?,
        None => 0,
    };
    let len = mic_len.max(loopback_len);
    let duration_ms = len * 1000 / SAMPLE_RATE;
    let bucket_ms = duration_ms.div_ceil(buckets as u64).max(1);
    let count = (duration_ms.div_ceil(bucket_ms) as usize).max(1);
    let bucket_samples = bucket_ms * SAMPLE_RATE / 1000;

    let peaks_of = |path: &Path| -> Result<Vec<f32>, WaveformError> {
        let mut peaks = Peaks::new(bucket_samples);
        decode(ffmpeg, path, |s| peaks.push(s))?;
        Ok(peaks.finish(count))
    };
    Ok(Waveform {
        duration_ms,
        bucket_ms,
        mic: peaks_of(&tracks.mic)?,
        loopback: tracks.loopback.as_deref().map(peaks_of).transpose()?,
        mic_track: tracks.mic_track.to_string(),
    })
}

/// The cache file: waveforms by bucket count, for one state of the audio.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    /// Size and modification time of each track, as `name:size:mtime_ns`.
    stamp: Vec<String>,
    waveforms: BTreeMap<usize, Waveform>,
}

fn stamp(tracks: &Tracks) -> Vec<String> {
    std::iter::once(&tracks.mic)
        .chain(tracks.loopback.iter())
        .map(|path| {
            let meta = std::fs::metadata(path).ok();
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!(
                "{}:{}:{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                meta.map_or(0, |m| m.len()),
                mtime
            )
        })
        .collect()
}

/// The waveform of the meeting in `dir`, from its cache when the audio has
/// not changed since, else computed with `ffmpeg` and cached.
pub fn load_or_compute(
    dir: &Path,
    ffmpeg: Option<&Path>,
    buckets: usize,
) -> Result<Waveform, WaveformError> {
    let buckets = buckets.clamp(1, MAX_BUCKETS);
    let tracks = tracks(dir)?;
    let stamp = stamp(&tracks);
    let cache_path = dir.join(CACHE_FILE);
    let mut cache: Cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(|c: &Cache| c.version == CACHE_VERSION && c.stamp == stamp)
        .unwrap_or_else(|| Cache {
            version: CACHE_VERSION,
            stamp,
            waveforms: BTreeMap::new(),
        });
    if let Some(waveform) = cache.waveforms.get(&buckets) {
        return Ok(waveform.clone());
    }
    let ffmpeg = ffmpeg.ok_or(WaveformError::NoFfmpeg)?;
    let waveform = compute(ffmpeg, &tracks, buckets)?;
    cache.waveforms.insert(buckets, waveform.clone());
    // A cache that cannot be written only costs the next caller the time.
    if let Ok(json) = serde_json::to_vec(&cache) {
        let staged = dir.join(format!("{CACHE_FILE}.{}.tmp", std::process::id()));
        if std::fs::write(&staged, json).is_ok() {
            let _ = std::fs::rename(&staged, &cache_path);
        }
    }
    Ok(waveform)
}

/// ffmpeg on the PATH.
pub fn find_ffmpeg() -> Option<PathBuf> {
    which::which("ffmpeg").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_are_dbfs_per_bucket_with_a_silence_floor() {
        assert_eq!(db(0.0), SILENCE_DB);
        assert_eq!(db(1.0), 0.0);
        assert_eq!(db(0.5), -6.0);
        assert_eq!(db(1e-9), SILENCE_DB);
        assert_eq!(db(2.0), 0.0, "clipped samples are full scale");

        let mut peaks = Peaks::new(4);
        for s in [0.0, -0.5, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            peaks.push(s);
        }
        assert_eq!(peaks.finish(4), vec![-6.0, SILENCE_DB, 0.0, SILENCE_DB]);
    }

    fn write_wav(path: &Path, samples: &[f32]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for s in samples {
            w.write_sample((s * 32767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// A fake ffmpeg that counts its runs and decodes a WAV by skipping its
    /// 44-byte header and widening each sample to f32.
    fn fake_ffmpeg(dir: &Path) -> (PathBuf, PathBuf) {
        let runs = dir.join("runs");
        let ffmpeg = dir.join("ffmpeg");
        crate::test_support::write_script(
            &ffmpeg,
            &format!(
                "#!/usr/bin/env python3\nimport sys, struct\nopen({runs:?}, 'a').write('run\\n')\n\
                 path = sys.argv[sys.argv.index('-i') + 1]\ndata = open(path, 'rb').read()[44:]\n\
                 n = len(data) // 2\nvals = struct.unpack('<%dh' % n, data[:n * 2])\n\
                 sys.stdout.buffer.write(struct.pack('<%df' % n, *[v / 32768 for v in vals]))\n",
                runs = runs.display().to_string()
            ),
        );
        (ffmpeg, runs)
    }

    fn runs(path: &Path) -> usize {
        std::fs::read_to_string(path).map_or(0, |s| s.lines().count())
    }

    #[test]
    fn a_meeting_folder_gives_both_tracks_and_caches_them() {
        let dir = tempfile::TempDir::new().unwrap();
        let tools = tempfile::TempDir::new().unwrap();
        let (ffmpeg, count) = fake_ffmpeg(tools.path());
        // 2 s of mic: silence, then a half-scale second. 1 s of loopback.
        let mut mic = vec![0.0f32; 16000];
        mic.extend(vec![0.5f32; 16000]);
        write_wav(&dir.path().join("mic.wav"), &mic);
        write_wav(&dir.path().join("loopback.wav"), &vec![0.25f32; 16000]);

        let w = load_or_compute(dir.path(), Some(&ffmpeg), 4).unwrap();
        assert_eq!((w.duration_ms, w.bucket_ms), (2000, 500));
        assert_eq!(w.mic_track, "mic");
        assert_eq!(w.mic, vec![SILENCE_DB, SILENCE_DB, -6.0, -6.0]);
        assert_eq!(
            w.loopback.as_deref(),
            Some(&[-12.0, -12.0, SILENCE_DB, SILENCE_DB][..]),
            "padded with silence to the meeting's length"
        );
        let decoded = runs(&count);

        // Again: from the cache, without ffmpeg.
        assert_eq!(load_or_compute(dir.path(), None, 4).unwrap(), w);
        assert_eq!(runs(&count), decoded);
        // Another bucket count is computed and joins the cache.
        let fine = load_or_compute(dir.path(), Some(&ffmpeg), 2000).unwrap();
        assert_eq!((fine.bucket_ms, fine.mic.len()), (1, 2000));
        assert_eq!(load_or_compute(dir.path(), None, 4).unwrap(), w);

        // New audio (a meeting still recording) throws the cache away.
        std::thread::sleep(std::time::Duration::from_millis(20));
        mic.extend(vec![1.0f32; 16000]);
        write_wav(&dir.path().join("mic.wav"), &mic);
        assert!(matches!(
            load_or_compute(dir.path(), None, 4),
            Err(WaveformError::NoFfmpeg)
        ));
        let grown = load_or_compute(dir.path(), Some(&ffmpeg), 4).unwrap();
        assert_eq!((grown.duration_ms, grown.bucket_ms), (3000, 750));
        assert_eq!(grown.mic.last(), Some(&0.0));
    }

    #[test]
    fn mixed_stands_in_for_the_mic_and_no_audio_is_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(matches!(tracks(dir.path()), Err(WaveformError::NoAudio)));
        write_wav(&dir.path().join("mixed.wav"), &[0.1; 1600]);
        let t = tracks(dir.path()).unwrap();
        assert_eq!((t.mic_track, t.loopback), ("mixed", None));

        let tools = tempfile::TempDir::new().unwrap();
        let (ffmpeg, _) = fake_ffmpeg(tools.path());
        let w = load_or_compute(dir.path(), Some(&ffmpeg), 1000).unwrap();
        assert_eq!((w.duration_ms, w.bucket_ms, w.mic.len()), (100, 1, 100));
        assert_eq!(w.loopback, None);
    }

    #[test]
    fn a_failed_decode_names_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write_wav(&dir.path().join("mic.wav"), &[0.1; 1600]);
        let tools = tempfile::TempDir::new().unwrap();
        let ffmpeg = tools.path().join("ffmpeg");
        crate::test_support::write_script(&ffmpeg, "#!/bin/sh\necho 'Invalid data' >&2\nexit 1\n");
        let err = load_or_compute(dir.path(), Some(&ffmpeg), 10).unwrap_err();
        assert!(err.to_string().contains("mic.wav"), "{err}");
        assert!(err.to_string().contains("Invalid data"), "{err}");
        assert!(!dir.path().join(CACHE_FILE).exists());
    }
}
