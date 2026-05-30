//! Meeting-mode subcommand actions.

use clap::Subcommand;

/// Meeting mode actions
#[derive(Subcommand)]
pub enum MeetingAction {
    /// Start a new meeting transcription
    Start {
        /// Meeting title (optional)
        #[arg(long, short)]
        title: Option<String>,

        /// Diarization backend override for this meeting only.
        ///
        /// `simple` attributes by audio source (You vs Remote) — best for 1:1 calls.
        /// `ml` uses ONNX speaker embeddings for multi-speaker meetings (requires
        /// the `ml-diarization` feature and the ECAPA-TDNN model).
        ///
        /// When omitted, falls back to `[meeting.diarization].backend` in config.
        #[arg(long, value_parser = ["simple", "ml"], env = "VOXTYPE_MEETING_DIARIZATION")]
        diarization: Option<String>,
    },
    /// Stop the current meeting
    Stop,
    /// Pause the current meeting
    Pause,
    /// Resume a paused meeting
    Resume,
    /// Show meeting status
    Status,
    /// List past meetings
    List {
        /// Maximum number of meetings to show
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
    /// Export a meeting transcript
    Export {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Output format: text, markdown, json
        #[arg(long, short, default_value = "markdown")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,

        /// Include timestamps in output
        #[arg(long)]
        timestamps: bool,

        /// Include speaker labels in output
        #[arg(long)]
        speakers: bool,

        /// Include metadata header in output
        #[arg(long)]
        metadata: bool,
    },
    /// Show meeting details
    Show {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,
    },
    /// Delete a meeting
    Delete {
        /// Meeting ID
        meeting_id: String,

        /// Skip confirmation prompt
        #[arg(long, short)]
        force: bool,
    },
    /// Label a speaker in a meeting transcript
    ///
    /// Assigns a human-readable name to an auto-generated speaker ID.
    /// Use with ML diarization to replace "SPEAKER_00" with "Alice".
    Label {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Speaker ID to label (e.g., "SPEAKER_00" or just "0")
        speaker_id: String,

        /// Human-readable label to assign
        label: String,
    },
    /// Generate an AI summary of a meeting
    ///
    /// Uses Ollama or a remote API to generate a summary with
    /// key points, action items, and decisions.
    Summarize {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Output format: text, json, or markdown
        #[arg(long, short, default_value = "markdown")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
    },
}
