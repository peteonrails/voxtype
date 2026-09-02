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
    mut portal: impl HotkeyListener,
    mut portal_rx: mpsc::Receiver<HotkeyEvent>,
    mut failure_rx: oneshot::Receiver<HotkeyError>,
    event_tx: mpsc::Sender<HotkeyEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let failure = tokio::select! {
        biased;
        _ = &mut stop_rx => {
            drop(portal_rx);
            let _ = portal.stop().await;
            return;
        }
        failure = &mut failure_rx => failure.ok(),
        _ = forward_events(&mut portal_rx, &event_tx) => {
            drop(portal_rx);
            let _ = portal.stop().await;
            return;
        },
    };

    // The portal can queue a release before reporting a permanent failure.
    let drained = tokio::select! {
        biased;
        _ = &mut stop_rx => false,
        _ = forward_events(&mut portal_rx, &event_tx) => true,
    };
    drop(portal_rx);
    let _ = portal.stop().await;
    if !drained {
        return;
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

    tokio::select! {
        biased;
        _ = &mut stop_rx => {},
        _ = forward_events(&mut evdev_rx, &event_tx) => {},
    }
    drop(evdev_rx);
    let _ = evdev.stop().await;
}

async fn forward_events(
    source: &mut mpsc::Receiver<HotkeyEvent>,
    destination: &mpsc::Sender<HotkeyEvent>,
) {
    loop {
        // A failure notification can cancel forwarding. Reserve before receiving
        // so cancellation cannot discard an event between the two channels.
        let Ok(permit) = destination.reserve().await else {
            return;
        };
        let Some(event) = source.recv().await else {
            return;
        };
        permit.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeListener {
        events: Option<mpsc::Sender<HotkeyEvent>>,
        stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl HotkeyListener for FakeListener {
        async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError> {
            unreachable!("the fake listener starts with an event channel")
        }

        async fn stop(&mut self) -> Result<(), HotkeyError> {
            self.events.take();
            self.stopped
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn fallback() -> EvdevFallback {
        EvdevFallback {
            config: HotkeyConfig::default(),
            secondary_model: None,
        }
    }

    #[tokio::test]
    async fn stop_interrupts_forwarding_and_failure_cleanup_with_a_full_queue() {
        for failed in [false, true] {
            let (portal_tx, portal_rx) = mpsc::channel(1);
            portal_tx
                .send(HotkeyEvent::Released)
                .await
                .expect("portal queue should be empty");
            let (event_tx, _event_rx) = mpsc::channel(1);
            event_tx
                .send(HotkeyEvent::Cancel)
                .await
                .expect("daemon queue should be empty");
            let (failure_tx, failure_rx) = oneshot::channel();
            let (stop_tx, stop_rx) = oneshot::channel();
            let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let portal = FakeListener {
                events: Some(portal_tx),
                stopped: stopped.clone(),
            };
            let run = run_auto(fallback(), portal, portal_rx, failure_rx, event_tx, stop_rx);
            tokio::pin!(run);
            assert!(futures_util::poll!(&mut run).is_pending());
            if failed {
                failure_tx
                    .send(HotkeyError::PortalCancelled)
                    .expect("failure should be delivered");
                assert!(futures_util::poll!(&mut run).is_pending());
            }
            stop_tx.send(()).expect("auto listener should be running");
            tokio::time::timeout(std::time::Duration::from_secs(1), run)
                .await
                .expect("stop must interrupt a blocked event send");
            assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn a_failure_during_forwarding_preserves_queued_events() {
        let (portal_tx, portal_rx) = mpsc::channel(2);
        let pressed = HotkeyEvent::Pressed {
            model_override: None,
            profile_override: None,
        };
        portal_tx
            .send(pressed.clone())
            .await
            .expect("portal queue should have room");
        portal_tx
            .send(HotkeyEvent::Released)
            .await
            .expect("portal queue should have room");
        drop(portal_tx);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        event_tx
            .send(HotkeyEvent::Cancel)
            .await
            .expect("daemon queue should be empty");
        let (failure_tx, failure_rx) = oneshot::channel();
        let (_stop_tx, stop_rx) = oneshot::channel();
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let portal = FakeListener {
            events: None,
            stopped,
        };
        let run = run_auto(fallback(), portal, portal_rx, failure_rx, event_tx, stop_rx);
        tokio::pin!(run);
        assert!(futures_util::poll!(&mut run).is_pending());
        drop(failure_tx);
        assert!(futures_util::poll!(&mut run).is_pending());
        let collect = async {
            let mut events = Vec::new();
            while let Some(event) = event_rx.recv().await {
                events.push(event);
            }
            events
        };
        let (_, events) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(run, collect)
        })
        .await
        .expect("all queued events should be forwarded");
        assert_eq!(
            events,
            vec![HotkeyEvent::Cancel, pressed, HotkeyEvent::Released]
        );
    }

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
