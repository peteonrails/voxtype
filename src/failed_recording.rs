//! Keep the audio of a recording whose transcription failed.
//!
//! A failed transcription used to discard the recording, so a long dictation
//! was gone along with the error. The daemon now writes the captured audio to
//! `$XDG_STATE_HOME/voxtype/failed-audio/` (default
//! `~/.local/state/voxtype/failed-audio/`) and tells the user how to
//! retry it with `voxtype transcribe <file>`. Only the most recent
//! [`KEEP`] recordings are kept so the directory cannot grow without bound.

use std::path::{Path, PathBuf};

/// How many failed recordings to keep before the oldest are removed.
pub const KEEP: usize = 20;

const SAMPLE_RATE: u32 = 16000;

/// Directory failed recordings are written to.
pub fn dir() -> PathBuf {
    state_dir().join("failed-audio")
}

/// `$XDG_STATE_HOME/voxtype`, defaulting to `~/.local/state/voxtype`, or the
/// data dir when there is no home to resolve it against.
fn state_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return base.join("voxtype");
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local/state/voxtype"))
        .unwrap_or_else(|| crate::config::Config::data_dir().join("state"))
}

/// Save `samples` (16 kHz mono) as a WAV file in [`dir`] and return its path.
pub fn save(samples: &[f32]) -> anyhow::Result<PathBuf> {
    save_in(&dir(), samples, chrono::Local::now())
}

fn save_in(
    dir: &Path,
    samples: &[f32],
    now: chrono::DateTime<chrono::Local>,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stem = format!("recording-{}", now.format("%Y%m%d-%H%M%S"));
    let mut path = dir.join(format!("{stem}.wav"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("{stem}-{n}.wav"));
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;

    prune(dir, KEEP);
    Ok(path)
}

/// Remove all but the newest `keep` recordings. Names sort by time, so the
/// oldest come first.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut wavs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "wav")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("recording-"))
        })
        .collect();
    wavs.sort();
    let excess = wavs.len().saturating_sub(keep);
    for old in &wavs[..excess] {
        if let Err(e) = std::fs::remove_file(old) {
            tracing::debug!("Could not remove old failed recording {:?}: {}", old, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 20, 12, 0, secs)
            .single()
            .unwrap()
    }

    #[test]
    fn saved_audio_reads_back_as_16khz_mono() {
        let dir = tempfile::tempdir().unwrap();
        let samples: Vec<f32> = (0..16000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let path = save_in(dir.path(), &samples, at(0)).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len() as usize, samples.len());
    }

    #[test]
    fn same_second_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let a = save_in(dir.path(), &[0.0; 160], at(0)).unwrap();
        let b = save_in(dir.path(), &[0.0; 160], at(0)).unwrap();
        assert_ne!(a, b);
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn only_the_newest_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..KEEP as u32 + 3)
            .map(|s| save_in(dir.path(), &[0.0; 160], at(s)).unwrap())
            .collect();
        for old in &paths[..3] {
            assert!(!old.exists(), "{old:?} should have been pruned");
        }
        for kept in &paths[3..] {
            assert!(kept.exists());
        }
    }
}
