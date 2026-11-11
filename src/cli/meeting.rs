//! Meeting-mode subcommand actions.

use clap::builder::PossibleValuesParser;
use clap::Subcommand;

use super::DIARIZATION_BACKENDS;

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
        /// `hybrid` keeps the microphone as You and clusters only the loopback,
        /// naming remote voices Remote 1, Remote 2 (one remote voice stays
        /// Remote). Needs the same feature and model as `ml`.
        ///
        /// When omitted, falls back to `[meeting.diarization].backend` in config.
        #[arg(
            long,
            value_parser = PossibleValuesParser::new(DIARIZATION_BACKENDS),
            env = "VOXTYPE_MEETING_DIARIZATION",
        )]
        diarization: Option<String>,
    },
    /// Stop the current meeting
    Stop,
    /// Pause the current meeting
    Pause,
    /// Resume a paused meeting
    Resume,
    /// Show meeting status
    Status {
        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// List past meetings
    List {
        /// Maximum number of meetings to show
        #[arg(long, short, default_value = "10")]
        limit: u32,

        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
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

        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
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
    /// Assigns a human-readable name to a speaker the transcript contains:
    /// "You" and "Remote" with simple diarization, "SPEAKER_00" and so on
    /// with ML diarization, "You", "Remote 1", "Remote 2" with hybrid.
    /// Works while the meeting is still recording.
    /// Giving two speakers the same name merges them in exports.
    #[command(after_help = "Examples:\n  \
        voxtype meeting label latest You Pete\n  \
        voxtype meeting label latest SPEAKER_01 Dana\n  \
        voxtype meeting label latest 'Remote 2' Dana\n  \
        voxtype meeting label latest Dana 'Dana Smith'")]
    Label {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Speaker to label: an ID from the transcript (You, Remote,
        /// Remote 1, SPEAKER_00 or just 0), or a name already assigned
        speaker_id: String,

        /// Human-readable label to assign
        label: String,
    },
    /// Check for a call and say whether voxtype would offer to record it
    ///
    /// Meeting detection watches for a call app (Zoom, Teams, or Google Meet
    /// and Teams in a browser) opening the microphone and offers to record
    /// it with a notification. The daemon does this when `[meeting.detect]`
    /// is on; this command shows its reasoning.
    ///
    /// With --dry-run, reads the PipeWire and Hyprland state once and prints
    /// what would be prompted and why. Add --watch to keep running and print
    /// each decision as calls start and end. Without --dry-run, runs the
    /// detector in the foreground with real prompts (refused while the
    /// daemon is already prompting).
    #[command(after_help = "Examples:\n  \
        voxtype meeting suggest --dry-run\n  \
        voxtype meeting suggest --dry-run --watch")]
    Suggest {
        /// Print decisions only; never show a notification or start a meeting
        #[arg(long)]
        dry_run: bool,

        /// Keep running and print each decision
        #[arg(long)]
        watch: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::*;
    use clap::Parser;

    #[test]
    fn test_meeting_start_diarization_simple_flag() {
        let cli = Cli::parse_from(["voxtype", "meeting", "start", "--diarization", "simple"]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, .. },
            }) => {
                assert_eq!(diarization.as_deref(), Some("simple"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_hybrid_flag() {
        let cli = Cli::parse_from(["voxtype", "meeting", "start", "--diarization", "hybrid"]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, .. },
            }) => {
                assert_eq!(diarization.as_deref(), Some("hybrid"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_ml_flag() {
        let cli = Cli::parse_from([
            "voxtype",
            "meeting",
            "start",
            "--diarization",
            "ml",
            "--title",
            "standup",
        ]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, title },
            }) => {
                assert_eq!(diarization.as_deref(), Some("ml"));
                assert_eq!(title.as_deref(), Some("standup"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_rejects_invalid() {
        let result = Cli::try_parse_from(["voxtype", "meeting", "start", "--diarization", "bogus"]);
        assert!(
            result.is_err(),
            "clap should reject diarization values outside [\"simple\", \"ml\"]"
        );
    }

    /// Env-var wiring is exercised together with the "no override" case in a
    /// single test to avoid `VOXTYPE_MEETING_DIARIZATION` leaking between
    /// tests that run in parallel — env vars are process-global, so two
    /// independent #[test] functions would race.
    #[test]
    fn test_meeting_start_diarization_env_and_default() {
        // Make sure no stale value is set from the host or a sibling test.
        std::env::remove_var("VOXTYPE_MEETING_DIARIZATION");

        // No flag, no env var → no override.
        let cli = Cli::parse_from(["voxtype", "meeting", "start"]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, title },
            }) => {
                assert_eq!(diarization, None);
                assert_eq!(title, None);
            }
            _ => panic!("Expected Meeting Start command"),
        }

        // Env var alone should be picked up by clap's #[arg(env = ...)].
        std::env::set_var("VOXTYPE_MEETING_DIARIZATION", "ml");
        let cli = Cli::parse_from(["voxtype", "meeting", "start"]);
        std::env::remove_var("VOXTYPE_MEETING_DIARIZATION");
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, .. },
            }) => {
                assert_eq!(diarization.as_deref(), Some("ml"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }
}
