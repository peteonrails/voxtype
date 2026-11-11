//! Commit on pause: end the sentence at a pause and start the next one fresh.
//!
//! With `[streaming] commit_on_pause_secs > 0`, a pause of that length closes
//! the current sentence. The buffer gets one last pass (the same settling pass
//! as at the end of a recording), and the next sentence starts from an empty
//! buffer. Text from earlier sentences is never re-transcribed or revised, so
//! a long dictation is a series of short windows instead of one window that
//! slides. A sliding window starts mid-sentence on every pass, which makes the
//! tail change from one pass to the next and keeps the cursor busy rewriting.
//!
//! A sentence that goes on without a pause until it nears `max_buffer_s` is
//! cut at its quietest moment instead, so the window never slides.
//!
//! Off by default (`0`): the engine behaves exactly as before.

use super::{rms, Delta, Session, Transcriber};
use std::sync::Arc;

/// Length of the frames the pause test looks at.
const FRAME_SECS: f32 = 0.02;

/// A frame is quiet when it is below this fraction of the sentence's speech
/// level (about -12 dB), or below `min_speech_rms`, whichever is higher. The
/// relative bar keeps pauses detectable when background noise sits above
/// `min_speech_rms` (automatic gain control raises it between words).
const QUIET_RATIO: f32 = 0.25;

/// Percentile of the sentence's frame levels taken as its speech level.
const SPEECH_PERCENTILE: f32 = 0.9;

/// Share of the pause window that must be quiet: tolerates a click or a
/// breath without letting a spoken word through.
const QUIET_SHARE: f32 = 0.9;

/// A sentence still going this close to `max_buffer_s` is cut.
const FORCED_CUT_HEADROOM_SECS: f32 = 5.0;

/// The forced cut lands at the quietest moment of the last few seconds...
const FORCED_CUT_SEARCH_SECS: f32 = 3.0;

/// ...leaving out the very end, where a word is being spoken.
const FORCED_CUT_SKIP_SECS: f32 = 0.3;

/// Where the current sentence ends, as an index into the session buffer:
/// its end after a pause, or the quietest moment of the last seconds when
/// the sentence is about to outgrow `max_buffer_s`. `None` while the
/// sentence goes on, and always when the feature is off.
pub(super) fn sentence_end(session: &Session) -> Option<usize> {
    if ends_in_pause(session) {
        return Some(session.buffer.len());
    }
    forced_cut(session)
}

/// Closes the sentence at [`sentence_end`]: one last pass settles its text,
/// then `session` is replaced by a fresh one that keeps the audio after the
/// cut. Returns the settling delta.
pub(super) fn close_sentence(session: &mut Session, base: &Arc<dyn Transcriber>) -> Option<Delta> {
    let end = sentence_end(session).unwrap_or(session.buffer.len());
    let rest = session.buffer.split_off(end);
    let settled = session.final_flush();
    *session = Session::new(Arc::clone(base), session.config);
    session.feed(&rest);
    settled
}

fn forced_cut(session: &Session) -> Option<usize> {
    let config = &session.config;
    if config.commit_on_pause_s <= 0.0 {
        return None;
    }
    let rate = config.sample_rate as f32;
    let limit = (config.max_buffer_s - FORCED_CUT_HEADROOM_SECS).max(FORCED_CUT_SEARCH_SECS * 2.0);
    let len = session.buffer.len();
    if (len as f32) < limit * rate {
        return None;
    }
    let lo = len - (FORCED_CUT_SEARCH_SECS * rate) as usize;
    let hi = len - (FORCED_CUT_SKIP_SECS * rate) as usize;
    Some(crate::eager::quietest_cut(&session.buffer, lo, hi))
}

/// True when `session`'s buffer ends in a pause of `commit_on_pause_s`
/// after some speech. Always false when the feature is off.
fn ends_in_pause(session: &Session) -> bool {
    let config = &session.config;
    if config.commit_on_pause_s <= 0.0 {
        return false;
    }
    let rate = config.sample_rate as f32;
    let pause_len = (config.commit_on_pause_s * rate) as usize;
    let frame_len = ((FRAME_SECS * rate) as usize).max(1);
    let buffer = &session.buffer;
    if pause_len < frame_len || buffer.len() <= pause_len + frame_len {
        return false;
    }

    let (speech, tail) = buffer.split_at(buffer.len() - pause_len);
    let mut levels: Vec<f32> = speech.chunks(frame_len).map(rms).collect();
    levels.sort_by(f32::total_cmp);
    let speech_level = levels[((levels.len() - 1) as f32 * SPEECH_PERCENTILE) as usize];
    if speech_level < config.min_speech_rms {
        // Nothing has been said yet: no sentence to close.
        return false;
    }

    let bar = (speech_level * QUIET_RATIO).max(config.min_speech_rms);
    let frames: Vec<f32> = tail.chunks(frame_len).map(rms).collect();
    let quiet = frames.iter().filter(|level| **level < bar).count();
    quiet as f32 >= frames.len() as f32 * QUIET_SHARE
}

/// Joins sentences into one text. Each sentence is typed by a fresh
/// [`Session`], which does not know that text precedes it, so this adds the
/// space between sentences, and a capital after a sentence that ended with
/// `.`, `?` or `!` (Whisper often hears the next sentence as a
/// continuation and starts it in lowercase). It follows what the session
/// has typed in the current sentence to tell when a correction reaches
/// back to the sentence's start.
#[derive(Debug, Default)]
pub(super) struct SentenceJoin {
    /// The current sentence as the session typed it, without the changes
    /// made here. Session backspace counts refer to this text.
    sentence: String,
    /// An earlier sentence typed something.
    typed_before: bool,
    /// The space before the current sentence is on screen.
    space_typed: bool,
    /// The previous sentence ended with terminal punctuation.
    capitalize: bool,
}

impl SentenceJoin {
    pub(super) fn apply(&mut self, delta: Delta) -> Delta {
        let (backspace, mut text) = match delta {
            Delta::Append(text) => (None, text),
            Delta::Replace { backspace, text } => (Some(backspace), text),
        };
        let typed = self.sentence.chars().count();
        let keep = typed - backspace.unwrap_or(0).min(typed);
        self.sentence = self.sentence.chars().take(keep).collect();
        self.sentence.push_str(&text);

        if keep == 0 && !text.is_empty() {
            if self.capitalize {
                capitalize_first_letter(&mut text);
            }
            if self.typed_before && !self.space_typed && !text.starts_with(char::is_whitespace) {
                text.insert(0, ' ');
                self.space_typed = true;
            }
        }
        match backspace {
            None => Delta::Append(text),
            Some(backspace) => Delta::Replace { backspace, text },
        }
    }

    /// Call once the current sentence is closed.
    pub(super) fn next_sentence(&mut self) {
        let sentence = self.sentence.trim_end();
        if !sentence.is_empty() {
            self.typed_before = true;
            self.capitalize = sentence.ends_with(['.', '?', '!', '…']);
        }
        self.sentence.clear();
        self.space_typed = false;
    }
}

/// Uppercase the first letter, past any leading quote or bracket. Leaves the
/// text alone when the uppercase form is not a single character, so the
/// session's backspace counts stay right.
fn capitalize_first_letter(text: &mut String) {
    let Some((at, letter)) = text.char_indices().find(|(_, c)| c.is_alphanumeric()) else {
        return;
    };
    let mut upper = letter.to_uppercase();
    if let (Some(up), None) = (upper.next(), upper.next()) {
        if up != letter {
            text.replace_range(at..at + letter.len_utf8(), up.encode_utf8(&mut [0; 4]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SlidingWindowConfig, Transcriber};
    use super::*;
    use crate::error::TranscribeError;
    use std::sync::Arc;

    struct Silent;
    impl Transcriber for Silent {
        fn transcribe(&self, _samples: &[f32]) -> Result<String, TranscribeError> {
            Ok(String::new())
        }
    }

    const RATE: usize = 16_000;

    fn session(pause_secs: f32, audio: Vec<f32>) -> Session {
        let config = SlidingWindowConfig {
            commit_on_pause_s: pause_secs,
            max_buffer_s: 60.0,
            ..SlidingWindowConfig::default()
        };
        let mut session = Session::new(Arc::new(Silent), config);
        session.feed(&audio);
        session
    }

    /// Alternating-sign signal with the given RMS.
    fn level(secs: f32, amplitude: f32) -> Vec<f32> {
        (0..(secs * RATE as f32) as usize)
            .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
            .collect()
    }

    fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
        parts.concat()
    }

    #[test]
    fn speech_then_silence_is_a_pause() {
        let audio = concat(&[level(2.0, 0.1), level(0.7, 0.0005)]);
        assert!(ends_in_pause(&session(0.6, audio)));
    }

    #[test]
    fn a_short_gap_is_not_a_pause() {
        let audio = concat(&[level(2.0, 0.1), level(0.3, 0.0005)]);
        assert!(!ends_in_pause(&session(0.6, audio)));
    }

    #[test]
    fn ongoing_speech_is_not_a_pause() {
        let audio = concat(&[level(2.0, 0.1), level(0.3, 0.0005), level(0.5, 0.08)]);
        assert!(!ends_in_pause(&session(0.6, audio)));
    }

    #[test]
    fn silence_before_any_speech_closes_nothing() {
        assert!(!ends_in_pause(&session(0.6, level(3.0, 0.0005))));
    }

    #[test]
    fn a_pause_over_background_noise_above_min_speech_rms_is_detected() {
        // Noise at 0.012 RMS is above the 0.005 default, speech at 0.1.
        let audio = concat(&[level(2.0, 0.1), level(0.7, 0.012)]);
        assert!(ends_in_pause(&session(0.6, audio)));
    }

    #[test]
    fn quiet_speech_is_not_taken_for_a_pause() {
        // Whispered sentence that keeps going at a third of its peak level.
        let audio = concat(&[level(2.0, 0.03), level(0.7, 0.012)]);
        assert!(!ends_in_pause(&session(0.6, audio)));
    }

    #[test]
    fn a_sentence_without_a_pause_is_cut_at_its_quietest_moment_before_sliding() {
        // max_buffer 25 s: no cut at 19 s, a cut at 20 s, inside the short
        // dip 1.5 s before the end rather than in the speech around it.
        let config = SlidingWindowConfig {
            commit_on_pause_s: 0.6,
            max_buffer_s: 25.0,
            ..SlidingWindowConfig::default()
        };
        let mut session = Session::new(Arc::new(Silent), config);
        session.feed(&level(19.0, 0.1));
        assert_eq!(sentence_end(&session), None);
        session.feed(&concat(&[level(0.2, 0.002), level(0.8, 0.1)]));
        let cut = sentence_end(&session).expect("forced cut") as f32 / RATE as f32;
        assert!((19.0..19.2).contains(&cut), "cut at {cut}s");
    }

    #[test]
    fn closing_a_sentence_keeps_the_audio_after_the_cut() {
        let config = SlidingWindowConfig {
            commit_on_pause_s: 0.6,
            max_buffer_s: 25.0,
            ..SlidingWindowConfig::default()
        };
        let base: Arc<dyn Transcriber> = Arc::new(Silent);
        let mut session = Session::new(Arc::clone(&base), config);
        session.feed(&concat(&[
            level(19.0, 0.1),
            level(0.2, 0.002),
            level(0.8, 0.1),
        ]));
        close_sentence(&mut session, &base);
        let kept = session.buffer.len() as f32 / RATE as f32;
        assert!((0.8..1.0).contains(&kept), "kept {kept}s");
    }

    #[test]
    fn off_by_default() {
        let audio = concat(&[level(2.0, 0.1), level(2.0, 0.0)]);
        assert!(!ends_in_pause(&session(0.0, audio)));
    }

    /// Names each sentence after its loudest sample, so the fake knows
    /// which sentence a buffer holds without real speech.
    struct BySentence;
    impl Transcriber for BySentence {
        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            Ok(if peak > 0.09 {
                "Bonjour."
            } else if peak > 0.06 {
                "ça va bien."
            } else {
                ""
            }
            .to_string())
        }
    }

    #[tokio::test]
    async fn two_sentences_are_typed_once_each_with_a_space_between() {
        use super::super::{
            SlidingWindowStreamingTranscriber, StreamingEvent, StreamingTranscriber,
        };
        use std::time::Duration;
        use tokio::sync::mpsc;

        let config = SlidingWindowConfig {
            interval_s: 0.02,
            min_audio_s: 0.5,
            commit_on_pause_s: 0.6,
            revision_mode: true,
            ..SlidingWindowConfig::default()
        };
        let engine = SlidingWindowStreamingTranscriber::new(Arc::new(BySentence), config);
        let (tx, rx) = mpsc::channel::<Vec<f32>>(64);
        let mut handle = engine.start_stream(rx).expect("start stream");

        let parts = [level(1.5, 0.1), level(0.8, 0.0005), level(1.5, 0.07)];
        for part in parts {
            for chunk in part.chunks(RATE / 10) {
                tx.send(chunk.to_vec()).await.unwrap();
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }
        drop(tx);

        let mut typed = String::new();
        while let Some(event) = handle.events.recv().await {
            match event {
                StreamingEvent::Partial { text, .. } | StreamingEvent::Final { text, .. } => {
                    typed.push_str(&text)
                }
                StreamingEvent::Replace {
                    backspace, text, ..
                } => {
                    assert!(backspace <= typed.chars().count());
                    let keep = typed.chars().count() - backspace;
                    typed = typed.chars().take(keep).collect();
                    // Settled text from the first sentence is never touched.
                    assert!(typed.starts_with("Bonjour.") || !typed.contains("Ça"));
                    typed.push_str(&text);
                }
                StreamingEvent::Ended => break,
                _ => {}
            }
        }
        handle.task.await.unwrap().expect("task ok");
        assert_eq!(typed, "Bonjour. Ça va bien.");
    }

    fn append(text: &str) -> Delta {
        Delta::Append(text.into())
    }

    fn replace(backspace: usize, text: &str) -> Delta {
        Delta::Replace {
            backspace,
            text: text.into(),
        }
    }

    #[test]
    fn sentences_after_the_first_get_one_leading_space() {
        let mut join = SentenceJoin::default();
        assert_eq!(join.apply(append("Bonjour")), append("Bonjour"));
        join.next_sentence();
        assert_eq!(join.apply(append("ça va")), append(" ça va"));
        // Later text in the same sentence already carries its own spacing.
        assert_eq!(join.apply(append(" bien")), append(" bien"));
    }

    #[test]
    fn a_sentence_after_terminal_punctuation_starts_with_a_capital() {
        let mut join = SentenceJoin::default();
        join.apply(append("Ça va fonctionner."));
        join.next_sentence();
        assert_eq!(
            join.apply(append("que ça marche")),
            append(" Que ça marche")
        );
        join.next_sentence();
        // No terminal punctuation: Whisper's casing is kept.
        assert_eq!(join.apply(append("et ensuite")), append(" et ensuite"));
    }

    #[test]
    fn a_correction_back_to_the_start_keeps_one_space_and_the_capital() {
        let mut join = SentenceJoin::default();
        join.apply(append("Fin."));
        join.next_sentence();
        assert_eq!(join.apply(append("OK.")), append(" OK."));
        assert_eq!(
            join.apply(replace(1, ", ça marche")),
            replace(1, ", ça marche")
        );
        // The session rewrites its whole sentence: the space added before it
        // is still on screen, so only the capital is applied.
        assert_eq!(
            join.apply(replace(14, "que ça marche.")),
            replace(14, "Que ça marche.")
        );
    }

    #[test]
    fn a_sentence_that_typed_nothing_changes_nothing() {
        let mut join = SentenceJoin::default();
        join.next_sentence();
        assert_eq!(join.apply(append("bonjour.")), append("bonjour."));
        join.next_sentence();
        join.next_sentence();
        assert_eq!(join.apply(replace(0, "suite")), replace(0, " Suite"));
    }

    #[test]
    fn capitals_skip_leading_quotes_and_keep_the_length() {
        let mut text = String::from("« écoute »");
        capitalize_first_letter(&mut text);
        assert_eq!(text, "« Écoute »");
        let mut text = String::from("ß");
        capitalize_first_letter(&mut text);
        assert_eq!(text, "ß");
    }
}
