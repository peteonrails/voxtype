use crate::transcribe::{TranscribeOutput, WordInfo};

#[derive(Debug, Default)]
pub struct HypothesisBuffer {
    committed_in_buffer: Vec<WordInfo>,
    buffer: Vec<WordInfo>,
    new: Vec<WordInfo>,
    last_committed_time: i64,
}

impl HypothesisBuffer {
    pub fn new() -> Self {
        Self {
            committed_in_buffer: vec![],
            buffer: vec![],
            new: vec![],
            last_committed_time: 0,
        }
    }

    pub fn insert(&mut self, new_words: Vec<WordInfo>) {
        self.new = new_words
            .into_iter()
            .filter(|word| word.start > self.last_committed_time - 10)
            .collect();

        if self.new.is_empty() {
            return;
        }

        if (self.new[0].start - self.last_committed_time).abs() < 100
            && !self.committed_in_buffer.is_empty()
        {
            let committed_len = self.committed_in_buffer.len();
            let new_len = self.new.len();
            let max_ngram = committed_len.min(new_len).min(5);

            for n in 1..=max_ngram {
                let committed_suffix = &self.committed_in_buffer[committed_len - n..committed_len];
                let new_prefix = &self.new[0..n];

                let matches = committed_suffix
                    .iter()
                    .zip(new_prefix.iter())
                    .all(|(a, b)| a.text == b.text);

                if matches {
                    self.new.drain(0..n);
                    break;
                }
            }
        }
    }

    pub fn flush(&mut self) -> Vec<WordInfo> {
        let mut committed = Vec::new();

        while !self.new.is_empty() && !self.buffer.is_empty() {
            if self.new[0].text != self.buffer[0].text {
                break;
            }

            let word = self.new.remove(0);
            self.last_committed_time = word.end;
            committed.push(word);
            self.buffer.remove(0);
        }

        self.buffer = self.new.clone();
        self.new.clear();
        self.committed_in_buffer.extend(committed.clone());

        committed
    }

    pub fn complete(&self) -> Vec<WordInfo> {
        self.buffer.clone()
    }

    pub fn pop_committed(&mut self, time: i64) {
        while !self.committed_in_buffer.is_empty() && self.committed_in_buffer[0].end <= time {
            self.committed_in_buffer.remove(0);
        }
    }
}

/// Configuration for the streaming session
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Sample rate (always 16000 for Whisper)
    pub sample_rate: u32,
    /// How often to re-transcribe in seconds (tick interval)
    pub tick_interval_secs: f32,
    /// Maximum active window size in seconds before trimming
    pub max_buffer_secs: f32,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            tick_interval_secs: 1.0,
            max_buffer_secs: 15.0,
        }
    }
}

/// Result from a streaming tick
pub struct TickResult {
    /// Newly committed text from this tick
    pub newly_committed: Vec<WordInfo>,
    /// All committed text so far
    pub all_committed: Vec<WordInfo>,
}

impl TickResult {
    pub fn committed_text(&self) -> String {
        self.newly_committed
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub struct StreamingSession {
    config: StreamingConfig,
    audio_buffer: Vec<f32>,
    buffer_time_offset: i64,
    hypothesis_buffer: HypothesisBuffer,
    committed: Vec<WordInfo>,
    prompt_tokens: Vec<i32>,
}

impl StreamingSession {
    pub fn new(config: StreamingConfig) -> Self {
        Self {
            config,
            audio_buffer: Vec::new(),
            buffer_time_offset: 0,
            hypothesis_buffer: HypothesisBuffer::new(),
            committed: Vec::new(),
            prompt_tokens: Vec::new(),
        }
    }

    pub fn push_audio(&mut self, samples: &[f32]) {
        self.audio_buffer.extend_from_slice(samples);
    }

    pub fn audio_buffer(&self) -> &[f32] {
        &self.audio_buffer
    }

    pub fn prompt_tokens(&self) -> Option<&[i32]> {
        if self.prompt_tokens.is_empty() {
            None
        } else {
            Some(&self.prompt_tokens)
        }
    }

    pub fn process_result(&mut self, output: &TranscribeOutput) -> TickResult {
        let words = output
            .words
            .iter()
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| WordInfo {
                text: word.text.clone(),
                start: word.start + self.buffer_time_offset,
                end: word.end + self.buffer_time_offset,
                tokens: word.tokens.clone(),
            })
            .collect::<Vec<_>>();

        self.hypothesis_buffer.insert(words);
        let newly_committed = self.hypothesis_buffer.flush();

        for word in &newly_committed {
            self.prompt_tokens.extend(&word.tokens);
        }

        if self.prompt_tokens.len() > 200 {
            let trim_from = self.prompt_tokens.len() - 200;
            self.prompt_tokens.drain(0..trim_from);
        }

        self.committed.extend(newly_committed.iter().cloned());

        if (self.audio_buffer.len() as f32 / self.config.sample_rate as f32)
            > self.config.max_buffer_secs
        {
            self.trim_buffer();
        }

        TickResult {
            newly_committed,
            all_committed: self.committed.clone(),
        }
    }

    pub fn finish(&mut self, final_output: &TranscribeOutput) -> String {
        self.process_result(final_output);

        let remaining = self
            .hypothesis_buffer
            .complete()
            .into_iter()
            .filter(|word| !word.text.trim().is_empty())
            .collect::<Vec<_>>();

        for word in &remaining {
            self.prompt_tokens.extend(&word.tokens);
        }

        if self.prompt_tokens.len() > 200 {
            let trim_from = self.prompt_tokens.len() - 200;
            self.prompt_tokens.drain(0..trim_from);
        }

        self.committed.extend(remaining);

        self.committed
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn should_tick(&self) -> bool {
        (self.audio_buffer.len() as f32 / self.config.sample_rate as f32)
            >= self.config.tick_interval_secs
    }

    fn trim_buffer(&mut self) {
        let Some(last_committed_end) = self.committed.last().map(|word| word.end) else {
            return;
        };

        if last_committed_end <= self.buffer_time_offset {
            return;
        }

        let samples_per_centisecond = self.config.sample_rate as usize / 100;
        let relative_end_cs = (last_committed_end - self.buffer_time_offset) as usize;
        let trim_samples = relative_end_cs
            .saturating_mul(samples_per_centisecond)
            .min(self.audio_buffer.len());

        if trim_samples == 0 {
            return;
        }

        self.audio_buffer.drain(0..trim_samples);

        let advanced_cs = (trim_samples / samples_per_centisecond) as i64;
        self.buffer_time_offset += advanced_cs;

        self.hypothesis_buffer
            .pop_committed(self.buffer_time_offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::{TranscribeOutput, WordInfo};

    fn w(text: &str, start: i64, end: i64) -> WordInfo {
        WordInfo {
            text: text.to_string(),
            start,
            end,
            tokens: vec![],
        }
    }

    #[test]
    fn test_empty_buffer_produces_nothing() {
        let mut buf = HypothesisBuffer::new();
        assert!(buf.flush().is_empty());
    }

    #[test]
    fn test_single_insert_produces_nothing() {
        let mut buf = HypothesisBuffer::new();
        buf.insert(vec![w("hello", 0, 50), w("world", 50, 100)]);
        assert!(buf.flush().is_empty());
    }

    #[test]
    fn test_two_agreeing_inserts_commits() {
        let mut buf = HypothesisBuffer::new();
        buf.insert(vec![w("hello", 0, 50), w("world", 50, 100)]);
        buf.flush();
        buf.insert(vec![
            w("hello", 0, 50),
            w("world", 50, 100),
            w("foo", 100, 150),
        ]);
        let committed = buf.flush();
        assert_eq!(committed.len(), 2);
        assert_eq!(committed[0].text, "hello");
        assert_eq!(committed[1].text, "world");
    }

    #[test]
    fn test_disagreement_stops_commit() {
        let mut buf = HypothesisBuffer::new();
        buf.insert(vec![w("hello", 0, 50), w("world", 50, 100)]);
        buf.flush();
        buf.insert(vec![w("hello", 0, 50), w("planet", 50, 100)]);
        let committed = buf.flush();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "hello");
    }

    #[test]
    fn test_ngram_dedup_on_insert() {
        let mut buf = HypothesisBuffer::new();
        buf.insert(vec![w("hello", 0, 50), w("world", 50, 100)]);
        buf.flush();
        buf.insert(vec![
            w("hello", 0, 50),
            w("world", 50, 100),
            w("foo", 100, 150),
        ]);
        buf.flush();
        buf.insert(vec![
            w("world", 90, 100),
            w("foo", 100, 150),
            w("bar", 150, 200),
        ]);
        let committed = buf.flush();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "foo");
    }

    #[test]
    fn test_complete_returns_uncommitted() {
        let mut buf = HypothesisBuffer::new();
        buf.insert(vec![w("hello", 0, 50), w("world", 50, 100)]);
        buf.flush();
        buf.insert(vec![
            w("hello", 0, 50),
            w("world", 50, 100),
            w("foo", 100, 150),
        ]);
        buf.flush();
        let remaining = buf.complete();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].text, "foo");
    }

    #[test]
    fn test_session_first_tick_commits_nothing() {
        let mut session = StreamingSession::new(StreamingConfig::default());
        session.push_audio(&vec![0.0f32; 16000]);
        let output = TranscribeOutput {
            text: "hello world".to_string(),
            words: vec![
                WordInfo {
                    text: "hello".to_string(),
                    start: 0,
                    end: 50,
                    tokens: vec![1],
                },
                WordInfo {
                    text: "world".to_string(),
                    start: 50,
                    end: 100,
                    tokens: vec![2],
                },
            ],
        };
        let result = session.process_result(&output);
        assert!(result.committed_text().is_empty());
    }

    #[test]
    fn test_session_second_agreeing_tick_commits() {
        let mut session = StreamingSession::new(StreamingConfig::default());
        session.push_audio(&vec![0.0f32; 16000]);
        let output1 = TranscribeOutput {
            text: "hello world".to_string(),
            words: vec![
                WordInfo {
                    text: "hello".to_string(),
                    start: 0,
                    end: 50,
                    tokens: vec![1],
                },
                WordInfo {
                    text: "world".to_string(),
                    start: 50,
                    end: 100,
                    tokens: vec![2],
                },
            ],
        };
        session.process_result(&output1);

        session.push_audio(&vec![0.0f32; 16000]);
        let output2 = TranscribeOutput {
            text: "hello world foo".to_string(),
            words: vec![
                WordInfo {
                    text: "hello".to_string(),
                    start: 0,
                    end: 50,
                    tokens: vec![1],
                },
                WordInfo {
                    text: "world".to_string(),
                    start: 50,
                    end: 100,
                    tokens: vec![2],
                },
                WordInfo {
                    text: "foo".to_string(),
                    start: 100,
                    end: 150,
                    tokens: vec![3],
                },
            ],
        };

        let result = session.process_result(&output2);
        assert_eq!(result.committed_text(), "hello world");
    }

    #[test]
    fn test_session_finish_includes_uncommitted() {
        let mut session = StreamingSession::new(StreamingConfig::default());
        session.push_audio(&vec![0.0f32; 16000]);
        let output = TranscribeOutput {
            text: "hello world".to_string(),
            words: vec![
                WordInfo {
                    text: "hello".to_string(),
                    start: 0,
                    end: 50,
                    tokens: vec![1],
                },
                WordInfo {
                    text: "world".to_string(),
                    start: 50,
                    end: 100,
                    tokens: vec![2],
                },
            ],
        };

        session.process_result(&output);
        let final_text = session.finish(&output);
        assert!(final_text.contains("hello"));
        assert!(final_text.contains("world"));
    }

    #[test]
    fn test_session_prompt_tokens_accumulate() {
        let mut session = StreamingSession::new(StreamingConfig::default());
        session.push_audio(&vec![0.0f32; 16000]);
        let output = TranscribeOutput {
            text: "hello".to_string(),
            words: vec![WordInfo {
                text: "hello".to_string(),
                start: 0,
                end: 50,
                tokens: vec![42, 43],
            }],
        };

        session.process_result(&output);
        session.process_result(&output);
        assert_eq!(session.prompt_tokens(), Some(&[42, 43][..]));
    }
}
