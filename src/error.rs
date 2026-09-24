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

    #[error("Meeting error: {0}")]
    Meeting(#[from] MeetingError),

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

    #[cfg(target_os = "linux")]
    #[error("XDG Desktop Portal is unavailable: {0}\n  Portal hotkeys need xdg-desktop-portal 1.20 or later running, or set [hotkey] backend = \"evdev\".\n  Run: voxtype setup check")]
    PortalUnavailable(#[source] zbus::Error),

    #[cfg(target_os = "linux")]
    #[error("XDG Desktop Portal could not register Voxtype: {0}\n  The desktop needs io.voxtype.Voxtype.desktop in an XDG applications directory.\n  Run: voxtype setup check")]
    PortalRegistration(#[source] zbus::Error),

    #[cfg(target_os = "linux")]
    #[error("XDG GlobalShortcuts could not bind Voxtype's shortcuts: {0}\n  Check the desktop's global shortcut settings for Voxtype, or set [hotkey] backend = \"evdev\".")]
    PortalBinding(#[source] zbus::Error),

    #[cfg(target_os = "linux")]
    #[error("XDG GlobalShortcuts returned an invalid response: {0}\n  This desktop's portal backend may not implement GlobalShortcuts.\n  Set [hotkey] backend = \"evdev\" to read /dev/input instead.")]
    PortalProtocol(String),

    #[cfg(target_os = "linux")]
    #[error("Global shortcut registration was cancelled\n  Accept the desktop's shortcut dialog so Voxtype can bind its shortcuts, or set [hotkey] backend = \"evdev\".")]
    PortalCancelled,

    #[cfg(target_os = "linux")]
    #[error("Global shortcut registration failed with response code {0}\n  Check the desktop's global shortcut settings for Voxtype, then restart the daemon.")]
    PortalResponse(u32),

    #[cfg(target_os = "linux")]
    #[error("The desktop did not bind the required '{0}' shortcut\n  Assign it in the desktop's global shortcut settings, or set [hotkey] backend = \"evdev\".")]
    PortalMissingRequired(String),
}

#[cfg(target_os = "linux")]
impl HotkeyError {
    pub(crate) fn allows_evdev_fallback(&self) -> bool {
        matches!(
            self,
            Self::PortalUnavailable(_) | Self::PortalRegistration(_)
        )
    }

    pub(crate) fn allows_portal_retry(&self) -> bool {
        matches!(
            self,
            Self::PortalUnavailable(_) | Self::PortalRegistration(_) | Self::PortalBinding(_)
        )
    }
}

/// Errors related to audio capture
#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio connection failed: {0}")]
    Connection(String),

    #[error("Audio device not found: '{0}'. List devices with: pactl list sources short")]
    DeviceNotFound(String),

    #[error("Audio device not found: '{requested}'.\n{available}")]
    DeviceNotFoundWithList {
        requested: String,
        available: String,
    },

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

    #[error("Transcriber initialization failed: {0}")]
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

    #[error("{0}")]
    LicenseRequired(String),
}

/// Errors related to Voice Activity Detection
#[derive(Error, Debug)]
pub enum VadError {
    #[error("VAD model not found: {0}\n  Run 'voxtype setup vad' to download.")]
    ModelNotFound(String),

    #[error("VAD initialization failed: {0}")]
    InitFailed(String),

    #[error("VAD detection failed: {0}")]
    DetectionFailed(String),
}

/// Errors related to text output
#[derive(Error, Debug)]
pub enum OutputError {
    #[error("ydotool daemon not running.\n  Start with: systemctl --user start ydotool\n  Enable at boot: systemctl --user enable ydotool")]
    YdotoolNotRunning,

    #[error("ydotool not found in PATH. Install via your package manager.")]
    YdotoolNotFound,

    #[error("dotool not found in PATH. Install from https://sr.ht/~geb/dotool/")]
    DotoolNotFound,

    #[error("wtype not found in PATH. Install via your package manager.")]
    WtypeNotFound,

    #[error("eitype not found in PATH. Install via: cargo install eitype")]
    EitypeNotFound,

    #[error("wl-copy not found in PATH. Install wl-clipboard via your package manager.")]
    WlCopyNotFound,

    #[error("wl-paste not found in PATH. Install wl-clipboard via your package manager.")]
    WlPasteNotFound,

    #[error("xclip not found in PATH. Install xclip via your package manager.")]
    XclipNotFound,

    #[error(
        "Neither xclip nor xsel is available for X11 clipboard access.\n  \
         Install one via your package manager:\n    \
         sudo pacman -S xclip   # Arch / Manjaro\n    \
         sudo apt install xclip # Debian / Ubuntu\n    \
         sudo dnf install xclip # Fedora"
    )]
    X11ClipboardToolMissing,

    #[error("Text injection failed: {0}")]
    InjectionFailed(String),

    #[error("Ctrl+V simulation failed: {0}")]
    CtrlVFailed(String),

    #[error(
        "All output methods failed. Ensure wtype, dotool, ydotool, wl-copy, or xclip is available."
    )]
    AllMethodsFailed,
}

/// Errors related to meeting transcription
#[derive(Error, Debug)]
pub enum MeetingError {
    #[error("Meeting already in progress")]
    AlreadyInProgress,

    #[error("No meeting in progress")]
    NotInProgress,

    #[error("No active meeting to pause")]
    NotActive,

    #[error("No paused meeting to resume")]
    NotPaused,

    #[error("Transcriber not initialized")]
    TranscriberNotInitialized,

    #[error("Meeting storage error: {0}")]
    Storage(String),
}

/// Result type alias using VoxtypeError
pub type Result<T> = std::result::Result<T, VoxtypeError>;
