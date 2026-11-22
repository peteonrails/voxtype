//! Is the *running* daemon actually GPU-accelerated?
//!
//! `voxtype info accel` answers that from observed facts, never from config.
//! Config records intent: `whisper.gpu_device`, a GPU-capable binary, an
//! engine that supports MIGraphX. None of it proves the daemon currently
//! loaded a model onto a GPU. Every GPU stack voxtype uses falls back to CPU
//! silently — whisper.cpp reports `use gpu = 0` and carries on, and ONNX
//! Runtime drops to the CPU execution provider when an EP fails to register
//! — so a report built from intent would tell users their transcription is
//! accelerated while it runs at CPU speed. When the evidence doesn't decide
//! the question, this says `unknown` rather than guessing.
//!
//! Evidence, in order of authority:
//!
//! 1. **The daemon's own answer.** The daemon writes its acceleration state
//!    to `$XDG_RUNTIME_DIR/voxtype/accel.json` when it loads a model (see
//!    [`AccelStateFile`] and [`publish`]); nothing else can know as reliably.
//!    It is preferred when the file exists and names the running PID. Today
//!    the local Whisper (whisper.cpp) and OpenVINO paths write it, and so do
//!    the GGUF engines, from the backend `voxtype-gguf-worker` reports once
//!    transcribe.cpp has bound one (see [`publish_gguf_worker`]). Engines that
//!    don't fall back to the evidence below, except a GGUF engine: the
//!    evidence below describes the main binary, which never runs its model,
//!    so with nothing published the worker's own `gguf_backend.json` report
//!    stands in (a daemon from before GGUF publishing), or the answer is
//!    `unknown`.
//! 2. **VRAM held by the daemon's PID**, via `rocm-smi` or `nvidia-smi`.
//!    Proof of GPU use when present. Absence proves nothing: a daemon with
//!    `on_demand_loading` holds no VRAM while idle.
//! 3. **The daemon's journal**, scoped to its PID. Distinguishes lines that
//!    prove acceleration (`ggml_vulkan: Found 1 Vulkan devices`,
//!    `whisper_backend_init_gpu: using Vulkan0 backend`) from lines that only
//!    state intent (`use gpu = 1`, `Configuring MIGraphX execution provider`,
//!    `registering execution providers ["CUDA"]`) — intent never decides
//!    `gpu`. whisper.cpp prints `use gpu` from the context params before it
//!    opens a device, so it records what was asked for; the answer comes
//!    later from `whisper_backend_init_gpu`, which says `no GPU found` and
//!    falls back to CPU when the request cannot be met.
//! 4. **The variant of the binary behind the PID**, read from
//!    `/proc/<pid>/exe` rather than the `/usr/bin/voxtype` symlink, since the
//!    symlink describes what a *new* process would launch. A CPU-only variant
//!    settles the question on its own.
//!
//! Deliberately not treated as negative evidence: ONNX Runtime's "Some nodes
//! were not assigned to the preferred execution providers" warning. That is
//! partial assignment, which still runs most of the graph on the GPU.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::binary::{self, Acceleration, Variant};
use crate::config::{ActiveGguf, Config};
use crate::daemon_status;

/// How the running daemon is transcribing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelState {
    /// Observed running on a GPU.
    Gpu,
    /// GPU-capable, but observed running on CPU.
    CpuFallback,
    /// The binary has no GPU support compiled in.
    CpuOnly,
    /// GPU-capable and nothing observed either way. Not a synonym for CPU.
    Unknown,
    /// No daemon to ask.
    NotRunning,
}

impl AccelState {
    pub const fn tag(self) -> &'static str {
        match self {
            AccelState::Gpu => "gpu",
            AccelState::CpuFallback => "cpu-fallback",
            AccelState::CpuOnly => "cpu-only",
            AccelState::Unknown => "unknown",
            AccelState::NotRunning => "not-running",
        }
    }

    /// One line a user can act on.
    pub const fn explanation(self) -> &'static str {
        match self {
            AccelState::Gpu => "The daemon is running on the GPU.",
            AccelState::CpuFallback => {
                "The daemon can use a GPU but is running on CPU. The evidence below says why."
            }
            AccelState::CpuOnly => {
                "This binary has no GPU support. Switch variants with: voxtype setup variant --to <NAME>"
            }
            AccelState::Unknown => {
                "Not determinable. Transcribe once so the daemon loads a model, then re-run."
            }
            AccelState::NotRunning => {
                "No daemon is running. Start it with: systemctl --user start voxtype"
            }
        }
    }
}

/// What `voxtype info accel` reports.
#[derive(Debug, Clone)]
pub struct AccelReport {
    pub state: AccelState,
    /// GPU stack in play or attempted. Names the backend even when `state`
    /// isn't `gpu`, so a fallback still says which stack fell back; it is
    /// never itself a claim of acceleration.
    pub backend: Option<&'static str>,
    /// The device the model runs on, when the daemon published one.
    pub device: Option<String>,
    /// Every observation the verdict rests on, each tagged with its source.
    pub evidence: Vec<String>,
    pub variant: Option<Variant>,
    /// PID the report describes, when a daemon was found.
    pub pid: Option<i32>,
}

impl AccelReport {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state.tag(),
            "backend": self.backend,
            "device": self.device,
            "evidence": self.evidence,
            "variant": self.variant.and_then(|v| serde_json::to_value(v).ok()),
            "pid": self.pid,
        })
    }
}

/// Contents of `$XDG_RUNTIME_DIR/voxtype/accel.json`.
///
/// Written by [`publish`] once per model load, after the backend has actually
/// initialized, and removed by the daemon at startup and shutdown. `pid` exists
/// so a file left by a crashed predecessor is ignored rather than believed.
/// `state` uses the same strings as [`AccelState::tag`], and `detail` is a
/// free-form line for the evidence list (e.g. the device name). `device` is
/// optional so files written before it existed still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccelStateFile {
    pub pid: i32,
    pub state: String,
    pub backend: Option<String>,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

/// Where the daemon publishes its acceleration state.
pub fn state_file_path() -> PathBuf {
    Config::runtime_dir().join("accel.json")
}

/// Which daemon a model load in this process belongs to: the daemon itself,
/// or its `gpu_isolation` transcription worker. `None` for anything else
/// (`voxtype transcribe <file>`, tests), which must not overwrite the running
/// daemon's answer.
fn owning_daemon_pid() -> Option<i32> {
    let daemon = daemon_status::read_pid_if_alive()?;
    let me = std::process::id() as i32;
    let parent = std::os::unix::process::parent_id() as i32;
    (daemon == me || daemon == parent).then_some(daemon)
}

/// Record what a model load actually got, for `voxtype info accel`.
///
/// Returns `true` when this is news for the running daemon: the published
/// state changed. Callers use that to notify once per daemon start (or once
/// per distinct outcome) rather than on every model load, including loads in
/// a fresh `gpu_isolation` worker process, which has no memory of its own.
/// Returns `false`, and writes nothing, outside a daemon.
pub fn publish(state: AccelState, backend: Option<&str>, detail: Option<String>) -> bool {
    let Some(pid) = owning_daemon_pid() else {
        return false;
    };
    publish_at(&state_file_path(), pid, state, backend, None, detail)
}

pub(crate) fn publish_at(
    path: &Path,
    pid: i32,
    state: AccelState,
    backend: Option<&str>,
    device: Option<String>,
    detail: Option<String>,
) -> bool {
    let file = AccelStateFile {
        pid,
        state: state.tag().to_string(),
        backend: backend.map(str::to_string),
        detail,
        device,
    };
    if let Some(prev) = read_state_file_at(path, pid) {
        if prev.state == file.state
            && prev.backend == file.backend
            && prev.detail == file.detail
            && prev.device == file.device
        {
            return false;
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string(&file) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                tracing::debug!("Could not write {}: {}", path.display(), e);
            }
        }
        Err(e) => tracing::debug!("Could not serialize acceleration state: {}", e),
    }
    true
}

/// What `voxtype-gguf-worker` writes to `$XDG_RUNTIME_DIR/voxtype/gguf_backend.json`
/// (see `status_json::GGUF_STATUS_FILE`) once transcribe.cpp has bound a
/// backend. Workers before 1.3.1 wrote only `model`, `backend` and `device`.
#[derive(Debug, Clone, Deserialize)]
pub struct GgufWorkerReport {
    /// Path of the model the worker loaded.
    pub model: String,
    /// transcribe.cpp's bound backend: "cpu", "vulkan", "cuda", "ROCm", "metal".
    pub backend: String,
    /// ggml device name, e.g. "Vulkan0".
    #[serde(default)]
    pub device: Option<String>,
    /// Human-readable device, e.g. "AMD Radeon RX 7800 XT (RADV NAVI32)".
    #[serde(default)]
    pub description: Option<String>,
    /// transcribe.cpp's vendor kind: "cpu", "accel", "vulkan", "cuda", ...
    #[serde(default)]
    pub kind: Option<String>,
    /// The worker's own PID, so a report from another worker (a concurrent
    /// `voxtype transcribe`, say) is not mistaken for this one's.
    #[serde(default)]
    pub pid: Option<u32>,
}

/// What a GGUF worker's report means for `voxtype info accel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GgufOutcome {
    pub(crate) state: AccelState,
    pub(crate) backend: &'static str,
    pub(crate) device: Option<String>,
    pub(crate) detail: String,
}

/// The backend tag for a transcribe.cpp backend name, `"cpu"` for its CPU
/// backends, or `None` for a name this doesn't recognise.
fn gguf_backend_tag(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    if name.contains("vulkan") {
        Some("vulkan")
    } else if name.contains("cuda") {
        Some("cuda")
    } else if name.contains("rocm") || name.contains("hip") {
        Some("rocm")
    } else if name.contains("metal") {
        Some("metal")
    } else if name.starts_with("cpu") || name == "accel" {
        Some("cpu")
    } else {
        None
    }
}

fn file_name_of(path: &str) -> Option<&std::ffi::OsStr> {
    Path::new(path).file_name()
}

/// Turn the worker's report into an acceleration outcome, or `None` when the
/// report is not from this worker (another PID, another model) or names a
/// backend this doesn't recognise. `worker_pid` is the worker the caller
/// expects, when it knows; `active` is the engine the worker was started for,
/// and its `backend` is the configured `gguf_backend`.
pub(crate) fn gguf_outcome(
    report: &GgufWorkerReport,
    worker_pid: Option<u32>,
    active: &ActiveGguf,
) -> Option<GgufOutcome> {
    if let (Some(expected), Some(reported)) = (worker_pid, report.pid) {
        if expected != reported {
            return None;
        }
    }
    if file_name_of(&report.model) != file_name_of(active.model) {
        return None;
    }
    let backend = gguf_backend_tag(&report.backend)
        .or_else(|| report.kind.as_deref().and_then(gguf_backend_tag))?;
    let device = match (report.description.as_deref(), report.device.as_deref()) {
        (Some(desc), Some(name)) if !desc.is_empty() && desc != name => {
            Some(format!("{desc} [{name}]"))
        }
        (Some(desc), _) if !desc.is_empty() => Some(desc.to_string()),
        (_, Some(name)) if !name.is_empty() => Some(name.to_string()),
        _ => None,
    };
    let section = active.section;
    let configured = active.backend;
    let who = match report.pid.or(worker_pid) {
        Some(pid) => format!("voxtype-gguf-worker pid {pid} runs the {section} GGUF model"),
        None => format!("voxtype-gguf-worker runs the {section} GGUF model"),
    };

    if backend != "cpu" {
        let detail = match &device {
            Some(device) => format!("{who} on {device}"),
            None => format!("{who} on {backend}"),
        };
        return Some(GgufOutcome {
            state: AccelState::Gpu,
            backend,
            device,
            detail,
        });
    }

    let asked_for_cpu = matches!(
        configured.to_ascii_lowercase().as_str(),
        "cpu" | "cpu_accel"
    );
    let detail = if asked_for_cpu {
        format!(
            "{who} on the CPU because {section}.gguf_backend = \"{configured}\"; \
             set it to \"vulkan\" to use the GPU"
        )
    } else {
        format!("{who} on the CPU; {section}.gguf_backend = \"{configured}\" did not bind a GPU")
    };
    Some(GgufOutcome {
        state: AccelState::CpuFallback,
        backend,
        device,
        detail,
    })
}

/// Where the GGUF worker writes its report.
fn gguf_report_path() -> PathBuf {
    Config::runtime_dir().join(crate::status_json::GGUF_STATUS_FILE)
}

pub(crate) fn read_gguf_report_at(path: &Path) -> Option<GgufWorkerReport> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The parent of a live process, from `/proc/<pid>/stat`.
fn parent_pid(pid: u32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesised and may contain spaces; the fields
    // after its closing parenthesis are state, then the parent PID.
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Publish what a GGUF worker bound, for `voxtype info accel`. The worker runs
/// in its own process and writes nothing to the daemon's journal, so without
/// this the report could only describe the main binary's variant.
///
/// Called by the daemon once the worker it spawned says READY; the worker
/// writes its report before that. `section`, `model` and `configured` are the
/// engine's config section, model and `gguf_backend`. Returns `false`, and
/// writes nothing, outside a daemon or when the report is missing or belongs
/// to another worker.
pub fn publish_gguf_worker(section: &str, model: &str, configured: &str, worker_pid: u32) -> bool {
    let Some(pid) = owning_daemon_pid() else {
        return false;
    };
    let Some(report) = read_gguf_report_at(&gguf_report_path()) else {
        return false;
    };
    let section: &'static str = match section {
        "cohere" => "cohere",
        "parakeet" => "parakeet",
        _ => "gguf",
    };
    let active = ActiveGguf {
        section,
        model,
        backend: configured,
    };
    let Some(outcome) = gguf_outcome(&report, Some(worker_pid), &active) else {
        return false;
    };
    publish_at(
        &state_file_path(),
        pid,
        outcome.state,
        Some(outcome.backend),
        outcome.device,
        Some(outcome.detail),
    )
}

/// Remove the published state. The daemon calls this at startup and shutdown.
pub fn clear() {
    let _ = std::fs::remove_file(state_file_path());
}

/// Whether a journal line proves acceleration, disproves it, or merely states
/// an intention to try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    /// The GPU is in use.
    Positive,
    /// The GPU was meant to be used and isn't.
    Negative,
    /// An attempt was configured. Says nothing about the outcome.
    Intent,
}

/// One distinct marker, with how many times the daemon logged it. A daemon
/// that has reloaded its model fifty times logs the same line fifty times;
/// the count is worth keeping, fifty copies of the text are not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) text: String,
    pub(crate) count: usize,
}

impl Observation {
    /// Evidence-list rendering: the message, plus the repeat count when it
    /// happened more than once.
    fn render(&self) -> String {
        if self.count > 1 {
            format!("journal: {} ({} occurrences)", self.text, self.count)
        } else {
            format!("journal: {}", self.text)
        }
    }
}

/// Distinct markers of one kind are capped: past the first few, more copies of
/// the same finding don't change the verdict or help the reader.
const MAX_OBSERVATIONS_PER_KIND: usize = 3;

/// Observations gathered about one daemon.
#[derive(Debug, Default, Clone)]
pub(crate) struct Signals {
    pub(crate) positive: Vec<Observation>,
    pub(crate) negative: Vec<Observation>,
    pub(crate) intent: Vec<Observation>,
    /// Backend named by the strongest marker seen.
    pub(crate) backend: Option<&'static str>,
}

impl Signals {
    fn record(&mut self, line: &str) {
        let Some((signal, backend)) = classify_line(line) else {
            return;
        };
        if let Some(b) = backend {
            self.backend = Some(b);
        }
        let text = message_of(line);
        if text.is_empty() {
            return;
        }
        let bucket = match signal {
            Signal::Positive => &mut self.positive,
            Signal::Negative => &mut self.negative,
            Signal::Intent => &mut self.intent,
        };
        if let Some(existing) = bucket.iter_mut().find(|o| o.text == text) {
            existing.count += 1;
        } else if bucket.len() < MAX_OBSERVATIONS_PER_KIND {
            bucket.push(Observation { text, count: 1 });
        }
    }

    fn evidence_lines(&self) -> Vec<String> {
        self.positive
            .iter()
            .chain(self.negative.iter())
            .chain(self.intent.iter())
            .map(Observation::render)
            .collect()
    }
}

/// The message part of a log line: colour codes gone, and the `tracing`
/// timestamp and level prefix dropped so repeats of one marker compare equal.
/// Lines that don't carry that prefix (ONNX Runtime writes straight to stderr)
/// are left as they are.
pub(crate) fn message_of(line: &str) -> String {
    let plain = strip_ansi(line);
    let mut rest = plain.trim_start();

    // A `tracing` timestamp: RFC3339, always contains 'T' and ends in 'Z'.
    if let Some((first, tail)) = rest.split_once(char::is_whitespace) {
        if first.contains('T') && first.ends_with('Z') {
            rest = tail.trim_start();
            if let Some((level, tail)) = rest.split_once(char::is_whitespace) {
                if matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR") {
                    rest = tail.trim_start();
                }
            }
        }
    }
    rest.trim().to_string()
}

/// Classify one log line. Matching is case-insensitive and substring-based;
/// the point is to recognise the handful of markers the GPU stacks emit, not
/// to parse logs in general.
pub(crate) fn classify_line(line: &str) -> Option<(Signal, Option<&'static str>)> {
    // Runs of whitespace collapse first: whisper.cpp pads its init table into
    // columns (`use gpu    = 1`), so patterns written with single spaces would
    // otherwise miss the real output.
    let l = line
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // whisper.cpp / ggml. `use gpu = 1` is the requested setting, printed
    // from the context params before any device is opened, so it is intent.
    // `use gpu = 0` is different: the GPU was never even asked for, which
    // settles the question on its own.
    if l.contains("use gpu = 1") {
        return Some((Signal::Intent, None));
    }
    if l.contains("use gpu = 0") {
        return Some((Signal::Negative, None));
    }
    // whisper_backend_init_gpu reports the outcome of that request. Its
    // per-device enumeration line (`device 0: CPU (type: 0)`) is neither
    // proof nor disproof and deliberately falls through.
    if l.contains("whisper_backend_init_gpu:") {
        if l.contains("no gpu found") {
            return Some((Signal::Negative, None));
        }
        if l.contains("found gpu device") || (l.contains("using") && l.contains("backend")) {
            let backend = if l.contains("vulkan") {
                Some("vulkan")
            } else if l.contains("cuda") {
                Some("cuda")
            } else {
                None
            };
            return Some((Signal::Positive, backend));
        }
    }
    if l.contains("ggml_vulkan") && l.contains("found 0") {
        return Some((Signal::Negative, Some("vulkan")));
    }
    if l.contains("ggml_vulkan") && l.contains("found") {
        return Some((Signal::Positive, Some("vulkan")));
    }
    if l.contains("ggml_cuda_init") && l.contains("found") {
        return Some((Signal::Positive, Some("cuda")));
    }
    if l.contains("no cuda devices") || l.contains("cuda error") {
        return Some((Signal::Negative, Some("cuda")));
    }

    // ONNX Runtime failures. The provider .so files are dlopened at runtime,
    // so a missing companion library shows up here and nowhere else.
    // ORT names the provider by full path when it sits beside the binary
    // ("Failed to load library /usr/lib/voxtype/cuda-13/libonnxruntime_...").
    if l.contains("cudnn") && (l.contains("cannot open") || l.contains("missing")) {
        return Some((Signal::Negative, Some("cuda")));
    }
    if (l.contains("failed to load library") && l.contains("libonnxruntime_providers"))
        || (l.contains("failed to create") && l.contains("executionprovider"))
        || l.contains("falling back to cpu")
        || l.contains("ep registration failed")
    {
        return Some((Signal::Negative, None));
    }

    // voxtype's own "about to try this EP" lines. Intent only: they are
    // logged before registration, and ort silently uses CPU if it fails.
    if l.contains("execution provider") || l.contains("execution providers") {
        let backend = if l.contains("migraphx") {
            Some("migraphx")
        } else if l.contains("cuda") || l.contains("tensorrt") {
            // TensorRT is an NVIDIA EP; `backend` reports the vendor stack
            // and the evidence line preserves which EP was named.
            Some("cuda")
        } else {
            None
        };
        return Some((Signal::Intent, backend));
    }

    None
}

/// Journal patterns worth pulling out of a daemon's log. Passed to
/// `journalctl -g` so the filtering happens there rather than by streaming
/// every line of a weeks-old daemon's journal into this process.
const JOURNAL_PATTERN: &str = "use gpu|whisper_backend_init_gpu|ggml_vulkan|ggml_cuda|execution provider|onnxruntime|falling back to cpu|no cuda devices|cudnn";

/// Read the GPU-relevant lines this PID logged.
fn scan_journal(pid: i32) -> Signals {
    let mut signals = Signals::default();
    let output = Command::new("journalctl")
        .args([
            "--user",
            &format!("_PID={}", pid),
            "--no-pager",
            "--output=cat",
            "--case-sensitive=false",
            "-g",
            JOURNAL_PATTERN,
            // Newest matches; a long-lived daemon may have reloaded models
            // many times and the most recent load is the relevant one.
            "-n",
            "200",
        ])
        .output();
    let Ok(output) = output else {
        return signals;
    };
    if !output.status.success() {
        return signals;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        signals.record(line);
    }
    signals
}

/// VRAM the PID holds, in bytes, if a GPU tool reports any.
///
/// Best effort on purpose: the tools are optional, their output formats vary
/// between driver versions, and a `None` here is never read as evidence of
/// CPU execution.
fn vram_held_by(pid: i32) -> Option<(u64, &'static str)> {
    if let Some(bytes) = run_and_parse("rocm-smi", &["--showpids"], |out| {
        parse_rocm_smi_pids(out, pid)
    }) {
        return Some((bytes, "rocm-smi"));
    }
    if let Some(bytes) = run_and_parse(
        "nvidia-smi",
        &[
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ],
        |out| parse_nvidia_smi_apps(out, pid),
    ) {
        return Some((bytes, "nvidia-smi"));
    }
    None
}

fn run_and_parse(program: &str, args: &[&str], parse: impl Fn(&str) -> Option<u64>) -> Option<u64> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// Pull one PID's VRAM out of `rocm-smi --showpids`, whose KFD table is
/// tab-separated with byte counts:
///
/// ```text
/// PID     PROCESS NAME    GPU(s)  VRAM USED       SDMA USED       CU OCCUPANCY
/// 3094859 voxtype-onnx-mi 1       392437760       0               UNKNOWN
/// ```
pub(crate) fn parse_rocm_smi_pids(out: &str, pid: i32) -> Option<u64> {
    for line in out.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[0] != pid.to_string() {
            continue;
        }
        // VRAM is the field after the GPU count; the process name is a single
        // token because rocm-smi truncates it to 15 characters.
        if let Ok(bytes) = fields[3].parse::<u64>() {
            return Some(bytes);
        }
    }
    None
}

/// Pull one PID's VRAM out of `nvidia-smi --query-compute-apps=pid,used_memory
/// --format=csv,noheader,nounits`, which reports mebibytes:
///
/// ```text
/// 1234, 812
/// ```
pub(crate) fn parse_nvidia_smi_apps(out: &str, pid: i32) -> Option<u64> {
    for line in out.lines() {
        let mut parts = line.split(',');
        let found: i32 = parts.next()?.trim().parse().ok()?;
        if found != pid {
            continue;
        }
        let mib: u64 = parts.next()?.trim().parse().ok()?;
        return Some(mib * 1024 * 1024);
    }
    None
}

/// The variant of the binary the daemon is actually executing.
///
/// Resolved through `/proc/<pid>/exe`, not the `/usr/bin/voxtype` symlink: the
/// symlink says what the *next* process would be, and a daemon started before
/// a variant switch (or from a systemd drop-in pointing elsewhere) keeps
/// running the old binary.
fn running_variant(pid: i32) -> (Option<Variant>, Option<String>) {
    let exe = binary::running_binary_path(pid);
    match exe {
        Some(path) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let variant = Variant::from_binary_name(&name);
            let note = match variant {
                Some(v) => format!(
                    "binary: {} (variant {})",
                    path.display(),
                    v.binary_name().trim_start_matches("voxtype-")
                ),
                None => format!(
                    "binary: {} (not a packaged variant; GPU support unknown from the name)",
                    path.display()
                ),
            };
            (variant, Some(note))
        }
        // /proc is unreadable for another user's process, and absent on macOS.
        None => (
            binary::active_variant(),
            Some(
                "binary: could not read /proc/<pid>/exe; fell back to the /usr/bin/voxtype symlink"
                    .to_string(),
            ),
        ),
    }
}

/// Read the daemon's published state, ignoring a file that belongs to a dead
/// predecessor.
pub(crate) fn read_state_file_at(path: &Path, pid: i32) -> Option<AccelStateFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: AccelStateFile = serde_json::from_str(&raw).ok()?;
    (parsed.pid == pid).then_some(parsed)
}

/// Is this variant able to use a GPU at all? `None` when the binary's name
/// doesn't settle it (a source build, or an unrecognised name).
pub(crate) fn gpu_capable(variant: Option<Variant>) -> Option<bool> {
    match variant.map(|v| v.acceleration()) {
        Some(Acceleration::Vulkan | Acceleration::Cuda | Acceleration::Migraphx) => Some(true),
        Some(Acceleration::Baseline | Acceleration::Avx2 | Acceleration::Avx512) => Some(false),
        Some(Acceleration::Native) | None => None,
    }
}

/// The backend a GPU-capable variant would use.
pub(crate) fn variant_backend(variant: Option<Variant>) -> Option<&'static str> {
    match variant.map(|v| v.acceleration()) {
        Some(Acceleration::Vulkan) => Some("vulkan"),
        Some(Acceleration::Cuda) => Some("cuda"),
        Some(Acceleration::Migraphx) => Some("migraphx"),
        _ => None,
    }
}

/// Turn observations into a verdict.
///
/// A CPU-only binary settles the question by itself. Otherwise proof outranks
/// disproof outranks silence, and silence is reported as silence: intent lines
/// alone never produce `gpu`.
pub(crate) fn decide(
    variant: Option<Variant>,
    signals: &Signals,
    vram: Option<u64>,
) -> (AccelState, Option<&'static str>) {
    let backend = variant_backend(variant).or(signals.backend);

    if gpu_capable(variant) == Some(false) {
        // Any GPU marker here would be a contradiction; it stays in the
        // evidence list where a reader can see it.
        return (AccelState::CpuOnly, None);
    }
    if vram.is_some_and(|b| b > 0) || !signals.positive.is_empty() {
        return (AccelState::Gpu, backend);
    }
    if !signals.negative.is_empty() {
        return (AccelState::CpuFallback, backend);
    }
    (AccelState::Unknown, backend)
}

/// Build the report for whatever daemon is running now. `config` says which
/// engine is active; a GGUF engine runs in `voxtype-gguf-worker`, so the main
/// binary's variant and journal say nothing about it.
pub fn report(config: &Config) -> AccelReport {
    report_for(
        daemon_status::read_pid_if_alive(),
        config.active_gguf().as_ref(),
    )
}

/// The report pipeline, with the daemon lookup already done so tests can drive
/// it without one.
pub(crate) fn report_for(pid: Option<i32>, gguf: Option<&ActiveGguf>) -> AccelReport {
    let Some(pid) = pid else {
        return AccelReport {
            state: AccelState::NotRunning,
            backend: None,
            device: None,
            evidence: vec![format!(
                "daemon: no live PID in {}",
                daemon_status::pid_file_path().display()
            )],
            // Still worth reporting which binary *would* run.
            variant: binary::active_variant(),
            pid: None,
        };
    };

    let (variant, variant_note) = running_variant(pid);
    let mut evidence: Vec<String> = variant_note.into_iter().collect();

    // The daemon's own answer wins when it publishes one.
    let path = state_file_path();
    if let Some(published) = read_state_file_at(&path, pid) {
        let state = match published.state.as_str() {
            "gpu" => AccelState::Gpu,
            "cpu-fallback" => AccelState::CpuFallback,
            "cpu-only" => AccelState::CpuOnly,
            _ => AccelState::Unknown,
        };
        evidence.push(format!(
            "runtime: {} reports {}{}",
            path.display(),
            published.state,
            published
                .detail
                .as_deref()
                .map(|d| format!(" ({})", d))
                .unwrap_or_default()
        ));
        let backend = match published.backend.as_deref() {
            Some("vulkan") => Some("vulkan"),
            Some("cuda") => Some("cuda"),
            Some("migraphx") => Some("migraphx"),
            Some("metal") => Some("metal"),
            Some("hip") => Some("hip"),
            Some("rocm") => Some("rocm"),
            Some("openvino") => Some("openvino"),
            Some("cpu") => Some("cpu"),
            _ => variant_backend(variant),
        };
        return AccelReport {
            state,
            backend,
            device: published.device,
            evidence,
            variant,
            pid: Some(pid),
        };
    }

    if let Some(gguf) = gguf {
        // The daemon removes the worker's report at startup, so one that is
        // here was written since. A report naming its worker must name one of
        // this daemon's children.
        let path = gguf_report_path();
        let worker = read_gguf_report_at(&path)
            .filter(|r| r.pid.is_none_or(|worker| parent_pid(worker) == Some(pid)));
        return unpublished_gguf_report(pid, gguf, variant, evidence, worker, &path);
    }

    let mut signals = scan_journal(pid);
    let vram = vram_held_by(pid);

    // A provider that cannot resolve its libraries can never load, whatever
    // the journal says or no longer says after rotation.
    let mut fix: Vec<String> = Vec::new();
    if let Some(exe) = binary::running_binary_path(pid) {
        if let Some(missing) =
            super::gpu::unresolved_provider_deps_at(&exe).filter(|m| !m.is_empty())
        {
            let name = exe
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            signals.negative.push(Observation {
                text: format!(
                    "the GPU execution provider beside {} cannot load: missing {}",
                    exe.display(),
                    missing.join(", ")
                ),
                count: 1,
            });
            fix = super::gpu::provider_fix_lines(&name, &missing);
        }
    }
    if fix.is_empty()
        && signals
            .negative
            .iter()
            .any(|o| o.text.to_ascii_lowercase().contains("cudnn"))
    {
        fix = super::parakeet::CUDNN_INSTALL_HINT
            .lines()
            .map(str::to_string)
            .collect();
    }

    if let Some((bytes, tool)) = vram {
        evidence.push(format!(
            "vram: {} MiB held by pid {} ({})",
            bytes / 1024 / 1024,
            pid,
            tool
        ));
    }
    evidence.extend(signals.evidence_lines());
    evidence.extend(fix.iter().map(|line| format!("fix: {}", line.trim())));

    let (state, backend) = decide(variant, &signals, vram.map(|(bytes, _)| bytes));
    if state == AccelState::Unknown && signals.intent.is_empty() && signals.positive.is_empty() {
        evidence.push(format!(
            "journal: no GPU markers found for pid {} (log may have rotated, or no model has been loaded yet)",
            pid
        ));
    }

    AccelReport {
        state,
        backend,
        device: None,
        evidence,
        variant,
        pid: Some(pid),
    }
}

/// A GGUF engine with nothing in `accel.json`. A daemon from before GGUF
/// publishing still has its worker's own report, `worker`, which says what
/// transcribe.cpp bound. Without one the worker has not loaded the model yet
/// (on-demand loading, or no dictation since the daemon started). Either way
/// the main binary's variant, journal and VRAM describe a process that is not
/// running the model, so none of them decide anything here; with nothing
/// reported, the backend is the configured `gguf_backend`, the one that will
/// be attempted.
fn unpublished_gguf_report(
    pid: i32,
    gguf: &ActiveGguf,
    variant: Option<Variant>,
    mut evidence: Vec<String>,
    worker: Option<GgufWorkerReport>,
    worker_path: &Path,
) -> AccelReport {
    evidence.push(format!(
        "engine: {section} runs a GGUF model in voxtype-gguf-worker with \
         {section}.gguf_backend = \"{backend}\"; the main binary's journal and VRAM do not \
         describe it",
        section = gguf.section,
        backend = gguf.backend,
    ));
    if let Some(outcome) = worker.and_then(|report| gguf_outcome(&report, None, gguf)) {
        evidence.push(format!(
            "worker: {} reports {} ({})",
            worker_path.display(),
            outcome.backend,
            outcome.detail
        ));
        return AccelReport {
            state: outcome.state,
            backend: Some(outcome.backend),
            device: outcome.device,
            evidence,
            variant,
            pid: Some(pid),
        };
    }
    evidence.push(format!(
        "runtime: no GGUF backend reported for pid {pid} (the worker has not loaded the \
         model yet)"
    ));
    AccelReport {
        state: AccelState::Unknown,
        backend: gguf_backend_tag(gguf.backend),
        device: None,
        evidence,
        variant,
        pid: Some(pid),
    }
}

/// Drop terminal colour codes so evidence lines are readable in JSON. The
/// daemon's `tracing` output carries them even when it writes to the journal.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Consume up to and including the sequence's final byte.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals_from(lines: &[&str]) -> Signals {
        let mut s = Signals::default();
        for line in lines {
            s.record(line);
        }
        s
    }

    #[test]
    fn whisper_gpu_markers_are_read_both_ways() {
        assert_eq!(
            classify_line("whisper_init_with_params_no_state: use gpu    = 1"),
            Some((Signal::Intent, None))
        );
        assert_eq!(
            classify_line("whisper_init_with_params_no_state: use gpu    = 0"),
            Some((Signal::Negative, None))
        );
        assert_eq!(
            classify_line("ggml_vulkan: Found 1 Vulkan devices:"),
            Some((Signal::Positive, Some("vulkan")))
        );
        assert_eq!(
            classify_line("ggml_vulkan: Found 0 Vulkan devices"),
            Some((Signal::Negative, Some("vulkan")))
        );
        assert_eq!(
            classify_line("ggml_cuda_init: found 1 CUDA devices"),
            Some((Signal::Positive, Some("cuda")))
        );
    }

    /// whisper.cpp asks for a GPU and then reports whether it got one. The
    /// request line and the answer line are two different facts.
    #[test]
    fn whisper_reports_the_outcome_not_just_the_request() {
        assert_eq!(
            classify_line("whisper_backend_init_gpu: no GPU found"),
            Some((Signal::Negative, None))
        );
        assert_eq!(
            classify_line(
                "whisper_backend_init_gpu: found GPU device 0: Vulkan0 (type: 1, cnt: 1)"
            ),
            Some((Signal::Positive, Some("vulkan")))
        );
        assert_eq!(
            classify_line("whisper_backend_init_gpu: using CUDA0 backend"),
            Some((Signal::Positive, Some("cuda")))
        );
        // Device enumeration lists whatever ggml can see, CPU included. It
        // decides nothing on its own.
        assert_eq!(
            classify_line("whisper_backend_init_gpu: device 0: CPU (type: 0)"),
            None
        );
    }

    /// The Vulkan build on a machine with no usable Vulkan device: the daemon
    /// asks for the GPU, whisper finds none, transcription runs on the CPU.
    /// Reported as `gpu` before the request/outcome split.
    #[test]
    fn requested_gpu_that_never_materialised_is_a_fallback() {
        let signals = signals_from(&[
            "whisper_init_with_params_no_state: use gpu    = 1",
            "whisper_backend_init_gpu: device 0: CPU (type: 0)",
            "whisper_backend_init_gpu: no GPU found",
        ]);
        let (state, backend) = decide(Some(Variant::WhisperVulkan), &signals, None);
        assert_eq!(
            state,
            AccelState::CpuFallback,
            "a GPU that was asked for but never found is a fallback, not acceleration"
        );
        assert_eq!(backend, Some("vulkan"));
    }

    /// The same build where the device is real.
    #[test]
    fn requested_gpu_that_materialised_is_acceleration() {
        let signals = signals_from(&[
            "whisper_init_with_params_no_state: use gpu    = 1",
            "whisper_backend_init_gpu: found GPU device 0: Vulkan0 (type: 1, cnt: 1)",
            "whisper_backend_init_gpu: using Vulkan0 backend",
        ]);
        let (state, _) = decide(Some(Variant::WhisperVulkan), &signals, None);
        assert_eq!(state, AccelState::Gpu);
    }

    /// The line the 0.7.5 daemon on an AMD box logs. It is printed *before*
    /// ort tries the EP, so it must not by itself produce a `gpu` verdict.
    #[test]
    fn onnx_provider_lines_are_intent_not_proof() {
        assert_eq!(
            classify_line("Configuring MIGraphX execution provider for AMD GPU acceleration"),
            Some((Signal::Intent, Some("migraphx")))
        );
        assert_eq!(
            classify_line("Parakeet encoder: registering execution providers [\"CUDA\"]"),
            Some((Signal::Intent, Some("cuda")))
        );

        let signals = signals_from(&["Configuring MIGraphX execution provider for AMD GPU"]);
        let (state, backend) = decide(Some(Variant::OnnxMigraphx), &signals, None);
        assert_eq!(
            state,
            AccelState::Unknown,
            "intent alone must not be reported as acceleration"
        );
        assert_eq!(backend, Some("migraphx"));
    }

    #[test]
    fn ep_failures_are_negative_evidence() {
        assert_eq!(
            classify_line(
                "[E:onnxruntime:Default, provider_bridge_ort.cc:1745] Failed to load library libonnxruntime_providers_cuda.so"
            ),
            Some((Signal::Negative, None))
        );
        assert_eq!(
            classify_line("Failed to create MIGraphXExecutionProvider"),
            Some((Signal::Negative, None))
        );
        // As ORT really logs it when the provider sits beside the binary
        // and cuDNN is not installed.
        assert_eq!(
            classify_line(
                "[E:onnxruntime:Default, provider_bridge_ort.cc:1745] Failed to load library /usr/lib/voxtype/cuda-13/libonnxruntime_providers_cuda.so with error: libcudnn.so.9: cannot open shared object file: No such file or directory"
            ),
            Some((Signal::Negative, Some("cuda")))
        );
        assert_eq!(
            classify_line(
                "ERROR CUDA is installed but cuDNN (libcudnn.so.9) is missing, so the CUDA execution provider cannot load: falling back to CPU."
            ),
            Some((Signal::Negative, Some("cuda")))
        );
        // Partial assignment still runs most of the graph on the GPU.
        assert_eq!(
            classify_line(
                "[W:onnxruntime:, session_state.cc:1166] Some nodes were not assigned to the preferred execution providers"
            ),
            Some((Signal::Intent, None)),
            "partial assignment is not a fallback"
        );
    }

    #[test]
    fn a_cpu_only_variant_settles_it_without_any_log() {
        let (state, backend) = decide(Some(Variant::OnnxAvx2), &Signals::default(), None);
        assert_eq!(state, AccelState::CpuOnly);
        assert_eq!(backend, None);

        let (state, _) = decide(Some(Variant::WhisperAvx512), &Signals::default(), None);
        assert_eq!(state, AccelState::CpuOnly);
    }

    #[test]
    fn vram_held_by_the_daemon_proves_acceleration() {
        let (state, backend) = decide(
            Some(Variant::OnnxMigraphx),
            &Signals::default(),
            Some(392_437_760),
        );
        assert_eq!(state, AccelState::Gpu);
        assert_eq!(backend, Some("migraphx"));
    }

    #[test]
    fn negative_evidence_on_a_gpu_variant_is_a_fallback() {
        let signals = signals_from(&["whisper_init: use gpu    = 0"]);
        let (state, backend) = decide(Some(Variant::WhisperVulkan), &signals, None);
        assert_eq!(state, AccelState::CpuFallback);
        assert_eq!(backend, Some("vulkan"));
    }

    /// A source build's name says nothing about GPU support, so silence has to
    /// stay silence rather than becoming either verdict.
    #[test]
    fn an_unrecognized_binary_with_no_evidence_is_unknown() {
        let (state, backend) = decide(None, &Signals::default(), None);
        assert_eq!(state, AccelState::Unknown);
        assert_eq!(backend, None);

        let (state, _) = decide(Some(Variant::OnnxNative), &Signals::default(), None);
        assert_eq!(state, AccelState::Unknown);
    }

    /// Real `rocm-smi --showpids` output from an AMD box running the daemon.
    #[test]
    fn rocm_smi_vram_is_parsed_for_the_right_pid() {
        let out = "\
WARNING: AMD GPU device(s) is/are in a low-power state.

============================ ROCm System Management Interface ============================
===================================== KFD Processes ======================================
KFD process information:
PID    \tPROCESS NAME   \tGPU(s)\tVRAM USED\tSDMA USED\tCU OCCUPANCY\t
3094859\tvoxtype-onnx-mi\t1     \t392437760\t0        \tUNKNOWN     \t
==========================================================================================
";
        assert_eq!(parse_rocm_smi_pids(out, 3_094_859), Some(392_437_760));
        assert_eq!(parse_rocm_smi_pids(out, 4_242_424), None);
    }

    #[test]
    fn nvidia_smi_vram_is_parsed_as_mebibytes() {
        let out = "1234, 812\n5678, 40\n";
        assert_eq!(parse_nvidia_smi_apps(out, 1234), Some(812 * 1024 * 1024));
        assert_eq!(parse_nvidia_smi_apps(out, 5678), Some(40 * 1024 * 1024));
        assert_eq!(parse_nvidia_smi_apps(out, 999), None);
    }

    /// The durable path: once the daemon publishes its own state, this command
    /// reports that instead of inferring, and ignores a stale file.
    #[test]
    fn a_published_state_file_is_preferred_and_pid_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accel.json");
        std::fs::write(
            &path,
            r#"{"pid":4242,"state":"gpu","backend":"vulkan","detail":"AMD Radeon"}"#,
        )
        .unwrap();

        let published = read_state_file_at(&path, 4242).expect("should read a matching pid");
        assert_eq!(published.state, "gpu");
        assert_eq!(published.backend.as_deref(), Some("vulkan"));

        assert!(
            read_state_file_at(&path, 5555).is_none(),
            "a file naming another pid is a leftover, not evidence"
        );
        assert!(read_state_file_at(&dir.path().join("absent.json"), 4242).is_none());
    }

    /// Publishing reports news only when the outcome changes, so a
    /// `gpu_isolation` worker that re-detects the same fallback on every
    /// transcription doesn't notify every time.
    #[test]
    fn publish_reports_news_once_per_distinct_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voxtype").join("accel.json");
        let detail = || Some("no Vulkan GPU found".to_string());

        assert!(publish_at(
            &path,
            10,
            AccelState::CpuFallback,
            Some("vulkan"),
            None,
            detail()
        ));
        assert!(!publish_at(
            &path,
            10,
            AccelState::CpuFallback,
            Some("vulkan"),
            None,
            detail()
        ));
        let read = read_state_file_at(&path, 10).unwrap();
        assert_eq!(read.state, "cpu-fallback");
        assert_eq!(read.detail, detail());

        // A different outcome in the same daemon is news again.
        assert!(publish_at(
            &path,
            10,
            AccelState::Gpu,
            Some("vulkan"),
            None,
            None
        ));
        // So is the same outcome in a new daemon.
        assert!(publish_at(
            &path,
            11,
            AccelState::Gpu,
            Some("vulkan"),
            None,
            None
        ));
    }

    #[test]
    fn state_file_round_trips_its_schema() {
        let file = AccelStateFile {
            pid: 7,
            state: AccelState::CpuFallback.tag().to_string(),
            backend: Some("migraphx".to_string()),
            detail: Some("MIGraphX EP failed to register".to_string()),
            device: None,
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: AccelStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.state, "cpu-fallback");
        assert_eq!(back.backend.as_deref(), Some("migraphx"));
    }

    #[test]
    fn evidence_lines_drop_terminal_colour_codes() {
        let raw = "\u{1b}[2m2026-08-07T21:02:03Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Configuring MIGraphX execution provider";
        assert_eq!(
            strip_ansi(raw),
            "2026-08-07T21:02:03Z  INFO Configuring MIGraphX execution provider"
        );
    }

    /// Repeats of one marker collapse, because a daemon that has reloaded its
    /// model twenty times logged the same line twenty times.
    #[test]
    fn repeated_markers_collapse_into_a_count() {
        let line = "\u{1b}[2m2026-08-14T22:14:16.047405Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Configuring MIGraphX execution provider for AMD GPU acceleration";
        let mut signals = Signals::default();
        for _ in 0..19 {
            signals.record(line);
        }
        assert_eq!(signals.intent.len(), 1);
        assert_eq!(signals.intent[0].count, 19);
        assert_eq!(
            signals.evidence_lines(),
            vec![
                "journal: Configuring MIGraphX execution provider for AMD GPU acceleration (19 occurrences)"
                    .to_string()
            ]
        );
    }

    #[test]
    fn distinct_markers_of_one_kind_are_capped() {
        let mut signals = Signals::default();
        for n in 0..10 {
            signals.record(&format!(
                "Configuring CUDA execution provider (device {})",
                n
            ));
        }
        assert_eq!(signals.intent.len(), MAX_OBSERVATIONS_PER_KIND);
    }

    #[test]
    fn message_of_strips_the_tracing_prefix_but_not_other_shapes() {
        assert_eq!(
            message_of("2026-08-07T21:02:03.115777Z  INFO Configuring MIGraphX execution provider"),
            "Configuring MIGraphX execution provider"
        );
        // ONNX Runtime writes its own format with no timestamp; leave it be.
        let ort = "[E:onnxruntime:Default, provider_bridge_ort.cc:1745] Failed to load library";
        assert_eq!(message_of(ort), ort);
        // whisper.cpp's stderr, also unprefixed.
        assert_eq!(
            message_of("  ggml_vulkan: Found 1 Vulkan devices"),
            "ggml_vulkan: Found 1 Vulkan devices"
        );
    }

    #[test]
    fn no_daemon_reports_not_running_without_probing_anything() {
        let report = report_for(None, None);
        assert_eq!(report.state, AccelState::NotRunning);
        assert_eq!(report.backend, None);
        assert_eq!(report.pid, None);
        assert!(
            report.evidence.iter().any(|e| e.starts_with("daemon:")),
            "{:?}",
            report.evidence
        );
        assert!(report.state.explanation().contains("systemctl"));
    }

    /// End to end against a live process that is not a voxtype daemon: the
    /// pipeline must come back `unknown` rather than inventing a verdict, and
    /// it must say what it looked for.
    #[test]
    fn a_process_with_no_gpu_history_is_unknown_not_cpu() {
        let report = report_for(Some(std::process::id() as i32), None);
        assert_eq!(report.state, AccelState::Unknown);
        assert!(
            report.evidence.iter().any(|e| e.starts_with("binary:")),
            "{:?}",
            report.evidence
        );
        assert!(
            report
                .evidence
                .iter()
                .any(|e| e.contains("no GPU markers found")),
            "{:?}",
            report.evidence
        );
    }

    #[test]
    fn state_tags_match_the_documented_vocabulary() {
        for (state, tag) in [
            (AccelState::Gpu, "gpu"),
            (AccelState::CpuFallback, "cpu-fallback"),
            (AccelState::CpuOnly, "cpu-only"),
            (AccelState::Unknown, "unknown"),
            (AccelState::NotRunning, "not-running"),
        ] {
            assert_eq!(state.tag(), tag);
            assert!(!state.explanation().is_empty());
        }
    }

    #[test]
    fn report_json_has_the_documented_shape() {
        let report = AccelReport {
            state: AccelState::Gpu,
            backend: Some("migraphx"),
            device: None,
            evidence: vec!["vram: 374 MiB held by pid 1 (rocm-smi)".to_string()],
            variant: Some(Variant::OnnxMigraphx),
            pid: Some(1),
        };
        let json = report.to_json();
        assert_eq!(json["state"], serde_json::json!("gpu"));
        assert_eq!(json["backend"], serde_json::json!("migraphx"));
        assert_eq!(json["variant"], serde_json::json!("onnx-migraphx"));
        assert!(json["evidence"].as_array().unwrap().len() == 1);
    }

    fn cohere(backend: &'static str) -> ActiveGguf<'static> {
        ActiveGguf {
            section: "cohere",
            model: "/home/u/.local/share/voxtype/models/cohere-transcribe-03-2026-Q4_K_M.gguf",
            backend,
        }
    }

    fn worker_report(json: &str) -> GgufWorkerReport {
        serde_json::from_str(json).unwrap()
    }

    /// #1085: a GGUF engine on Vulkan, reported by the worker the daemon
    /// spawned, is `gpu` on Vulkan with the device named, whatever the main
    /// binary's variant is.
    #[test]
    fn a_gguf_worker_on_vulkan_is_gpu_with_its_device() {
        let report = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"vulkan","device":"Vulkan0","description":"AMD Radeon RX 7800 XT (RADV NAVI32)","kind":"vulkan","pid":4321}"#,
        );
        let outcome = gguf_outcome(&report, Some(4321), &cohere("vulkan")).unwrap();
        assert_eq!(outcome.state, AccelState::Gpu);
        assert_eq!(outcome.backend, "vulkan");
        assert_eq!(
            outcome.device.as_deref(),
            Some("AMD Radeon RX 7800 XT (RADV NAVI32) [Vulkan0]")
        );
        assert!(
            outcome.detail.contains("voxtype-gguf-worker pid 4321"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn transcribe_cpp_backend_names_map_to_tags() {
        for (name, tag) in [
            ("vulkan", Some("vulkan")),
            ("Vulkan", Some("vulkan")),
            ("cuda", Some("cuda")),
            ("ROCm", Some("rocm")),
            ("metal", Some("metal")),
            ("cpu", Some("cpu")),
            ("cpu_accel", Some("cpu")),
            ("accel", Some("cpu")),
            ("auto", None),
            ("", None),
        ] {
            assert_eq!(gguf_backend_tag(name), tag, "{name}");
        }
    }

    /// A CPU run the user asked for says so and how to change it; one that
    /// `auto` ended up with says the GPU was not bound. Both are on the CPU of
    /// a GPU-capable worker, so neither is `cpu-only`.
    #[test]
    fn a_gguf_worker_on_the_cpu_says_why() {
        let report = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"cpu","device":"CPU","pid":7}"#,
        );

        let chosen = gguf_outcome(&report, Some(7), &cohere("cpu")).unwrap();
        assert_eq!(chosen.state, AccelState::CpuFallback);
        assert_eq!(chosen.backend, "cpu");
        assert!(
            chosen.detail.contains("cohere.gguf_backend = \"cpu\""),
            "{}",
            chosen.detail
        );
        assert!(chosen.detail.contains("\"vulkan\""), "{}", chosen.detail);

        let auto = gguf_outcome(&report, Some(7), &cohere("auto")).unwrap();
        assert_eq!(auto.state, AccelState::CpuFallback);
        assert!(
            auto.detail.contains("did not bind a GPU"),
            "{}",
            auto.detail
        );
    }

    /// The report file is shared by every worker on the machine. One written by
    /// another worker, or about another model, must not describe this one.
    #[test]
    fn a_gguf_report_from_another_worker_or_model_is_ignored() {
        let other_pid = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"vulkan","device":"Vulkan0","pid":99}"#,
        );
        assert_eq!(gguf_outcome(&other_pid, Some(7), &cohere("vulkan")), None);

        let other_model = worker_report(
            r#"{"model":"/m/parakeet-ultra-Q8_0.gguf","backend":"vulkan","device":"Vulkan0","pid":7}"#,
        );
        assert_eq!(gguf_outcome(&other_model, Some(7), &cohere("vulkan")), None);

        let unknown_backend = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"sycl0","pid":7}"#,
        );
        assert_eq!(
            gguf_outcome(&unknown_backend, Some(7), &cohere("auto")),
            None
        );
    }

    /// Workers from 1.3.0 write no PID, kind or description. Their report still
    /// counts when it names the model this worker was started for.
    #[test]
    fn a_gguf_report_without_a_pid_is_matched_by_model() {
        let report = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"vulkan","device":"Vulkan0"}"#,
        );
        let outcome = gguf_outcome(&report, Some(7), &cohere("vulkan")).unwrap();
        assert_eq!(outcome.state, AccelState::Gpu);
        assert_eq!(outcome.device.as_deref(), Some("Vulkan0"));
    }

    /// The device travels through the state file, and a file written before
    /// the field existed still parses.
    #[test]
    fn the_state_file_carries_the_device_and_reads_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accel.json");
        assert!(publish_at(
            &path,
            3,
            AccelState::Gpu,
            Some("vulkan"),
            Some("AMD Radeon RX 7800 XT (Vulkan0)".to_string()),
            Some("detail".to_string())
        ));
        let read = read_state_file_at(&path, 3).unwrap();
        assert_eq!(
            read.device.as_deref(),
            Some("AMD Radeon RX 7800 XT (Vulkan0)")
        );

        // Only the device changed: still news.
        assert!(publish_at(
            &path,
            3,
            AccelState::Gpu,
            Some("vulkan"),
            Some("Vulkan1".to_string()),
            Some("detail".to_string())
        ));

        std::fs::write(
            &path,
            r#"{"pid":3,"state":"gpu","backend":"vulkan","detail":null}"#,
        )
        .unwrap();
        assert_eq!(read_state_file_at(&path, 3).unwrap().device, None);
    }

    /// Before the worker has loaded anything, a GGUF engine is `unknown`, and
    /// the report names the configured GGUF backend rather than the main
    /// binary's (#1085 showed `migraphx` for a Vulkan GGUF model).
    #[test]
    fn an_unpublished_gguf_engine_is_unknown_on_its_configured_backend() {
        let pid = std::process::id() as i32;
        let path = Path::new("/run/user/1000/voxtype/gguf_backend.json");
        let active = cohere("vulkan");
        let report = unpublished_gguf_report(pid, &active, None, Vec::new(), None, path);
        assert_eq!(report.state, AccelState::Unknown);
        assert_eq!(report.backend, Some("vulkan"));
        assert!(
            report
                .evidence
                .iter()
                .any(|e| e.starts_with("engine: cohere runs a GGUF model")),
            "{:?}",
            report.evidence
        );

        let auto = cohere("auto");
        let report = unpublished_gguf_report(pid, &auto, None, Vec::new(), None, path);
        assert_eq!(report.backend, None);
    }

    /// A daemon from before GGUF publishing writes no `accel.json`, but its
    /// worker's own report still says what transcribe.cpp bound.
    #[test]
    fn a_pre_publishing_daemon_is_read_from_its_worker_report() {
        let path = Path::new("/run/user/1000/voxtype/gguf_backend.json");
        let worker = worker_report(
            r#"{"model":"/m/cohere-transcribe-03-2026-Q4_K_M.gguf","backend":"vulkan","device":"Vulkan0"}"#,
        );
        let report = unpublished_gguf_report(
            1,
            &cohere("vulkan"),
            Some(Variant::OnnxMigraphx),
            Vec::new(),
            Some(worker),
            path,
        );
        assert_eq!(report.state, AccelState::Gpu);
        assert_eq!(report.backend, Some("vulkan"));
        assert_eq!(report.device.as_deref(), Some("Vulkan0"));
        assert!(
            report.evidence.iter().any(|e| e.starts_with("worker: ")),
            "{:?}",
            report.evidence
        );
    }

    /// End to end, a GGUF engine skips the main binary's journal: whatever it
    /// says is about another runtime.
    #[test]
    fn report_for_a_gguf_engine_does_not_read_the_daemon_journal() {
        let active = ActiveGguf {
            section: "gguf",
            model: "voxtype-test-model-that-no-worker-loaded.gguf",
            backend: "vulkan",
        };
        let report = report_for(Some(std::process::id() as i32), Some(&active));
        assert_eq!(report.state, AccelState::Unknown);
        assert_eq!(report.backend, Some("vulkan"));
        assert!(
            !report
                .evidence
                .iter()
                .any(|e| e.contains("no GPU markers found")),
            "{:?}",
            report.evidence
        );
    }

    #[test]
    fn active_gguf_follows_the_engine_and_model() {
        let parse = |text: &str| -> Config { toml::from_str(text).unwrap() };
        let gguf_cohere = parse(
            "engine = \"cohere\"\n[cohere]\nmodel = \"cohere-transcribe-03-2026-Q4_K_M.gguf\"\ngguf_backend = \"vulkan\"\n",
        );
        let active = gguf_cohere.active_gguf().unwrap();
        assert_eq!(active.section, "cohere");
        assert_eq!(active.backend, "vulkan");

        let onnx_cohere =
            parse("engine = \"cohere\"\n[cohere]\nmodel = \"cohere-transcribe-03-2026\"\n");
        assert_eq!(onnx_cohere.active_gguf(), None);

        let whisper = parse("engine = \"whisper\"\n");
        assert_eq!(whisper.active_gguf(), None);
    }
}
