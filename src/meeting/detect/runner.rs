//! The live detection loop: watch PipeWire and Hyprland, feed the
//! [`Detector`], show prompts and start meetings when the user says so.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

use super::notify::{self, Notice, Notifier, Response};
use super::{
    describe, hyprland, pipewire, read_activity, Activity, Call, Detector, Event, Snapshot,
};
use crate::config::{CallEnd, Config};

/// Starts and stops meetings.
#[async_trait]
pub trait MeetingControl: Send {
    async fn start(&mut self, title: &str) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
}

/// Runs `voxtype meeting start --window-title ...`, the same path a user's command
/// takes, so every check it makes (meeting mode enabled, daemon up, nothing
/// already recording) applies to a prompt's Record too.
pub struct CliMeetingControl {
    pub exe: PathBuf,
    pub config_path: Option<PathBuf>,
}

impl CliMeetingControl {
    pub fn current(config_path: Option<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self {
            exe: std::env::current_exe()?,
            config_path,
        })
    }

    async fn run(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut cmd = tokio::process::Command::new(&self.exe);
        if let Some(path) = &self.config_path {
            cmd.arg("--config").arg(path);
        }
        let out = cmd
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            anyhow::bail!(
                "voxtype {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )
        }
    }
}

#[async_trait]
impl MeetingControl for CliMeetingControl {
    async fn start(&mut self, title: &str) -> anyhow::Result<()> {
        self.run(&["meeting", "start", "--window-title", title])
            .await
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        self.run(&["meeting", "stop"]).await
    }
}

/// What an open notification is asking.
#[derive(Debug, Clone)]
enum Asking {
    Record(Call),
    /// "Call ended. Stop the meeting recording?", about this app's call
    Stop(String),
    /// "Call ended, stopping in N s": its Keep recording cancels the stop.
    AutoStop,
}

/// What to do when a detected call ends while a meeting records
/// (`[meeting.detect] on_call_end` and `stop_grace_secs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallEndPolicy {
    pub mode: CallEnd,
    pub grace: Duration,
}

impl CallEndPolicy {
    pub fn from_config(detect: &crate::config::MeetingDetectConfig) -> Self {
        Self {
            mode: detect.call_end(),
            grace: Duration::from_secs(u64::from(detect.stop_grace_secs)),
        }
    }
}

impl Default for CallEndPolicy {
    fn default() -> Self {
        Self {
            mode: CallEnd::Stop,
            grace: Duration::from_secs(20),
        }
    }
}

/// A stop waiting out its grace period.
#[derive(Debug, Clone)]
struct PendingStop {
    at: tokio::time::Instant,
    app: String,
    notice: Option<u32>,
}

/// Turns detector events into notifications, and the user's answers into
/// meeting starts and stops.
pub struct Controller<N: Notifier, M: MeetingControl> {
    notifier: N,
    control: M,
    open: HashMap<u32, Asking>,
    call_end: CallEndPolicy,
    pending_stop: Option<PendingStop>,
}

/// The prompt's reminder that voxtype tells no one else.
pub const CONSENT_LINE: &str = "Let the others know you're transcribing.";

pub fn record_notice(call: &Call) -> Notice {
    Notice {
        summary: format!("{} call detected", call.app),
        body: format!("Record \"{}\" as a meeting?\n{}", call.title, CONSENT_LINE),
        actions: vec![
            (notify::ACTION_DEFAULT.into(), "Record".into()),
            (notify::ACTION_RECORD.into(), "Record".into()),
            (notify::ACTION_DISMISS.into(), "Dismiss".into()),
        ],
    }
}

/// The notice before an automatic stop. The only button keeps recording;
/// closing it lets the stop go ahead.
pub fn auto_stop_notice(call: &Call, grace: Duration) -> Notice {
    Notice {
        summary: "Call ended".into(),
        body: format!(
            "{} released the microphone. Stopping the recording in {} s.",
            call.app,
            grace.as_secs()
        ),
        actions: vec![(notify::ACTION_KEEP.into(), "Keep recording".into())],
    }
}

pub fn stop_notice(call: &Call) -> Notice {
    Notice {
        summary: "Call ended".into(),
        body: format!(
            "{} released the microphone. Stop the meeting recording?",
            call.app
        ),
        actions: vec![
            (notify::ACTION_STOP.into(), "Stop meeting".into()),
            (notify::ACTION_KEEP.into(), "Keep recording".into()),
        ],
    }
}

impl<N: Notifier, M: MeetingControl> Controller<N, M> {
    pub fn new(notifier: N, control: M) -> Self {
        Self {
            notifier,
            control,
            open: HashMap::new(),
            call_end: CallEndPolicy::default(),
            pending_stop: None,
        }
    }

    pub fn with_call_end(mut self, policy: CallEndPolicy) -> Self {
        self.call_end = policy;
        self
    }

    /// When the waiting automatic stop is due, if one is.
    pub fn stop_due_at(&self) -> Option<tokio::time::Instant> {
        self.pending_stop.as_ref().map(|p| p.at)
    }

    /// Drop the waiting automatic stop, closing its notice.
    async fn cancel_stop(&mut self, why: &str) {
        if let Some(p) = self.pending_stop.take() {
            tracing::info!("Meeting detection: not stopping the meeting: {}", why);
            if let Some(id) = p.notice {
                self.open.remove(&id);
                self.notifier.close(id).await;
            }
        }
    }

    /// The grace period is over: stop the meeting, unless it already
    /// stopped some other way.
    pub async fn fire_due_stop(&mut self, meeting_active: bool) {
        let Some(p) = self.pending_stop.take() else {
            return;
        };
        if let Some(id) = p.notice {
            self.open.remove(&id);
            self.notifier.close(id).await;
        }
        if !meeting_active {
            tracing::info!("Meeting detection: the meeting had already stopped");
            return;
        }
        tracing::info!(
            "Meeting detection: stopping the meeting, the {} call ended",
            p.app
        );
        if let Err(e) = self.control.stop().await {
            tracing::warn!("Could not stop the meeting: {}", e);
        }
    }

    pub fn control(&self) -> &M {
        &self.control
    }

    /// Act on one detector event.
    pub async fn on_event(&mut self, event: &Event) {
        match event {
            Event::Prompt(call) => match self.notifier.show(&record_notice(call)).await {
                Ok(id) => {
                    self.open.insert(id, Asking::Record(call.clone()));
                }
                Err(e) => tracing::warn!("Could not show the meeting prompt: {}", e),
            },
            Event::AutoRecord(call) => {
                tracing::info!("Auto-recording {} call \"{}\"", call.app, call.title);
                if let Err(e) = self.control.start(&call.title).await {
                    tracing::warn!("Could not start the meeting: {}", e);
                }
            }
            Event::Ended { call, offer_stop } => {
                // A Record prompt nobody answered is stale once the call is over.
                let stale: Vec<u32> = self
                    .open
                    .iter()
                    .filter(|(_, a)| matches!(a, Asking::Record(c) if c.app == call.app))
                    .map(|(id, _)| *id)
                    .collect();
                for id in stale {
                    self.open.remove(&id);
                    self.notifier.close(id).await;
                }
                if !*offer_stop {
                    return;
                }
                match self.call_end.mode {
                    CallEnd::Keep => tracing::info!(
                        "Meeting detection: the {} call ended; keeping the meeting (on_call_end = keep)",
                        call.app
                    ),
                    CallEnd::Ask => {
                        tracing::info!(
                            "Meeting detection: the {} call ended; asking whether to stop",
                            call.app
                        );
                        match self.notifier.show(&stop_notice(call)).await {
                            Ok(id) => {
                                self.open.insert(id, Asking::Stop(call.app.clone()));
                            }
                            Err(e) => tracing::warn!("Could not show the stop prompt: {}", e),
                        }
                    }
                    CallEnd::Stop => {
                        let grace = self.call_end.grace;
                        tracing::info!(
                            "Meeting detection: the {} call ended; stopping the meeting in {} s unless kept",
                            call.app,
                            grace.as_secs()
                        );
                        let notice = match self.notifier.show(&auto_stop_notice(call, grace)).await {
                            Ok(id) => {
                                self.open.insert(id, Asking::AutoStop);
                                Some(id)
                            }
                            Err(e) => {
                                tracing::warn!("Could not show the stop notice: {}", e);
                                None
                            }
                        };
                        self.pending_stop = Some(PendingStop {
                            at: tokio::time::Instant::now() + grace,
                            app: call.app.clone(),
                            notice,
                        });
                    }
                }
            }
            // The same call seen again within the grace period (its app
            // back on the mic, or its window back): it went on after all.
            Event::Seen { call, .. } | Event::Returned { call } => {
                if self
                    .pending_stop
                    .as_ref()
                    .is_some_and(|p| p.app == call.app)
                {
                    self.cancel_stop("the call came back").await;
                }
                // "Stop the meeting recording?" asks about a call that is
                // not over after all.
                let stale: Vec<u32> = self
                    .open
                    .iter()
                    .filter(|(_, a)| matches!(a, Asking::Stop(app) if *app == call.app))
                    .map(|(id, _)| *id)
                    .collect();
                for id in stale {
                    tracing::info!(
                        "Meeting detection: not asking to stop the meeting any more: the call came back"
                    );
                    self.open.remove(&id);
                    self.notifier.close(id).await;
                }
            }
            Event::Held { .. } | Event::Brief { .. } | Event::Continues { .. } => {}
        }
    }

    /// Act on the user's answer to a notification. Ignores notifications
    /// this controller did not post.
    pub async fn on_response(&mut self, (id, key): Response) {
        let Some(asking) = self.open.remove(&id) else {
            return;
        };
        match (asking, key.as_str()) {
            (Asking::Record(call), notify::ACTION_RECORD | notify::ACTION_DEFAULT) => {
                tracing::info!("Recording {} call \"{}\"", call.app, call.title);
                if let Err(e) = self.control.start(&call.title).await {
                    tracing::warn!("Could not start the meeting: {}", e);
                }
                self.notifier.close(id).await;
            }
            (Asking::Stop(_), notify::ACTION_STOP | notify::ACTION_DEFAULT) => {
                tracing::info!("Meeting detection: stopping the meeting, as asked");
                if let Err(e) = self.control.stop().await {
                    tracing::warn!("Could not stop the meeting: {}", e);
                }
                self.notifier.close(id).await;
            }
            (Asking::AutoStop, notify::ACTION_KEEP) => {
                // Put it back so cancel_stop closes it with the stop.
                self.open.insert(id, Asking::AutoStop);
                self.cancel_stop("Keep recording chosen").await;
            }
            // Dismissing the notice does not keep the meeting going.
            (Asking::AutoStop, _) => {}
            (_, notify::CLOSED) => {}
            (_, _) => self.notifier.close(id).await,
        }
    }
}

/// Where the loop reads voxtype's own activity from.
#[derive(Debug, Clone)]
pub struct ActivitySource {
    pub runtime_dir: PathBuf,
    pub state_file: Option<PathBuf>,
}

impl ActivitySource {
    pub fn from_config(config: &Config) -> Self {
        Self {
            runtime_dir: Config::runtime_dir(),
            state_file: config.resolve_state_file(),
        }
    }

    pub fn read(&self) -> Activity {
        read_activity(&self.runtime_dir, self.state_file.as_deref())
    }
}

/// Run `pw-dump` once.
pub async fn dump_streams() -> anyhow::Result<Vec<super::MicStream>> {
    let out = tokio::process::Command::new("pw-dump")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("cannot run pw-dump ({}); is PipeWire installed?", e))?;
    if !out.status.success() {
        anyhow::bail!("pw-dump failed; is PipeWire running?");
    }
    Ok(pipewire::parse_dump(&String::from_utf8_lossy(&out.stdout))?)
}

/// One look at the system.
pub async fn snapshot() -> anyhow::Result<Snapshot> {
    Ok(Snapshot {
        streams: dump_streams().await?,
        windows: hyprland::query_windows().await,
    })
}

/// A signal on every PipeWire graph change, from `pw-dump --monitor`. Its
/// output is only used as a wake-up; each change is then read with a fresh
/// `pw-dump`, so we never depend on the incremental format.
fn watch_pipewire() -> Option<mpsc::Receiver<()>> {
    let mut child = tokio::process::Command::new("pw-dump")
        .arg("--monitor")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| tracing::warn!("Cannot watch PipeWire (pw-dump --monitor): {}", e))
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let _child = child;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(_)) = lines.next_line().await {
            // A full channel already means "changed"; drop the rest.
            let _ = tx.try_send(());
            if tx.is_closed() {
                break;
            }
        }
    });
    Some(rx)
}

/// How the loop reports and acts.
pub enum Mode<N: Notifier, M: MeetingControl> {
    /// Print each decision; never notify or start anything.
    DryRun,
    /// Prompt and act; `print` also echoes decisions to stdout.
    Live {
        controller: Controller<N, M>,
        responses: mpsc::UnboundedReceiver<Response>,
        print: bool,
    },
}

/// How often to re-check while a call is tracked, and how long a burst of
/// PipeWire changes is let settle before reading the graph.
const TRACK_TICK: Duration = Duration::from_secs(2);
const SETTLE: Duration = Duration::from_millis(500);
/// Floor between two `pw-dump` reads. A browser in a call adjusts its
/// capture volume often, and each change wakes the monitor.
const MIN_DUMP_GAP: Duration = Duration::from_secs(2);

/// Watch until cancelled.
pub async fn run<N: Notifier, M: MeetingControl>(
    mut detector: Detector,
    activity: ActivitySource,
    mode: Mode<N, M>,
) {
    let (mut controller, mut responses, print) = match mode {
        Mode::DryRun => (None, None, true),
        Mode::Live {
            controller,
            responses,
            print,
        } => (Some(controller), Some(responses), print),
    };
    let mut changes = watch_pipewire();
    if changes.is_none() {
        tracing::warn!("Meeting detection falls back to polling PipeWire every 2 seconds");
    }
    let mut tick = tokio::time::interval(TRACK_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut streams = Vec::new();
    let mut graph_changed = true;
    let mut dump_failed = false;
    let mut last_dump: Option<Instant> = None;

    loop {
        let stop_at = controller.as_ref().and_then(|c| c.stop_due_at());
        tokio::select! {
            _ = async {
                match stop_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(c) = controller.as_mut() {
                    c.fire_due_stop(activity.read().meeting_active).await;
                }
                continue;
            }
            changed = async {
                match changes.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if changed.is_none() {
                    tracing::warn!("pw-dump --monitor exited; polling PipeWire instead");
                    changes = None;
                }
                let since = last_dump.map_or(MIN_DUMP_GAP, |t| t.elapsed());
                tokio::time::sleep(SETTLE.max(MIN_DUMP_GAP.saturating_sub(since))).await;
                if let Some(rx) = changes.as_mut() {
                    while rx.try_recv().is_ok() {}
                }
                graph_changed = true;
            }
            _ = tick.tick() => {
                // Without a monitor, poll the graph; with one, only re-check
                // windows and timers while a call is tracked.
                if changes.is_none() {
                    graph_changed = true;
                } else if !detector.is_tracking() {
                    continue;
                }
            }
            Some(response) = async {
                match responses.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(c) = controller.as_mut() {
                    c.on_response(response).await;
                }
                continue;
            }
        }

        if graph_changed {
            last_dump = Some(Instant::now());
            match dump_streams().await {
                Ok(s) => {
                    streams = s;
                    dump_failed = false;
                }
                Err(e) => {
                    if !dump_failed {
                        tracing::warn!("Meeting detection cannot read PipeWire: {}", e);
                    }
                    dump_failed = true;
                    continue;
                }
            }
            graph_changed = false;
        }
        let snap = Snapshot {
            streams: streams.clone(),
            windows: hyprland::query_windows().await,
        };
        let events = detector.evaluate(&snap, activity.read(), Instant::now());
        for event in &events {
            // Info, not debug: when a call does not stop or start a meeting,
            // the journal has to say what detection saw.
            tracing::info!("Meeting detection: {}", describe(event));
            if print {
                println!("{}  {}", timestamp(), describe(event));
            }
            if let Some(c) = controller.as_mut() {
                c.on_event(event).await;
            }
        }
    }
}

fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// `voxtype meeting suggest`.
///
/// With `dry_run` and no `watch`: read the system once and print what would
/// be prompted and why. With `watch`: keep running and print each decision.
/// Without `dry_run`: run a foreground detector that really prompts, unless
/// the daemon is already doing so.
pub async fn suggest(
    config: &Config,
    config_path: Option<PathBuf>,
    dry_run: bool,
    watch: bool,
) -> anyhow::Result<()> {
    let detect = &config.meeting.detect;
    let state = match (config.meeting.enabled, detect.enabled, detect.is_enabled()) {
        (false, ..) => "off: [meeting] enabled = false".to_string(),
        (true, Some(true), _) => "on".to_string(),
        (true, Some(false), _) => "off: [meeting.detect] enabled = false".to_string(),
        (true, None, true) => "on (default on Omarchy)".to_string(),
        (true, None, false) => {
            "off (default off outside Omarchy; set [meeting.detect] enabled = true)".to_string()
        }
    };
    println!("Meeting detection in the daemon: {}", state);
    let activity = ActivitySource::from_config(config);

    if dry_run && !watch {
        let snap = snapshot().await?;
        for line in super::report(&snap, activity.read(), detect) {
            println!("{}", line);
        }
        return Ok(());
    }

    if dry_run {
        println!("Watching for calls (dry run: nothing is shown or recorded). Ctrl+C to stop.");
        let mode = Mode::<notify::BusNotifier, CliMeetingControl>::DryRun;
        run(Detector::new(detect.clone()), activity, mode).await;
        return Ok(());
    }

    if other_detector_running() {
        anyhow::bail!(
            "The voxtype daemon is already watching for calls and will prompt.\n  \
             Use --dry-run --watch to see its decisions without a second prompt."
        );
    }
    let conn = zbus::Connection::session().await?;
    let (notifier, responses) = notify::BusNotifier::connect(&conn).await?;
    let control = CliMeetingControl::current(config_path)?;
    println!("Watching for calls; prompts are live. Ctrl+C to stop.");
    let mode = Mode::Live {
        controller: Controller::new(notifier, control)
            .with_call_end(CallEndPolicy::from_config(&config.meeting.detect)),
        responses,
        print: true,
    };
    run(Detector::new(detect.clone()), activity, mode).await;
    Ok(())
}

/// The marker the daemon's detector holds, so `meeting suggest` without
/// `--dry-run` does not prompt a second time for the same call.
pub fn marker_path() -> PathBuf {
    Config::runtime_dir().join("meeting_detect.pid")
}

/// Whether a live detector other than this process is running.
pub fn other_detector_running() -> bool {
    std::fs::read_to_string(marker_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| {
            pid != std::process::id() && PathBuf::from(format!("/proc/{pid}")).exists()
        })
}

/// The daemon's detector task; stops the loop and removes the marker when
/// dropped.
pub struct DetectorHandle {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for DetectorHandle {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(marker_path());
    }
}

/// Start detection inside the daemon, when `[meeting] enabled` and
/// `[meeting.detect]` is on (by default only on Omarchy).
pub fn spawn_in_daemon(config: &Config, config_path: Option<PathBuf>) -> Option<DetectorHandle> {
    if !config.meeting.enabled || !config.meeting.detect.is_enabled() {
        return None;
    }
    let detector = Detector::new(config.meeting.detect.clone());
    let activity = ActivitySource::from_config(config);
    let call_end = CallEndPolicy::from_config(&config.meeting.detect);
    let control = match CliMeetingControl::current(config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Meeting detection disabled: {}", e);
            return None;
        }
    };
    let task = tokio::spawn(async move {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Meeting detection disabled: no session bus ({})", e);
                return;
            }
        };
        let (notifier, responses) = match notify::BusNotifier::connect(&conn).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("Meeting detection disabled: no notification server ({})", e);
                return;
            }
        };
        let _ = std::fs::write(marker_path(), std::process::id().to_string());
        tracing::info!(
            "Meeting detection on: will offer to record calls (on_call_end = {:?}, grace {} s)",
            call_end.mode,
            call_end.grace.as_secs()
        );
        let mode = Mode::Live {
            controller: Controller::new(notifier, control).with_call_end(call_end),
            responses,
            print: false,
        };
        run(detector, activity, mode).await;
    });
    Some(DetectorHandle { task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::detect::{MicStream, Window};
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl Log {
        fn push(&self, s: String) {
            self.0.lock().unwrap().push(s);
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.lock().unwrap())
        }
    }

    struct FakeNotifier {
        log: Log,
        next: u32,
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        async fn show(&mut self, notice: &Notice) -> anyhow::Result<u32> {
            self.next += 1;
            let keys: Vec<&str> = notice.actions.iter().map(|(k, _)| k.as_str()).collect();
            self.log.push(format!(
                "show {} {} [{}]",
                self.next,
                notice.body,
                keys.join(",")
            ));
            Ok(self.next)
        }
        async fn close(&mut self, id: u32) {
            self.log.push(format!("close {}", id));
        }
    }

    struct FakeControl(Log);

    #[async_trait]
    impl MeetingControl for FakeControl {
        async fn start(&mut self, title: &str) -> anyhow::Result<()> {
            self.0.push(format!("start {}", title));
            Ok(())
        }
        async fn stop(&mut self) -> anyhow::Result<()> {
            self.0.push("stop".into());
            Ok(())
        }
    }

    fn call(app: &str, title: &str) -> Call {
        Call {
            app: app.into(),
            title: title.into(),
            reason: String::new(),
            auto_record: false,
            by_window: false,
        }
    }

    fn controller(log: &Log) -> Controller<FakeNotifier, FakeControl> {
        Controller::new(
            FakeNotifier {
                log: log.clone(),
                next: 0,
            },
            FakeControl(log.clone()),
        )
    }

    #[tokio::test]
    async fn record_starts_the_meeting_with_the_call_title() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&Event::Prompt(call("Google Meet", "Meet - abc-defg-hij")))
            .await;
        c.on_response((1, "record".into())).await;
        assert_eq!(
            log.take(),
            vec![
                "show 1 Record \"Meet - abc-defg-hij\" as a meeting?\nLet the others know you're transcribing. [default,record,dismiss]",
                "start Meet - abc-defg-hij",
                "close 1",
            ]
        );
        // The prompt is answered; a second click does nothing.
        c.on_response((1, "record".into())).await;
        assert!(log.take().is_empty());
    }

    #[tokio::test]
    async fn clicking_the_body_records_too() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&Event::Prompt(call("Zoom", "Zoom Meeting")))
            .await;
        c.on_response((1, "default".into())).await;
        assert!(log.take().contains(&"start Zoom Meeting".to_string()));
    }

    #[tokio::test]
    async fn dismiss_and_close_never_start() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&Event::Prompt(call("Zoom", "Zoom Meeting")))
            .await;
        c.on_event(&Event::Prompt(call("Microsoft Teams", "Standup")))
            .await;
        c.on_response((1, "dismiss".into())).await;
        c.on_response((2, "closed".into())).await;
        let log = log.take();
        assert!(log.iter().all(|l| !l.starts_with("start")), "{log:?}");
        assert!(log.contains(&"close 1".to_string()));
    }

    #[tokio::test]
    async fn responses_to_other_notifications_are_ignored() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_response((99, "record".into())).await;
        assert!(log.take().is_empty());
    }

    #[tokio::test]
    async fn auto_record_starts_without_a_prompt() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&Event::AutoRecord(call("Zoom", "Zoom Meeting")))
            .await;
        assert_eq!(log.take(), vec!["start Zoom Meeting"]);
    }

    #[tokio::test]
    async fn call_end_closes_the_stale_prompt_and_offers_stop() {
        let log = Log::default();
        let mut c = controller(&log).with_call_end(ask_policy());
        c.on_event(&Event::Prompt(call("Zoom", "Zoom Meeting")))
            .await;
        c.on_event(&Event::Ended {
            call: call("Zoom", "Zoom Meeting"),
            offer_stop: true,
        })
        .await;
        c.on_response((2, "stop".into())).await;
        assert_eq!(
            log.take(),
            vec![
                "show 1 Record \"Zoom Meeting\" as a meeting?\nLet the others know you're transcribing. [default,record,dismiss]",
                "close 1",
                "show 2 Zoom released the microphone. Stop the meeting recording? [stop,keep]",
                "stop",
                "close 2",
            ]
        );
    }

    fn ask_policy() -> CallEndPolicy {
        CallEndPolicy {
            mode: CallEnd::Ask,
            grace: Duration::from_secs(20),
        }
    }

    fn ended(app: &str, meeting_active: bool) -> Event {
        Event::Ended {
            call: call(app, "Zoom call"),
            offer_stop: meeting_active,
        }
    }

    #[tokio::test]
    async fn stop_mode_announces_then_stops_after_the_grace_period() {
        let log = Log::default();
        let mut c = controller(&log);
        assert_eq!(
            c.call_end,
            CallEndPolicy::default(),
            "stop after 20 s by default"
        );
        let before = tokio::time::Instant::now();
        c.on_event(&ended("Zoom", true)).await;
        assert_eq!(
            log.take(),
            vec!["show 1 Zoom released the microphone. Stopping the recording in 20 s. [keep]"]
        );
        let at = c.stop_due_at().expect("a stop is waiting");
        assert!(at >= before + Duration::from_secs(20) && at <= before + Duration::from_secs(21));
        // Dismissing the notice does not keep the meeting.
        c.on_response((1, "closed".into())).await;
        assert!(c.stop_due_at().is_some());
        c.fire_due_stop(true).await;
        assert_eq!(log.take(), vec!["close 1", "stop"]);
        assert_eq!(c.stop_due_at(), None);
    }

    #[tokio::test]
    async fn keep_recording_cancels_the_automatic_stop() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&ended("Zoom", true)).await;
        c.on_response((1, "keep".into())).await;
        assert_eq!(c.stop_due_at(), None);
        c.fire_due_stop(true).await;
        let log = log.take();
        assert!(!log.contains(&"stop".to_string()), "{log:?}");
        assert!(log.contains(&"close 1".to_string()), "{log:?}");
    }

    #[tokio::test]
    async fn the_call_coming_back_cancels_the_stop() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&ended("Zoom", true)).await;
        c.on_event(&Event::Seen {
            call: call("Microsoft Teams", "Teams call"),
            settle: Duration::from_secs(5),
        })
        .await;
        assert!(
            c.stop_due_at().is_some(),
            "another app's call changes nothing"
        );
        c.on_event(&Event::Seen {
            call: call("Zoom", "Zoom call"),
            settle: Duration::from_secs(5),
        })
        .await;
        assert_eq!(c.stop_due_at(), None);
        assert!(!log.take().contains(&"stop".to_string()));
    }

    #[tokio::test]
    async fn the_call_coming_back_withdraws_the_question_to_stop() {
        let log = Log::default();
        let mut c = controller(&log).with_call_end(ask_policy());
        c.on_event(&ended("Zoom", true)).await;
        c.on_event(&Event::Returned {
            call: call("Microsoft Teams", "Teams call"),
        })
        .await;
        c.on_event(&Event::Returned {
            call: call("Zoom", "Zoom call"),
        })
        .await;
        // Answering a question that is gone does nothing.
        c.on_response((1, "stop".into())).await;
        assert_eq!(
            log.take(),
            vec![
                "show 1 Zoom released the microphone. Stop the meeting recording? [stop,keep]",
                "close 1",
            ]
        );
    }

    #[tokio::test]
    async fn the_same_call_returning_cancels_the_stop() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&ended("Zoom", true)).await;
        c.on_event(&Event::Returned {
            call: call("Microsoft Teams", "Teams call"),
        })
        .await;
        assert!(
            c.stop_due_at().is_some(),
            "another app's call changes nothing"
        );
        c.on_event(&Event::Returned {
            call: call("Zoom", "Zoom call"),
        })
        .await;
        assert_eq!(c.stop_due_at(), None);
        c.fire_due_stop(true).await;
        let log = log.take();
        assert!(!log.contains(&"stop".to_string()), "{log:?}");
        assert!(log.contains(&"close 1".to_string()), "{log:?}");
    }

    // The detector and the controller together, as the live loop runs
    // them, on a Zoom call in a browser shaped like the ones that were cut
    // off on 2026-11-17 (see the detector's tests).

    const LAUNCHER_TAB: &str = "Join from Zoom Workplace app - Zoom - Chromium";
    const OTHER_TAB: &str = "Pull requests - Chromium";

    /// What the system shows: whether Chromium has the mic, and the title
    /// of its front tab, if it has a window at all.
    fn look(mic: bool, front_tab: Option<&str>) -> Snapshot {
        let streams = if mic {
            vec![MicStream {
                id: 115,
                app_name: "Chromium input".into(),
                binary: "chromium".into(),
                node_name: "Chromium input".into(),
                media_name: "RecordStream".into(),
                pid: Some(42010),
            }]
        } else {
            Vec::new()
        };
        let windows = front_tab
            .map(|title| Window {
                class: "chromium".into(),
                title: title.into(),
                pid: Some(42000),
            })
            .into_iter()
            .collect();
        Snapshot {
            streams,
            windows: Some(windows),
        }
    }

    /// The live loop's part between two looks: evaluate every two seconds
    /// from `from` up to `to` (seconds after `t0`) and act on each event.
    async fn live(
        d: &mut Detector,
        c: &mut Controller<FakeNotifier, FakeControl>,
        snapshot: &Snapshot,
        meeting_active: bool,
        t0: Instant,
        from: u64,
        to: u64,
    ) {
        let activity = Activity {
            meeting_active,
            dictating: false,
        };
        for at in (from..to).step_by(2) {
            let now = t0 + Duration::from_secs(at);
            for event in d.evaluate(snapshot, activity, now) {
                c.on_event(&event).await;
            }
        }
    }

    /// A Zoom call in Chromium recorded from the prompt: the detector and
    /// the controller a minute into it, with the prompt's lines taken.
    async fn recorded_zoom_call(
        log: &Log,
        t0: Instant,
    ) -> (Detector, Controller<FakeNotifier, FakeControl>) {
        let mut d = Detector::new(crate::config::MeetingDetectConfig::default());
        let mut c = controller(log);
        let joined = look(true, Some(LAUNCHER_TAB));
        live(&mut d, &mut c, &joined, false, t0, 0, 10).await;
        c.on_response((1, "record".into())).await;
        assert_eq!(
            log.take(),
            vec![
                "show 1 Record \"Zoom call\" as a meeting?\nLet the others know you're transcribing. [default,record,dismiss]",
                "start Zoom call",
                "close 1",
            ]
        );
        live(&mut d, &mut c, &joined, true, t0, 10, 60).await;
        (d, c)
    }

    #[tokio::test]
    async fn another_tab_in_front_of_the_call_stops_nothing() {
        let log = Log::default();
        let t0 = Instant::now();
        let (mut d, mut c) = recorded_zoom_call(&log, t0).await;
        assert_eq!(c.call_end.mode, CallEnd::Stop);
        // The launcher tab goes behind another one; the call has the mic.
        let behind = look(true, Some(OTHER_TAB));
        live(&mut d, &mut c, &behind, true, t0, 60, 1860).await;
        assert!(log.take().is_empty(), "no notice, no stop");
        assert_eq!(c.stop_due_at(), None);
    }

    #[tokio::test]
    async fn a_released_mic_with_the_call_window_open_stops_nothing() {
        let log = Log::default();
        let t0 = Instant::now();
        let (mut d, mut c) = recorded_zoom_call(&log, t0).await;
        let muted = look(false, Some(LAUNCHER_TAB));
        let unmuted = look(true, Some(LAUNCHER_TAB));
        live(&mut d, &mut c, &muted, true, t0, 60, 660).await;
        live(&mut d, &mut c, &unmuted, true, t0, 660, 720).await;
        live(&mut d, &mut c, &muted, true, t0, 720, 1320).await;
        assert!(
            log.take().is_empty(),
            "no notice, no stop, no second prompt"
        );
        assert_eq!(c.stop_due_at(), None);
    }

    #[tokio::test]
    async fn leaving_the_call_stops_the_meeting_after_the_grace_period() {
        let log = Log::default();
        let t0 = Instant::now();
        let (mut d, mut c) = recorded_zoom_call(&log, t0).await;
        // Muted for a while, then the browser window is closed.
        live(
            &mut d,
            &mut c,
            &look(false, Some(LAUNCHER_TAB)),
            true,
            t0,
            60,
            300,
        )
        .await;
        assert_eq!(c.stop_due_at(), None);
        live(&mut d, &mut c, &look(false, None), true, t0, 300, 304).await;
        assert_eq!(c.stop_due_at(), None, "not before the debounce");
        live(&mut d, &mut c, &look(false, None), true, t0, 304, 310).await;
        assert_eq!(
            log.take(),
            vec![
                "show 2 Zoom (web) released the microphone. Stopping the recording in 20 s. [keep]"
            ]
        );
        assert!(c.stop_due_at().is_some());
        c.fire_due_stop(true).await;
        assert_eq!(log.take(), vec!["close 2", "stop"]);
    }

    #[tokio::test]
    async fn the_call_back_within_the_grace_period_calls_the_stop_off() {
        for back in [
            // The call's tab is in front again, still muted
            look(false, Some(LAUNCHER_TAB)),
            // Unmuted, with another tab in front
            look(true, Some(OTHER_TAB)),
            look(true, Some(LAUNCHER_TAB)),
        ] {
            let log = Log::default();
            let t0 = Instant::now();
            let (mut d, mut c) = recorded_zoom_call(&log, t0).await;
            live(
                &mut d,
                &mut c,
                &look(false, Some(OTHER_TAB)),
                true,
                t0,
                60,
                68,
            )
            .await;
            assert!(c.stop_due_at().is_some(), "{back:?}");
            live(&mut d, &mut c, &back, true, t0, 76, 400).await;
            assert_eq!(c.stop_due_at(), None, "{back:?}");
            c.fire_due_stop(true).await;
            assert_eq!(
                log.take(),
                vec![
                    "show 2 Zoom (web) released the microphone. Stopping the recording in 20 s. [keep]",
                    "close 2",
                ],
                "{back:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_meeting_stopped_meanwhile_is_not_stopped_again() {
        let log = Log::default();
        let mut c = controller(&log);
        c.on_event(&ended("Zoom", true)).await;
        log.take();
        c.fire_due_stop(false).await;
        assert_eq!(log.take(), vec!["close 1"]);
    }

    #[tokio::test]
    async fn keep_mode_leaves_the_meeting_running() {
        let log = Log::default();
        let mut c = controller(&log).with_call_end(CallEndPolicy {
            mode: CallEnd::Keep,
            grace: Duration::from_secs(20),
        });
        c.on_event(&ended("Zoom", true)).await;
        assert!(log.take().is_empty(), "no notice, no stop");
        assert_eq!(c.stop_due_at(), None);
    }

    #[tokio::test]
    async fn no_meeting_means_nothing_to_stop_in_every_mode() {
        for mode in [CallEnd::Stop, CallEnd::Ask, CallEnd::Keep] {
            let log = Log::default();
            let mut c = controller(&log).with_call_end(CallEndPolicy {
                mode,
                grace: Duration::from_secs(20),
            });
            c.on_event(&ended("Zoom", false)).await;
            assert!(log.take().is_empty(), "{mode:?}");
            assert_eq!(c.stop_due_at(), None, "{mode:?}");
        }
    }

    #[tokio::test]
    async fn a_zero_grace_period_stops_at_once() {
        let log = Log::default();
        let mut c = controller(&log).with_call_end(CallEndPolicy {
            mode: CallEnd::Stop,
            grace: Duration::ZERO,
        });
        c.on_event(&ended("Zoom", true)).await;
        assert!(c.stop_due_at().unwrap() <= tokio::time::Instant::now());
    }

    #[tokio::test]
    async fn keep_recording_does_not_stop() {
        let log = Log::default();
        let mut c = controller(&log).with_call_end(ask_policy());
        c.on_event(&Event::Ended {
            call: call("Zoom", "Zoom Meeting"),
            offer_stop: true,
        })
        .await;
        c.on_response((1, "keep".into())).await;
        assert!(!log.take().contains(&"stop".to_string()));
    }

    #[tokio::test]
    async fn cli_control_runs_meeting_start_with_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("args");
        let exe = dir.path().join("fake-voxtype");
        // write_script, not fs::write + chmod: exec'ing a file another
        // test thread's fork still holds open for writing fails with
        // ETXTBSY under parallel tests.
        crate::test_support::write_script(
            &exe,
            &format!("#!/bin/sh\nprintf '%s|' \"$@\" > '{}'\n", out.display()),
        );

        let mut control = CliMeetingControl {
            exe,
            config_path: Some(PathBuf::from("/etc/vt.toml")),
        };
        control.start("Meet - abc-defg-hij").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "--config|/etc/vt.toml|meeting|start|--window-title|Meet - abc-defg-hij|"
        );
    }
}
