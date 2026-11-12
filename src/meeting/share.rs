//! `voxtype meeting share`: a meeting's summary and notes, and optionally
//! its transcript, sent somewhere on purpose.
//!
//! Sharing is always something the user asks for: nothing here runs on a
//! timer or after a meeting by itself. What leaves is exactly what
//! [`compose`] renders, which `--preview` prints without sending.
//!
//! Destinations:
//! - **clipboard**: `wl-copy`, as Markdown or text, or as `text/html`
//! - **file**: a new file in `[meeting.share] folder` (Documents/Voxtype)
//! - **email**: a `mailto:` draft through `xdg-open`, subject and body
//!   filled in. A transcript is too long for a mail link, so it is saved as
//!   a file for the user to attach.
//! - **command**: `[meeting.share] command`, given the share on stdin

use crate::meeting::assistant::prompt::ts;
use crate::meeting::data::MeetingData;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Longest `mailto:` link handed to a mail client. Clients cut or refuse
/// longer ones; past it the notes go in a file and the mail points to it.
pub const MAILTO_LIMIT: usize = 16_000;

/// How a share is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareFormat {
    Markdown,
    Text,
    Html,
}

impl ShareFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::Text),
            "html" => Some(Self::Html),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Html => "html",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Html => "html",
        }
    }
}

/// Where a share goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareTarget {
    Clipboard,
    File,
    Email,
    Command,
}

impl ShareTarget {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "clipboard" | "copy" => Some(Self::Clipboard),
            "file" | "save" => Some(Self::File),
            "email" | "mail" => Some(Self::Email),
            "command" | "cmd" => Some(Self::Command),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::File => "file",
            Self::Email => "email",
            Self::Command => "command",
        }
    }
}

/// One part of a share, rendered per format.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    Title(String),
    /// Label and value pairs under the title
    Facts(Vec<(&'static str, String)>),
    Heading(String),
    Paragraph(String),
    Bullets(Vec<String>),
    /// Action items: open checkboxes
    Checklist(Vec<String>),
    /// Transcript lines: timestamp, speaker, text
    Lines(Vec<(String, String, String)>),
}

/// What a share holds, ready to render.
#[derive(Debug, Clone)]
pub struct Share {
    pub meeting_id: String,
    pub title: String,
    /// The mail subject: the title and the meeting's date
    pub subject: String,
    /// The sections included, in order, by name
    pub sections: Vec<&'static str>,
    pub transcript_lines: usize,
    blocks: Vec<Block>,
}

/// Build the share of `meeting`: title, date, length, attendees, overview,
/// key points, decisions, action items with owners, chapters, the user's
/// notes and highlights, then the transcript when `with_transcript`.
/// `meeting` is as voxtype shows it: names applied, deleted lines gone,
/// edited lines corrected.
pub fn compose(meeting: &MeetingData, with_transcript: bool) -> Share {
    let m = &meeting.metadata;
    let title = m.display_title();
    let local = m.started_at.with_timezone(&chrono::Local);
    let mut blocks = vec![Block::Title(title.clone())];
    let mut sections = Vec::new();

    let mut facts = vec![("Date", local.format("%A, %B %-d, %Y at %H:%M").to_string())];
    let length_secs = m
        .duration_secs
        .unwrap_or_else(|| meeting.transcript.duration_ms() / 1000);
    if length_secs > 0 {
        facts.push(("Length", length(length_secs)));
    }
    let attendees = attendees(meeting);
    if !attendees.is_empty() {
        facts.push(("Attendees", attendees.join(", ")));
    }
    if let Some(url) = &m.url {
        facts.push(("Link", url.clone()));
    }
    blocks.push(Block::Facts(facts));

    if let Some(summary) = &m.summary {
        if !summary.summary.trim().is_empty() {
            sections.push("overview");
            blocks.push(Block::Heading("Overview".into()));
            blocks.push(Block::Paragraph(summary.summary.trim().to_string()));
        }
        if !summary.key_points.is_empty() {
            sections.push("key_points");
            blocks.push(Block::Heading("Key points".into()));
            blocks.push(Block::Bullets(summary.key_points.clone()));
        }
        if !summary.decisions.is_empty() {
            sections.push("decisions");
            blocks.push(Block::Heading("Decisions".into()));
            blocks.push(Block::Bullets(summary.decisions.clone()));
        }
        if !summary.action_items.is_empty() {
            sections.push("action_items");
            blocks.push(Block::Heading("Action items".into()));
            blocks.push(Block::Checklist(
                summary
                    .action_items
                    .iter()
                    .map(|item| {
                        let mut line = item.description.trim().to_string();
                        let extra: Vec<String> = item
                            .assignee
                            .iter()
                            .cloned()
                            .chain(item.due_date.iter().map(|d| format!("due {d}")))
                            .collect();
                        if !extra.is_empty() {
                            line.push_str(&format!(" ({})", extra.join(", ")));
                        }
                        line
                    })
                    .collect(),
            ));
        }
        if !summary.chapters.is_empty() {
            sections.push("chapters");
            blocks.push(Block::Heading("Chapters".into()));
            blocks.push(Block::Bullets(
                summary
                    .chapters
                    .iter()
                    .map(|c| format!("{} {}", ts(c.start_ms), c.title))
                    .collect(),
            ));
        }
    }

    if let Some(notes) = meeting.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        sections.push("notes");
        blocks.push(Block::Heading("Notes".into()));
        blocks.push(Block::Paragraph(notes.trim().to_string()));
    }

    let highlights: Vec<String> = meeting
        .transcript
        .segments
        .iter()
        .filter_map(|s| {
            let a = s.annotation.as_ref()?;
            let mut line = format!(
                "{} {}: {}",
                ts(s.start_ms),
                s.speaker_display(),
                s.text.trim()
            );
            if !a.text.trim().is_empty() {
                line.push_str(&format!(" (note: {})", a.text.trim()));
            }
            Some(line)
        })
        .collect();
    if !highlights.is_empty() {
        sections.push("highlights");
        blocks.push(Block::Heading("Highlights".into()));
        blocks.push(Block::Bullets(highlights));
    }

    let mut transcript_lines = 0;
    if with_transcript {
        let lines: Vec<(String, String, String)> = meeting
            .transcript
            .segments
            .iter()
            .filter(|s| !s.deleted && !s.text.trim().is_empty())
            .map(|s| {
                (
                    ts(s.start_ms),
                    s.speaker_display(),
                    s.text.trim().to_string(),
                )
            })
            .collect();
        transcript_lines = lines.len();
        sections.push("transcript");
        blocks.push(Block::Heading("Transcript".into()));
        blocks.push(Block::Lines(lines));
    }

    Share {
        meeting_id: m.id.to_string(),
        subject: format!("{} ({})", title, local.format("%b %-d, %Y")),
        title,
        sections,
        transcript_lines,
        blocks,
    }
}

/// "42 min", "1 h 5 min", "40 s".
fn length(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    match (h, m) {
        (0, 0) => format!("{secs} s"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The speakers in the order they first spoke.
fn attendees(meeting: &MeetingData) -> Vec<String> {
    let mut seen = Vec::new();
    for s in meeting.transcript.segments.iter().filter(|s| !s.deleted) {
        let name = s.speaker_display();
        if name != "Unknown" && !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

impl Share {
    /// The share as `format` writes it.
    pub fn render(&self, format: ShareFormat) -> String {
        match format {
            ShareFormat::Markdown => self.markdown(),
            ShareFormat::Text => self.text(),
            ShareFormat::Html => self.html(),
        }
    }

    fn markdown(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            match block {
                Block::Title(t) => out.push_str(&format!("# {t}\n\n")),
                Block::Facts(facts) => {
                    for (label, value) in facts {
                        out.push_str(&format!("**{label}:** {value}  \n"));
                    }
                    out.push('\n');
                }
                Block::Heading(h) => out.push_str(&format!("## {h}\n\n")),
                Block::Paragraph(p) => out.push_str(&format!("{p}\n\n")),
                Block::Bullets(items) => {
                    for i in items {
                        out.push_str(&format!("- {i}\n"));
                    }
                    out.push('\n');
                }
                Block::Checklist(items) => {
                    for i in items {
                        out.push_str(&format!("- [ ] {i}\n"));
                    }
                    out.push('\n');
                }
                Block::Lines(lines) => {
                    for (t, who, text) in lines {
                        out.push_str(&format!("**[{t}] {who}:** {text}  \n"));
                    }
                    out.push('\n');
                }
            }
        }
        out.trim_end().to_string() + "\n"
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            match block {
                Block::Title(t) => {
                    out.push_str(&format!("{t}\n{}\n\n", "=".repeat(t.chars().count())))
                }
                Block::Facts(facts) => {
                    for (label, value) in facts {
                        out.push_str(&format!("{label}: {value}\n"));
                    }
                    out.push('\n');
                }
                Block::Heading(h) => {
                    out.push_str(&format!("{h}\n{}\n\n", "-".repeat(h.chars().count())))
                }
                Block::Paragraph(p) => out.push_str(&format!("{p}\n\n")),
                Block::Bullets(items) => {
                    for i in items {
                        out.push_str(&format!("- {i}\n"));
                    }
                    out.push('\n');
                }
                Block::Checklist(items) => {
                    for i in items {
                        out.push_str(&format!("[ ] {i}\n"));
                    }
                    out.push('\n');
                }
                Block::Lines(lines) => {
                    for (t, who, text) in lines {
                        out.push_str(&format!("[{t}] {who}: {text}\n"));
                    }
                    out.push('\n');
                }
            }
        }
        out.trim_end().to_string() + "\n"
    }

    fn html(&self) -> String {
        let e = escape_html;
        let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
        out.push_str(&format!(
            "<title>{}</title></head>\n<body>\n",
            e(&self.title)
        ));
        for block in &self.blocks {
            match block {
                Block::Title(t) => out.push_str(&format!("<h1>{}</h1>\n", e(t))),
                Block::Facts(facts) => {
                    out.push_str("<p>");
                    let rows: Vec<String> = facts
                        .iter()
                        .map(|(l, v)| format!("<strong>{}:</strong> {}", e(l), e(v)))
                        .collect();
                    out.push_str(&rows.join("<br>\n"));
                    out.push_str("</p>\n");
                }
                Block::Heading(h) => out.push_str(&format!("<h2>{}</h2>\n", e(h))),
                Block::Paragraph(p) => {
                    for para in p.split("\n\n") {
                        out.push_str(&format!("<p>{}</p>\n", e(para).replace('\n', "<br>\n")));
                    }
                }
                Block::Bullets(items) => {
                    out.push_str("<ul>\n");
                    for i in items {
                        out.push_str(&format!("<li>{}</li>\n", e(i)));
                    }
                    out.push_str("</ul>\n");
                }
                Block::Checklist(items) => {
                    out.push_str("<ul>\n");
                    for i in items {
                        out.push_str(&format!("<li>&#9744; {}</li>\n", e(i)));
                    }
                    out.push_str("</ul>\n");
                }
                Block::Lines(lines) => {
                    for (t, who, text) in lines {
                        out.push_str(&format!(
                            "<p><strong>[{}] {}:</strong> {}</p>\n",
                            e(t),
                            e(who),
                            e(text)
                        ));
                    }
                }
            }
        }
        out.push_str("</body></html>\n");
        out
    }

    /// This share without its transcript, for a mail body.
    pub fn without_transcript(&self) -> Share {
        let mut share = self.clone();
        if let Some(at) = share
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Heading(h) if h == "Transcript"))
        {
            share.blocks.truncate(at);
        }
        share.sections.retain(|s| *s != "transcript");
        share.transcript_lines = 0;
        share
    }

    /// The overview paragraph alone, if the meeting has one.
    fn overview(&self) -> Option<&str> {
        self.blocks.windows(2).find_map(|w| match w {
            [Block::Heading(h), Block::Paragraph(p)] if h == "Overview" => Some(p.as_str()),
            _ => None,
        })
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Percent-encode for a `mailto:` header value (RFC 6068): everything but
/// unreserved characters, with line breaks as CRLF.
pub fn mailto_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for line_break_free in s.replace("\r\n", "\n").split_inclusive('\n') {
        for b in line_break_free.bytes() {
            match b {
                b'\n' => out.push_str("%0D%0A"),
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    out.push(b as char)
                }
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    out
}

/// A `mailto:` link with `subject` and `body`, no recipient.
pub fn mailto(subject: &str, body: &str) -> String {
    format!(
        "mailto:?subject={}&body={}",
        mailto_encode(subject),
        mailto_encode(body)
    )
}

/// The folder `--to file` writes into: `folder` unless it is "auto", else
/// the XDG documents folder's (or ~/Documents') `Voxtype`.
pub fn share_folder(folder: &str) -> PathBuf {
    if folder.trim().is_empty() || folder == "auto" {
        dirs::document_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Documents")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Voxtype")
    } else if let Some(rest) = folder.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        PathBuf::from(folder)
    }
}

/// A new file in `dir` for `share`, named by date and title, never one
/// that exists: "2026-11-12 Launch sync.md", then "... 2.md".
pub fn new_file_path(dir: &Path, share: &Share, started: &str, ext: &str, suffix: &str) -> PathBuf {
    let safe: String = share
        .title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let safe = safe.trim().trim_start_matches('.').to_string();
    let base = if safe.is_empty() {
        format!("{started} meeting{suffix}")
    } else {
        format!("{started} {safe}{suffix}")
    };
    let mut path = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    path
}

/// The programs a share runs, so tests can stand in for them.
#[derive(Debug, Clone)]
pub struct Tools {
    pub wl_copy: PathBuf,
    pub xdg_open: PathBuf,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            wl_copy: "wl-copy".into(),
            xdg_open: "xdg-open".into(),
        }
    }
}

impl Tools {
    /// Which destinations can work here: the clipboard and email need
    /// their programs, the command a configured one.
    pub fn available(&self, command: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "clipboard": on_path(&self.wl_copy),
            "file": true,
            "email": on_path(&self.xdg_open),
            "command": command.is_some_and(|c| !c.trim().is_empty()),
        })
    }

    /// For each destination that cannot work here, one line saying what
    /// to do about it, so a menu can say why an item is greyed out.
    pub fn why(&self, command: Option<&str>) -> serde_json::Value {
        let mut why = serde_json::Map::new();
        if !on_path(&self.wl_copy) {
            why.insert(
                "clipboard".into(),
                missing(&self.wl_copy, "Install wl-clipboard.")
                    .to_string()
                    .into(),
            );
        }
        if !on_path(&self.xdg_open) {
            why.insert(
                "email".into(),
                missing(&self.xdg_open, "Install xdg-utils.")
                    .to_string()
                    .into(),
            );
        }
        if !command.is_some_and(|c| !c.trim().is_empty()) {
            why.insert(
                "command".into(),
                "No share command is set. Set one with: voxtype config set meeting.share.command \"<command>\"".into(),
            );
        }
        why.into()
    }
}

fn on_path(program: &Path) -> bool {
    if program.components().count() > 1 {
        return program.is_file();
    }
    which::which(program).is_ok()
}

/// What a share asks for, beyond the meeting.
#[derive(Debug, Clone)]
pub struct ShareRequest {
    pub target: ShareTarget,
    pub format: ShareFormat,
    pub with_transcript: bool,
    /// `--output`: a file or folder for `file`, overriding the folder
    pub output: Option<PathBuf>,
    /// `[meeting.share] folder`
    pub folder: String,
    /// `--command` or `[meeting.share] command`
    pub command: Option<String>,
    pub timeout: Duration,
}

/// What a share did.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ShareOutcome {
    pub meeting_id: String,
    pub to: String,
    pub format: String,
    pub transcript: bool,
    pub sections: Vec<&'static str>,
    pub chars: usize,
    /// The file written: the share itself for `file`, or what the mail
    /// draft asks the user to attach
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// How the share command ended, for `command`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_status: Option<String>,
}

/// Share errors, each saying what to do.
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error("{program} is not installed. {hint}")]
    Missing { program: String, hint: &'static str },
    #[error("{program} failed: {detail}")]
    Failed { program: String, detail: String },
    #[error(
        "No share command is set. Set one with:\n  voxtype config set meeting.share.command \"<command>\"\nor pass --command."
    )]
    NoCommand,
    #[error("The share command ran past {0}s and was stopped")]
    TimedOut(u64),
    #[error("Could not write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

/// Send `share` where `request` says, as it says.
pub async fn send(
    meeting: &MeetingData,
    share: &Share,
    request: &ShareRequest,
    tools: &Tools,
) -> Result<ShareOutcome, ShareError> {
    let started = meeting
        .metadata
        .started_at
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string();
    let mut outcome = ShareOutcome {
        meeting_id: share.meeting_id.clone(),
        to: request.target.name().to_string(),
        format: request.format.name().to_string(),
        transcript: request.with_transcript,
        sections: share.sections.clone(),
        ..Default::default()
    };
    match request.target {
        ShareTarget::Clipboard => {
            let body = share.render(request.format);
            outcome.chars = body.chars().count();
            let mut args: Vec<&str> = Vec::new();
            if request.format == ShareFormat::Html {
                args.extend(["--type", "text/html"]);
            }
            pipe_to(&tools.wl_copy, &args, &body, "Install wl-clipboard.")?;
        }
        ShareTarget::File => {
            let body = share.render(request.format);
            outcome.chars = body.chars().count();
            let path = match &request.output {
                Some(p) if !p.is_dir() => p.clone(),
                Some(dir) => new_file_path(dir, share, &started, request.format.extension(), ""),
                None => {
                    let dir = share_folder(&request.folder);
                    new_file_path(&dir, share, &started, request.format.extension(), "")
                }
            };
            write_new(&path, &body)?;
            outcome.path = Some(path);
        }
        ShareTarget::Email => {
            // Mail links carry plain text; the transcript goes in a file.
            let notes = share.without_transcript();
            let mut body = notes.render(ShareFormat::Text);
            let dir = share_folder(&request.folder);
            if request.with_transcript {
                let path = new_file_path(&dir, share, &started, "md", " transcript");
                write_new(&path, &share.render(ShareFormat::Markdown))?;
                body.push_str(&format!(
                    "\nThe full transcript is in {}. Attach it before sending.\n",
                    path.display()
                ));
                outcome.path = Some(path);
            }
            let mut link = mailto(&share.subject, &body);
            if link.len() > MAILTO_LIMIT {
                // Too long for a mail link: the notes go in a file too.
                let path = new_file_path(&dir, share, &started, "md", "");
                write_new(&path, &share.render(ShareFormat::Markdown))?;
                body = format!(
                    "{}\n\nThe full notes{} are in {}. Attach it before sending.\n",
                    notes.overview().unwrap_or(&share.title),
                    if request.with_transcript {
                        " and transcript"
                    } else {
                        ""
                    },
                    path.display()
                );
                link = mailto(&share.subject, &body);
                outcome.path = Some(path);
            }
            outcome.chars = body.chars().count();
            run(&tools.xdg_open, &[&link], "Install xdg-utils.")?;
        }
        ShareTarget::Command => {
            let command = request
                .command
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .ok_or(ShareError::NoCommand)?;
            let body = share.render(request.format);
            outcome.chars = body.chars().count();
            let status = run_command(command, meeting, share, request, &body).await?;
            outcome.command_status = Some(status);
        }
    }
    Ok(outcome)
}

/// Write `body` to `path`, a file that must not exist yet.
fn write_new(path: &Path, body: &str) -> Result<(), ShareError> {
    let err = |source| ShareError::Write {
        path: path.display().to_string(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(body.as_bytes()).map_err(err)
}

fn missing(program: &Path, hint: &'static str) -> ShareError {
    ShareError::Missing {
        program: program.display().to_string(),
        hint,
    }
}

/// Run `program` with `body` on its stdin.
fn pipe_to(
    program: &Path,
    args: &[&str],
    body: &str,
    hint: &'static str,
) -> Result<(), ShareError> {
    use std::io::Write;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| missing(program, hint))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    finish(program, child.wait_with_output())
}

fn run(program: &Path, args: &[&str], hint: &'static str) -> Result<(), ShareError> {
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| missing(program, hint))?;
    finish(program, child.wait_with_output())
}

fn finish(program: &Path, output: std::io::Result<std::process::Output>) -> Result<(), ShareError> {
    let failed = |detail: String| ShareError::Failed {
        program: program.display().to_string(),
        detail,
    };
    let output = output.map_err(|e| failed(e.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(failed(if stderr.is_empty() {
            output.status.to_string()
        } else {
            stderr
        }))
    }
}

/// Run the share command through `sh -c`, the share on its stdin and in
/// `VOXTYPE_SHARE_FILE`, in its own process group so a timeout stops
/// everything it started.
async fn run_command(
    command: &str,
    meeting: &MeetingData,
    share: &Share,
    request: &ShareRequest,
    body: &str,
) -> Result<String, ShareError> {
    let staged = tempfile::Builder::new()
        .prefix("voxtype-share-")
        .suffix(&format!(".{}", request.format.extension()))
        .tempfile()
        .map_err(|source| ShareError::Write {
            path: std::env::temp_dir().display().to_string(),
            source,
        })?;
    std::fs::write(staged.path(), body).map_err(|source| ShareError::Write {
        path: staged.path().display().to_string(),
        source,
    })?;
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .env("VOXTYPE_MEETING_ID", &share.meeting_id)
        .env("VOXTYPE_MEETING_TITLE", &share.title)
        .env("VOXTYPE_SHARE_FORMAT", request.format.name())
        .env("VOXTYPE_SHARE_FILE", staged.path())
        .env(
            "VOXTYPE_SHARE_TRANSCRIPT",
            if request.with_transcript { "1" } else { "0" },
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = &meeting.metadata.storage_path {
        cmd.env("VOXTYPE_MEETING_DIR", dir);
    }
    let mut child = cmd.spawn().map_err(|e| ShareError::Failed {
        program: "sh".into(),
        detail: e.to_string(),
    })?;
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        let body = body.to_string();
        tokio::spawn(async move {
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, body.as_bytes()).await;
        });
    }
    match tokio::time::timeout(request.timeout, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(ShareError::Failed {
                program: "The share command".into(),
                detail: if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                },
            })
        }
        Ok(Err(e)) => Err(ShareError::Failed {
            program: "The share command".into(),
            detail: e.to_string(),
        }),
        Err(_) => {
            if let Some(pid) = pid {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            Err(ShareError::TimedOut(request.timeout.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{
        ActionItem, AudioSource, Chapter, MeetingMetadata, MeetingSummary, SegmentAnnotation,
        Transcript, TranscriptSegment,
    };

    fn meeting() -> MeetingData {
        let mut metadata = MeetingMetadata::new(Some("Launch <sync> & plan".into()));
        metadata.started_at = "2026-11-12T15:00:00Z".parse().unwrap();
        metadata.duration_secs = Some(42 * 60);
        metadata.summary = Some(MeetingSummary {
            summary: "We agreed to ship on Friday.".into(),
            key_points: vec!["Build is green".into()],
            action_items: vec![
                ActionItem {
                    description: "Tag the release".into(),
                    assignee: Some("Dana".into()),
                    due_date: Some("Friday".into()),
                    completed: false,
                },
                ActionItem {
                    description: "Write the notes".into(),
                    assignee: None,
                    due_date: None,
                    completed: false,
                },
            ],
            decisions: vec!["Ship Friday".into()],
            chapters: vec![Chapter {
                start_ms: 65_000,
                title: "Release".into(),
            }],
            generated_at: chrono::Utc::now(),
            model: None,
        });
        let mut transcript = Transcript::new();
        for (id, who, src, text) in [
            (0, "Pete", AudioSource::Microphone, "Is the build green?"),
            (1, "Dana", AudioSource::Loopback, "It is, ship it."),
            (2, "Pete", AudioSource::Microphone, "Great."),
        ] {
            let mut s = TranscriptSegment::new(
                id,
                id as u64 * 5000,
                id as u64 * 5000 + 4000,
                text.into(),
                0,
            );
            s.source = src;
            s.speaker_label = Some(who.into());
            transcript.add_segment(s);
        }
        transcript.segments[1].annotation = Some(SegmentAnnotation {
            text: "the decision".into(),
            highlight: true,
        });
        MeetingData {
            metadata,
            transcript,
            notes: Some("Remember the changelog.\n".into()),
        }
    }

    #[test]
    fn a_share_holds_the_summary_notes_and_highlights_but_no_transcript() {
        let share = compose(&meeting(), false);
        assert_eq!(
            share.sections,
            [
                "overview",
                "key_points",
                "decisions",
                "action_items",
                "chapters",
                "notes",
                "highlights"
            ]
        );
        let md = share.render(ShareFormat::Markdown);
        assert!(md.starts_with("# Launch <sync> & plan\n"), "{md}");
        assert!(md.contains("**Length:** 42 min"), "{md}");
        assert!(md.contains("**Attendees:** Pete, Dana"), "{md}");
        assert!(md.contains("## Overview\n\nWe agreed to ship on Friday."));
        assert!(md.contains("- [ ] Tag the release (Dana, due Friday)"));
        assert!(md.contains("- [ ] Write the notes\n"));
        assert!(md.contains("- 01:05 Release"));
        assert!(md.contains("## Notes\n\nRemember the changelog."));
        assert!(md.contains("- 00:05 Dana: It is, ship it. (note: the decision)"));
        assert!(!md.contains("Transcript"));
        assert!(!md.contains("Is the build green?"));
        assert_eq!(share.transcript_lines, 0);
    }

    #[test]
    fn with_the_transcript_every_shown_line_follows() {
        let mut m = meeting();
        m.transcript.segments[2].deleted = true;
        let share = compose(&m, true);
        assert_eq!(share.sections.last(), Some(&"transcript"));
        assert_eq!(share.transcript_lines, 2);
        let md = share.render(ShareFormat::Markdown);
        assert!(md.contains("## Transcript\n\n**[00:00] Pete:** Is the build green?  \n**[00:05] Dana:** It is, ship it."), "{md}");
        assert!(!md.contains("Great."));
        let txt = share.render(ShareFormat::Text);
        assert!(txt.contains("Transcript\n----------\n\n[00:00] Pete: Is the build green?\n"));
        assert!(txt.starts_with("Launch <sync> & plan\n====================\n"));
        assert!(txt.contains("[ ] Tag the release (Dana, due Friday)"));
        // The mail body leaves it out.
        let notes = share.without_transcript();
        assert!(!notes
            .render(ShareFormat::Text)
            .contains("Is the build green?"));
        assert!(!notes.sections.contains(&"transcript"));
    }

    #[test]
    fn html_escapes_everything_it_is_given() {
        let html = compose(&meeting(), true).render(ShareFormat::Html);
        assert!(
            html.contains("<h1>Launch &lt;sync&gt; &amp; plan</h1>"),
            "{html}"
        );
        assert!(html.contains("<title>Launch &lt;sync&gt; &amp; plan</title>"));
        assert!(html.contains("<li>&#9744; Tag the release (Dana, due Friday)</li>"));
        assert!(html.contains("<p><strong>[00:05] Dana:</strong> It is, ship it.</p>"));
        assert!(!html.contains("<sync>"));
    }

    #[test]
    fn a_meeting_without_a_summary_shares_what_it_has() {
        let mut m = meeting();
        m.metadata.summary = None;
        m.notes = None;
        m.transcript.segments[1].annotation = None;
        let share = compose(&m, false);
        assert!(share.sections.is_empty());
        let md = share.render(ShareFormat::Markdown);
        assert!(md.starts_with("# Launch"));
        assert!(md.contains("**Date:**"));
    }

    #[test]
    fn mailto_encodes_per_rfc_6068() {
        assert_eq!(
            mailto_encode("a b&c=d\n\u{e9}"),
            "a%20b%26c%3Dd%0D%0A%C3%A9"
        );
        assert_eq!(mailto_encode("x\r\ny"), "x%0D%0Ay");
        assert_eq!(mailto("Hi", "There"), "mailto:?subject=Hi&body=There");
        let share = compose(&meeting(), false);
        assert!(share.subject.starts_with("Launch <sync> & plan ("));
    }

    #[test]
    fn files_are_named_by_date_and_title_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let share = compose(&meeting(), false);
        let first = new_file_path(dir.path(), &share, "2026-11-12", "md", "");
        assert_eq!(
            first.file_name().unwrap(),
            "2026-11-12 Launch -sync- & plan.md"
        );
        std::fs::write(&first, "x").unwrap();
        let second = new_file_path(dir.path(), &share, "2026-11-12", "md", "");
        assert_eq!(
            second.file_name().unwrap(),
            "2026-11-12 Launch -sync- & plan 2.md"
        );
        assert_eq!(share_folder("/srv/notes"), PathBuf::from("/srv/notes"));
        assert!(share_folder("auto").ends_with("Voxtype"));
    }

    fn request(target: ShareTarget, dir: &Path) -> ShareRequest {
        ShareRequest {
            target,
            format: ShareFormat::Markdown,
            with_transcript: false,
            output: None,
            folder: dir.display().to_string(),
            command: None,
            timeout: Duration::from_secs(10),
        }
    }

    /// Stand-ins that record their arguments and stdin.
    fn tools(dir: &Path) -> Tools {
        let script = |name: &str| {
            let path = dir.join(name);
            crate::test_support::write_script(
                &path,
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > {d}/{name}.args\ncat > {d}/{name}.stdin\n",
                    d = dir.display()
                ),
            );
            path
        };
        Tools {
            wl_copy: script("wl-copy"),
            xdg_open: script("xdg-open"),
        }
    }

    #[tokio::test]
    async fn the_clipboard_gets_exactly_the_rendered_share() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(dir.path());
        let m = meeting();
        let share = compose(&m, false);
        let out = send(
            &m,
            &share,
            &request(ShareTarget::Clipboard, dir.path()),
            &tools,
        )
        .await
        .unwrap();
        let copied = std::fs::read_to_string(dir.path().join("wl-copy.stdin")).unwrap();
        assert_eq!(copied, share.render(ShareFormat::Markdown));
        assert_eq!(out.chars, copied.chars().count());
        assert_eq!(out.to, "clipboard");

        let mut html = request(ShareTarget::Clipboard, dir.path());
        html.format = ShareFormat::Html;
        send(&m, &share, &html, &tools).await.unwrap();
        let args = std::fs::read_to_string(dir.path().join("wl-copy.args")).unwrap();
        assert_eq!(args, "--type\ntext/html\n");
    }

    #[tokio::test]
    async fn a_file_share_is_a_new_file_in_the_share_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Voxtype");
        let m = meeting();
        let share = compose(&m, true);
        let mut req = request(ShareTarget::File, &folder);
        req.with_transcript = true;
        let out = send(&m, &share, &req, &tools(dir.path())).await.unwrap();
        let path = out.path.unwrap();
        assert!(path.starts_with(&folder));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            share.render(ShareFormat::Markdown)
        );
        let again = send(&m, &share, &req, &tools(dir.path())).await.unwrap();
        assert_ne!(again.path.unwrap(), path, "never overwrites");

        // --output naming a file writes exactly there.
        req.output = Some(dir.path().join("out.md"));
        let out = send(&m, &share, &req, &tools(dir.path())).await.unwrap();
        assert_eq!(out.path.unwrap(), dir.path().join("out.md"));
    }

    #[tokio::test]
    async fn email_opens_a_draft_and_saves_the_transcript_to_attach() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(dir.path());
        let m = meeting();
        let share = compose(&m, true);
        let mut req = request(ShareTarget::Email, &dir.path().join("Voxtype"));
        req.with_transcript = true;
        let out = send(&m, &share, &req, &tools).await.unwrap();
        let link = std::fs::read_to_string(dir.path().join("xdg-open.args")).unwrap();
        assert!(
            link.starts_with("mailto:?subject=Launch%20%3Csync%3E%20%26%20plan%20%28"),
            "{link}"
        );
        assert!(link.contains("We%20agreed%20to%20ship%20on%20Friday."));
        assert!(
            !link.contains("Is%20the%20build%20green"),
            "no transcript inline"
        );
        let attach = out.path.unwrap();
        assert!(link.contains(&mailto_encode(&attach.display().to_string())));
        assert!(std::fs::read_to_string(&attach)
            .unwrap()
            .contains("Is the build green?"));
    }

    #[tokio::test]
    async fn an_overlong_mail_points_at_a_saved_file() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(dir.path());
        let mut m = meeting();
        m.notes = Some("word ".repeat(5000));
        let share = compose(&m, false);
        let out = send(&m, &share, &request(ShareTarget::Email, dir.path()), &tools)
            .await
            .unwrap();
        let link = std::fs::read_to_string(dir.path().join("xdg-open.args")).unwrap();
        assert!(link.len() < MAILTO_LIMIT, "{}", link.len());
        assert!(link.contains("We%20agreed"));
        assert!(std::fs::read_to_string(out.path.unwrap())
            .unwrap()
            .contains("word word"));
    }

    #[tokio::test]
    async fn the_command_gets_the_share_on_stdin_and_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let m = meeting();
        let share = compose(&m, false);
        let mut req = request(ShareTarget::Command, dir.path());
        assert!(matches!(
            send(&m, &share, &req, &tools(dir.path())).await,
            Err(ShareError::NoCommand)
        ));
        let d = dir.path().display();
        req.command = Some(format!(
            "cat > {d}/stdin; cp \"$VOXTYPE_SHARE_FILE\" {d}/file; \
             echo \"$VOXTYPE_MEETING_TITLE|$VOXTYPE_SHARE_FORMAT|$VOXTYPE_SHARE_TRANSCRIPT\" > {d}/env; echo posted"
        ));
        let out = send(&m, &share, &req, &tools(dir.path())).await.unwrap();
        let body = share.render(ShareFormat::Markdown);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stdin")).unwrap(),
            body
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file")).unwrap(),
            body
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("env")).unwrap(),
            "Launch <sync> & plan|markdown|0\n"
        );
        assert_eq!(out.command_status.as_deref(), Some("posted"));

        req.command = Some("echo nope >&2; exit 3".into());
        let err = send(&m, &share, &req, &tools(dir.path()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");

        req.command = Some("sleep 30".into());
        req.timeout = Duration::from_millis(200);
        let err = send(&m, &share, &req, &tools(dir.path()))
            .await
            .unwrap_err();
        assert!(matches!(err, ShareError::TimedOut(_)), "{err}");
    }

    #[test]
    fn missing_programs_say_what_to_install() {
        let m = meeting();
        let share = compose(&m, false);
        let tools = Tools {
            wl_copy: "/nonexistent/wl-copy".into(),
            xdg_open: "/nonexistent/xdg-open".into(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = rt
            .block_on(send(
                &m,
                &share,
                &request(ShareTarget::Clipboard, dir.path()),
                &tools,
            ))
            .unwrap_err();
        assert!(err.to_string().contains("Install wl-clipboard"), "{err}");
        let available = tools.available(None);
        assert_eq!(available["clipboard"], false);
        assert_eq!(available["file"], true);
        assert_eq!(available["command"], false);
        assert_eq!(tools.available(Some("cat"))["command"], true);
        let why = tools.why(None);
        assert_eq!(
            why["clipboard"],
            "/nonexistent/wl-copy is not installed. Install wl-clipboard."
        );
        assert!(why["email"].as_str().unwrap().contains("xdg-utils"));
        assert!(why["command"]
            .as_str()
            .unwrap()
            .contains("voxtype config set meeting.share.command"));
        assert!(why.get("file").is_none(), "only what is unavailable");
        assert!(tools.why(Some("cat")).get("command").is_none());
    }
}
