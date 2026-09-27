//! Benchmark the real Cohere transcriber without starting a daemon or typing text.
//! See docs/COHERE_GPU_BENCHMARK.md for a reproducible CPU/GPU comparison.

use anyhow::{bail, Context};
use clap::Parser;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;
use voxtype::transcribe::{cohere::CohereTranscriber, Transcriber};

#[derive(Parser)]
struct Args {
    /// Isolated config selecting language, threads, model, and encoder backend.
    #[arg(long)]
    config: PathBuf,
    /// Human speech recordings: mono, 16 kHz, PCM16 WAV. Repeat to exercise
    /// changing recording lengths on the same transcriber (all one language).
    #[arg(long, required = true)]
    audio: Vec<PathBuf>,
    /// Timed warm inferences per clip, after first inference and extra warmups.
    #[arg(long, default_value_t = 5)]
    runs: usize,
    /// Extra warmups per clip after the separately measured first inference.
    #[arg(long, default_value_t = 2)]
    warmups: usize,
    /// Load, run the playlist, and drop the model this many times in one process.
    #[arg(long, default_value_t = 1)]
    reloads: usize,
    /// Permit empty WAVs for edge-case testing (their real-time factor is null).
    #[arg(long)]
    allow_empty: bool,
    /// Record inference failures and continue the playlist; still exit nonzero
    /// after writing the report if any call failed. For diagnostic stress runs.
    #[arg(long)]
    continue_on_error: bool,
}

struct Audio {
    path: PathBuf,
    samples: Vec<f32>,
}

fn read_audio(path: &Path, allow_empty: bool) -> anyhow::Result<Audio> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        bail!(
            "expected mono, 16 kHz, PCM16 WAV; convert with ffmpeg -ac 1 -ar 16000 -c:a pcm_s16le"
        );
    }
    let samples = reader
        .into_samples::<i16>()
        .map(|s| s.map(|s| s as f32 / 32768.0))
        .collect::<Result<Vec<_>, _>>()?;
    if samples.is_empty() && !allow_empty {
        bail!(
            "empty audio: {} (use --allow-empty for edge-case testing)",
            path.display()
        );
    }
    Ok(Audio {
        path: path.to_owned(),
        samples,
    })
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    }
}

/// Resident process memory, not GPU allocation accounting. Missing off Linux.
fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

/// Linux DRM client accounting, separate from RSS (never add the two: they
/// can overlap on an iGPU). Empty means no client exposed usable accounting.
fn drm_memory_kib() -> Option<Vec<Value>> {
    let entries = std::fs::read_dir("/proc/self/fdinfo").ok()?;
    let mut clients = std::collections::BTreeMap::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let fields: std::collections::BTreeMap<_, _> = text
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim(), v.trim()))
            .collect();
        let (Some(driver), Some(id)) = (fields.get("drm-driver"), fields.get("drm-client-id"))
        else {
            continue;
        };
        let pci = fields.get("drm-pdev").copied().unwrap_or("");
        let memory: serde_json::Map<String, Value> = fields
            .iter()
            .filter_map(|(key, value)| {
                if ![
                    "drm-total-",
                    "drm-resident-",
                    "drm-shared-",
                    "drm-active-",
                    "drm-purgeable-",
                    "drm-memory-",
                ]
                .iter()
                .any(|prefix| key.starts_with(prefix))
                {
                    return None;
                }
                let mut parts = value.split_whitespace();
                let amount = parts.next()?.parse::<u64>().ok()?;
                if parts.next()? != "KiB" {
                    return None;
                }
                Some(((*key).to_owned(), json!(amount)))
            })
            .collect();
        // Duplicate fds can expose the same client. Count it once.
        clients.insert(
            format!("{driver}:{pci}:{id}"),
            json!({"driver":driver,"pci":pci,"client_id":id,"memory_kib":memory}),
        );
    }
    Some(clients.into_values().collect())
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.runs == 0 || args.reloads == 0 {
        bail!("--runs and --reloads must be at least 1");
    }
    let rounds = args
        .warmups
        .checked_add(args.runs)
        .and_then(|n| n.checked_add(1))
        .context("too many benchmark rounds")?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let config = voxtype::config::load_config(Some(&args.config))?;
    let cohere = config.cohere.context("config needs a [cohere] section")?;
    let audio: Vec<_> = args
        .audio
        .iter()
        .map(|p| read_audio(p, args.allow_empty))
        .collect::<Result<_, _>>()?;
    let mut sessions = Vec::new();
    let mut had_errors = false;
    for reload in 0..args.reloads {
        let rss_before_load = rss_kib();
        let start = Instant::now();
        let transcriber = CohereTranscriber::new(&cohere)?;
        let load_secs = start.elapsed().as_secs_f64();
        let rss_after_load = rss_kib();
        let drm_after_load = drm_memory_kib();
        let mut measurements: Vec<Vec<Value>> = vec![Vec::new(); audio.len()];
        let mut timed = vec![Vec::new(); audio.len()];
        // Interleave clip lengths instead of warming each shape in isolation.
        for round in 0..rounds {
            let kind = if round == 0 {
                "first"
            } else if round <= args.warmups {
                "warmup"
            } else {
                "measured"
            };
            for (index, clip) in audio.iter().enumerate() {
                let start = Instant::now();
                let result = transcriber.transcribe(&clip.samples);
                let seconds = start.elapsed().as_secs_f64();
                let measurement = match result {
                    Ok(text) => {
                        if kind == "measured" {
                            timed[index].push(seconds);
                        }
                        json!({"kind":kind,"seconds":seconds,"text":text,"rss_kib":rss_kib()})
                    }
                    Err(error) if args.continue_on_error => {
                        had_errors = true;
                        json!({"kind":kind,"seconds":seconds,"error":error.to_string(),"rss_kib":rss_kib()})
                    }
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("transcribing {}", clip.path.display()))
                    }
                };
                let mut measurement = measurement;
                measurement["drm_clients"] = json!(drm_memory_kib());
                // A long soak still leaves progress if the runtime aborts.
                tracing::debug!(reload, round, audio = %clip.path.display(), seconds, rss_kib = ?rss_kib(), "Cohere benchmark sample completed");
                measurements[index].push(measurement);
            }
        }
        let rss_before_drop = rss_kib();
        let drm_before_drop = drm_memory_kib();
        drop(transcriber);
        let rss_after_drop = rss_kib();
        let drm_after_drop = drm_memory_kib();
        let clips: Vec<_> = audio
            .iter()
            .enumerate()
            .map(|(i, clip)| {
                let duration = clip.samples.len() as f64 / 16_000.0;
                let median = if timed[i].is_empty() {
                    None
                } else {
                    Some(median(&mut timed[i]))
                };
                json!({"audio":clip.path,"audio_seconds":duration,"warm_median_seconds":median,
                "warm_realtime_factor":if duration > 0.0 {median.map(|m| m / duration)} else {None},
                "measurements":measurements[i]})
            })
            .collect();
        sessions.push(
            json!({"load_seconds":load_secs,"rss_before_load_kib":rss_before_load,
            "rss_after_load_kib":rss_after_load,"rss_before_drop_kib":rss_before_drop,
            "rss_after_drop_kib":rss_after_drop,"drm_after_load":drm_after_load,
            "drm_before_drop":drm_before_drop,"drm_after_drop":drm_after_drop,"clips":clips}),
        );
    }
    let result = if audio.len() == 1 && sessions.len() == 1 {
        // Preserve the original single-clip JSON schema used by the comparison script.
        let mut clip = sessions[0]["clips"][0].clone();
        for key in [
            "load_seconds",
            "rss_before_load_kib",
            "rss_after_load_kib",
            "rss_before_drop_kib",
            "rss_after_drop_kib",
            "drm_after_load",
            "drm_before_drop",
            "drm_after_drop",
        ] {
            clip[key] = sessions[0][key].clone();
        }
        clip["config"] = serde_json::to_value(&cohere)?;
        clip
    } else {
        json!({"config":cohere,"sessions":sessions})
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    if had_errors {
        bail!("one or more inference calls failed; see the JSON error records");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_handles_even_and_odd_counts() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), 2.5);
    }
}
