//! kwtype-based text output for KDE Plasma Wayland

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// KDE Plasma Wayland text output via KWin's Fake Input protocol.
pub struct KwtypeOutput {
    auto_submit: bool,
    append_text: Option<String>,
    pre_type_delay_ms: u32,
}

impl KwtypeOutput {
    pub fn new(auto_submit: bool, append_text: Option<String>, pre_type_delay_ms: u32) -> Self {
        Self {
            auto_submit,
            append_text,
            pre_type_delay_ms,
        }
    }

    fn is_kde_wayland() -> bool {
        let is_kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
            desktop
                .split(':')
                .any(|component| component.eq_ignore_ascii_case("kde"))
        });
        is_kde && super::session::detect() == super::session::DisplaySession::Wayland
    }
}

#[async_trait::async_trait]
impl TextOutput for KwtypeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        if self.pre_type_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.pre_type_delay_ms as u64,
            ))
            .await;
        }

        let mut text = text.to_owned();
        if let Some(append) = &self.append_text {
            text.push_str(append);
        }
        if self.auto_submit {
            text.push('\n');
        }

        let output = Command::new("kwtype")
            .arg("--")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    OutputError::KwtypeNotFound
                } else {
                    OutputError::InjectionFailed(error.to_string())
                }
            })?;

        if !output.status.success() {
            return Err(OutputError::InjectionFailed(format!(
                "kwtype failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(())
    }

    async fn is_available(&self) -> bool {
        if !Self::is_kde_wayland() {
            return false;
        }
        Command::new("which")
            .arg("kwtype")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success())
    }

    fn name(&self) -> &'static str {
        "kwtype"
    }
}
