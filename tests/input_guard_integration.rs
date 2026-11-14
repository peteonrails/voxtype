//! Dead-input guard against real recordings.
//!
//! Every fixture here is an excerpt of an actual 16 kHz dictation capture,
//! not synthetic audio: a dead one taken from a recording that transcribed
//! to nothing, and live ones taken from the quietest recordings in the same
//! corpus, which are the hardest cases for a guard that must never throw
//! away a real dictation.

use std::path::PathBuf;
use voxtype::audio::input_guard::{DeadInputDetector, Verdict};
use voxtype::config::DeadInputConfig;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/input_guard")
}

/// Load a fixture as the daemon's capture would hand it over: mono f32 at
/// 16 kHz.
fn load_wav(filename: &str) -> Vec<f32> {
    let path = fixtures_dir().join(filename);
    let reader = hound::WavReader::open(&path)
        .unwrap_or_else(|e| panic!("Failed to open {}: {}", path.display(), e));

    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16000, "Expected 16kHz audio");
    assert_eq!(spec.channels, 1, "Expected mono audio");

    let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
    reader
        .into_samples::<i32>()
        .map(|sample| {
            sample.unwrap_or_else(|error| {
                panic!("Failed to decode {}: {}", path.display(), error);
            })
        })
        .map(|sample| sample as f32 / max_val)
        .collect()
}

/// Run the detector over a fixture the way the chunk pump does: in small
/// chunks, stopping at the first verdict.
fn judge(filename: &str) -> (Verdict, f32) {
    let samples = load_wav(filename);
    let mut detector = DeadInputDetector::new(16000, &DeadInputConfig::default());
    let mut verdict = Verdict::Pending;
    for chunk in samples.chunks(512) {
        verdict = detector.push(chunk);
        if verdict != Verdict::Pending {
            break;
        }
    }
    (verdict, detector.peak_dbfs())
}

#[test]
fn a_recording_that_lost_its_dictation_is_dead() {
    // 2026-11-04T18-20-50: 20.6 s of bit-exact silence. The daemon's own
    // Deepgram session for that press produced no text at all.
    let (verdict, peak) = judge("dead_silence.wav");
    assert_eq!(verdict, Verdict::Dead, "peak was {peak} dBFS");
    assert_eq!(peak, f32::NEG_INFINITY);
}

#[test]
fn the_quietest_live_recording_survives() {
    // 2026-04-08T16-43-18: the quietest capture in a corpus of 22,811 live
    // recordings, and the closest any of them comes to the threshold.
    let (verdict, peak) = judge("live_quietest.wav");
    assert_eq!(verdict, Verdict::Live, "peak was {peak} dBFS");
    assert!(
        (-90.0..-70.0).contains(&peak),
        "expected a very quiet but live peak, got {peak} dBFS"
    );
}

#[test]
fn a_recent_quiet_live_recording_survives() {
    // 2026-11-09T17-30-47: quiet room, external microphone, same month as
    // the dead captures this guard is for.
    let (verdict, peak) = judge("live_quiet_recent.wav");
    assert_eq!(verdict, Verdict::Live, "peak was {peak} dBFS");
}

#[test]
fn a_live_built_in_microphone_survives_its_dc_offset() {
    // A laptop's built-in microphone with a hardware DC bias near -0.21
    // full scale, recording nothing but a quiet room. Raw level alone would
    // read this as a loud signal and a raw-level silence test would read it
    // as silence; after the high-pass it is what it is, a live microphone.
    let samples = load_wav("live_builtin_mic.wav");
    let dc: f32 = samples.iter().sum::<f32>() / samples.len() as f32;
    assert!(
        dc < -0.15,
        "fixture should carry a large DC offset, got {dc}"
    );

    let (verdict, peak) = judge("live_builtin_mic.wav");
    assert_eq!(verdict, Verdict::Live, "peak was {peak} dBFS");
}

#[test]
fn dc_with_sub_audible_drift_is_left_alone() {
    // The measured limit of this guard, kept as a test so that a change to
    // it is deliberate: a capture carrying a DC offset plus slow drift and
    // no sound (49 clips of the corpus, nearly all from 2026-03 to
    // 2026-05) peaks at -66.7 dBFS and above through the high-pass, which
    // is inside the range live recordings occupy. The guard does not try to
    // separate that population, because no threshold does so without
    // throwing away real dictations.
    let (verdict, peak) = judge("dead_dc_drift.wav");
    assert_eq!(verdict, Verdict::Live, "peak was {peak} dBFS");
    assert!(
        peak > -80.0,
        "this population sits above the live floor, got {peak} dBFS"
    );
}

#[test]
fn the_guard_can_be_made_to_fail_on_live_audio() {
    // The control for every assertion above: with the threshold lifted to
    // -40 dBFS the same live fixtures are reported dead, so a passing
    // verdict means the detector looked at the audio and not past it.
    for fixture in [
        "live_quietest.wav",
        "live_quiet_recent.wav",
        "live_builtin_mic.wav",
    ] {
        let samples = load_wav(fixture);
        let config = DeadInputConfig {
            threshold_dbfs: -40.0,
            ..DeadInputConfig::default()
        };
        let mut detector = DeadInputDetector::new(16000, &config);
        assert_eq!(
            detector.push(&samples),
            Verdict::Dead,
            "{fixture} should fall below a -40 dBFS threshold"
        );
    }
}

#[test]
fn dead_input_defaults_are_enabled() {
    let config = DeadInputConfig::default();
    assert!(config.enabled, "the guard ships on");
    assert_eq!(config.window_ms, 1500);
    assert_eq!(config.threshold_dbfs, -100.0);
}

#[test]
fn peak_levels_match_the_offline_measurement() {
    // The thresholds in this crate were chosen from an offline survey of
    // 22,889 recordings written in Python (150 Hz 2nd-order Butterworth
    // high-pass primed on the first sample, 100 ms frames, peak frame RMS
    // over 1.5 s after a 100 ms settle). This checks that the Rust
    // implementation reproduces that measurement on the fixtures, so the
    // survey the defaults rest on describes this code.
    //
    // The threshold is set above full scale, out of reach, so that no
    // fixture ends early and every one reports its peak over the whole
    // window.
    let expected = [
        ("dead_silence.wav", f32::NEG_INFINITY),
        ("dead_dc_drift.wav", -70.4),
        ("live_quietest.wav", -82.8),
        ("live_quiet_recent.wav", -79.1),
        ("live_builtin_mic.wav", -50.8),
    ];
    for (fixture, offline_dbfs) in expected {
        let samples = load_wav(fixture);
        let config = DeadInputConfig {
            threshold_dbfs: 40.0,
            ..DeadInputConfig::default()
        };
        let mut detector = DeadInputDetector::new(16000, &config);
        assert_eq!(detector.push(&samples), Verdict::Dead);
        let peak = detector.peak_dbfs();
        if offline_dbfs == f32::NEG_INFINITY {
            assert_eq!(peak, f32::NEG_INFINITY, "{fixture}");
        } else {
            assert!(
                (peak - offline_dbfs).abs() < 0.5,
                "{fixture}: this crate measures {peak} dBFS, the survey measured {offline_dbfs} dBFS"
            );
        }
    }
}
