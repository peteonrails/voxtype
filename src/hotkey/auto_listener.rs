//! Hotkey listener that prefers the XDG GlobalShortcuts portal and uses evdev
//! when the portal cannot be reached.

use super::evdev_listener::EvdevListener;
use super::portal_listener::{notify_permanent_failure, OnPermanentFailure, PortalListener};
use super::{HotkeyEvent, HotkeyListener};
use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use async_trait::async_trait;
use std::collections::HashSet;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// The evdev listener this backend starts when the portal is unreachable.
struct EvdevFallback {
    config: HotkeyConfig,
    secondary_model: Option<String>,
}

impl EvdevFallback {
    async fn start(&self) -> Result<(EvdevListener, mpsc::Receiver<HotkeyEvent>), HotkeyError> {
        let mut listener = EvdevListener::new(&self.config)?;
        listener.set_secondary_model(self.secondary_model.clone());
        let events = listener.start().await?;

        Ok((listener, events))
    }
}

/// What to do once the portal listener has given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterPortal {
    /// Start the evdev listener. The portal was never reachable, so the user
    /// was not asked to approve anything.
    FallBackToEvdev,
    /// Report the failure. The desktop answered, and starting evdev would take
    /// the raw keyboard access that answer withheld.
    ReportFailure,
    /// Do nothing. The portal listener stopped without reporting a failure,
    /// which is what `stop` looks like from here.
    Stop,
}

fn after_portal(failure: Option<&HotkeyError>) -> AfterPortal {
    match failure {
        None => AfterPortal::Stop,
        Some(error) if error.allows_evdev_fallback() => AfterPortal::FallBackToEvdev,
        Some(_) => AfterPortal::ReportFailure,
    }
}

/// Receives global shortcut events from whichever backend is available.
pub(crate) struct AutoListener {
    config: HotkeyConfig,
    secondary_model: Option<String>,
    profiles: HashSet<String>,
    stop_signal: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl AutoListener {
    pub(crate) fn new(
        config: &HotkeyConfig,
        secondary_model: Option<String>,
        profiles: &HashSet<String>,
    ) -> Self {
        Self {
            config: config.clone(),
            secondary_model,
            profiles: profiles.clone(),
            stop_signal: None,
            task: None,
        }
    }
}

#[async_trait]
impl HotkeyListener for AutoListener {
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError> {
        let (failure_tx, failure_rx) = oneshot::channel();
        let mut portal = PortalListener::new(
            &self.config,
            self.secondary_model.clone(),
            &self.profiles,
            OnPermanentFailure::Delegate(failure_tx),
        );
        let portal_rx = portal.start().await?;
        let (event_tx, event_rx) = mpsc::channel(32);
        let (stop_tx, stop_rx) = oneshot::channel();
        let fallback = EvdevFallback {
            config: self.config.clone(),
            secondary_model: self.secondary_model.clone(),
        };

        self.stop_signal = Some(stop_tx);
        self.task = Some(tokio::spawn(async move {
            run_auto(fallback, portal, portal_rx, failure_rx, event_tx, stop_rx).await;
        }));

        Ok(event_rx)
    }

    async fn stop(&mut self) -> Result<(), HotkeyError> {
        if let Some(stop) = self.stop_signal.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }

        Ok(())
    }
}

/// Forwards the portal listener's events to the daemon, and starts evdev if the
/// portal listener gives up for a reason that permits it.
///
/// The daemon holds one receiver for the life of the listener, so both backends
/// send through the same channel.
async fn run_auto(
    fallback: EvdevFallback,
    mut portal: PortalListener,
    mut portal_rx: mpsc::Receiver<HotkeyEvent>,
    mut failure_rx: oneshot::Receiver<HotkeyError>,
    event_tx: mpsc::Sender<HotkeyEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let failure = loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx => {
                let _ = portal.stop().await;
                return;
            }
            failure = &mut failure_rx => break failure.ok(),
            event = portal_rx.recv() => {
                let Some(event) = event else { break None };
                if event_tx.send(event).await.is_err() {
                    let _ = portal.stop().await;
                    return;
                }
            }
        }
    };
    let _ = portal.stop().await;

    // The portal listener sends a release for a held shortcut before it gives
    // up. Drain those events, or the daemon keeps recording.
    while let Some(event) = portal_rx.recv().await {
        if event_tx.send(event).await.is_err() {
            return;
        }
    }

    match after_portal(failure.as_ref()) {
        AfterPortal::Stop => return,
        AfterPortal::ReportFailure => {
            if let Some(error) = &failure {
                notify_permanent_failure(error).await;
            }
            return;
        }
        AfterPortal::FallBackToEvdev => {}
    }

    if let Some(error) = &failure {
        tracing::warn!(
            "XDG GlobalShortcuts is unavailable, falling back to evdev: {}",
            error
        );
    }
    let (mut evdev, mut evdev_rx) = match fallback.start().await {
        Ok(started) => started,
        Err(error) => return notify_permanent_failure(&error).await,
    };

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx => break,
            event = evdev_rx.recv() => {
                let Some(event) = event else { break };
                if event_tx.send(event).await.is_err() {
                    break;
                }
            }
        }
    }

    let _ = evdev.stop().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable_portal() -> HotkeyError {
        HotkeyError::PortalUnavailable(zbus::Error::Unsupported)
    }

    #[test]
    fn an_unreachable_portal_falls_back_to_evdev() {
        let decisions = [
            after_portal(Some(&unreachable_portal())),
            after_portal(Some(&HotkeyError::PortalRegistration(
                zbus::Error::Unsupported,
            ))),
        ];

        assert_eq!(
            decisions,
            [AfterPortal::FallBackToEvdev, AfterPortal::FallBackToEvdev]
        );
    }

    #[test]
    fn a_refused_binding_does_not_fall_back_to_evdev() {
        let decisions = [
            after_portal(Some(&HotkeyError::PortalCancelled)),
            after_portal(Some(&HotkeyError::PortalResponse(2))),
            after_portal(Some(&HotkeyError::PortalMissingRequired(
                "dictate".to_string(),
            ))),
        ];

        assert_eq!(
            decisions,
            [
                AfterPortal::ReportFailure,
                AfterPortal::ReportFailure,
                AfterPortal::ReportFailure
            ]
        );
    }

    #[test]
    fn a_portal_listener_that_stopped_starts_nothing_else() {
        assert_eq!(after_portal(None), AfterPortal::Stop);
    }

    #[tokio::test]
    async fn stopping_before_starting_is_not_an_error() {
        let mut listener = AutoListener::new(&HotkeyConfig::default(), None, &HashSet::new());

        assert!(listener.stop().await.is_ok());
    }
}
