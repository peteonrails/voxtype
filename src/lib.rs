//! Voxtype: Push-to-talk voice-to-text for Wayland
//!
//! This library provides the core functionality for:
//! - Detecting hotkey presses via evdev (kernel-level, works on all compositors)
//! - Capturing audio via cpal (supports PipeWire, PulseAudio, ALSA)
//! - Transcribing speech using whisper.cpp (fast, local, offline)
//! - Outputting text via ydotool or clipboard (fallback chain)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                         Daemon                              │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
//! │  │   Hotkey     │  │    Audio     │  │   Text Output    │  │
//! │  │  (evdev)     │──│   (cpal)     │──│  (ydotool/clip)  │  │
//! │  └──────────────┘  └──────────────┘  └──────────────────┘  │
//! │         │               │                    │              │
//! │         │               ▼                    │              │
//! │         │        ┌──────────────┐            │              │
//! │         │        │   Whisper    │            │              │
//! │         └───────▶│  (whisper-rs)│────────────┘              │
//! │                  └──────────────┘                           │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod audio;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod hotkey;
pub mod output;
pub mod setup;
pub mod state;
pub mod text;
pub mod transcribe;

pub use cli::{Cli, Commands, RecordAction, SetupAction};
pub use config::Config;
pub use daemon::Daemon;
pub use error::{Result, VoxtypeError};
