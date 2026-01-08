//! Error types for voxtype
//!
//! Uses thiserror for ergonomic error definitions with clear messages
//! that guide users toward fixing common issues.

use thiserror::Error;

/// Top-level error type for the voxtype application
#[derive(Error, Debug)]
pub enum VoxtypeError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Hotkey error: {0}")]
    Hotkey(#[from] HotkeyError),

    #[error("Audio capture error: {0}")]
    Audio(#[from] AudioError),

    #[error("Transcription error: {0}")]
    Transcribe(#[from] TranscribeError),

    #[error("Output error: {0}")]
    Output(#[from] OutputError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors related to hotkey detection
#[derive(Error, Debug)]
pub enum HotkeyError {
    #[error("Cannot open input device '{0}'. Is the user in the 'input' group?\n  Run: sudo usermod -aG input $USER\n  Then log out and back in.")]
    DeviceAccess(String),

    #[error("Unknown key name: '{0}'. Use evtest or wev to find valid key names.")]
    UnknownKey(String),

    #[error("No keyboard device found in /dev/input/")]
    NoKeyboard,

    #[error("evdev error: {0}")]
    Evdev(String),
}

/// Errors related to audio capture
#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio connection failed: {0}")]
    Connection(String),

    #[error("Audio device not found: '{0}'. List devices with: pactl list sources short")]
    DeviceNotFound(String),

    #[error("Recording timeout: exceeded {0} seconds")]
    Timeout(u32),

    #[error("No audio was captured. Check your microphone.")]
    EmptyRecording,

    #[error("Audio stream error: {0}")]
    StreamError(String),
}

/// Errors related to speech-to-text transcription
#[derive(Error, Debug)]
pub enum TranscribeError {
    #[error("Model not found: {0}\n  Run 'voxtype setup' to download models.")]
    ModelNotFound(String),

    #[error("Whisper initialization failed: {0}")]
    InitFailed(String),

    #[error("Transcription failed: {0}")]
    InferenceFailed(String),

    #[error("Audio format error: {0}")]
    AudioFormat(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Remote server error: {0}")]
    RemoteError(String),
}

/// Errors related to text output
#[derive(Error, Debug)]
pub enum OutputError {
    #[error("ydotool daemon not running.\n  Start with: systemctl --user start ydotool\n  Enable at boot: systemctl --user enable ydotool")]
    YdotoolNotRunning,

    #[error("ydotool not found in PATH. Install via your package manager.")]
    YdotoolNotFound,

    #[error("wtype not found in PATH. Install via your package manager.")]
    WtypeNotFound,

    #[error("wl-copy not found in PATH. Install wl-clipboard via your package manager.")]
    WlCopyNotFound,

    #[error("Text injection failed: {0}")]
    InjectionFailed(String),

    #[error("Ctrl+V simulation failed: {0}")]
    CtrlVFailed(String),

    #[error("All output methods failed. Ensure wtype, ydotool, or wl-copy is available.")]
    AllMethodsFailed,
}

/// Result type alias using VoxtypeError
pub type Result<T> = std::result::Result<T, VoxtypeError>;

impl From<evdev::Error> for HotkeyError {
    fn from(e: evdev::Error) -> Self {
        HotkeyError::Evdev(e.to_string())
    }
}
