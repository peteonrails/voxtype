//! The user's media, suppressed while a recording is in flight.
//!
//! Holding the microphone open next to playing audio leaks playback into the
//! start of a recording, so the daemon asks MPRIS players to pause and lowers
//! the volume of the streams it can find. Which players were paused, which
//! streams were ducked, and the fade still in progress are all part of the same
//! session, so they live here rather than as three more fields on `Daemon`.

use crate::audio::media::{self, DuckedMediaStream};
use crate::config::AudioConfig;
use tokio::task::JoinHandle;

/// What the daemon paused, ducked or is still fading for the recording in
/// flight.
#[derive(Default)]
pub struct MediaSession {
    /// MPRIS players the daemon paused, to be resumed when recording ends.
    paused_players: Vec<String>,
    /// Streams whose volume the daemon lowered, with their original volumes.
    ducked_streams: Vec<DuckedMediaStream>,
    /// The fade in flight, kept so a stop can abort it rather than join it.
    fade: Option<JoinHandle<()>>,
}

impl MediaSession {
    /// Suppress media before the microphone opens, so playback cannot leak into
    /// the beginning of a recording.
    pub async fn suppress(&mut self, config: &AudioConfig) {
        self.pause_players(config).await;
        self.duck_streams(config).await;
    }

    /// Restore media as soon as capture has stopped. Transcription and text
    /// output may continue after this point without keeping playback paused or
    /// ducked.
    pub fn restore(&mut self, config: &AudioConfig) {
        self.restore_ducked(config);
        self.resume_players();
    }

    /// Pause MPRIS players if configured, remembering which ones were paused.
    async fn pause_players(&mut self, config: &AudioConfig) {
        if config.pause_media {
            self.paused_players =
                media::pause_playing_players(&config.pause_media_ignored_players).await;
        }
    }

    /// Duck active audio streams if configured, remembering their volumes.
    async fn duck_streams(&mut self, config: &AudioConfig) {
        if config.duck_media {
            // Wait out any restore still fading up. Its final write is what puts
            // the streams back at their true original volumes, and enumerating
            // before that lands would capture intermediate values as the new
            // originals: every fast toggle cycle would then store a quieter
            // baseline and media would drift down permanently. Normally already
            // finished, so this costs nothing.
            if let Some(task) = self.fade.take() {
                let _ = task.await;
            }
            let (streams, fade) = media::duck_playing_audio(
                config.duck_media_volume_percent,
                config.duck_media_fade_ms,
            )
            .await;
            self.ducked_streams = streams;
            self.fade = fade;
        }
    }

    /// Restore the streams ducked at recording start.
    fn restore_ducked(&mut self, config: &AudioConfig) {
        if !self.ducked_streams.is_empty() {
            // Abort rather than await a fade still on its way down: this path is
            // synchronous, and the restore we are about to spawn ends by writing
            // the stored originals, so an interrupted duck ramp is corrected
            // either way.
            if let Some(task) = self.fade.take() {
                task.abort();
            }
            let streams = std::mem::take(&mut self.ducked_streams);
            self.fade = Some(tokio::spawn(media::restore_ducked_audio(
                streams,
                config.duck_media_volume_percent,
                config.duck_media_fade_ms,
            )));
        }
    }

    /// Resume the MPRIS players paused at recording start.
    fn resume_players(&mut self) {
        if !self.paused_players.is_empty() {
            let players = std::mem::take(&mut self.paused_players);
            tokio::spawn(media::resume_players(players));
        }
    }
}
