//! Everything the daemon takes from outside itself.
//!
//! The audio device, the transcription engine, the output drivers and the
//! hotkey events all arrive from outside this process, and the loop stops when
//! the process is signalled. Production builds each of those from the
//! configuration. A test substitutes its own, which is what makes a daemon cycle
//! reachable without a microphone, a display, a model file or a signal to the
//! test process.
//!
//! Every field is an override: `None` means "the real thing", so
//! [`Deps::production`] is the honest default and a test replaces exactly what
//! it needs.

use crate::audio::AudioCapture;
use crate::config::{AudioConfig, Config, OutputConfig};
use crate::error::{AudioError, TranscribeError};
use crate::output::TextOutput;
use crate::transcribe::Transcriber;
use std::sync::Arc;
use tokio::sync::oneshot;

#[cfg(target_os = "linux")]
use crate::hotkey::HotkeyEvent;
#[cfg(target_os = "macos")]
use crate::hotkey_macos::HotkeyEvent;

/// Builds the audio capture for one recording.
pub type CaptureFactory =
    Arc<dyn Fn(&AudioConfig) -> Result<Box<dyn AudioCapture>, AudioError> + Send + Sync>;

/// Builds a transcriber, which may mean loading a model.
pub type TranscriberFactory =
    Arc<dyn Fn(&Config) -> Result<Box<dyn Transcriber>, TranscribeError> + Send + Sync>;

/// Builds the output chain one transcription is delivered through.
pub type OutputChainFactory = Arc<dyn Fn(&OutputConfig) -> Vec<Box<dyn TextOutput>> + Send + Sync>;

/// The four things the daemon asks the outside world to build.
#[derive(Clone, Default)]
pub struct Factories {
    pub capture: Option<CaptureFactory>,
    pub transcriber: Option<TranscriberFactory>,
    pub output_chain: Option<OutputChainFactory>,
}

impl Factories {
    pub fn create_capture(
        &self,
        config: &AudioConfig,
    ) -> Result<Box<dyn AudioCapture>, AudioError> {
        match &self.capture {
            Some(build) => build(config),
            None => crate::audio::create_capture(config),
        }
    }

    pub fn create_transcriber(
        &self,
        config: &Config,
    ) -> Result<Box<dyn Transcriber>, TranscribeError> {
        match &self.transcriber {
            Some(build) => build(config),
            None => crate::transcribe::create_transcriber(config),
        }
    }

    pub fn create_output_chain(&self, config: &OutputConfig) -> Vec<Box<dyn TextOutput>> {
        match &self.output_chain {
            Some(build) => build(config),
            None => crate::output::create_output_chain(config),
        }
    }
}

/// The daemon's outside world.
pub struct Deps {
    pub factories: Factories,

    /// Hotkey events to consume in place of a listener. Production leaves this
    /// `None`: the listener is built from the configuration and opens the input
    /// device. A test keeps the sender and drives the loop directly, which is
    /// the only way to reach the state machine without an input device and
    /// without touching the user's keyboard.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub hotkey_events: Option<tokio::sync::mpsc::Receiver<HotkeyEvent>>,

    /// Stops the loop the way SIGINT and SIGTERM do. Production leaves this
    /// `None`: being signalled is how a daemon is asked to stop.
    pub shutdown: Option<oneshot::Receiver<()>>,

    /// Whether to end the process after the shutdown tail. Production sets it:
    /// the daemon owns its process and `_exit` skips destructors that can
    /// block. A test clears it, because `_exit(0)` from a test would take the
    /// whole test binary with it.
    pub exit_process_on_shutdown: bool,
}

impl Deps {
    pub fn production() -> Self {
        Self {
            factories: Factories::default(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            hotkey_events: None,
            shutdown: None,
            exit_process_on_shutdown: true,
        }
    }

    pub fn create_capture(
        &self,
        config: &AudioConfig,
    ) -> Result<Box<dyn AudioCapture>, AudioError> {
        self.factories.create_capture(config)
    }

    pub fn create_transcriber(
        &self,
        config: &Config,
    ) -> Result<Box<dyn Transcriber>, TranscribeError> {
        self.factories.create_transcriber(config)
    }

    pub fn create_output_chain(&self, config: &OutputConfig) -> Vec<Box<dyn TextOutput>> {
        self.factories.create_output_chain(config)
    }
}
