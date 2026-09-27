//! The completion sidecar, and the atomic file write it reports on.
//!
//! The daemon's control surface is fire-and-forget: a client that asked for file
//! output has no way to learn that the recording finished, so it has to poll the
//! transcript until its own deadline expires. When no speech is detected nothing
//! is ever written and that deadline is the only thing that ends the wait,
//! reported to the user as a timeout, which it is not. The sidecar is the
//! missing completion signal: exactly one is written per file-mode recording,
//! and `voxtype record stop --wait` blocks on it.

use crate::config::FileMode;

/// How a file-mode recording ended.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TranscriptOutcome {
    /// `ok`, `empty`, or `error`.
    pub status: String,
    /// Characters written. Zero for `empty` and `error`.
    pub chars: usize,
    /// Present only for `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl TranscriptOutcome {
    pub fn ok(chars: usize) -> Self {
        Self {
            status: "ok".to_string(),
            chars,
            message: None,
        }
    }

    /// No speech survived voice-activity detection, so nothing was transcribed.
    pub fn empty() -> Self {
        Self {
            status: "empty".to_string(),
            chars: 0,
            message: None,
        }
    }

    pub fn error(message: &str) -> Self {
        Self {
            status: "error".to_string(),
            chars: 0,
            message: Some(message.to_string()),
        }
    }
}

/// Path of the completion sidecar for a transcript.
pub fn result_sidecar_path(transcript: &std::path::Path) -> std::path::PathBuf {
    let mut sidecar = transcript.as_os_str().to_os_string();
    sidecar.push(".done");
    std::path::PathBuf::from(sidecar)
}

/// Publish `outcome` beside `transcript`, atomically and last.
///
/// Written after the transcript itself so a client that sees the sidecar can
/// read a complete transcript. Failure to write it is logged and otherwise
/// ignored: the transcription already succeeded, and a client that misses the
/// signal falls back to its own timeout.
pub(crate) fn write_result_sidecar(transcript: &std::path::Path, outcome: &TranscriptOutcome) {
    let sidecar = result_sidecar_path(transcript);
    let body = match serde_json::to_string(outcome) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to encode transcript outcome: {}", e);
            return;
        }
    };
    let staged = temp_sibling(&sidecar);
    if let Err(e) = std::fs::write(&staged, format!("{}\n", body)) {
        tracing::warn!("Failed to stage transcript outcome {:?}: {}", staged, e);
        return;
    }
    if let Err(e) = std::fs::rename(&staged, &sidecar) {
        tracing::warn!("Failed to publish transcript outcome {:?}: {}", sidecar, e);
        let _ = std::fs::remove_file(&staged);
    }
}

/// Sibling temporary path used to stage an atomic transcript write.
///
/// Kept in the same directory as the target so the rename stays within one
/// filesystem. The pid keeps two daemons from colliding on it.
pub(crate) fn temp_sibling(path: &std::path::Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "transcription".to_string());
    let mut staged = path.to_path_buf();
    staged.set_file_name(format!(".{}.{}.tmp", name, std::process::id()));
    staged
}

/// Write transcription to a file, respecting file_mode (overwrite or append)
pub(crate) async fn write_transcription_to_file(
    path: &std::path::Path,
    text: &str,
    file_mode: &FileMode,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // Ensure text ends with newline
    let output_text = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{}\n", text)
    };

    match file_mode {
        FileMode::Overwrite => {
            // Write through a sibling temporary file and rename, so a reader
            // polling for a non-empty transcript can never observe a partial
            // one. Programmatic consumers (OmaPilot, agent harnesses) return
            // the first non-empty read they get; a truncate-then-write would
            // hand them a half-written transcript.
            let temporary = temp_sibling(path);
            tokio::fs::write(&temporary, output_text).await?;
            if let Err(e) = tokio::fs::rename(&temporary, path).await {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(e);
            }
        }
        FileMode::Append => {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            file.write_all(output_text.as_bytes()).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn overwrite_is_atomic_from_a_readers_point_of_view() {
        // A reader that polls for a non-empty transcript must never observe a
        // partial one, so the staged file must not be the target path and the
        // target must appear complete in one step.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");
        let staged = temp_sibling(&target);
        assert_ne!(staged, target);
        assert_eq!(staged.parent(), target.parent());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(write_transcription_to_file(
                &target,
                "the quick brown fox",
                &FileMode::Overwrite,
            ))
            .unwrap();

        assert_eq!(
            fs::read_to_string(&target).unwrap().trim_end(),
            "the quick brown fox"
        );
        assert!(!staged.exists(), "staging file must not survive the write");
    }

    #[test]
    fn overwrite_replaces_previous_transcript() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for text in ["first pass", "second"] {
            runtime
                .block_on(write_transcription_to_file(
                    &target,
                    text,
                    &FileMode::Overwrite,
                ))
                .unwrap();
        }
        assert_eq!(fs::read_to_string(&target).unwrap().trim_end(), "second");
    }

    #[test]
    fn sidecar_sits_beside_the_transcript() {
        assert_eq!(
            result_sidecar_path(std::path::Path::new("/run/user/1000/x/dictation.txt")),
            std::path::PathBuf::from("/run/user/1000/x/dictation.txt.done")
        );
    }

    #[test]
    fn sidecar_reports_a_terminal_outcome() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");

        write_result_sidecar(&target, &TranscriptOutcome::ok(19));
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["chars"], 19);
        assert!(parsed.get("message").is_none(), "ok carries no message");

        write_result_sidecar(&target, &TranscriptOutcome::empty());
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "empty");
        assert_eq!(parsed["chars"], 0);

        write_result_sidecar(&target, &TranscriptOutcome::error("disk went away"));
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["message"], "disk went away");
    }
}
