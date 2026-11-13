//! Meeting-mode subcommand actions.

use clap::builder::PossibleValuesParser;
use clap::Subcommand;

use super::DIARIZATION_BACKENDS;

/// Meeting mode actions
#[derive(Subcommand)]
pub enum MeetingAction {
    /// Start a new meeting transcription
    ///
    /// Without --engine or --model the meeting transcribes with dictation's
    /// engine and model, reusing the model dictation already has loaded so
    /// the meeting starts at once. A different engine, model or language
    /// loads its own copy. An engine this binary lacks, or a model that is
    /// not installed, is refused before the meeting starts.
    #[command(after_help = "Examples:\n  \
        voxtype meeting start --title Standup\n  \
        voxtype meeting start --title 'Planning' --url https://meet.example.com/abc-defg-hij\n  \
        voxtype meeting start --model large-v3-turbo --language de\n  \
        voxtype meeting start --engine parakeet --model parakeet-ultra-Q8_0.gguf\n  \
        voxtype meeting start --announce\n  \
        voxtype meeting start --announce --announcement-file ~/announce-de.wav")]
    Start {
        /// Meeting title (optional)
        #[arg(long, short)]
        title: Option<String>,

        /// Transcription engine for this meeting (default: the configured
        /// engine). Without --model it uses that engine's configured model.
        #[arg(long, env = "VOXTYPE_MEETING_ENGINE")]
        engine: Option<String>,

        /// Model for this meeting, as `voxtype info models` lists it. Its
        /// engine is implied, so --engine is optional.
        #[arg(long, env = "VOXTYPE_MEETING_MODEL")]
        model: Option<String>,

        /// Language hint for this meeting, for engines and checkpoints that
        /// take one (multilingual Whisper, Cohere)
        #[arg(long, env = "VOXTYPE_MEETING_LANGUAGE")]
        language: Option<String>,

        /// Link to the call, stored with the meeting and shown by
        /// `meeting show` and exports
        #[arg(long)]
        url: Option<String>,

        /// A title taken from the call's window, as call detection gives
        /// it. Unlike --title, a title from your calendar replaces it.
        #[arg(long, value_name = "TITLE", conflicts_with = "title")]
        window_title: Option<String>,

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

        /// Announce the recording when the meeting starts: "This meeting is
        /// being recorded and transcribed with Voxtype", mixed into the call
        /// app's microphone so the others on the call hear it, and played on
        /// your speakers. Waits up to two minutes for a call app to open the
        /// mic; without one it plays only on your speakers, and `meeting
        /// status` says so. A courtesy, not consent: you remain responsible
        /// for the laws on recording calls.
        ///
        /// When omitted, falls back to `[meeting] announce_recording`
        /// (VOXTYPE_MEETING_ANNOUNCE_RECORDING), off by default.
        #[arg(long, overrides_with = "no_announce")]
        announce: bool,

        /// Do not announce this meeting, whatever the config says
        #[arg(long, overrides_with = "announce")]
        no_announce: bool,

        /// Audio file (WAV, FLAC or Ogg) to announce with instead of the
        /// built-in clip, for your own wording or language. Falls back to
        /// `[meeting] announcement_file` (VOXTYPE_MEETING_ANNOUNCEMENT_FILE).
        #[arg(long, value_name = "FILE")]
        announcement_file: Option<std::path::PathBuf>,

        /// Voice of the built-in announcement for this meeting: kokoro (built
        /// in) or jessica (an ElevenLabs voice, downloaded once; see `voxtype
        /// meeting announcement`). Falls back to `[meeting]
        /// announcement_voice` (VOXTYPE_MEETING_ANNOUNCEMENT_VOICE).
        #[arg(long, value_name = "VOICE", value_parser = PossibleValuesParser::new(super::ANNOUNCEMENT_VOICES))]
        announcement_voice: Option<String>,
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

        /// Output format; md and txt also work for markdown and text
        #[arg(
            long,
            short,
            default_value = "markdown",
            ignore_case = true,
            value_parser = super::export_format_parser()
        )]
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
    /// Share a meeting's summary and notes, and optionally its transcript
    ///
    /// Sends the title, date, length and attendees, the summary's overview,
    /// key points, decisions, action items and chapters, your notes and
    /// highlights, and with --transcript the transcript as voxtype shows it
    /// (names applied, deleted lines left out, edited lines corrected).
    /// Nothing is shared unless you run this; --preview prints exactly what
    /// would leave without sending it.
    ///
    /// Destinations: clipboard (wl-copy, the default), file (a new file in
    /// [meeting.share] folder, Documents/Voxtype by default), email (a
    /// mailto: draft in your mail app, with any transcript saved as a file
    /// to attach), or command ([meeting.share] command or --command, given
    /// the share on stdin).
    ///
    /// --json prints the result as JSON; with --preview it prints the share
    /// itself and which destinations work here.
    #[command(after_help = "Examples:\n  \
        voxtype meeting share latest\n  \
        voxtype meeting share latest --transcript --to file\n  \
        voxtype meeting share latest --to email\n  \
        voxtype meeting share latest --format html --to clipboard\n  \
        voxtype meeting share latest --to command --command 'pandoc -o notes.pdf'\n  \
        voxtype meeting share latest --transcript --preview")]
    Share {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Include the transcript after the notes
        #[arg(long)]
        transcript: bool,

        /// How to write it: md (markdown), txt (text) or html. Email always
        /// sends text.
        #[arg(
            long,
            short,
            default_value = "md",
            ignore_case = true,
            value_parser = clap::builder::PossibleValuesParser::new(["md", "markdown", "txt", "text", "html"])
        )]
        format: String,

        /// Where to send it: clipboard, file, email or command
        #[arg(
            long,
            default_value = "clipboard",
            ignore_case = true,
            value_parser = clap::builder::PossibleValuesParser::new(["clipboard", "file", "email", "command"])
        )]
        to: String,

        /// For --to file: the file to write, or a folder to write a new file
        /// in (default: [meeting.share] folder)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,

        /// For --to command: the shell command to pipe the share to,
        /// instead of [meeting.share] command
        #[arg(long, value_name = "COMMAND")]
        command: Option<String>,

        /// Print what would be shared and send nothing
        #[arg(long)]
        preview: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List a meeting's action items, or send them to Todoist or as
    /// Omarchy reminders
    ///
    /// The items come from the meeting's summary (`voxtype meeting
    /// summarize`): what to do, who took it on, the deadline as said and
    /// the day it resolves to, and where in the meeting it came up.
    ///
    /// --send todoist adds each item as a task (connect first with
    /// `voxtype meeting todoist connect`). --send reminder sets an Omarchy
    /// reminder for each, after [meeting.actions] remind_in_mins, or at the
    /// time said when that is later today. Every item is sent unless you
    /// narrow it with --pick or --mine, and an item already sent to that
    /// place is skipped unless you add --resend.
    #[command(after_help = "Examples:\n  \
        voxtype meeting actions latest\n  \
        voxtype meeting actions latest --send todoist\n  \
        voxtype meeting actions latest --send todoist --pick 1,3\n  \
        voxtype meeting actions latest --send reminder --mine --in 30\n  \
        voxtype meeting actions latest --json")]
    Actions {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Send the items: todoist or reminder
        #[arg(
            long,
            value_name = "TARGET",
            value_parser = clap::builder::PossibleValuesParser::new(["todoist", "reminder"])
        )]
        send: Option<String>,

        /// Every item (the default)
        #[arg(long, conflicts_with_all = ["pick", "mine"])]
        all: bool,

        /// Only these items, by number: 1,3 or 2-4
        #[arg(long, value_name = "LIST")]
        pick: Option<String>,

        /// Only your own items: owned by You or [meeting] my_name
        #[arg(long)]
        mine: bool,

        /// Send again items already sent there
        #[arg(long)]
        resend: bool,

        /// For reminders: minutes until they go off, instead of
        /// [meeting.actions] remind_in_mins
        #[arg(long = "in", value_name = "MINUTES")]
        remind_in: Option<u32>,

        /// For Todoist: the project ID to add tasks to, instead of
        /// [meeting.actions] todoist_project_id (default: Inbox)
        #[arg(long, value_name = "ID")]
        project: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
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
    /// Finish or discard meetings the daemon never finished
    ///
    /// A meeting is saved as it records, so when the daemon crashes or is
    /// killed mid-meeting its transcript and audio are on disk, but the
    /// meeting never ends: no end time, no summary, no post-meeting
    /// command. The daemon marks such meetings interrupted when it next
    /// starts. Without options this lists them; --finish transcribes any
    /// audio the daemon kept but never transcribed, ends the meeting when it
    /// last wrote, then summarizes it and runs post_meeting_command as a
    /// normal stop would; --discard deletes it and its folder. A meeting
    /// still recording is never touched.
    #[command(after_help = "Examples:\n  \
        voxtype meeting recover\n  \
        voxtype meeting recover --json\n  \
        voxtype meeting recover --finish 3f2a\n  \
        voxtype meeting recover --discard 3f2a")]
    Recover {
        /// Finish this meeting (ID or unique prefix)
        #[arg(long, value_name = "ID", conflicts_with = "discard")]
        finish: Option<String>,

        /// Delete this meeting and its folder (ID or unique prefix)
        #[arg(long, value_name = "ID")]
        discard: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Meeting titles from your calendar: the private iCal link
    #[command(subcommand)]
    Calendar(CalendarAction),
    /// Give a meeting a new title
    ///
    /// Works on a meeting that is recording, paused or finished. The new
    /// title shows in the meeting view at once and is stored everywhere
    /// the meeting is listed; the meeting's folder keeps its name. A
    /// two-person title such as "Pete <> Chad" names the speakers, as at
    /// the start of a meeting, without replacing names you gave by hand.
    #[command(after_help = "Examples:\n  \
        voxtype meeting title latest \"Pete <> Chad Bi-Weekly\"\n  \
        voxtype meeting title 3f2a \"Q4 planning\" --json")]
    Title {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// The new title
        title: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Label a speaker in a meeting transcript
    ///
    /// Assigns a human-readable name to a speaker the transcript contains:
    /// "You" and "Remote" with simple diarization, "SPEAKER_00" and so on
    /// with ML diarization, "You", "Remote 1", "Remote 2" with hybrid.
    /// Works while the meeting is still recording.
    /// Giving two speakers the same name merges them in exports.
    ///
    /// Action items in the summary owned by the speaker's old name (exactly,
    /// ignoring case) take the new one; nothing is summarized again, and
    /// items you own stay yours. Tasks already sent to Todoist are not edited.
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
    /// Correct the text of one transcript line
    ///
    /// The line keeps what was transcribed as `original_text`, and --reset
    /// restores it. Segment IDs are the `id` of each segment in
    /// `meeting show --json`. Works while the meeting is still recording;
    /// later chunks do not undo the correction.
    #[command(after_help = "Examples:\n  \
        voxtype meeting edit latest 28 'We ship on Friday.'\n  \
        voxtype meeting edit latest 28 'We ship on Friday.' --json\n  \
        voxtype meeting edit latest 28 --reset")]
    Edit {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// The segment to correct (its `id` in `meeting show --json`)
        segment_id: u32,

        /// The corrected text
        #[arg(required_unless_present = "reset", conflicts_with = "reset")]
        text: Option<String>,

        /// Restore the text as transcribed
        #[arg(long)]
        reset: bool,

        /// Print the updated segment as JSON
        #[arg(long)]
        json: bool,
    },
    /// Hide one transcript line, or bring it back
    ///
    /// The line is kept (marked deleted in transcript.json) so --restore can
    /// bring it back, but exports, transcript.md, `meeting show` and the
    /// meeting assistant leave it out. Works while the meeting is still
    /// recording.
    #[command(after_help = "Examples:\n  \
        voxtype meeting delete-line latest 28\n  \
        voxtype meeting delete-line latest 28 --restore")]
    DeleteLine {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// The segment to hide (its `id` in `meeting show --json`)
        segment_id: u32,

        /// Bring the line back
        #[arg(long)]
        restore: bool,

        /// Print the segment as JSON (with "deleted": true when hidden)
        #[arg(long)]
        json: bool,
    },
    /// Show the recording announcement's voice, or download the Jessica voice
    ///
    /// The announcement (`[meeting] announce_recording`) speaks in the
    /// built-in Kokoro voice unless `announcement_voice = "jessica"`.
    /// Jessica is an ElevenLabs voice that is not part of voxtype: it is
    /// downloaded once, checked against its published sha256, and kept in
    /// ~/.local/share/voxtype/sounds. Its terms are not MIT: it may be
    /// redistributed only unmodified with voxtype, and may not be used to
    /// train or improve AI models. A meeting downloads it itself when it
    /// isn't there yet, and plays the built-in voice if that fails;
    /// `--voice jessica` downloads it now instead.
    #[command(after_help = "Examples:\n  \
        voxtype meeting announcement\n  \
        voxtype meeting announcement --voice jessica\n  \
        voxtype config set meeting.announcement_voice jessica")]
    Announcement {
        /// Prepare this voice now: jessica downloads it (once), kokoro is
        /// built in and needs nothing
        #[arg(long, value_name = "VOICE", value_parser = PossibleValuesParser::new(super::ANNOUNCEMENT_VOICES))]
        voice: Option<String>,

        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show the recording-consent notice, or acknowledge it
    ///
    /// Laws on recording and transcribing conversations differ by place, and
    /// some require everyone on the call to agree. Voxtype doesn't tell the
    /// other people for you. Until you acknowledge this, `meeting start`
    /// prints it on stderr and the meeting views show it. --ack records that
    /// you have read it.
    #[command(after_help = "Examples:\n  \
        voxtype meeting notice\n  \
        voxtype meeting notice --ack\n  \
        voxtype meeting notice --json")]
    Notice {
        /// Record that you have read the notice, so it is no longer shown
        #[arg(long)]
        ack: bool,

        /// Output as JSON: {"notice", "acknowledged", "acknowledged_at"}
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
    /// Play a meeting's kept audio from a line or a time
    ///
    /// Plays the audio kept with `[meeting] retain_audio = true` in the
    /// background, through ffmpeg and pw-play (or paplay, or aplay), and
    /// returns at once. Only one playback runs at a time: starting another
    /// stops the one before. Segment IDs are the `id` of each segment in
    /// `meeting show --json`.
    #[command(after_help = "Examples:\n  \
        voxtype meeting play latest\n  \
        voxtype meeting play latest --segment 28\n  \
        voxtype meeting play latest --at 12:40 --track loopback\n  \
        voxtype meeting play latest --segment 28 --json\n  \
        voxtype meeting play --status --json\n  \
        voxtype meeting play --stop")]
    Play {
        /// Meeting ID, the start of one, or "latest" for the most recent
        #[arg(required_unless_present_any = ["stop", "status"])]
        meeting_id: Option<String>,

        /// Start at this transcript segment (its `id` in `meeting show --json`)
        #[arg(long, value_name = "ID", conflicts_with = "at")]
        segment: Option<u32>,

        /// Start this far into the meeting: seconds (95), mm:ss (1:35) or hh:mm:ss
        #[arg(long, value_name = "TIME")]
        at: Option<String>,

        /// Track to play: mixed (both sides), mic (you) or loopback (the others)
        #[arg(long, default_value = "mixed", value_parser = ["mixed", "mic", "loopback"])]
        track: String,

        /// Stop the playback that is running
        #[arg(long, conflicts_with_all = ["status", "meeting_id", "segment", "at"])]
        stop: bool,

        /// Report the playback that is running, or the last one
        #[arg(long, conflicts_with_all = ["meeting_id", "segment", "at"])]
        status: bool,

        /// Print the result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search across meetings
    ///
    /// Full-text search over every meeting's transcript lines as they are
    /// displayed (speaker names, moved lines and text corrections applied;
    /// deleted lines left out), and over titles, notes and summaries. Every
    /// word must appear. Put a phrase in double quotes (inside single quotes
    /// for the shell) to keep it together, and end a word with * to match
    /// words starting with it. Best matches come first. Meetings recorded
    /// before search existed are indexed the first time you search.
    #[command(after_help = "Examples:\n  \
        voxtype meeting search budget\n  \
        voxtype meeting search budget review --speaker Dana\n  \
        voxtype meeting search '\"next quarter\"' --since 2026-11-01\n  \
        voxtype meeting search deploy* --limit 5 --json")]
    Search {
        /// Words to find
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,

        /// Show at most this many matches
        #[arg(long, short, default_value = "20")]
        limit: u32,

        /// Only lines spoken by this speaker, as shown in the transcript
        /// (case-insensitive). Your `[meeting] my_name` also finds "You".
        #[arg(long, value_name = "NAME")]
        speaker: Option<String>,

        /// Only meetings started on or after this date (2026-11-01) or time
        /// (2026-11-01T09:00:00Z)
        #[arg(long, value_name = "DATE")]
        since: Option<String>,

        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show the name meetings use for you, or a suggestion for it
    ///
    /// Meetings show your side as "You" unless `[meeting] my_name` is set.
    /// This prints the setting and, when it is unset, the full name on your
    /// account as a suggestion. It never sets anything.
    #[command(after_help = "Examples:\n  \
        voxtype meeting my-name\n  \
        voxtype config set meeting.my_name \"Pete\"")]
    MyName {
        /// Output as JSON: {"my_name": ..., "suggestion": ...}
        #[arg(long)]
        json: bool,
    },
    /// Peak levels of a meeting's kept audio, for drawing a waveform
    ///
    /// Reads the meeting's mic and loopback audio (kept with
    /// `[meeting] retain_audio = true`), decodes it with ffmpeg and reports
    /// the peak level of each time bucket in dBFS, -100 for silence. The
    /// result is cached in the meeting folder as waveform.json, so asking
    /// again is instant until the audio changes. A meeting that kept no
    /// audio exits with status 2.
    #[command(after_help = "Examples:\n  \
        voxtype meeting waveform latest\n  \
        voxtype meeting waveform latest --buckets 2000 --json")]
    Waveform {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// How many buckets to split the meeting into (1 to 100000). Each
        /// is a whole number of milliseconds, so there can be fewer.
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..=100_000))]
        buckets: u64,

        /// Print the result as JSON:
        /// {"duration_ms", "bucket_ms", "mic": [...], "loopback": [...] or null, "mic_track"}
        #[arg(long)]
        json: bool,
    },

    /// Open the live meeting view
    ///
    /// Asks whichever meeting view is running to open: the Voxtype plugin in
    /// the Omarchy shell, or voxtype's standalone Quickshell shell (`voxtype
    /// setup quickshell`). Both read the request this writes to
    /// $XDG_RUNTIME_DIR/voxtype/meeting-view.request, so this command does not
    /// need to know which one you use. An Ask notification's Open action runs
    /// it. Nothing opens if neither is running.
    ///
    /// Without Quickshell (GNOME, Plasma, Sway, X11), this opens the meeting
    /// window `voxtype-meetings-gtk4` instead. `[meeting] view` (or
    /// VOXTYPE_MEETING_VIEW) chooses: auto, quickshell, gtk4, terminal (runs
    /// `voxtype meeting watch`) or none. --minimize and --expand only act on
    /// the Quickshell view.
    ///
    /// With a meeting ID other than the current one, the view opens on that
    /// meeting's saved transcript. --ask asks one of the view's quick
    /// questions; with --no-open the answer arrives as a notification instead.
    ///
    /// As `[meeting.assistant] open_command`, plain `voxtype meeting view`
    /// is enough: the notification sets VOXTYPE_MEETING_ID and
    /// VOXTYPE_ASK_ID, and the view opens on that answer.
    #[command(after_help = "Examples:\n  \
        voxtype meeting view\n  \
        voxtype meeting view latest --tab summary\n  \
        voxtype meeting view --ask last-5m --no-open")]
    View {
        /// Meeting ID (or "latest"); default: the current or last meeting
        #[arg(env = "VOXTYPE_MEETING_ID")]
        meeting_id: Option<String>,

        /// A saved answer to show on the Ask tab
        #[arg(long, env = "VOXTYPE_ASK_ID")]
        ask_id: Option<String>,

        /// Tab to open on
        #[arg(long, value_parser = ["transcript", "notes", "ask", "summary"])]
        tab: Option<String>,

        /// Ask a quick question: the last five minutes, what the last remote
        /// speaker just said, or the action items so far
        #[arg(long, value_parser = ["last-5m", "last-speaker", "actions"])]
        ask: Option<String>,

        /// Do not open the view (with --ask, the answer is a notification)
        #[arg(long)]
        no_open: bool,

        /// Collapse the view to its pill (opening it as one if it is closed)
        #[arg(long, conflicts_with = "expand")]
        minimize: bool,

        /// Expand the pill back to the full view
        #[arg(long)]
        expand: bool,
    },
    /// Import a recording made elsewhere as a meeting
    ///
    /// Any file ffmpeg reads (mp3, m4a, ogg, opus, wav, flac, video) becomes
    /// a normal meeting: transcript, speakers, exports, kept audio, the
    /// summary at the end when auto-summary is on, and post_meeting_command.
    /// Runs here rather than in the daemon, so it never interrupts a meeting
    /// being recorded. Prints progress as it goes.
    ///
    /// By default every channel is mixed into one track and speakers are
    /// told apart by diarization alone. For a call recorded with you on one
    /// channel and the others on another, name the channels in order with
    /// --channels mic,loopback (or loopback,mic) so lines are attributed to
    /// You and Remote, and hybrid diarization can split the remote side.
    ///
    /// Given a folder written by Omarchy Meeting Recorder, imports that
    /// meeting as it is: its transcript, speaker names, chapters, notes and
    /// audio. Transcribe it again with `meeting retranscribe` if you like.
    #[command(after_help = "Examples:\n  \
        voxtype meeting import call.mp3 --title 'Client call'\n  \
        voxtype meeting import recording.wav --channels mic,loopback\n  \
        voxtype meeting import memo.m4a --date 2026-11-02T15:00:00Z --model large-v3-turbo\n  \
        voxtype meeting import zoom.mp4 --json\n  \
        voxtype meeting import ~/Documents/Meetings/'202511021500 Launch sync'")]
    Import {
        /// The recording to import, or a Meeting Recorder meeting folder
        file: std::path::PathBuf,

        /// Meeting title (default: the file name)
        #[arg(long, short)]
        title: Option<String>,

        /// Transcription engine (default: the configured engine)
        #[arg(long)]
        engine: Option<String>,

        /// Model, as `voxtype info models` lists it; implies its engine
        #[arg(long)]
        model: Option<String>,

        /// Language hint for engines that take one
        #[arg(long)]
        language: Option<String>,

        /// When the recording started, RFC 3339 (default: the file's
        /// creation_time tag, else its modification time)
        #[arg(long, value_name = "RFC3339")]
        date: Option<String>,

        /// How channels map to sides: `mono` (mix them all, the default),
        /// or one of mic, loopback or ignore per channel, comma-separated
        #[arg(long, value_name = "MAP")]
        channels: Option<String>,

        /// Diarization backend for this import (default: the configured one)
        #[arg(long, value_parser = PossibleValuesParser::new(DIARIZATION_BACKENDS))]
        diarization: Option<String>,

        /// Print progress and the result as JSON lines
        #[arg(long)]
        json: bool,
    },
    /// Transcribe a meeting again from its kept audio
    ///
    /// Re-runs a meeting recorded with retain_audio, or imported, with
    /// another engine, model or language. The meeting keeps its ID, notes,
    /// speaker names and summary; its previous transcript is kept as
    /// transcript.v1.json (then v2, ...). Annotations move to the new line
    /// they overlap most in time. Speaker moves, text corrections and
    /// deleted lines do the same with --keep-edits (the default), or are
    /// dropped with --discard-edits; the output says how many carried over.
    /// Speaker names carry over by speaker, so with ML diarization check
    /// that the same voice kept the same speaker number.
    #[command(after_help = "Examples:\n  \
        voxtype meeting retranscribe latest --model large-v3-turbo\n  \
        voxtype meeting retranscribe b6b2 --engine parakeet --discard-edits\n  \
        voxtype meeting retranscribe latest --language de --json")]
    Retranscribe {
        /// Meeting ID, the start of one, or "latest" for the most recent
        meeting_id: String,

        /// Transcription engine (default: the configured engine)
        #[arg(long)]
        engine: Option<String>,

        /// Model, as `voxtype info models` lists it; implies its engine
        #[arg(long)]
        model: Option<String>,

        /// Language hint for engines that take one
        #[arg(long)]
        language: Option<String>,

        /// Diarization backend for this run (default: the configured one)
        #[arg(long, value_parser = PossibleValuesParser::new(DIARIZATION_BACKENDS))]
        diarization: Option<String>,

        /// Carry speaker moves, text corrections and deleted lines over to
        /// the new lines they overlap (the default)
        #[arg(long, conflicts_with = "discard_edits")]
        keep_edits: bool,

        /// Drop speaker moves, text corrections and deleted lines
        #[arg(long)]
        discard_edits: bool,

        /// Print progress and the result as JSON lines
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

        /// Agent to use for this summary (claude, codex or pi)
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
    /// claude or pi found on PATH. Its first use asks once whether transcripts
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

        /// Agent to use for this question (claude, codex or pi)
        #[arg(long, value_name = "AGENT", env = "VOXTYPE_MEETING_AGENT")]
        agent: Option<String>,

        /// Use the local model (Ollama) instead of an agent
        #[arg(long, conflicts_with = "agent")]
        local: bool,
    },
    /// Watch the meeting in the terminal
    ///
    /// Shows the running meeting live: state and elapsed time, the
    /// transcript as it is transcribed (following new lines until you
    /// scroll up; End follows again), and its speakers. Keys: p pause or
    /// resume, s stop, r move the selected line to another speaker, n note
    /// on the selected line, a ask the assistant, q quit. With no meeting
    /// running it lists recent meetings; Enter opens one, Esc goes back.
    ///
    /// --json prints the daemon's meeting.sock events as NDJSON instead,
    /// for scripts: one JSON object per line, starting with a `hello`
    /// snapshot, and a new `hello` after the daemon restarts. Level events
    /// are left out unless --levels is given too.
    ///
    /// --levels prints the per-track level events (about ten a second while
    /// a meeting records) as NDJSON, for a level meter.
    #[command(after_help = "Examples:\n  \
        voxtype meeting watch\n  \
        voxtype meeting watch --json | jq -c 'select(.event == \"segments\")'\n  \
        voxtype meeting watch --levels\n  \
        voxtype meeting watch --json --levels")]
    Watch {
        /// Print the live events as NDJSON instead of drawing the screen
        #[arg(long)]
        json: bool,

        /// Print the microphone and loopback level events as NDJSON (with
        /// --json, alongside every other event)
        #[arg(long)]
        levels: bool,
    },
    /// Check the meeting microphone and loopback levels without recording
    ///
    /// Opens the meeting's microphone and loopback exactly as a meeting
    /// would (the same devices, and on speakers the same echo canceller),
    /// records and transcribes nothing, and shows both tracks' levels about
    /// ten times a second until the time is up or you press Ctrl+C. Speak,
    /// then play something on the computer: your voice should move the mic
    /// meter, the other side the loopback meter.
    ///
    /// While the daemon runs it does the check, since it owns the audio, and
    /// the levels also go to meeting.sock for the meeting view. Without it,
    /// this command opens the audio itself. A meeting that is recording
    /// already has its levels: use `voxtype meeting watch --levels`.
    ///
    /// --json prints NDJSON: a `soundcheck` event when it starts (with the
    /// echo path and devices), `levels` events, and a `soundcheck` event
    /// when it stops.
    #[command(after_help = "Examples:\n  \
        voxtype meeting soundcheck\n  \
        voxtype meeting soundcheck --seconds 60 --json\n  \
        voxtype meeting soundcheck --stop")]
    Soundcheck {
        /// How long to run, in seconds (1 to 600)
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=600))]
        seconds: u64,

        /// Print NDJSON events instead of meters
        #[arg(long)]
        json: bool,

        /// Stop a running sound check
        #[arg(long, conflicts_with_all = ["seconds", "json"])]
        stop: bool,
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
    /// Connect a Todoist account for `meeting actions --send todoist`
    Todoist {
        #[command(subcommand)]
        action: TodoistAction,
    },
}

/// `voxtype meeting todoist` actions
#[derive(Subcommand)]
pub enum TodoistAction {
    /// Connect your Todoist account in the browser
    ///
    /// Opens Todoist's consent page and waits for you to approve. Voxtype
    /// asks only to add tasks (scope task:add): it cannot read your tasks
    /// or projects. The access token is kept in
    /// $XDG_STATE_HOME/voxtype/todoist.json, readable only by you, and
    /// never in config.toml.
    ///
    /// The browser returns to http://localhost on one of ports 47815 to
    /// 47819. On a machine without a browser, set
    /// [meeting.actions] todoist_token_file to a file holding a personal API
    /// token instead.
    ///
    /// --json prints one JSON object per line: {"event":"authorize","url":...}
    /// once the consent page is open, then {"event":"connected",...} or
    /// {"event":"error","message":...}.
    #[command(after_help = "Examples:\n  \
        voxtype meeting todoist connect\n  \
        voxtype meeting todoist connect --no-browser\n  \
        voxtype meeting todoist status")]
    Connect {
        /// Seconds to wait for you to approve
        #[arg(long, default_value = "300", value_name = "SECS")]
        timeout: u64,

        /// Print the address to open instead of opening a browser
        #[arg(long)]
        no_browser: bool,

        /// Print JSON events, one per line
        #[arg(long)]
        json: bool,
    },
    /// Forget the Todoist connection on this machine
    Disconnect {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show whether Todoist is connected
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Where to find the link, and what it gives away: for `--help`, the
/// docs, and the Settings entry.
pub const CALENDAR_LINK_HELP: &str = "Where to find your calendar's private iCal link:
  Google Calendar: Settings, your calendar under \"Settings for my calendars\",
    Integrate calendar, then \"Secret address in iCal format\".
  Outlook: Settings, Calendar, Shared calendars, Publish a calendar. Pick the
    calendar and \"Can view all details\", Publish, then copy the ICS link.
  iCloud: on iCloud.com, the share button beside the calendar, tick
    \"Public Calendar\", then copy the webcal:// link.
  Fastmail: Settings, Calendars, the calendar's Edit & share, then its
    read-only iCalendar URL.

The link lets anyone who has it read your calendar. Voxtype keeps it in
~/.local/state/voxtype/calendar.json, readable only by you, never in
config.toml and never in logs. If it leaks, reset the secret address in your
calendar's settings and set the new one.";

/// `voxtype meeting calendar` actions
#[derive(Subcommand)]
pub enum CalendarAction {
    /// Set your calendar's private iCal link
    ///
    /// A meeting that starts without a title (or with only the call
    /// window's) is named after the calendar event happening then. The
    /// daemon fetches the link in the background every 15 minutes.
    ///
    /// Pass - to read the link from stdin, which keeps it out of your
    /// shell history. The link is fetched once to check it, unless
    /// --no-check.
    #[command(after_help = CALENDAR_LINK_HELP)]
    SetIcs {
        /// The link (https://, http:// or webcal://), or - to read it
        /// from stdin
        link: String,

        /// Save without fetching it first
        #[arg(long)]
        no_check: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Forget the iCal link and the events fetched from it
    Clear {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show where meeting titles come from, and the next event
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
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
        /// Agent: claude, codex or pi
        agent: String,
    },
    /// Stop sending meeting transcripts to an agent
    Revoke {
        /// Agent: claude, codex or pi
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
                action:
                    MeetingAction::Start {
                        diarization, title, ..
                    },
            }) => {
                assert_eq!(diarization.as_deref(), Some("ml"));
                assert_eq!(title.as_deref(), Some("standup"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn meeting_start_announce_flags() {
        let parse = |args: &[&str]| {
            let mut argv = vec!["voxtype", "meeting", "start"];
            argv.extend_from_slice(args);
            match Cli::parse_from(argv).command {
                Some(Commands::Meeting {
                    action:
                        MeetingAction::Start {
                            announce,
                            no_announce,
                            announcement_file,
                            ..
                        },
                }) => (announce, no_announce, announcement_file),
                _ => panic!("Expected Meeting Start command"),
            }
        };
        assert_eq!(parse(&[]), (false, false, None));
        assert_eq!(parse(&["--announce"]), (true, false, None));
        assert_eq!(parse(&["--no-announce"]), (false, true, None));
        // The last of the pair wins, so a wrapper's default can be overridden.
        assert_eq!(parse(&["--announce", "--no-announce"]), (false, true, None));
        assert_eq!(parse(&["--no-announce", "--announce"]), (true, false, None));
        assert_eq!(
            parse(&["--announce", "--announcement-file", "/tmp/de.wav"]),
            (true, false, Some("/tmp/de.wav".into()))
        );
    }

    #[test]
    fn meeting_start_takes_engine_model_language_and_url() {
        let cli = Cli::parse_from([
            "voxtype",
            "meeting",
            "start",
            "--title",
            "Planning",
            "--engine",
            "parakeet",
            "--model",
            "parakeet-ultra-Q8_0.gguf",
            "--language",
            "en",
            "--url",
            "https://meet.example.com/abc",
        ]);
        match cli.command {
            Some(Commands::Meeting {
                action:
                    MeetingAction::Start {
                        title,
                        engine,
                        model,
                        language,
                        url,
                        ..
                    },
            }) => {
                assert_eq!(title.as_deref(), Some("Planning"));
                assert_eq!(engine.as_deref(), Some("parakeet"));
                assert_eq!(model.as_deref(), Some("parakeet-ultra-Q8_0.gguf"));
                assert_eq!(language.as_deref(), Some("en"));
                assert_eq!(url.as_deref(), Some("https://meet.example.com/abc"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    /// The detection prompt's Record runs `meeting start --title T`, and the
    /// tray's Start Meeting runs it with no options: both still parse.
    #[test]
    fn meeting_start_still_takes_title_alone_and_nothing() {
        for args in [
            &["voxtype", "meeting", "start", "--title", "Standup"][..],
            &["voxtype", "meeting", "start"][..],
        ] {
            assert!(Cli::try_parse_from(args).is_ok(), "{args:?}");
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
                action:
                    MeetingAction::Start {
                        diarization, title, ..
                    },
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

    fn waveform(args: &[&str]) -> Result<MeetingAction, clap::Error> {
        let argv = ["voxtype", "meeting", "waveform"]
            .iter()
            .chain(args)
            .copied();
        match Cli::try_parse_from(argv)?.command {
            Some(Commands::Meeting { action }) => Ok(action),
            _ => panic!("Expected a meeting command"),
        }
    }

    #[test]
    fn meeting_waveform_takes_a_bucket_count() {
        match waveform(&["latest", "--json"]).unwrap() {
            MeetingAction::Waveform {
                meeting_id,
                buckets,
                json,
            } => assert_eq!((meeting_id.as_str(), buckets, json), ("latest", 1000, true)),
            _ => panic!("Expected Meeting Waveform"),
        }
        match waveform(&["abc1", "--buckets", "250"]).unwrap() {
            MeetingAction::Waveform { buckets, json, .. } => {
                assert_eq!((buckets, json), (250, false))
            }
            _ => panic!("Expected Meeting Waveform"),
        }
        assert!(waveform(&["latest", "--buckets", "0"]).is_err());
        assert!(waveform(&["latest", "--buckets", "100001"]).is_err());
        assert!(waveform(&[]).is_err(), "a meeting is required");
    }

    fn play(args: &[&str]) -> Result<MeetingAction, clap::Error> {
        let argv = ["voxtype", "meeting", "play"].iter().chain(args).copied();
        match Cli::try_parse_from(argv)?.command {
            Some(Commands::Meeting { action }) => Ok(action),
            _ => panic!("Expected a meeting command"),
        }
    }

    #[test]
    fn meeting_play_parses_its_start_points() {
        match play(&["latest", "--segment", "28", "--json"]).unwrap() {
            MeetingAction::Play {
                meeting_id,
                segment,
                at,
                track,
                stop,
                status,
                json,
            } => {
                assert_eq!(meeting_id.as_deref(), Some("latest"));
                assert_eq!((segment, at), (Some(28), None));
                assert_eq!(track, "mixed");
                assert!(!stop && !status && json);
            }
            _ => panic!("Expected Meeting Play"),
        }
        match play(&["abc1", "--at", "12:40", "--track", "loopback"]).unwrap() {
            MeetingAction::Play { at, track, .. } => {
                assert_eq!(at.as_deref(), Some("12:40"));
                assert_eq!(track, "loopback");
            }
            _ => panic!("Expected Meeting Play"),
        }
    }

    #[test]
    fn meeting_play_rejects_conflicting_flags() {
        assert!(play(&[]).is_err(), "a meeting is required to play");
        assert!(play(&["--stop"]).is_ok());
        assert!(play(&["--status", "--json"]).is_ok());
        assert!(play(&["latest", "--segment", "1", "--at", "5"]).is_err());
        assert!(play(&["latest", "--stop"]).is_err());
        assert!(play(&["--stop", "--status"]).is_err());
        assert!(play(&["latest", "--track", "speakers"]).is_err());
    }

    fn export_format(format: &str) -> Result<String, clap::Error> {
        let argv = ["voxtype", "meeting", "export", "latest", "--format", format];
        match Cli::try_parse_from(argv)?.command {
            Some(Commands::Meeting {
                action: MeetingAction::Export { format, .. },
            }) => Ok(format),
            _ => panic!("Expected Meeting Export"),
        }
    }

    fn meeting(args: &[&str]) -> Result<MeetingAction, clap::Error> {
        let argv = ["voxtype", "meeting"].iter().chain(args).copied();
        match Cli::try_parse_from(argv)?.command {
            Some(Commands::Meeting { action }) => Ok(action),
            _ => panic!("Expected a meeting command"),
        }
    }

    #[test]
    fn meeting_export_takes_every_format_and_its_alias() {
        // Accepted as given; `ExportFormat::parse` resolves aliases and case
        // (pinned against this list in src/meeting/export/mod.rs).
        for given in [
            "markdown", "md", "text", "txt", "json", "srt", "vtt", "SRT", "Md",
        ] {
            let format = export_format(given).unwrap_or_else(|e| panic!("--format {given}: {e}"));
            assert!(
                format.eq_ignore_ascii_case(given),
                "--format {given} gave {format}"
            );
        }
        let cli = Cli::try_parse_from(["voxtype", "meeting", "export", "latest"]).unwrap();
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Export { format, .. },
            }) => assert_eq!(format, "markdown"),
            _ => panic!("Expected Meeting Export"),
        }
    }

    #[test]
    fn meeting_export_refuses_an_unknown_format_with_the_list() {
        let err = export_format("bogus").unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
        let text = err.to_string();
        for name in ["markdown", "text", "json", "srt", "vtt"] {
            assert!(text.contains(name), "{name} missing from: {text}");
        }
    }

    #[test]
    fn meeting_export_help_lists_every_format() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let export = cmd
            .find_subcommand_mut("meeting")
            .and_then(|m| m.find_subcommand_mut("export"))
            .expect("meeting export");
        let help = export.render_long_help().to_string();
        assert!(
            help.contains("[possible values: markdown, text, json, srt, vtt]"),
            "{help}"
        );
    }

    #[test]
    fn meeting_import_and_retranscribe_parse() {
        match meeting(&[
            "import",
            "call.mp3",
            "--title",
            "Call",
            "--channels",
            "mic,loopback",
            "--date",
            "2026-11-02T15:00:00Z",
            "--model",
            "large-v3-turbo",
            "--json",
        ])
        .unwrap()
        {
            MeetingAction::Import {
                file,
                title,
                channels,
                date,
                model,
                json,
                ..
            } => {
                assert_eq!(file, std::path::PathBuf::from("call.mp3"));
                assert_eq!(title.as_deref(), Some("Call"));
                assert_eq!(channels.as_deref(), Some("mic,loopback"));
                assert_eq!(date.as_deref(), Some("2026-11-02T15:00:00Z"));
                assert_eq!(model.as_deref(), Some("large-v3-turbo"));
                assert!(json);
            }
            _ => panic!("Expected Meeting Import"),
        }
        match meeting(&[
            "retranscribe",
            "latest",
            "--engine",
            "parakeet",
            "--discard-edits",
        ])
        .unwrap()
        {
            MeetingAction::Retranscribe {
                meeting_id,
                engine,
                keep_edits,
                discard_edits,
                ..
            } => {
                assert_eq!(meeting_id, "latest");
                assert_eq!(engine.as_deref(), Some("parakeet"));
                assert!(!keep_edits && discard_edits);
            }
            _ => panic!("Expected Meeting Retranscribe"),
        }
        assert!(meeting(&["import"]).is_err(), "a file is required");
        assert!(meeting(&["retranscribe", "latest", "--keep-edits", "--discard-edits"]).is_err());
        assert!(meeting(&["import", "a.mp3", "--diarization", "magic"]).is_err());
    }
}
