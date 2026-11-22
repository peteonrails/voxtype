//! Helpers shared by unit tests.

use std::path::Path;

/// Set in the environment of the probe run [`write_script`] makes; a
/// script it writes exits at once when it sees it.
pub const PROBE_ENV: &str = "VOXTYPE_TEST_SCRIPT_PROBE";

/// Write an executable script that is safe to run straight away.
///
/// Writing a file and then executing it races with any other thread that
/// forks meanwhile: the child inherits the still-open write descriptor
/// (close-on-exec only closes it at the child's own exec), and while it
/// holds it, executing the file fails with ETXTBSY. Renaming does not help,
/// since the inode is the same. So after writing, this runs the script
/// once, with [`PROBE_ENV`] set so it exits immediately, retrying while the
/// kernel says ETXTBSY. Once one exec succeeds no writer remains, and no
/// later fork can pick one up.
///
/// `contents` must start with a `#!` line for a POSIX shell or Python; the
/// probe check goes right after it.
pub fn write_script(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    let (shebang, body) = contents
        .split_once('\n')
        .expect("a script starts with a #! line");
    assert!(shebang.starts_with("#!"), "a script starts with a #! line");
    let probe = if shebang.contains("python") {
        format!("import os, sys\nif os.environ.get(\"{PROBE_ENV}\"): sys.exit(0)\n")
    } else {
        format!("[ -n \"${PROBE_ENV}\" ] && exit 0\n")
    };
    std::fs::write(path, format!("{shebang}\n{probe}{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_executable(path);
}

/// Exec `path` (a script written by [`write_script`]) until the kernel stops
/// answering ETXTBSY.
pub fn wait_until_executable(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match std::process::Command::new(path)
            .env(PROBE_ENV, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "{} stayed busy",
                    path.display()
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            _ => return,
        }
    }
}

/// Run the calling test in a process of its own, for a test that depends
/// on process-wide state. `test` is its full path
/// (`module::tests::name`), and `prepare` sets up the child's command, its
/// environment for instance.
///
/// Returns true in the child, where the test goes on, and false in the
/// parent once the child has passed: `if !own_process(..) { return; }`.
pub fn own_process(test: &str, prepare: impl FnOnce(&mut std::process::Command)) -> bool {
    const CHILD: &str = "VOXTYPE_TEST_OWN_PROCESS";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return true;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, test);
    prepare(&mut command);
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "1 passed": a path that matches no test also exits successfully.
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{test} in its own process:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

/// What the code under test logs, as the daemon's own formatter writes it.
///
/// Installs a subscriber for the current thread that keeps events at
/// `level` and above (`INFO` is what the daemon and its systemd unit run
/// at) and collects the formatted lines. Dropping it restores the previous
/// subscriber. It follows the thread, so on a current-thread runtime it
/// also sees what is logged across `.await`s.
///
/// Use it from a test in its own process ([`own_process`]) whenever the
/// test asserts that something was logged. tracing caches, per call site,
/// whether any subscriber wants it. A call site first reached on another
/// test's thread while this is the only subscriber is cached as unwanted,
/// and the capture then never sees it.
pub struct LogCapture {
    lines: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    _guard: Option<tracing::subscriber::DefaultGuard>,
}

impl LogCapture {
    pub fn at(level: tracing::Level) -> Self {
        let (lines, subscriber) = Self::subscriber(level);
        Self {
            lines,
            _guard: Some(tracing::subscriber::set_default(subscriber)),
        }
    }

    /// The same for every thread of the process, for code that logs from a
    /// thread of its own (the history recorder writes on a blocking one).
    /// It is the process's subscriber from here on, so only a test in its
    /// own process ([`own_process`]) can use it, and only once.
    pub fn for_process(level: tracing::Level) -> Self {
        let (lines, subscriber) = Self::subscriber(level);
        tracing::subscriber::set_global_default(subscriber)
            .expect("this process already has a subscriber");
        Self {
            lines,
            _guard: None,
        }
    }

    fn subscriber(
        level: tracing::Level,
    ) -> (
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        impl tracing::Subscriber + Send + Sync + 'static,
    ) {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = CaptureWriter(lines.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || sink.clone())
            .with_max_level(level)
            .with_ansi(false)
            .finish();
        (lines, subscriber)
    }

    /// Everything logged so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.lines.lock().unwrap()).into_owned()
    }
}

#[derive(Clone)]
struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Forks constantly on other threads while writing and running scripts:
    /// the conditions of a full parallel `cargo test`. Returns how many runs
    /// failed with ETXTBSY.
    fn hammer(rounds: usize, safe: bool) -> usize {
        let stop = Arc::new(AtomicBool::new(false));
        let forkers: Vec<_> = (0..6)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = std::process::Command::new("/bin/true").status();
                    }
                })
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let mut busy = 0;
        for i in 0..rounds {
            let path = dir.path().join(format!("s{i}"));
            if safe {
                write_script(&path, "#!/bin/sh\nexit 0\n");
            } else {
                use std::os::unix::fs::PermissionsExt;
                std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            if let Err(e) = std::process::Command::new(&path).status() {
                assert_eq!(e.raw_os_error(), Some(libc::ETXTBSY), "{e}");
                busy += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        for f in forkers {
            f.join().unwrap();
        }
        busy
    }

    #[test]
    fn written_scripts_run_at_once_while_other_threads_fork() {
        assert_eq!(hammer(200, true), 0);
    }

    /// The race itself, for the record:
    /// `cargo test --lib test_support -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn naive_write_then_exec_hits_etxtbsy() {
        let busy = hammer(2000, false);
        eprintln!("naive write-then-exec: {busy}/2000 runs failed with ETXTBSY");
        let safe = hammer(2000, true);
        eprintln!("write_script: {safe}/2000 runs failed with ETXTBSY");
        assert_eq!(safe, 0);
    }
}
