//! An in-process daemon to test the event loop against.
//!
//! Everything outside the process is faked: the runtime directory is temporary,
//! hotkey events are pushed by the test, the capture hands back a scripted
//! buffer, the transcriber returns scripted text, and the output driver records
//! what it would have typed. No audio device, no display, no model file, and no
//! signal to the test process.
//!
//! Usage:
//!
//! ```ignore
//! let harness = TestDaemon::speaking("hello world");
//! let ctl = harness.controls();
//! harness
//!     .run(async {
//!         ctl.press();
//!         assert!(ctl.wait_for_state("recording").await);
//!         tokio::time::sleep(Duration::from_millis(400)).await;
//!         ctl.release();
//!         assert!(ctl.wait_for_state("idle").await);
//!     })
//!     .await;
//! assert_eq!(ctl.typed(), ["hello world"]);
//! ```

use super::deps::{Deps, Factories};
use super::Daemon;
use crate::audio::{AudioCapture, DualSamples, MeetingCapture};
use crate::config::{ActivationMode, Config, OutputConfig, Profile};
use crate::error::{AudioError, OutputError, TranscribeError};
use crate::output::TextOutput;
use crate::runtime_files::RuntimePaths;
use crate::transcribe::{StreamHandle, StreamingEvent, StreamingTranscriber, Transcriber};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

#[cfg(target_os = "linux")]
use crate::hotkey::HotkeyEvent;
#[cfg(target_os = "macos")]
use crate::hotkey_macos::HotkeyEvent;

/// How long a scenario waits for a state before calling it a failure.
const STATE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the fakes should do, for the rows that need something other than a
/// clean recording.
#[derive(Clone, Copy, Default)]
pub struct Fakes {
    /// A transcriber that can stream, so the streaming pipeline is exercised.
    pub streaming: bool,
    /// Record silence, so the speech gate has nothing to find.
    pub silence: bool,
    /// Fail to open a capture, as a missing or busy device does.
    pub capture_fails: bool,
    /// Fail to transcribe, as a model that cannot decode does.
    pub transcriber_fails: bool,
    /// Milliseconds the engine takes, so a row can act while it is running.
    pub transcribe_delay_ms: u64,
    /// Have the streaming backend report a failure instead of a final segment.
    pub stream_error: bool,
    /// Have the streaming backend end the session on its own, the way a remote
    /// stream does when the socket closes or the server finishes.
    pub stream_self_ends: bool,
    /// Load the engine before the first recording instead of on demand, so a
    /// poisoned instance has somewhere to be cached.
    pub preload: bool,
    /// Panic on the first transcription, as a broken engine does (#643).
    pub panic_once: bool,
    /// Turn on the eager pipeline, which transcribes chunks while recording.
    pub eager: bool,
    /// Fail to build the capture pair a meeting records from, as a machine with
    /// no loopback device or a busy one does.
    pub meeting_capture_unavailable: bool,
    /// Fail the transcriber factory from this call on (1-based), as an engine
    /// that loads once and then stops being able to.
    pub factory_fails_from: Option<usize>,
}

/// Audio capture that hands the daemon a scripted buffer.
///
/// It records for as long as the daemon holds it open and returns only that
/// much audio, because the daemon's accidental-press floor is a count of
/// samples: a fake that always returned the whole buffer would make every
/// duration-dependent row meaningless.
struct FakeCapture {
    samples: Vec<f32>,
    /// How many times the daemon stopped this capture, so a row can pin that a
    /// session closed the microphone even though it was not asked to stop.
    stops: Arc<AtomicUsize>,
    /// Fail `start`, as an unavailable device does.
    fail: bool,
    /// When the daemon opened the device, so `stop` can say how much it got.
    started_at: Option<Instant>,
    /// Samples handed out by `get_samples` so far.
    drained: usize,
    /// Kept alive so the frame tap's channel does not close while recording.
    feed: Option<mpsc::Sender<Vec<f32>>>,
}

impl FakeCapture {
    fn new(seconds: f32, silence: bool, fail: bool, stops: Arc<AtomicUsize>) -> Self {
        let count = (seconds * 16_000.0) as usize;
        // A tone rather than silence by default: the level tap and the speech
        // gate see something shaped like audio, so the daemon's silence paths
        // stay out of the way. A row that is about silence asks for zeros.
        let samples = (0..count)
            .map(|i| {
                if silence {
                    0.0
                } else {
                    let t = i as f32 / 16_000.0;
                    0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                }
            })
            .collect();
        Self {
            samples,
            stops,
            fail,
            started_at: None,
            drained: 0,
            feed: None,
        }
    }

    /// How many samples a recording of `elapsed` would have collected.
    fn recorded(&self, elapsed: Duration) -> usize {
        let recorded = (elapsed.as_secs_f32() * 16_000.0) as usize;
        recorded.min(self.samples.len())
    }
}

#[async_trait::async_trait]
impl AudioCapture for FakeCapture {
    async fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, AudioError> {
        if self.fail {
            return Err(AudioError::Connection(
                "fake capture unavailable".to_string(),
            ));
        }
        let (tx, rx) = mpsc::channel(8);
        let _ = tx.send(self.samples.clone()).await;
        self.feed = Some(tx);
        self.started_at = Some(Instant::now());
        self.drained = 0;
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<Vec<f32>, AudioError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.feed = None;
        let recorded = match self.started_at.take() {
            Some(at) => self.recorded(at.elapsed()),
            None => 0,
        };
        Ok(self.samples[..recorded].to_vec())
    }

    async fn get_samples(&mut self) -> Vec<f32> {
        let recorded = match self.started_at {
            Some(at) => self.recorded(at.elapsed()),
            None => 0,
        };
        let fresh = self.samples[self.drained..recorded].to_vec();
        self.drained = recorded;
        fresh
    }
}

/// Transcriber that returns scripted text and records the audio it was handed.
struct FakeTranscriber {
    text: String,
    fail: bool,
    delay_ms: u64,
    /// Set by the first transcription, so `panic_once` happens once.
    panicked: Arc<AtomicBool>,
    panic_once: bool,
    calls: Arc<Mutex<Vec<usize>>>,
    /// Bumped when the last reference goes away, so a row can pin that a cycle
    /// released the engine it loaded.
    drops: Arc<AtomicUsize>,
}

impl Drop for FakeTranscriber {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.calls.lock().expect("calls lock").push(samples.len());
        if self.panic_once && !self.panicked.swap(true, Ordering::SeqCst) {
            // Inside `spawn_blocking`, so this surfaces as a JoinError whose
            // `is_panic` is set, and the daemon drops its cached engine (#643).
            panic!("fake engine panic");
        }
        if self.delay_ms > 0 {
            // Real engines take time, and the cancel-during-transcription rows
            // need a window to act in. `transcribe` is synchronous and runs
            // inside `spawn_blocking`, so sleeping here is honest.
            std::thread::sleep(Duration::from_millis(self.delay_ms));
        }
        if self.fail {
            return Err(TranscribeError::ModelNotFound("fake failure".to_string()));
        }
        Ok(self.text.clone())
    }
}

/// A transcriber that can stream, so the streaming pipeline is reachable.
///
/// It emits one `Final` when the daemon drops the samples channel (end of
/// input) and `Ended` after it, which is the shape a real backend produces.
struct FakeStreamingTranscriber {
    text: String,
    fail: bool,
    /// End the session after the first segment instead of waiting for the
    /// daemon to stop the capture.
    self_ends: bool,
    calls: Arc<Mutex<Vec<usize>>>,
}

impl Transcriber for FakeStreamingTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.calls.lock().expect("calls lock").push(samples.len());
        Ok(self.text.clone())
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        Some(self)
    }
}

impl StreamingTranscriber for FakeStreamingTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let (events_tx, events) = mpsc::channel(16);
        // The daemon sends on this to abort a session. This fake ends when the
        // samples channel closes instead, so the receiver only has to exist for
        // the sender to be constructible.
        let (cancel, _abandoned) = oneshot::channel();
        let text = self.text.clone();
        let fail = self.fail;
        let self_ends = self.self_ends;

        let task = tokio::spawn(async move {
            // A real backend commits a Final as an utterance completes, while
            // the session is live. It must not wait for the end: a live session
            // disowns its typing surface at stop, so anything emitted during the
            // drain is discarded by design.
            let first = samples_rx.recv().await;
            if fail {
                // The failure is reported when the session ends.
                while samples_rx.recv().await.is_some() {}
                let _ = events_tx
                    .send(StreamingEvent::Error(TranscribeError::ModelNotFound(
                        "fake stream failure".to_string(),
                    )))
                    .await;
                return Ok(());
            }
            if first.is_some() && !text.is_empty() {
                let _ = events_tx
                    .send(StreamingEvent::Final {
                        text,
                        segment_id: 1,
                    })
                    .await;
            }
            if self_ends {
                // A remote backend ends the session on its own terms: the socket
                // closes, the server finishes, the task dies. It stops reading
                // here, which leaves the daemon's pump with nowhere to send.
                // The delay is what makes the live phase observable: a backend
                // that ended in the same tick as the start would race the test.
                tokio::time::sleep(Duration::from_millis(150)).await;
                let _ = events_tx.send(StreamingEvent::Ended).await;
                return Ok(());
            }
            while samples_rx.recv().await.is_some() {}
            let _ = events_tx.send(StreamingEvent::Ended).await;
            Ok(())
        });

        Ok(StreamHandle {
            events,
            cancel,
            task,
        })
    }
}

/// A meeting's capture pair with no devices behind it.
///
/// `DualCapture::new` opens real devices, which is why meetings had no rows: a
/// test could not get past the first line of `start_meeting`. This one starts,
/// hands out silence (so the chunk pump has nothing to transcribe) and reports
/// whether it has a loopback leg.
#[derive(Default)]
struct FakeDualCapture {
    /// Whether a loopback leg is part of the pair.
    loopback: bool,
}

#[async_trait::async_trait]
impl MeetingCapture for FakeDualCapture {
    async fn start(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    async fn stop(&mut self) -> Result<DualSamples, AudioError> {
        Ok(DualSamples::default())
    }

    async fn get_samples(&mut self) -> DualSamples {
        DualSamples::default()
    }

    fn has_loopback(&self) -> bool {
        self.loopback
    }
}

/// Output driver that records what would have been typed.
#[derive(Clone, Default)]
struct FakeOutput {
    typed: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl TextOutput for FakeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        self.typed
            .lock()
            .expect("typed lock")
            .push(text.to_string());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "fake"
    }
}

/// What a scenario uses to drive the daemon and read what it did.
#[derive(Clone)]
pub struct Controls {
    typed: Arc<Mutex<Vec<String>>>,
    transcribed: Arc<Mutex<Vec<usize>>>,
    output_configs: Arc<Mutex<Vec<OutputConfig>>>,
    transcriber_factory_calls: Arc<AtomicUsize>,
    capture_stops: Arc<AtomicUsize>,
    transcriber_drops: Arc<AtomicUsize>,
    hotkey_tx: mpsc::Sender<HotkeyEvent>,
    external_start_tx: mpsc::Sender<()>,
    external_stop_tx: mpsc::Sender<()>,
    hook_log: PathBuf,
    state_file: PathBuf,
    paths: RuntimePaths,
}

impl Controls {
    pub fn press(&self) {
        self.send(HotkeyEvent::Pressed {
            model_override: None,
            profile_override: None,
        });
    }

    pub fn release(&self) {
        self.send(HotkeyEvent::Released);
    }

    /// Press with the profile modifier held, as the hotkey listener reports it.
    pub fn press_with_profile(&self, profile: &str) {
        self.send(HotkeyEvent::Pressed {
            model_override: None,
            profile_override: Some(profile.to_string()),
        });
    }

    /// Write the profile sentinel the way `voxtype record start --profile` does.
    pub fn write_profile_sentinel(&self, name: &str) {
        std::fs::write(self.paths.profile_override(), name).expect("write profile sentinel");
    }

    pub fn profile_sentinel_exists(&self) -> bool {
        self.paths.profile_override().exists()
    }

    /// The cancel key: discard the cycle in flight without a transcript.
    pub fn cancel(&self) {
        self.send(HotkeyEvent::Cancel);
    }

    /// An external start, as `voxtype record start` sends with SIGUSR1.
    pub fn external_start(&self) {
        let _ = self.external_start_tx.try_send(());
    }

    /// An external stop, as `voxtype record stop` sends with SIGUSR2.
    pub fn external_stop(&self) {
        let _ = self.external_stop_tx.try_send(());
    }

    /// How many times the external stop hook has run. The configured hook
    /// appends one byte per run to a file, so its length is the count.
    pub fn hook_runs(&self) -> usize {
        std::fs::read_to_string(&self.hook_log)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// Write a sentinel the way `voxtype record start --auto-submit` would.
    pub fn write_override(&self, name: &str, value: &str) {
        std::fs::write(self.paths.bool_override(name), value).expect("write override");
    }

    pub fn override_exists(&self, name: &str) -> bool {
        self.paths.bool_override(name).exists()
    }

    /// How many times the daemon asked the factory for a transcriber: once for
    /// a preloaded engine, twice when a poisoned one is discarded.
    /// What the meeting state file says, if it has been written. The daemon
    /// reports meetings through this file, not through its own state.
    pub fn meeting_state(&self) -> Option<String> {
        std::fs::read_to_string(self.paths.meeting_state()).ok()
    }

    /// How many times a capture was stopped. The external stop and any stop the
    /// daemon decides on itself both count here.
    pub fn capture_stops(&self) -> usize {
        self.capture_stops.load(Ordering::SeqCst)
    }

    /// How many engines have been dropped. The load task holds one when a cycle
    /// loads a model it never transcribes with, so this is what shows a released
    /// load against a parked one.
    pub fn transcriber_drops(&self) -> usize {
        self.transcriber_drops.load(Ordering::SeqCst)
    }

    pub fn transcriber_factory_calls(&self) -> usize {
        self.transcriber_factory_calls.load(Ordering::SeqCst)
    }

    /// The output configuration every chain was built with, in order: the
    /// startup log line first, then one per delivered transcription. The flags
    /// the daemon resolved for a cycle are visible here.
    pub fn output_configs(&self) -> Vec<OutputConfig> {
        self.output_configs.lock().expect("configs lock").clone()
    }

    fn send(&self, event: HotkeyEvent) {
        // The queue is small and the daemon drains it; a failure here means the
        // daemon stopped, which the state assertions will report.
        let _ = self.hotkey_tx.try_send(event);
    }

    /// The state file's current contents ("idle", "recording", ...).
    pub fn state(&self) -> String {
        std::fs::read_to_string(&self.state_file)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    /// Text the fake output driver was asked to deliver, in order.
    pub fn typed(&self) -> Vec<String> {
        self.typed.lock().expect("typed lock").clone()
    }

    /// How many samples the fake transcriber was handed, per call.
    pub fn transcription_calls(&self) -> Vec<usize> {
        self.transcribed.lock().expect("calls lock").clone()
    }

    async fn wait_for_state(&self, want: &str) -> bool {
        let deadline = Instant::now() + STATE_TIMEOUT;
        while Instant::now() < deadline {
            if self.state() == want {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    /// Wait for a state, reporting the states seen instead of only a failure.
    pub async fn expect_state(&self, want: &str) {
        assert!(
            self.wait_for_state(want).await,
            "never reached state {want:?}; ended at {:?}",
            self.state()
        );
    }
}

/// A daemon wired to fakes, with the handles a scenario needs.
pub struct TestDaemon {
    daemon: Option<Daemon>,
    shutdown: Option<oneshot::Sender<()>>,
    controls: Controls,
    _dir: TempDir,
}

impl TestDaemon {
    /// A daemon that records for a moment, transcribes to `text`, and returns
    /// to idle.
    pub fn speaking(text: &str) -> Self {
        Self::with_config(text, |_| {})
    }

    /// A daemon whose transcriber can stream, so the streaming pipeline is the
    /// one under test.
    pub fn streaming(text: &str) -> Self {
        Self::build(
            text,
            Fakes {
                streaming: true,
                ..Fakes::default()
            },
            |_| {},
        )
    }

    /// As [`TestDaemon::speaking`], with fakes configured for the row under
    /// test (silence, a failing device, a failing engine).
    pub fn with_fakes(text: &str, fakes: Fakes, tweak: impl FnOnce(&mut Config)) -> Self {
        Self::build(text, fakes, tweak)
    }

    /// As [`TestDaemon::speaking`], with a chance to adjust the configuration
    /// before the daemon is built.
    pub fn with_config(text: &str, tweak: impl FnOnce(&mut Config)) -> Self {
        Self::build(text, Fakes::default(), tweak)
    }

    fn build(text: &str, fakes: Fakes, tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let state_file = dir.path().join("state");
        let hook_log = dir.path().join("hook-runs");
        let mut config = base_config(fakes.streaming || fakes.preload);
        config.hotkey.enabled = true;
        config.hotkey.mode = ActivationMode::PushToTalk;
        config.state_file = Some(state_file.display().to_string());
        // Nothing outside the process: no OSD, no feedback sounds, no desktop
        // notifications, no voice-activity gate, and meetings stored in the
        // temporary directory rather than the user's data directory.
        config.osd.enabled = false;
        config.audio.feedback.enabled = false;
        config.vad.enabled = false;
        let notification = &mut config.output.notification;
        notification.on_recording_start = false;
        notification.on_recording_stop = false;
        notification.on_transcription = false;
        config.meeting.storage_path = dir.path().join("meetings").display().to_string();
        config.whisper.eager_processing = fakes.eager;
        // The external stop hook is a shell command, so a test observes it by
        // having it append one byte per run.
        config.audio.external_trigger_stop_command =
            Some(format!("printf x >> '{}'", hook_log.display()));
        tweak(&mut config);

        let (hotkey_tx, hotkey_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (external_start_tx, external_start_rx) = mpsc::channel(4);
        let (external_stop_tx, external_stop_rx) = mpsc::channel(4);
        let typed = Arc::new(Mutex::new(Vec::new()));
        let transcribed = Arc::new(Mutex::new(Vec::new()));
        let output_configs = Arc::new(Mutex::new(Vec::new()));

        let output = FakeOutput {
            typed: typed.clone(),
        };
        let calls = transcribed.clone();
        let panicked = Arc::new(AtomicBool::new(false));
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let transcriber_factory_calls = factory_calls.clone();
        let capture_stops = Arc::new(AtomicUsize::new(0));
        let capture_stops_for_factory = capture_stops.clone();
        let meeting_capture_unavailable = fakes.meeting_capture_unavailable;
        let transcriber_drops = Arc::new(AtomicUsize::new(0));
        let transcriber_drops_for_factory = transcriber_drops.clone();
        let configs = output_configs.clone();
        let scripted = text.to_string();
        let factories = Factories {
            capture: Some(Arc::new(move |_config| {
                Ok(Box::new(FakeCapture::new(
                    1.5,
                    fakes.silence,
                    fakes.capture_fails,
                    capture_stops_for_factory.clone(),
                )) as Box<dyn AudioCapture>)
            })),
            transcriber: Some(Arc::new(move |_config| {
                let call = transcriber_factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
                if fakes.factory_fails_from.is_some_and(|from| call >= from) {
                    return Err(TranscribeError::ModelNotFound(
                        "fake factory failure".to_string(),
                    ));
                }
                if fakes.streaming {
                    Ok(Box::new(FakeStreamingTranscriber {
                        text: scripted.clone(),
                        fail: fakes.stream_error,
                        self_ends: fakes.stream_self_ends,
                        calls: calls.clone(),
                    }) as Box<dyn Transcriber>)
                } else {
                    Ok(Box::new(FakeTranscriber {
                        text: scripted.clone(),
                        fail: fakes.transcriber_fails,
                        delay_ms: fakes.transcribe_delay_ms,
                        panicked: panicked.clone(),
                        panic_once: fakes.panic_once,
                        calls: calls.clone(),
                        drops: transcriber_drops_for_factory.clone(),
                    }) as Box<dyn Transcriber>)
                }
            })),
            dual_capture: Some(Arc::new(move |_config, _loopback| {
                if meeting_capture_unavailable {
                    return Err(AudioError::Connection(
                        "fake meeting capture unavailable".to_string(),
                    ));
                }
                Ok(Box::new(FakeDualCapture::default()) as Box<dyn MeetingCapture>)
            })),
            output_chain: Some(Arc::new(move |config| {
                configs.lock().expect("configs lock").push(config.clone());
                vec![Box::new(output.clone()) as Box<dyn TextOutput>]
            })),
        };
        let deps = Deps {
            factories,
            hotkey_events: Some(hotkey_rx),
            shutdown: Some(shutdown_rx),
            external_start: Some(external_start_rx),
            external_stop: Some(external_stop_rx),
            exit_process_on_shutdown: false,
        };

        let paths = RuntimePaths::new(dir.path());
        let daemon = Daemon::with_deps(config, None, paths.clone(), deps);
        let controls = Controls {
            typed,
            transcribed,
            output_configs,
            transcriber_factory_calls: factory_calls,
            capture_stops,
            transcriber_drops,
            hotkey_tx,
            external_start_tx,
            external_stop_tx,
            hook_log,
            state_file,
            paths,
        };
        Self {
            daemon: Some(daemon),
            shutdown: Some(shutdown_tx),
            controls,
            _dir: dir,
        }
    }

    pub fn controls(&self) -> Controls {
        self.controls.clone()
    }

    /// Run the daemon and `scenario` on one task until the scenario finishes,
    /// then stop the daemon the way a signal would. Both halves share the task,
    /// so a failure in either surfaces in the test instead of a detached task.
    pub async fn run<F: Future<Output = ()>>(mut self, scenario: F) {
        let mut daemon = self.daemon.take().expect("daemon is taken once");
        let shutdown = self.shutdown.take();
        let run = async { daemon.run().await.expect("daemon run") };
        let drive = async {
            scenario.await;
            if let Some(tx) = shutdown {
                let _ = tx.send(());
            }
        };
        let ((), ()) = tokio::join!(run, drive);
    }
}

/// A first-run configuration with one deliberate change: the engine.
///
/// The Whisper path builds its transcriber through `ModelManager` (a streaming
/// wrapper, or a gpu-isolation worker), which is not one of the factories the
/// daemon takes. Every other engine builds it through
/// `Deps::create_transcriber`, so SenseVoice with on-demand loading is the
/// engine a harness can stand in for.
const BATCH_CONFIG: &str = r#"
engine = "sensevoice"

[sensevoice]
model = "sensevoice-small"
on_demand_loading = true
"#;

/// Streaming needs the transcriber before recording starts: `try_start_streaming`
/// reads the preloaded slot, which only the non-on-demand path fills.
const STREAMING_CONFIG: &str = r#"
engine = "sensevoice"

[sensevoice]
model = "sensevoice-small"
on_demand_loading = false
"#;

fn base_config(preload: bool) -> Config {
    let text = if preload {
        STREAMING_CONFIG
    } else {
        BATCH_CONFIG
    };
    toml::from_str(text).expect("the harness configuration deserializes")
}

#[tokio::test]
async fn a_push_to_talk_cycle_reaches_idle_with_a_transcript() {
    // The done test for the harness: a whole cycle, in process, with no audio
    // device, no display and no model.
    let harness = TestDaemon::speaking("hello world");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;

            // The daemon ignores anything under 0.3 s as an accidental press.
            tokio::time::sleep(Duration::from_millis(400)).await;

            ctl.release();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(ctl.typed(), vec!["hello world".to_string()]);
    assert_eq!(
        ctl.transcription_calls().len(),
        1,
        "one recording, one call"
    );
    let samples = ctl.transcription_calls()[0];
    let seconds = samples as f32 / 16_000.0;
    assert!(
        (0.3..=1.6).contains(&seconds),
        "the engine got {seconds:.2}s of audio: the recording was delivered, above \
         the accidental-press floor and within what was recorded"
    );
}

#[tokio::test]
async fn a_cancelled_recording_does_not_leak_its_submit_override() {
    // The boolean overrides are read when a transcript is delivered, so a
    // cancelled cycle has to clear them: otherwise the `--auto-submit` written
    // for the recording the user just discarded is applied to whatever
    // recording is delivered next.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.write_override("auto_submit", "true");

            ctl.press();
            ctl.expect_state("recording").await;
            ctl.cancel();
            ctl.expect_state("idle").await;
            assert!(
                !ctl.override_exists("auto_submit"),
                "the cancel left the sentinel for the next cycle to consume"
            );

            // An unrelated recording, started with no override of its own.
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ctl.release();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the second recording still has to be delivered"
    );
    assert!(
        ctl.output_configs().iter().all(|c| !c.auto_submit),
        "the cancelled recording's --auto-submit was applied to the next one: {:?}",
        ctl.output_configs()
            .iter()
            .map(|c| c.auto_submit)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_cancelled_external_streaming_session_runs_the_stop_hook() {
    // Compositor users leave a submap in the stop hook, so a session that ends
    // without running it strands them. Cancelling a *streaming* session used to
    // skip it: the cancel path tore the session down without telling the
    // caller, and left `is_external_trigger` set for the next session.
    let harness = TestDaemon::streaming("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("streaming").await;
            assert_eq!(
                ctl.hook_runs(),
                0,
                "the hook marks the end of the session, not its start"
            );

            ctl.cancel();
            ctl.expect_state("idle").await;

            // Asserted inside the run: the harness owns the temporary runtime
            // directory and drops it when `run` returns, so a file check after
            // that would read nothing whatever the daemon did. The state file
            // flips to idle only after the cancel path has finished, hook
            // included.
            assert_eq!(
                ctl.hook_runs(),
                1,
                "cancelling an external streaming session has to run the stop hook once"
            );
        })
        .await;
}

#[tokio::test]
async fn a_profile_override_belongs_to_the_recording_it_was_started_for() {
    // The profile modifier used to be laundered through a runtime file: the
    // press wrote it, delivery read it, and seven cancel paths had to remember
    // to delete it. A cycle that never reached delivery left it behind, and the
    // next transcript was post-processed by a profile the user did not ask for.
    let harness = TestDaemon::with_config("hello", |config| {
        config.profiles.insert(
            "shout".to_string(),
            Profile {
                post_process_command: Some("tr a-z A-Z".to_string()),
                post_process_timeout_ms: None,
                output_mode: None,
            },
        );
    });
    let ctl = harness.controls();

    harness
        .run(async {
            // Started with the modifier: this one is post-processed by it.
            ctl.press_with_profile("shout");
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            // The next recording has no modifier, and must not inherit one.
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            // A sentinel the CLI wrote for a start that never happened is
            // cleared at the end of a cycle rather than waiting for an
            // unrelated start to consume it.
            ctl.write_profile_sentinel("shout");
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ctl.release();
            ctl.expect_state("idle").await;
            assert!(
                !ctl.profile_sentinel_exists(),
                "the stale sentinel outlived the cycle"
            );
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec![
            "HELLO".to_string(),
            "hello".to_string(),
            "hello".to_string()
        ],
        "only the recording started with the modifier is post-processed by it"
    );
}

#[tokio::test]
async fn a_stale_cancel_sentinel_does_not_swallow_the_next_recording() {
    // `voxtype record cancel` writes a trigger file. If the daemon is idle when
    // it arrives the file stays, because the 500 ms idle arm that was meant to
    // sweep it can never fire: the unconditional 100 ms arm recreates its timer
    // on every iteration (#644). The sweep that works is the one at capture
    // start (#606), and this pins it now that the dead arm is gone.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            std::fs::write(ctl.paths.cancel(), "cancel").expect("write cancel sentinel");

            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ctl.release();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the stale cancel sentinel swallowed the recording"
    );
}

/// Comfortably over the daemon's 0.3 s accidental-press floor.
const FLOOR_MS: u64 = 500;

/// A press and release that records something worth transcribing.
async fn record_once(ctl: &Controls) {
    ctl.press();
    ctl.expect_state("recording").await;
    tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
    ctl.release();
    ctl.expect_state("idle").await;
}

#[tokio::test]
async fn a_recording_below_the_floor_is_discarded() {
    // Released inside the floor: an accidental press, so nothing is built and
    // nothing is delivered.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.release();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(ctl.typed(), Vec::<String>::new());
    assert_eq!(ctl.transcription_calls(), Vec::<usize>::new());
}

#[tokio::test]
async fn a_silent_recording_is_skipped_by_the_speech_gate() {
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            silence: true,
            ..Fakes::default()
        },
        |config| config.vad.enabled = true,
    );
    let ctl = harness.controls();

    harness.run(async { record_once(&ctl).await }).await;

    assert_eq!(
        ctl.transcription_calls(),
        Vec::<usize>::new(),
        "a silent recording never reaches the engine"
    );
}

#[tokio::test]
async fn a_toggle_press_starts_and_a_second_stops() {
    let harness = TestDaemon::with_config("hello", |config| {
        config.hotkey.mode = ActivationMode::Toggle
    });
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.press();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(ctl.typed(), vec!["hello".to_string()]);
}

#[tokio::test]
async fn an_external_stop_delivers_the_recording_and_runs_the_hook() {
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert_eq!(
                ctl.hook_runs(),
                1,
                "an external session ends through its hook"
            );
        })
        .await;

    assert_eq!(ctl.typed(), vec!["hello".to_string()]);
}

#[tokio::test]
async fn an_external_recording_below_the_floor_still_ends_the_session() {
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("recording").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert_eq!(
                ctl.hook_runs(),
                1,
                "the session has to end even with nothing to deliver"
            );
        })
        .await;

    assert_eq!(ctl.transcription_calls(), Vec::<usize>::new());
}

#[tokio::test]
async fn a_failed_capture_returns_to_idle_without_delivering() {
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            capture_fails: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(ctl.transcription_calls(), Vec::<usize>::new());
    assert_eq!(ctl.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_failed_transcription_returns_to_idle_without_output() {
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            transcriber_fails: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness.run(async { record_once(&ctl).await }).await;

    assert_eq!(
        ctl.transcription_calls().len(),
        1,
        "the engine was asked exactly once"
    );
    assert_eq!(ctl.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_recording_that_reaches_its_limit_is_still_transcribed() {
    let harness = TestDaemon::with_config("hello", |config| config.audio.max_duration_secs = 1);
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            // The limit fires on its own; no release is sent.
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "audio captured up to the limit is transcribed, not dropped"
    );
}

#[tokio::test]
async fn a_streaming_session_cancelled_by_the_cli_file_ends_cleanly() {
    let harness = TestDaemon::streaming("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("streaming").await;
            std::fs::write(ctl.paths.cancel(), "cancel").expect("write cancel sentinel");
            ctl.expect_state("idle").await;

            assert_eq!(
                ctl.hook_runs(),
                1,
                "a cancelled streaming session still ends through its hook"
            );
        })
        .await;
}

#[tokio::test]
async fn a_transcription_cancelled_by_the_hotkey_returns_to_idle_without_output() {
    // The engine is slow enough to act inside the transcribing window.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            transcribe_delay_ms: 1_500,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("transcribing").await;

            ctl.cancel();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        Vec::<String>::new(),
        "a cancelled transcription delivers nothing"
    );
}

#[tokio::test]
async fn a_transcription_cancelled_by_the_cli_file_returns_to_idle_without_output() {
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            transcribe_delay_ms: 1_500,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("transcribing").await;

            std::fs::write(ctl.paths.cancel(), "cancel").expect("write cancel sentinel");
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        Vec::<String>::new(),
        "a cancelled transcription delivers nothing"
    );
}

#[tokio::test]
async fn a_recording_cancelled_by_the_cli_file_returns_to_idle_without_output() {
    // The poll arm's cancel path, which is the one the compositor binding uses
    // when it writes the trigger instead of pressing the cancel key.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;

            std::fs::write(ctl.paths.cancel(), "cancel").expect("write cancel sentinel");
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.transcription_calls(),
        Vec::<usize>::new(),
        "a cancelled recording is never transcribed"
    );
}

#[tokio::test]
async fn a_streaming_session_that_ends_delivers_its_final_segment() {
    let harness = TestDaemon::streaming("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert_eq!(ctl.hook_runs(), 1, "the session ends through its hook");
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the committed segment is delivered"
    );
}

#[tokio::test]
async fn a_streaming_backend_that_ends_the_session_itself_tells_the_caller() {
    // The backend can end the session without the daemon stopping it: the
    // remote stream closes, the server finishes, the task dies. The daemon then
    // walks into `end_streaming` with the capture still open and
    // `is_external_trigger` still set, so the caller that started the session
    // is never told it may leave its compositor submap, and the microphone
    // stays open until the next recording overwrites it.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            streaming: true,
            stream_self_ends: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.expect_state("idle").await;

            assert_eq!(
                ctl.hook_runs(),
                1,
                "the caller that started the session has to be told it ended"
            );
            assert_eq!(
                ctl.capture_stops(),
                1,
                "the capture the backend left behind has to be closed"
            );
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the segment the backend committed is still delivered"
    );
}

#[tokio::test]
async fn a_streaming_session_that_fails_returns_to_idle_without_output() {
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            streaming: true,
            stream_error: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        Vec::<String>::new(),
        "a backend failure types nothing"
    );
}

#[tokio::test]
async fn a_file_session_writes_the_transcript_and_types_nothing() {
    // `--file=path` takes its own close path, which accumulates instead of
    // typing and skips the post-output hook: it is a batch dump, not a
    // live-typing operation the hook is meant to wrap.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();
    let transcript = ctl.paths.dir().join("dictation.txt");

    harness
        .run(async {
            std::fs::write(
                ctl.paths.output_mode_override(),
                format!("file:{}", transcript.display()),
            )
            .expect("write output mode override");

            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            // The writer terminates the line, so appended dictations stay
            // separate; assert on the text, not on the newline.
            assert_eq!(
                std::fs::read_to_string(&transcript)
                    .unwrap_or_default()
                    .trim_end(),
                "hello",
                "the transcript is written to the requested file"
            );
        })
        .await;

    assert_eq!(
        ctl.typed(),
        Vec::<String>::new(),
        "a file session never types into the focused window"
    );
}

#[tokio::test]
async fn a_streaming_file_session_writes_its_accumulated_text() {
    let harness = TestDaemon::streaming("hello");
    let ctl = harness.controls();
    let transcript = ctl.paths.dir().join("streamed.txt");

    harness
        .run(async {
            std::fs::write(
                ctl.paths.output_mode_override(),
                format!("file:{}", transcript.display()),
            )
            .expect("write output mode override");
            // A file session reads neither of these, so neither may survive it:
            // the output mode is consumed at the start, the boolean at no point.
            ctl.write_override("auto_submit", "true");

            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert_eq!(
                std::fs::read_to_string(&transcript)
                    .unwrap_or_default()
                    .trim_end(),
                "hello",
                "the accumulated text is written when the session ends"
            );
            assert!(
                !ctl.override_exists("auto_submit"),
                "the file session left the sentinel for the next recording"
            );
        })
        .await;

    assert_eq!(ctl.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_panicking_engine_is_dropped_and_the_next_recording_reloads_it() {
    // #643: a panic inside `spawn_blocking` leaves the engine's internal state
    // unknown, so the daemon drops the cached instance and the next recording
    // builds a clean one. The observable is the factory being asked twice: once
    // for the preloaded engine, once after the poison.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            preload: true,
            panic_once: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            record_once(&ctl).await; // panics; the cycle must still end
            record_once(&ctl).await; // the clean engine has to deliver
        })
        .await;

    std::panic::set_hook(previous_hook);

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "only the recording after the panic delivers"
    );
    assert_eq!(
        ctl.transcriber_factory_calls(),
        2,
        "the poisoned engine must be rebuilt, not reused"
    );
}

#[tokio::test]
async fn a_file_session_that_cannot_write_still_returns_to_idle() {
    // The delivery path's write-failure close: it publishes an error sidecar
    // and returns to idle rather than leaving the daemon in Outputting.
    let harness = TestDaemon::speaking("hello");
    let ctl = harness.controls();
    let unwritable = ctl.paths.dir().join("no-such-dir").join("dictation.txt");

    harness
        .run(async {
            std::fs::write(
                ctl.paths.output_mode_override(),
                format!("file:{}", unwritable.display()),
            )
            .expect("write output mode override");

            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            assert_eq!(
                ctl.transcription_calls().len(),
                1,
                "the recording was transcribed; only the write failed"
            );
        })
        .await;

    assert_eq!(ctl.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_streaming_toggle_session_delivers_its_segment() {
    // The toggle path into streaming, as opposed to the external trigger the
    // other streaming rows use.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            streaming: true,
            ..Fakes::default()
        },
        |config| config.hotkey.mode = ActivationMode::Toggle,
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("streaming").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.press(); // the second press ends it
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the committed segment is delivered"
    );
}

#[tokio::test]
async fn an_eager_recording_is_transcribed_from_its_chunks() {
    // The eager pipeline takes its own route through the stop: it finishes the
    // accumulated chunks and the tail rather than calling the batch path, and
    // it had no row before this one.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            eager: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("idle").await;
        })
        .await;

    assert!(
        !ctl.transcription_calls().is_empty(),
        "the eager path still asks the engine for the tail"
    );
    assert_eq!(ctl.typed(), vec!["hello".to_string()]);
}

#[tokio::test]
async fn an_external_eager_recording_is_transcribed() {
    // The external stop takes the same eager path as a hotkey stop; this pins
    // the shared helper from that side, where the third copy used to live.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            eager: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.external_start();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert_eq!(ctl.hook_runs(), 1, "the session ends through its hook");
        })
        .await;

    assert_eq!(ctl.typed(), vec!["hello".to_string()]);
}

#[tokio::test]
async fn a_meeting_whose_audio_cannot_open_starts_nothing() {
    // `start_meeting` builds the meeting daemon before the capture pair, so a
    // pair that cannot be built leaves a daemon to stop and an error to return.
    // What must not survive is a session: no state file entry, and a daemon that
    // still runs a dictation cycle afterwards.
    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            meeting_capture_unavailable: true,
            ..Fakes::default()
        },
        |config| config.meeting.enabled = true,
    );
    let ctl = harness.controls();

    harness
        .run(async {
            // The file `voxtype meeting start` writes, with the title line it
            // carries.
            std::fs::write(ctl.paths.meeting_start(), "Standup\n").expect("write meeting start");
            tokio::time::sleep(Duration::from_millis(300)).await;

            assert_eq!(
                ctl.meeting_state(),
                None,
                "a meeting that never started must not report a state"
            );

            // The daemon is still a working daemon: the failed start left it idle
            // and ready, which a dictation cycle proves.
            record_once(&ctl).await;
        })
        .await;

    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "the dictation cycle after the failed meeting start still delivers"
    );
}

#[tokio::test]
async fn a_cycle_that_ends_before_transcription_releases_its_model_load() {
    // With on-demand loading the model starts loading when the recording starts,
    // and `get_transcriber_for_recording` takes its result on the way to
    // transcription. A cycle that ends before that - here, released inside the
    // 0.3 s floor - used to leave the finished task parked in its field with the
    // engine inside it, so the whole model stayed resident for the session
    // (B4). Nothing pinned the release, which is what this row is for.
    let harness = TestDaemon::with_fakes("hello", Fakes::default(), |_| {});
    let ctl = harness.controls();

    harness
        .run(async {
            // No preload, so the load task holds the only reference to the
            // engine it builds.
            ctl.press();
            ctl.expect_state("recording").await;
            ctl.release();
            ctl.expect_state("idle").await;

            // The load runs on a blocking thread: give it time to finish, then
            // for its result to drop.
            for _ in 0..50 {
                if ctl.transcriber_drops() > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }

            assert_eq!(
                ctl.transcriber_drops(),
                1,
                "the load this cycle never consumed has to release its engine"
            );
            assert!(
                ctl.transcription_calls().is_empty(),
                "the cycle ended before transcription, so nothing was transcribed"
            );
        })
        .await;
}

#[tokio::test]
async fn a_panic_during_eager_tail_transcription_drops_the_engine() {
    // #643's policy — do not reuse an engine whose task panicked — was applied on
    // the batch path only. The eager tail's panic was logged and the poisoned
    // engine stayed cached, so the next recording reused it.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            preload: true,
            eager: true,
            panic_once: true,
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            record_once(&ctl).await; // the panic happens inside the tail
            record_once(&ctl).await; // a clean engine has to be built for this
        })
        .await;

    std::panic::set_hook(previous_hook);

    assert_eq!(
        ctl.transcriber_factory_calls(),
        2,
        "the engine whose task panicked must be rebuilt, not reused"
    );
    assert_eq!(
        ctl.typed(),
        vec!["hello".to_string()],
        "only the recording after the panic delivers"
    );
}

#[tokio::test]
async fn a_streaming_session_does_not_leave_the_submit_override_behind() {
    // A streaming session delivers its segments as they finalize, so nothing
    // ever reads the one-shot boolean overrides. Without discarding them at the
    // session's close, the `--auto-submit` written for it survives and is
    // applied to the next batch recording — the leak A2 fixed for cancels,
    // still open on the streaming success path.
    let harness = TestDaemon::streaming("hello");
    let ctl = harness.controls();

    harness
        .run(async {
            ctl.write_override("auto_submit", "true");

            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;

            assert!(
                !ctl.override_exists("auto_submit"),
                "the streaming session left the sentinel for the next recording"
            );

            // A second session on the same daemon still works end to end: the
            // discard must not have taken anything the next close needs. (The
            // consequence of the leak - a *batch* delivery applying the stale
            // sentinel - needs a batch-configured daemon, and is pinned by the
            // cancel row.)
            ctl.external_start();
            ctl.expect_state("streaming").await;
            ctl.external_stop();
            ctl.expect_state("idle").await;
        })
        .await;

    assert_eq!(ctl.typed().last(), Some(&"hello".to_string()));
}

#[tokio::test]
async fn a_cycle_that_cannot_get_a_transcriber_still_discards_its_overrides() {
    // The eager stop's failure branch (no transcriber available) ended in a bare
    // idle rather than `reset_to_idle`, so it skipped discarding the one-shot
    // overrides and left a compositor submap entered at pre-recording behind.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let harness = TestDaemon::with_fakes(
        "hello",
        Fakes {
            preload: true,
            eager: true,
            panic_once: true,
            // Second call is the re-create after the panic poisoned the cache.
            factory_fails_from: Some(2),
            ..Fakes::default()
        },
        |_| {},
    );
    let ctl = harness.controls();

    harness
        .run(async {
            // The poisoned engine is dropped, so the next cycle asks again.
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            ctl.write_override("auto_submit", "true");
            ctl.press();
            ctl.expect_state("recording").await;
            tokio::time::sleep(Duration::from_millis(FLOOR_MS)).await;
            ctl.release();
            ctl.expect_state("idle").await;

            assert!(
                !ctl.override_exists("auto_submit"),
                "the failed cycle left its override behind"
            );
        })
        .await;

    std::panic::set_hook(previous_hook);
}
