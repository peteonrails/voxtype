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

    /// Move one transcript line to another speaker
    ///
    /// For a line the diarizer gave to the wrong person. SPEAKER matches an
    /// existing speaker the way `meeting label` does: an ID from the
    /// transcript (You, Remote 1, SPEAKER_00 or just 0) or a name already
    /// assigned, ignoring case. Text that matches no one creates a new
    /// speaker with that name. The move sticks: later chunks and speaker
    /// renames do not undo it. Works while the meeting is still recording.
    /// Segment IDs are the `id` of each segment in `meeting show --json`.
    #[command(after_help = "Examples:\n  \
        voxtype meeting reassign latest 28 'Remote 2'\n  \
        voxtype meeting reassign latest 28 Dana\n  \
        voxtype meeting reassign f82f0c11 28 'Someone New' --json\n  \
        voxtype meeting reassign latest 28 --reset")]
    Reassign {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// The segment to move (its `id` in `meeting show --json`)
        segment_id: u32,

        /// Speaker to move it to: an ID, a name, or a new name
        #[arg(required_unless_present = "reset", conflicts_with = "reset")]
        speaker: Option<String>,

        /// Give the segment back to the speaker the diarizer assigned
        #[arg(long)]
        reset: bool,

        /// Print the updated segment as JSON
        #[arg(long)]
        json: bool,
    },
    /// Read or write a meeting's notes
    ///
    /// Free-text notes kept in notes.md in the meeting's folder. With no
    /// option, prints them. Safe while the meeting records: the daemon
    /// never overwrites them. Exports and transcript.md show them at the
    /// top under "Notes".
    #[command(after_help = "Examples:\n  \
        voxtype meeting notes latest\n  \
        voxtype meeting notes latest --append 'Follow up with Dana about the budget'\n  \
        voxtype meeting notes latest --set-file notes.md\n  \
        echo 'New notes' | voxtype meeting notes latest --set-file -\n  \
        voxtype meeting notes latest --json")]
    Notes {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// Add TEXT as a new line at the end of the notes
        #[arg(long, value_name = "TEXT", conflicts_with = "set_file")]
        append: Option<String>,

        /// Replace the notes with the contents of PATH ("-" reads stdin)
        #[arg(long, value_name = "PATH")]
        set_file: Option<std::path::PathBuf>,

        /// Print {meeting_id, notes, updated_at} as JSON
        #[arg(long)]
        json: bool,
    },
    /// Attach a note or highlight to one transcript line
    ///
    /// Replaces any annotation the line had. Segment IDs are the `id` of
    /// each segment in `meeting show --json`. Works while the meeting is
    /// still recording. Exports and transcript.md show the annotation as a
    /// quote under its line.
    #[command(after_help = "Examples:\n  \
        voxtype meeting annotate latest 28 'Check this number'\n  \
        voxtype meeting annotate latest 28 'Decision' --highlight\n  \
        voxtype meeting annotate latest 28 --highlight\n  \
        voxtype meeting annotate latest 28 --remove")]
    Annotate {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// The segment to annotate (its `id` in `meeting show --json`)
        segment_id: u32,

        /// The note
        #[arg(required_unless_present_any = ["highlight", "remove"], conflicts_with = "remove")]
        text: Option<String>,

        /// Highlight the line, with or without a note
        #[arg(long, conflicts_with = "remove")]
        highlight: bool,

        /// Remove the line's annotation
        #[arg(long)]
        remove: bool,

        /// Print the updated segment as JSON
        #[arg(long)]
        json: bool,
    },
    /// Generate an AI summary of a meeting
    ///
    /// Writes an overview, key points, decisions, action items with owners
    /// and chapters with timestamps, saves it with the meeting (summary.md,
    /// metadata.json and the index) and prints it. A meeting that already
    /// has a summary prints the saved one; --regenerate writes a new one,
    /// for example after naming speakers or moving lines.
    ///
    /// The summary comes from the meeting assistant (your coding agent, run
    /// with no tools, or the local Ollama model), unless
    /// [meeting.summary] backend is "local" or "remote", which keep the
    /// older Ollama and API summarizers.
    #[command(after_help = "Examples:\n  \
        voxtype meeting summarize latest\n  \
        voxtype meeting summarize latest --regenerate\n  \
        voxtype meeting summarize latest --local --format json")]
    Summarize {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Output format: text, json, or markdown
        #[arg(long, short, default_value = "markdown")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,

        /// Write a new summary even if the meeting already has one
        #[arg(long)]
        regenerate: bool,

        /// Agent to use for this summary (claude or codex)
        #[arg(long, value_name = "AGENT", env = "VOXTYPE_MEETING_AGENT")]
        agent: Option<String>,

        /// Use the local model (Ollama) instead of an agent
        #[arg(long, conflicts_with = "agent")]
        local: bool,
    },
    /// Ask a question about a meeting, during or after it
    ///
    /// Sends the transcript, with speaker names as you labelled them, your
    /// notes and the question to the meeting assistant and streams the
    /// answer. Answers cite the [mm:ss] of the lines they rely on. Each
    /// question and answer is saved to the meeting's asks.jsonl.
    ///
    /// The assistant is your coding agent run headless with no tools:
    /// [meeting.assistant] agent, else the Omarchy default agent, else
    /// claude found on PATH. Its first use asks once whether transcripts
    /// may go to that agent (see `voxtype meeting assistant`). --local, or
    /// [meeting.assistant] local_only = true, answers with the local Ollama
    /// model instead.
    ///
    /// --json prints one JSON object per line: {"event":"start",...},
    /// {"event":"delta","text":...} as the answer streams, then
    /// {"event":"answer",...} with the saved record, or {"event":"error",...}.
    #[command(after_help = "Examples:\n  \
        voxtype meeting ask latest \"What did Steve just say?\" --last 2m\n  \
        voxtype meeting ask latest \"Summarize the last few minutes\" --last 5m\n  \
        voxtype meeting ask latest \"What are the action items so far?\"\n  \
        voxtype meeting ask latest \"Who owns the release?\" --notify\n  \
        voxtype meeting ask 3f2a \"What did we decide?\" --local --json")]
    Ask {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// The question
        question: String,

        /// Only consider the end of the transcript: 5m, 90s, 1h, 2m30s,
        /// or a number of minutes
        #[arg(long, value_name = "DURATION")]
        last: Option<String>,

        /// Print JSON events (one per line) instead of plain text
        #[arg(long)]
        json: bool,

        /// Also show the answer as a desktop notification. Its Open action
        /// raises the meeting view on the Ask tab, or runs
        /// [meeting.assistant] open_command when that is set
        #[arg(long)]
        notify: bool,

        /// Agent to use for this question (claude or codex)
        #[arg(long, value_name = "AGENT", env = "VOXTYPE_MEETING_AGENT")]
        agent: Option<String>,

        /// Use the local model (Ollama) instead of an agent
        #[arg(long, conflicts_with = "agent")]
        local: bool,
    },
    /// Choose and allow the agent that answers questions and writes summaries
    #[command(after_help = "Examples:\n  \
        voxtype meeting assistant status\n  \
        voxtype meeting assistant status --json\n  \
        voxtype meeting assistant allow claude\n  \
        voxtype meeting assistant revoke claude")]
    Assistant {
        #[command(subcommand)]
        action: AssistantAction,
    },
}

/// `voxtype meeting assistant` actions
#[derive(Subcommand)]
pub enum AssistantAction {
    /// Allow meeting transcripts to go to an agent
    ///
    /// The agent sends what it is given to the account it is signed in to,
    /// usually a cloud service. voxtype asks once per agent; this records
    /// the answer. It also turns on summaries at stop unless
    /// [meeting.summary] auto = false.
    Allow {
        /// Agent: claude or codex
        agent: String,
    },
    /// Stop sending meeting transcripts to an agent
    Revoke {
        /// Agent: claude or codex
        agent: String,
    },
    /// Show which agent answers, what was found, and consent per agent
    Status {
        /// Output as JSON (for the meeting view and Settings panel)
        #[arg(long)]
        json: bool,
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
