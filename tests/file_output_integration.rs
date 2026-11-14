//! Regression tests for file-mode output chains.
//!
//! A streaming session builds its delivery chain before any final transcript
//! arrives. File mode must therefore select a file sink there instead of
//! silently falling back to the clipboard.

use std::path::Path;

use voxtype::config::{FileMode, OutputConfig, OutputMode};
use voxtype::output::create_output_chain;

fn file_config(path: &Path) -> OutputConfig {
    OutputConfig {
        mode: OutputMode::File,
        file_path: Some(path.to_path_buf()),
        file_mode: FileMode::Overwrite,
        ..OutputConfig::default()
    }
}

#[test]
fn file_mode_builds_a_file_chain() {
    let directory = tempfile::tempdir().unwrap();
    let chain = create_output_chain(&file_config(&directory.path().join("transcript.txt")));

    assert_eq!(chain.first().map(|output| output.name()), Some("file"));
}

#[tokio::test]
async fn file_chain_writes_a_streaming_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transcript.txt");
    let chain = create_output_chain(&file_config(&path));

    chain[0].output("first segment").await.unwrap();
    chain[0].output("second segment").await.unwrap();

    assert_eq!(
        tokio::fs::read_to_string(path).await.unwrap(),
        "first segment\nsecond segment\n"
    );
}

#[test]
fn file_mode_without_a_path_falls_back_to_an_available_output() {
    let chain = create_output_chain(&OutputConfig {
        mode: OutputMode::File,
        ..OutputConfig::default()
    });

    assert_ne!(chain.first().map(|output| output.name()), Some("file"));
}
