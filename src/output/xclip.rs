//! X11 clipboard output
//!
//! Copies text to the X11 CLIPBOARD selection using `xclip` (preferred) or
//! `xsel` (fallback). Activates only under an X11 session; Wayland sessions
//! are handled by `ClipboardOutput` (wl-copy).
//!
//! See GitHub issue #346 for the original report: under XLibre/X11 sessions
//! voxtype was invoking wl-copy unconditionally, leaving the clipboard
//! untouched.
//!
//! Requires one of: `xclip` or `xsel` installed.

use super::session::{detect, DisplaySession};
use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// X11 clipboard output (xclip with xsel fallback)
pub struct XclipOutput {
    /// Text to append after transcription
    append_text: Option<String>,
}

impl XclipOutput {
    /// Create a new X11 clipboard output
    pub fn new(append_text: Option<String>) -> Self {
        Self { append_text }
    }
}

/// Which X11 clipboard tool to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum X11ClipboardTool {
    Xclip,
    Xsel,
}

impl X11ClipboardTool {
    fn command(self) -> &'static str {
        match self {
            X11ClipboardTool::Xclip => "xclip",
            X11ClipboardTool::Xsel => "xsel",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            X11ClipboardTool::Xclip => &["-selection", "clipboard"],
            X11ClipboardTool::Xsel => &["--clipboard", "--input"],
        }
    }
}

/// Probe `which $cmd` to see if a binary is on PATH.
async fn binary_on_path(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Find an installed X11 clipboard tool, preferring xclip.
async fn find_tool() -> Option<X11ClipboardTool> {
    if binary_on_path("xclip").await {
        return Some(X11ClipboardTool::Xclip);
    }
    if binary_on_path("xsel").await {
        return Some(X11ClipboardTool::Xsel);
    }
    None
}

/// Run an X11 clipboard tool, piping `text` to its stdin.
async fn copy_via(tool: X11ClipboardTool, text: &[u8]) -> Result<(), OutputError> {
    let mut child = Command::new(tool.command())
        .args(tool.args())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                match tool {
                    X11ClipboardTool::Xclip => OutputError::XclipNotFound,
                    X11ClipboardTool::Xsel => OutputError::X11ClipboardToolMissing,
                }
            } else {
                OutputError::InjectionFailed(e.to_string())
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text)
            .await
            .map_err(|e| OutputError::InjectionFailed(e.to_string()))?;
        drop(stdin);
    }

    let status = child
        .wait()
        .await
        .map_err(|e| OutputError::InjectionFailed(e.to_string()))?;

    if !status.success() {
        return Err(OutputError::InjectionFailed(format!(
            "{} exited with error",
            tool.command()
        )));
    }

    Ok(())
}

/// Public helper: copy `text` to the X11 clipboard, trying xclip then xsel.
///
/// Returns `OutputError::X11ClipboardToolMissing` if neither tool is on PATH.
/// Used by both `XclipOutput` and `PasteOutput` so they share the same
/// dispatch logic.
pub(crate) async fn copy_to_x11_clipboard(text: &[u8]) -> Result<(), OutputError> {
    let tool = find_tool()
        .await
        .ok_or(OutputError::X11ClipboardToolMissing)?;
    tracing::debug!("Using {} for X11 clipboard", tool.command());
    copy_via(tool, text).await
}

#[async_trait::async_trait]
impl TextOutput for XclipOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        let text = if let Some(ref append) = self.append_text {
            std::borrow::Cow::Owned(format!("{}{}", text, append))
        } else {
            std::borrow::Cow::Borrowed(text)
        };

        copy_to_x11_clipboard(text.as_bytes()).await?;

        tracing::info!("Text copied to X11 clipboard ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Only available under an X11 session with at least one tool installed.
        if detect() != DisplaySession::X11 {
            tracing::debug!("clipboard (xclip/xsel) skipped: not an X11 session");
            return false;
        }
        find_tool().await.is_some()
    }

    fn name(&self) -> &'static str {
        "clipboard (xclip/xsel)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = XclipOutput::new(None);
        assert!(output.append_text.is_none());

        let output = XclipOutput::new(Some(" ".to_string()));
        assert_eq!(output.append_text, Some(" ".to_string()));
    }

    #[test]
    fn test_tool_command_and_args() {
        assert_eq!(X11ClipboardTool::Xclip.command(), "xclip");
        assert_eq!(X11ClipboardTool::Xclip.args(), &["-selection", "clipboard"]);
        assert_eq!(X11ClipboardTool::Xsel.command(), "xsel");
        assert_eq!(X11ClipboardTool::Xsel.args(), &["--clipboard", "--input"]);
    }
}
