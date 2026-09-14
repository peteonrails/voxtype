//! `voxtype setup benchmark`: time the installed binary variants on this
//! machine and recommend one from the measurements.
//!
//! The library half ([`run_benchmark`], [`record_until_enter`], the saved
//! [`Report`]) is shared by the `setup benchmark` command and the benchmark
//! screen in `voxtype configure`. Each variant runs as its own process through
//! `voxtype transcribe`, because the packaged binaries in `/usr/lib/voxtype/`
//! can be older than the binary running the benchmark, and that command's
//! output is the interface they all share. The recording is of a known
//! passage, so a variant that finishes quickly with the wrong words (a GPU
//! build silently failing, say) is not the one recommended, and a variant that
//! is already slower than an accurate one is stopped rather than waited for.

use super::binary::{self, Acceleration, Variant, LIB_DIR};
use crate::config::{AudioConfig, Config};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Text the user reads aloud. Plain words with no names or numbers, so the
/// word error rate measures the variant rather than the model's spelling.
pub const PASSAGE: &str = "Speech recognition runs on this computer without sending your voice \
anywhere. Please read these sentences at your normal pace and volume. Each installed build \
will transcribe the recording, and the fastest one that gets the words right will be \
recommended.";

/// A variant may be this much less accurate than the best result and still
/// be recommended for being faster.
const WER_TOLERANCE: f64 = 0.05;

/// Once its model has loaded, a run is stopped when its transcription takes
/// this much longer than the typical transcription of the fastest accurate
/// other variant. The slack keeps near-ties from being cut off by noise.
const STOP_RATIO: f64 = 1.10;
const STOP_SLACK_SECS: f64 = 0.25;

/// Before a run reports its model loaded (or for an engine that never does),
/// it may take this much longer than the pace variant's whole run: model load
/// times differ widely between CPU and GPU builds.
const LOAD_RATIO: f64 = 3.0;
const LOAD_SLACK_SECS: f64 = 2.0;

/// Below this RMS the recording is treated as silence (roughly -46 dBFS).
pub const MIN_RMS: f32 = 0.005;
pub const MIN_RECORDING_SECS: f32 = 3.0;
pub const MAX_RECORDING_SECS: u64 = 60;
pub const DEFAULT_RUNS: usize = 3;

const SAMPLE_RATE: u32 = 16_000;
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const REPORT_FILE: &str = "benchmark.json";

/// Log lines `voxtype transcribe` prints, which carry its timings.
const MODEL_LOADED: &str = "Model loaded in ";
const TRANSCRIBED: &str = "Transcription completed in ";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscribeOutput {
    pub model_load_secs: Option<f64>,
    pub transcribe_secs: Option<f64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantResult {
    pub variant: Variant,
    pub binary_name: String,
    /// Transcription time of each timed run, in seconds.
    pub runs_secs: Vec<f64>,
    pub median_secs: Option<f64>,
    pub model_load_secs: Option<f64>,
    /// Transcription time of every completed run, warm-up included. Sets the
    /// pace other variants are stopped against.
    #[serde(default)]
    pub observed_secs: Vec<f64>,
    /// Process wall time of every completed run, warm-up included.
    #[serde(default)]
    pub wall_secs: Vec<f64>,
    /// `None` when no reference text was available to score against.
    pub word_error_rate: Option<f64>,
    pub transcript: Option<String>,
    pub error: Option<String>,
    /// Why the variant was stopped early, when it was.
    #[serde(default)]
    pub stopped: Option<String>,
}

impl VariantResult {
    fn new(variant: Variant) -> Self {
        Self {
            variant,
            binary_name: variant.binary_name().to_string(),
            runs_secs: Vec::new(),
            median_secs: None,
            model_load_secs: None,
            observed_secs: Vec::new(),
            wall_secs: Vec::new(),
            word_error_rate: None,
            transcript: None,
            error: None,
            stopped: None,
        }
    }

    fn finished(&self) -> bool {
        self.error.is_none() && self.stopped.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Unix time of the measurement, in seconds.
    pub measured_at: u64,
    pub engine: String,
    pub audio_secs: f32,
    /// Whether word error rates were scored against the passage.
    pub scored: bool,
    pub results: Vec<VariantResult>,
    pub recommended: Option<Variant>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn result(&self, variant: Variant) -> Option<&VariantResult> {
        self.results.iter().find(|r| r.variant == variant)
    }

    /// True when the variants that can run the engine here are no longer the
    /// ones that were measured, e.g. a build was installed or removed since.
    pub fn is_stale(&self, eligible_now: &[Variant]) -> bool {
        self.results.len() != eligible_now.len()
            || !self
                .results
                .iter()
                .all(|r| eligible_now.contains(&r.variant))
    }
}

/// Progress of [`run_benchmark`], reported as it happens.
#[derive(Debug, Clone, PartialEq)]
pub enum BenchEvent {
    WarmUp(Variant),
    Run {
        round: usize,
        runs: usize,
        variant: Variant,
    },
    Finished {
        variant: Variant,
        secs: f64,
    },
    Stopped {
        variant: Variant,
        after_secs: f64,
        faster: Variant,
    },
    Failed {
        variant: Variant,
        error: String,
    },
}

impl BenchEvent {
    pub fn describe(&self) -> String {
        match self {
            BenchEvent::WarmUp(v) => format!("warm-up    {}", v.display()),
            BenchEvent::Run {
                round,
                runs,
                variant,
            } => format!("run {}/{}    {}", round, runs, variant.display()),
            BenchEvent::Finished { variant, secs } => {
                format!("           {} finished in {:.2}s", variant.display(), secs)
            }
            BenchEvent::Stopped {
                variant,
                after_secs,
                faster,
            } => format!(
                "           {} stopped after {:.1}s: already slower than {}",
                variant.display(),
                after_secs,
                faster.display()
            ),
            BenchEvent::Failed { variant, error } => {
                format!("           {} failed: {}", variant.display(), error)
            }
        }
    }
}

pub struct BenchmarkInput {
    pub engine: String,
    /// Variants in the order to run them. The likeliest fastest goes first so
    /// it sets the pace and slower variants can be stopped early.
    pub candidates: Vec<Variant>,
    pub wav: PathBuf,
    /// Text the recording should contain; `None` skips accuracy scoring.
    pub reference: Option<String>,
    pub runs: usize,
    pub config_path: Option<PathBuf>,
    /// Directory holding the variant binaries, normally [`LIB_DIR`].
    pub lib_dir: PathBuf,
}

/// The `voxtype setup benchmark` command.
pub async fn run(
    config: &Config,
    config_path: Option<&Path>,
    audio: Option<PathBuf>,
    runs: usize,
    save_audio: Option<PathBuf>,
    json: bool,
    record_to: Option<PathBuf>,
) -> anyhow::Result<()> {
    if let Some(dest) = record_to {
        // Child mode for `voxtype configure`: record until the parent writes a
        // newline, save, report, exit.
        let samples = record_until_enter(&config.audio).await?;
        write_wav(&dest, &samples)?;
        println!(
            "{}",
            serde_json::json!({
                "secs": samples.len() as f32 / SAMPLE_RATE as f32,
                "rms": rms(&samples),
            })
        );
        return Ok(());
    }

    anyhow::ensure!(runs > 0, "--runs must be at least 1");
    let engine = config.engine.name();
    let candidates = candidates(engine);
    if candidates.is_empty() {
        anyhow::bail!(
            "No installed variant in {} can run the {} engine on this machine, so there is \
             nothing to benchmark.",
            LIB_DIR,
            engine
        );
    }
    for w in warnings() {
        say(json, &format!("Note: {}", w));
    }

    // Holds the temporary recording until the benchmark finishes.
    let mut _recording: Option<tempfile::NamedTempFile> = None;
    let (wav, reference) = match audio {
        Some(path) => (path, None),
        None => {
            say(json, "Read this passage aloud:\n");
            say(json, &format!("  {}\n", PASSAGE));
            say(json, "Press Enter to start recording.");
            wait_for_enter().await;
            say(
                json,
                "Recording. Press Enter when you have finished reading.",
            );
            let samples = record_until_enter(&config.audio).await?;

            let file = tempfile::Builder::new()
                .prefix("voxtype-benchmark-")
                .suffix(".wav")
                .tempfile()?;
            write_wav(file.path(), &samples)?;
            if let Some(dest) = &save_audio {
                std::fs::copy(file.path(), dest)
                    .with_context(|| format!("Cannot save the recording to {}", dest.display()))?;
                say(json, &format!("Saved the recording to {}", dest.display()));
            }
            let path = file.path().to_path_buf();
            _recording = Some(file);
            (path, Some(PASSAGE.to_string()))
        }
    };

    say(
        json,
        &format!(
            "\nBenchmarking {} variant(s): one warm-up run each, then {} timed run(s). \
             A variant already slower than an accurate one is stopped.",
            candidates.len(),
            runs
        ),
    );
    let input = BenchmarkInput {
        engine: engine.to_string(),
        candidates,
        wav,
        reference,
        runs,
        config_path: config_path.map(Path::to_path_buf),
        lib_dir: PathBuf::from(LIB_DIR),
    };
    let cancel = AtomicBool::new(false);
    let report = run_benchmark(
        &input,
        &mut |event| say(json, &format!("  {}", event.describe())),
        &cancel,
    )?;
    // A run without a recommendation must not replace earlier good results.
    let saved = report.recommended.map(|_| save_report(&report));

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!();
        for line in report_lines(&report) {
            println!("{}", line);
        }
        if let Some(best) = report.recommended {
            if binary::active_variant() == Some(best) {
                println!("It is already the active variant.");
            } else {
                println!(
                    "Switch with: sudo voxtype setup variant --to {}",
                    best.binary_name()
                );
            }
        }
    }
    match saved {
        Some(Ok(path)) => say(
            json,
            &format!(
                "\nSaved to {}. `voxtype configure` and `voxtype info variants` show these results.",
                path.display()
            ),
        ),
        Some(Err(e)) => say(json, &format!("\nCould not save the results: {}", e)),
        None => say(
            json,
            "\nNothing was saved, so any earlier results are kept.",
        ),
    }
    Ok(())
}

/// Installed variants in `installed` that can run `engine` on this CPU and GPU.
pub fn eligible(
    installed: &[Variant],
    cpu: &binary::Cpu,
    gpus: &binary::Gpus,
    engine: &str,
) -> Vec<Variant> {
    installed
        .iter()
        .copied()
        .filter(|&v| {
            v.supports_engine(engine)
                && binary::variant_runs_on_cpu(v, cpu)
                && binary::variant_gpu_available(v, gpus)
        })
        .collect()
}

/// Variants to benchmark for `engine`, with the hardware recommendation first
/// so it sets the pace for the others. Reads the lib dir directly so a source
/// build can still benchmark the packaged binaries.
pub fn candidates(engine: &str) -> Vec<Variant> {
    let inv = binary::inventory();
    let mut found = eligible(&binary::enumerate_installed(), &inv.cpu, &inv.gpus, engine);
    let rec = inv.recommendation;
    found.sort_by_key(|&v| !(v == rec.whisper || v == rec.onnx));
    found
}

/// Conditions that make timings unreliable.
pub fn warnings() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(w) = load_warning() {
        out.push(w);
    }
    if crate::daemon_status::read_pid_if_alive().is_some() {
        out.push(
            "The voxtype daemon is running. It keeps a model loaded and competes for the \
             same CPU and GPU, which can slow every variant down."
                .to_string(),
        );
    }
    out
}

/// Warm up and time every candidate, stopping any run that falls behind an
/// accurate variant, and recommend one. Blocking.
pub fn run_benchmark(
    input: &BenchmarkInput,
    on_event: &mut dyn FnMut(BenchEvent),
    cancel: &AtomicBool,
) -> anyhow::Result<Report> {
    anyhow::ensure!(input.runs > 0, "At least one timed run is needed");
    let audio_secs = wav_duration_secs(&input.wav)?;
    let warnings = warnings();
    let mut results: Vec<VariantResult> = input
        .candidates
        .iter()
        .map(|&v| VariantResult::new(v))
        .collect();

    // Warm-up: fills the page cache with the model and builds any GPU shader
    // cache, which would otherwise be charged to whichever variant runs first.
    for i in 0..results.len() {
        check_cancelled(cancel)?;
        on_event(BenchEvent::WarmUp(results[i].variant));
        run_once(input, &mut results, i, false, on_event, cancel)?;
    }

    // Alternate the order each round so thermal throttling and background
    // load do not consistently favour the same variant.
    for round in 0..input.runs {
        let mut order: Vec<usize> = (0..results.len()).collect();
        if round % 2 == 1 {
            order.reverse();
        }
        for i in order {
            if !results[i].finished() {
                continue;
            }
            check_cancelled(cancel)?;
            on_event(BenchEvent::Run {
                round: round + 1,
                runs: input.runs,
                variant: results[i].variant,
            });
            run_once(input, &mut results, i, true, on_event, cancel)?;
        }
    }

    for result in &mut results {
        result.median_secs = median(&result.runs_secs);
    }
    let recommended = pick_recommendation(&results);
    Ok(Report {
        measured_at: now_secs(),
        engine: input.engine.clone(),
        audio_secs,
        scored: input.reference.is_some(),
        results,
        recommended,
        warnings,
    })
}

fn check_cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("Benchmark cancelled");
    }
    Ok(())
}

/// Run variant `i` once and fold the outcome into its result.
fn run_once(
    input: &BenchmarkInput,
    results: &mut [VariantResult],
    i: usize,
    timed: bool,
    on_event: &mut dyn FnMut(BenchEvent),
    cancel: &AtomicBool,
) -> anyhow::Result<()> {
    let variant = results[i].variant;
    // Warm-up absorbs one-time costs such as GPU shader and kernel compilation,
    // so only builds without them can be stopped during it.
    let pace = if timed || !compiles_on_first_run(variant) {
        pace_for(results, variant)
    } else {
        None
    };
    let outcome = run_variant(
        &input.lib_dir,
        variant,
        &input.wav,
        input.config_path.as_deref(),
        pace,
        cancel,
    );

    match outcome {
        RunOutcome::Done(output, wall) => {
            let result = &mut results[i];
            let secs = output.transcribe_secs.unwrap_or(wall);
            result.observed_secs.push(secs);
            result.wall_secs.push(wall);
            if timed {
                result.runs_secs.push(secs);
            }
            if result.model_load_secs.is_none() {
                result.model_load_secs = output.model_load_secs;
            }
            result.word_error_rate = input
                .reference
                .as_deref()
                .map(|reference| word_error_rate(reference, &output.text));
            result.transcript = Some(output.text);
            on_event(BenchEvent::Finished { variant, secs });
        }
        RunOutcome::Stopped(after_secs) => {
            // Only reachable with a pace, which names the faster variant.
            let faster = pace.map_or(variant, |p| p.variant);
            results[i].stopped = Some(format!(
                "stopped after {:.1}s, slower than {}",
                after_secs,
                faster.display()
            ));
            on_event(BenchEvent::Stopped {
                variant,
                after_secs,
                faster,
            });
        }
        RunOutcome::Failed(error) => {
            results[i].error = Some(error.clone());
            on_event(BenchEvent::Failed { variant, error });
        }
        RunOutcome::Cancelled => anyhow::bail!("Benchmark cancelled"),
    }
    Ok(())
}

enum RunOutcome {
    Done(TranscribeOutput, f64),
    Stopped(f64),
    Failed(String),
    Cancelled,
}

/// Run one variant's `transcribe` on `wav`. The run is killed once it falls
/// behind `pace`, or when `cancel` is set.
fn run_variant(
    lib_dir: &Path,
    variant: Variant,
    wav: &Path,
    config_path: Option<&Path>,
    pace: Option<Pace>,
    cancel: &AtomicBool,
) -> RunOutcome {
    let binary = lib_dir.join(variant.binary_name());
    let mut cmd = Command::new(&binary);
    if let Some(path) = config_path {
        cmd.arg("--config").arg(path);
    }
    cmd.arg("transcribe")
        .arg(wav)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so stopping it also stops any helper it
        // started, such as the GPU-isolation worker.
        .process_group(0);

    let start = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return RunOutcome::Failed(format!("cannot run {}: {}", binary.display(), e)),
    };
    // Drain both pipes on their own threads so a chatty child cannot block on
    // a full pipe while it is being waited for.
    let loaded = Arc::new(OnceLock::new());
    let stdout = spawn_stdout_reader(child.stdout.take(), loaded.clone());
    let stderr = spawn_reader(child.stderr.take());

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                kill_run(&mut child);
                return RunOutcome::Failed(format!("waiting for {}: {}", binary.display(), e));
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        if cancel.load(Ordering::Relaxed) {
            kill_run(&mut child);
            return RunOutcome::Cancelled;
        }
        if let Some(pace) = pace {
            // Time the transcription itself once the model has loaded, so a
            // build that loads slowly but transcribes faster is not cut off.
            let behind = match loaded.get() {
                Some(at) => at.elapsed().as_secs_f64() > pace.transcribe_secs,
                None => elapsed > pace.wall_secs,
            };
            if behind {
                kill_run(&mut child);
                return RunOutcome::Stopped(elapsed);
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let wall = start.elapsed().as_secs_f64();
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();

    if !status.success() {
        let last = stderr
            .lines()
            .rev()
            .map(super::accel::strip_ansi)
            .find(|l| !l.trim().is_empty())
            .unwrap_or_else(|| "no error output".to_string());
        return RunOutcome::Failed(format!(
            "{} exited with {}: {}",
            variant.binary_name(),
            status,
            last
        ));
    }
    RunOutcome::Done(parse_transcribe_output(&stdout), wall)
}

fn spawn_reader<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Like [`spawn_reader`], and records when the "Model loaded in" line arrives
/// so the run can be timed from that point.
fn spawn_stdout_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    loaded: Arc<OnceLock<Instant>>,
) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let Some(pipe) = pipe else {
            return text;
        };
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        while matches!(reader.read_until(b'\n', &mut line), Ok(n) if n > 0) {
            let chunk = String::from_utf8_lossy(&line);
            if chunk.contains(MODEL_LOADED) {
                let _ = loaded.set(Instant::now());
            }
            text.push_str(&chunk);
            line.clear();
        }
        text
    })
}

/// Kill a run together with anything it started. It was spawned as the leader
/// of its own process group, so the group id is its pid.
fn kill_run(child: &mut Child) {
    if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: killpg only sends a signal, to the run's own process group.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Builds whose first run can include one-time GPU shader or kernel compilation.
fn compiles_on_first_run(variant: Variant) -> bool {
    matches!(
        variant.acceleration(),
        Acceleration::Vulkan | Acceleration::Cuda | Acceleration::Migraphx
    )
}

/// Parse `voxtype transcribe` stdout: timing comes from the "Model loaded in"
/// and "Transcription completed in" log lines, and the transcript is the text
/// after the final blank line.
pub fn parse_transcribe_output(stdout: &str) -> TranscribeOutput {
    let mut out = TranscribeOutput::default();
    let secs_after = |line: &str, marker: &str| -> Option<f64> {
        let rest = &line[line.find(marker)? + marker.len()..];
        let number: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        number.parse().ok()
    };

    let lines: Vec<String> = stdout.lines().map(super::accel::strip_ansi).collect();
    for line in &lines {
        if let Some(s) = secs_after(line, MODEL_LOADED) {
            out.model_load_secs = Some(s);
        }
        if let Some(s) = secs_after(line, TRANSCRIBED) {
            out.transcribe_secs = Some(s);
        }
    }
    if let Some(blank) = lines.iter().rposition(|l| l.trim().is_empty()) {
        out.text = lines[blank + 1..].join(" ").trim().to_string();
    }
    out
}

/// Record from the configured input until a line or end of file arrives on
/// stdin, or [`MAX_RECORDING_SECS`] pass. Used by the interactive command and
/// by the `--record-to` child that `voxtype configure` drives, which keeps
/// audio device access out of the TUI process (see #541).
pub async fn record_until_enter(audio: &AudioConfig) -> anyhow::Result<Vec<f32>> {
    let mut capture = crate::audio::create_capture(audio)?;
    let mut chunks = capture.start().await?;
    // Drain the live chunk channel; the full recording comes back from stop().
    let drain = tokio::spawn(async move { while chunks.recv().await.is_some() {} });

    let _ = tokio::time::timeout(Duration::from_secs(MAX_RECORDING_SECS), wait_for_enter()).await;
    let samples = capture.stop().await?;
    drain.abort();

    check_recording(&samples)?;
    Ok(samples)
}

/// Reject a recording too short or too quiet to benchmark with.
pub fn check_recording(samples: &[f32]) -> anyhow::Result<()> {
    let secs = samples.len() as f32 / SAMPLE_RATE as f32;
    if secs < MIN_RECORDING_SECS {
        anyhow::bail!(
            "The recording is only {:.1}s long. Read the whole passage before stopping.",
            secs
        );
    }
    if rms(samples) < MIN_RMS {
        anyhow::bail!(
            "The recording is silent, so the microphone is not delivering audio. \
             Check the input device with: voxtype info devices"
        );
    }
    Ok(())
}

/// Wait for a line or end of file on stdin. The read runs on a plain thread
/// rather than tokio's stdin: tokio parks that read on a blocking-pool task the
/// runtime waits for at shutdown, so a read abandoned by the recording time cap
/// would keep the process from ever exiting.
async fn wait_for_enter() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let _ = tx.send(());
    });
    let _ = rx.await;
}

fn say(json: bool, line: &str) {
    // Keep stdout clean for the JSON report.
    if json {
        eprintln!("{}", line);
    } else {
        println!("{}", line);
    }
}

fn write_wav(path: &Path, samples: &[f32]) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

fn wav_duration_secs(path: &Path) -> anyhow::Result<f32> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("Cannot read {} as a WAV file", path.display()))?;
    let spec = reader.spec();
    Ok(reader.duration() as f32 / spec.sample_rate as f32)
}

fn normalize_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Word error rate: word-level edit distance divided by the reference length.
/// Case and punctuation are ignored.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let r = normalize_words(reference);
    let h = normalize_words(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let substitution = prev[j] + usize::from(rw != hw);
            cur[j + 1] = substitution.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Results good enough to recommend or to set the pace: finished, produced
/// text, and within `WER_TOLERANCE` of the most accurate when scored.
fn acceptable(results: &[VariantResult]) -> Vec<&VariantResult> {
    let usable: Vec<&VariantResult> = results
        .iter()
        .filter(|r| r.finished())
        .filter(|r| {
            r.transcript
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty())
        })
        .collect();
    let best_wer = usable
        .iter()
        .filter_map(|r| r.word_error_rate)
        .min_by(|a, b| a.total_cmp(b));
    usable
        .into_iter()
        .filter(|r| match (best_wer, r.word_error_rate) {
            (Some(best), Some(wer)) => wer <= best + WER_TOLERANCE,
            _ => true,
        })
        .collect()
}

/// How far a run may fall behind before it is stopped, and which variant set
/// that pace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    pub variant: Variant,
    /// Transcription seconds allowed once the run's model has loaded.
    pub transcribe_secs: f64,
    /// Wall seconds allowed while the run has not reported its model loaded.
    pub wall_secs: f64,
}

/// The pace for a run of `variant`: the typical (median) timings of the other
/// acceptable variant that transcribes fastest, plus slack. `None` until
/// another variant has finished accurately.
pub fn pace_for(results: &[VariantResult], variant: Variant) -> Option<Pace> {
    acceptable(results)
        .into_iter()
        .filter(|r| r.variant != variant)
        .filter_map(|r| Some((r.variant, median(&r.observed_secs)?, median(&r.wall_secs)?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, transcribe, wall)| Pace {
            variant: v,
            transcribe_secs: transcribe * STOP_RATIO + STOP_SLACK_SECS,
            wall_secs: wall * LOAD_RATIO + LOAD_SLACK_SECS,
        })
}

/// The fastest variant whose word error rate is within `WER_TOLERANCE` of the
/// best. Without a reference text, the fastest variant that produced any text.
pub fn pick_recommendation(results: &[VariantResult]) -> Option<Variant> {
    acceptable(results)
        .into_iter()
        .filter_map(|r| r.median_secs.map(|m| (r.variant, m)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
}

fn load_warning() -> Option<String> {
    let loadavg = std::fs::read_to_string("/proc/loadavg").ok()?;
    let load: f64 = loadavg.split_whitespace().next()?.parse().ok()?;
    let cores = std::thread::available_parallelism().ok()?.get() as f64;
    (load > cores * 0.75).then(|| {
        format!(
            "System load is {:.1} on {} cores. Other work will skew the timings; close it \
             or rerun later for results you can trust.",
            load, cores
        )
    })
}

/// The results table and recommendation, shared by the command and the TUI.
pub fn report_lines(report: &Report) -> Vec<String> {
    let mut lines = vec![format!(
        "{:<22} {:>11} {:>11} {:>10} {:>11}",
        "Variant", "Transcribe", "Model load", "RT factor", "Word errors"
    )];
    for r in &report.results {
        let name = r.variant.display();
        if let Some(err) = &r.error {
            lines.push(format!("{:<22} failed: {}", name, err));
            continue;
        }
        if let Some(why) = &r.stopped {
            lines.push(format!("{:<22} {}", name, why));
            continue;
        }
        let secs = |v: Option<f64>| v.map_or("n/a".to_string(), |s| format!("{:.2}s", s));
        let rt = r.median_secs.map_or("n/a".to_string(), |s| {
            format!("{:.2}x", s / f64::from(report.audio_secs))
        });
        let wer = r
            .word_error_rate
            .map_or("n/a".to_string(), |w| format!("{:.1}%", w * 100.0));
        let mark = if Some(r.variant) == report.recommended {
            "  ★"
        } else {
            ""
        };
        lines.push(format!(
            "{:<22} {:>11} {:>11} {:>10} {:>11}{}",
            name,
            secs(r.median_secs),
            secs(r.model_load_secs),
            rt,
            wer,
            mark
        ));
    }
    lines.push(String::new());
    match report.recommended {
        Some(best) => {
            lines.push(format!(
                "Recommended on this machine: {} ({})",
                best.display(),
                best.binary_name()
            ));
            if !report.scored {
                lines.push(
                    "Accuracy was not scored without the passage, so this is simply the fastest."
                        .to_string(),
                );
            }
        }
        None => lines.push(
            "No variant produced a usable transcription, so there is no recommendation."
                .to_string(),
        ),
    }
    lines
}

/// One line describing the measured pick, e.g.
/// "Whisper (Vulkan) · 0.64s · 0% word errors · 2 h ago".
pub fn summary(report: &Report, now: u64) -> Option<String> {
    let best = report.result(report.recommended?)?;
    let mut parts = vec![best.variant.display().to_string()];
    if let Some(secs) = best.median_secs {
        parts.push(format!("{:.2}s", secs));
    }
    if let Some(wer) = best.word_error_rate {
        parts.push(format!("{:.0}% word errors", wer * 100.0));
    }
    parts.push(age(report.measured_at, now));
    Some(parts.join(" · "))
}

pub fn age(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => format!("{} h ago", secs / 3_600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Where the last benchmark is saved: `$XDG_STATE_HOME/voxtype/benchmark.json`.
pub fn report_path() -> PathBuf {
    Config::state_dir().join(REPORT_FILE)
}

pub fn save_report(report: &Report) -> anyhow::Result<PathBuf> {
    let path = report_path();
    save_report_to(report, &path)?;
    Ok(path)
}

pub fn load_report() -> Option<Report> {
    load_report_from(&report_path())
}

fn save_report_to(report: &Report, path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(report)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn load_report_from(path: &Path) -> Option<Report> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `voxtype-vulkan transcribe` 1.0.1, colour codes included.
    const VULKAN_STDOUT: &str = "Loading audio file: \"speech_long.wav\"\n\
Audio format: 16000 Hz, 1 channel(s), Int\n\
Processing 76048 samples (4.75s)...\n\
\u{1b}[2m2026-09-14T09:47:02.928465Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Using local whisper transcription mode\n\
\u{1b}[2m2026-09-14T09:47:03.983799Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Model loaded in 1.05s\n\
\u{1b}[2m2026-09-14T09:47:05.211902Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Transcription completed in 1.23s: \"This is a longer test ...\"\n\
\n\
This is a longer test of voice activity detection with multiple words and phrases.\n";

    #[test]
    fn parses_timing_and_transcript() {
        let out = parse_transcribe_output(VULKAN_STDOUT);
        assert_eq!(out.model_load_secs, Some(1.05));
        assert_eq!(out.transcribe_secs, Some(1.23));
        assert_eq!(
            out.text,
            "This is a longer test of voice activity detection with multiple words and phrases."
        );
    }

    #[test]
    fn no_speech_output_has_no_transcript() {
        let out = parse_transcribe_output(
            "Loading audio file: \"x.wav\"\nVAD: 0.00s speech (0.0% of audio)\nNo speech detected, skipping transcription.\n",
        );
        assert_eq!(out.text, "");
        assert_eq!(out.transcribe_secs, None);
    }

    #[test]
    fn word_error_rate_ignores_case_and_punctuation() {
        assert_eq!(word_error_rate("Hello, world.", "hello world"), 0.0);
        assert_eq!(
            word_error_rate("one two three four", "one too three four"),
            0.25
        );
        assert_eq!(
            word_error_rate("one two three four", "one three four"),
            0.25
        );
        assert_eq!(word_error_rate("one two", "one two three"), 0.5);
        assert_eq!(word_error_rate("one two", ""), 1.0);
        assert_eq!(word_error_rate("", ""), 0.0);
    }

    #[test]
    fn passage_scores_itself_perfectly() {
        assert_eq!(word_error_rate(PASSAGE, PASSAGE), 0.0);
    }

    fn result(v: Variant, median: f64, wer: Option<f64>) -> VariantResult {
        VariantResult {
            runs_secs: vec![median],
            median_secs: Some(median),
            observed_secs: vec![median],
            wall_secs: vec![median + 0.5],
            word_error_rate: wer,
            transcript: Some("text".to_string()),
            ..VariantResult::new(v)
        }
    }

    #[test]
    fn a_fast_but_wrong_variant_is_not_recommended() {
        let results = vec![
            result(Variant::WhisperNative, 1.3, Some(0.02)),
            result(Variant::WhisperVulkan, 0.4, Some(0.60)),
        ];
        assert_eq!(pick_recommendation(&results), Some(Variant::WhisperNative));
    }

    #[test]
    fn a_faster_variant_within_tolerance_wins() {
        let results = vec![
            result(Variant::WhisperNative, 1.3, Some(0.00)),
            result(Variant::WhisperVulkan, 0.7, Some(0.04)),
        ];
        assert_eq!(pick_recommendation(&results), Some(Variant::WhisperVulkan));
    }

    #[test]
    fn failed_stopped_and_empty_variants_are_skipped() {
        let mut failed = result(Variant::WhisperVulkan, 0.1, Some(0.0));
        failed.error = Some("exited with 1".to_string());
        let mut stopped = result(Variant::WhisperAvx512, 0.2, Some(0.0));
        stopped.stopped = Some("stopped after 1.0s".to_string());
        let mut empty = result(Variant::WhisperAvx2, 0.3, None);
        empty.transcript = Some(String::new());
        let results = vec![
            failed,
            stopped,
            empty,
            result(Variant::WhisperNative, 1.3, Some(0.0)),
        ];
        assert_eq!(pick_recommendation(&results), Some(Variant::WhisperNative));
    }

    #[test]
    fn without_a_reference_the_fastest_wins() {
        let results = vec![
            result(Variant::WhisperNative, 1.3, None),
            result(Variant::WhisperVulkan, 0.7, None),
        ];
        assert_eq!(pick_recommendation(&results), Some(Variant::WhisperVulkan));
    }

    #[test]
    fn pace_follows_the_fastest_other_accurate_transcription() {
        let results = vec![
            result(Variant::WhisperVulkan, 0.6, Some(0.0)),
            // Faster, but wrong: must not set the pace.
            result(Variant::WhisperAvx2, 0.1, Some(0.9)),
            result(Variant::WhisperNative, 5.0, Some(0.0)),
        ];
        let pace = pace_for(&results, Variant::WhisperNative).unwrap();
        assert_eq!(pace.variant, Variant::WhisperVulkan);
        assert!((pace.transcribe_secs - (0.6 * STOP_RATIO + STOP_SLACK_SECS)).abs() < 1e-9);
        assert!((pace.wall_secs - (1.1 * LOAD_RATIO + LOAD_SLACK_SECS)).abs() < 1e-9);

        // A variant never races against itself.
        let pace = pace_for(&results, Variant::WhisperVulkan).unwrap();
        assert_eq!(pace.variant, Variant::WhisperNative);
        assert_eq!(
            pace_for(
                &[result(Variant::WhisperVulkan, 0.6, Some(0.0))],
                Variant::WhisperVulkan
            ),
            None
        );
    }

    /// One lucky run must not tighten the pace for every other variant.
    #[test]
    fn pace_uses_the_typical_run() {
        let mut vulkan = result(Variant::WhisperVulkan, 0.6, Some(0.0));
        vulkan.observed_secs = vec![0.2, 0.6, 0.7];
        let pace = pace_for(&[vulkan], Variant::WhisperNative).unwrap();
        assert!((pace.transcribe_secs - (0.6 * STOP_RATIO + STOP_SLACK_SECS)).abs() < 1e-9);
    }

    #[test]
    fn only_gpu_builds_are_spared_during_warm_up() {
        assert!(compiles_on_first_run(Variant::WhisperVulkan));
        assert!(compiles_on_first_run(Variant::OnnxCuda12));
        assert!(compiles_on_first_run(Variant::OnnxMigraphx));
        assert!(!compiles_on_first_run(Variant::WhisperNative));
        assert!(!compiles_on_first_run(Variant::WhisperAvx512));
        assert!(!compiles_on_first_run(Variant::OnnxAvx2));
    }

    #[test]
    fn recordings_must_be_long_and_loud_enough() {
        let second = SAMPLE_RATE as usize;
        assert!(check_recording(&vec![0.1; second]).is_err());
        assert!(check_recording(&vec![0.0; 5 * second]).is_err());
        let voiced: Vec<f32> = (0..5 * second)
            .map(|i| if i % 2 == 0 { 0.1 } else { -0.1 })
            .collect();
        assert!(check_recording(&voiced).is_ok());
    }

    #[test]
    fn median_and_rms() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(rms(&[]), 0.0);
        assert!(rms(&[0.1, -0.1, 0.1, -0.1]) > MIN_RMS);
    }

    fn report(results: Vec<VariantResult>) -> Report {
        let recommended = pick_recommendation(&results);
        Report {
            measured_at: 1_000,
            engine: "whisper".to_string(),
            audio_secs: 12.0,
            scored: true,
            results,
            recommended,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn report_round_trips_and_notices_changed_builds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join(REPORT_FILE);
        let saved = report(vec![
            result(Variant::WhisperVulkan, 0.6, Some(0.0)),
            result(Variant::WhisperNative, 1.3, Some(0.0)),
        ]);
        save_report_to(&saved, &path).unwrap();
        let loaded = load_report_from(&path).unwrap();
        assert_eq!(loaded.recommended, Some(Variant::WhisperVulkan));
        assert_eq!(loaded.results, saved.results);

        assert!(!loaded.is_stale(&[Variant::WhisperNative, Variant::WhisperVulkan]));
        assert!(loaded.is_stale(&[Variant::WhisperVulkan]));
        assert!(loaded.is_stale(&[
            Variant::WhisperVulkan,
            Variant::WhisperNative,
            Variant::WhisperAvx2
        ]));
        assert!(load_report_from(&dir.path().join("missing.json")).is_none());
    }

    #[test]
    fn summary_and_age_read_naturally() {
        let r = report(vec![result(Variant::WhisperVulkan, 0.64, Some(0.0))]);
        assert_eq!(
            summary(&r, 1_000 + 7_200).as_deref(),
            Some("Whisper (Vulkan) · 0.64s · 0% word errors · 2 h ago")
        );
        assert_eq!(age(100, 130), "just now");
        assert_eq!(age(0, 600), "10 min ago");
        assert_eq!(age(0, 3 * 86_400), "3 days ago");
    }

    #[cfg(unix)]
    fn fake_variant(dir: &Path, variant: Variant, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(variant.binary_name());
        std::fs::write(&path, format!("#!/bin/sh\n{}\n", script)).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[cfg(unix)]
    fn fake_input(dir: &Path, candidates: Vec<Variant>) -> BenchmarkInput {
        let wav = dir.join("clip.wav");
        write_wav(&wav, &vec![0.0; SAMPLE_RATE as usize]).unwrap();
        BenchmarkInput {
            engine: "whisper".to_string(),
            candidates,
            wav,
            reference: Some(PASSAGE.to_string()),
            runs: 1,
            config_path: None,
            lib_dir: dir.to_path_buf(),
        }
    }

    /// Runs real child processes: a quick accurate variant, and one that would
    /// take 30 seconds. The slow one must be stopped instead of waited for.
    #[cfg(unix)]
    #[test]
    fn a_variant_slower_than_an_accurate_one_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(
            dir.path(),
            Variant::WhisperVulkan,
            &format!(
                "sleep 0.2\nprintf 'INFO Model loaded in 0.05s\\nINFO Transcription completed in 0.10s: ok\\n\\n%s\\n' \"{}\"",
                PASSAGE
            ),
        );
        fake_variant(dir.path(), Variant::WhisperNative, "exec sleep 30");

        let input = fake_input(
            dir.path(),
            vec![Variant::WhisperVulkan, Variant::WhisperNative],
        );
        let mut events = Vec::new();
        let start = Instant::now();
        let report =
            run_benchmark(&input, &mut |e| events.push(e), &AtomicBool::new(false)).unwrap();

        assert!(
            start.elapsed() < Duration::from_secs(15),
            "the slow variant was waited for"
        );
        assert_eq!(report.recommended, Some(Variant::WhisperVulkan));
        let fast = report.result(Variant::WhisperVulkan).unwrap();
        assert_eq!(fast.runs_secs, vec![0.10]);
        assert_eq!(fast.word_error_rate, Some(0.0));
        assert!(report
            .result(Variant::WhisperNative)
            .unwrap()
            .stopped
            .is_some());
        assert!(events.iter().any(|e| matches!(
            e,
            BenchEvent::Stopped {
                variant: Variant::WhisperNative,
                faster: Variant::WhisperVulkan,
                ..
            }
        )));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_variant_is_reported_not_recommended() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(
            dir.path(),
            Variant::WhisperNative,
            "echo 'model not found' >&2\nexit 3",
        );
        let input = fake_input(dir.path(), vec![Variant::WhisperNative]);
        let report = run_benchmark(&input, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        let native = report.result(Variant::WhisperNative).unwrap();
        assert!(native.error.as_deref().unwrap().contains("model not found"));
        assert_eq!(report.recommended, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_cancelled_benchmark_stops() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(dir.path(), Variant::WhisperNative, "exec sleep 30");
        let input = fake_input(dir.path(), vec![Variant::WhisperNative]);
        let start = Instant::now();
        let result = run_benchmark(&input, &mut |_| {}, &AtomicBool::new(true));
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn zero_timed_runs_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(dir.path(), Variant::WhisperNative, "exit 0");
        let mut input = fake_input(dir.path(), vec![Variant::WhisperNative]);
        input.runs = 0;
        assert!(run_benchmark(&input, &mut |_| {}, &AtomicBool::new(false)).is_err());
    }

    /// A fake `transcribe` that loads for `load` seconds, then transcribes for
    /// `transcribe` seconds, and prints the passage with those timings.
    #[cfg(unix)]
    fn timed_script(load: f64, transcribe: f64) -> String {
        format!(
            "sleep {load}\nprintf 'INFO Model loaded in {load}s\\n'\nsleep {transcribe}\nprintf 'INFO Transcription completed in {transcribe}s: ok\\n\\n%s\\n' \"{PASSAGE}\""
        )
    }

    /// Wall time is not the metric: a build that loads slowly but transcribes
    /// faster must finish and win, while a build that only loads faster is
    /// stopped once its transcription falls behind.
    #[cfg(unix)]
    #[test]
    fn a_slow_loading_faster_transcriber_is_not_stopped() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(dir.path(), Variant::WhisperNative, &timed_script(0.05, 0.6));
        fake_variant(dir.path(), Variant::WhisperAvx2, &timed_script(1.5, 0.1));
        let input = fake_input(
            dir.path(),
            vec![Variant::WhisperNative, Variant::WhisperAvx2],
        );
        let report = run_benchmark(&input, &mut |_| {}, &AtomicBool::new(false)).unwrap();

        let slow_loader = report.result(Variant::WhisperAvx2).unwrap();
        assert!(slow_loader.stopped.is_none(), "{:?}", slow_loader.stopped);
        assert_eq!(report.recommended, Some(Variant::WhisperAvx2));
        assert!(
            report
                .result(Variant::WhisperNative)
                .unwrap()
                .stopped
                .is_some(),
            "the slower transcriber is stopped during its transcription"
        );
    }

    /// A GPU build's first run can include shader or kernel compilation, so it
    /// is not stopped during warm-up even when that run is slow.
    #[cfg(unix)]
    #[test]
    fn a_gpu_build_is_not_stopped_during_warm_up() {
        let dir = tempfile::tempdir().unwrap();
        fake_variant(dir.path(), Variant::WhisperNative, &timed_script(0.05, 0.3));
        let marker = dir.path().join("compiled");
        fake_variant(
            dir.path(),
            Variant::WhisperVulkan,
            &format!(
                "if [ ! -e '{m}' ]; then touch '{m}'; sleep 5; fi\n{t}",
                m = marker.display(),
                t = timed_script(0.05, 0.1)
            ),
        );
        let input = fake_input(
            dir.path(),
            vec![Variant::WhisperNative, Variant::WhisperVulkan],
        );
        let report = run_benchmark(&input, &mut |_| {}, &AtomicBool::new(false)).unwrap();

        let vulkan = report.result(Variant::WhisperVulkan).unwrap();
        assert!(vulkan.stopped.is_none(), "{:?}", vulkan.stopped);
        assert_eq!(vulkan.runs_secs, vec![0.1]);
    }
}
