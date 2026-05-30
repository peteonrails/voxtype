//! Record-command actions and the output-mode override that goes with them.

use clap::Subcommand;

/// Output mode override for record commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputModeOverride {
    Type,
    Clipboard,
    Paste,
    File,
}

#[derive(Subcommand)]
pub enum RecordAction {
    /// Start recording (send SIGUSR1 to daemon)
    Start {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,

        /// Write transcription to a file
        /// Use --file alone to use file_path from config, or --file=path.txt for explicit path
        #[arg(long, value_name = "FILE", group = "output_mode", num_args = 0..=1, default_missing_value = "")]
        file: Option<String>,

        /// Use a specific model for this transcription (e.g., large-v3-turbo)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Use a named profile for post-processing (e.g., --profile slack)
        /// Profiles are defined in config.toml under [profiles.name]
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,

        /// Auto-submit (press Enter) after this transcription
        #[arg(long)]
        auto_submit: bool,

        /// Disable auto-submit for this transcription (overrides config)
        #[arg(long, conflicts_with = "auto_submit")]
        no_auto_submit: bool,

        /// Use Shift+Enter for newlines in this transcription
        #[arg(long)]
        shift_enter_newlines: bool,

        /// Disable Shift+Enter newlines for this transcription (overrides config)
        #[arg(long, conflicts_with = "shift_enter_newlines")]
        no_shift_enter_newlines: bool,

        /// Enable smart auto-submit for this recording (say "submit" to press Enter)
        #[arg(long, conflicts_with = "no_smart_auto_submit")]
        smart_auto_submit: bool,

        /// Disable smart auto-submit for this recording
        #[arg(long, conflicts_with = "smart_auto_submit")]
        no_smart_auto_submit: bool,
    },
    /// Stop recording and transcribe (send SIGUSR2 to daemon)
    Stop {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,
    },
    /// Toggle recording state
    Toggle {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,

        /// Write transcription to a file
        /// Use --file alone to use file_path from config, or --file=path.txt for explicit path
        #[arg(long, value_name = "FILE", group = "output_mode", num_args = 0..=1, default_missing_value = "")]
        file: Option<String>,

        /// Use a specific model for this transcription (e.g., large-v3-turbo)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Use a named profile for post-processing (e.g., --profile slack)
        /// Profiles are defined in config.toml under [profiles.name]
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,

        /// Auto-submit (press Enter) after this transcription
        #[arg(long)]
        auto_submit: bool,

        /// Disable auto-submit for this transcription (overrides config)
        #[arg(long, conflicts_with = "auto_submit")]
        no_auto_submit: bool,

        /// Use Shift+Enter for newlines in this transcription
        #[arg(long)]
        shift_enter_newlines: bool,

        /// Disable Shift+Enter newlines for this transcription (overrides config)
        #[arg(long, conflicts_with = "shift_enter_newlines")]
        no_shift_enter_newlines: bool,

        /// Enable smart auto-submit for this recording (say "submit" to press Enter)
        #[arg(long, conflicts_with = "no_smart_auto_submit")]
        smart_auto_submit: bool,

        /// Disable smart auto-submit for this recording (overrides config)
        #[arg(long, conflicts_with = "smart_auto_submit")]
        no_smart_auto_submit: bool,
    },
    /// Cancel current recording or transcription (discard without output)
    Cancel,
}

impl RecordAction {
    /// Extract the output mode override from the action flags
    /// Returns (mode_override, optional_file_path)
    pub fn output_mode_override(&self) -> Option<OutputModeOverride> {
        let (type_mode, clipboard, paste, file) = match self {
            RecordAction::Start {
                type_mode,
                clipboard,
                paste,
                file,
                ..
            } => (*type_mode, *clipboard, *paste, file.as_ref()),
            RecordAction::Stop {
                type_mode,
                clipboard,
                paste,
            } => (*type_mode, *clipboard, *paste, None),
            RecordAction::Toggle {
                type_mode,
                clipboard,
                paste,
                file,
                ..
            } => (*type_mode, *clipboard, *paste, file.as_ref()),
            RecordAction::Cancel => return None,
        };

        if type_mode {
            Some(OutputModeOverride::Type)
        } else if clipboard {
            Some(OutputModeOverride::Clipboard)
        } else if paste {
            Some(OutputModeOverride::Paste)
        } else if file.is_some() {
            Some(OutputModeOverride::File)
        } else {
            None
        }
    }

    /// Get the file path for --file flag (if specified with explicit path)
    /// Returns Some("") if --file was used without a path (use config's file_path)
    /// Returns Some(path) if --file=path was used
    /// Returns None if --file was not used
    pub fn file_path(&self) -> Option<&str> {
        match self {
            RecordAction::Start { file, .. } | RecordAction::Toggle { file, .. } => file.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Extract the model override from the action flags
    /// Note: --model is only available on start/toggle, not stop (model is selected at recording start)
    pub fn model_override(&self) -> Option<&str> {
        match self {
            RecordAction::Start { model, .. } => model.as_deref(),
            RecordAction::Toggle { model, .. } => model.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Get the profile name from --profile flag
    /// Returns the profile name if specified on start or toggle commands
    pub fn profile(&self) -> Option<&str> {
        match self {
            RecordAction::Start { profile, .. } => profile.as_deref(),
            RecordAction::Toggle { profile, .. } => profile.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Get the auto_submit override from --auto-submit / --no-auto-submit flags
    /// Returns Some(true) for --auto-submit, Some(false) for --no-auto-submit, None if unset
    pub fn auto_submit_override(&self) -> Option<bool> {
        let (auto_submit, no_auto_submit) = match self {
            RecordAction::Start {
                auto_submit,
                no_auto_submit,
                ..
            } => (*auto_submit, *no_auto_submit),
            RecordAction::Toggle {
                auto_submit,
                no_auto_submit,
                ..
            } => (*auto_submit, *no_auto_submit),
            RecordAction::Stop { .. } | RecordAction::Cancel => return None,
        };

        if auto_submit {
            Some(true)
        } else if no_auto_submit {
            Some(false)
        } else {
            None
        }
    }

    /// Get the shift_enter_newlines override from --shift-enter-newlines / --no-shift-enter-newlines flags
    /// Returns Some(true) to enable, Some(false) to disable, None if unset
    pub fn shift_enter_newlines_override(&self) -> Option<bool> {
        let (shift_enter, no_shift_enter) = match self {
            RecordAction::Start {
                shift_enter_newlines,
                no_shift_enter_newlines,
                ..
            } => (*shift_enter_newlines, *no_shift_enter_newlines),
            RecordAction::Toggle {
                shift_enter_newlines,
                no_shift_enter_newlines,
                ..
            } => (*shift_enter_newlines, *no_shift_enter_newlines),
            RecordAction::Stop { .. } | RecordAction::Cancel => return None,
        };

        if shift_enter {
            Some(true)
        } else if no_shift_enter {
            Some(false)
        } else {
            None
        }
    }

    /// Get the smart auto-submit override from --smart-auto-submit / --no-smart-auto-submit flags
    /// Returns Some(true) to enable, Some(false) to disable, None if not specified
    pub fn smart_auto_submit_override(&self) -> Option<bool> {
        let (enable, disable) = match self {
            RecordAction::Start {
                smart_auto_submit,
                no_smart_auto_submit,
                ..
            } => (*smart_auto_submit, *no_smart_auto_submit),
            RecordAction::Toggle {
                smart_auto_submit,
                no_smart_auto_submit,
                ..
            } => (*smart_auto_submit, *no_smart_auto_submit),
            RecordAction::Stop { .. } | RecordAction::Cancel => return None,
        };

        if enable {
            Some(true)
        } else if disable {
            Some(false)
        } else {
            None
        }
    }
}
