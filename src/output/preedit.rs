//! Wayland preedit output via `zwp_input_method_v2`.
//!
//! Every other backend here is blind: it synthesises key events and hopes the
//! focused field takes them. This one talks to the compositor as an input
//! method, which buys two things nothing else can:
//!
//! - **Free revision.** Preedit is uncommitted text the application renders
//!   inline (underlined, or highlighted in a terminal). Replacing it costs one
//!   message — no backspaces, no guessing what is still at the cursor. That is
//!   what makes a cleanup pass viable: the raw transcript can sit in preedit
//!   while a model rewrites it, and only the finished text is ever committed.
//! - **It knows where it is typing.** `activate`/`deactivate` say whether a
//!   text field has focus at all, so output can be withheld instead of fired
//!   blindly at whatever is on screen.
//!
//! The compositor grants one input method per seat, so the connection is a
//! process-wide singleton, started on first use and living on a thread of its
//! own.
//!
//! ponytail: that thread polls its channel every 20 ms between Wayland
//! roundtrips instead of putting both in one poll set. 20 ms is far below what
//! anyone perceives while speaking; wire up `prepare_read` if this ever needs
//! to carry something latency-critical.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};

use super::{OutputError, TextOutput};

/// What the Wayland thread is asked to do.
enum Request {
    /// Replace the uncommitted text. Empty clears it.
    Preedit(String),
    /// Commit text into the field and clear any preedit.
    Commit(String),
}

/// Handle on the input-method thread.
pub struct Preedit {
    tx: Sender<Request>,
    /// Whether a text field currently has focus, as of the last `done`.
    active: Arc<AtomicBool>,
}

static GLOBAL: OnceLock<Option<Preedit>> = OnceLock::new();

/// The process-wide input method, started on first call.
///
/// `None` when the compositor exposes no `zwp_input_method_manager_v2`, or
/// when another input method already holds the seat — only one may.
pub fn global() -> Option<&'static Preedit> {
    GLOBAL.get_or_init(Preedit::start).as_ref()
}

/// Show in-progress text, if an input method is available and focused.
///
/// Returns whether it was shown, so callers can fall back to their own
/// preview (the OSD) when it was not.
pub fn show(text: &str) -> bool {
    match global() {
        Some(p) if p.is_active() => p.send(Request::Preedit(text.to_string())),
        _ => false,
    }
}

impl Preedit {
    fn start() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let active = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread_active = Arc::clone(&active);
        std::thread::Builder::new()
            .name("voxtype-preedit".into())
            .spawn(move || run(rx, thread_active, ready_tx))
            .ok()?;

        // The thread reports whether it got an input method before anything
        // tries to use it; a failure here is normal (X11, another IME) and
        // must fall through to the next backend, not error.
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => Some(Self { tx, active }),
            _ => {
                tracing::info!("No Wayland input method available; preedit output disabled");
                None
            }
        }
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    fn send(&self, request: Request) -> bool {
        self.tx.send(request).is_ok()
    }
}

/// Output backend that commits finished text through the input method.
pub struct PreeditOutput;

#[async_trait]
impl TextOutput for PreeditOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        let preedit = global().ok_or_else(|| {
            OutputError::InjectionFailed("no Wayland input method (zwp_input_method_v2)".into())
        })?;
        if !preedit.is_active() {
            return Err(OutputError::InjectionFailed(
                "no text field focused; nothing would receive the text".into(),
            ));
        }
        if !preedit.send(Request::Commit(text.to_string())) {
            return Err(OutputError::InjectionFailed(
                "input-method thread is gone".into(),
            ));
        }
        Ok(())
    }

    async fn is_available(&self) -> bool {
        global().is_some_and(|p| p.is_active())
    }

    fn name(&self) -> &'static str {
        "preedit"
    }
}

// ---------------------------------------------------------------------------
// Wayland thread
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpInputMethodManagerV2>,
    /// Focus as of the last `done`: the protocol batches state and applies it
    /// atomically, so `activate`/`deactivate` only stage a change.
    pending_active: bool,
    active: Arc<AtomicBool>,
    /// Counts `done` events. Every commit must echo the latest one back or the
    /// compositor discards it.
    serial: u32,
    unavailable: bool,
    shown: String,
}

fn run(rx: Receiver<Request>, active: Arc<AtomicBool>, ready: Sender<bool>) {
    let Ok(conn) = Connection::connect_to_env() else {
        let _ = ready.send(false);
        return;
    };
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut state = State {
        active,
        ..Default::default()
    };
    if queue.roundtrip(&mut state).is_err() {
        let _ = ready.send(false);
        return;
    }

    let (Some(seat), Some(manager)) = (state.seat.clone(), state.manager.clone()) else {
        let _ = ready.send(false);
        return;
    };
    let im = manager.get_input_method(&seat, &qh, ());
    if queue.roundtrip(&mut state).is_err() || state.unavailable {
        let _ = ready.send(false);
        return;
    }
    let _ = ready.send(true);
    tracing::info!("Wayland input method bound; preedit output available");

    loop {
        if queue.roundtrip(&mut state).is_err() || state.unavailable {
            state.active.store(false, Ordering::Relaxed);
            return;
        }

        while let Ok(request) = rx.try_recv() {
            match request {
                Request::Preedit(text) => {
                    if text == state.shown {
                        continue;
                    }
                    let cursor = text.len() as i32;
                    im.set_preedit_string(text.clone(), cursor, cursor);
                    im.commit(state.serial);
                    state.shown = text;
                }
                Request::Commit(text) => {
                    // Committing with no preedit set clears whatever was
                    // showing, so the guess is replaced by the final text in
                    // one atomic step — the app never renders both.
                    im.commit_string(text);
                    im.commit(state.serial);
                    state.shown.clear();
                }
            }
        }

        if conn.flush().is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_seat" => state.seat = Some(registry.bind(name, 1, qh, ())),
                "zwp_input_method_manager_v2" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.pending_active = true,
            zwp_input_method_v2::Event::Deactivate => state.pending_active = false,
            zwp_input_method_v2::Event::Done => {
                state.serial += 1;
                if state.active.load(Ordering::Relaxed) != state.pending_active {
                    state.active.store(state.pending_active, Ordering::Relaxed);
                    // Focus changes discard preedit on the client side, so what
                    // we think is on screen is gone with it.
                    state.shown.clear();
                    tracing::debug!("preedit target: {}", state.pending_active);
                }
            }
            zwp_input_method_v2::Event::Unavailable => {
                tracing::info!("Another input method holds the seat; preedit output disabled");
                state.unavailable = true;
            }
            _ => {}
        }
    }
}

macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event,
                     _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(wl_seat::WlSeat, ZwpInputMethodManagerV2);
