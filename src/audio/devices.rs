//! Input-device enumeration shared by the TUI's audio section and
//! `voxtype info devices --json`.
//!
//! Extracted from `src/tui/audio.rs` so both surfaces list the same devices
//! and both get the stderr silencing: ALSA's PCM probing writes "Cannot open
//! device /dev/dsp" and similar to stderr for every device cpal touches.
//! Inside the TUI's alternate screen those lines paint over the frame; in
//! `--json` output they'd be noise interleaved with the payload.

use crate::error::AudioError;
use cpal::traits::{DeviceTrait, HostTrait};
use std::time::Duration;

/// How long listing or opening input devices may take before voxtype gives
/// up on the sound server.
pub const SOUND_SERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// Run `f` on a thread of its own and wait up to `timeout` for its result.
///
/// ALSA's PipeWire and PulseAudio plugins wait in epoll for the server's
/// reply, forever, when its socket exists but nothing answers: a wedged
/// PipeWire, or the sandboxes and clean chroots that builds and tests run
/// in. A blocked ALSA call cannot be cancelled, so a thread that never
/// returns is left behind; it costs one idle thread, and process exit ends
/// it. `None` means the time ran out.
pub fn bounded<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    match std::thread::Builder::new()
        .name("sound-server-query".into())
        .spawn(move || {
            let _ = tx.send(f());
        }) {
        Ok(_) => rx.recv_timeout(timeout).ok(),
        // No thread to spare: treat it as no answer.
        Err(_) => None,
    }
}

/// [`bounded`] for async callers, which must not block their runtime while
/// they wait.
pub async fn bounded_async<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("sound-server-query".into())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .ok()?;
    tokio::time::timeout(timeout, rx).await.ok()?.ok()
}

/// A capture device offered to the user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InputDevice {
    pub name: String,
    /// True for the synthetic `default` entry, which follows whatever the
    /// system default is at record time.
    pub default: bool,
}

/// Capture device names, with the synthetic `default` entry first. Fails
/// with [`AudioError::NoSoundServer`] when the sound server does not answer
/// within [`SOUND_SERVER_TIMEOUT`].
///
/// The listing runs in a short-lived child (`voxtype configure
/// --probe-audio-devices`), not a thread: a thread stuck in ALSA also keeps
/// this process from exiting, because libasound's and PipeWire's exit-time
/// cleanup waits on the lock the stuck call holds. A child is simply killed.
pub fn enumerate_input_devices() -> Result<Vec<String>, AudioError> {
    match std::env::current_exe() {
        Ok(exe) => probe_input_devices(&exe, SOUND_SERVER_TIMEOUT),
        Err(_) => bounded(SOUND_SERVER_TIMEOUT, list_input_device_names)
            .ok_or(AudioError::NoSoundServer(SOUND_SERVER_TIMEOUT.as_secs())),
    }
}

/// Run `exe configure --probe-audio-devices` and read its list, killing it
/// after `timeout`.
pub fn probe_input_devices(
    exe: &std::path::Path,
    timeout: Duration,
) -> Result<Vec<String>, AudioError> {
    use std::io::Read;
    let mut child = std::process::Command::new(exe)
        .args(["configure", "--probe-audio-devices"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| AudioError::Connection(format!("could not list devices: {e}")))?;

    let deadline = std::time::Instant::now() + timeout;
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AudioError::NoSoundServer(timeout.as_secs()));
            }
        }
    };
    if !success {
        return Err(AudioError::Connection(
            "listing input devices failed (libasound crashed or exited early)".into(),
        ));
    }
    // The child has exited, so this read is bounded; the list is far smaller
    // than the pipe buffer, so the child never blocked writing it.
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// The listing itself, unbounded: only for the probe child, which its
/// parent kills if it hangs.
pub fn list_input_device_names() -> Vec<String> {
    let _silenced = SilencedStderr::install();
    let mut out = vec!["default".to_string()];
    let host = cpal::default_host();
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                if name != "default" && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// Same list as [`enumerate_input_devices`], tagged for JSON output.
pub fn input_devices() -> Result<Vec<InputDevice>, AudioError> {
    Ok(enumerate_input_devices()?
        .into_iter()
        .map(|name| InputDevice {
            default: name == "default",
            name,
        })
        .collect())
}

/// RAII guard that redirects fd 2 (stderr) to /dev/null on construction and
/// restores the original fd on drop.
struct SilencedStderr {
    saved_fd: Option<libc::c_int>,
}

impl SilencedStderr {
    fn install() -> Self {
        let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY) };
        if null_fd < 0 {
            return Self { saved_fd: None };
        }
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved < 0 {
            unsafe { libc::close(null_fd) };
            return Self { saved_fd: None };
        }
        unsafe { libc::dup2(null_fd, libc::STDERR_FILENO) };
        unsafe { libc::close(null_fd) };
        Self {
            saved_fd: Some(saved),
        }
    }
}

impl Drop for SilencedStderr {
    fn drop(&mut self) {
        if let Some(saved) = self.saved_fd.take() {
            unsafe {
                libc::dup2(saved, libc::STDERR_FILENO);
                libc::close(saved);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn write_script(path: &std::path::Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn a_wait_that_never_ends_gives_up_at_the_deadline() {
        let started = Instant::now();
        let got = bounded(Duration::from_millis(200), || loop {
            std::thread::park();
        });
        assert_eq!(got, None::<()>);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(bounded(Duration::from_secs(5), || 7), Some(7));
    }

    /// A probe that hangs is killed at the deadline and reported as no
    /// sound server; one that answers is read. `sh -c` stands in for
    /// `voxtype configure --probe-audio-devices`.
    #[test]
    fn the_probe_child_is_killed_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let hang = dir.path().join("hang");
        write_script(&hang, "#!/bin/sh\nexec sleep 30\n");
        let started = Instant::now();
        let err = probe_input_devices(&hang, Duration::from_millis(300)).unwrap_err();
        assert!(matches!(err, AudioError::NoSoundServer(0)), "{err}");
        assert!(
            err.to_string().contains("No sound server answered"),
            "{err}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));

        let answer = dir.path().join("answer");
        write_script(&answer, "#!/bin/sh\nprintf 'default\\nUSB mic\\n'\n");
        assert_eq!(
            probe_input_devices(&answer, Duration::from_secs(5)).unwrap(),
            vec!["default", "USB mic"]
        );
    }

    #[tokio::test]
    async fn the_async_wait_gives_up_without_blocking_the_runtime() {
        let started = Instant::now();
        let stuck = bounded_async(Duration::from_millis(200), || loop {
            std::thread::park();
        });
        let ticker = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            "runtime kept running"
        };
        let (stuck, ticked) = tokio::join!(stuck, ticker);
        assert_eq!(stuck, None::<()>);
        assert_eq!(ticked, "runtime kept running");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(bounded_async(Duration::from_secs(5), || 7).await, Some(7));
    }
}
