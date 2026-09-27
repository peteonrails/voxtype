//! The engine instance for the next recording, and the task loading it.
//!
//! Three fields on `Daemon` used to hold this between them: the background load
//! a recording starts when on-demand loading is on, the instance cloned out of
//! it for the in-flight transcription, and the instance loaded at startup when
//! on-demand loading is off. Their agreement was a comment, and one of the paths
//! that broke it kept a whole model resident: a cycle that ended before
//! transcription left the finished load task parked in its field, still holding
//! the `Arc` (B4). Moving between the states is a method here now, so there is
//! one place that releases a load, one that consumes it, and one that discards a
//! poisoned instance.

use crate::error::TranscribeError;
use crate::transcribe::Transcriber;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// What a background load hands back when it finishes.
pub type LoadResult = std::result::Result<Arc<dyn Transcriber>, TranscribeError>;

/// The engine for the next recording, plus the load in flight for it.
#[derive(Default)]
pub struct ModelSlot {
    /// The background load, present between a recording's start and the moment
    /// transcription takes its result.
    load: Option<JoinHandle<LoadResult>>,
    /// The instance cloned for the in-flight transcription. The result handler
    /// reads language metadata off it after the task completes, which is why it
    /// outlives the task.
    active: Option<Arc<dyn Transcriber>>,
    /// The instance loaded at startup when on-demand loading is off. Whisper
    /// draws its transcriber from the model manager instead; every other engine
    /// clones from here.
    preloaded: Option<Arc<dyn Transcriber>>,
}

impl ModelSlot {
    /// Start a background load for the recording about to begin.
    pub fn begin_load(&mut self, task: JoinHandle<LoadResult>) {
        self.load = Some(task);
    }

    /// Take the load so its result can be awaited. Taking it is what makes the
    /// result this cycle's, rather than something to release.
    pub fn take_load(&mut self) -> Option<JoinHandle<LoadResult>> {
        self.load.take()
    }

    /// Drop a load this cycle never consumed.
    ///
    /// This is the release B4 was about: dropping the handle lets the task's
    /// result drop when the task finishes, and with it the `Arc` holding the
    /// model. Without it, a recording that ended before transcription (too
    /// short, no speech, capture failure) left hundreds of MiB resident for the
    /// rest of the session.
    pub fn release_unconsumed_load(&mut self) {
        self.load = None;
    }

    /// The instance loaded at startup, if there is one.
    pub fn preloaded(&self) -> Option<Arc<dyn Transcriber>> {
        self.preloaded.clone()
    }

    /// Adopt an instance loaded at startup.
    pub fn set_preloaded(&mut self, transcriber: Arc<dyn Transcriber>) {
        self.preloaded = Some(transcriber);
    }

    /// Discard the preloaded instance, for the panic recovery: an engine whose
    /// task panicked is not reused, so the next recording builds a clean one.
    /// Returns whether there was one to discard.
    pub fn discard_preloaded(&mut self) -> bool {
        self.preloaded.take().is_some()
    }

    /// Keep the instance the in-flight transcription is using.
    pub fn set_active(&mut self, transcriber: Arc<dyn Transcriber>) {
        self.active = Some(transcriber);
    }

    /// Take the instance for the transcription this cycle is about to start.
    pub fn take_active(&mut self) -> Option<Arc<dyn Transcriber>> {
        self.active.take()
    }

    /// Drop the instance once its transcription is done with it.
    pub fn clear_active(&mut self) {
        self.active = None;
    }
}
