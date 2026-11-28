//! Saving and restoring the clipboard around a paste-mode dictation.
//!
//! Paste mode puts the transcript on the clipboard, presses the paste key,
//! and with `restore_clipboard = true` puts back what was there before.
//! Three rules keep that from costing the user anything (#1003, #1079):
//!
//! - **Every read has a deadline.** A selection owner that never answers
//!   (a frozen app) would otherwise hold the dictation forever. Readers run
//!   in their own process group, and the whole group is killed on timeout,
//!   because `wl-paste` forks `cat` to copy the data and killing only the
//!   parent leaves the pipe open.
//! - **A failed read is not an empty clipboard.** `wl-paste` saying nothing
//!   is copied means there is nothing to restore. Any other failure means
//!   the clipboard could not be saved: the paste still goes ahead, since the
//!   dictation matters more, but the log says the clipboard will keep it.
//! - **A newer copy wins.** The restore runs after the paste returns, off
//!   the daemon's event loop. Before writing, it checks the clipboard still
//!   holds the transcript; if the user copied something in the meantime,
//!   the restore is dropped rather than overwrite it.
//!
//! `wl-copy` serves one representation per invocation, so only one can be
//! restored. A text representation is preferred when one is offered, so a
//! plain-text paste after the restore gives the original plain text rather
//! than, say, HTML markup served under a text alias. When nothing textual
//! is offered, the first type is kept (an image, for example).

use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::oneshot;

use super::clipboard::WL_COPY_TEXT_MIME;
use super::session::{detect, DisplaySession};

/// How long one clipboard read may take. Answering a selection request is
/// a few milliseconds for a live app; two seconds only ever trips for an
/// owner that is not going to answer.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Largest clipboard content that is saved for restoration.
const MAX_CLIPBOARD_SIZE: usize = 100 * 1024 * 1024;

/// Text types in the order a restore prefers them.
const TEXT_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// Clipboard content with the MIME type it was offered under.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ClipboardContent {
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl std::fmt::Debug for ClipboardContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardContent")
            .field("mime_type", &self.mime_type)
            .field("data", &format!("[{} bytes]", self.data.len()))
            .finish()
    }
}

/// What reading the clipboard before a paste found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Snapshot {
    /// Nothing was copied; there is nothing to restore.
    Empty,
    /// The content to put back after the paste.
    Saved(ClipboardContent),
    /// The clipboard could not be read (an error or the deadline). It
    /// cannot be restored.
    Failed(String),
}

/// Why a bounded command produced no output.
#[derive(Debug)]
enum RunError {
    NotFound,
    TimedOut,
    Io(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::NotFound => write!(f, "not installed"),
            RunError::TimedOut => write!(
                f,
                "no answer within {:.0} s (the app that owns the clipboard may be frozen)",
                READ_TIMEOUT.as_secs_f32()
            ),
            RunError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Run a clipboard reader with a deadline. The child leads its own process
/// group so a timeout can kill everything it forked.
async fn run_bounded(program: &str, args: &[&str]) -> Result<std::process::Output, RunError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RunError::NotFound,
        _ => RunError::Io(e.to_string()),
    })?;
    let pid = child.id();
    match tokio::time::timeout(READ_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(RunError::Io(e.to_string())),
        Err(_) => {
            // Dropping the future already SIGKILLed the leader through
            // kill_on_drop; the group kill reaches the `cat` it forked.
            #[cfg(unix)]
            if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            #[cfg(not(unix))]
            let _ = pid;
            Err(RunError::TimedOut)
        }
    }
}

/// Whether wl-paste's stderr says the clipboard is empty, as opposed to a
/// real failure. wl-clipboard prints "Nothing is copied" (and "No selection"
/// in older releases).
fn wl_paste_says_empty(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("nothing is copied") || stderr.contains("no selection")
}

/// Pick the representation to save from `wl-paste --list-types` output:
/// the most specific text type offered, else the first type.
pub(crate) fn preferred_mime(types: &str) -> Option<String> {
    let offered: Vec<&str> = types
        .lines()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    TEXT_TYPES
        .iter()
        .find(|want| offered.iter().any(|t| t.eq_ignore_ascii_case(want)))
        .map(|t| t.to_string())
        .or_else(|| {
            offered
                .iter()
                .find(|t| t.to_ascii_lowercase().starts_with("text/plain"))
                .map(|t| t.to_string())
        })
        .or_else(|| offered.first().map(|t| t.to_string()))
}

/// Read the clipboard before a paste.
pub(crate) async fn snapshot() -> Snapshot {
    if detect() == DisplaySession::X11 {
        return snapshot_xclip().await;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        // Matches the old behaviour: no session to read from.
        return snapshot_xclip().await;
    }
    snapshot_wl_paste().await
}

async fn snapshot_wl_paste() -> Snapshot {
    let types = match run_bounded("wl-paste", &["--list-types"]).await {
        Ok(output) => output,
        Err(e) => return Snapshot::Failed(format!("wl-paste --list-types: {e}")),
    };
    if !types.status.success() {
        let stderr = String::from_utf8_lossy(&types.stderr);
        if wl_paste_says_empty(&stderr) {
            return Snapshot::Empty;
        }
        return Snapshot::Failed(format!("wl-paste --list-types: {}", stderr.trim()));
    }
    let Some(mime_type) = preferred_mime(&String::from_utf8_lossy(&types.stdout)) else {
        return Snapshot::Empty;
    };
    match read_wl_paste(&mime_type).await {
        Ok(Some(data)) => saved(data, mime_type),
        Ok(None) => Snapshot::Empty,
        Err(e) => Snapshot::Failed(e),
    }
}

/// Read one representation. `Ok(None)` when the clipboard is empty.
async fn read_wl_paste(mime_type: &str) -> Result<Option<Vec<u8>>, String> {
    match run_bounded("wl-paste", &["--no-newline", "--type", mime_type]).await {
        Ok(output) if output.status.success() => Ok(Some(output.stdout)),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if wl_paste_says_empty(&stderr) {
                Ok(None)
            } else {
                Err(format!("wl-paste --type {mime_type}: {}", stderr.trim()))
            }
        }
        Err(e) => Err(format!("wl-paste --type {mime_type}: {e}")),
    }
}

async fn snapshot_xclip() -> Snapshot {
    if std::env::var_os("DISPLAY").is_none() {
        return Snapshot::Failed("no Wayland or X11 display to read the clipboard from".into());
    }
    match read_xclip().await {
        Ok(Some(data)) => {
            let mime_type = if std::str::from_utf8(&data).is_ok() {
                "text/plain"
            } else {
                "application/octet-stream"
            };
            saved(data, mime_type.to_string())
        }
        Ok(None) => Snapshot::Empty,
        Err(e) => Snapshot::Failed(e),
    }
}

async fn read_xclip() -> Result<Option<Vec<u8>>, String> {
    match run_bounded("xclip", &["-selection", "clipboard", "-o"]).await {
        Ok(output) if output.status.success() => {
            Ok((!output.stdout.is_empty()).then_some(output.stdout))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // xclip reports an empty clipboard as an unavailable target.
            if stderr.contains("not available") {
                Ok(None)
            } else {
                Err(format!("xclip: {}", stderr.trim()))
            }
        }
        Err(e) => Err(format!("xclip: {e}")),
    }
}

fn saved(data: Vec<u8>, mime_type: String) -> Snapshot {
    if data.is_empty() {
        return Snapshot::Empty;
    }
    if data.len() > MAX_CLIPBOARD_SIZE {
        return Snapshot::Failed(format!(
            "clipboard content too large to save ({} bytes)",
            data.len()
        ));
    }
    Snapshot::Saved(ClipboardContent { data, mime_type })
}

/// Whether the clipboard still holds `pasted`, the transcript this paste put
/// there. `None` when that cannot be told (the read failed).
async fn still_holds(pasted: &[u8]) -> Option<bool> {
    let current =
        if detect() == DisplaySession::X11 || std::env::var_os("WAYLAND_DISPLAY").is_none() {
            read_xclip().await
        } else {
            read_wl_paste(WL_COPY_TEXT_MIME).await
        };
    match current {
        Ok(Some(data)) => Some(data == pasted),
        // Emptied since the paste: whoever did that is newer than us.
        Ok(None) => Some(false),
        Err(_) => None,
    }
}

/// Put `content` back on the clipboard.
async fn write_back(content: &ClipboardContent) -> Result<(), String> {
    let (program, args): (&str, Vec<&str>) =
        if detect() == DisplaySession::X11 || std::env::var_os("WAYLAND_DISPLAY").is_none() {
            ("xclip", vec!["-selection", "clipboard"])
        } else {
            ("wl-copy", vec!["--type", content.mime_type.as_str()])
        };
    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&content.data)
            .await
            .map_err(|e| format!("{program}: {e}"))?;
    }
    // wl-copy forks a server and returns at once; xclip likewise.
    match tokio::time::timeout(READ_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("{program} exited with {status}")),
        Ok(Err(e)) => Err(format!("{program}: {e}")),
        Err(_) => Err(format!("{program} did not finish")),
    }
}

/// How a scheduled restore ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RestoreOutcome {
    /// The original content is back.
    Restored,
    /// Something newer was copied after the paste, so it was left alone.
    KeptNewerCopy,
    /// The check or the write failed; the clipboard keeps the transcript.
    Failed(String),
    /// A later paste took over before the delay ran out. It carries the
    /// original so that paste restores it instead.
    Superseded(ClipboardContent),
}

struct PendingRestore {
    cancel: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<RestoreOutcome>,
    pasted: Vec<u8>,
}

/// The restore scheduled by the last paste, until it runs.
static PENDING: Mutex<Option<PendingRestore>> = Mutex::new(None);

/// Schedule putting `original` back `delay` after a paste of `pasted`.
/// Returns at once: the paste is done, and the daemon can report success
/// and take the next command while the delay runs (#1079).
pub(crate) fn schedule_restore(original: ClipboardContent, pasted: Vec<u8>, delay: Duration) {
    let (cancel, cancelled) = oneshot::channel::<()>();
    let check = pasted.clone();
    let task = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancelled => return RestoreOutcome::Superseded(original),
        }
        let outcome = match still_holds(&check).await {
            Some(false) => RestoreOutcome::KeptNewerCopy,
            None => RestoreOutcome::Failed(
                "could not read the clipboard to check for a newer copy".into(),
            ),
            Some(true) => match write_back(&original).await {
                Ok(()) => RestoreOutcome::Restored,
                Err(e) => RestoreOutcome::Failed(e),
            },
        };
        match &outcome {
            RestoreOutcome::Restored => tracing::debug!("Restored original clipboard content"),
            RestoreOutcome::KeptNewerCopy => tracing::info!(
                "Clipboard changed after the paste; keeping the newer copy instead of restoring"
            ),
            RestoreOutcome::Failed(e) => {
                tracing::warn!("Failed to restore clipboard content: {}", e)
            }
            RestoreOutcome::Superseded(_) => {}
        }
        outcome
    });
    let previous = PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(PendingRestore {
            cancel,
            task,
            pasted,
        });
    // take_pending() runs before every snapshot, so there is never an
    // earlier restore still waiting here; abort one defensively.
    if let Some(previous) = previous {
        previous.task.abort();
    }
}

/// Settle the previous paste's restore before a new paste reads the
/// clipboard. Cuts its delay short; if it had not run yet, returns the
/// original it was going to restore, together with the transcript it
/// pasted, so the new paste can restore that original instead of saving
/// the previous transcript.
pub(crate) async fn take_pending() -> Option<(ClipboardContent, Vec<u8>)> {
    let pending = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let _ = pending.cancel.send(());
    match tokio::time::timeout(READ_TIMEOUT * 2, pending.task).await {
        Ok(Ok(RestoreOutcome::Superseded(original))) => Some((original, pending.pasted)),
        _ => None,
    }
}

/// Decide what a new paste must restore, given its fresh snapshot and the
/// previous paste's unfinished restore (from [`take_pending`]). While the
/// clipboard still holds the previous transcript, the user's real content
/// is the original the previous paste saved; if anything else is there now,
/// the user copied it since, and that is what to keep.
pub(crate) fn merge_pending(
    fresh: Snapshot,
    pending: Option<(ClipboardContent, Vec<u8>)>,
) -> Snapshot {
    match (fresh, pending) {
        (Snapshot::Saved(current), Some((original, previous_paste)))
            if current.data == previous_paste =>
        {
            Snapshot::Saved(original)
        }
        (fresh, _) => fresh,
    }
}

/// Wait for a scheduled restore, for tests and shutdown.
#[cfg(test)]
pub(crate) async fn wait_pending() -> Option<RestoreOutcome> {
    let pending = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    pending.task.await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_content_debug_redacts_data() {
        let content = ClipboardContent {
            data: vec![1, 2, 3, 4, 5],
            mime_type: "text/plain".to_string(),
        };
        let debug_str = format!("{:?}", content);
        assert!(debug_str.contains("[5 bytes]"));
        assert!(debug_str.contains("text/plain"));
        assert!(!debug_str.contains("[1, 2, 3"));
    }

    #[test]
    fn prefers_plain_text_over_a_rich_first_type() {
        let types = "text/html\ntext/plain;charset=utf-8\ntext/plain\nUTF8_STRING\n";
        assert_eq!(
            preferred_mime(types).as_deref(),
            Some("text/plain;charset=utf-8")
        );
    }

    #[test]
    fn falls_back_through_the_text_aliases() {
        assert_eq!(
            preferred_mime("application/x-foo\nUTF8_STRING\n").as_deref(),
            Some("UTF8_STRING")
        );
        assert_eq!(
            preferred_mime("text/plain;charset=iso-8859-1\nimage/png").as_deref(),
            Some("text/plain;charset=iso-8859-1")
        );
    }

    #[test]
    fn keeps_the_first_type_when_nothing_is_text() {
        assert_eq!(
            preferred_mime("image/png\nimage/jpeg\n").as_deref(),
            Some("image/png")
        );
        assert_eq!(preferred_mime("\n  \n"), None);
    }

    #[test]
    fn tells_an_empty_clipboard_from_a_failure() {
        assert!(wl_paste_says_empty("Nothing is copied\n"));
        assert!(wl_paste_says_empty("No selection"));
        assert!(!wl_paste_says_empty(
            "Failed to connect to a Wayland server: Broken pipe"
        ));
    }

    fn content(data: &str) -> ClipboardContent {
        ClipboardContent {
            data: data.as_bytes().to_vec(),
            mime_type: "text/plain".into(),
        }
    }

    #[test]
    fn a_superseded_restore_hands_its_original_to_the_next_paste() {
        let merged = merge_pending(
            Snapshot::Saved(content("first transcript")),
            Some((content("user's text"), b"first transcript".to_vec())),
        );
        assert_eq!(merged, Snapshot::Saved(content("user's text")));
    }

    #[test]
    fn a_copy_made_between_pastes_is_what_gets_restored() {
        let merged = merge_pending(
            Snapshot::Saved(content("copied in between")),
            Some((content("user's text"), b"first transcript".to_vec())),
        );
        assert_eq!(merged, Snapshot::Saved(content("copied in between")));
    }

    #[test]
    fn a_failed_snapshot_stays_failed() {
        let merged = merge_pending(
            Snapshot::Failed("boom".into()),
            Some((content("user's text"), b"first transcript".to_vec())),
        );
        assert!(matches!(merged, Snapshot::Failed(_)));
    }
}
