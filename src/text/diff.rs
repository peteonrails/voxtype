//! Word-level diff of a transcription against its cleaned-up form, rendered
//! for a desktop notification.
//!
//! The point is to show *what the cleanup did*, not just that it did
//! something: deletions struck through, replacements highlighted. Notification
//! servers disagree about how to express that, so rendering is split from the
//! diff itself and the caller passes the dialect its server understands.

/// What happened to a run of words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    /// Survived the cleanup unchanged.
    Same,
    /// Present before, gone after.
    Deleted,
    /// Not present before; the cleanup put it there.
    Inserted,
}

/// A run of words sharing one fate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub kind: SpanKind,
    pub text: String,
}

/// How a notification server expects inline markup.
///
/// The two markup families are mutually unintelligible: Qt's rich text knows
/// `<font color>` and ignores `<span>`, Pango is exactly the other way round.
/// Verified against quickshell, which silently drops `<span>` in both the
/// Pango and CSS spellings while honouring `<font color>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyMarkup {
    /// Qt rich text: quickshell, Plasma.
    Qt,
    /// Pango: mako, dunst, swaync, GNOME Shell.
    Pango,
    /// Server does not advertise `body-markup`; use ASCII markers.
    #[default]
    Plain,
}

/// Escape text that is about to sit inside markup. Dictation really does
/// produce `&` and `<` - "ampersand" and "less than" are both in the spoken
/// punctuation map - and an unescaped one breaks the whole notification body.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Longest common subsequence over whitespace-separated words.
fn lcs_table(before: &[&str], after: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] = if before[i] == after[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

/// Diff `before` against `after` at word granularity.
pub fn word_diff(before: &str, after: &str) -> Vec<Span> {
    let b: Vec<&str> = before.split_whitespace().collect();
    let a: Vec<&str> = after.split_whitespace().collect();
    let table = lcs_table(&b, &a);

    let mut spans: Vec<Span> = Vec::new();
    let push = |kind: SpanKind, word: &str, spans: &mut Vec<Span>| {
        // Merge into the previous run when it shares a fate, so the rendered
        // output reads as phrases rather than word-by-word tagging.
        match spans.last_mut() {
            Some(last) if last.kind == kind => {
                last.text.push(' ');
                last.text.push_str(word);
            }
            _ => spans.push(Span {
                kind,
                text: word.to_string(),
            }),
        }
    };

    let (mut i, mut j) = (0usize, 0usize);
    while i < b.len() && j < a.len() {
        if b[i] == a[j] {
            push(SpanKind::Same, b[i], &mut spans);
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            push(SpanKind::Deleted, b[i], &mut spans);
            i += 1;
        } else {
            push(SpanKind::Inserted, a[j], &mut spans);
            j += 1;
        }
    }
    while i < b.len() {
        push(SpanKind::Deleted, b[i], &mut spans);
        i += 1;
    }
    while j < a.len() {
        push(SpanKind::Inserted, a[j], &mut spans);
        j += 1;
    }
    spans
}

/// Number of distinct edits, for the notification title.
pub fn edit_count(spans: &[Span]) -> usize {
    spans.iter().filter(|s| s.kind != SpanKind::Same).count()
}

/// Render a diff for a notification body in the given dialect.
pub fn render(spans: &[Span], markup: BodyMarkup) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(spans.len());
    for span in spans {
        let body = match markup {
            BodyMarkup::Plain => span.text.clone(),
            _ => escape(&span.text),
        };
        parts.push(match (span.kind, markup) {
            (SpanKind::Same, _) => body,
            (SpanKind::Deleted, BodyMarkup::Plain) => format!("[-{}-]", body),
            (SpanKind::Inserted, BodyMarkup::Plain) => format!("{{+{}+}}", body),
            (SpanKind::Deleted, _) => format!("<s>{}</s>", body),
            (SpanKind::Inserted, BodyMarkup::Qt) => {
                format!("<font color=\"#22cc22\">{}</font>", body)
            }
            (SpanKind::Inserted, BodyMarkup::Pango) => {
                format!("<span foreground=\"#22cc22\">{}</span>", body)
            }
        });
    }
    parts.join(" ")
}

/// Visible length of a diff, ignoring markup.
fn visible_len(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.text.chars().count() + 1).sum()
}

/// Render a diff to fit a notification body.
///
/// Notification servers truncate the body (quickshell caps it around three
/// lines and appends an ellipsis), and slicing rendered markup would cut a tag
/// in half. So when the whole diff will not fit, drop the untouched text and
/// show only the edits - never slice the rendered string.
pub fn render_for_notification(spans: &[Span], markup: BodyMarkup, budget: usize) -> String {
    if visible_len(spans) <= budget {
        return render(spans, markup);
    }
    let edits: Vec<Span> = spans
        .iter()
        .filter(|s| s.kind != SpanKind::Same)
        .cloned()
        .collect();
    let rendered: Vec<String> = edits
        .iter()
        .map(|s| render(std::slice::from_ref(s), markup))
        .collect();
    rendered.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(spans: &[Span]) -> Vec<(SpanKind, &str)> {
        spans.iter().map(|s| (s.kind, s.text.as_str())).collect()
    }

    #[test]
    fn identical_text_has_no_edits() {
        let spans = word_diff("hello world", "hello world");
        assert_eq!(edit_count(&spans), 0);
        assert_eq!(kinds(&spans), vec![(SpanKind::Same, "hello world")]);
    }

    #[test]
    fn a_removed_filler_is_one_deletion() {
        let spans = word_diff("uhhhh hello world", "hello world");
        assert_eq!(
            kinds(&spans),
            vec![
                (SpanKind::Deleted, "uhhhh"),
                (SpanKind::Same, "hello world"),
            ]
        );
        assert_eq!(edit_count(&spans), 1);
    }

    #[test]
    fn a_restart_collapses_to_one_deleted_run() {
        let spans = word_diff(
            "and the independent tool. the independent booking tool",
            "and the independent booking tool",
        );
        assert_eq!(
            kinds(&spans),
            vec![
                (SpanKind::Same, "and the independent"),
                (SpanKind::Deleted, "tool. the independent"),
                (SpanKind::Same, "booking tool"),
            ]
        );
        assert_eq!(edit_count(&spans), 1);
    }

    #[test]
    fn a_replacement_shows_as_delete_plus_insert() {
        let spans = word_diff("consent line. Lines, so", "consent lines, so");
        assert_eq!(edit_count(&spans), 2);
        assert!(spans.iter().any(|s| s.kind == SpanKind::Deleted));
        assert!(spans.iter().any(|s| s.kind == SpanKind::Inserted));
    }

    #[test]
    fn qt_dialect_uses_font_color() {
        let spans = word_diff("old", "new");
        let out = render(&spans, BodyMarkup::Qt);
        assert!(out.contains("<s>old</s>"), "{out}");
        assert!(out.contains("<font color=\"#22cc22\">new</font>"), "{out}");
    }

    #[test]
    fn pango_dialect_uses_span_foreground() {
        let spans = word_diff("old", "new");
        let out = render(&spans, BodyMarkup::Pango);
        assert!(out.contains("<s>old</s>"), "{out}");
        assert!(
            out.contains("<span foreground=\"#22cc22\">new</span>"),
            "{out}"
        );
    }

    #[test]
    fn plain_dialect_uses_ascii_markers_and_does_not_escape() {
        let spans = word_diff("a & b", "a c");
        let out = render(&spans, BodyMarkup::Plain);
        assert!(out.contains("[-"), "{out}");
        assert!(out.contains("{+"), "{out}");
        assert!(!out.contains("&amp;"), "{out}");
    }

    #[test]
    fn markup_dialects_escape_dictated_angle_brackets_and_ampersands() {
        // "ampersand" and "less than" are reachable from spoken punctuation,
        // so this is ordinary dictation, not a hostile input.
        let spans = word_diff("tom & jerry <x>", "tom & jerry");
        let out = render(&spans, BodyMarkup::Qt);
        assert!(out.contains("&amp;"), "{out}");
        assert!(out.contains("&lt;x&gt;"), "{out}");
        assert!(!out.contains("<x>"), "{out}");
    }

    #[test]
    fn everything_deleted_still_renders() {
        let spans = word_diff("um uh", "");
        assert_eq!(edit_count(&spans), 1);
        assert_eq!(render(&spans, BodyMarkup::Qt), "<s>um uh</s>");
    }

    #[test]
    fn empty_before_is_all_insertion() {
        let spans = word_diff("", "hello");
        assert_eq!(kinds(&spans), vec![(SpanKind::Inserted, "hello")]);
    }

    #[test]
    fn short_diffs_render_in_full() {
        let spans = word_diff("uhhhh hello world", "hello world");
        let out = render_for_notification(&spans, BodyMarkup::Qt, 200);
        assert!(out.contains("hello world"), "{out}");
        assert!(out.contains("<s>uhhhh</s>"), "{out}");
    }

    #[test]
    fn long_diffs_collapse_to_edits_only_without_slicing_markup() {
        let before = format!("{} uhhhh tail", "word ".repeat(60));
        let after = format!("{} tail", "word ".repeat(60));
        let spans = word_diff(&before, &after);
        let out = render_for_notification(&spans, BodyMarkup::Qt, 80);
        assert_eq!(out, "<s>uhhhh</s>");
        // Whatever we emit must be balanced markup, never a sliced tag.
        assert_eq!(out.matches("<s>").count(), out.matches("</s>").count());
    }
}
