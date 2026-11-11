//! Helpers shared by integration tests. Each test file that needs them says
//! `mod common;`.

#![allow(dead_code)]

use std::path::Path;

/// Set in the environment of the probe run [`write_script`] makes; a script
/// it writes exits at once when it sees it.
pub const PROBE_ENV: &str = "VOXTYPE_TEST_SCRIPT_PROBE";

/// Write an executable script that is safe to run straight away.
///
/// Another test thread forking while the file is open for writing hands
/// the write descriptor to its child until the child execs, and executing
/// the file meanwhile fails with ETXTBSY. So after writing, run the script
/// once with [`PROBE_ENV`] set (it exits immediately), retrying while the
/// kernel says ETXTBSY; once one exec succeeds, no writer is left. Same as
/// `src/test_support.rs`, which unit tests use.
pub fn write_script(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    let (shebang, body) = contents
        .split_once('\n')
        .expect("a script starts with a #! line");
    assert!(shebang.starts_with("#!"), "a script starts with a #! line");
    std::fs::write(
        path,
        format!("{shebang}\n[ -n \"${PROBE_ENV}\" ] && exit 0\n{body}"),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match std::process::Command::new(path)
            .env(PROBE_ENV, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Err(e) if e.raw_os_error() == Some(26) => {
                // 26 is ETXTBSY on Linux.
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
