//! Acoustic echo cancellation for meetings played on speakers.
//!
//! A meeting's microphone hears whatever the speakers play: the other side
//! of the call, or a video someone plays during it. Transcribed, that lands
//! under You. GTCRN (`audio::enhance`) is a denoiser and does not remove a
//! second voice, so when the output is speakers, a meeting runs PipeWire's
//! echo canceller with the WebRTC engine and records the mic from its
//! cancelled source instead.
//!
//! The canceller runs as a child `pipewire -c <conf>` process loading
//! `libpipewire-module-echo-cancel` with `monitor.mode = true`, so its
//! reference is the default sink's monitor: nobody's audio is rerouted, and
//! nothing in the user's PipeWire configuration is touched. The child is
//! wrapped in a shell that reads its stdin from the daemon; when the daemon
//! exits for any reason, including SIGKILL, stdin closes and the shell
//! stops the canceller. A canceller left over anyway (its shell killed on
//! its own) is found by its config path and stopped at the next start.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// The echo-cancelled source the meeting mic is recorded from.
pub const SOURCE_NODE: &str = "voxtype_meeting_aec_source";
/// The canceller's capture stream from the real microphone.
pub const CAPTURE_NODE: &str = "voxtype_meeting_aec_capture";
/// A sink of ours only appears on a PipeWire that ignores `monitor.mode`.
pub const SINK_NODE: &str = "voxtype_meeting_aec_sink";
/// The canceller's config, in the runtime directory.
pub const CONF_FILE: &str = "meeting-aec.conf";

/// How long the canceller has to come up before a meeting falls back.
const READY_TIMEOUT: Duration = Duration::from_secs(3);

/// How much later the cancelled mic arrives than the loopback, which comes
/// straight from the sink's monitor. Measured at 31 to 48 ms (37 ms
/// typical) with this config on PipeWire 1.6; the meeting drops this much
/// from the start of the mic so the two tracks share a timeline.
pub const MIC_DELAY_MS: u32 = 37;

/// The cancelled mic's level below which it carries nothing: about -100
/// dBFS. A working canceller on a real mic delivers the room's noise floor
/// after its noise suppression, well above this; one whose capture is not
/// getting the mic delivers digital silence.
pub const SILENT_RMS: f32 = 1e-5;
/// How much of the cancelled mic is checked before trusting it.
pub const LIVENESS_SECS: u32 = 3;

/// What the start of the cancelled mic says about the canceller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Not enough audio yet
    Pending,
    /// It carries sound
    Alive,
    /// [`LIVENESS_SECS`] of digital silence: the canceller is not hearing
    /// the mic, and the meeting would record nothing of you
    Silent,
}

/// Watches the first [`LIVENESS_SECS`] of the cancelled mic.
#[derive(Debug, Clone)]
pub struct LivenessCheck {
    window: usize,
    seen: usize,
    energy: f64,
}

impl LivenessCheck {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            window: (sample_rate * LIVENESS_SECS) as usize,
            seen: 0,
            energy: 0.0,
        }
    }

    /// Add the next samples of the cancelled mic. Alive as soon as the
    /// window could no longer average below [`SILENT_RMS`]; Silent only once
    /// the whole window has come in under it.
    pub fn feed(&mut self, samples: &[f32]) -> Liveness {
        let take = samples.len().min(self.window - self.seen.min(self.window));
        self.energy += samples[..take]
            .iter()
            .map(|&s| f64::from(s) * f64::from(s))
            .sum::<f64>();
        self.seen += take;
        let floor = f64::from(SILENT_RMS).powi(2) * self.window as f64;
        if self.energy >= floor {
            Liveness::Alive
        } else if self.seen >= self.window {
            Liveness::Silent
        } else {
            Liveness::Pending
        }
    }

    /// The RMS of what has come in so far.
    pub fn rms(&self) -> f32 {
        (self.energy / self.seen.max(1) as f64).sqrt() as f32
    }
}

/// `[meeting.audio] echo_cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The WebRTC canceller unless the output is headphones, GTCRN otherwise
    Auto,
    /// The WebRTC canceller whatever the output
    Webrtc,
    /// GTCRN only, as before 1.3.0
    Gtcrn,
    /// Neither
    Disabled,
}

impl Mode {
    /// Parse the config value. `aec` and `off` are accepted as `webrtc`
    /// and `disabled`; an unknown value is `auto`, with a warning.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Mode::Auto,
            "webrtc" | "aec" => Mode::Webrtc,
            "gtcrn" => Mode::Gtcrn,
            "disabled" | "off" | "none" => Mode::Disabled,
            other => {
                tracing::warn!(
                    "Unknown [meeting.audio] echo_cancel '{}'; using auto (auto, webrtc, gtcrn or disabled)",
                    other
                );
                Mode::Auto
            }
        }
    }
}

/// What the default output is, as far as echo goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Sound in the room: the mic hears it
    Speakers,
    /// Sound in the ears: nothing to cancel
    Headphones,
    /// Could not tell (no pactl, no default sink)
    Unknown,
}

/// The echo handling a meeting used, as `meeting.json` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoPath {
    /// PipeWire's WebRTC echo canceller (`webrtc`)
    Webrtc,
    /// The GTCRN enhancer on the mic track (`gtcrn`)
    Gtcrn,
    /// Nothing (`none`)
    None,
}

impl EchoPath {
    pub fn as_str(self) -> &'static str {
        match self {
            EchoPath::Webrtc => "webrtc",
            EchoPath::Gtcrn => "gtcrn",
            EchoPath::None => "none",
        }
    }

    /// What a meeting on this path really runs, and so reports: GTCRN
    /// only when it can be used, nothing otherwise.
    pub fn effective(self, gtcrn_usable: bool) -> Self {
        match self {
            EchoPath::Gtcrn if !gtcrn_usable => EchoPath::None,
            path => path,
        }
    }
}

/// The GTCRN model's file name in the models directory.
pub const GTCRN_MODEL_FILE: &str = "gtcrn_simple.onnx";

/// Whether a meeting started now could run GTCRN: this build has it and
/// its model is installed.
pub fn gtcrn_available() -> bool {
    cfg!(feature = "onnx-common")
        && crate::config::Config::models_dir()
            .join(GTCRN_MODEL_FILE)
            .exists()
}

/// The meeting mic, as the canceller would capture it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mic {
    /// `mic_device` is the default: the canceller follows the default source
    Default,
    /// `mic_device` names a PipeWire source the canceller can capture
    Node(String),
    /// `mic_device` names something no PipeWire source is called (an ALSA
    /// device name, say), so the canceller cannot record it
    Unmapped(String),
}

/// What the decision depends on, read from the sound server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub output: Output,
    pub mic: Mic,
    /// The default source is already an echo canceller the user runs
    pub system_canceller: bool,
    /// The default source, when it is a filter over the microphone `mic`
    /// names (Camera Effects, a noise-suppression chain): the canceller
    /// records the microphone under it
    pub wraps: Option<String>,
    /// pipewire, pactl and parec are installed
    pub available: bool,
    /// The sink whose sound the canceller removes, when it is not the
    /// default sink: the one `loopback_device` records the call from
    pub reference: Option<String>,
    /// The mic is that sink's own monitor (directly or through filters), so
    /// it hears nothing but what the canceller would remove
    pub mic_hears_output: bool,
}

/// Classify one sink from `pactl -f json list sinks`.
pub fn classify_sink(sink: &serde_json::Value) -> Output {
    let props = &sink["properties"];
    let prop = |key: &str| props[key].as_str().unwrap_or("").to_ascii_lowercase();
    let form_factor = prop("device.form_factor");
    let active = sink["active_port"].as_str().unwrap_or("");
    let port_type = sink["ports"]
        .as_array()
        .and_then(|ports| ports.iter().find(|p| p["name"].as_str() == Some(active)))
        .and_then(|p| p["type"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if matches!(port_type.as_str(), "headphones" | "headset" | "handset")
        || matches!(
            form_factor.as_str(),
            "headphone" | "headset" | "hands-free" | "handset"
        )
    {
        return Output::Headphones;
    }
    // A Bluetooth sink is headphones in A2DP and in HFP, unless the device
    // says it is a speaker.
    if prop("device.bus") == "bluetooth" && form_factor != "speaker" {
        return Output::Headphones;
    }
    Output::Speakers
}

/// The default sink, classified, from `pactl -f json list sinks` and
/// `pactl get-default-sink` output.
pub fn classify_default(sinks_json: &str, default_sink: &str) -> Output {
    let Ok(sinks) = serde_json::from_str::<serde_json::Value>(sinks_json) else {
        return Output::Unknown;
    };
    sinks
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|s| s["name"].as_str() == Some(default_sink.trim()))
        })
        .map(classify_sink)
        .unwrap_or(Output::Unknown)
}

/// What a meeting should try first, and why when it is not the canceller.
pub fn plan(mode: Mode, env: &Environment) -> (EchoPath, &'static str) {
    match mode {
        Mode::Disabled => (EchoPath::None, "echo_cancel = \"disabled\""),
        Mode::Gtcrn => (EchoPath::Gtcrn, "echo_cancel = \"gtcrn\""),
        _ if !env.available => (EchoPath::Gtcrn, "pipewire, pactl or parec is not installed"),
        _ if matches!(env.mic, Mic::Unmapped(_)) => (
            EchoPath::Gtcrn,
            "meeting.audio.mic_device is not a PipeWire source the canceller can record",
        ),
        _ if env.mic_hears_output => (
            EchoPath::Gtcrn,
            "the mic is a monitor of the output the canceller would cancel, so it would remove \
             everything the mic hears",
        ),
        _ if env.system_canceller && env.mic == Mic::Default => (
            EchoPath::Gtcrn,
            "the default source is already an echo canceller",
        ),
        Mode::Auto if env.output == Output::Headphones => {
            (EchoPath::Gtcrn, "the output is headphones")
        }
        Mode::Auto | Mode::Webrtc => (EchoPath::Webrtc, ""),
    }
}

/// Whether `source` (one entry of `pactl -f json list sources`) is an echo
/// canceller: PipeWire's echo-cancel module groups its nodes as
/// `echo-cancel-…`, however they are named.
pub fn is_echo_canceller(source: &serde_json::Value) -> bool {
    source["name"].as_str() != Some(SOURCE_NODE)
        && source["properties"]["node.group"]
            .as_str()
            .is_some_and(|g| g.starts_with("echo-cancel"))
}

/// The microphone under a filter source: a source that shares its
/// `node.link-group` with a capture stream (a source output) recording
/// another source, as Camera Effects' `camera-effects-mic` does with the
/// webcam mic. Follows up to four filters, stopping at a source whose input
/// is a monitor (a virtual mic fed from a sink) or voxtype's own canceller.
/// `None` when `name` is not a filter, or is one straight over a monitor.
pub fn filter_upstream(
    name: &str,
    sources: &[serde_json::Value],
    outputs: &[serde_json::Value],
) -> Option<String> {
    let mut current = name.to_string();
    for _ in 0..4 {
        let source = sources
            .iter()
            .find(|s| s["name"].as_str() == Some(current.as_str()))?;
        let Some(group) = source["properties"]["node.link-group"].as_str() else {
            break;
        };
        let Some(upstream_index) = outputs
            .iter()
            .find(|o| o["properties"]["node.link-group"].as_str() == Some(group))
            .and_then(|o| o["source"].as_u64())
        else {
            break;
        };
        let upstream = sources
            .iter()
            .find(|s| s["index"].as_u64() == Some(upstream_index))?["name"]
            .as_str()?;
        if upstream.ends_with(".monitor") || upstream == SOURCE_NODE {
            break;
        }
        current = upstream.to_string();
    }
    (current != name).then_some(current)
}

/// Read the environment from `pactl` output: the default sink and source,
/// and the sink, source and source-output lists as JSON. `mic_device` is
/// `[meeting.audio] mic_device`, `loopback_device` its `loopback_device`.
// Five pactl outputs and two settings, kept flat so tests read as the
// pactl session they model.
#[allow(clippy::too_many_arguments)]
pub fn environment_from(
    default_sink: &str,
    default_source: &str,
    sinks_json: &str,
    sources_json: &str,
    source_outputs_json: &str,
    mic_device: &str,
    loopback_device: &str,
    available: bool,
) -> Environment {
    let sources: Vec<serde_json::Value> = serde_json::from_str(sources_json).unwrap_or_default();
    let outputs: Vec<serde_json::Value> =
        serde_json::from_str(source_outputs_json).unwrap_or_default();
    let default_source = default_source.trim();
    let system_canceller = sources
        .iter()
        .find(|s| s["name"].as_str() == Some(default_source))
        .is_some_and(is_echo_canceller);
    let mic_device = mic_device.trim();
    let mut wraps = None;
    let mic = if matches!(mic_device, "default" | "" | "pipewire" | "pulse") {
        // A filter as the default source sits between the real mic and the
        // canceller, and its effects (a noise gate, a voice changer, auto
        // level) bend the echo the canceller models as linear, so record
        // the mic under it. A system echo canceller is left alone: the plan
        // skips ours for it.
        match filter_upstream(default_source, &sources, &outputs) {
            Some(real) if !system_canceller => {
                wraps = Some(default_source.to_string());
                Mic::Node(real)
            }
            _ => Mic::Default,
        }
    } else if sources
        .iter()
        .any(|s| s["name"].as_str() == Some(mic_device))
    {
        Mic::Node(mic_device.to_string())
    } else {
        Mic::Unmapped(mic_device.to_string())
    };
    let reference = reference_sink(loopback_device, sinks_json);
    let cancelled = reference.as_deref().unwrap_or(default_sink.trim());
    let mic_name = match mic_device {
        "default" | "" | "pipewire" | "pulse" => default_source,
        named => named,
    };
    let mic_hears_output = !cancelled.is_empty()
        && monitor_beneath(mic_name, &sources, &outputs).as_deref() == Some(cancelled);
    Environment {
        output: classify_default(sinks_json, cancelled),
        mic,
        system_canceller,
        wraps,
        available,
        reference,
        mic_hears_output,
    }
}

/// The sink a `loopback_device` records, when it names a sink's monitor
/// outright (`auto` follows the default sink, like the canceller).
pub fn reference_sink(loopback_device: &str, sinks_json: &str) -> Option<String> {
    let sink = loopback_device.trim().strip_suffix(".monitor")?;
    let sinks: Vec<serde_json::Value> = serde_json::from_str(sinks_json).unwrap_or_default();
    sinks
        .iter()
        .any(|s| s["name"].as_str() == Some(sink))
        .then(|| sink.to_string())
}

/// The sink whose monitor `name` is, or sits on through filters: a virtual
/// mic fed from an output rather than a microphone.
pub fn monitor_beneath(
    name: &str,
    sources: &[serde_json::Value],
    outputs: &[serde_json::Value],
) -> Option<String> {
    let mut current = sources.iter().find(|s| s["name"].as_str() == Some(name))?;
    for _ in 0..5 {
        let monitored = current["monitor_source"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| current["name"].as_str()?.strip_suffix(".monitor"));
        if let Some(sink) = monitored {
            return Some(sink.to_string());
        }
        let group = current["properties"]["node.link-group"].as_str()?;
        let upstream = outputs
            .iter()
            .find(|o| o["properties"]["node.link-group"].as_str() == Some(group))?["source"]
            .as_u64()?;
        current = sources
            .iter()
            .find(|s| s["index"].as_u64() == Some(upstream))?;
    }
    None
}

/// Decide a meeting's echo handling and, for the canceller, start it. Never
/// fails: a canceller that cannot start is logged once and the meeting
/// uses GTCRN instead. `mic_device` is `[meeting.audio] mic_device`.
pub fn begin(
    mode: Mode,
    mic_device: &str,
    loopback_device: &str,
    runtime_dir: &Path,
) -> (EchoPath, Option<EchoCanceller>) {
    let tools = Tools::detect();
    let pactl = |args: &[&str]| -> String {
        Command::new("pactl")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let env = if matches!(mode, Mode::Disabled | Mode::Gtcrn) {
        Environment {
            output: Output::Unknown,
            mic: Mic::Default,
            system_canceller: false,
            wraps: None,
            available: tools.is_some(),
            reference: None,
            mic_hears_output: false,
        }
    } else {
        environment_from(
            &pactl(&["get-default-sink"]),
            &pactl(&["get-default-source"]),
            &pactl(&["-f", "json", "list", "sinks"]),
            &pactl(&["-f", "json", "list", "sources"]),
            &pactl(&["-f", "json", "list", "source-outputs"]),
            mic_device,
            loopback_device,
            tools.is_some(),
        )
    };
    let (path, why) = plan(mode, &env);
    match (path, tools) {
        (EchoPath::Webrtc, Some(tools)) => {
            if let (Some(filter), Mic::Node(real)) = (&env.wraps, &env.mic) {
                tracing::info!(
                    "The default source {} is a filter over {}; the echo canceller records {} \
                     so the filter's effects do not bend the echo it removes",
                    filter,
                    real,
                    real
                );
            }
            let target = match env.mic {
                Mic::Node(ref node) => Some(node.as_str()),
                _ => None,
            };
            let reference = env.reference.as_deref();
            match EchoCanceller::start(&tools, runtime_dir, target, reference) {
                Ok(canceller) => {
                    tracing::info!(
                        "Meeting echo handling: webrtc (PipeWire's echo canceller; output {:?}, mic {}, cancelling {})",
                        env.output,
                        target.unwrap_or("the default source"),
                        reference.unwrap_or("the default sink")
                    );
                    (EchoPath::Webrtc, Some(canceller))
                }
                Err(e) => {
                    tracing::warn!(
                        "Echo cancellation could not start ({}); this meeting uses GTCRN instead",
                        e
                    );
                    (EchoPath::Gtcrn, None)
                }
            }
        }
        (path, _) => {
            if mode == Mode::Webrtc {
                tracing::warn!(
                    "echo_cancel = \"webrtc\" but {}; using {}",
                    why,
                    path.as_str()
                );
            }
            tracing::info!("Meeting echo handling: {} ({})", path.as_str(), why);
            (path, None)
        }
    }
}

/// The programs the canceller needs.
#[derive(Debug, Clone)]
pub struct Tools {
    pub pipewire: PathBuf,
    pub pactl: PathBuf,
    /// How long the canceller's source may take to appear.
    pub ready_timeout: Duration,
}

impl Tools {
    /// `pipewire`, `pactl` and `parec` (which records the mic from the
    /// cancelled source) on PATH, or `None`.
    pub fn detect() -> Option<Self> {
        which::which("parec").ok()?;
        Some(Self {
            pipewire: which::which("pipewire").ok()?,
            pactl: which::which("pactl").ok()?,
            ready_timeout: READY_TIMEOUT,
        })
    }
}

/// The canceller's PipeWire config: a client-only context that loads the
/// echo-cancel module in monitor mode, capturing from `target` (or the
/// default source), removing what `reference` plays (or the default sink),
/// and offering the cancelled audio as [`SOURCE_NODE`].
pub fn config(target: Option<&str>, reference: Option<&str>) -> String {
    let object = |node: Option<&str>| {
        node.map(|t| format!(" target.object = \"{}\"", t.replace('"', "")))
            .unwrap_or_default()
    };
    let (target, reference) = (object(target), object(reference));
    format!(
        r#"# Written by voxtype for one meeting; removed when the meeting stops.
context.properties = {{ log.level = 2 }}
context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}
context.modules = [
    {{ name = libpipewire-module-rt flags = [ ifexists nofail ] }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-echo-cancel
        args = {{
            library.name = aec/libspa-aec-webrtc
            monitor.mode = true
            audio.channels = 1
            audio.position = [ MONO ]
            capture.props = {{ node.name = {CAPTURE_NODE} node.passive = true{target} }}
            source.props = {{
                node.name = {SOURCE_NODE}
                node.description = "Voxtype meeting echo canceller"
                priority.session = 1
            }}
            sink.props = {{ node.name = {SINK_NODE}{reference} }}
        }}
    }}
]
"#
    )
}

/// A running echo canceller. Stopped when dropped.
pub struct EchoCanceller {
    child: Child,
    stdin: Option<ChildStdin>,
    conf: PathBuf,
}

impl EchoCanceller {
    /// Start the canceller and wait until its source is up. `target` is the
    /// mic to capture (`None` follows the default source), `reference` the
    /// sink whose sound it removes (`None` follows the default sink).
    pub fn start(
        tools: &Tools,
        runtime_dir: &Path,
        target: Option<&str>,
        reference: Option<&str>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(runtime_dir).map_err(|e| e.to_string())?;
        let conf = runtime_dir.join(CONF_FILE);
        let stale = stop_stale(&conf);
        if stale > 0 {
            tracing::info!(
                "Stopped {} echo canceller(s) left by an earlier daemon",
                stale
            );
        }
        std::fs::write(&conf, config(target, reference)).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(runtime_dir.join("meeting-aec.log"))
            .map_err(|e| e.to_string())?;

        // The shell runs the canceller and stops it when its stdin, held
        // by the daemon, closes.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(r#""$1" -c "$2" & p=$!; read _ignored; kill $p 2>/dev/null; wait $p"#)
            .arg("voxtype-meeting-aec")
            .arg(&tools.pipewire)
            .arg(&conf)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log);
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot run pipewire: {e}"))?;
        let stdin = child.stdin.take();
        let mut canceller = Self { child, stdin, conf };

        let deadline = Instant::now() + tools.ready_timeout;
        loop {
            if names(&tools.pactl, "sources")
                .iter()
                .any(|n| n == SOURCE_NODE)
            {
                return Ok(canceller);
            }
            if names(&tools.pactl, "sinks").iter().any(|n| n == SINK_NODE) {
                canceller.stop();
                return Err(
                    "this PipeWire does not support the echo canceller's monitor mode".to_string(),
                );
            }
            if let Ok(Some(_)) = canceller.child.try_wait() {
                let log = std::fs::read_to_string(runtime_dir.join("meeting-aec.log"))
                    .unwrap_or_default();
                canceller.stop();
                return Err(format!(
                    "the echo canceller exited: {}",
                    log.lines().last().unwrap_or("no output")
                ));
            }
            if Instant::now() >= deadline {
                canceller.stop();
                return Err(format!(
                    "the echo canceller's source did not appear within {}s",
                    tools.ready_timeout.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The source to record the meeting mic from.
    pub fn source(&self) -> &str {
        SOURCE_NODE
    }

    /// Stop the canceller and remove its config.
    pub fn stop(&mut self) {
        if self.stdin.take().is_some() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(self.child.id() as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
                let _ = self.child.wait();
            }
            let _ = std::fs::remove_file(&self.conf);
            tracing::info!("Meeting echo canceller stopped");
        }
    }

    /// Drop the daemon's end of the shell's stdin without stopping or
    /// waiting, as a crash does, and hand back the shell's PID.
    #[cfg(test)]
    fn abandon(mut self) -> u32 {
        let pid = self.child.id();
        drop(self.stdin.take());
        std::mem::forget(self);
        pid
    }
}

impl Drop for EchoCanceller {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Node names pactl lists for `kind` (`sources` or `sinks`).
fn names(pactl: &Path, kind: &str) -> Vec<String> {
    Command::new(pactl)
        .args(["list", "short", kind])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.split('\t').nth(1).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Stop cancellers still running from `conf`: processes whose command line
/// is `… -c <conf>`. Returns how many were signalled.
fn stop_stale(conf: &Path) -> usize {
    let conf = conf.as_os_str().as_encoded_bytes();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    let mut stopped = 0;
    for pid in entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|&pid| pid != std::process::id() as i32)
    {
        let mut cmdline = Vec::new();
        let read = std::fs::File::open(format!("/proc/{pid}/cmdline"))
            .and_then(|mut f| f.read_to_end(&mut cmdline));
        if read.is_err() {
            continue;
        }
        let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
        let runs_conf = args.windows(2).any(|w| w[0] == b"-c" && w[1] == conf);
        // The wrapper shell names the conf as its third argument, not
        // after -c; only the canceller itself matches.
        if runs_conf
            && nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            )
            .is_ok()
        {
            stopped += 1;
        }
    }
    stopped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gtcrn_without_its_model_is_reported_as_none() {
        assert_eq!(EchoPath::Gtcrn.effective(false), EchoPath::None);
        assert_eq!(EchoPath::Gtcrn.effective(true), EchoPath::Gtcrn);
        assert_eq!(EchoPath::Webrtc.effective(false), EchoPath::Webrtc);
        assert_eq!(EchoPath::None.effective(true), EchoPath::None);
    }

    fn sink(
        name: &str,
        port: &str,
        port_type: &str,
        props: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "name": name,
            "active_port": port,
            "ports": [{"name": port, "type": port_type}],
            "properties": props,
        })
    }

    #[test]
    fn modes_parse_with_old_names_accepted() {
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse("WebRTC"), Mode::Webrtc);
        assert_eq!(Mode::parse("aec"), Mode::Webrtc);
        assert_eq!(Mode::parse("gtcrn"), Mode::Gtcrn);
        assert_eq!(Mode::parse("disabled"), Mode::Disabled);
        assert_eq!(Mode::parse("off"), Mode::Disabled);
        assert_eq!(Mode::parse("loud"), Mode::Auto);
    }

    #[test]
    fn sinks_classify_as_speakers_or_headphones() {
        let usb = json!({"device.bus": "usb"});
        // Pete's USB card: one sink per port (UCM HiFi).
        assert_eq!(
            classify_sink(&sink(
                "x.HiFi__Speaker__sink",
                "[Out] Speaker",
                "Speaker",
                usb.clone()
            )),
            Output::Speakers
        );
        assert_eq!(
            classify_sink(&sink(
                "x.HiFi__Headphones__sink",
                "[Out] Headphones",
                "Headphones",
                usb
            )),
            Output::Headphones
        );
        assert_eq!(
            classify_sink(&sink(
                "hdmi",
                "hdmi-output-0",
                "HDMI",
                json!({"device.bus": "pci"})
            )),
            Output::Speakers
        );
        // A USB headset says so in its form factor, whatever its port.
        assert_eq!(
            classify_sink(&sink(
                "h",
                "analog-output",
                "Unknown",
                json!({"device.form_factor": "headset"})
            )),
            Output::Headphones
        );
        // Bluetooth: headphones in A2DP and in HFP, unless it is a speaker.
        for profile in ["a2dp-sink", "headset-head-unit"] {
            assert_eq!(
                classify_sink(&sink(
                    "bluez_output.x",
                    "headset-output",
                    "Headset",
                    json!({
                    "device.bus": "bluetooth", "api.bluez5.profile": profile,
                    "device.form_factor": "headphone"})
                )),
                Output::Headphones,
                "{profile}"
            );
        }
        assert_eq!(
            classify_sink(&sink(
                "bluez_output.y",
                "speaker-output",
                "Unknown",
                json!({
                "device.bus": "bluetooth", "device.form_factor": "speaker"})
            )),
            Output::Speakers
        );
        // A null sink or anything without ports counts as speakers.
        assert_eq!(
            classify_sink(&json!({"name": "null", "properties": {}})),
            Output::Speakers
        );
    }

    #[test]
    fn the_default_sink_is_the_one_classified() {
        let sinks = json!([
            sink("speaker", "[Out] Speaker", "Speaker", json!({})),
            sink("phones", "[Out] Headphones", "Headphones", json!({})),
        ])
        .to_string();
        assert_eq!(classify_default(&sinks, "phones\n"), Output::Headphones);
        assert_eq!(classify_default(&sinks, "speaker"), Output::Speakers);
        assert_eq!(classify_default(&sinks, "gone"), Output::Unknown);
        assert_eq!(classify_default("not json", "speaker"), Output::Unknown);
    }

    fn env(output: Output) -> Environment {
        Environment {
            output,
            mic: Mic::Default,
            system_canceller: false,
            wraps: None,
            available: true,
            reference: None,
            mic_hears_output: false,
        }
    }

    #[test]
    fn the_plan_follows_mode_output_mic_and_availability() {
        use EchoPath::*;
        use Output::*;
        let path = |mode, e: Environment| plan(mode, &e).0;
        // auto: the canceller for everything but headphones
        assert_eq!(path(Mode::Auto, env(Speakers)), Webrtc);
        assert_eq!(
            path(Mode::Auto, env(Unknown)),
            Webrtc,
            "HDMI, line out, unknown"
        );
        assert_eq!(path(Mode::Auto, env(Headphones)), Gtcrn);
        // webrtc forces it past headphones
        assert_eq!(path(Mode::Webrtc, env(Headphones)), Webrtc);
        assert_eq!(path(Mode::Gtcrn, env(Speakers)), Gtcrn);
        assert_eq!(path(Mode::Disabled, env(Speakers)), None);
        // Module tools missing: GTCRN, whatever the mode asked for.
        let missing = Environment {
            available: false,
            ..env(Speakers)
        };
        assert_eq!(path(Mode::Auto, missing.clone()), Gtcrn);
        assert_eq!(path(Mode::Webrtc, missing), Gtcrn);
        // A mic the canceller cannot record from.
        let alsa = Environment {
            mic: Mic::Unmapped("hw:1,0".into()),
            ..env(Speakers)
        };
        let (p, why) = plan(Mode::Auto, &alsa);
        assert_eq!(p, Gtcrn);
        assert!(why.contains("mic_device"), "{why}");
        let node = Environment {
            mic: Mic::Node("alsa_input.usb".into()),
            ..env(Speakers)
        };
        assert_eq!(path(Mode::Auto, node), Webrtc);
        // The user already cancels echo on the default source.
        let system = Environment {
            system_canceller: true,
            ..env(Speakers)
        };
        assert_eq!(path(Mode::Auto, system.clone()), Gtcrn);
        assert_eq!(path(Mode::Webrtc, system.clone()), Gtcrn);
        // ...but a mic named past it gets ours.
        let past = Environment {
            mic: Mic::Node("alsa_input.usb".into()),
            ..system
        };
        assert_eq!(path(Mode::Auto, past), Webrtc);
    }

    #[test]
    fn the_environment_reads_mic_and_system_canceller_from_pactl() {
        let sources = json!([
            {"name": "alsa_input.usb-mic", "properties": {"media.class": "Audio/Source"}},
            {"name": "echo-cancel-source",
             "properties": {"node.group": "echo-cancel-13-14", "media.class": "Audio/Source"}},
            {"name": SOURCE_NODE, "properties": {"node.group": "echo-cancel-40-41"}},
        ])
        .to_string();
        let sinks = json!([sink("spk", "[Out] Speaker", "Speaker", json!({}))]).to_string();
        let e = environment_from(
            "spk\n",
            "alsa_input.usb-mic\n",
            &sinks,
            &sources,
            "[]",
            "default",
            "auto",
            true,
        );
        assert_eq!(
            (e.output, e.mic.clone(), e.system_canceller),
            (Output::Speakers, Mic::Default, false)
        );
        let e = environment_from(
            "spk",
            "echo-cancel-source",
            &sinks,
            &sources,
            "[]",
            "pipewire",
            "auto",
            true,
        );
        assert!(e.system_canceller);
        // Our own canceller left as the default is not the user's.
        let e = environment_from("spk", SOURCE_NODE, &sinks, &sources, "[]", "", "auto", true);
        assert!(!e.system_canceller);
        let e = environment_from(
            "spk",
            "x",
            &sinks,
            &sources,
            "[]",
            "alsa_input.usb-mic",
            "auto",
            true,
        );
        assert_eq!(e.mic, Mic::Node("alsa_input.usb-mic".into()));
        let e = environment_from(
            "spk",
            "x",
            &sinks,
            &sources,
            "[]",
            "hw:CARD=Mic,DEV=0",
            "auto",
            true,
        );
        assert_eq!(e.mic, Mic::Unmapped("hw:CARD=Mic,DEV=0".into()));
        // No pactl output at all: unknown output, default mic.
        let e = environment_from("", "", "", "", "", "default", "auto", true);
        assert_eq!(
            (e.output, e.mic, e.system_canceller),
            (Output::Unknown, Mic::Default, false)
        );
    }

    /// The call can play on a sink other than the default, with
    /// `loopback_device` naming its monitor; and a virtual mic can be fed
    /// from the very output the canceller removes.
    #[test]
    fn the_reference_follows_the_loopback_and_a_monitor_mic_is_left_alone() {
        let sinks = json!([
            sink("spk", "[Out] Speaker", "Speaker", json!({})),
            sink("call", "[Out] Speaker", "Speaker", json!({})),
            sink("hp", "[Out] Headphones", "Headphones", json!({})),
        ])
        .to_string();
        // pactl names a monitor "<sink>.monitor" and gives its sink as
        // monitor_source; a filter over one shares a link group with the
        // capture stream that reads it.
        let sources = json!([
            {"index": 1, "name": "mic", "monitor_source": "", "properties": {}},
            {"index": 2, "name": "spk.monitor", "monitor_source": "spk", "properties": {}},
            {"index": 3, "name": "call.monitor", "monitor_source": "call", "properties": {}},
            {"index": 4, "name": "monmic", "monitor_source": "",
             "properties": {"node.link-group": "remap-4"}},
            {"index": 5, "name": "odd_name", "monitor_source": "spk", "properties": {}},
        ])
        .to_string();
        let outputs = json!([
            {"source": 2, "properties": {"node.link-group": "remap-4"}},
        ])
        .to_string();
        let env = |sink: &str, source: &str, mic: &str, loopback: &str| {
            environment_from(
                sink, source, &sinks, &sources, &outputs, mic, loopback, true,
            )
        };

        assert_eq!(
            reference_sink("call.monitor", &sinks).as_deref(),
            Some("call")
        );
        for loopback in ["auto", "disabled", "", "gone.monitor", "mic"] {
            assert_eq!(reference_sink(loopback, &sinks), None, "{loopback}");
        }

        // The call's sink, not the default, decides speakers or headphones.
        let e = env("hp", "mic", "default", "call.monitor");
        assert_eq!(
            (e.reference.as_deref(), e.output),
            (Some("call"), Output::Speakers)
        );
        assert_eq!(plan(Mode::Auto, &e).0, EchoPath::Webrtc);
        let e = env("hp", "mic", "default", "auto");
        assert_eq!((e.reference, e.output), (None, Output::Headphones));
        assert!(!e.mic_hears_output);

        // A mic that is the cancelled output's monitor, outright, through a
        // filter, or named in mic_device: nothing would be left of it.
        for (source, mic, loopback) in [
            ("spk.monitor", "default", "auto"),
            ("monmic", "default", "auto"),
            ("odd_name", "default", "auto"),
            ("mic", "call.monitor", "call.monitor"),
        ] {
            let e = env("spk\n", source, mic, loopback);
            assert!(e.mic_hears_output, "{source} {mic} {loopback}");
            let (path, why) = plan(Mode::Webrtc, &e);
            assert_eq!(path, EchoPath::Gtcrn);
            assert!(why.contains("monitor of the output"), "{why}");
        }
        // A monitor of some other output is a mic like any other.
        let e = env("spk", "spk.monitor", "default", "call.monitor");
        assert!(!e.mic_hears_output);
        assert_eq!(plan(Mode::Auto, &e).0, EchoPath::Webrtc);
    }

    /// Pete's machine on 2026-11-11, from `pactl -f json list sources` and
    /// `source-outputs`: Camera Effects wraps the C920 webcam mic as
    /// `camera-effects-mic`, the default source.
    fn camera_effects() -> (String, String) {
        let sources = json!([
            {"index": 69, "name": "alsa_input.usb-046d_HD_Pro_Webcam_C920_C1FB42FF-02.analog-stereo",
             "properties": {"media.class": "Audio/Source", "device.api": "alsa"}},
            {"index": 68, "name": "alsa_input.usb-Generic_USB_Audio-00.HiFi__Mic__source",
             "properties": {"media.class": "Audio/Source", "device.api": "alsa"}},
            {"index": 65, "name": "alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink.monitor",
             "properties": {}},
            {"index": 37704, "name": "camera-effects-mic",
             "properties": {"media.class": "Audio/Source", "device.api": "camera-effects-server",
                            "node.link-group": "camera-effects-mic"}},
        ])
        .to_string();
        let outputs = json!([
            {"index": 463, "source": 37704, "properties": {"node.name": "alsa_capture.hex"}},
            {"index": 37725, "source": 69,
             "properties": {"node.name": "camera-effects-mic-input",
                            "node.link-group": "camera-effects-mic"}},
        ])
        .to_string();
        (sources, outputs)
    }

    #[test]
    fn a_filter_default_source_is_unwrapped_to_its_mic() {
        let (sources, outputs) = camera_effects();
        let sinks = json!([sink("spk", "[Out] Speaker", "Speaker", json!({}))]).to_string();
        let webcam = "alsa_input.usb-046d_HD_Pro_Webcam_C920_C1FB42FF-02.analog-stereo";
        let env = |default: &str, mic: &str| {
            environment_from(
                "spk", default, &sinks, &sources, &outputs, mic, "auto", true,
            )
        };
        let e = env("camera-effects-mic\n", "default");
        assert_eq!(e.mic, Mic::Node(webcam.to_string()));
        assert_eq!(e.wraps.as_deref(), Some("camera-effects-mic"));
        assert!(
            !e.system_canceller,
            "Camera Effects is not an echo canceller"
        );
        assert_eq!(plan(Mode::Auto, &e).0, EchoPath::Webrtc);
        // With Camera Effects hiding the real mics, other clients (pactl
        // included) cannot see the webcam, so there is nothing to unwrap to:
        // the canceller records camera-effects-mic as the default source.
        let hidden: Vec<serde_json::Value> =
            serde_json::from_str::<Vec<serde_json::Value>>(&sources)
                .unwrap()
                .into_iter()
                .filter(|s| s["index"] != 69)
                .collect();
        let hidden = serde_json::to_string(&hidden).unwrap();
        let e = environment_from(
            "spk",
            "camera-effects-mic",
            &sinks,
            &hidden,
            &outputs,
            "default",
            "auto",
            true,
        );
        assert_eq!((e.mic.clone(), e.wraps.clone()), (Mic::Default, None));
        assert_eq!(plan(Mode::Auto, &e).0, EchoPath::Webrtc);
        // A plain mic as the default is not a filter.
        let e = env(webcam, "default");
        assert_eq!((e.mic, e.wraps), (Mic::Default, None));
        // A mic_device named outright is taken as given.
        let e = env("camera-effects-mic", "camera-effects-mic");
        assert_eq!(e.mic, Mic::Node("camera-effects-mic".to_string()));
        assert_eq!(e.wraps, None);
    }

    #[test]
    fn filters_over_monitors_or_echo_cancellers_are_not_unwrapped() {
        let s: Vec<serde_json::Value> = serde_json::from_value(json!([
            {"index": 1, "name": "mic", "properties": {}},
            {"index": 2, "name": "speakers.monitor", "properties": {}},
            {"index": 3, "name": "fx", "properties": {"node.link-group": "fx"}},
            {"index": 4, "name": "fx2", "properties": {"node.link-group": "fx2"}},
            {"index": 5, "name": "loud", "properties": {"node.link-group": "loud"}},
            {"index": 7, "name": "virtual", "properties": {"node.link-group": "virtual"}},
            {"index": 8, "name": "fx_virtual", "properties": {"node.link-group": "fxv"}},
            {"index": 6, "name": "echo-cancel-source",
             "properties": {"node.link-group": "echo-cancel-6-7", "node.group": "echo-cancel-6-7"}},
        ]))
        .unwrap();
        let o: Vec<serde_json::Value> = serde_json::from_value(json!([
            {"source": 1, "properties": {"node.link-group": "fx"}},
            {"source": 3, "properties": {"node.link-group": "fx2"}},
            {"source": 2, "properties": {"node.link-group": "loud"}},
            {"source": 2, "properties": {"node.link-group": "virtual"}},
            {"source": 7, "properties": {"node.link-group": "fxv"}},
            {"source": 1, "properties": {"node.link-group": "echo-cancel-6-7"}},
        ]))
        .unwrap();
        assert_eq!(filter_upstream("fx", &s, &o).as_deref(), Some("mic"));
        assert_eq!(
            filter_upstream("fx2", &s, &o).as_deref(),
            Some("mic"),
            "two deep"
        );
        assert_eq!(
            filter_upstream("loud", &s, &o),
            None,
            "straight over a monitor"
        );
        assert_eq!(
            filter_upstream("fx_virtual", &s, &o).as_deref(),
            Some("virtual"),
            "stops at the virtual mic a monitor feeds"
        );
        assert_eq!(filter_upstream("mic", &s, &o), None, "not a filter");
        // A user's echo canceller as default source keeps the skip rule.
        let sources = serde_json::to_string(&s).unwrap();
        let outputs = serde_json::to_string(&o).unwrap();
        let e = environment_from(
            "x",
            "echo-cancel-source",
            "[]",
            &sources,
            &outputs,
            "",
            "auto",
            true,
        );
        assert!(e.system_canceller);
        assert_eq!((e.mic.clone(), e.wraps.clone()), (Mic::Default, None));
        assert_eq!(plan(Mode::Auto, &e).0, EchoPath::Gtcrn);
    }

    #[test]
    fn the_config_runs_the_webrtc_canceller_in_monitor_mode() {
        let c = config(None, None);
        assert!(c.contains("libpipewire-module-echo-cancel"));
        assert!(c.contains("library.name = aec/libspa-aec-webrtc"));
        assert!(c.contains("monitor.mode = true"));
        assert!(c.contains(&format!("node.name = {SOURCE_NODE}")));
        assert!(c.contains("node.description = \"Voxtype meeting echo canceller\""));
        assert!(
            !c.contains("target.object"),
            "follows the default source and sink"
        );
        let c = config(Some("alsa_input.usb-mic"), Some("call_sink"));
        assert!(c.contains(&format!(
            "node.name = {CAPTURE_NODE} node.passive = true target.object = \"alsa_input.usb-mic\""
        )));
        assert!(c.contains(&format!(
            "sink.props = {{ node.name = {SINK_NODE} target.object = \"call_sink\" }}"
        )));
    }

    /// Fake `pipewire` and `pactl`: the fake canceller records its PID and
    /// waits; pactl lists `node` as a source or sink once it is running.
    struct Fakes {
        dir: tempfile::TempDir,
    }

    impl Fakes {
        fn new(kind: &str, node: &str) -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            let pid = dir.path().join("canceller.pid");
            crate::test_support::write_script(
                &dir.path().join("pipewire"),
                // Stays a shell (no exec) so its command line keeps "-c <conf>".
                // The pid file is written once the TERM trap is in place, so
                // a test that waits for it knows the fake is fully up.
                &format!(
                    "#!/bin/sh\ntrap 'kill $! 2>/dev/null; exit 0' TERM\necho $$ > {}\nsleep 600 &\nwait\n",
                    pid.display()
                ),
            );
            crate::test_support::write_script(
                &dir.path().join("pactl"),
                &format!(
                    "#!/bin/sh\n[ -s {pid} ] && kill -0 $(cat {pid}) 2>/dev/null && [ \"$3\" = {kind} ] \
                     && printf '7\\t{node}\\tPipeWire\\tfloat32le 1ch 48000Hz\\tRUNNING\\n'\nexit 0\n",
                    pid = pid.display()
                ),
            );
            Self { dir }
        }

        fn tools(&self) -> Tools {
            Tools {
                pipewire: self.dir.path().join("pipewire"),
                pactl: self.dir.path().join("pactl"),
                // Room for a fake to start on a machine running the whole
                // suite in parallel.
                ready_timeout: Duration::from_secs(30),
            }
        }

        fn canceller_pid(&self) -> i32 {
            let path = self.dir.path().join("canceller.pid");
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(pid) = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| s.trim().parse().ok())
                {
                    return pid;
                }
                assert!(Instant::now() < deadline, "the fake canceller never ran");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn running(pid: i32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, rest)| rest.trim_start().starts_with('Z'))
            })
            .is_some_and(|zombie| !zombie)
    }

    fn wait_gone(pid: i32) {
        // Generous: it only costs time when the test is failing anyway, and a
        // machine running the whole suite in parallel can take seconds to
        // schedule a process's exit.
        let deadline = Instant::now() + Duration::from_secs(30);
        while running(pid) {
            assert!(Instant::now() < deadline, "canceller {pid} still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_canceller_starts_and_stops_with_its_config() {
        let fakes = Fakes::new("sources", SOURCE_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let mut aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None, None).unwrap();
        assert_eq!(aec.source(), SOURCE_NODE);
        let pid = fakes.canceller_pid();
        assert!(runtime.path().join(CONF_FILE).is_file());
        aec.stop();
        wait_gone(pid);
        assert!(!runtime.path().join(CONF_FILE).exists());
    }

    /// The daemon dies without stopping it: the shell sees its stdin close
    /// and stops the canceller.
    #[test]
    fn a_vanished_daemon_takes_the_canceller_with_it() {
        let fakes = Fakes::new("sources", SOURCE_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None, None).unwrap();
        let pid = fakes.canceller_pid();
        let shell = aec.abandon();
        wait_gone(pid);
        wait_gone(shell as i32);
        let _ = nix::sys::wait::waitpid(
            nix::unistd::Pid::from_raw(shell as i32),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
        );
    }

    /// A canceller whose shell was killed on its own is stopped by the next
    /// start.
    #[test]
    fn a_stale_canceller_is_stopped_at_the_next_start() {
        let fakes = Fakes::new("sources", SOURCE_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let conf = runtime.path().join(CONF_FILE);
        let mut orphan = Command::new(fakes.dir.path().join("pipewire"))
            .arg("-c")
            .arg(&conf)
            .spawn()
            .unwrap();
        // Up and trapping TERM before anything looks for it; its pid file
        // is then cleared so the fake pactl waits for the new canceller.
        assert_eq!(fakes.canceller_pid(), orphan.id() as i32);
        std::fs::remove_file(fakes.dir.path().join("canceller.pid")).unwrap();

        let mut aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None, None).unwrap();
        // Reaped rather than read from /proc, and with room for a machine
        // under full test load to schedule the orphan's exit.
        let deadline = Instant::now() + Duration::from_secs(30);
        while orphan.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "the stale canceller is still running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        aec.stop();
    }

    #[test]
    fn a_canceller_delivering_digital_silence_is_caught_after_three_seconds() {
        let mut check = LivenessCheck::new(16000);
        // What a canceller that never hears the mic delivers.
        let dead = vec![1e-6f32; 1600];
        for _ in 0..29 {
            assert_eq!(check.feed(&dead), Liveness::Pending);
        }
        assert_eq!(check.feed(&dead), Liveness::Silent);
        assert!(check.rms() < SILENT_RMS);
        // Exact zeros too, and in uneven pieces.
        let mut check = LivenessCheck::new(16000);
        assert_eq!(check.feed(&vec![0.0; 47_999]), Liveness::Pending);
        assert_eq!(check.feed(&[0.0; 5]), Liveness::Silent);
    }

    #[test]
    fn a_quiet_room_or_a_late_first_word_is_alive() {
        // A quiet room after the canceller's noise suppression, which takes
        // about 18 dB off a mic's hiss: -90 dBFS here, from a mic at about
        // -72 dBFS. Webcam and laptop mics are louder than that.
        let mut check = LivenessCheck::new(16000);
        let hiss: Vec<f32> = (0..16000)
            .map(|i| if i % 2 == 0 { 3e-5 } else { -3e-5 })
            .collect();
        assert_eq!(check.feed(&hiss[..4000]), Liveness::Pending);
        assert_eq!(check.feed(&hiss), Liveness::Alive);

        // Silence, then a word in the last half second.
        let mut check = LivenessCheck::new(16000);
        assert_eq!(check.feed(&vec![0.0; 40_000]), Liveness::Pending);
        assert_eq!(check.feed(&vec![0.01; 1000]), Liveness::Alive);

        // Speech after the window does not count: the verdict is already in.
        let mut check = LivenessCheck::new(16000);
        assert_eq!(check.feed(&vec![0.0; 48_000]), Liveness::Silent);
        assert_eq!(check.feed(&vec![0.1; 16_000]), Liveness::Silent);
    }

    #[test]
    fn an_old_pipewire_or_a_silent_one_is_refused() {
        // monitor.mode ignored: the module made a sink instead.
        let fakes = Fakes::new("sinks", SINK_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let err = EchoCanceller::start(&fakes.tools(), runtime.path(), None, None)
            .err()
            .unwrap();
        assert!(err.contains("monitor mode"), "{err}");
        wait_gone(fakes.canceller_pid());

        // Nothing appears at all.
        let fakes = Fakes::new("sources", "something_else");
        let tools = Tools {
            ready_timeout: READY_TIMEOUT,
            ..fakes.tools()
        };
        let started = Instant::now();
        let err = EchoCanceller::start(&tools, runtime.path(), None, None)
            .err()
            .unwrap();
        assert!(err.contains("did not appear"), "{err}");
        assert!(started.elapsed() >= READY_TIMEOUT);
        wait_gone(fakes.canceller_pid());
    }
}
