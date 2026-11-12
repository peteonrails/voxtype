//! Per-track level meters for a recording meeting, published on
//! meeting.sock as `levels` events about ten times a second.
//!
//! The meters are fed by the audio the meeting already captures: the
//! microphone's chunk stream from cpal and the loopback reader thread. They
//! only add up squares and peaks, so feeding them costs the capture next to
//! nothing and never waits on a client. A publisher task reads them on a
//! timer and sends an event only while a client is connected.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often levels are sent.
pub const LEVELS_INTERVAL: Duration = Duration::from_millis(100);

/// The floor for silence, in dBFS.
pub const SILENCE_DBFS: f32 = -100.0;

/// Sample rate of the meeting tracks (16 kHz mono after resampling).
const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Default)]
struct Accum {
    sum_sq: f64,
    samples: u64,
    peak: f32,
}

/// Running level of one track since it was last read.
#[derive(Debug, Default)]
pub struct TrackMeter {
    acc: Mutex<Accum>,
}

/// One track's level over one window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// Root mean square, 0.0 to 1.0
    pub rms: f32,
    /// Largest absolute sample, 0.0 to 1.0
    pub peak: f32,
    /// Samples in the window
    pub samples: u64,
}

impl TrackMeter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Add samples as they arrive from the capture.
    pub fn feed(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let (mut sum_sq, mut peak) = (0.0f64, 0.0f32);
        for &s in samples {
            sum_sq += (s as f64) * (s as f64);
            peak = peak.max(s.abs());
        }
        if let Ok(mut acc) = self.acc.lock() {
            acc.sum_sq += sum_sq;
            acc.samples += samples.len() as u64;
            acc.peak = acc.peak.max(peak);
        }
    }

    /// The level since the last call, and start a new window. `None` when
    /// no samples arrived.
    pub fn take(&self) -> Option<Window> {
        let acc = std::mem::take(&mut *self.acc.lock().ok()?);
        (acc.samples > 0).then(|| Window {
            rms: (acc.sum_sq / acc.samples as f64).sqrt().min(1.0) as f32,
            peak: acc.peak.min(1.0),
            samples: acc.samples,
        })
    }
}

/// A level in dBFS, rounded to 0.1 dB, with silence at [`SILENCE_DBFS`].
pub fn dbfs(level: f32) -> f32 {
    if level <= 1e-5 {
        return SILENCE_DBFS;
    }
    ((20.0 * level.log10()) * 10.0).round() / 10.0
}

/// The meters of one meeting's capture.
#[derive(Debug, Clone)]
pub struct MeetingMeters {
    pub mic: Arc<TrackMeter>,
    /// `None` when the meeting records no loopback.
    pub loopback: Option<Arc<TrackMeter>>,
}

impl MeetingMeters {
    pub fn new(with_loopback: bool) -> Self {
        Self {
            mic: TrackMeter::new(),
            loopback: with_loopback.then(TrackMeter::new),
        }
    }
}

/// A `levels` event's content. Levels are dBFS of the window's RMS and peak.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Levels {
    /// The meeting measured; `None` during a sound check.
    pub meeting_id: Option<String>,
    /// Present (and true) only during a sound check, which records nothing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub soundcheck: bool,
    /// Where the window ends on the meeting's timeline, in ms: microphone
    /// audio recorded so far, paused time left out, like transcript times.
    pub t_ms: u64,
    pub mic: Option<f32>,
    pub mic_peak: Option<f32>,
    /// `None` when there is no loopback track or no loopback audio in the
    /// window.
    pub loopback: Option<f32>,
    pub loopback_peak: Option<f32>,
}

/// Read `meters` every [`LEVELS_INTERVAL`] and hand a [`Levels`] to `send`
/// while `active` (recording, not paused) and `has_clients()`. Windows are
/// read and dropped otherwise, so a client that connects sees the current
/// level, not a backlog. Runs until the task is aborted.
pub async fn run_publisher(
    meters: MeetingMeters,
    meeting_id: Option<String>,
    active: Arc<AtomicBool>,
    has_clients: impl Fn() -> bool + Send,
    send: impl Fn(Levels) + Send,
) {
    let mut tick = tokio::time::interval(LEVELS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut timeline_samples: u64 = 0;
    loop {
        tick.tick().await;
        let mic = meters.mic.take();
        let loopback = meters.loopback.as_ref().and_then(|m| m.take());
        if !active.load(Ordering::Relaxed) {
            continue;
        }
        timeline_samples += mic.map_or(0, |w| w.samples);
        if (mic.is_none() && loopback.is_none()) || !has_clients() {
            continue;
        }
        send(Levels {
            soundcheck: meeting_id.is_none(),
            meeting_id: meeting_id.clone(),
            t_ms: timeline_samples * 1000 / SAMPLE_RATE,
            mic: mic.map(|w| dbfs(w.rms)),
            mic_peak: mic.map(|w| dbfs(w.peak)),
            loopback: loopback.map(|w| dbfs(w.rms)),
            loopback_peak: loopback.map(|w| dbfs(w.peak)),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin())
            .collect()
    }

    #[test]
    fn a_sine_reads_its_rms_and_peak() {
        let m = TrackMeter::new();
        // 100 ms of a 0.5 sine: RMS 0.5/sqrt(2), -9.0 dBFS; peak -6.0 dBFS.
        m.feed(&tone(0.5, 800));
        m.feed(&tone(0.5, 800)[..800]);
        let w = m.take().unwrap();
        assert_eq!(w.samples, 1600);
        assert!((w.rms - 0.3536).abs() < 0.002, "{w:?}");
        assert_eq!(dbfs(w.rms), -9.0);
        assert_eq!(dbfs(w.peak), -6.0);
        // The window starts over.
        assert_eq!(m.take(), None);
    }

    #[test]
    fn quiet_and_silent_levels() {
        let m = TrackMeter::new();
        m.feed(&tone(0.01, 1600));
        assert_eq!(dbfs(m.take().unwrap().rms), -43.0);
        m.feed(&[0.0; 1600]);
        assert_eq!(dbfs(m.take().unwrap().rms), SILENCE_DBFS);
        assert_eq!(dbfs(1.0), 0.0);
    }

    /// Feeds 100 ms of tone per tick on the mic and silence on the
    /// loopback, under tokio's paused clock.
    async fn run_for(ticks: u32, clients: Arc<AtomicBool>, active: Arc<AtomicBool>) -> Vec<Levels> {
        let meters = MeetingMeters::new(true);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let task = {
            let (meters, sent, clients) = (meters.clone(), sent.clone(), clients.clone());
            tokio::spawn(run_publisher(
                meters,
                Some("m1".into()),
                active,
                move || clients.load(Ordering::Relaxed),
                move |l| sent.lock().unwrap().push(l),
            ))
        };
        tokio::task::yield_now().await;
        for _ in 0..ticks {
            meters.mic.feed(&tone(0.5, 1600));
            meters.loopback.as_ref().unwrap().feed(&[0.0; 1600]);
            tokio::time::advance(LEVELS_INTERVAL).await;
            tokio::task::yield_now().await;
        }
        task.abort();
        let out = sent.lock().unwrap().clone();
        out
    }

    #[tokio::test(start_paused = true)]
    async fn ten_events_a_second_with_the_levels_of_each_track() {
        let sent = run_for(
            20,
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
        )
        .await;
        // 2 s: about 20 events, never more than one per 100 ms tick.
        assert!((19..=21).contains(&sent.len()), "{}", sent.len());
        let last = sent.last().unwrap();
        assert_eq!(last.meeting_id.as_deref(), Some("m1"));
        assert!(!last.soundcheck);
        assert_eq!(last.mic, Some(-9.0));
        assert_eq!(last.mic_peak, Some(-6.0));
        assert_eq!(last.loopback, Some(SILENCE_DBFS));
        // The timeline follows the microphone audio: 100 ms per tick.
        let steps: Vec<u64> = sent.windows(2).map(|w| w[1].t_ms - w[0].t_ms).collect();
        assert!(steps.iter().all(|&s| s == 100), "{steps:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_sent_without_a_client_and_no_backlog_builds_up() {
        let clients = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(true));
        let meters = MeetingMeters::new(false);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let task = {
            let (m, s, c) = (meters.clone(), sent.clone(), clients.clone());
            tokio::spawn(run_publisher(
                m,
                Some("m1".into()),
                active,
                move || c.load(Ordering::Relaxed),
                move |l| s.lock().unwrap().push(l),
            ))
        };
        tokio::task::yield_now().await;
        for _ in 0..10 {
            meters.mic.feed(&tone(0.5, 1600));
            tokio::time::advance(LEVELS_INTERVAL).await;
            tokio::task::yield_now().await;
        }
        assert!(sent.lock().unwrap().is_empty());

        // A client connects: the first event is the current window only,
        // at the right place on the timeline, with no loopback track.
        clients.store(true, Ordering::Relaxed);
        meters.mic.feed(&tone(0.01, 1600));
        tokio::time::advance(LEVELS_INTERVAL).await;
        tokio::task::yield_now().await;
        task.abort();
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].mic, Some(-43.0));
        assert_eq!(sent[0].loopback, None);
        assert_eq!(sent[0].t_ms, 1100);
    }

    #[tokio::test(start_paused = true)]
    async fn a_paused_meeting_sends_nothing_and_its_time_does_not_count() {
        let active = Arc::new(AtomicBool::new(false));
        let sent = run_for(5, Arc::new(AtomicBool::new(true)), active.clone()).await;
        assert!(sent.is_empty());
    }
}
