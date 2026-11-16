//! `voxtype history` subcommands.

use clap::Subcommand;

pub const HISTORY_AFTER_HELP: &str = "\
History is a record of every dictation, including failed and cancelled ones,
kept as one plain JSON line per dictation in
~/.local/share/voxtype/history/YYYY-MM-DD.jsonl. Every command takes --json.

EXAMPLES:
  voxtype history                      Recent dictations, newest first
  voxtype history -n 50 --failed       The last 50 that failed or were cancelled
  voxtype history search invoice       Find a dictation by its words
  voxtype history search budget        Search dictations and meetings together
  voxtype history copy                 Copy the last transcript to the clipboard
  voxtype history paste 20261114-093012-a1b2c3 --delay-ms 500
                                       Type an old dictation again
  voxtype history correct 20261114-093012-a1b2c3 \"Hyprland, not high land\"
  voxtype history pause                Stop recording before something private
  voxtype history resume
  voxtype history clear --before 2026-11-01";

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum HistoryAction {
    /// Open the history window, or a terminal reader when no window is installed
    View {
        /// Explicitly use the terminal viewer
        #[arg(long)]
        tui: bool,
    },

    /// List recent dictations, newest first (the default)
    List {
        /// How many to show
        #[arg(short = 'n', long, value_name = "N", default_value_t = 20)]
        limit: usize,

        /// Only dictations that failed, were cancelled or heard no speech
        #[arg(long)]
        failed: bool,
    },

    /// Search dictations and meetings by their words
    ///
    /// Every word must appear; "quoted phrases" stay together and a
    /// trailing * matches a prefix (invoic*). Searches the output text, any
    /// correction, and what the engine heard.
    Search {
        /// Words to find
        #[arg(required = true, num_args = 1..)]
        words: Vec<String>,

        /// Most results to show
        #[arg(short = 'n', long, value_name = "N", default_value_t = 20)]
        limit: u32,

        /// Only from this date on (YYYY-MM-DD)
        #[arg(long, value_name = "DATE")]
        since: Option<String>,
    },

    /// Show one dictation in full, with its automatic edits
    Show {
        /// Entry ID, as `voxtype history` lists it, or `last`
        id: String,
    },

    /// Copy a dictation's text to the clipboard (the most recent when no ID)
    ///
    /// Copies the correction if there is one, otherwise what was output,
    /// otherwise what the engine heard (so a failed dictation can still be
    /// recovered).
    Copy {
        /// Entry ID, or `last`
        id: Option<String>,
    },

    /// Send a dictation's text again through the normal output path
    ///
    /// Types (or pastes, per [output] mode) into whatever has focus. From a
    /// terminal that is the terminal itself; use --delay-ms to switch
    /// windows first. Does not add a new history entry.
    Paste {
        /// Entry ID, or `last`
        id: String,

        /// Wait this long before typing, to focus the target window
        #[arg(long, value_name = "MS", default_value_t = 0)]
        delay_ms: u64,
    },

    /// Set your own correction of a dictation
    ///
    /// Stored as `corrected_text` beside what was output, never replacing
    /// it. An empty TEXT is an intentional empty correction.
    Correct {
        /// Entry ID, or `last`
        id: String,

        /// The corrected text
        text: String,
    },

    /// Delete one dictation from history and the search index
    Delete {
        /// Entry ID, or `last`
        id: String,
    },

    /// Delete whole days of history
    Clear {
        /// Only days before this date (YYYY-MM-DD); without it, everything
        #[arg(long, value_name = "DATE")]
        before: Option<String>,

        /// Don't ask for confirmation (required when not on a terminal)
        #[arg(short, long)]
        yes: bool,
    },

    /// Stop recording dictations until `voxtype history resume`
    ///
    /// Prevents this history collector from writing day files or search-index
    /// rows. Sets history.paused in the selected config; daemon CLI/environment
    /// overrides can take precedence. Does not disable independent legacy
    /// logging or normal file output.
    Pause,

    /// Start recording dictations again after `voxtype history pause`
    Resume,

    /// Turn history on (sets history.enabled = true)
    Enable,

    /// Turn history off (sets history.enabled = false); keeps what is stored
    Disable,

    /// Set how many days of history to keep (sets history.retention_days)
    ///
    /// Older days are deleted the next time the daemon prunes: when it starts
    /// and once a day after that. 0 keeps history forever.
    Retention {
        /// Days to keep; 0 keeps everything
        days: u32,
    },

    /// Whether history is on, paused, and how much it holds
    Status,

    /// Rebuild the search index from the day files
    Reindex,
}
