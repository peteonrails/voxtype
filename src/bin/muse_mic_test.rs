//! Mic streaming test for Muse Voice Transcribe
//! Run: cargo run --bin muse-mic-test
//! Streams mic 16k mono to Muse WS and prints partial/final live.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;
use voxtype::transcribe::Transcriber;

fn main() -> anyhow::Result<()> {
    // Load Muse config from file + env, same as daemon
    let cfg = voxtype::config::load_config(None)?;
    let muse_cfg = cfg.muse.clone().unwrap_or_default();
    let transcriber = voxtype::transcribe::muse::MuseTranscriber::new(&muse_cfg)?;
    let streaming = transcriber
        .as_streaming()
        .expect("muse streaming disabled in config");
    println!(
        "Muse mic test: endpoint={} ws={} model={} interim={}",
        muse_cfg.effective_endpoint(),
        muse_cfg.effective_ws_endpoint(),
        muse_cfg.model,
        muse_cfg.interim_results
    );
    if muse_cfg.resolve_api_key().is_none() {
        anyhow::bail!("No Muse API key: set [muse] api_key or VOXTYPE_MUSE_API_KEY");
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let (tx, rx) = mpsc::channel::<Vec<f32>>(32);
        let handle = streaming.start_stream(rx)?;
        // Spawn printer
        let mut events = handle.events;
        let printer = tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    voxtype::transcribe::StreamingEvent::Partial { text, .. } => {
                        print!("\r\x1b[2K[partial] {}", text);
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                    voxtype::transcribe::StreamingEvent::Final { text, .. } => {
                        println!("\n[final] {}", text);
                    }
                    voxtype::transcribe::StreamingEvent::Replace {
                        backspace, text, ..
                    } => {
                        println!("\n[replace backspace={} text={:?}]", backspace, text);
                    }
                    voxtype::transcribe::StreamingEvent::Error(e) => {
                        eprintln!("\n[error] {}", e);
                    }
                    voxtype::transcribe::StreamingEvent::Ended => {
                        println!("\n[ended]");
                        break;
                    }
                }
            }
        });

        // Capture mic
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No input device"))?;
        println!("Using input device: {}", device.name()?);
        let supported = device.default_input_config()?;
        // Force 16k mono f32 if device supports, otherwise use default and resample in daemon would handle
        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(16000),
            buffer_size: cpal::BufferSize::Default,
        };
        // If device doesn't support 16k, fallback to default (streaming expects 16k)
        let use_config = if device.supported_input_configs().unwrap().any(|c| {
            c.min_sample_rate().0 <= 16000 && c.max_sample_rate().0 >= 16000 && c.channels() == 1
        }) {
            config
        } else {
            println!(
                "Device doesn't support 16k mono, using default {:?}",
                supported.config()
            );
            supported.into()
        };

        let tx_clone = tx.clone();
        let err_fn = |err| eprintln!("Stream error: {}", err);
        let data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Chunk 80ms = 1280 samples @16k, but send whatever we get
            let _ = tx_clone.try_send(data.to_vec());
        };
        let stream = device.build_input_stream(&use_config, data_fn, err_fn, None)?;
        stream.play()?;
        println!("Listening... speak for 10s (Ctrl-C to stop)");
        // Keep tx alive via stream; drop after timeout
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        drop(stream);
        drop(tx);
        println!("\nMic closed, waiting for finals...");
        let _ = printer.await;
        // Cancel if still running
        let _ = handle.cancel.send(());
        handle.task.await.unwrap().unwrap();
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}
