//! The echo guard: the remote side, heard through speakers, kept out of
//! the microphone's "You" lines.
//!
//! With speakers instead of headphones, the microphone hears the remote
//! participants (or a video playing on the desktop), and the speech engine
//! transcribes it under "You". Echo cancellation removes most of it from
//! the audio; this is the safety net in the transcript. Long echoed phrases
//! that match the loopback word for word are caught by the 4-gram match and
//! short ones by the exact-words rule in [`crate::meeting::data::Transcript`].
//! The guard catches what those miss:
//!
//! - [`EchoProbe`] compares the two tracks' levels frame by frame. Echo is
//!   the loopback, delayed and quieter, so its level follows the loopback's
//!   at a steady ratio; your voice is louder than that path explains. A line
//!   with a stretch of such frames is yours ([`EchoProbe::own_voice`]) and
//!   no echo rule touches it. A line whose sound is all explained is echo
//!   ([`EchoProbe::echo_only`]), whatever its length: Pete's 1.3.0 test had
//!   long passages of a video transcribed as his.
//! - [`same_speech`] decides what to do with such a line: drop it when the
//!   loopback said the same at the time, move it to the remote speaker when
//!   not (see [`crate::meeting::data::Transcript::resolve_echo_lines`]).
//! - [`fuzzy_echo`]: a short line the engine *misheard*, whose words are,
//!   most of them, close to words of an overlapping loopback line by spelling
//!   (normalized edit distance) or by sound (a consonant skeleton). The demo
//!   replay left "Omarky problem." while the loopback said "...the Amarchy
//!   package... without any problems".
//! - [`no_speech_phrase`]: a short line that is only what speech engines
//!   write for sound they cannot make out ("Thank you.", "Thanks for
//!   watching."), on a microphone holding no voice of yours while the
//!   remote talks. Pete's 1.3.0 test with the WebRTC canceller had two
//!   "Thank you." lines under You, made from the canceller's faint residual,
//!   that no word of the loopback explained.

use crate::meeting::data::{AudioSource, TranscriptSegment};

/// Tunables for the short-line echo rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoRules {
    /// The echo guard (`echo_guard`): lines of any length that the audio
    /// shows to be only echo, and short lines that are the remote misheard.
    /// Off, only the exact-words and phrase rules run, as before 1.3.0.
    pub guard: bool,
    /// The longest microphone line, in words, the fuzzy rule looks at.
    pub fuzzy_max_words: usize,
}

impl Default for EchoRules {
    fn default() -> Self {
        Self {
            guard: true,
            fuzzy_max_words: 4,
        }
    }
}

impl From<&crate::config::MeetingAudioConfig> for EchoRules {
    fn from(audio: &crate::config::MeetingAudioConfig) -> Self {
        Self {
            guard: audio.echo_guard,
            fuzzy_max_words: audio.echo_fuzzy_max_words.clamp(1, 12) as usize,
        }
    }
}

/// How far apart in time a microphone line and the loopback line it echoes
/// may be: the echo path's latency plus how loosely each track's lines are
/// timed.
pub const ECHO_TOLERANCE_MS: u64 = 1000;

/// Normalized Levenshtein distance at or below which two words count as
/// the same word misheard.
const MAX_WORD_DISTANCE: f32 = 0.34;

/// Lowercase letters and digits only.
pub fn normalize_word(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Edit distance divided by the longer word's length.
pub fn word_distance(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    levenshtein(&a, &b) as f32 / longest as f32
}

/// A rough sound key: the word's consonants as heard, vowels dropped.
/// "Omarky" and "Amarchy" both give "mrk", "problem" and "problems" give
/// "prblm"/"prblms". Deliberately loose: it only ever counts in favour of
/// echo for a short line that already overlaps the loopback in time and is
/// not shown to be your own voice by the audio.
pub fn sound_key(word: &str) -> String {
    let w: Vec<char> = normalize_word(word).chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < w.len() {
        let c = w[i];
        let next = w.get(i + 1).copied();
        let mapped: Option<char> = match (c, next) {
            ('c', Some('h')) | ('k', Some('h')) | ('g', Some('h')) => {
                i += 1;
                if c == 'g' {
                    None
                } else {
                    Some('k')
                }
            }
            ('p', Some('h')) => {
                i += 1;
                Some('f')
            }
            ('s', Some('h')) => {
                i += 1;
                Some('s')
            }
            ('t', Some('h')) => {
                i += 1;
                Some('t')
            }
            ('c', Some('e' | 'i' | 'y')) => Some('s'),
            ('c' | 'q', _) => Some('k'),
            ('x', _) => Some('k'),
            ('z', _) => Some('s'),
            ('d', _) => Some('t'),
            ('b', _) => Some('p'),
            ('v', _) => Some('f'),
            ('g', _) => Some('k'),
            (c, _) if "aeiouyhw".contains(c) => None,
            (c, _) => Some(c),
        };
        if let Some(m) = mapped {
            if !out.ends_with(m) {
                out.push(m);
            }
        }
        i += 1;
    }
    out
}

/// Numbers come out as digits or as words depending on the engine and the
/// context ("by five" on the loopback, "5." on the microphone).
fn spelled(word: &str) -> &str {
    const NUMBERS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    word.parse::<usize>()
        .ok()
        .and_then(|n| NUMBERS.get(n).copied())
        .unwrap_or(word)
}

/// Whether `heard` (a microphone word) is `said` (a loopback word) misheard.
pub fn words_match(heard: &str, said: &str) -> bool {
    let (heard, said) = (spelled(heard), spelled(said));
    if heard == said {
        return true;
    }
    // Very short words carry too little to match on anything but spelling.
    if heard.chars().count() <= 2 || said.chars().count() <= 2 {
        return false;
    }
    if word_distance(heard, said) <= MAX_WORD_DISTANCE {
        return true;
    }
    let (a, b) = (sound_key(heard), sound_key(said));
    a.len() >= 2 && a == b
}

/// Whether a short microphone line's words are, most of them, misheard
/// words of `loopback_words`. Returns false for lines longer than
/// `max_words` or with nothing to compare.
pub fn fuzzy_echo(mic_words: &[String], loopback_words: &[String], max_words: usize) -> bool {
    if mic_words.is_empty() || mic_words.len() > max_words || loopback_words.is_empty() {
        return false;
    }
    let matched = mic_words
        .iter()
        .filter(|w| loopback_words.iter().any(|l| words_match(w, l)))
        .count();
    // "Most": three in four, rounded up, so a one- or two-word line must
    // match entirely.
    matched * 4 >= mic_words.len() * 3
}

/// The words of the loopback lines that overlap `seg` in time.
pub fn overlapping_loopback_words(
    seg: &TranscriptSegment,
    all: &[TranscriptSegment],
) -> Vec<String> {
    all.iter()
        .filter(|s| s.source == AudioSource::Loopback)
        .filter(|s| {
            seg.start_ms < s.end_ms + ECHO_TOLERANCE_MS
                && s.start_ms < seg.end_ms + ECHO_TOLERANCE_MS
        })
        .flat_map(|s| s.text.split_whitespace().map(normalize_word))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether a microphone line the audio already shows to be echo also says
/// what the loopback said at the time: even part of it is enough to
/// confirm, since the audio made the case. A short line needs most of its
/// words to match; a longer one at least two words of five letters or more,
/// and a quarter of those, so that "the", "and" or "with" confirm nothing.
pub fn same_speech(mic_words: &[String], loopback_words: &[String]) -> bool {
    if mic_words.is_empty() || loopback_words.is_empty() {
        return false;
    }
    if mic_words.len() <= 3 {
        return fuzzy_echo(mic_words, loopback_words, 3);
    }
    let content: Vec<&String> = mic_words
        .iter()
        .filter(|w| w.chars().count() >= 5)
        .collect();
    let matched = content
        .iter()
        .filter(|w| loopback_words.iter().any(|l| words_match(w, l)))
        .count();
    matched >= 2 && matched * 4 >= content.len()
}

/// What speech engines write for silence, noise or faint residual they
/// cannot make out: the closing lines of the videos they learned from.
/// Only whole lines made of these are taken for one, and only when the
/// audio shows no voice of yours (see [`no_speech_phrase`]).
const NO_SPEECH_PHRASES: &[&str] = &[
    // English
    "thank you",
    "thank you very much",
    "thank you so much",
    "thanks",
    "thanks for watching",
    "thank you for watching",
    "you",
    "bye",
    "bye bye",
    "goodbye",
    // German, French, Spanish, Portuguese, Italian, Dutch
    "danke",
    "vielen dank",
    "danke schön",
    "danke fürs zuschauen",
    "merci",
    "merci beaucoup",
    "merci d'avoir regardé",
    "gracias",
    "muchas gracias",
    "gracias por ver",
    "obrigado",
    "obrigada",
    "obrigado por assistir",
    "grazie",
    "grazie mille",
    "grazie per la visione",
    "dank je wel",
    "bedankt",
    "bedankt voor het kijken",
    // Russian, Japanese, Chinese, Korean
    "спасибо",
    "спасибо за просмотр",
    "ありがとうございました",
    "ご視聴ありがとうございました",
    "谢谢",
    "谢谢观看",
    "감사합니다",
    "시청해주셔서 감사합니다",
];

/// The longest line, in words, taken for a no-speech phrase: every phrase
/// above fits, and longer lines are real speech often enough to leave.
pub const NO_SPEECH_MAX_WORDS: usize = 4;

/// Whether a line's words are nothing but no-speech phrases, one or more
/// ("Thank you. Bye."), in at most [`NO_SPEECH_MAX_WORDS`] words.
pub fn no_speech_phrase(words: &[String]) -> bool {
    if words.is_empty() || words.len() > NO_SPEECH_MAX_WORDS {
        return false;
    }
    let phrases: Vec<Vec<String>> = NO_SPEECH_PHRASES
        .iter()
        .map(|p| {
            p.split_whitespace()
                .map(normalize_word)
                .filter(|w| !w.is_empty())
                .collect()
        })
        .collect();
    // ends[i]: the first i words split into whole phrases.
    let mut ends = vec![false; words.len() + 1];
    ends[0] = true;
    for i in 0..words.len() {
        if !ends[i] {
            continue;
        }
        for phrase in &phrases {
            if words[i..].starts_with(phrase) {
                ends[i + phrase.len()] = true;
            }
        }
    }
    ends[words.len()]
}

/// Analysis frame for the energy guard.
const FRAME_MS: u64 = 30;
/// The longest echo path delay looked for.
const MAX_LAG_FRAMES: usize = 10;
/// Your voice: this much louder on the microphone than the echo path
/// explains...
const OWN_VOICE_MARGIN: f32 = 2.5;
/// ...for this many consecutive frames (150 ms), a real onset rather than
/// a click.
const OWN_VOICE_FRAMES: usize = 5;
/// A line counts as echo only with at least this much voiced microphone
/// audio (300 ms) to judge ...
const MIN_ECHO_FRAMES: usize = 10;
/// ... and when at least this share of its voiced frames is explained.
const ECHO_EXPLAINED: f32 = 0.9;

/// Energy of one chunk of both tracks, for telling your voice from echo.
#[derive(Debug, Clone)]
pub struct EchoProbe {
    mic: Vec<f32>,
    loopback: Vec<f32>,
    mic_start_ms: u64,
    loopback_start_ms: u64,
    threshold: f32,
    /// Below this a microphone frame is silence: the VAD threshold, or three
    /// times the microphone's own noise floor if that is higher, so that a
    /// noisy room or a low threshold does not read as speech between words.
    mic_floor: f32,
    /// The same for the loopback: below it, the far side is silent.
    loopback_floor: f32,
    /// Echo delay in frames and microphone/loopback level ratio, when the
    /// chunk had enough remote speech to measure them.
    path: Option<(usize, f32)>,
}

fn frame_rms(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let n = (sample_rate as u64 * FRAME_MS / 1000).max(1) as usize;
    samples
        .chunks(n)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

fn median(mut v: Vec<f32>) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

impl EchoProbe {
    /// Measure one chunk of each track. `threshold` is the meeting VAD's
    /// RMS threshold; frames below it are silence.
    pub fn new(
        mic: &[f32],
        mic_start_ms: u64,
        loopback: &[f32],
        loopback_start_ms: u64,
        sample_rate: u32,
        threshold: f32,
    ) -> Self {
        let mut probe = Self {
            mic: frame_rms(mic, sample_rate),
            loopback: frame_rms(loopback, sample_rate),
            mic_start_ms,
            loopback_start_ms,
            threshold: threshold.max(1e-4),
            mic_floor: 0.0,
            loopback_floor: 0.0,
            path: None,
        };
        probe.path = probe.measure_path();
        // The quietest tenth of the chunk is the room with nobody talking.
        let mut sorted = probe.mic.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
        probe.mic_floor = probe.threshold.max(3.0 * noise);
        let mut sorted = probe.loopback.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
        probe.loopback_floor = probe.threshold.max(3.0 * noise);
        probe
    }

    /// The echo path: the delay at which the microphone's level best
    /// follows the loopback's during remote speech, and the median level
    /// ratio there. Medians keep moments when you talk over the remote from
    /// skewing it.
    fn measure_path(&self) -> Option<(usize, f32)> {
        let voiced: Vec<usize> = (0..self.loopback.len())
            .filter(|&i| self.loopback[i] > self.threshold)
            .collect();
        if voiced.len() < 10 {
            return None;
        }
        let offset = self.loopback_start_ms as i64 - self.mic_start_ms as i64;
        let shift = offset / FRAME_MS as i64;
        let mut best: Option<(usize, f32, f32)> = None;
        for lag in 0..=MAX_LAG_FRAMES {
            let ratios: Vec<f32> = voiced
                .iter()
                .filter_map(|&i| {
                    let j = i as i64 + shift + lag as i64;
                    let m = *self.mic.get(usize::try_from(j).ok()?)?;
                    Some(m / self.loopback[i])
                })
                .collect();
            let Some(ratio) = median(ratios.clone()) else {
                continue;
            };
            // How tightly the ratio holds at this lag: echo is a scaled copy,
            // so the right lag has the least spread.
            let spread = median(ratios.iter().map(|r| (r - ratio).abs()).collect())
                .unwrap_or(f32::MAX)
                / ratio.max(1e-6);
            if best.is_none_or(|(_, _, s)| spread < s) {
                best = Some((lag, ratio, spread));
            }
        }
        best.map(|(lag, ratio, _)| (lag, ratio))
    }

    /// The loudest the echo of the loopback could be in microphone frame
    /// `i`: the measured echo path, with a frame of slack either side of
    /// its delay. That slack and the margin also cover a room's ringing:
    /// the speaker-echo test still catches every echo line with reflections
    /// dying away at 0.75 per frame. `None` without a measured path.
    fn explained(&self, i: usize) -> Option<f32> {
        let (lag, ratio) = self.path?;
        let offset = self.loopback_start_ms as i64 - self.mic_start_ms as i64;
        let shift = offset / FRAME_MS as i64;
        let centre = i as i64 - shift - lag as i64;
        let level = |j: i64| -> f32 {
            usize::try_from(j)
                .ok()
                .and_then(|j| self.loopback.get(j))
                .copied()
                .unwrap_or(0.0)
        };
        let remote = (centre - 1..=centre + 1).map(level).fold(0.0f32, f32::max);
        Some(ratio * remote)
    }

    /// Whether the loopback between `start_ms` and `end_ms` (on the
    /// loopback's timeline) holds speech: a run of frames above its floor
    /// as long as an onset of your own voice ([`OWN_VOICE_FRAMES`]). A span
    /// the chunk does not cover counts as speech, so a line is never
    /// dropped on a guess.
    pub fn remote_speech(&self, start_ms: u64, end_ms: u64) -> bool {
        let first = (start_ms.saturating_sub(self.loopback_start_ms) / FRAME_MS) as usize;
        let last = (end_ms
            .saturating_sub(self.loopback_start_ms)
            .div_ceil(FRAME_MS) as usize)
            .min(self.loopback.len());
        if start_ms < self.loopback_start_ms || first >= last {
            return true;
        }
        let mut run = 0;
        for &level in &self.loopback[first..last] {
            if level > self.loopback_floor {
                run += 1;
                if run >= OWN_VOICE_FRAMES {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }

    fn frames(&self, start_ms: u64, end_ms: u64) -> std::ops::Range<usize> {
        let first = (start_ms.saturating_sub(self.mic_start_ms) / FRAME_MS) as usize;
        let last = (end_ms.saturating_sub(self.mic_start_ms).div_ceil(FRAME_MS) as usize)
            .min(self.mic.len());
        first.min(last)..last
    }

    /// Whether the microphone between `start_ms` and `end_ms` holds speech
    /// of its own: a run of frames louder than the echo path explains, or
    /// voiced while the remote is silent. Unknown audio counts as yours, so
    /// a line is never dropped on a guess.
    pub fn own_voice(&self, start_ms: u64, end_ms: u64) -> bool {
        if self.path.is_none() {
            return true;
        }
        let frames = self.frames(start_ms, end_ms);
        if frames.is_empty() {
            return true;
        }
        let mut run = 0;
        for i in frames {
            let m = self.mic[i];
            if m <= self.mic_floor {
                run = 0;
                continue;
            }
            let explained = self.explained(i).unwrap_or(0.0);
            if m > explained * OWN_VOICE_MARGIN {
                run += 1;
                if run >= OWN_VOICE_FRAMES {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }

    /// Whether the microphone between `start_ms` and `end_ms` is only the
    /// remote side's echo: enough voiced audio to judge, at least
    /// nine-tenths of it no louder than the echo path explains, and no
    /// stretch of your own voice. This is the test for lines of any length;
    /// a line with some of your speech in it is never echo.
    pub fn echo_only(&self, start_ms: u64, end_ms: u64) -> bool {
        if self.path.is_none() || self.own_voice(start_ms, end_ms) {
            return false;
        }
        let (mut voiced, mut explained) = (0usize, 0usize);
        for i in self.frames(start_ms, end_ms) {
            let m = self.mic[i];
            if m <= self.mic_floor {
                continue;
            }
            voiced += 1;
            if m <= self.explained(i).unwrap_or(0.0) * OWN_VOICE_MARGIN {
                explained += 1;
            }
        }
        voiced >= MIN_ECHO_FRAMES && explained as f32 >= voiced as f32 * ECHO_EXPLAINED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace()
            .map(normalize_word)
            .filter(|w| !w.is_empty())
            .collect()
    }

    #[test]
    fn no_speech_phrases_are_whole_lines_of_them() {
        for line in [
            "Thank you.",
            "Thank you. Thank you.",
            "Thanks for watching!",
            "you",
            "Bye.",
            "Merci d'avoir regardé.",
            "Vielen Dank.",
            "Спасибо за просмотр.",
            "ご視聴ありがとうございました",
        ] {
            assert!(no_speech_phrase(&words(line)), "{line:?}");
        }
        for line in [
            "Thank you, Dana.",
            "Thanks for the update.",
            "Sounds good.",
            "Thank you. Thank you. Thank you.",
            "",
        ] {
            assert!(!no_speech_phrase(&words(line)), "{line:?}");
        }
    }

    #[test]
    fn misheard_words_match_by_spelling_or_sound() {
        assert!(words_match("problem", "problems"));
        assert!(words_match("omarky", "amarchy"));
        assert!(words_match("omarky", "amarky"));
        assert!(words_match("demon", "daemon"));
        assert!(!words_match("package", "problems"));
        assert!(!words_match("yes", "no"));
        assert!(!words_match("ok", "on"));
        assert_eq!(sound_key("Amarchy"), "mrk");
        assert_eq!(sound_key("Omarky"), "mrk");
        assert!(words_match("5", "five"));
        assert!(words_match("ten", "10"));
        assert!(!words_match("5", "six"));
    }

    /// Echo fragments the 1.2.1 demo left under "You", against the remote
    /// line they came from (exports/ and the 1.3.0 replay), and real lines
    /// of yours from the same meeting that sit next to remote speech.
    #[test]
    fn demo_echo_fragments_and_real_replies() {
        let tested = words(
            "Great! I tested the Amarky package this morning, and the settings panel opened without any problems.",
        );
        let banner = words(
            "Only one. The restart banner stays visible after the demon restarts. I think we can ship with that as a known issue.",
        );
        let draft = words("I'll draft the notes today and send them to you by five.");
        for (heard, remote) in [
            ("Omarky problem.", &tested),
            ("Amarky", &tested),
            ("Omarky", &tested),
            ("after the daemon restarts.", &banner),
            ("the daemon restarts.", &banner),
            ("daemon", &banner),
            ("known issue.", &banner),
            ("5.", &draft),
        ] {
            assert!(fuzzy_echo(&words(heard), remote, 4), "{heard:?} is echo");
        }
        for (said, remote) in [
            ("Good.", &tested),
            ("Agreed.", &banner),
            ("Thanks, Dana.", &draft),
            ("Perfect, then we tag", &draft),
        ] {
            assert!(!fuzzy_echo(&words(said), remote, 4), "{said:?} is yours");
        }
    }

    #[test]
    fn the_demo_fragment_is_echo_of_the_remote_line() {
        let remote =
            words("Great! I tested the Amarchy package this morning, and the settings panel opened without any problems.");
        assert!(fuzzy_echo(&words("Omarky problem."), &remote, 4));
        assert!(fuzzy_echo(&words("Amarky."), &remote, 4));
        // Most, not all: one stray word in four is still echo.
        assert!(fuzzy_echo(&words("tested the omarky package"), &remote, 4));
        // Your own words are not.
        assert!(!fuzzy_echo(&words("Let's tag it."), &remote, 4));
        assert!(!fuzzy_echo(&words("Okay problem"), &remote, 4));
        // Longer lines are the phrase rule's business.
        assert!(!fuzzy_echo(
            &words("tested the omarky package this"),
            &remote,
            4
        ));
    }

    const RATE: u32 = 16000;

    fn tone(len_s: f32, spans: &[(f32, f32, f32)]) -> Vec<f32> {
        let mut v = vec![0.0f32; (len_s * RATE as f32) as usize];
        for &(a, b, amp) in spans {
            let (a, b) = ((a * RATE as f32) as usize, (b * RATE as f32) as usize);
            for (i, s) in v.iter_mut().enumerate().take(b).skip(a) {
                *s += amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / RATE as f32).sin();
            }
        }
        v
    }

    /// The remote track, and the microphone hearing it at 15% 60 ms late.
    fn with_echo(remote: &[f32], own: &[f32]) -> Vec<f32> {
        let delay = (0.06 * RATE as f32) as usize;
        own.iter()
            .enumerate()
            .map(|(i, &o)| {
                o + if i >= delay {
                    0.15 * remote[i - delay]
                } else {
                    0.0
                }
            })
            .collect()
    }

    #[test]
    fn the_loopback_holds_speech_only_where_it_is_voiced() {
        let remote = tone(6.0, &[(1.0, 3.0, 0.1)]);
        let mic = tone(6.0, &[]);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(probe.remote_speech(1_200, 2_000));
        assert!(!probe.remote_speech(3_500, 5_500));
        // A span the chunk does not cover is not known to be silent.
        assert!(probe.remote_speech(7_000, 8_000));
        let late = EchoProbe::new(&mic, 30_000, &remote, 30_000, RATE, 0.01);
        assert!(late.remote_speech(29_000, 29_500));
        assert!(!late.remote_speech(33_500, 35_000));
    }

    #[test]
    fn echo_alone_is_not_your_voice() {
        let remote = tone(10.0, &[(1.0, 6.0, 0.5)]);
        let mic = with_echo(&remote, &vec![0.0; remote.len()]);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(!probe.own_voice(2000, 3000));
    }

    #[test]
    fn your_reply_over_the_remote_is_your_voice() {
        let remote = tone(10.0, &[(1.0, 6.0, 0.5)]);
        // You say "yes" at 3-3.5 s at a normal level while Dana talks.
        let own = tone(10.0, &[(3.0, 3.5, 0.3)]);
        let mic = with_echo(&remote, &own);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(probe.own_voice(2800, 3700));
        // The echo either side of it is still echo.
        assert!(!probe.own_voice(1500, 2500));
    }

    #[test]
    fn speech_while_the_remote_is_silent_is_yours() {
        let remote = tone(10.0, &[(1.0, 4.0, 0.5)]);
        let mic = with_echo(&remote, &tone(10.0, &[(6.0, 6.5, 0.05)]));
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(probe.own_voice(5900, 6600));
    }

    /// The remote track at 0.3 into the microphone, 60 ms late.
    fn speaker_echo(remote: &[f32], own: &[f32]) -> Vec<f32> {
        let delay = (0.06 * RATE as f32) as usize;
        own.iter()
            .enumerate()
            .map(|(i, &o)| {
                o + if i >= delay {
                    0.3 * remote[i - delay]
                } else {
                    0.0
                }
            })
            .collect()
    }

    #[test]
    fn a_long_echo_is_echo_only() {
        let remote = tone(20.0, &[(1.0, 15.0, 0.5)]);
        let mic = speaker_echo(&remote, &vec![0.0; remote.len()]);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(probe.echo_only(1000, 15000), "14 s of echo");
        assert!(!probe.own_voice(1000, 15000));
    }

    /// A line that is partly yours is never echo, however much echo it has.
    #[test]
    fn a_line_mixing_echo_and_your_voice_is_not_echo() {
        let remote = tone(20.0, &[(1.0, 15.0, 0.5)]);
        // Your voice louder than the speakers' echo, as at a desk.
        let own = tone(20.0, &[(8.0, 9.0, 0.5)]);
        let mic = speaker_echo(&remote, &own);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        assert!(!probe.echo_only(1000, 15000));
        assert!(probe.own_voice(1000, 15000));
        // The echo either side of you is still echo on its own.
        assert!(probe.echo_only(1000, 7500));
    }

    /// A hissing microphone and a low VAD threshold: the noise between the
    /// echo's words is not taken for your voice.
    #[test]
    fn microphone_noise_between_echoed_words_is_not_your_voice() {
        // Remote words with gaps, the echo, and a steady noise floor.
        let spans: Vec<(f32, f32, f32)> = (0..12)
            .map(|i| (1.0 + i as f32, 1.6 + i as f32, 0.5))
            .collect();
        let remote = tone(15.0, &spans);
        let mut mic = speaker_echo(&remote, &vec![0.0; remote.len()]);
        let mut seed = 1u32;
        for s in mic.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *s += ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.01;
        }
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.001);
        assert!(!probe.own_voice(1000, 13000));
        assert!(probe.echo_only(1000, 13000));
    }

    #[test]
    fn too_little_audio_or_no_echo_path_is_not_echo() {
        let remote = tone(20.0, &[(1.0, 15.0, 0.5)]);
        let mic = speaker_echo(&remote, &vec![0.0; remote.len()]);
        let probe = EchoProbe::new(&mic, 0, &remote, 0, RATE, 0.01);
        // 210 ms of voiced audio is too little to judge.
        assert!(!probe.echo_only(2000, 2210));
        // Remote silent all chunk: no path, nothing is echo.
        let quiet = vec![0.0f32; remote.len()];
        let probe = EchoProbe::new(&mic, 0, &quiet, 0, RATE, 0.01);
        assert!(!probe.echo_only(1000, 15000));
    }

    #[test]
    fn same_speech_confirms_on_part_of_a_long_line_but_not_on_small_words() {
        let remote = words(
            "Only one. The restart banner stays visible after the demon restarts. I think we can ship with that as a known issue.",
        );
        assert!(same_speech(
            &words("only won the restore banner stay visible after the daemon restart i thing we can shit"),
            &remote
        ));
        // Two real words out of a long garble is enough: the audio made the case.
        assert!(same_speech(
            &words("la la banner la la la visible la la la"),
            &remote
        ));
        assert!(!same_speech(
            &words("mm hmm yeah yeah okay sure thing"),
            &remote
        ));
        assert!(!same_speech(
            &words("and the and the and with that as"),
            &remote
        ));
        assert!(same_speech(&words("known issue"), &remote));
        assert!(!same_speech(&words("sure thing"), &remote));
    }

    #[test]
    fn without_remote_speech_nothing_is_called_echo() {
        let quiet = vec![0.0f32; 5 * RATE as usize];
        let probe = EchoProbe::new(&tone(5.0, &[(1.0, 2.0, 0.1)]), 0, &quiet, 0, RATE, 0.01);
        assert!(probe.own_voice(1000, 2000));
    }
}
