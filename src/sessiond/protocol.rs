//! Wire protocol for souveraine-sessiond — the session authority daemon.
//!
//! One JSON object per line over a Unix socket in the user's runtime dir,
//! guarded `ok`/`reason` responses, same shape as machined's protocol. The
//! socket lives on the user tier (unlike machined's system-tier socket): the
//! session authority serves exactly one seat and dies with it.
//!
//! The handoff contract with the shell:
//!
//! 1. sessiond starts before the shell and takes ext-session-lock — the
//!    session is locked before any shell surface can exist.
//! 2. The shell connects, sends `shell_ready`, and KEEPS the connection
//!    open. That connection is the heartbeat; EOF means the shell died.
//! 3. sessiond releases by dropping its Wayland connection WITHOUT
//!    unlocking. The compositor keeps the session locked (abandoned-client
//!    state) and `misc:allow_session_lock_restore` lets the shell's own
//!    WlSessionLock take over. There is never an unlocked instant.
//! 4. The shell sends `locked_ack` once its lock surface is secure. If the
//!    ack does not arrive in time, sessiond takes the lock back.
//! 5. Heartbeat EOF at any point → sessiond retakes the lock immediately,
//!    whether or not the session was locked at the time. Fail closed.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Socket path relative to `$XDG_RUNTIME_DIR`.
pub const SOCKET_RELPATH: &str = "souveraine/sessiond.sock";

/// Upper bound on one request line.
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024;

/// PAM service for the fallback unlock surface. The shell's lock uses
/// quickshell's default (`login`); the dedicated file lets the PIN stack be
/// audited separately and is shipped as root-owned system config by the OS
/// overlay, never by this crate (same stance as `souveraine-stepup`).

/// How long after releasing the lock we wait for the shell's `locked_ack`
/// before deciding the shell is broken and taking the lock back.
pub const LOCKED_ACK_TIMEOUT_SECS: u64 = 15;

/// Why a request was refused, in a form a caller can branch on.
///
/// Refusals were prose. A human reads "a live shell owns the session lock" and
/// knows what to do; an agent composing several verbs into one intent
/// (doctrine §13) cannot branch on a sentence. The reason stays — it is the
/// only thing that explains *this* refusal rather than its class — but the
/// code is what a chain reads.
///
/// The taxonomy is deliberately small. RedFlag's executor exit codes are the
/// precedent (`NET_LAYER_PLAN.md` cites them for the same reason): a fixed
/// vocabulary a caller can exhaust, not a growing list it must keep up with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    /// The op does not exist, or the line did not parse as a request.
    /// Retrying is pointless; the caller is speaking a vocabulary this daemon
    /// does not have. `describe` is the cure.
    UnsupportedOp,
    /// The op exists and the arguments are wrong — out of range, mismatched,
    /// or self-contradictory. Retrying with the same arguments is pointless.
    InvalidArgument,
    /// The op and arguments are fine and the machine's current state forbids
    /// it. This is the one a chain most often wants: it means *not now*, and
    /// the state that forbade it is worth reading before deciding what next.
    RefusedByState,
    /// The caller may not do this. Power policy issues it when login1 answers
    /// `no`; capability tokens will eventually make it caller-specific across
    /// the rest of the surface too.
    NotPermitted,
    /// A dependency this op needs is absent — no compositor, no PAM stack, no
    /// sensor. Not the caller's fault and possibly transient.
    Unavailable,
}

impl RefusalCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::UnsupportedOp => "unsupported_op",
            RefusalCode::InvalidArgument => "invalid_argument",
            RefusalCode::RefusedByState => "refused_by_state",
            RefusalCode::NotPermitted => "not_permitted",
            RefusalCode::Unavailable => "unavailable",
        }
    }
}

/// One entry in the verb table `describe` returns.
///
/// Serialize-only: it is a const table on this side of the wire and JSON on the
/// other, so the borrowed `&'static` fields never need to come back.
#[derive(Debug, Clone, Serialize)]
pub struct VerbDoc {
    /// The `op` string, exactly as it goes on the wire.
    pub op: &'static str,
    /// True when the op changes something. A caller planning a chain needs to
    /// know which steps are reversible reads and which are not.
    pub mutates: bool,
    /// What it does, in one line.
    pub summary: &'static str,
    /// The refusal codes this op can return. A chain plans its branches from
    /// this rather than discovering them by being refused.
    pub refuses: &'static [RefusalCode],
    /// A minimal request line that parses. Knowing an op exists is not enough
    /// to call it — `panel` needs `on`, `sensor_input` needs a source and a
    /// matching value — and a caller should not have to discover required
    /// fields by being refused. The test round-trips every one of these, so an
    /// example that stops parsing fails the build rather than the agent.
    pub example: &'static str,
}

/// The verb table. Every `Request` variant appears here; a test enforces it.
///
/// This exists because the surface was not enumerable. An agent that owns the
/// device (doctrine §13) has to be able to ask what it may do rather than be
/// told, and a verb that is missing from this table is as unreachable as one
/// that was never written.
pub const VERBS: &[VerbDoc] = &[
    VerbDoc {
        op: "describe",
        mutates: false,
        summary: "this table — the verbs, which ones mutate, how each can refuse",
        refuses: &[],
        example: r#"{"op":"describe"}"#,
    },
    VerbDoc {
        op: "status",
        mutates: false,
        summary: "liveness and who currently owns the session lock",
        refuses: &[],
        example: r#"{"op":"status"}"#,
    },
    VerbDoc {
        op: "shell_ready",
        mutates: true,
        summary: "the shell announces itself; this connection becomes the heartbeat",
        refuses: &[RefusalCode::RefusedByState],
        example: r#"{"op":"shell_ready"}"#,
    },
    VerbDoc {
        op: "locked_ack",
        mutates: true,
        summary: "the shell's lock surface reached compositor-acknowledged secure",
        refuses: &[RefusalCode::RefusedByState],
        example: r#"{"op":"locked_ack"}"#,
    },
    VerbDoc {
        op: "lock",
        mutates: true,
        summary: "sessiond takes the session lock itself; refused while a live shell owns it",
        refuses: &[RefusalCode::RefusedByState, RefusalCode::Unavailable],
        example: r#"{"op":"lock"}"#,
    },
    VerbDoc {
        op: "device_state",
        mutates: false,
        summary: "the unified state machine: state, panel, evidence, confidence, health",
        refuses: &[],
        example: r#"{"op":"device_state"}"#,
    },
    VerbDoc {
        op: "sensor_input",
        mutates: true,
        summary: "report a sensor reading as evidence; never an authority (doctrine §9)",
        refuses: &[RefusalCode::InvalidArgument],
        example: r#"{"op":"sensor_input","source":"proximity","value":{"near":true}}"#,
    },
    VerbDoc {
        op: "input",
        mutates: true,
        summary: "real user input happened; resets the idle budget",
        // A squeeze is refusable — a pocket can produce one, and proximity is
        // the veto (DEVICE-STATE-MACHINE §4). A power button never is.
        refuses: &[RefusalCode::RefusedByState],
        example: r#"{"op":"input","trigger":"touch"}"#,
    },
    VerbDoc {
        op: "button",
        mutates: true,
        summary: "a hardware button edge; the machine recognises taps and holds from these",
        refuses: &[RefusalCode::InvalidArgument],
        example: r#"{"op":"button","button":"power","edge":"down"}"#,
    },
    VerbDoc {
        op: "gesture",
        mutates: true,
        summary: "a recognised touch gesture; the machine decides what it means",
        refuses: &[RefusalCode::InvalidArgument],
        example: r#"{"op":"gesture","fingers":3,"gesture":"tap","target":1}"#,
    },
    VerbDoc {
        op: "panel",
        mutates: true,
        summary: "the DPMS executor reports the panel's real power state",
        refuses: &[],
        example: r#"{"op":"panel","on":false}"#,
    },
    VerbDoc {
        op: "screen",
        mutates: true,
        summary: "turn the panel on or off; a blank still locks the session first",
        // Refusable because turning it OFF routes through the same
        // lock-before-blank path everything else does — the agent owns
        // operation (§13) and still cannot blank an unlocked session, because
        // that is an ordering invariant rather than a permission.
        refuses: &[RefusalCode::RefusedByState],
        example: r#"{"op":"screen","on":true}"#,
    },
    VerbDoc {
        op: "forensic_log",
        mutates: false,
        summary: "recent trail entries from the in-memory buffer",
        refuses: &[],
        example: r#"{"op":"forensic_log","count":50}"#,
    },
    VerbDoc {
        op: "subscribe",
        mutates: false,
        summary: "this connection becomes an event stream of notable belief changes",
        refuses: &[],
        example: r#"{"op":"subscribe"}"#,
    },
    VerbDoc {
        op: "get_policy",
        mutates: false,
        summary: "read the timed policy — the Auto-Lock shaped settings",
        refuses: &[],
        example: r#"{"op":"get_policy"}"#,
    },
    VerbDoc {
        op: "set_policy",
        mutates: true,
        summary: "change the timed policy; persisted, so a setting survives restart",
        refuses: &[RefusalCode::InvalidArgument, RefusalCode::Unavailable],
        example: r#"{"op":"set_policy","lock_blank_after_secs":15}"#,
    },
    VerbDoc {
        op: "bearer",
        mutates: false,
        summary: "which link carries traffic, and whether the tunnel is being answered",
        refuses: &[],
        example: r#"{"op":"bearer"}"#,
    },
    VerbDoc {
        op: "usb",
        mutates: false,
        summary: "USB-C role, gadget mode, charger evidence, attachment identity and probe owner",
        refuses: &[RefusalCode::Unavailable],
        example: r#"{"op":"usb"}"#,
    },
    VerbDoc {
        op: "set_usb_mode",
        mutates: true,
        summary: "ask the device-state authority to change the USB gadget posture",
        refuses: &[RefusalCode::RefusedByState, RefusalCode::Unavailable],
        example: r#"{"op":"set_usb_mode","mode":"hid"}"#,
    },
    VerbDoc {
        op: "power",
        mutates: true,
        summary: "power the machine off, restart it, or put it to sleep",
        refuses: &[
            RefusalCode::RefusedByState,
            RefusalCode::NotPermitted,
            RefusalCode::Unavailable,
        ],
        example: r#"{"op":"power","verb":"poweroff"}"#,
    },
];

/// A request plus the caller's declared intent.
///
/// The wire shape is unchanged — `{"op":"panel","on":false}` still parses,
/// because `intent` is optional and flattened alongside the tagged enum. A
/// caller composing several verbs into one decision (doctrine §13) sets it
/// once per verb and the trail joins the leaves back to the intent that
/// produced them: `{"op":"panel","on":false,"intent":"quiet the room"}`.
///
/// Declared, never verified. It is a label the caller supplies about itself,
/// so it is evidence in exactly the sense doctrine §9 means — useful for
/// reconstruction, never a basis for a decision. Nothing branches on it.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    #[serde(flatten)]
    pub request: Request,
    #[serde(default)]
    pub intent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// The verb table — what this daemon can be asked, which ops mutate, and
    /// how each can refuse. A caller composing verbs reads this first.
    Describe,
    /// Liveness + who currently owns the session lock.
    Status,
    /// The shell announces itself. The connection carrying this request
    /// becomes the heartbeat. If sessiond is holding the lock, it releases
    /// (see module docs) and the shell must lock and then `locked_ack`.
    ShellReady,
    /// The shell's lock surface reached compositor-acknowledged `secure`.
    LockedAck,
    /// Ask sessiond to take the session lock itself. Refused while a live
    /// shell owns steady state — the shell's lock IPC is the front door.
    Lock,
    /// Full device state: the unified state machine's current state,
    /// sensor evidence confidence, and doze tier. Superset of Status.
    DeviceState,
    /// Report a sensor reading to the device state machine.
    /// The shell or a sensor daemon feeds proximity/accel/touch evidence.
    SensorInput(SensorInput),
    /// Real user input happened — touch, key, power button, dt2w wake.
    /// Resets the idle budget that the lock-blank rule counts against.
    /// This is the wire that lets the machine tell "the user is looking at
    /// the lock screen" from "the lock screen has been lit for ten minutes."
    Input { trigger: Option<InputTrigger> },
    /// A hardware button went down or came up. Edges only — the caller reports
    /// what the hardware did and nothing else.
    ///
    /// `blueline-power-button` used to *be* the policy: it read a state file,
    /// asked the shell to lock over `qs ipc`, polled `session state` twenty
    /// times at 100 ms grepping for `"locked": true`, then called
    /// `blueline-screen-toggle off` itself. That is a path to a dark panel that
    /// never routes through `request_blank()` — so LOCK-DPMS-LESSONS §1's
    /// "every path routes through it, an invariant not a coincidence" had a hole
    /// in it, and the hole was the most-used control on the device. It also made
    /// the button the eighth blind actor of DEVICE-STATE-MACHINE §1, with its
    /// own copy of the lock-then-blank ordering and its own panel truth.
    ///
    /// Now it reports and stops deciding. Recognition — tap, double, triple,
    /// hold — happens in the machine, because a gesture is an accumulation over
    /// time and time is the one thing a fire-and-forget script does not have.
    Button { button: Button, edge: ButtonEdge },
    /// A recognised touch gesture, already named by the compositor.
    ///
    /// Unlike [`Self::Button`], which carries raw edges because recognition is
    /// an accumulation over time and the machine owns time, this arrives named:
    /// touch contacts exist only inside the compositor, so nothing else *can*
    /// recognise them. The line §12 draws still holds — the compositor names
    /// what the fingers did and the machine decides what it means. A compositor
    /// that both recognised and acted would be the eighth blind actor.
    ///
    /// `target` is the window the gesture landed on, named by the compositor
    /// through the same hit test a finger goes through. It travels with the
    /// gesture because that mapping exists nowhere else — it accounts for the
    /// zone strip and any pose — and a machine re-deriving it from a centroid
    /// would eventually disagree with what the hand actually hit. Absent means
    /// the gesture landed on the wallpaper, which is a real answer and a
    /// different one from "no window exists".
    Gesture {
        fingers: u8,
        gesture: TouchGesture,
        #[serde(default)]
        target: Option<u64>,
    },
    /// The DPMS executor reports the panel's real power state. The machine
    /// keeps the panel as a field, not a state: "locked with the screen off"
    /// is not a doze tier, and doze (frozen apps, Wi-Fi save) is not a dark
    /// glance. Reported, never assumed — the executor owns the panel.
    Panel { on: bool },
    /// Ask for the panel. The agent's verb, and the user's, and the shell's.
    ///
    /// Distinct from [`Request::Panel`], which is the executor *reporting*
    /// what the hardware did. This one *asks*, and the distinction is the
    /// whole reason both exist: a report must never be able to actuate, or a
    /// stale report would drive the panel; and an ask must never be able to
    /// silently edit the machine's idea of the hardware.
    ///
    /// Waking is immediate. Blanking goes through the same `request_blank()`
    /// every other path takes, so it locks first and waits for the ack —
    /// doctrine §13's shape exactly: operation is hers, and the one thing she
    /// cannot do is make the glass dark on a session that is not locked.
    Screen { on: bool },
    /// Query recent forensic log entries. Returns the last N entries
    /// from the in-memory forensic buffer for post-hoc analysis.
    ForensicLog { count: Option<usize> },
    /// Turn this connection into an event stream. After the `ok`, the daemon
    /// pushes one trail entry per line, unprompted, for as long as the caller
    /// holds the socket open.
    ///
    /// The point is the *filter*, not the transport. `forensic_log` already
    /// hands over everything; a consumer that polls it and diffs is doing the
    /// machine's job for it, badly and late. Only NOTABLE entries are pushed —
    /// state transitions, a source that died or came back, a violated
    /// guarantee, sensors that contradict each other. Heartbeats, ticks and
    /// ordinary readings never cross.
    ///
    /// That compression IS the product, in the same sense six strain gauges at
    /// 100 Hz becoming one `squeeze` bit is the product. An agent that received
    /// every reading would be reading drivers, and doctrine's load-bearing rule
    /// is that interpretation may consume evidence but never a driver — reading
    /// the raw stream is how gait, typing and identity get inferred from data
    /// that looks innocent per-field (SECURITY-AUDIT P1). The narrowness here is
    /// a security control, not a performance one.
    Subscribe,
    /// Read the timed policy — the Auto-Lock shaped settings.
    GetPolicy,
    /// Bearer posture: per-link health, the tunnel's handshake state, and
    /// which bearer the machine has settled on.
    ///
    /// A read, deliberately. The corresponding write does not exist and should
    /// not: the thing that decides which link carries traffic is `tick()`, and
    /// a verb that let a caller set it would be the second writer TASK-49
    /// acceptance #6 forbids. What a caller can do is read this and change the
    /// *policy* (`set_policy`), which is the difference between steering the
    /// machine and reaching around it.
    Bearer,
    /// Port proprioception. The mechanism is adjacent to souveraine-upower:
    /// usb-signaller reports/acts on the data posture, while UPower reports
    /// charging posture. sessiond is the one place the two become a snapshot.
    Usb,
    /// Ask for a gadget posture. This does not let the shell write configfs;
    /// it becomes an Action and is executed through usb-signaller's system
    /// D-Bus API. Full KVM is admitted because usb-signaller prepares both
    /// FunctionFS responders and rolls the composite back as one transaction.
    SetUsbMode { mode: UsbMode },
    /// Change the timed policy. Every field is optional; omitted fields keep
    /// their current value. This is the seam the Settings control center reads
    /// and writes, so a control there is a view over the owning daemon rather
    /// than a switch that only looks like it did something (TASK-19's rule).
    SetPolicy {
        /// Seconds; `Some(0)` means never blank.
        lock_blank_after_secs: Option<u64>,
        lock_blank_after_held_secs: Option<u64>,
        dim_grace_secs: Option<u64>,
        evidence_ttl_secs: Option<u64>,
        dim_warning: Option<bool>,
        /// Seconds a pending blank waits for its lock ack before going dark
        /// anyway. Never 0 — a blank that waits for nothing is the ordering
        /// bug this field exists to close.
        lock_ack_budget_secs: Option<u64>,
        /// Seconds; `Some(0)` means never blank an unlocked session here.
        unlocked_blank_after_secs: Option<u64>,
        /// The SSIDs that are the home LAN. Replaces the list wholesale;
        /// `Some([])` means "never claim to be home", which is the safe
        /// default rather than an erasure.
        ///
        /// Identity, never a prefix. The gate this replaces matched on an
        /// address prefix, so a foreign network sharing those octets read as
        /// home — which is the whole reason this is a list of SSIDs.
        home_ssids: Option<Vec<String>>,
        /// Seconds a changed bearer preference must hold before it is acted
        /// on. Never 0: a window of zero is the event-speed controller that
        /// recycled the tunnel 652 times.
        bearer_settle_secs: Option<u64>,
    },
    /// End the session's power state: off, restart, or asleep.
    ///
    /// The last device verb that was not here. Both menu surfaces called a QML
    /// singleton that ran `systemctl poweroff` itself, which is §12's eighth
    /// blind actor on the one transition that cannot be undone or observed
    /// after the fact — no Action, no executor, no trail entry.
    ///
    /// logind keeps what it already owns. It answers `CanPowerOff` and carries
    /// out the verb; doctrine §4 says suspend goes through logind and never
    /// `/sys/power/state`, and that is unchanged. What moves is the decision to
    /// ask and the record of its outcome. Actual sleep state remains logind
    /// evidence; this request does not claim that a transition happened.
    Power { verb: PowerVerb },
}

/// The power transitions sessiond will carry out.
///
/// `logout` is deliberately absent: it ends a *session*, not a device power
/// state, and the machine has no cell for it. It stays the shell's, which is
/// also the only actor that knows what it would be tearing down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerVerb {
    Poweroff,
    Reboot,
    Suspend,
    Hibernate,
}

impl PowerVerb {
    /// systemctl owns these verbs, not loginctl — `loginctl poweroff` exits 1
    /// with "Unknown command verb". Preferring loginctl silently broke every
    /// power button on the device for a day (2026-07-20).
    pub fn as_systemctl(self) -> &'static str {
        match self {
            Self::Poweroff => "poweroff",
            Self::Reboot => "reboot",
            Self::Suspend => "suspend",
            Self::Hibernate => "hibernate",
        }
    }

    pub fn as_str(self) -> &'static str {
        self.as_systemctl()
    }

    /// The logind capability that gates this verb.
    pub fn logind_capability(self) -> &'static str {
        match self {
            Self::Poweroff => "CanPowerOff",
            Self::Reboot => "CanReboot",
            Self::Suspend => "CanSuspend",
            Self::Hibernate => "CanHibernate",
        }
    }
}

/// USB postures sessiond is prepared to own today.
///
/// Keep this narrower than usb-signaller's raw vocabulary. MTP and tethering
/// are not power-menu promises; host mode is unsafe until the SMB2/TCPM lane
/// can source VBUS. Full KVM is here because GUD and smoo now share one
/// readiness-gated, rollback-capable lifecycle in usb-signaller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbMode {
    Developer,
    Hid,
    Kvm,
    ChargingOnly,
}

impl UsbMode {
    pub fn as_usb_moded(self) -> &'static str {
        match self {
            Self::Developer => "developer_mode",
            Self::Hid => "hid_mode",
            Self::Kvm => "kvm_mode",
            Self::ChargingOnly => "charging_only",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Developer => "developer",
            Self::Hid => "hid",
            Self::Kvm => "kvm",
            Self::ChargingOnly => "charging_only",
        }
    }
}

/// What produced a user input. Recorded so the forensic trail can tell a
/// deliberate power-button press from an accidental pocket touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTrigger {
    Touch,
    Key,
    PowerButton,
    DoubleTapToWake,
    /// Active Edge — the frame was squeezed. Deliberate intent, like a button,
    /// but unlike a button it is a sensor and it can be produced by a pocket:
    /// a squeezed chassis is exactly what a phone in a tight pocket is. So it
    /// carries the same proximity veto as tap-to-wake (`suppress_wake`), and
    /// for the same reason — see DEVICE-STATE-MACHINE §4, "a covered sensor is
    /// the right veto for the one wake a pocket can produce by itself".
    Squeeze,
    Unknown,
}

/// Which hardware button. Volume is here because the recognizer is per-button
/// and costs nothing to reuse; nothing binds them yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Power,
    VolumeUp,
    VolumeDown,
}

impl Button {
    pub fn as_str(self) -> &'static str {
        match self {
            Button::Power => "power",
            Button::VolumeUp => "volume_up",
            Button::VolumeDown => "volume_down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonEdge {
    Down,
    Up,
}

/// What a set of fingers did. Named by the compositor, meant by the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchGesture {
    /// Down and up without travelling.
    Tap,
    /// Travelled and let go. The direction is what a binding usually cares
    /// about; the distance is the compositor's business.
    SwipeUp,
    SwipeDown,
    SwipeLeft,
    SwipeRight,
}

impl TouchGesture {
    pub fn as_str(self) -> &'static str {
        match self {
            TouchGesture::Tap => "tap",
            TouchGesture::SwipeUp => "swipe_up",
            TouchGesture::SwipeDown => "swipe_down",
            TouchGesture::SwipeLeft => "swipe_left",
            TouchGesture::SwipeRight => "swipe_right",
        }
    }
}

/// What the machine made of a run of edges.
///
/// `Hold` fires while the button is still down — a hold you only learn about on
/// release is a hold that cannot light anything up while you are waiting, and
/// waiting with no feedback is how a user decides the device is broken and lets
/// go. Taps resolve on the multi-tap window expiring, which is the opposite
/// trade and the right one: a double-tap must not first fire a single.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonGesture {
    Tap,
    DoubleTap,
    TripleTap,
    /// Held past the first threshold, still down.
    Hold,
    /// Held past the second, still down. The "you may let go now" point for
    /// anything destructive — nothing binds it yet.
    LongHold,
}

impl ButtonGesture {
    pub fn as_str(self) -> &'static str {
        match self {
            ButtonGesture::Tap => "tap",
            ButtonGesture::DoubleTap => "double_tap",
            ButtonGesture::TripleTap => "triple_tap",
            ButtonGesture::Hold => "hold",
            ButtonGesture::LongHold => "long_hold",
        }
    }
}

/// A sensor reading fed to the device state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorInput {
    /// Which sensor produced this reading.
    pub source: SensorSource,
    /// The reading value.
    pub value: SensorValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorSource {
    Proximity,
    Accelerometer,
    Light,
    Touch,
    /// The power supplies under `/sys/class/power_supply`. Reported by the
    /// same reporter as the others — charge is evidence, and it enters the
    /// machine through the same gate.
    Charge,
}

impl SensorSource {
    /// The name this source is known by in the forensic trail and in logs.
    /// Matches the `evidence_fresh` / `sensor_health` keys so a reader can
    /// join them without a translation table.
    pub fn as_str(self) -> &'static str {
        match self {
            SensorSource::Proximity => "proximity",
            SensorSource::Accelerometer => "accel",
            SensorSource::Light => "light",
            SensorSource::Touch => "touch",
            SensorSource::Charge => "charge",
        }
    }
}

/// What the kernel says the battery and its supplies are doing, verbatim.
///
/// Fields are kept as the kernel's own strings rather than re-enumed: a value
/// the crate does not recognise must survive to the readout untouched, because
/// "the kernel said something new" is itself the evidence. A non-battery
/// supply asserting `online` counts as plugged; `None` means no supply node
/// was readable at all — absence is not "unplugged".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChargeFields {
    pub plugged: Option<bool>,
    /// The fuel gauge's `status`: "Charging", "Discharging", "Not charging",
    /// "Full", or whatever the driver said.
    pub status: Option<String>,
    /// The charger's `charge_type` — "Fast", "Slow", "Trickle" on blueline's
    /// pmi8998. The fuel gauge has no such attribute; it is read off the
    /// charger side.
    pub charge_type: Option<String>,
    /// Fuel gauge percentage, 0–100.
    pub capacity: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorValue {
    /// Proximity: near or far.
    Near(bool),
    /// Accelerometer: moving or stationary.
    Moving(bool),
    /// Light: the level, and whether it moved enough to count as a change.
    ///
    /// The number rides so the machine and the plexus can use it as a level;
    /// `changing` is the debounced edge the confidence table believes (§9).
    Light { changing: bool, lux: f64 },
    /// Touch: active or inactive.
    Active(bool),
    /// The raw charge reading — evidence only, interpreted by the machine.
    Charge(ChargeFields),
}

/// What sessiond currently is, as reported by `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// sessiond holds ext-session-lock and renders the fallback surface.
    Holding,
    /// Lock released to the shell; heartbeat live; ack not yet seen.
    AwaitingShellLock,
    /// Shell owns steady state (heartbeat live, ack seen).
    Released,
    /// No shell heartbeat and not holding (initial-lock disabled or the
    /// user unlocked at the fallback surface with no shell to hand off to).
    Idle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let req: Request = serde_json::from_str(r#"{"op":"shell_ready"}"#).unwrap();
        assert!(matches!(req, Request::ShellReady));
        let req: Request = serde_json::from_str(r#"{"op":"status"}"#).unwrap();
        assert!(matches!(req, Request::Status));
    }

    #[test]
    fn every_request_variant_is_in_the_verb_table() {
        // The table is the vocabulary an agent composes over (doctrine §13).
        // A verb missing from it is as unreachable as one never written, so
        // this test is the thing that keeps them from drifting apart.
        //
        // Checked by round-tripping each op string through the deserializer:
        // if `describe` advertises an op the daemon cannot parse, the entry is
        // a lie, and if a variant is added without a table entry the count
        // below fails.
        for v in VERBS {
            let parsed: Result<Request, _> = serde_json::from_str(v.example);
            assert!(
                parsed.is_ok(),
                "describe advertises `{}` with example `{}`, which the daemon cannot parse: {:?}",
                v.op,
                v.example,
                parsed.err()
            );
        }

        // And the other direction: every variant must be advertised. Bump this
        // deliberately when a verb is added, having added its VerbDoc.
        // 20 since device power became a session-authority verb (2026-08-09).
        assert_eq!(
            VERBS.len(),
            20,
            "a Request variant was added without a VerbDoc"
        );
    }

    #[test]
    fn every_power_verb_survives_the_wire_and_maps_to_its_mechanism() {
        let cases = [
            ("poweroff", PowerVerb::Poweroff, "CanPowerOff"),
            ("reboot", PowerVerb::Reboot, "CanReboot"),
            ("suspend", PowerVerb::Suspend, "CanSuspend"),
            ("hibernate", PowerVerb::Hibernate, "CanHibernate"),
        ];

        for (wire, expected, capability) in cases {
            let raw = format!(r#"{{"op":"power","verb":"{wire}"}}"#);
            let request: Request = serde_json::from_str(&raw).unwrap();
            assert!(matches!(&request, Request::Power { verb } if *verb == expected));
            assert_eq!(expected.as_str(), wire);
            assert_eq!(expected.as_systemctl(), wire);
            assert_eq!(expected.logind_capability(), capability);
            assert_eq!(serde_json::to_string(&request).unwrap(), raw);
        }
    }

    #[test]
    fn power_advertises_every_refusal_its_policy_can_make() {
        let power = VERBS.iter().find(|verb| verb.op == "power").unwrap();
        assert_eq!(
            power.refuses,
            &[
                RefusalCode::RefusedByState,
                RefusalCode::NotPermitted,
                RefusalCode::Unavailable,
            ]
        );
    }

    #[test]
    fn the_envelope_is_backward_compatible() {
        // Every line written before intent existed must still parse, or the
        // shell and the reporters stop talking to the daemon on upgrade —
        // TASK-28's failure mode, caused by the fix for it.
        let e: Envelope = serde_json::from_str(r#"{"op":"status"}"#).unwrap();
        assert!(matches!(e.request, Request::Status));
        assert_eq!(e.intent, None);

        let e: Envelope =
            serde_json::from_str(r#"{"op":"panel","on":false,"intent":"quiet the room"}"#).unwrap();
        assert!(matches!(e.request, Request::Panel { on: false }));
        assert_eq!(e.intent.as_deref(), Some("quiet the room"));
    }

    #[test]
    fn a_refusal_code_survives_the_wire() {
        // A chain branches on the code, so it has to arrive intact.
        let raw = serde_json::to_string(&RefusalCode::RefusedByState).unwrap();
        assert_eq!(raw, r#""refused_by_state""#);
        let back: RefusalCode = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, RefusalCode::RefusedByState);
    }

    #[test]
    fn phase_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Phase::AwaitingShellLock).unwrap(),
            r#""awaiting_shell_lock""#
        );
    }

    #[test]
    fn full_kvm_is_a_named_usb_posture() {
        let req: Request = serde_json::from_str(r#"{"op":"set_usb_mode","mode":"kvm"}"#).unwrap();
        assert!(matches!(req, Request::SetUsbMode { mode: UsbMode::Kvm }));
        assert_eq!(UsbMode::Kvm.as_usb_moded(), "kvm_mode");
    }
}
