//! Echo of the remote side on the microphone track, beyond exact words.
//!
//! With speakers instead of headphones, the microphone hears the remote
//! participants, and the speech engine transcribes that bleed under "You".
//! Long echoed phrases are caught by the 4-gram match and short ones by the
//! exact-words rule in [`crate::meeting::data::Transcript`]. Both miss a
//! fragment the engine *misheard*: the demo replay left "Omarky problem."
//! on the microphone while the loopback said "...the Amarchy package...
//! without any problems".
//!
//! Two pieces close that gap:
//!
//! - [`fuzzy_echo`]: a short microphone line whose words are, most of them,
//!   close to words of an overlapping loopback line, by spelling
//!   (normalized edit distance) or by sound (a consonant skeleton).
//! - [`EchoProbe`]: the guard. Your own short reply ("yes", "right") said
//!   while the remote also said it looks the same in text, but not in the
//!   audio: echo is the loopback, delayed and quieter; your voice is louder
//!   on the microphone than the echo path explains. A line with a run of
//!   such frames is yours and is kept by both short-line rules.

use crate::meeting::data::{AudioSource, TranscriptSegment};

/// Tunables for the short-line echo rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoRules {
    /// Drop misheard short echoes, not only exact ones.
    pub fuzzy: bool,
    /// The longest microphone line, in words, the fuzzy rule looks at.
    pub fuzzy_max_words: usize,
}

impl Default for EchoRules {
    fn default() -> Self {
        Self {
            fuzzy: true,
            fuzzy_max_words: 4,
        }
    }
}

impl From<&crate::config::MeetingAudioConfig> for EchoRules {
    fn from(audio: &crate::config::MeetingAudioConfig) -> Self {
        Self {
            fuzzy: audio.echo_fuzzy,
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

/// Energy of one chunk of both tracks, for telling your voice from echo.
#[derive(Debug, Clone)]
pub struct EchoProbe {
    mic: Vec<f32>,
    loopback: Vec<f32>,
    mic_start_ms: u64,
    loopback_start_ms: u64,
    threshold: f32,
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
            path: None,
        };
        probe.path = probe.measure_path();
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

    /// Whether the microphone between `start_ms` and `end_ms` holds speech
    /// of its own: a run of frames louder than the echo path explains, or
    /// voiced while the remote is silent. Unknown audio counts as yours, so
    /// a line is never dropped on a guess.
    pub fn own_voice(&self, start_ms: u64, end_ms: u64) -> bool {
        let Some((lag, ratio)) = self.path else {
            return true;
        };
        let first = start_ms.saturating_sub(self.mic_start_ms) / FRAME_MS;
        let last = end_ms.saturating_sub(self.mic_start_ms).div_ceil(FRAME_MS);
        if first as usize >= self.mic.len() {
            return true;
        }
        let offset = self.loopback_start_ms as i64 - self.mic_start_ms as i64;
        let shift = offset / FRAME_MS as i64;
        let mut run = 0;
        for i in first as usize..(last as usize).min(self.mic.len()) {
            let m = self.mic[i];
            if m <= self.threshold {
                run = 0;
                continue;
            }
            // The loudest the echo could be here, allowing a frame of slack
            // either side of the measured delay.
            let centre = i as i64 - shift - lag as i64;
            let remote = (centre - 1..=centre + 1)
                .filter_map(|j| self.loopback.get(usize::try_from(j).ok()?))
                .fold(0.0f32, |a, &b| a.max(b));
            let explained = ratio * remote;
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

    #[test]
    fn without_remote_speech_nothing_is_called_echo() {
        let quiet = vec![0.0f32; 5 * RATE as usize];
        let probe = EchoProbe::new(&tone(5.0, &[(1.0, 2.0, 0.1)]), 0, &quiet, 0, RATE, 0.01);
        assert!(probe.own_voice(1000, 2000));
    }
}
