use crate::transcribe::WordInfo;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::WordInfo;

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
}
