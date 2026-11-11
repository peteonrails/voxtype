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
