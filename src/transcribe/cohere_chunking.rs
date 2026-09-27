//! Quiet-boundary segmentation shared by the Cohere ONNX and GGUF runtimes.

use crate::config::CohereConfig;
use crate::error::TranscribeError;
use std::ops::Range;

const SAMPLE_RATE: usize = 16_000;
const HALF_WINDOW: usize = SAMPLE_RATE / 20; // 50 ms on each side

#[derive(Clone, Copy)]
pub struct CohereChunking {
    max_samples: usize,
    search_samples: usize,
}

impl CohereChunking {
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        if !(5..=35).contains(&config.max_chunk_secs) {
            return Err(TranscribeError::ConfigError(
                "cohere.max_chunk_secs must be between 5 and 35".into(),
            ));
        }
        if !config.boundary_search_secs.is_finite()
            || !(0.0..=5.0).contains(&config.boundary_search_secs)
        {
            return Err(TranscribeError::ConfigError(
                "cohere.boundary_search_secs must be between 0 and 5".into(),
            ));
        }
        Ok(Self {
            max_samples: config.max_chunk_secs as usize * SAMPLE_RATE,
            search_samples: (config.boundary_search_secs * SAMPLE_RATE as f32) as usize,
        })
    }

    pub fn ranges(self, samples: &[f32]) -> Vec<Range<usize>> {
        if samples.is_empty() {
            return Vec::new();
        }
        let mut ranges = Vec::new();
        let mut start = 0;
        while samples.len() - start > self.max_samples {
            let remaining = samples.len() - start;
            let chunks = remaining.div_ceil(self.max_samples);
            let target = start + remaining.div_ceil(chunks);
            let low = target.saturating_sub(self.search_samples).max(start + 1);
            let high = target
                .saturating_add(self.search_samples)
                .min(start + self.max_samples)
                .min(samples.len() - 1);
            let cut = if self.search_samples == 0 {
                target
            } else {
                quietest_cut(samples, low, high, target)
            };
            ranges.push(start..cut);
            start = cut;
        }
        ranges.push(start..samples.len());
        ranges
    }

    pub fn transcribe<F>(self, samples: &[f32], mut infer: F) -> Result<String, TranscribeError>
    where
        F: FnMut(&[f32]) -> Result<String, TranscribeError>,
    {
        let mut parts = Vec::new();
        for range in self.ranges(samples) {
            let text = infer(&samples[range])?;
            if !text.trim().is_empty() {
                parts.push(text.trim().to_owned());
            }
        }
        Ok(parts.join(" "))
    }
}

fn quietest_cut(samples: &[f32], low: usize, high: usize, target: usize) -> usize {
    let mut left = low.saturating_sub(HALF_WINDOW);
    let mut right = (low + HALF_WINDOW).min(samples.len());
    let mut sum: f64 = samples[left..right]
        .iter()
        .map(|sample| sample.abs() as f64)
        .sum();
    let mut best = low;
    let mut best_average = sum / (right - left) as f64;
    for cut in low + 1..=high {
        let next_left = cut.saturating_sub(HALF_WINDOW);
        let next_right = (cut + HALF_WINDOW).min(samples.len());
        while left < next_left {
            sum -= samples[left].abs() as f64;
            left += 1;
        }
        while right < next_right {
            sum += samples[right].abs() as f64;
            right += 1;
        }
        let average = sum / (right - left) as f64;
        if average < best_average
            || (average == best_average && cut.abs_diff(target) < best.abs_diff(target))
        {
            best = cut;
            best_average = average;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_every_sample_once_within_limit() {
        let chunking = CohereChunking::new(&CohereConfig::default()).unwrap();
        for len in [
            0,
            1,
            35 * SAMPLE_RATE,
            35 * SAMPLE_RATE + 1,
            60 * SAMPLE_RATE,
            120 * SAMPLE_RATE,
        ] {
            let samples = vec![0.5; len];
            let ranges = chunking.ranges(&samples);
            let mut next = 0;
            for range in ranges {
                assert_eq!(range.start, next);
                assert!(range.end > range.start);
                assert!(range.len() <= chunking.max_samples);
                next = range.end;
            }
            assert_eq!(next, len);
        }
    }

    #[test]
    fn prefers_quiet_boundary_near_balanced_split() {
        let chunking = CohereChunking::new(&CohereConfig::default()).unwrap();
        let mut samples = vec![0.5; 60 * SAMPLE_RATE];
        samples[31 * SAMPLE_RATE - HALF_WINDOW..31 * SAMPLE_RATE + HALF_WINDOW].fill(0.0);
        let ranges = chunking.ranges(&samples);
        assert_eq!(ranges[0].end, 31 * SAMPLE_RATE);
    }

    #[test]
    fn rejects_invalid_settings() {
        let mut config = CohereConfig::default();
        config.max_chunk_secs = 36;
        assert!(CohereChunking::new(&config).is_err());
        config.max_chunk_secs = 35;
        config.boundary_search_secs = f32::NAN;
        assert!(CohereChunking::new(&config).is_err());
    }
}
