//! ext-idle-notify-v1 client: the machine's input truth.
//!
//! The state machine needs one fact it cannot work out for itself — is the
//! user touching the device right now. Nothing in sessiond could answer that
//! before: the daemon saw handshakes and PAM verdicts, never input. So the
//! lock-blank rule had no honest clock to run against and the only actor that
//! knew about input was hypridle, off in its own config with a single
//! 600-second timer shared with the desktop case.
//!
//! The compositor is the only thing that actually sees input, so we ask it
//! rather than guess. `ext_idle_notifier_v1` gives exactly two events:
//! `idled` after the requested quiet period, and `resumed` on the next input.
//! Tracking the *boolean* — active or quiet — instead of stamping "last input"
//! ourselves is what makes continuous input safe: a long swipe on the lock
//! surface never idles, so the machine never blanks mid-gesture.
//!
//! This connection is deliberately separate from `lock.rs`. A lock session is
//! transient — it exists only while sessiond holds the lock — but input truth
//! must be continuous, including the whole time the shell owns the lock.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{info, warn};
use wayland_client::{
    delegate_noop,
    protocol::wl_registry::{self, WlRegistry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

use wayland_client::protocol::wl_seat::WlSeat;

/// How long a quiet period must last before the compositor tells us the user
/// went away. Short, because it is only the *edge* we care about — the budgets
/// themselves live in the machine's policy. One second keeps the reported idle
/// start accurate to within a second without waking us needlessly.
const QUIET_PERIOD: Duration = Duration::from_secs(1);

/// What the idle watcher reports back into the daemon.
pub enum IdleEvent {
    /// The compositor has seen no input for `QUIET_PERIOD`. The user stopped
    /// interacting approximately that long ago.
    Idled { since: Instant },
    /// Input happened. The user is here now.
    Resumed,
}

struct IdleState {
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<WlSeat>,
    sink: Arc<dyn Fn(IdleEvent) + Send + Sync>,
}

/// Run the idle watcher until the connection drops. Blocking; call on its own
/// thread.
pub fn run(sink: Arc<dyn Fn(IdleEvent) + Send + Sync>) -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the compositor")?;
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut state = IdleState {
        notifier: None,
        seat: None,
        sink,
    };

    // Two round trips: the first surfaces the globals, the second lets the
    // seat and notifier binds settle before we ask for a notification.
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;

    let (Some(notifier), Some(seat)) = (state.notifier.clone(), state.seat.clone()) else {
        anyhow::bail!(
            "compositor does not offer ext_idle_notifier_v1 — the machine has no input truth"
        );
    };

    // `get_idle_notification`, which **respects idle inhibitors** — not
    // `get_input_idle_notification`, which ignores them and reports pure input
    // silence. An earlier comment here had this backwards.
    //
    // The difference is video. Input alone cannot see a film playing: nobody
    // touches the glass for two hours and the machine would blank on a screen
    // someone is watching. A player takes a `zwp_idle_inhibitor_v1`, viewtop
    // reports it through `set_is_inhibited`, and this notification simply never
    // idles while it is held. So the answer to "does playback keep the phone
    // awake" is yes, through the client's inhibitor rather than through faked
    // input — which also means a client that lies is visible as an inhibitor
    // rather than as phantom activity.
    let _notification =
        notifier.get_idle_notification(QUIET_PERIOD.as_millis() as u32, &seat, &qh, ());
    info!(
        "[idle] watching input via ext_idle_notifier_v1 (quiet period {}ms)",
        QUIET_PERIOD.as_millis()
    );

    loop {
        queue.blocking_dispatch(&mut state)?;
    }
}

impl Dispatch<WlRegistry, ()> for IdleState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_idle_notifier_v1" => {
                    state.notifier =
                        Some(registry.bind::<ExtIdleNotifierV1, _, _>(name, 1, qh, ()));
                }
                "wl_seat" => {
                    if state.seat.is_none() {
                        state.seat =
                            Some(registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for IdleState {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                // Input stopped one quiet period ago, not now.
                let since = Instant::now()
                    .checked_sub(QUIET_PERIOD)
                    .unwrap_or_else(Instant::now);
                (state.sink)(IdleEvent::Idled { since });
            }
            ext_idle_notification_v1::Event::Resumed => {
                (state.sink)(IdleEvent::Resumed);
            }
            _ => {}
        }
    }
}

// The seat is bound only to scope the notification; its events are not ours.
delegate_noop!(IdleState: ignore WlSeat);
delegate_noop!(IdleState: ignore ExtIdleNotifierV1);

/// Spawn the watcher and keep it alive. A compositor restart drops the
/// connection; input truth is load-bearing, so we reconnect rather than
/// silently degrade into a machine that cannot see the user.
pub fn spawn(sink: Arc<dyn Fn(IdleEvent) + Send + Sync>) {
    std::thread::spawn(move || loop {
        match run(Arc::clone(&sink)) {
            Ok(()) => warn!("[idle] watcher exited cleanly — retrying"),
            Err(e) => warn!("[idle] watcher failed: {e:#} — retrying"),
        }
        std::thread::sleep(Duration::from_secs(5));
    });
}
