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
use crate::audio::AudioCapture;
use crate::config::{ActivationMode, Config, OutputConfig, Profile};
use crate::error::{AudioError, OutputError, TranscribeError};
use crate::output::TextOutput;
use crate::runtime_files::RuntimePaths;
use crate::transcribe::{StreamHandle, StreamingEvent, StreamingTranscriber, Transcriber};
use std::future::Future;
use std::path::PathBuf;
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

/// Audio capture that hands the daemon a fixed buffer.
struct FakeCapture {
    samples: Vec<f32>,
    /// Kept alive so the frame tap's channel does not close while recording.
    feed: Option<mpsc::Sender<Vec<f32>>>,
}

impl FakeCapture {
    fn new(seconds: f32) -> Self {
        let count = (seconds * 16_000.0) as usize;
        // A tone rather than silence: the level tap and any speech gate see
        // something shaped like audio, and the daemon's silence paths stay off.
        let samples = (0..count)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();
        Self {
            samples,
            feed: None,
        }
    }
}

#[async_trait::async_trait]
impl AudioCapture for FakeCapture {
    async fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, AudioError> {
        let (tx, rx) = mpsc::channel(8);
        let _ = tx.send(self.samples.clone()).await;
        self.feed = Some(tx);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<Vec<f32>, AudioError> {
        self.feed = None;
        Ok(self.samples.clone())
    }

    async fn get_samples(&mut self) -> Vec<f32> {
        self.samples.clone()
    }
}

/// Transcriber that returns scripted text and records the audio it was handed.
struct FakeTranscriber {
    text: String,
    calls: Arc<Mutex<Vec<usize>>>,
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.calls.lock().expect("calls lock").push(samples.len());
        Ok(self.text.clone())
    }
}

/// A transcriber that can stream, so the streaming pipeline is reachable.
///
/// It emits one `Final` when the daemon drops the samples channel (end of
/// input) and `Ended` after it, which is the shape a real backend produces.
struct FakeStreamingTranscriber {
    text: String,
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

        let task = tokio::spawn(async move {
            // One segment is enough: the daemon types finals, and the test is
            // about the session's lifecycle rather than the text.
            while samples_rx.recv().await.is_some() {}
            if !text.is_empty() {
                let _ = events_tx
                    .send(StreamingEvent::Final {
                        text,
                        segment_id: 1,
                    })
                    .await;
            }
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
    hotkey_tx: mpsc::Sender<HotkeyEvent>,
    external_start_tx: mpsc::Sender<()>,
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
        Self::build(text, true, |_| {})
    }

    /// As [`TestDaemon::speaking`], with a chance to adjust the configuration
    /// before the daemon is built.
    pub fn with_config(text: &str, tweak: impl FnOnce(&mut Config)) -> Self {
        Self::build(text, false, tweak)
    }

    fn build(text: &str, streaming: bool, tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let state_file = dir.path().join("state");
        let hook_log = dir.path().join("hook-runs");
        let mut config = base_config(streaming);
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
        // The external stop hook is a shell command, so a test observes it by
        // having it append one byte per run.
        config.audio.external_trigger_stop_command =
            Some(format!("printf x >> '{}'", hook_log.display()));
        tweak(&mut config);

        let (hotkey_tx, hotkey_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (external_start_tx, external_start_rx) = mpsc::channel(4);
        let typed = Arc::new(Mutex::new(Vec::new()));
        let transcribed = Arc::new(Mutex::new(Vec::new()));
        let output_configs = Arc::new(Mutex::new(Vec::new()));

        let output = FakeOutput {
            typed: typed.clone(),
        };
        let calls = transcribed.clone();
        let configs = output_configs.clone();
        let scripted = text.to_string();
        let factories = Factories {
            capture: Some(Arc::new(|_config| {
                Ok(Box::new(FakeCapture::new(1.5)) as Box<dyn AudioCapture>)
            })),
            transcriber: Some(Arc::new(move |_config| {
                if streaming {
                    Ok(Box::new(FakeStreamingTranscriber {
                        text: scripted.clone(),
                        calls: calls.clone(),
                    }) as Box<dyn Transcriber>)
                } else {
                    Ok(Box::new(FakeTranscriber {
                        text: scripted.clone(),
                        calls: calls.clone(),
                    }) as Box<dyn Transcriber>)
                }
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
            exit_process_on_shutdown: false,
        };

        let paths = RuntimePaths::new(dir.path());
        let daemon = Daemon::with_deps(config, None, paths.clone(), deps);
        let controls = Controls {
            typed,
            transcribed,
            output_configs,
            hotkey_tx,
            external_start_tx,
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
/// reads `transcriber_preloaded`, which only the non-on-demand path fills.
const STREAMING_CONFIG: &str = r#"
engine = "sensevoice"

[sensevoice]
model = "sensevoice-small"
on_demand_loading = false
"#;

fn base_config(streaming: bool) -> Config {
    let text = if streaming {
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
    assert!(
        (samples as f32 / 16_000.0 - 1.5).abs() < 0.05,
        "the transcribe call got {samples} samples, not the scripted recording"
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
