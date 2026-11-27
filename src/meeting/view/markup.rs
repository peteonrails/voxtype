//! Pango markup for views that draw with GTK: transcript lines and the
//! summary. Plain strings, so they are tested without a toolkit.

use super::{clock, LineMarks, MeetingView};
use crate::meeting::data::TranscriptSegment;
use std::time::Duration;

/// Speaker colors, by chip index, for views that color names. Picked to
/// read on light and dark backgrounds.
pub const SPEAKER_COLORS: [&str; 8] = [
    "#1c71d8", "#c061cb", "#e5a50a", "#2ec27e", "#3584e4", "#e01b24", "#26a269", "#9141ac",
];

/// Escape text for Pango markup.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
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

/// One transcript line as plain text, for the clipboard: its time, speaker
/// and text on two lines, as `meeting export --format text --timestamps
/// --speakers` writes each line.
pub fn line_text(segment: &TranscriptSegment) -> String {
    format!(
        "[{}] {}:\n{}",
        clock(Duration::from_millis(segment.start_ms)),
        segment.speaker_display(),
        segment.text.trim()
    )
}

/// One transcript line as Pango markup: dimmed time, the speaker's name in
/// their color, the text, and the annotation on a second line. A moved line
/// names the speaker the diarizer gave it, an edited one says so, a
/// highlighted one is marked, and `playing` puts ▶ before the line being
/// heard.
pub fn line_markup(view: &MeetingView, segment: &TranscriptSegment, playing: bool) -> String {
    let color = view
        .speaker_index(segment)
        .map(|i| SPEAKER_COLORS[i % SPEAKER_COLORS.len()])
        .unwrap_or("#808080");
    let marks = LineMarks::of(segment);
    let mut m = String::new();
    if playing {
        m.push_str("<span foreground=\"#26a269\"><b>▶</b></span> ");
    }
    m.push_str(&format!(
        "<span alpha=\"60%\">[{}]</span> <b><span foreground=\"{}\">{}</span></b>",
        clock(Duration::from_millis(segment.start_ms)),
        color,
        escape_markup(&segment.speaker_display()),
    ));
    if let Some(from) = segment.original_speaker.as_deref() {
        m.push_str(&format!(
            " <span alpha=\"60%\">(was {})</span>",
            escape_markup(from)
        ));
    }
    let text = escape_markup(segment.text.trim());
    if marks.highlighted {
        m.push_str(&format!(
            ": <span background=\"#f6d32d\" bgalpha=\"35%\">{text}</span>"
        ));
    } else {
        m.push_str(&format!(": {text}"));
    }
    if marks.edited {
        m.push_str(" <i><span alpha=\"60%\">(edited)</span></i>");
    }
    if let Some(a) = &segment.annotation {
        m.push_str(&format!(
            "\n<i><span foreground=\"#c64600\">✎ {}</span></i>",
            escape_markup(&crate::meeting::export::annotation_line(a))
        ));
    }
    m
}

/// Inline Markdown (`**bold**`, `*italic*`, `` `code` ``) as Pango markup.
fn inline(text: &str) -> String {
    let escaped = escape_markup(text);
    let mut out = String::with_capacity(escaped.len());
    let mut rest = escaped.as_str();
    let pairs = [
        ("**", "<b>", "</b>"),
        ("`", "<tt>", "</tt>"),
        ("*", "<i>", "</i>"),
    ];
    'outer: while !rest.is_empty() {
        for (mark, open, close) in pairs {
            if let Some(after) = rest.strip_prefix(mark) {
                if let Some(end) = after.find(mark).filter(|&e| e > 0) {
                    out.push_str(open);
                    out.push_str(&after[..end]);
                    out.push_str(close);
                    rest = &after[end + mark.len()..];
                    continue 'outer;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A meeting's `summary.md` as Pango markup for a label: headings bold and
/// larger, list items with bullets or check boxes, inline emphasis kept.
pub fn summary_markup(markdown: &str) -> String {
    let mut lines = Vec::new();
    for raw in markdown.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let rendered = if let Some(h) = trimmed.strip_prefix("# ") {
            format!("<span size=\"large\"><b>{}</b></span>", inline(h))
        } else if let Some(h) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
        {
            format!("<b>{}</b>", inline(h))
        } else if let Some(item) = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("* [ ] "))
        {
            format!("{indent}☐ {}", inline(item))
        } else if let Some(item) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
        {
            format!("{indent}☑ {}", inline(item))
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            format!("{indent}• {}", inline(item))
        } else {
            inline(line)
        };
        lines.push(rendered);
    }
    // Collapse runs of blank lines.
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&l);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_with(segment: serde_json::Value) -> MeetingView {
        let mut v = MeetingView::live();
        v.apply_line(
            &serde_json::json!({
                "event": "hello", "status": "recording", "meeting_id": "m", "segments": [segment]
            })
            .to_string(),
        );
        v
    }

    #[test]
    fn markup_escapes_and_colors_by_speaker() {
        let v = view_with(serde_json::json!(
            {"id": 0, "start_ms": 61000, "end_ms": 62000, "text": "a < b & c", "source": "loopback",
             "speaker_id": "Remote", "speaker_label": "Dana <PM>", "chunk_id": 0,
             "annotation": {"text": "check", "highlight": false}}
        ));
        let m = line_markup(&v, &v.lines()[0], false);
        assert_eq!(
            m,
            "<span alpha=\"60%\">[01:01]</span> <b><span foreground=\"#1c71d8\">Dana &lt;PM&gt;</span></b>: a &lt; b &amp; c\n<i><span foreground=\"#c64600\">✎ Note: check</span></i>"
        );
    }

    #[test]
    fn a_line_copies_as_plain_text_without_marks() {
        let v = view_with(serde_json::json!(
            {"id": 0, "start_ms": 61000, "end_ms": 62000, "text": " a < b & c ", "source": "loopback",
             "speaker_id": "Remote", "speaker_label": "Dana", "original_speaker": "Remote 2",
             "chunk_id": 0, "annotation": {"text": "check", "highlight": true}}
        ));
        assert_eq!(line_text(&v.lines()[0]), "[01:01] Dana:\na < b & c");
    }

    #[test]
    fn markup_marks_moved_edited_highlighted_and_playing_lines() {
        let v = view_with(serde_json::json!(
            {"id": 0, "start_ms": 1000, "end_ms": 2000, "text": "Friday", "source": "loopback",
             "speaker_id": "Remote 2", "original_speaker": "Remote 1", "original_text": "fry day",
             "chunk_id": 0, "annotation": {"text": "", "highlight": true}}
        ));
        let m = line_markup(&v, &v.lines()[0], true);
        assert!(
            m.starts_with("<span foreground=\"#26a269\"><b>▶</b></span> "),
            "{m}"
        );
        assert!(
            m.contains("Remote 2</span></b> <span alpha=\"60%\">(was Remote 1)</span>"),
            "{m}"
        );
        assert!(m.contains("bgalpha=\"35%\">Friday</span>"), "{m}");
        assert!(m.contains("(edited)"), "{m}");
    }

    #[test]
    fn summary_markdown_becomes_markup() {
        let md = "# Standup\n\n\n## Decisions\n- Ship **Friday** & tag\n\n## Action items\n- [ ] Tag the release (Dana)\n- [x] Draft notes\n  - nested `code`\n\nPlain *text* <here>.";
        assert_eq!(
            summary_markup(md),
            "<span size=\"large\"><b>Standup</b></span>\n\n<b>Decisions</b>\n• Ship <b>Friday</b> &amp; tag\n\n<b>Action items</b>\n☐ Tag the release (Dana)\n☑ Draft notes\n  • nested <tt>code</tt>\n\nPlain <i>text</i> &lt;here&gt;."
        );
    }

    #[test]
    fn unmatched_emphasis_is_left_as_text() {
        assert_eq!(
            summary_markup("2 * 3 = 6 and a ` tick"),
            "2 * 3 = 6 and a ` tick"
        );
    }
}
