use std::time::Duration;

use tokio::sync::mpsc;
use voxtype::audio::input_guard::DeadInputGuard;
use voxtype::audio::levels::spawn_chunk_pump;
use voxtype::config::DeadInputConfig;

#[tokio::test]
async fn guard_only_pump_reports_dead_input() {
    let config = DeadInputConfig {
        enabled: true,
        window_ms: 300,
        threshold_dbfs: -100.0,
    };
    let (guard, mut reports) = DeadInputGuard::new(1_000, &config);
    let (samples_tx, samples_rx) = mpsc::channel(1);
    let pump = spawn_chunk_pump(samples_rx, None, None, None, Some(guard));

    samples_tx.send(vec![0.0; 400]).await.unwrap();

    let report = tokio::time::timeout(Duration::from_secs(1), reports.recv())
        .await
        .expect("the pump should report a dead input")
        .expect("the guard should send a report");
    assert_eq!(report.peak_dbfs, f32::NEG_INFINITY);
    assert_eq!(report.window_ms, 300);
    assert_eq!(report.threshold_dbfs, -100.0);

    drop(samples_tx);
    pump.await.unwrap();
}

#[tokio::test]
async fn live_input_closes_the_report_channel_and_keeps_streaming() {
    let config = DeadInputConfig {
        enabled: true,
        window_ms: 300,
        threshold_dbfs: -100.0,
    };
    let (guard, mut reports) = DeadInputGuard::new(1_000, &config);
    let (samples_tx, samples_rx) = mpsc::channel(2);
    let (streaming_tx, mut streaming_rx) = mpsc::channel(2);
    let pump = spawn_chunk_pump(samples_rx, None, Some(streaming_tx), None, Some(guard));
    let first: Vec<f32> = (0..200)
        .map(|index| if index % 2 == 0 { -0.01 } else { 0.01 })
        .collect();
    let second = first.clone();

    samples_tx.send(first.clone()).await.unwrap();
    samples_tx.send(second.clone()).await.unwrap();

    assert_eq!(streaming_rx.recv().await, Some(first));
    assert_eq!(streaming_rx.recv().await, Some(second));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), reports.recv())
            .await
            .expect("a live input should settle the guard"),
        None
    );

    drop(samples_tx);
    pump.await.unwrap();
}
