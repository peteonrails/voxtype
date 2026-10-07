//! Clipboard-free Linux keyboard output. Text is fully planned before a device
//! is opened; the configured keymap must also be assigned to this device by the
//! desktop. No compositor or application can acknowledge text insertion here.

use super::TextOutput;
use crate::config::OutputConfig;
use crate::error::OutputError;

#[cfg(feature = "uinput")]
mod native;

pub const DEVICE_NAME: &str = "Voxtype virtual keyboard";

pub struct UinputOutput {
    #[cfg(feature = "uinput")]
    config: OutputConfig,
    reason: Option<String>,
    #[cfg(feature = "uinput")]
    device: std::sync::Arc<std::sync::Mutex<Option<evdev::uinput::VirtualDevice>>>,
}

impl UinputOutput {
    pub fn new(config: &OutputConfig, reason: Option<String>) -> Self {
        #[cfg(not(feature = "uinput"))]
        let _ = config;
        Self {
            #[cfg(feature = "uinput")]
            config: config.clone(),
            reason,
            #[cfg(feature = "uinput")]
            device: native::device(),
        }
    }
}

#[async_trait::async_trait]
impl TextOutput for UinputOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if let Some(reason) = &self.reason {
            return Err(OutputError::KeyboardStopped(reason.clone()));
        }
        #[cfg(feature = "uinput")]
        {
            native::output(
                self.config.clone(),
                self.device.clone(),
                text.to_owned(),
                None,
            )
            .await
        }
        #[cfg(not(feature = "uinput"))]
        {
            let _ = text;
            Err(OutputError::KeyboardStopped(
                "this binary lacks native keyboard support; rebuild with --features uinput".into(),
            ))
        }
    }

    async fn is_available(&self) -> bool {
        // Keep the explicit choice in the chain so setup errors are reported,
        // rather than silently selecting another output method.
        true
    }

    fn name(&self) -> &'static str {
        "uinput"
    }

    fn normalize_quotes(&self) -> bool {
        false
    }

    async fn backspace(&self, count: usize) -> Option<Result<usize, OutputError>> {
        if let Some(reason) = &self.reason {
            return Some(Err(OutputError::KeyboardStopped(reason.clone())));
        }
        #[cfg(feature = "uinput")]
        return Some(
            native::output(
                self.config.clone(),
                self.device.clone(),
                String::new(),
                Some(count),
            )
            .await
            .map(|()| count),
        );
        #[cfg(not(feature = "uinput"))]
        Some(self.output("").await.map(|()| count))
    }
}
