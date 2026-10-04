//! One-shot CLI commands terminate with an exit status, never a signal.
//!
//! On the ONNX variants `voxtype transcribe <file>` printed its result and then
//! aborted inside static destructions teardown: `corrupted double-linked list`,
//! exit 134, and a core dump per run (#772). The one-shot exit path now leaves
//! the process without running static destructors, the same way the daemon does.
//!
//! Reproducing the abort needs the CUDA build, the ONNX Runtime libraries, and a
//! model, so this test pins the observable contract that CI can check: a
//! one-shot run reaches the subcommand, reports its failure, and returns an
//! exit code instead of dying from a signal. The packaged-binary reproduction
//! stays a manual smoke test (see `docs/smoke_tests/one-shot-exit-status.md`).

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

fn voxtype() -> Command {
    Command::new(env!("CARGO_BIN_EXE_voxtype"))
}

/// A `transcribe` run that fails before any engine loads still leaves through
/// the one-shot exit path, so it must report a status rather than a signal.
#[test]
fn transcribe_reaches_the_command_and_exits_with_a_code() {
    let output = voxtype()
        .args(["transcribe", "/nonexistent/voxtype-one-shot-exit.wav"])
        .output()
        .expect("spawn voxtype transcribe");

    assert_eq!(
        output.status.signal(),
        None,
        "one-shot command died from a signal instead of exiting"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a missing input file is an error, not a success"
    );

    // Proves the subcommand ran: it announces the file before it opens it.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Loading audio file"),
        "transcribe did not reach the file handler, stdout was: {stdout}"
    );

    // Proves the failure was reported rather than swallowed.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error:"),
        "transcribe failed without reporting an error, stderr was: {stderr}"
    );
}
