use super::*;
use crate::meeting::data::AudioSource;

fn seg(id: u32, start_ms: u64, end_ms: u64, text: &str, source: AudioSource) -> TranscriptSegment {
    let mut s = TranscriptSegment::new(id, start_ms, end_ms, text.to_string(), 0);
    s.source = source;
    s
}

fn port(id: u32, direction: Direction, channel: &str) -> Port {
    Port {
        id,
        node_id: 1,
        direction,
        channel: channel.to_string(),
        monitor: false,
    }
}

#[test]
fn the_builtin_clip_is_a_short_wav() {
    let reader = hound::WavReader::new(std::io::Cursor::new(BUILTIN_CLIP)).unwrap();
    assert_eq!(reader.spec().channels, 1);
    let secs = Clip::Builtin.expected_duration().as_secs_f32();
    assert!((2.0..8.0).contains(&secs), "{secs}");
    // Small enough to sit in the binary.
    assert!(BUILTIN_CLIP.len() < 300_000, "{}", BUILTIN_CLIP.len());
    assert_eq!(Clip::Builtin.phrase(), Some(ANNOUNCEMENT_TEXT));
}

#[test]
fn the_builtin_clip_is_the_one_the_feature_selects() {
    let sounds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sounds");
    let jessica = std::fs::read(sounds.join("meeting-announcement.wav")).unwrap();
    let kokoro = std::fs::read(sounds.join("meeting-announcement-kokoro.wav")).unwrap();
    assert_ne!(jessica, kokoro);
    let (expected, other) = if cfg!(feature = "announcement-kokoro") {
        (&kokoro, &jessica)
    } else {
        (&jessica, &kokoro)
    };
    assert_eq!(BUILTIN_CLIP, expected.as_slice());
    assert_ne!(BUILTIN_CLIP, other.as_slice());
    assert_eq!(
        std::fs::read(sounds.join(BUILTIN_CLIP_FILE)).unwrap(),
        BUILTIN_CLIP
    );
}

#[test]
fn off_unless_asked_for() {
    let config = MeetingConfig::default();
    assert_eq!(
        Clip::for_meeting(&config, &MeetingStartOptions::default()),
        None
    );
    let on = MeetingConfig {
        announce_recording: true,
        ..Default::default()
    };
    assert_eq!(
        Clip::for_meeting(&on, &MeetingStartOptions::default()),
        Some(Clip::Builtin)
    );
    // The start's choice beats the config either way.
    let off_for_this_one = MeetingStartOptions {
        announce: Some(false),
        ..Default::default()
    };
    assert_eq!(Clip::for_meeting(&on, &off_for_this_one), None);
    let on_for_this_one = MeetingStartOptions {
        announce: Some(true),
        ..Default::default()
    };
    assert_eq!(
        Clip::for_meeting(&config, &on_for_this_one),
        Some(Clip::Builtin)
    );
}

#[test]
fn a_custom_file_replaces_the_builtin_clip() {
    let config = MeetingConfig {
        announce_recording: true,
        announcement_file: Some("/srv/announce-de.wav".into()),
        ..Default::default()
    };
    let clip = Clip::for_meeting(&config, &MeetingStartOptions::default()).unwrap();
    assert_eq!(clip, Clip::File("/srv/announce-de.wav".into()));
    assert_eq!(clip.phrase(), None);
    let options = MeetingStartOptions {
        announcement_file: Some("/srv/other.wav".into()),
        ..Default::default()
    };
    assert_eq!(
        Clip::for_meeting(&config, &options),
        Some(Clip::File("/srv/other.wav".into()))
    );
    // A blank setting is the built-in clip.
    let blank = MeetingConfig {
        announcement_file: Some("  ".into()),
        ..config
    };
    assert_eq!(
        Clip::for_meeting(&blank, &MeetingStartOptions::default()),
        Some(Clip::Builtin)
    );
}

#[test]
fn start_options_resolve_flags_over_config() {
    let off = MeetingConfig::default();
    let on = MeetingConfig {
        announce_recording: true,
        ..Default::default()
    };
    let resolve = |c: &MeetingConfig, a, n| {
        resolve_start_options(c, MeetingStartOptions::default(), a, n, None)
            .unwrap()
            .announce
    };
    assert_eq!(resolve(&off, false, false), Some(false));
    assert_eq!(resolve(&off, true, false), Some(true));
    assert_eq!(resolve(&on, false, false), Some(true));
    assert_eq!(resolve(&on, false, true), Some(false));
}

#[test]
fn start_options_refuse_a_missing_file_only_when_announcing() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.wav");
    let err = resolve_start_options(
        &MeetingConfig::default(),
        MeetingStartOptions::default(),
        true,
        false,
        Some(&missing),
    )
    .unwrap_err();
    assert!(err.contains("nope.wav"), "{err}");
    // Not announcing, the file is never looked at.
    assert!(resolve_start_options(
        &MeetingConfig::default(),
        MeetingStartOptions::default(),
        false,
        true,
        Some(&missing),
    )
    .is_ok());

    let present = dir.path().join("mine.wav");
    std::fs::write(&present, BUILTIN_CLIP).unwrap();
    let options = resolve_start_options(
        &MeetingConfig::default(),
        MeetingStartOptions::default(),
        true,
        false,
        Some(&present),
    )
    .unwrap();
    assert_eq!(
        options.announcement_file.as_deref(),
        Some(present.to_str().unwrap())
    );
}

#[test]
fn the_builtin_clip_is_written_once_to_the_runtime_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = Clip::Builtin.materialize(dir.path()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), BUILTIN_CLIP);
    assert_eq!(Clip::Builtin.materialize(dir.path()).unwrap(), path);
    assert!(Clip::File(dir.path().join("missing.wav"))
        .materialize(dir.path())
        .is_err());
}

#[test]
fn a_mono_clip_feeds_every_input_of_the_call_stream() {
    let ours = [port(10, Direction::Out, "MONO")];
    let stereo = [port(20, Direction::In, "FL"), port(21, Direction::In, "FR")];
    assert_eq!(link_pairs(&ours, &stereo), vec![(10, 20), (10, 21)]);
    let mono = [port(30, Direction::In, "MONO")];
    assert_eq!(link_pairs(&ours, &mono), vec![(10, 30)]);
}

#[test]
fn a_stereo_clip_pairs_channels_and_falls_back_to_its_first() {
    let ours = [
        port(10, Direction::Out, "FL"),
        port(11, Direction::Out, "FR"),
    ];
    let stereo = [port(21, Direction::In, "FR"), port(20, Direction::In, "FL")];
    assert_eq!(link_pairs(&ours, &stereo), vec![(11, 21), (10, 20)]);
    let mono = [port(30, Direction::In, "MONO")];
    assert_eq!(link_pairs(&ours, &mono), vec![(10, 30)]);
}

#[test]
fn nothing_to_link_without_ports() {
    let stereo = [port(20, Direction::In, "FL")];
    assert!(link_pairs(&[], &stereo).is_empty());
    assert!(link_pairs(&[port(10, Direction::Out, "MONO")], &[]).is_empty());
    // A monitor port is not an input to feed.
    let mut monitor = port(20, Direction::In, "FL");
    monitor.monitor = true;
    assert!(link_pairs(&[port(10, Direction::Out, "MONO")], &[monitor]).is_empty());
}

#[test]
fn the_announcement_as_heard_is_stripped_whole() {
    for heard in [
        "This meeting is being recorded and transcribed with Voxtype.",
        "this meeting is being recorded and transcribed with vox type",
        "This meeting is being recorded and transcribed with Fox Type.",
        "This meeting's being recorded and transcribed with box type.",
        "Meeting is being recorded.",
    ] {
        assert_eq!(
            strip_phrase(heard, ANNOUNCEMENT_TEXT).as_deref(),
            Some(""),
            "{heard:?}"
        );
    }
}

#[test]
fn speech_around_the_announcement_stays() {
    assert_eq!(
        strip_phrase(
            "Hi everyone. This meeting is being recorded and transcribed with Voxtype. Let's start.",
            ANNOUNCEMENT_TEXT
        )
        .as_deref(),
        Some("Hi everyone. Let's start.")
    );
    // Lines that only share a word or two are not the announcement.
    for line in [
        "Let's start the meeting.",
        "Is everyone here?",
        "We shipped the build with the fix.",
    ] {
        assert_eq!(strip_phrase(line, ANNOUNCEMENT_TEXT), None, "{line:?}");
    }
}

#[test]
fn the_window_drops_the_announcement_lines_and_nothing_else() {
    let window = Window {
        start_ms: 5_000,
        end_ms: 8_200,
        phrase: Some(ANNOUNCEMENT_TEXT.to_string()),
    };
    let mut segments = vec![
        seg(
            1,
            5_300,
            8_400,
            "This meeting is being recorded and transcribed with Voxtype.",
            AudioSource::Loopback,
        ),
        seg(
            2,
            5_100,
            8_000,
            "This meeting is being recorded and transcribed with box type.",
            AudioSource::Microphone,
        ),
        seg(
            3,
            8_500,
            10_000,
            "Thanks, sounds good.",
            AudioSource::Microphone,
        ),
        // Said again long after the clip: a person quoting it keeps the line.
        seg(
            4,
            60_000,
            63_000,
            "This meeting is being recorded and transcribed with Voxtype.",
            AudioSource::Microphone,
        ),
    ];
    assert_eq!(window.scrub(&mut segments), 2);
    let kept: Vec<u32> = segments.iter().map(|s| s.id).collect();
    assert_eq!(kept, vec![3, 4]);
}

#[test]
fn a_custom_clip_is_judged_by_time_alone() {
    let window = Window {
        start_ms: 5_000,
        end_ms: 9_000,
        phrase: None,
    };
    let mut segments = vec![
        seg(
            1,
            5_200,
            8_800,
            "Dieses Meeting wird aufgezeichnet.",
            AudioSource::Loopback,
        ),
        seg(
            2,
            8_000,
            14_000,
            "Okay, first item on the agenda is the release.",
            AudioSource::Microphone,
        ),
        seg(3, 1_000, 4_000, "Hello?", AudioSource::Microphone),
    ];
    assert_eq!(window.scrub(&mut segments), 1);
    let kept: Vec<u32> = segments.iter().map(|s| s.id).collect();
    assert_eq!(kept, vec![2, 3]);
}

#[test]
fn status_lines_never_claim_the_call_heard_a_local_play() {
    let local = Status::local("no call app opened the microphone within 120 s");
    assert!(
        local.describe().contains("did not hear it"),
        "{}",
        local.describe()
    );
    assert_eq!(local.to_json()["state"], "local");
    let call = Status::call("Zoom");
    assert_eq!(
        call.describe(),
        "played into Zoom's microphone and on your speakers"
    );
    let json = call.to_json();
    assert_eq!(json["state"], "call");
    assert_eq!(json["app"], "Zoom");
    let back: Status = serde_json::from_value(json).unwrap();
    assert_eq!(back, call);
}

/// Live tests against the real PipeWire, silent by construction: the "call"
/// is a `pw-record` capture stream whose own mic is a null sink's monitor,
/// and the local copy plays to a second null sink. Run with
/// `cargo test --lib announce::tests::live -- --ignored --test-threads=1`
/// outside any sandbox that hides the PipeWire socket.
mod live {
    use super::*;
    use crate::config::{CallAppPattern, MeetingDetectConfig};
    use std::process::Command;

    /// A null sink that is unloaded again when dropped.
    struct NullSink {
        name: String,
        module: String,
    }

    impl NullSink {
        fn new(name: &str) -> Self {
            let out = Command::new("pactl")
                .args([
                    "load-module",
                    "module-null-sink",
                    &format!("sink_name={name}"),
                    &format!("sink_properties=device.description={name}"),
                ])
                .output()
                .expect("pactl");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            Self {
                name: name.to_string(),
                module: String::from_utf8_lossy(&out.stdout).trim().to_string(),
            }
        }
    }

    impl Drop for NullSink {
        fn drop(&mut self) {
            let _ = Command::new("pactl")
                .args(["unload-module", &self.module])
                .status();
        }
    }

    /// A stand-in call app: a capture stream named `app` recording its
    /// "mic" (the null sink's monitor) to `out`.
    struct FakeCall(std::process::Child);

    impl FakeCall {
        fn start(app: &str, mic: &NullSink, out: &Path) -> Self {
            let child = Command::new("pw-record")
                .arg("--target")
                .arg(&mic.name)
                .arg("-P")
                .arg(format!(
                    "{{ application.name = \"{app}\" node.name = \"{app}\" stream.capture.sink = true }}"
                ))
                .arg(out)
                .spawn()
                .expect("pw-record");
            std::thread::sleep(Duration::from_millis(1000));
            Self(child)
        }

        /// Stop recording, leaving a complete WAV.
        fn stop(mut self) {
            unsafe {
                libc::kill(self.0.id() as i32, libc::SIGTERM);
            }
            let _ = self.0.wait();
        }
    }

    fn detect_for(app: &str) -> MeetingDetectConfig {
        MeetingDetectConfig {
            apps: vec![CallAppPattern {
                name: "Fake Call".into(),
                streams: vec![app.into()],
                titles: Vec::new(),
            }],
            ..Default::default()
        }
    }

    /// The loudest sample in each 100 ms of a WAV, as a fraction of full
    /// scale.
    fn peaks(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        let block = (spec.sample_rate / 10 * u32::from(spec.channels)) as usize;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / (1u32 << (spec.bits_per_sample - 1)) as f32)
                .collect(),
            hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
        };
        samples
            .chunks(block)
            .map(|c| c.iter().fold(0f32, |m, s| m.max(s.abs())))
            .collect()
    }

    #[tokio::test]
    #[ignore = "needs a running PipeWire"]
    async fn the_call_hears_the_announcement_through_its_own_mic_stream() {
        let tag = std::process::id();
        let dir = tempfile::tempdir().unwrap();
        let mic = NullSink::new(&format!("vt_announce_mic_{tag}"));
        let speakers = NullSink::new(&format!("vt_announce_speakers_{tag}"));
        let app = format!("vt-fakecall-{tag}");
        let recording = dir.path().join("call.wav");
        let call = FakeCall::start(&app, &mic, &recording);

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut runner = Runner::new(Clip::Builtin, detect_for(&app), dir.path().to_path_buf());
        runner.wait = Duration::from_secs(5);
        runner.local_target = Some(speakers.name.clone());
        let status = runner.run(tx).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        call.stop();

        assert_eq!(status, Status::call("Fake Call"), "{}", status.describe());
        let mut updates = Vec::new();
        while let Ok(u) = rx.try_recv() {
            updates.push(u);
        }
        assert!(
            matches!(
                updates.first(),
                Some(Update::Started {
                    phrase: Some(_),
                    ..
                })
            ),
            "{updates:?}"
        );
        assert!(matches!(updates.last(), Some(Update::Finished { .. })));

        // The call's own capture got the clip, though its mic is silent.
        let peaks = peaks(&recording);
        let loud = peaks.iter().filter(|&&p| p > 0.05).count();
        eprintln!(
            "call recording: {} blocks, {} loud, max {:.3}",
            peaks.len(),
            loud,
            peaks.iter().cloned().fold(0., f32::max)
        );
        assert!(loud >= 20, "only {loud} loud 100 ms blocks: {peaks:?}");
        // At the clip's own level (peak about 0.65), not a remembered
        // stream volume.
        let max = peaks.iter().cloned().fold(0f32, f32::max);
        assert!(max > 0.5, "the call heard it at {max:.3} of full scale");
    }

    #[tokio::test]
    #[ignore = "needs a running PipeWire"]
    async fn without_a_call_it_plays_locally_and_says_so() {
        let tag = std::process::id();
        let dir = tempfile::tempdir().unwrap();
        let speakers = NullSink::new(&format!("vt_announce_speakers2_{tag}"));
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut runner = Runner::new(
            Clip::Builtin,
            detect_for(&format!("vt-nocall-{tag}")),
            dir.path().to_path_buf(),
        );
        runner.wait = Duration::from_secs(1);
        runner.local_target = Some(speakers.name.clone());
        let status = runner.run(tx).await;
        assert_eq!(status.state, State::Local, "{}", status.describe());
        assert!(status.detail.contains("no call app"), "{}", status.detail);
        assert!(matches!(rx.try_recv(), Ok(Update::Waiting)));
    }
}
