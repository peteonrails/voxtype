//! File-backed text output.
//!
//! A [`FileOutput`] instance belongs to one output chain. Its first write
//! follows the configured file mode; later writes append so finalized
//! streaming segments form one transcript.

use std::path::PathBuf;

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::config::FileMode;
use crate::error::OutputError;

use super::TextOutput;

/// Writes transcribed text to a file.
pub struct FileOutput {
    path: PathBuf,
    mode: FileMode,
    written: Mutex<bool>,
}

impl FileOutput {
    pub fn new(path: PathBuf, mode: FileMode) -> Self {
        Self {
            path,
            mode,
            written: Mutex::new(false),
        }
    }

    fn temporary_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "transcription".to_string());
        let mut temporary = self.path.clone();
        temporary.set_file_name(format!(".{name}.{}.tmp", std::process::id()));
        temporary
    }

    async fn write_first_transcript(&self, text: &str) -> Result<(), OutputError> {
        match &self.mode {
            FileMode::Overwrite => {
                let temporary = self.temporary_path();
                tokio::fs::write(&temporary, text).await.map_err(|error| {
                    OutputError::InjectionFailed(format!(
                        "failed to write {}: {error}",
                        self.path.display()
                    ))
                })?;
                if let Err(error) = tokio::fs::rename(&temporary, &self.path).await {
                    let _ = tokio::fs::remove_file(&temporary).await;
                    return Err(OutputError::InjectionFailed(format!(
                        "failed to write {}: {error}",
                        self.path.display()
                    )));
                }
            }
            FileMode::Append => self.append(text).await?,
        }
        Ok(())
    }

    async fn append(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|error| {
                OutputError::InjectionFailed(format!(
                    "failed to write {}: {error}",
                    self.path.display()
                ))
            })?;
        file.write_all(text.as_bytes()).await.map_err(|error| {
            OutputError::InjectionFailed(format!(
                "failed to write {}: {error}",
                self.path.display()
            ))
        })?;
        file.flush().await.map_err(|error| {
            OutputError::InjectionFailed(format!(
                "failed to write {}: {error}",
                self.path.display()
            ))
        })
    }
}

#[async_trait::async_trait]
impl TextOutput for FileOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                OutputError::InjectionFailed(format!(
                    "failed to create {}: {error}",
                    parent.display()
                ))
            })?;
        }

        let text = if text.is_empty() || text.ends_with('\n') {
            text.to_owned()
        } else {
            format!("{text}\n")
        };
        let mut written = self.written.lock().await;
        if *written {
            self.append(&text).await?;
        } else {
            self.write_first_transcript(&text).await?;
            *written = true;
        }

        tracing::info!(path = %self.path.display(), "Wrote transcription to file");
        Ok(())
    }

    async fn is_available(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_a_transcript_with_a_trailing_newline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript.txt");
        let output = FileOutput::new(path.clone(), FileMode::Overwrite);

        output.output("hello there").await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(path).await.unwrap(),
            "hello there\n"
        );
    }

    #[tokio::test]
    async fn overwrite_mode_replaces_a_previous_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript.txt");
        tokio::fs::write(&path, "stale\n").await.unwrap();
        let output = FileOutput::new(path.clone(), FileMode::Overwrite);

        output.output("fresh").await.unwrap();

        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), "fresh\n");
    }

    #[tokio::test]
    async fn streaming_segments_accumulate_when_overwriting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript.txt");
        let output = FileOutput::new(path.clone(), FileMode::Overwrite);

        output.output("first").await.unwrap();
        output.output("second").await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(path).await.unwrap(),
            "first\nsecond\n"
        );
    }

    #[tokio::test]
    async fn append_mode_preserves_the_previous_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript.txt");
        tokio::fs::write(&path, "earlier\n").await.unwrap();
        let output = FileOutput::new(path.clone(), FileMode::Append);

        output.output("later").await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(path).await.unwrap(),
            "earlier\nlater\n"
        );
    }

    #[tokio::test]
    async fn creates_missing_parent_directories() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/transcript.txt");
        let output = FileOutput::new(path.clone(), FileMode::Overwrite);

        output.output("deep").await.unwrap();

        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), "deep\n");
    }
}
