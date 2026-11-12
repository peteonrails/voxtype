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

/// `[meeting.audio] echo_cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The WebRTC canceller on speakers, GTCRN otherwise
    Auto,
    /// The WebRTC canceller whatever the output
    Aec,
    /// GTCRN only, as before 1.3.0
    Gtcrn,
    /// Neither
    Off,
}

impl Mode {
    /// Parse the config value. `disabled` is the old name of `off`; an
    /// unknown value is `auto`, with a warning.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Mode::Auto,
            "aec" | "webrtc" => Mode::Aec,
            "gtcrn" => Mode::Gtcrn,
            "off" | "disabled" | "none" => Mode::Off,
            other => {
                tracing::warn!(
                    "Unknown [meeting.audio] echo_cancel '{}'; using auto (auto, aec, gtcrn or off)",
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
    /// PipeWire's WebRTC echo canceller
    Aec,
    /// The GTCRN enhancer on the mic track
    Gtcrn,
    /// Nothing
    Off,
}

impl EchoPath {
    pub fn as_str(self) -> &'static str {
        match self {
            EchoPath::Aec => "aec",
            EchoPath::Gtcrn => "gtcrn",
            EchoPath::Off => "off",
        }
    }
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

/// Ask pactl what the default output is.
pub fn default_output() -> Output {
    let run = |args: &[&str]| -> Option<String> {
        let out = Command::new("pactl")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    match (
        run(&["get-default-sink"]),
        run(&["-f", "json", "list", "sinks"]),
    ) {
        (Some(default), Some(sinks)) => classify_default(&sinks, &default),
        _ => Output::Unknown,
    }
}

/// What a meeting should try first. `specific_mic` is a
/// `[meeting.audio] mic_device` other than the default, which `auto` leaves
/// to GTCRN: it names an ALSA device, not a PipeWire node the canceller can
/// capture. `available` is whether the canceller's tools are installed.
pub fn plan(mode: Mode, output: Output, specific_mic: bool, available: bool) -> EchoPath {
    match mode {
        Mode::Off => EchoPath::Off,
        Mode::Gtcrn => EchoPath::Gtcrn,
        Mode::Aec if available => EchoPath::Aec,
        Mode::Aec => EchoPath::Gtcrn,
        Mode::Auto if available && output != Output::Headphones && !specific_mic => EchoPath::Aec,
        Mode::Auto => EchoPath::Gtcrn,
    }
}

/// Decide a meeting's echo handling and, for the canceller, start it. Never
/// fails: a canceller that cannot start is logged once and the meeting
/// uses GTCRN instead. `mic_device` is `[meeting.audio] mic_device`.
pub fn begin(
    mode: Mode,
    mic_device: &str,
    runtime_dir: &Path,
) -> (EchoPath, Option<EchoCanceller>) {
    let specific_mic = !matches!(mic_device.trim(), "default" | "" | "pipewire" | "pulse");
    let tools = Tools::detect();
    let output = match mode {
        Mode::Auto => default_output(),
        _ => Output::Unknown,
    };
    let path = plan(mode, output, specific_mic, tools.is_some());
    match (path, tools) {
        (EchoPath::Aec, Some(tools)) => {
            let target = specific_mic.then_some(mic_device.trim());
            match EchoCanceller::start(&tools, runtime_dir, target) {
                Ok(canceller) => {
                    tracing::info!(
                        "Meeting echo handling: PipeWire WebRTC echo canceller (output: {:?})",
                        output
                    );
                    (EchoPath::Aec, Some(canceller))
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
        (path, tools) => {
            let why = if path != EchoPath::Gtcrn || mode == Mode::Gtcrn {
                "as configured"
            } else if tools.is_none() {
                "pipewire, pactl or parec is not installed"
            } else if output == Output::Headphones {
                "the output is headphones"
            } else {
                "meeting.audio.mic_device names a specific device"
            };
            if mode == Mode::Aec {
                tracing::warn!("Echo cancellation needs pipewire, pactl and parec; using GTCRN");
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
}

impl Tools {
    /// `pipewire`, `pactl` and `parec` (which records the mic from the
    /// cancelled source) on PATH, or `None`.
    pub fn detect() -> Option<Self> {
        which::which("parec").ok()?;
        Some(Self {
            pipewire: which::which("pipewire").ok()?,
            pactl: which::which("pactl").ok()?,
        })
    }
}

/// The canceller's PipeWire config: a client-only context that loads the
/// echo-cancel module in monitor mode, capturing from `target` (or the
/// default source) and offering the cancelled audio as [`SOURCE_NODE`].
pub fn config(target: Option<&str>) -> String {
    let target = target
        .map(|t| format!(" target.object = \"{}\"", t.replace('"', "")))
        .unwrap_or_default();
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
                node.description = "Voxtype meeting mic (echo cancelled)"
                priority.session = 1
            }}
            sink.props = {{ node.name = {SINK_NODE} }}
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
    /// mic to capture (`None` follows the default source).
    pub fn start(tools: &Tools, runtime_dir: &Path, target: Option<&str>) -> Result<Self, String> {
        std::fs::create_dir_all(runtime_dir).map_err(|e| e.to_string())?;
        let conf = runtime_dir.join(CONF_FILE);
        let stale = stop_stale(&conf);
        if stale > 0 {
            tracing::info!(
                "Stopped {} echo canceller(s) left by an earlier daemon",
                stale
            );
        }
        std::fs::write(&conf, config(target)).map_err(|e| e.to_string())?;
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

        let deadline = Instant::now() + READY_TIMEOUT;
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
                    READY_TIMEOUT.as_secs()
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
    fn modes_parse_with_disabled_as_off() {
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse("AEC"), Mode::Aec);
        assert_eq!(Mode::parse("gtcrn"), Mode::Gtcrn);
        assert_eq!(Mode::parse("off"), Mode::Off);
        assert_eq!(Mode::parse("disabled"), Mode::Off);
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

    #[test]
    fn the_plan_follows_mode_output_mic_and_availability() {
        use EchoPath::*;
        use Output::*;
        assert_eq!(plan(Mode::Auto, Speakers, false, true), Aec);
        assert_eq!(plan(Mode::Auto, Unknown, false, true), Aec);
        assert_eq!(plan(Mode::Auto, Headphones, false, true), Gtcrn);
        assert_eq!(
            plan(Mode::Auto, Speakers, true, true),
            Gtcrn,
            "a specific mic device"
        );
        assert_eq!(
            plan(Mode::Auto, Speakers, false, false),
            Gtcrn,
            "no module tools"
        );
        assert_eq!(plan(Mode::Aec, Headphones, true, true), Aec);
        assert_eq!(plan(Mode::Aec, Speakers, false, false), Gtcrn);
        assert_eq!(plan(Mode::Gtcrn, Speakers, false, true), Gtcrn);
        assert_eq!(plan(Mode::Off, Speakers, false, true), Off);
    }

    #[test]
    fn the_config_runs_the_webrtc_canceller_in_monitor_mode() {
        let c = config(None);
        assert!(c.contains("libpipewire-module-echo-cancel"));
        assert!(c.contains("library.name = aec/libspa-aec-webrtc"));
        assert!(c.contains("monitor.mode = true"));
        assert!(c.contains(&format!("node.name = {SOURCE_NODE}")));
        assert!(!c.contains("target.object"), "follows the default source");
        assert!(
            config(Some("alsa_input.usb-mic")).contains("target.object = \"alsa_input.usb-mic\"")
        );
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
                &format!(
                    "#!/bin/sh\necho $$ > {}\ntrap 'kill $! 2>/dev/null; exit 0' TERM\nsleep 600 &\nwait\n",
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
            }
        }

        fn canceller_pid(&self) -> i32 {
            let path = self.dir.path().join("canceller.pid");
            let deadline = Instant::now() + Duration::from_secs(5);
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
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(pid) {
            assert!(Instant::now() < deadline, "canceller {pid} still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_canceller_starts_and_stops_with_its_config() {
        let fakes = Fakes::new("sources", SOURCE_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let mut aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None).unwrap();
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
        let aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None).unwrap();
        let pid = fakes.canceller_pid();
        let shell = aec.abandon();
        wait_gone(pid);
        let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(shell as i32), None);
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
        let orphan_pid = orphan.id() as i32;
        assert!(running(orphan_pid));
        let _ = std::fs::remove_file(fakes.dir.path().join("canceller.pid"));

        let mut aec = EchoCanceller::start(&fakes.tools(), runtime.path(), None).unwrap();
        let _ = orphan.wait();
        assert!(!running(orphan_pid), "the stale one was stopped");
        aec.stop();
    }

    #[test]
    fn an_old_pipewire_or_a_silent_one_is_refused() {
        // monitor.mode ignored: the module made a sink instead.
        let fakes = Fakes::new("sinks", SINK_NODE);
        let runtime = tempfile::TempDir::new().unwrap();
        let err = EchoCanceller::start(&fakes.tools(), runtime.path(), None)
            .err()
            .unwrap();
        assert!(err.contains("monitor mode"), "{err}");
        wait_gone(fakes.canceller_pid());

        // Nothing appears at all.
        let fakes = Fakes::new("sources", "something_else");
        let started = Instant::now();
        let err = EchoCanceller::start(&fakes.tools(), runtime.path(), None)
            .err()
            .unwrap();
        assert!(err.contains("did not appear"), "{err}");
        assert!(started.elapsed() < READY_TIMEOUT + Duration::from_secs(2));
        wait_gone(fakes.canceller_pid());
    }
}
