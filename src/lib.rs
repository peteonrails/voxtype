//! Voxtype: Push-to-talk voice-to-text for Linux
//!
//! This library provides the core functionality for:
//! - Detecting hotkey presses via evdev (kernel-level, works on all compositors)
//! - Capturing audio via cpal (supports PipeWire, PulseAudio, ALSA)
//! - Transcribing speech using whisper.cpp (fast, local, offline)
//! - Processing text (punctuation, replacements, optional LLM post-processing)
//! - Outputting text via wtype/dotool/ydotool/clipboard fallback chain
//!
//! # Architecture
//!
//! ```text
//!                            ┌─────────────────────────────────────┐
//!                            │              Daemon                 │
//!                            └─────────────────────────────────────┘
//!                                            │
//!                   ┌────────────────────────┼────────────────────────┐
//!                   │                        │                        │
//!                   ▼                        ▼                        ▼
//!          ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
//!          │    Hotkey    │         │    Audio     │         │    State     │
//!          │   (evdev)    │         │    (cpal)    │         │   Manager    │
//!          └──────────────┘         └──────────────┘         └──────────────┘
//!                   │                        │
//!                   │  key press             │ audio samples
//!                   │  key release           │
//!                   ▼                        ▼
//!          ┌─────────────────────────────────────────────────────────────────┐
//!          │                        Recording Flow                           │
//!          │  [Press] ──▶ Start Recording ──▶ [Release] ──▶ Stop & Process   │
//!          └─────────────────────────────────────────────────────────────────┘
//!                                            │
//!                                            ▼
//!                                   ┌──────────────┐
//!                                   │   Whisper    │
//!                                   │ (whisper-rs) │
//!                                   └──────────────┘
//!                                            │
//!                                            ▼ raw text
//!                                   ┌──────────────┐
//!                                   │     Text     │
//!                                   │  Processing  │
//!                                   └──────────────┘
//!                                            │
//!                                            ▼ processed text
//!                                   ┌──────────────┐
//!                                   │ Post-Process │ (optional: LLM cleanup)
//!                                   │   Command    │
//!                                   └──────────────┘
//!                                            │
//!                                            ▼ final text
//!                                   ┌──────────────┐
//!                                   │  Pre-Output  │ (optional: compositor hook)
//!                                   │    Hook      │
//!                                   └──────────────┘
//!                                            │
//!                                            ▼
//!                                   ┌──────────────┐
//!                                   │    Output    │
//!                                   │wtype/dotool/ │
//!                                   │ydotool/clip  │
//!                                   └──────────────┘
//!                                            │
//!                                            ▼
//!                                   ┌──────────────┐
//!                                   │ Post-Output  │ (optional: compositor hook)
//!                                   │    Hook      │
//!                                   └──────────────┘
//! ```

pub mod audio;
pub mod cli;
pub mod config;
pub mod cpu;
pub mod daemon;
pub mod error;
pub mod hotkey;
pub mod model_manager;
pub mod output;
pub mod setup;
pub mod state;
pub mod text;
pub mod transcribe;

pub use cli::{Cli, Commands, CompositorType, OutputModeOverride, RecordAction, SetupAction};
pub use config::Config;
pub use daemon::Daemon;
pub use error::{Result, VoxtypeError};
