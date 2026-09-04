//! Unified device state machine — the single authority for device power state.
//!
//! Every actor that changes device power (idle timers, proximity sensor,
//! sleep signals, DPMS) routes through this machine. The machine owns the
//! state; actors are inputs, not authorities.
//!
//! The state graph:
//!   Active → Dimmed → Locked → {Observed, DozeLight, DozeDeep} → Suspending → Asleep
//!   Any locked state → Active (on PAM auth)
//!   Any pre-sleep state → Suspending (on PrepareForSleep)
//!
//! Doctrine: SESSION-AUTHORITY-DOCTRINE §9 ("sensor readings are evidence,
//! not fact") and §11 ("the session authority and the binary authority are
//! the same authority"). TASK-08 and TASK-15 define the tiers.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::belief::SomaticEvent;
use crate::sessiond::bearer::{Bearer, BearerEvidence};
use crate::sessiond::protocol::{
    Button, ButtonEdge, ButtonGesture, ChargeFields, InputTrigger, PowerVerb, SensorSource,
    SensorValue, TouchGesture, UsbMode,
};

/// How long a locked, lit panel waits for input before it blanks.
///
/// Android blanks the lock screen in about this long; Sailfish's mce keeps a
/// separate blank-from-lockscreen timeout for exactly this case. We had
/// neither: the only backstop was hypridle's 600 s screen-off timer, shared
/// with the desktop case, so glancing at the clock lit the panel for ten
/// minutes.
pub const LOCK_BLANK_AFTER: Duration = Duration::from_secs(15);

/// Same, when evidence says the device is in a hand rather than on a table.
pub const LOCK_BLANK_AFTER_HELD: Duration = Duration::from_secs(25);

/// How long the dimmed warning lasts before the panel goes dark.
///
/// Dimming first is the pre-warning: the screen visibly fades and a tap
/// inside this window cancels the blank. The mechanism already existed and
/// was orphaned — hypridle once carried a dim listener
/// (`brightnessctl -s set 10`, `on-resume = blueline-undim`) which is no
/// longer in the live config, while `blueline-undim` is still installed.
pub const LOCK_DIM_GRACE: Duration = Duration::from_secs(10);

/// Evidence older than this is stale, and stale evidence is *unknown* —
/// neither "user present" nor "user absent".
///
/// This exists because of a measured failure: on 2026-07-25 the SLPI took a
/// CHRE fatal, `blueline-hexagonrpcd-sdsp` exited with status 0 so systemd's
/// `Restart=on-failure` never fired, and every sensor was dead for hours
/// while iio-sensor-proxy still answered `HasProximity: true`. A consumer of
/// a dead sensor looked exactly like a consumer of a quiet one.
pub const EVIDENCE_TTL: Duration = Duration::from_secs(30);

/// How long a source that was previously reporting may say nothing before the
/// machine calls it **down** rather than merely stale.
///
/// The two are different claims and the difference is the whole point.
/// `EVIDENCE_TTL` is about the *reading*: stop believing a 30-second-old "near".
/// This is about the *source*: proximity that sat at `far` all afternoon has an
/// unexpired flag of `false` and looks identical whether the sensor is healthy
/// and the phone is on a table, or the DSP took a CHRE fatal an hour ago. The
/// staleness rule cannot tell those apart, because nothing about a `false` flag
/// going unreported is anomalous.
///
/// It is longer than the TTL on purpose. A source is allowed to be quiet for a
/// while — that is normal. What is not normal is a source that was speaking and
/// then stopped for longer than any legitimate quiet period, which is what this
/// threshold names. Reporters keep this honest by re-reporting their last value
/// periodically, so silence means silence rather than "nothing changed".
pub const SOURCE_DOWN_AFTER: Duration = Duration::from_secs(90);

/// How long after daemon start an *expected* source may stay silent before the
/// machine calls it `Absent`.
///
/// Deliberately much longer than `SOURCE_DOWN_AFTER`. sessiond starts before
/// the session does — that is the whole point of it, it takes the lock before
/// quickshell exists — so its reporters legitimately arrive late. This window
/// has to cover session startup on a cold boot without covering a reporter
/// that is never coming, and it is a policy field so it can be moved off the
/// trail rather than argued about here.
pub const SOURCE_EXPECTED_WITHIN: Duration = Duration::from_secs(300);

/// Which sources a reporter is serving on this device *right now*.
///
/// Exactly what `souveraine-sensord` reports: proximity and light off
/// iio-sensor-proxy, charge off `/sys/class/power_supply`. `Touch` has no
/// reporter either, so it stays `Unknown` and silent per §10. A source
/// listed here with nothing behind it manufactures a permanent false alarm
/// — the failure this mechanism exists to avoid in the other direction —
/// and it did: accelerometer stayed here after `6b67512` (2026-08-04)
/// dropped the claim, so every boot went `Absent` at 300 s and
/// `sensors_degraded` read true on 3074 consecutive snapshots.
///
/// The accelerometer is wanted and deliberately unclaimed: a continuous
/// iio-sensor-proxy claim cost 14% of a core (TASK-15). It comes back through
/// TASK-36's SLPI batching, and this list is where it returns.
pub const EXPECTED_SOURCES: &[SensorSource] = &[
    SensorSource::Proximity,
    SensorSource::Light,
    SensorSource::Charge,
];

/// How long proximity must read `near` before the machine believes it.
///
/// §9.5 specified Android's `DisplayPowerProximityStateController` — 0 ms
/// positive, 250 ms negative — and that is the wrong shape for this device.
/// Measured on the trail 2026-07-26: 44 near-episodes over 2.4 hours, median
/// dwell **1 second**, 15 of them sub-second, and a median 115 s of quiet
/// between them. That is not a sensor bouncing around a threshold; it is
/// isolated one-second blips. A negative debounce delays believing `far`, so it
/// would have turned each 1 s blip into a 1.25 s blip and left all 88
/// transitions in place.
///
/// Android debounces the negative edge because there `near` means *screen off
/// at the ear, immediately* — a positive delay would be felt. That constraint
/// left when proximity stopped actuating: it now only vetoes tap-to-wake, where
/// waiting is imperceptible and a false veto is the more annoying failure.
///
/// Nine of the 44 episodes ran ≥5 s (max 173 s). Those are the real ones —
/// pocket, ear, deliberate cover — and they survive this threshold untouched.
pub const PROXIMITY_NEAR_DEBOUNCE: Duration = Duration::from_millis(700);

/// How long proximity must read `far` before the machine believes it.
///
/// Zero. An uncovered sensor is believed at once: the reading that ends a veto
/// should never be the slow one, and erring toward `far` errs toward letting a
/// wake through, which is the recoverable direction.
pub const PROXIMITY_FAR_DEBOUNCE: Duration = Duration::ZERO;

/// How long `Observed` must be held before a `far` reading may end it.
///
/// Measured on the phone 2026-08-03, which is what this is for. The trail was
/// almost entirely `Locked → Observed → Locked`, and reading the snapshots
/// showed why: `prox=true` on the way in and `prox=false` **one second later**
/// on the way out, over and over. `near` must hold 700 ms to be believed and
/// `far` is believed instantly, so a one-second blip is long enough to enter
/// and its end is immediate — the asymmetry that protects the wake veto is the
/// same asymmetry that makes this state chatter.
///
/// The debounce itself cannot be the place to fix it. [`suppress_wake`] reads
/// the *debounced* value, so slowing `far` there would keep vetoing tap-to-wake
/// after the sensor was uncovered, which is precisely what
/// [`PROXIMITY_FAR_DEBOUNCE`]'s zero exists to prevent. One reading, two
/// consumers, opposite needs: the veto wants `far` fast, the state wants it
/// stable. So the hysteresis goes here, on the state, and the veto keeps its
/// instant edge.
///
/// 3 s, and the bound comes from data already in this file rather than from
/// feel: the measured blips ran ~1 s, and of the 44 recorded proximity episodes
/// the nine real ones — pocket, ear, deliberate cover — all ran **≥5 s**
/// (max 173 s). 3 s sits above the noise and below every genuine episode, so it
/// suppresses the chatter without shortening a single real one.
///
/// This reduces the flapping rather than abolishing it: a sensor that keeps
/// blipping still enters `Observed` on each ≥700 ms `near`. Raising the *entry*
/// bar would need the same split applied to the near edge, and that is a second
/// change with its own justification to earn.
pub const OBSERVED_MIN_DWELL: Duration = Duration::from_secs(3);

/// How long a changed bearer preference must hold before the machine acts.
///
/// 20 s, chosen against the failure rather than against a feel. The gate this
/// replaces reacted at NM-event speed and produced ~7 tunnel recycles a
/// minute; a wifi association that comes and goes during a roam settles well
/// inside 20 s, and a genuine bearer change (walking out of range) does not
/// reverse itself within one. The cost of being slow here is 20 s on the wrong
/// link; the cost of being fast was an unusable phone.
pub const BEARER_SETTLE: Duration = Duration::from_secs(20);

/// How long a button must stay down to be a hold rather than a tap.
///
/// **Provisional, and the number is the weakest part of this file.** §9.5's
/// lesson is the standing warning here: the proximity debounce was specified
/// from Android's prior art, and when it came time to build it the trail said
/// the specification was wrong for a device whose constraints had changed. 500
/// ms is AOSP's long-press default and it is a *reference*, not a measurement.
/// The trail now records every gesture with the duration that produced it, so
/// this can be set from real presses the way `PROXIMITY_NEAR_DEBOUNCE` was —
/// do that before defending the value.
pub const BUTTON_HOLD: Duration = Duration::from_millis(500);

/// Held past this, still down. The point where a destructive binding may fire
/// without the user having meant a hold.
pub const BUTTON_LONG_HOLD: Duration = Duration::from_millis(2000);

/// How long after a release to keep waiting for another tap.
///
/// This one is felt directly and in the wrong direction: it is the delay
/// between a single tap and anything happening, because a single cannot fire
/// until the window proves no second tap is coming. Too long and the phone
/// feels broken; too short and a double-tap fires a single first. 300 ms is
/// AOSP's double-tap timeout. Same caveat as above — measure it.
pub const BUTTON_MULTI_TAP_WINDOW: Duration = Duration::from_millis(300);

/// How long a blank waits for the compositor to acknowledge the lock it asked
/// for before going dark anyway.
///
/// `LOCK-DPMS-LESSONS.md` §1 ("Ordering: lock, then off") fixes this at ≤2 s
/// for the power-button path and gives the reason for the fail-open: "if the
/// lock IPC fails, blank anyway — dark-but-unlocked is recoverable,
/// lit-and-unlocked in a pocket is not." Doctrine §8 supplies the other half:
/// the panel may proceed, but the session is never *recorded* as locked. The
/// timeout is an `error-security` in the trail, not a silent success.
pub const LOCK_ACK_BUDGET: Duration = Duration::from_secs(2);

/// Tunable policy for the machine's timed behaviour.
///
/// These are settings, not constants: the shape is iOS's Auto-Lock — a
/// user-chosen timeout with a visible dim shortly before it, and a "never"
/// option for the desk-clock case. The Settings control center (TASK-19) is
/// meant to be a view over this struct, so the values live here rather than
/// being baked into the rule.
/// `serde(default)` at the container level, so a policy file written by an
/// older sessiond loads with the new fields filled from `Default` instead of
/// failing outright. Without it, adding one field here would make every
/// existing device fall back to built-in timers on the next upgrade and lose
/// whatever the user had set — silently, which is the failure mode this whole
/// area keeps producing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceStatePolicy {
    /// Locked, panel lit, no input for this long → blank. `None` = never.
    pub lock_blank_after: Option<Duration>,
    /// Same, when evidence says the device is being held.
    pub lock_blank_after_held: Option<Duration>,
    /// How long the dimmed warning shows before the blank.
    pub dim_grace: Duration,
    /// Evidence older than this is unknown.
    pub evidence_ttl: Duration,
    /// Whether the dim pre-warning is used at all.
    pub dim_warning: bool,
    /// How long a pending blank waits for the lock it asked for.
    pub lock_ack_budget: Duration,
    /// Unlocked, panel lit, no input for this long → lock, then blank.
    /// `None` = never, and that is the default: the shell's `IdleCoordinator`
    /// owns the unlocked idle→lock timer through `ext-idle-notify` (doctrine
    /// §5), and a second unlocked timer here would recreate the competing-owner
    /// disease this machine exists to end. The field is settable so the
    /// capability is real rather than implied.
    pub unlocked_blank_after: Option<Duration>,
    /// A source silent for this long, having previously reported, is down.
    pub source_down_after: Duration,
    /// Grace from daemon start before an expected-but-silent source is
    /// reported `Absent`. See `SOURCE_EXPECTED_WITHIN`.
    pub source_expected_within: Duration,
    /// How long proximity must hold `near` before the machine believes it.
    pub proximity_near_debounce: Duration,
    /// How long proximity must hold `far` before the machine believes it.
    pub proximity_far_debounce: Duration,
    /// SSIDs that are the home LAN.
    ///
    /// Identity, never an address prefix. The gate this replaces matched a /16
    /// address prefix and so read the foreign network this phone lives on — a
    /// neighbouring /24 — as home, taking the tunnel down as redundant. Empty
    /// means "never claim to be home", which is the safe default: an unknown
    /// network is not a reason to drop the tunnel.
    pub home_ssids: Vec<String>,
    /// How long a bearer preference must hold before the machine acts on it.
    ///
    /// This is the entire anti-flap mechanism, and it is a debounce rather
    /// than a rate limit on purpose. the old dispatcher gate recycled the tunnel 652
    /// times in 90 minutes because every correction re-entered the controller;
    /// a decision that has to survive a settling window cannot oscillate at
    /// event speed no matter how the events arrive.
    pub bearer_settle: Duration,
}

impl DeviceStatePolicy {
    /// Where the user's choices live between runs.
    ///
    /// They did not live anywhere until 2026-07-26. `SetPolicy` mutated the
    /// in-memory struct and nothing wrote it down, so every lock-screen timer
    /// set in Settings survived exactly until the next restart and then
    /// silently reverted to the built-in 15 s. The Settings page was honest
    /// about reading the daemon — the daemon was the one forgetting.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("souveraine").join("device-state-policy.json"))
    }

    /// Load the saved policy, or the defaults.
    ///
    /// A file that exists but cannot be read or parsed is LOUD and then
    /// ignored: continuing on built-in timers is the only thing a session
    /// authority can do at startup, but doing it quietly would present
    /// defaults as if they were the user's settings.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            warn!(
                "[device-state] no HOME or XDG_CONFIG_HOME — policy cannot be persisted this run"
            );
            return Self::default();
        };
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                warn!("[device-state] policy at {} is unreadable ({e}) — RUNNING ON DEFAULTS, not on your settings", path.display());
                return Self::default();
            }
        };
        match serde_json::from_str(&raw) {
            Ok(p) => {
                info!("[device-state] policy loaded from {}", path.display());
                p
            }
            Err(e) => {
                warn!("[device-state] policy at {} is malformed ({e}) — RUNNING ON DEFAULTS, not on your settings", path.display());
                Self::default()
            }
        }
    }

    /// Write the policy out. Errors are returned, never swallowed — the caller
    /// tells the Settings page, so a control that appears to have taken effect
    /// has actually taken effect past the next reboot.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "no HOME or XDG_CONFIG_HOME".to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serializing the policy: {e}"))?;
        // Write-then-rename, so a crash mid-write cannot leave a truncated
        // file that the next boot reads as malformed and discards.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("renaming into {}: {e}", path.display()))?;
        Ok(())
    }
}

impl Default for DeviceStatePolicy {
    fn default() -> Self {
        Self {
            lock_blank_after: Some(LOCK_BLANK_AFTER),
            lock_blank_after_held: Some(LOCK_BLANK_AFTER_HELD),
            dim_grace: LOCK_DIM_GRACE,
            evidence_ttl: EVIDENCE_TTL,
            dim_warning: true,
            lock_ack_budget: LOCK_ACK_BUDGET,
            unlocked_blank_after: None,
            source_down_after: SOURCE_DOWN_AFTER,
            source_expected_within: SOURCE_EXPECTED_WITHIN,
            proximity_near_debounce: PROXIMITY_NEAR_DEBOUNCE,
            proximity_far_debounce: PROXIMITY_FAR_DEBOUNCE,
            home_ssids: Vec::new(),
            bearer_settle: BEARER_SETTLE,
        }
    }
}

/// What the machine wants done. The machine decides; it never runs a command
/// itself. `tick` returns intent and the daemon executes it, so the decision
/// stays testable without a compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Dim the panel as the pre-warning. Saves the current brightness first.
    Dim,
    /// Restore brightness after a dim. Runs `blueline-undim`, never a bare
    /// `brightnessctl -r`: the restore needs the floor, or a save that landed
    /// while already dim leaves the panel at 10/255 and every later wake
    /// looks like dead glass (Pixel3Arch b9f83f0).
    Restore,
    /// Blank the panel through the DPMS executor.
    Blank,
    /// One step of output volume, up or down.
    ///
    /// Under Hyprland these were `wpctl` calls bound to `XF86AudioRaiseVolume`
    /// / `LowerVolume` in `hyprland.lua`, with key repeat. viewtop consumes
    /// the volume keys as device buttons — they must not be interceptable by a
    /// fullscreen client any more than the power button is — so the binding
    /// had to come somewhere, and §12 is explicit that the somewhere is an
    /// `Action` through this table rather than a small daemon that reads a
    /// signal and calls a tool. A second writer to the sink is the eighth
    /// blind actor.
    ///
    /// Emitted on the **down edge**, not from a recognised gesture:
    /// `BUTTON_MULTI_TAP_WINDOW` is 300 ms, so a tap-driven volume key would
    /// lag a third of a second behind the press and feel broken. Volume is one
    /// of the two controls whose whole quality is immediacy.
    Volume { up: bool },
    /// Show the volume the step just set.
    ///
    /// The shell owns the indicator; the machine owns what raises it, which is
    /// the same split `WindowSheet` already makes. It has to come from here
    /// because the keys never reach the shell: viewtop consumes them as device
    /// buttons, so the compositor global shortcut the shell registers for this
    /// (`osdVolumeTrigger`) has no chord that can ever arrive.
    ///
    /// Separate from [`Action::Volume`] rather than folded into it: the
    /// executor runs one named tool with named arguments per action, and
    /// changing the volume and drawing the volume are two tools. Emitted
    /// beside it on the same edge, so the indicator and the sound move
    /// together.
    VolumeOsd,
    /// Light the panel through the same executor.
    ///
    /// The machine could turn the screen off and had no way to turn it back
    /// on: `apply_gesture` answers a power tap on a dark panel with `Restore`,
    /// which is *brightness*, and there was no unblank in this table at all.
    /// That worked only because the wake never came through the daemon —
    /// `hyprland.lua` bound the physical key straight to
    /// `blueline-screen-toggle`, so the compositor woke the panel and sessiond
    /// merely heard about it afterwards. Under viewtop the compositor consumes
    /// that key (a device button must not be interceptable by whatever is
    /// fullscreen), and the gap became a phone that could sleep and never wake.
    ///
    /// So the machine owns both directions now, which is what §1 asks for in
    /// the first place: seven blind actors became one authority, and an
    /// authority that can only act in one direction is half an authority. It
    /// also makes waking something the agent can *reach* — doctrine §13, a
    /// verb she cannot reach is a defect — rather than a side effect of a
    /// keybinding in a compositor config she has no say over.
    ///
    /// Unlike [`Action::Blank`] this carries no ordering obligation: §1 is
    /// about paths to a *dark* panel. Lighting one discloses nothing that the
    /// lock surface is not already responsible for covering, and
    /// `LOCK-DPMS-LESSONS.md` §7 says it outright — "wake sources move the
    /// panel, only sessiond moves the lock".
    Unblank,
    /// Raise the window action sheet for one window.
    ///
    /// A touch gesture bound the way every other control is: viewtop reports
    /// what the fingers did, the machine decides what it means, and the
    /// behaviour leaves here through the executor table. §12 is explicit that
    /// the alternative — the compositor recognising three fingers and calling
    /// `qs ipc` itself — is the eighth blind actor, and the old dispatcher gate is what
    /// that costs: 652 tunnel recycles in ninety minutes with no way to turn it
    /// off.
    ///
    /// The compositor has to do the *recognition* because contacts only exist
    /// there, which is why this arrives already named rather than as raw
    /// touches. What it must not do is decide what a name means.
    ///
    /// This replaced `Overview`, which the same tap used to raise. Two gestures
    /// reaching one surface is the one-decider problem in miniature: the rail's
    /// swipe already opens the overview, so the tap spent its whole existence
    /// duplicating a gesture the thumb already had. `target` is what makes the
    /// difference — the sheet is *about a window*, and the tap now knows which.
    WindowSheet { target: u64 },
    /// Raise the power menu — the device's own verbs, on a hold.
    ///
    /// The counterpart to [`Self::WindowSheet`]: that one is about a window,
    /// this one is about the device. Restart, power off, and the USB-C port's
    /// state — whether it is host or device, whether it is sourcing or sinking
    /// power, and what is on the other end.
    ///
    /// Those last ones are here rather than in a settings page because the port
    /// is not a preference; it is what the machine currently *is*, and it
    /// changes what every other peripheral means. Casey, 2026-08-05: "that
    /// whole power usb state probably belongs in the power switch options".
    PowerMenu,
    /// End the session's power state: off, restart, or asleep.
    ///
    /// The counterpart to every other row here, on the one transition that
    /// cannot be walked back. It arrives as an `Action` for the same reason
    /// `Volume` and `WindowSheet` do — §12 forbids the small actor that reads a
    /// signal and calls a tool — but the stakes are the argument's strongest
    /// case rather than its weakest: a poweroff executed outside this table
    /// leaves no entry anywhere, and there is no later moment to notice.
    Power(PowerVerb),
    /// Change the port's gadget posture through usb-signaller.
    ///
    /// The mode daemon is the mechanism and souveraine-upower is its adjacent
    /// power sensor. Neither decides. sessiond emits this action so the mode
    /// change is named, audited, and later refusable when a known peer or an
    /// active probe lease says the port is occupied.
    UsbMode(UsbMode),
    /// Request the session lock, because something wants the panel dark and
    /// the session is not locked yet.
    ///
    /// This is the machine's half of `LOCK-DPMS-LESSONS.md` §1 — the ordering
    /// is lock, *then* off, and it is an invariant the authority enforces
    /// rather than a coincidence of two timers (hypridle's 300 s lock landing
    /// before its own 600 s blank). The daemon routes this to the shell's lock
    /// surface while a live shell owns steady state, and takes the lock itself
    /// otherwise; either way the panel does not go dark until it is answered
    /// or the ack budget expires.
    Lock,
    /// Carry ordinary traffic over this bearer.
    ///
    /// Expressed to NetworkManager as a route metric, never written with `ip
    /// route`: NM stays the single writer of routes, which is TASK-49's
    /// acceptance #6. Emitted only when the decision *changes* and has held
    /// for `bearer_settle`, so a link flapping at event speed cannot produce
    /// a command per event — the failure that recycled the tunnel 652 times.
    PreferLink(Bearer),
    /// Pin the WireGuard endpoint's host route to this bearer.
    ///
    /// The tunnel is not a bearer; it rides one. Over clat it was measured
    /// sending and never receiving, and pinning the endpoint via wlan0
    /// produced a handshake in seconds. This does not turn the tunnel on or
    /// off — that switch is the user's and stays the user's.
    PinTunnelUnderlay(Bearer),
}

/// When each evidence source last reported. Kept beside `SensorEvidence`
/// rather than inside it so the weights stay a pure function of the readings.
#[derive(Debug, Clone, Default)]
pub struct EvidenceSeen {
    pub proximity: Option<Instant>,
    pub accel: Option<Instant>,
    pub light: Option<Instant>,
    pub touch: Option<Instant>,
    pub charge: Option<Instant>,
}

impl EvidenceSeen {
    /// Last time this source was heard from, if ever. `None` means never —
    /// which is a different claim from "not recently", and the one the
    /// `Absent` check is built on.
    pub fn get(&self, source: SensorSource) -> Option<Instant> {
        match source {
            SensorSource::Proximity => self.proximity,
            SensorSource::Accelerometer => self.accel,
            SensorSource::Light => self.light,
            SensorSource::Touch => self.touch,
            SensorSource::Charge => self.charge,
        }
    }
}

/// Whether an evidence source is *there*, as opposed to what it last said.
///
/// Doctrine §9 says sensor readings are evidence, not fact. This is the
/// sentence underneath that one: the machine must also know whether it is
/// receiving evidence at all. `DEVICE-STATE-MACHINE.md` §0 states the
/// requirement — "'No evidence' and 'evidence says nothing is happening' must
/// not be the same state" — and until now they were.
///
/// Health is deliberately NOT read from `net.hadess.SensorProxy`'s
/// `HasProximity`/`HasAccelerometer` properties, which is the obvious place to
/// look and is wrong. Those lie in both directions, measured: on 2026-07-25 at
/// 12:19 the stack was dead and the proxy answered `HasProximity: true` for
/// hours (`PAF/slpi.md`), and in the 06:04 incident it answered `false` while
/// remoteproc had already recovered SLPI. A property that is wrong both ways is
/// not an authority. What the machine can actually trust is its own experience:
/// whether readings arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceHealth {
    /// Nothing has ever been heard from this source, and nothing is expected
    /// to be. Correct for hardware with no reporter: `Touch` has none, so it
    /// sits here forever and says nothing. A signal that fires for a source
    /// nobody wired up is a signal nobody reads.
    #[default]
    Unknown,
    /// Reporting within `source_down_after`.
    Live,
    /// Reported once and has now been silent past the threshold.
    Down,
    /// A reporter is expected to serve this source and it has never once
    /// spoken, past `source_expected_within` from daemon start.
    ///
    /// This variant exists because `Unknown` was silent by design and that
    /// silence hid a real outage for a whole boot (2026-07-27). `Down` cannot
    /// catch it: `evaluate_source_health` starts from the last-seen stamp, and
    /// a source that has never reported has no stamp, so it is skipped and
    /// stays `Unknown` forever. `souveraine-sensord` — the one reporter for
    /// every iio-sensor-proxy source (§12) — was `enabled` but never started,
    /// because its unit hung off `graphical-session.target` and nothing on
    /// this device starts that target. The machine ran the entire session with
    /// zero evidence, and the trail recorded not one line about it.
    ///
    /// §10 is explicit that "no evidence" and "evidence says nothing is
    /// happening" must not be the same state. It made that true for a source
    /// that dies mid-session. This makes it true for one that never lived.
    Absent,
}

impl SourceHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceHealth::Unknown => "unknown",
            SourceHealth::Live => "live",
            SourceHealth::Down => "down",
            SourceHealth::Absent => "absent",
        }
    }
}

/// Per-source health, in the same shape as `EvidenceSeen`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceHealthTable {
    pub proximity: SourceHealth,
    pub accel: SourceHealth,
    pub light: SourceHealth,
    pub touch: SourceHealth,
    pub charge: SourceHealth,
}

impl SourceHealthTable {
    fn get_mut(&mut self, source: SensorSource) -> &mut SourceHealth {
        match source {
            SensorSource::Proximity => &mut self.proximity,
            SensorSource::Accelerometer => &mut self.accel,
            SensorSource::Light => &mut self.light,
            SensorSource::Touch => &mut self.touch,
            SensorSource::Charge => &mut self.charge,
        }
    }

    /// True when any source the machine should be hearing from is not
    /// arriving — whether it died mid-session (`Down`) or never started
    /// (`Absent`). This is the single question the rest of the system asks: a
    /// surface showing "sensors degraded" does not need to know which one, and
    /// a decision taken on absent evidence is no sounder than one taken on
    /// evidence that stopped.
    pub fn any_down(&self) -> bool {
        [
            self.proximity,
            self.accel,
            self.light,
            self.touch,
            self.charge,
        ]
        .iter()
        .any(|h| matches!(h, SourceHealth::Down | SourceHealth::Absent))
    }

    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "proximity": self.proximity.as_str(),
            "accel": self.accel.as_str(),
            "light": self.light.as_str(),
            "touch": self.touch.as_str(),
            "charge": self.charge.as_str(),
        })
    }
}

// ── Forensic logging ─────────────────────────────────────────────────
// Every decision point emits a ForensicEntry capturing the full state
// at that moment. These are appended to a JSONL file alongside the
// SessionAudit trail. The audit trail says "lock-requested"; the
// forensic trail says "lock-requested because idle timer fired at T,
// state was Active, no inhibitor held, IdleCoordinator was at Dimmed,
// last sensor input was proximity-far at T-30s."

/// How large the live trail may grow before it rotates.
///
/// The trail was unbounded until 2026-07-26 and measured at ~3.6 MB/day, so
/// "unbounded" meant "fills the disk on a device that has no room for it".
/// Bounding it is the other half of making it durable: a file that survives
/// reboot is only an improvement if it also stops growing.
pub const FORENSIC_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How many rotated generations are kept behind the live file.
///
/// Three files in total, so the ceiling is 12 MiB — about three days at the
/// volume measured before proximity debounce (§9.5) exists, and considerably
/// more once it does. The number is a floor on how far back an incident can be
/// reconstructed, which is the only thing it is for.
pub const FORENSIC_KEEP: usize = 2;

/// A forensic entry — one decision point in the device state machine.
///
/// `hash` is deliberately not a field here. It is computed over this struct's
/// serialization and injected into the written line as the last key, exactly
/// as `SessionAudit.qml` does, so one verifier reads both trails: strip the
/// trailing `,"hash":"<hex>"`, close the object, SHA-256, compare.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForensicEntry {
    /// Monotonic sequence (from the audit trail).
    pub seq: u64,
    /// The hash of the previous entry, empty at the head of a chain.
    ///
    /// §5 called this trail tamper-evident while it carried nothing but a
    /// sequence number, which detects a gap and nothing else — any past entry
    /// could be edited in place and the file still read as consistent. The
    /// chain is what the word was always claiming.
    #[serde(default)]
    pub prev: String,
    /// Wall-clock timestamp (seconds since epoch).
    pub ts: u64,
    /// What happened.
    pub event: ForensicEvent,
    /// Full state snapshot at this moment.
    pub snapshot: StateSnapshot,
    /// Why this decision was made (human-readable).
    pub reason: String,
    /// The caller's declared intent, when one was given.
    ///
    /// A chain is one decision and several verbs (doctrine §13). Without this
    /// the trail records the leaves and loses the thing that produced them —
    /// reconstruction sees four unrelated calls and cannot tell a considered
    /// sequence from four accidents. Skipped entirely when absent, so entries
    /// without an intent serialize byte-for-byte as they did before and the
    /// hash contract does not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

/// What kind of forensic event occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicEvent {
    /// A state transition happened (or was refused).
    Transition {
        from: DeviceState,
        to: DeviceState,
        legal: bool,
    },
    /// Sensor input was received and evaluated.
    SensorInput {
        source: SensorSource,
        value: SensorValue,
        confidence: f32,
    },
    /// A wake event occurred (screen on, dt2w, power button).
    Wake { trigger: WakeTrigger },
    /// An error occurred that affected device state.
    Error {
        component: String,
        action: String,
        error: String,
    },
    /// A decision was made (e.g., "suppress DPMS wake" or "promote idle").
    Decision {
        decision: String,
        inputs: serde_json::Value,
    },
    /// The somatic plexus spoke — a source went silent or recovered, a trend
    /// shifted, or a viability variable is heading out of range. The body
    /// reporting on itself, in the body's own vocabulary.
    Somatic {
        field: String,
        kind: String,
        detail: String,
    },
    /// Periodic heartbeat snapshot (for reconstructing timeline gaps).
    Heartbeat,
}

/// What triggered a wake event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeTrigger {
    PowerButton,
    DoubleTapToWake,
    Squeeze,
    ProximityFar,
    RtcAlarm,
    ModemIrq,
    UserInput,
    Unknown,
}

/// Full state snapshot — everything needed to reconstruct the decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub device_state: DeviceState,
    pub locked: bool,
    pub display_active: bool,
    pub phase: String,
    pub shell_alive: bool,
    pub proximity_near: bool,
    pub confidence: f32,
    pub suppress_dpms_wake: bool,
    pub promote_idle_faster: bool,
    pub screen_locked: bool,
    pub screen_lock_secure: bool,
    pub idle_coordinator_state: String,
    pub sleep_inhibitor_held: bool,
    /// True when some evidence source that used to report has gone silent.
    ///
    /// On the snapshot rather than only on the health entry because this is
    /// what makes the trail diagnostic after the fact: every decision taken
    /// during an outage is stamped with the fact that the machine was deciding
    /// on absent evidence. Without it, a `panel-off` recorded during four dead
    /// hours is indistinguishable from a healthy one, and the log answers
    /// "what happened" but not "why was it wrong".
    pub sensors_degraded: bool,
    /// Where the machine believed the phone was, and how sure it was.
    ///
    /// On every snapshot for the same reason `sensors_degraded` is: a decision
    /// is only reconstructable if the trail records what it was deciding on.
    /// "Refused a wake" and "refused a wake believing this was a pocket at 0.35"
    /// are the same line and different facts.
    pub placement: Placement,
}

/// Forensic log — accumulates entries for post-hoc analysis.
/// Thread-safe; entries can be added from any thread.
pub struct ForensicLog {
    entries: Arc<Mutex<Vec<ForensicEntry>>>,
    /// Sequence, chain head, and file health under **one** lock.
    ///
    /// They were two locks and a write that happened after both were dropped,
    /// which was survivable while the only content was a sequence number and
    /// is not once entries chain: two threads could take the same `prev` and
    /// write in either order, and the resulting file would read as tampered.
    writer: Arc<Mutex<TrailWriter>>,
}

/// The durable half of the trail: which file, where the chain is, and whether
/// writing is currently working.
struct TrailWriter {
    /// `None` means in-memory only — no writable home (LOUD at startup), or a
    /// test that has no business touching the real trail.
    path: Option<PathBuf>,
    next_seq: u64,
    /// Hash of the last entry written, which the next entry's `prev` carries.
    /// Deliberately **not** reset by rotation: the chain runs across the file
    /// boundary, so a rotated set verifies end to end rather than as three
    /// unrelated logs.
    last_hash: String,
    /// Bytes in the live file, tracked rather than stat'd per append.
    bytes: u64,
    /// The rotation threshold. A field rather than the constant read inline so
    /// the rotation rules can be tested at a few hundred bytes instead of by
    /// writing 12 MiB of real entries.
    max_bytes: u64,
    /// The intent the current caller declared, stamped onto every entry
    /// produced while it is set. Lives here because `append` already takes
    /// this lock — anywhere else would need threading through every call site
    /// that records anything.
    intent: Option<String>,
    /// True while writes are failing, so the warning is one per outage edge
    /// instead of one per tick. A trail that cannot write is exactly the kind
    /// of failure that must not drown out what it was recording.
    failed: bool,
}

impl ForensicLog {
    /// Open the durable trail at its standard path.
    pub fn new() -> Self {
        // Tests get an in-memory trail unless they ask for a file. Otherwise
        // every `DeviceStateMachine::new()` in the suite would append to the
        // developer's real trail and rotate it — the file-backed behaviour is
        // covered by tests that name their own path.
        #[cfg(test)]
        let path = None;
        #[cfg(not(test))]
        let path = match forensic_log_path() {
            Some(p) => Some(p),
            None => {
                warn!(
                    "[device-state] no HOME or XDG_STATE_HOME — the forensic trail is MEMORY-ONLY this run and dies with the daemon"
                );
                None
            }
        };
        Self::open(path)
    }

    /// Open the trail at an explicit path.
    #[cfg(test)]
    pub fn with_path(path: PathBuf) -> Self {
        Self::open_bounded(Some(path), FORENSIC_MAX_BYTES)
    }

    /// Open the trail with an explicit rotation threshold. Tests only — the
    /// live bound is `FORENSIC_MAX_BYTES` and is not a per-caller choice.
    #[cfg(test)]
    pub fn with_path_bounded(path: PathBuf, max_bytes: u64) -> Self {
        Self::open_bounded(Some(path), max_bytes)
    }

    fn open(path: Option<PathBuf>) -> Self {
        Self::open_bounded(path, FORENSIC_MAX_BYTES)
    }

    fn open_bounded(path: Option<PathBuf>, max_bytes: u64) -> Self {
        let writer = match path {
            Some(path) => TrailWriter::open(path, max_bytes),
            None => TrailWriter::memory_only(),
        };
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    /// Where the trail is being written, or `None` when it is memory-only.
    pub fn path(&self) -> Option<PathBuf> {
        self.writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .path
            .clone()
    }

    /// How the chain the daemon just opened relates to the one before it.
    pub fn chain_state(&self) -> &'static str {
        let w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if w.path.is_none() {
            "memory-only"
        } else if w.next_seq == 0 {
            "new"
        } else {
            "resumed"
        }
    }

    /// Append a forensic entry. The seq is auto-incremented and the entry is
    /// chained to the one before it, then written to the durable trail.
    pub fn append(&self, event: ForensicEvent, snapshot: StateSnapshot, reason: &str) {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let entry = ForensicEntry {
            seq: w.next_seq,
            prev: w.last_hash.clone(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            event,
            snapshot,
            reason: reason.to_string(),
            intent: w.intent.clone(),
        };
        w.next_seq += 1;
        w.write(&entry);
        drop(w);

        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry);
        // Keep last 1000 entries in memory (the JSONL file is the
        // durable store; this is for IPC queries).
        let len = entries.len();
        if len > 1000 {
            entries.drain(0..len - 1000);
        }
    }

    /// Declare what the caller is doing. Every entry recorded until this is
    /// cleared carries it. Set around one request, not held across them — an
    /// intent that outlives its chain mislabels whatever comes next.
    pub fn set_intent(&self, intent: Option<String>) {
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).intent = intent;
    }

    /// Get recent entries (for IPC queries).
    pub fn recent(&self, count: usize) -> Vec<ForensicEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let start = entries.len().saturating_sub(count);
        entries[start..].to_vec()
    }

    /// Entries recorded after `seq`, oldest first.
    ///
    /// The cursor is the sequence number the trail already mints for its hash
    /// chain, so a consumer that reconnects resumes exactly where it stopped
    /// and a restarted daemon cannot silently replay — the chain and the
    /// stream agree by construction rather than by a second counter kept in
    /// step by hand.
    pub fn since(&self, seq: u64) -> Vec<ForensicEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().filter(|e| e.seq > seq).cloned().collect()
    }

    /// The highest sequence the in-memory buffer holds.
    pub fn head_seq(&self) -> u64 {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.last().map(|e| e.seq).unwrap_or(0)
    }
}

/// One button's edges, accumulating into gestures.
///
/// This is the chordotonal principle applied to a button: the thing that turns
/// a run of drivers (down, up, down, up) into one piece of evidence ("double
/// tap") lives below the decision, and only the compressed answer travels
/// upward. A consumer never sees the edges.
///
/// Deliberately per-button and stateless about *meaning*. It recognises; it
/// does not decide what a triple-tap is for. Binding is policy and lives above.
#[derive(Debug, Clone, Default)]
pub struct ButtonRecognizer {
    /// When the button went down, while it is down.
    down_since: Option<Instant>,
    /// Holds already announced for the current press, so `tick` does not
    /// re-fire them every second while a finger rests on the button.
    hold_fired: bool,
    long_hold_fired: bool,
    /// Releases that have not yet resolved into a tap gesture, and when the
    /// most recent one landed.
    pending_taps: u8,
    last_release: Option<Instant>,
}

impl ButtonRecognizer {
    /// A press edge. Returns nothing — a press alone is never a gesture, and
    /// pretending otherwise is what makes a double-tap fire a single first.
    pub fn down(&mut self, now: Instant) {
        self.down_since = Some(now);
        self.hold_fired = false;
        self.long_hold_fired = false;
    }

    /// A release edge. Returns a gesture only when the release *completes* one
    /// immediately — which is never, for taps. A hold that already fired
    /// resolves to nothing here: it was announced while the button was down,
    /// and announcing it again on release would double-fire every binding.
    pub fn up(&mut self, now: Instant) -> Option<ButtonGesture> {
        let held = self.down_since.take().map(|t| now.duration_since(t));
        if self.hold_fired {
            // A hold was already announced. Releasing ends it and starts no
            // tap — a long press is not also a tap, and counting it as one is
            // how "hold to power off" also toggles your screen.
            self.pending_taps = 0;
            self.last_release = None;
            return None;
        }
        if held.is_some_and(|d| d >= BUTTON_HOLD) {
            // Held long enough, but tick never saw it (a press shorter than one
            // tick interval that still crossed the threshold). Announce now.
            self.pending_taps = 0;
            self.last_release = None;
            return Some(ButtonGesture::Hold);
        }
        self.pending_taps = self.pending_taps.saturating_add(1);
        self.last_release = Some(now);
        None
    }

    /// Called on every tick. Fires holds while the button is still down, and
    /// resolves pending taps once the multi-tap window has closed.
    pub fn tick(&mut self, now: Instant) -> Option<ButtonGesture> {
        if let Some(since) = self.down_since {
            let held = now.duration_since(since);
            if held >= BUTTON_LONG_HOLD && !self.long_hold_fired {
                self.long_hold_fired = true;
                return Some(ButtonGesture::LongHold);
            }
            if held >= BUTTON_HOLD && !self.hold_fired {
                self.hold_fired = true;
                return Some(ButtonGesture::Hold);
            }
            return None;
        }
        let last = self.last_release?;
        if now.duration_since(last) < BUTTON_MULTI_TAP_WINDOW {
            return None;
        }
        let n = std::mem::take(&mut self.pending_taps);
        self.last_release = None;
        match n {
            0 => None,
            1 => Some(ButtonGesture::Tap),
            2 => Some(ButtonGesture::DoubleTap),
            // Four taps is a triple plus a stray, not a new gesture. Saturating
            // here beats inventing a QuadrupleTap nobody asked for.
            _ => Some(ButtonGesture::TripleTap),
        }
    }
}

/// Is this entry worth waking a mind for?
///
/// The whole value of the subscribe stream is this predicate. §10 is the
/// argument in miniature: a sensor resting at `far` and a sensor whose stack
/// took a CHRE fatal produce byte-identical silence, so "no evidence" and
/// "evidence says nothing is happening" had to become different states. The
/// same distinction decides what crosses to the agent — an edge means
/// something, a level does not.
///
/// What crosses:
/// - **Transitions**, legal or refused. A refused one is the more interesting:
///   the machine wanted to move and its own guard said no.
/// - **Errors.** Both classes. `error-security` is a violated guarantee;
///   `error-operational` is an actuator that did not do as it was told, which
///   is how a `panel-off` recorded during four dead hours stops reading like a
///   healthy one.
/// - **Decisions**, which is where `source-down` / `source-recovered` and
///   cross-sensor disagreement already land.
///
/// What never crosses: `Heartbeat` (it exists to fill timeline gaps for a
/// reader, and a mind is not a reader), `SensorInput` (a reading is a driver
/// wearing evidence's clothes — 2,900 proximity lines a day saying nothing
/// changed, and the exact stream P1 says an inferring model must not have),
/// and `Wake`, which is already implied by the transition it causes. A
/// somatic `trend_shift` stays on the bench for the same reason a raw sensor
/// reading does — the body's idling is evidence; its alarms are decisions.
pub fn is_notable(event: &ForensicEvent) -> bool {
    match event {
        ForensicEvent::Transition { .. } => true,
        ForensicEvent::Error { .. } => true,
        ForensicEvent::Decision { .. } => true,
        ForensicEvent::SensorInput { .. } => false,
        ForensicEvent::Wake { .. } => false,
        ForensicEvent::Somatic { kind, .. } => matches!(
            kind.as_str(),
            "source_silent" | "source_recovered" | "viability_threatened"
        ),
        ForensicEvent::Heartbeat => false,
    }
}

impl TrailWriter {
    fn memory_only() -> Self {
        Self {
            path: None,
            next_seq: 0,
            last_hash: String::new(),
            bytes: 0,
            max_bytes: FORENSIC_MAX_BYTES,
            intent: None,
            failed: false,
        }
    }

    /// Open the trail, continuing the existing chain where there is one.
    fn open(path: PathBuf, max_bytes: u64) -> Self {
        let mut w = Self {
            path: Some(path.clone()),
            next_seq: 0,
            last_hash: String::new(),
            bytes: 0,
            max_bytes,
            intent: None,
            failed: false,
        };

        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                warn!(
                    "[device-state] cannot create {} ({e}) — the forensic trail is MEMORY-ONLY this run",
                    dir.display()
                );
                w.path = None;
                return w;
            }
        }

        let existing = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return w,
            Err(e) => {
                warn!(
                    "[device-state] cannot read the existing trail at {} ({e}) — the forensic trail is MEMORY-ONLY this run",
                    path.display()
                );
                w.path = None;
                return w;
            }
        };

        // Walk back to the last line that parses.
        //
        // A torn *final* line is the expected result of losing power mid-write,
        // not evidence of tampering — measured 2026-07-26, when a hard reboot
        // left the file at exactly 4096 bytes, a page boundary, with the last
        // entry cut in half. Treating that as tampering cost the chain its
        // continuity across precisely the event most worth investigating.
        //
        // So: discard the torn tail, resume from the last good entry, and say
        // so. A bad hash in the *middle* is a different claim and this does not
        // hide it — the verifier still reads every line, and everything up to
        // the tear stays intact and chained.
        let mut good_end = 0usize; // byte offset just past the last good line
        let mut last_good: Option<serde_json::Value> = None;
        for line in existing.split_inclusive('\n') {
            let trimmed = line.trim_end_matches('\n');
            if trimmed.trim().is_empty() {
                good_end += line.len();
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(v) => {
                    good_end += line.len();
                    last_good = Some(v);
                }
                Err(_) => break, // torn or damaged — everything from here goes
            }
        }

        let discarded = existing.len() - good_end;
        if discarded > 0 {
            let Some(_) = last_good.as_ref() else {
                // Nothing in the file parses at all. That is not a torn tail;
                // it is a file we cannot chain onto. Keep it as evidence.
                warn!(
                    "[device-state] no parseable entry in {} — rotating it aside and STARTING A NEW CHAIN; the old file is kept",
                    path.display()
                );
                w.rotate();
                return w;
            };
            warn!(
                "[device-state] discarding {discarded} torn bytes from the tail of {} — a write that did not reach disk, almost certainly a hard reboot; the chain resumes from the last intact entry",
                path.display()
            );
            if let Err(e) = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_len(good_end as u64))
            {
                warn!(
                    "[device-state] could not truncate the torn tail of {} ({e}) — the forensic trail is MEMORY-ONLY this run",
                    path.display()
                );
                w.path = None;
                return w;
            }
        }

        let Some(v) = last_good else {
            return w; // present but empty: a fresh chain, not a damaged one
        };

        w.next_seq = v.get("seq").and_then(|s| s.as_u64()).unwrap_or(0) + 1;
        w.last_hash = v
            .get("hash")
            .and_then(|h| h.as_str())
            .unwrap_or_default()
            .to_string();
        w.bytes = good_end as u64;
        info!(
            "[device-state] forensic trail resumed at {} (seq {}, {} KiB)",
            path.display(),
            w.next_seq,
            w.bytes / 1024
        );
        w
    }

    /// Serialize, hash, and write one entry. Every failure is loud once.
    fn write(&mut self, entry: &ForensicEntry) {
        if self.path.is_none() {
            return;
        }
        let body = match serde_json::to_string(entry) {
            Ok(b) => b,
            Err(e) => {
                // Not a file problem: the entry itself will not serialize.
                // Never silent — a decision that cannot be recorded is one the
                // trail would otherwise imply never happened.
                warn!(
                    "[device-state] forensic entry seq {} will not serialize ({e}) — NOT RECORDED",
                    entry.seq
                );
                return;
            }
        };
        let hash = sha256_hex(&body);
        // The hash goes in as the last key, so the hashed bytes are the line
        // with `,"hash":"<hex>"` removed and the object closed again.
        let line = match body.strip_suffix('}') {
            Some(open) => format!("{open},\"hash\":\"{hash}\"}}\n"),
            None => {
                warn!("[device-state] forensic entry seq {} serialized to something that is not an object — NOT RECORDED", entry.seq);
                return;
            }
        };

        if self.bytes + line.len() as u64 > self.max_bytes {
            self.rotate();
        }

        let Some(path) = self.path.clone() else {
            return;
        };
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(line.as_bytes())
            });

        match written {
            Ok(()) => {
                self.bytes += line.len() as u64;
                if self.failed {
                    self.failed = false;
                    info!(
                        "[device-state] forensic trail at {} is writable again",
                        path.display()
                    );
                }
                // The chain only advances on a line that actually landed.
                // Advancing it on a failed write would leave the next entry
                // pointing at a `prev` no file contains, which reads as
                // tampering rather than as the outage it is.
                self.last_hash = hash;
            }
            Err(e) => {
                if !self.failed {
                    self.failed = true;
                    warn!(
                        "[device-state] CANNOT WRITE the forensic trail at {} ({e}) — decisions are being taken and not recorded",
                        path.display()
                    );
                }
            }
        }
    }

    /// Shift the generations along and start a new live file.
    ///
    /// `last_hash` survives this on purpose: the first entry of the new file
    /// carries the hash of the last entry of the rotated one, so a rotated set
    /// verifies as one chain. A verifier reading a single file in isolation
    /// finds a non-empty `prev` on line 1, which is the correct answer — its
    /// predecessor is the next file along, not nothing.
    fn rotate(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let gen = |n: usize| path.with_extension(format!("jsonl.{n}"));

        let _ = std::fs::remove_file(gen(FORENSIC_KEEP));
        for n in (1..FORENSIC_KEEP).rev() {
            let _ = std::fs::rename(gen(n), gen(n + 1));
        }
        if let Err(e) = std::fs::rename(&path, gen(1)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "[device-state] cannot rotate the forensic trail at {} ({e}) — it will keep growing",
                    path.display()
                );
                return;
            }
        }
        self.bytes = 0;
        info!(
            "[device-state] forensic trail rotated at {} bytes, keeping {} generations",
            self.max_bytes, FORENSIC_KEEP
        );
    }
}

fn sha256_hex(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Path to the forensic log file.
///
/// `$XDG_STATE_HOME/souveraine/forensic.jsonl`, which is `~/.local/state` in
/// practice and is where the XDG spec puts logs — the same directory as
/// `crashes.log`. It was `$XDG_RUNTIME_DIR` until 2026-07-26, which is tmpfs:
/// RAM on a 3.5 GB phone, erased on every reboot. §5 calls this trail
/// tamper-evident and pairs it with the audit hash chain; a file that
/// evaporates when the device restarts cannot be either. Nothing is migrated
/// from the old location because there is never anything there to migrate.
///
/// `None` when there is no home to write to. There is no `/tmp` fallback: a
/// trail nobody can find later is not a trail, and pretending otherwise is how
/// this one spent a month looking durable.
#[cfg(not(test))]
fn forensic_log_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("souveraine").join("forensic.jsonl"))
}

// Re-export for use in protocol.rs
pub use self::ForensicEvent as DeviceStateForensicEvent;
pub use self::WakeTrigger as DeviceStateWakeTrigger;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    /// Screen on, user present, unlocked or lockable.
    Active,
    /// Screen dim, user idle, not yet locked.
    Dimmed,
    /// Screen locked, compositor secure, user may or may not be present.
    /// This is the base locked state. Observed/Doze are sub-states.
    Locked,
    /// Locked + sensor evidence of user presence. EVIDENCE, not FACT.
    /// Gates: DPMS wake suppression, idle tier promotion.
    /// Never gates: lock/unlock, security tiers, personal data.
    Observed,
    /// Locked + idle N min. App tier frozen. Wi-Fi power-save.
    DozeLight,
    /// Locked + idle M min. Network fetchers stopped. RTC wake only.
    DozeDeep,
    /// PrepareForSleep(true). Inhibitor held. Waiting for lock secure.
    Suspending,
    /// s2idle. Panel off. Touch in gesture mode. RTC + modem IRQs only.
    Asleep,
}

/// Transition table — legal (from, to) pairs. Anything not in this list
/// is refused and logged.
const LEGAL_TRANSITIONS: &[(DeviceState, DeviceState)] = &[
    // Active → idle dim/lock
    (DeviceState::Active, DeviceState::Dimmed),
    (DeviceState::Active, DeviceState::Locked),
    // Dimmed → user input or idle lock
    (DeviceState::Dimmed, DeviceState::Active),
    (DeviceState::Dimmed, DeviceState::Locked),
    // Locked → user auth, sensor evidence, or idle promotion
    (DeviceState::Locked, DeviceState::Active),
    (DeviceState::Locked, DeviceState::Observed),
    (DeviceState::Locked, DeviceState::DozeLight),
    // PAM auth unlocks from ANY locked sub-state, not just the base one.
    // Without these, unlocking a phone that was in Observed (proximity had
    // fired) or dozing was refused as an illegal transition.
    (DeviceState::Observed, DeviceState::Active),
    (DeviceState::DozeLight, DeviceState::Active),
    (DeviceState::DozeDeep, DeviceState::Active),
    // Observed → back to Locked (proximity far) or deeper doze
    (DeviceState::Observed, DeviceState::Locked),
    (DeviceState::Observed, DeviceState::DozeLight),
    // DozeLight → user wake or deeper doze
    (DeviceState::DozeLight, DeviceState::Locked),
    (DeviceState::DozeLight, DeviceState::DozeDeep),
    // DozeDeep → user wake (RTC, modem, input)
    (DeviceState::DozeDeep, DeviceState::Locked),
    // Any pre-sleep → Suspending (logind is the authority)
    (DeviceState::Active, DeviceState::Suspending),
    (DeviceState::Dimmed, DeviceState::Suspending),
    (DeviceState::Locked, DeviceState::Suspending),
    (DeviceState::Observed, DeviceState::Suspending),
    (DeviceState::DozeLight, DeviceState::Suspending),
    (DeviceState::DozeDeep, DeviceState::Suspending),
    // Suspending → Asleep (lock secure + inhibitor released)
    (DeviceState::Suspending, DeviceState::Asleep),
    // Asleep → Locked (wake, lock persists)
    (DeviceState::Asleep, DeviceState::Locked),
];

/// Which states count as "locked" for security purposes.
/// Where the phone believes it is. See [`DeviceStateMachine::placement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementBelief {
    Hand,
    Table,
    Pocket,
    /// A call is the only thing that makes a covered sensor mean a face, and
    /// there is no call-state input yet (§4, "Owed: the call"). Never inferred.
    Face,
    Unknown,
}

/// A placement belief and the evidence that carried it.
///
/// The facts travel with the answer so a refusal can say what it believed and
/// why, rather than asserting a conclusion the caller cannot argue with.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Placement {
    pub belief: PlacementBelief,
    pub confidence: f32,
    pub covered: bool,
    pub locked: bool,
    pub lit: bool,
    pub active: bool,
}

/// How sure the machine must be it is in a pocket before it refuses a wake.
///
/// Below what a locked, covered phone earns, so the veto still does the job §4
/// keeps for it — and above what anything else here reaches, so only a pocket
/// reading can fire it. A threshold rather than a hardcoded rule because the
/// call-state input §4 calls owed, or a stillness signal, changes the numbers
/// and not the shape.
const POCKET_VETO_CONFIDENCE: f32 = 0.45;

/// How close together refused double-taps must fall to read as a burst.
///
/// Off the trail, 2026-08-14 (DUMP-taskdocs §2): the only refusal burst on
/// record — all three generations, ~10 MB — is 17 `DoubleTapToWake` refusals
/// in 10 s, tightening from one per 2 s to two per second. That is Casey at a
/// dark screen tapping harder because nothing happened. The veto's true
/// positives are zero; it has never once refused an actual pocket.
pub const DOUBLE_TAP_BURST_WINDOW: Duration = Duration::from_secs(5);

/// How many refused double-taps inside the window override the veto.
///
/// Three. The FTS controller reports DBLTAP only for the deliberate gesture
/// (PAF/touch.md), so an accelerating run of them is a person insisting, and
/// a pocket produces nothing of the kind. The veto keeps the first two — a
/// tap or two against fabric is plausible — and fails open on the third.
pub const DOUBLE_TAP_BURST_COUNT: usize = 3;

pub fn is_locked(state: DeviceState) -> bool {
    matches!(
        state,
        DeviceState::Locked
            | DeviceState::Observed
            | DeviceState::DozeLight
            | DeviceState::DozeDeep
            | DeviceState::Suspending
            | DeviceState::Asleep
    )
}

/// Which states count as "display active" for poller gating.
pub fn is_display_active(state: DeviceState) -> bool {
    matches!(state, DeviceState::Active | DeviceState::Dimmed)
}

/// The device state machine. Owns the current state and enforces
/// transition guards.
pub struct DeviceStateMachine {
    state: DeviceState,
    /// Sensor evidence for the Observed state.
    pub sensor_evidence: SensorEvidence,
    /// Forensic log — captures every decision point for post-hoc analysis.
    pub forensic: ForensicLog,
    /// Panel power. A field, not a ninth state: panel-off is orthogonal to
    /// the doze tier, and the enum had no cell for "locked, screen dark".
    panel_on: bool,
    /// When the compositor's quiet period began, or `None` while the user is
    /// interacting. This is reported by `ext-idle-notify`, never inferred: a
    /// continuous gesture produces no events at all, so a machine that stamped
    /// "last input" itself would blank in the middle of a swipe.
    idle_since: Option<Instant>,
    /// The power verb already in flight, if any.
    ///
    /// Held from admission until the mechanism returns. A second request is
    /// refused rather than executed so that "poweroff, then reboot" cannot
    /// leave the machine having asked for both. The latch is not sleep state:
    /// a zero exit only says logind accepted the command, never that the
    /// physical transition was observed.
    power_requested: Option<PowerVerb>,
    /// Per-source freshness, for the staleness rule.
    evidence_seen: EvidenceSeen,
    /// Per-source health — whether evidence is arriving at all, which is a
    /// different question from what it says. See `SourceHealth`.
    pub source_health: SourceHealthTable,
    /// When this run of the daemon began. The only thing that can tell an
    /// expected source which has not reported *yet* from one that is never
    /// going to — see `SOURCE_EXPECTED_WITHIN`.
    started_at: Instant,
    /// Per-button gesture recognition. Keyed rather than a field per button so
    /// adding volume costs nothing and binds nothing.
    buttons: std::collections::HashMap<Button, ButtonRecognizer>,
    /// When `Observed` was entered, for [`OBSERVED_MIN_DWELL`].
    observed_since: Option<Instant>,
    /// Whether the panel was dark when the current press began.
    ///
    /// Latched on the DOWN edge because **the press itself changes the
    /// answer.** `note_input` fires on that same edge — correctly, a finger on
    /// the button is a user present — which takes the device out of `Locked`,
    /// and the executor lights the panel and reports it back through
    /// [`Self::set_panel`]. By the time the UP edge resolves the tap,
    /// `panel_on` is already true, so the press that woke the screen is read as
    /// a press to blank it: wake, lock screen, black.
    ///
    /// One physical press, two reports, and the second cannot see what the
    /// first did. The level is unreadable by then; only the edge is true. That
    /// makes this the sixth edge-versus-level bug here, after `locked_ack`,
    /// `ChargeRate`, `bootBloomActive`, `hasLoginctl` and the dormant OR in the
    /// compositor's `disclosure_locked`.
    press_began_dark: std::collections::HashMap<Button, bool>,
    /// True once a blank has been asked for, so we ask exactly once per wake
    /// instead of every tick.
    blank_requested: bool,
    /// True while the pre-warning dim is showing. The machine tracks this so
    /// brightness is saved exactly once per dim; the old hypridle `-s/-r`
    /// pair had no such memory, which is how it wedged at 10/255.
    dimmed: bool,
    /// A blank is decided but withheld until the session locks. Carries the
    /// deadline after which the panel goes dark regardless — see
    /// `LOCK_ACK_BUDGET` for why that fail-open is the documented choice.
    pending_blank: Option<Instant>,
    /// The exact panel brightness captured immediately before dimming.
    ///
    /// The dim used to be `brightnessctl -s set 10` and the restore a
    /// save/restore pair with a floor: anything that came back under 26/255
    /// was pushed to 40%. That floor was defending against the old hypridle
    /// `-s`/`-r` wedge — a second save while already dim pinned brightness at
    /// 10 and every later wake looked like dead glass — but sessiond's
    /// `dimmed` field already makes a double save impossible, so the floor was
    /// guarding a bug that no longer exists and corrupting a real setting to
    /// do it. A phone deliberately run dark came back *brighter* than it
    /// started, which is the jump on tap-to-dismiss.
    ///
    /// The machine remembers the number instead of inferring it. `None` means
    /// the capture failed, and only then is the floor the right answer.
    brightness_before_dim: Option<u32>,
    /// The last raw proximity reading, before debounce. `sensor_evidence`
    /// carries the believed value; this carries what the sensor actually said.
    proximity_raw: bool,
    /// When the raw reading last changed. The debounce measures from here.
    proximity_since: Option<Instant>,
    /// Timed policy. Settable, so Settings can own it.
    pub policy: DeviceStatePolicy,
    /// Last bearer posture read off the system. Evidence, refreshed by the
    /// daemon and never inferred here — the same contract `panel_on` has with
    /// the DPMS executor.
    pub bearer: BearerEvidence,
    /// Charge evidence, as reported by sensord through `sensor_input`. The
    /// machine never reads the supplies itself: a decider that also probes is
    /// one refactor away from being a driver (DEVICE-STATE-MACHINE.md §10).
    pub charge: ChargeFields,
    /// The conclusion last recorded, so the trail carries the edge rather
    /// than one entry per report.
    charge_conclusion: Option<&'static str>,
    /// The bearer the machine has actually acted on, and when the current
    /// candidate first differed from it.
    ///
    /// These two fields *are* the anti-flap rule. A preference is only emitted
    /// once it has differed from `bearer_applied` continuously for
    /// `bearer_settle`; anything that reverses inside that window never
    /// produces a command at all. the old dispatcher gate had no such memory, which is
    /// why every NM event it caused became another correction.
    bearer_applied: Option<Bearer>,
    bearer_candidate_since: Option<(Bearer, Instant)>,
    /// Whether the tunnel was deaf last time we looked, so the trail records
    /// the edge rather than one entry per tick.
    bearer_deaf: bool,
    /// When double-tap wakes were refused, one entry each, pruned to
    /// [`DOUBLE_TAP_BURST_WINDOW`]. The memory [`suppress_wake`] does not
    /// have: the veto is a pure function of placement, so the burst escape
    /// lives in `note_input_gated`. Cleared when a wake is allowed, so an
    /// allow never counts toward the next burst.
    double_tap_refusals: std::collections::VecDeque<Instant>,
}

/// Sensor inputs that feed the Observed state. Each is a reading, not
/// an authority — the machine decides what to do with them.
#[derive(Debug, Clone, Default)]
pub struct SensorEvidence {
    /// Proximity: true = near (in-pocket, face-down), false = far.
    pub proximity_near: bool,
    /// Accelerometer: true = device is moving.
    pub accel_moving: bool,
    /// Light sensor: true = ambient light is changing.
    pub light_changing: bool,
    /// Ambient light level, lux, as reported. `changing` is what the
    /// confidence table believes; the number is what the plexus and
    /// auto-brightness act on — levels ride, edges decide.
    pub lux: Option<f64>,
    /// Touch: true = recent touch input detected.
    pub touch_active: bool,
}

impl SensorEvidence {
    /// Compute confidence that the user is present. 0.0–1.0.
    /// Each sensor contributes weighted evidence. Cross-sensor
    /// disagreements reduce confidence (§9: "accelerometer says
    /// face-down, light sensor says bright — one of them is lying").
    pub fn confidence(&self) -> f32 {
        let mut c = 0.0f32;
        if self.proximity_near {
            c += 0.4;
        }
        if self.accel_moving {
            c += 0.3;
        }
        if self.light_changing {
            c += 0.2;
        }
        if self.touch_active {
            c += 0.1;
        }
        // Proximity near while the device is moving is genuinely ambiguous —
        // a phone walking in a pocket and a phone held to an ear read the
        // same. It lowers confidence and nothing more; it does not decide the
        // wake, which is a question about the session, not the sensors.
        if self.proximity_near && self.accel_moving {
            c -= 0.2;
        }
        c.clamp(0.0, 1.0)
    }

    /// Should idle tier promotion be accelerated?
    pub fn should_promote_idle_faster(&self) -> bool {
        self.confidence() >= 0.6
    }
}

impl DeviceStateMachine {
    pub fn new() -> Self {
        let machine = Self {
            state: DeviceState::Active,
            sensor_evidence: SensorEvidence::default(),
            forensic: ForensicLog::new(),
            panel_on: true,
            idle_since: None,
            power_requested: None,
            evidence_seen: EvidenceSeen::default(),
            source_health: SourceHealthTable::default(),
            started_at: Instant::now(),
            buttons: std::collections::HashMap::new(),
            blank_requested: false,
            observed_since: None,
            // Boot comes up lit, so no press is in flight and the latch would
            // only ever be read after a real DOWN edge has set it.
            press_began_dark: std::collections::HashMap::new(),
            dimmed: false,
            pending_blank: None,
            brightness_before_dim: None,
            proximity_raw: false,
            proximity_since: None,
            // Loaded, not defaulted: the user's Auto-Lock choices are settings,
            // and a setting that reverts on reboot is not a setting.
            policy: DeviceStatePolicy::load(),
            bearer: BearerEvidence::default(),
            charge: ChargeFields {
                plugged: None,
                status: None,
                charge_type: None,
                capacity: None,
            },
            charge_conclusion: None,
            bearer_applied: None,
            bearer_candidate_since: None,
            bearer_deaf: false,
            double_tap_refusals: std::collections::VecDeque::new(),
        };

        // First entry of the run, and the only one that proves the trail is
        // writable before something worth recording needs it. It also marks
        // the boot boundary, which the trail could not show while it lived on
        // tmpfs — every reboot simply produced an empty file.
        machine.record_decision(
            "trail-opened",
            serde_json::json!({
                "path": machine.forensic.path().map(|p| p.display().to_string()),
                "chain": machine.forensic.chain_state(),
                "max_bytes": FORENSIC_MAX_BYTES,
                "generations": FORENSIC_KEEP,
            }),
            "sessiond started and opened the forensic trail",
        );
        machine
    }

    /// The clock. Called on a fixed interval by the daemon.
    ///
    /// This is the piece the machine did not have. Everything else was
    /// event-driven, so the machine could only answer other actors and could
    /// never notice that time had passed — which is why it sat in `Active`
    /// with an empty forensic log while the phone locked and unlocked around
    /// it. Every timed tier (blank, dim, doze promotion, wake windows) hangs
    /// off this one function.
    pub fn tick(&mut self) -> Vec<Action> {
        self.tick_at(Instant::now())
    }

    /// `tick` with the clock passed in, so the rules can be tested at any
    /// point on the timeline instead of by sleeping.
    /// A button edge. Returns any actions the resulting gesture asks for.
    ///
    /// A press is user input regardless of what it turns out to mean, so the
    /// idle budget resets on the DOWN edge, not on the gesture. Waiting for
    /// recognition would let the multi-tap window count against a user who is
    /// visibly touching the device.
    pub fn button_edge(&mut self, button: Button, edge: ButtonEdge) -> Vec<Action> {
        self.button_edge_at(button, edge, Instant::now())
    }

    pub fn button_edge_at(
        &mut self,
        button: Button,
        edge: ButtonEdge,
        now: Instant,
    ) -> Vec<Action> {
        // Before anything below can change it. `note_input` on this same edge
        // is what lights the panel, so this is the last moment the answer is
        // still about the screen the user actually pressed against.
        if edge == ButtonEdge::Down {
            // Per button, not one field for all of them.
            //
            // A tap never resolves on the UP edge — `ButtonRecognizer::up`
            // returns `None` and the tap lands when the 300 ms multi-tap window
            // closes on a later tick, up to about 1.3 s after the press. A
            // single shared latch had to survive that whole gap, and any other
            // button's DOWN edge inside it overwrote the answer: power wakes a
            // dark phone, the volume rocker beside it gets nudged within the
            // second, and the pending power tap resolves as "the panel was lit"
            // and blanks the screen. That is the exact regression the latch was
            // added to fix, coming back through a second button.
            //
            // `buttons` is keyed per button for this reason already.
            self.press_began_dark.insert(button, !self.panel_on);
        }
        let rec = self.buttons.entry(button).or_default();
        let gesture = match edge {
            ButtonEdge::Down => {
                rec.down(now);
                None
            }
            ButtonEdge::Up => rec.up(now),
        };
        let mut actions = Vec::new();

        // Volume acts on the press, not on a recognised gesture. See
        // `Action::Volume`: the multi-tap window would otherwise put 300 ms
        // between the press and the sound changing.
        //
        // The gesture recogniser still sees these edges and still resolves
        // taps and holds from them — that is how a future binding table gets
        // volume double-tap or hold-to-ramp without this changing. It simply
        // is not what produces the step today.
        if edge == ButtonEdge::Down && matches!(button, Button::VolumeUp | Button::VolumeDown) {
            actions.push(Action::Volume {
                up: button == Button::VolumeUp,
            });
            // The sound moving with nothing on screen to say so is how volume
            // reads as broken even when it works — which is exactly how it
            // read tonight.
            actions.push(Action::VolumeOsd);
        }

        if edge == ButtonEdge::Down {
            // A hardware button is intent — §4's table says of the power button
            // "can it lie? no — hardware signal" — so it is never subject to the
            // proximity veto that governs tap-to-wake and squeeze. Input is
            // noted on the DOWN edge because a finger on the button is a user
            // present, whatever the press turns out to mean; making the idle
            // budget wait for recognition would count the multi-tap window
            // against someone visibly touching the device.
            actions.extend(self.note_input(InputTrigger::PowerButton));
        }
        if let Some(g) = gesture {
            actions.extend(self.apply_gesture(button, g, now));
        }
        actions
    }

    /// Bind a recognised gesture to behaviour.
    ///
    /// **This match is a placeholder for a binding table.** The end state is
    /// Activator-shaped: `(button, gesture) -> action` as editable, persisted
    /// data, so a triple-tap can be pointed at something without a rebuild.
    /// `DeviceStatePolicy` is already the persisted home for exactly this kind
    /// of setting ("a setting that reverts on reboot is not a setting") and is
    /// where the table belongs. Recognition is deliberately separate from
    /// binding so that change touches only this function.
    ///
    /// Only the power tap is bound today, and it is bound to what the button
    /// already did — wake if the panel is dark, otherwise lock-then-blank
    /// through `request_blank()` like every other path. Everything else is
    /// recognised, recorded, and inert on purpose: a gesture that fires
    /// something nobody chose is worse than one that fires nothing.
    fn apply_gesture(
        &mut self,
        button: Button,
        gesture: ButtonGesture,
        now: Instant,
    ) -> Vec<Action> {
        let bound =
            button == Button::Power && matches!(gesture, ButtonGesture::Tap | ButtonGesture::Hold);
        self.record_decision(
            "button-gesture",
            serde_json::json!({
                "button": button.as_str(),
                "gesture": gesture.as_str(),
                "bound": bound,
            }),
            "hardware button recognised",
        );
        // Hold raises the power menu — the surface that owns restart, power
        // off, and what the USB-C port currently *is* (data role, power
        // direction, what is attached). That state is proprioception, not a
        // setting, which is why it belongs to this machine and is reached
        // through a verb rather than a config page.
        //
        // Deliberately NOT gated on the panel or the lock. A hold on a dark
        // panel is still a request for the menu, and on the lockscreen it must
        // still work — powering off is a thing you do to a locked phone, and a
        // menu that vanishes when locked would send the user to the hardware
        // button they are already holding. Disclosure is unaffected: the menu
        // shows device verbs, never session content.
        if button == Button::Power && gesture == ButtonGesture::Hold {
            self.press_began_dark.remove(&button);
            return vec![Action::PowerMenu];
        }
        if button != Button::Power || gesture != ButtonGesture::Tap {
            return Vec::new();
        }
        // The panel as it was when THIS button's press began, not as it is
        // now. See `press_began_dark`: this same press already woke it.
        if self
            .press_began_dark
            .remove(&button)
            .unwrap_or(!self.panel_on)
        {
            // Dark: the tap is a wake, and a power-button wake is never vetoed.
            //
            // `Unblank` *then* `Restore`, and the order is the whole of it: the
            // panel has to come back before the brightness it comes back at
            // means anything. `Restore` alone was what this returned, which was
            // only ever enough because the compositor woke the panel through a
            // keybinding before the daemon was ever consulted — see
            // `Action::Unblank`. Under viewtop nothing else does it, so a tap
            // set the brightness of a screen that stayed off.
            //
            // `Restore` puts back the brightness the dim captured rather than a
            // guessed floor — see the comment on that capture.
            self.blank_requested = false;
            // Recorded, because the blank branch below records and this one did
            // not: the trail showed `button-gesture … recognised` and then
            // silence, so "the press woke it" and "the press did nothing" were
            // the same entry. A trail that cannot tell those apart is the one
            // question anyone debugging a dark phone is actually asking.
            self.record_decision(
                "panel-on",
                serde_json::json!({ "button": "power", "gesture": "tap" }),
                "a tap on a dark panel is a wake",
            );
            return vec![Action::Unblank, Action::Restore];
        }
        self.request_blank(
            now,
            serde_json::json!({ "button": "power", "gesture": "tap" }),
            "power button tap",
        )
    }

    /// Bind a recognised touch gesture to behaviour.
    ///
    /// The same shape as [`Self::apply_gesture`] for buttons, and deliberately
    /// as small: three fingers raise the overview, everything else is
    /// recognised, recorded and inert. A gesture that fires something nobody
    /// chose is worse than one that fires nothing, and the binding table this
    /// is a placeholder for is `DeviceStatePolicy`'s to hold — persisted and
    /// agent-writable, so a triple-tap can be re-pointed without a rebuild.
    pub fn touch_gesture(
        &mut self,
        fingers: u8,
        gesture: TouchGesture,
        target: Option<u64>,
    ) -> Vec<Action> {
        // A three-finger tap raises the window action sheet for the window it
        // landed on. It used to raise the overview, and that was wrong twice
        // over: the rail's swipe already reaches the same surface — a gesture
        // that duplicates another gesture is a defect by construction — and
        // the tap arrived with no subject at all, because the compositor
        // dropped the centroid, so the overview was the only thing it *could*
        // name. Now that the target travels with the gesture, the tap can be
        // about the window under the fingers, which is what it was always for.
        let bound = fingers == 3 && matches!(gesture, TouchGesture::Tap) && target.is_some();
        self.record_decision(
            "touch-gesture",
            serde_json::json!({
                "fingers": fingers,
                "gesture": gesture.as_str(),
                "target": target,
                "bound": bound,
            }),
            "touch gesture recognised",
        );
        if !bound {
            return Vec::new();
        }
        // Not while the screen is dark: the sheet is content, and putting
        // content on an unauthenticated glass is what §4's disclosure rules
        // exist to prevent. A tap on a dark panel is a wake, and that is the
        // power button's business, not this one's.
        if !self.panel_on {
            return Vec::new();
        }
        // `target` is Some — `bound` required it. A tap on the wallpaper falls
        // out above with `bound: false` in the trail rather than raising an
        // empty sheet, which is the "why did a blank sheet appear" bug the
        // no-window case exists to avoid.
        vec![Action::WindowSheet {
            target: target.unwrap_or_default(),
        }]
    }

    pub fn tick_at(&mut self, now: Instant) -> Vec<Action> {
        // Buttons first: a hold that crossed its threshold during this tick
        // should be announced before the idle rules below decide anything, or
        // a hold and a blank can land in the same tick in the wrong order.
        let mut button_actions = Vec::new();
        let pending: Vec<(Button, ButtonGesture)> = self
            .buttons
            .iter_mut()
            .filter_map(|(b, r)| r.tick(now).map(|g| (*b, g)))
            .collect();
        for (button, gesture) in pending {
            button_actions.extend(self.apply_gesture(button, gesture, now));
        }
        let mut actions = self.tick_rules_at(now);
        button_actions.append(&mut actions);
        return button_actions;
    }

    fn tick_rules_at(&mut self, now: Instant) -> Vec<Action> {
        // Expiry first. A source that has gone stale must not be allowed to
        // win a pending debounce and then be cleared in the same tick — that
        // would put a transition in the trail for a reading nobody confirmed.
        self.expire_stale_evidence(now);
        self.resolve_proximity_debounce(now);
        self.evaluate_source_health(now);

        let mut actions = self.bearer_actions(now);

        // A blank is already decided and waiting on the lock it asked for.
        // Nothing else may run while that is outstanding — the whole point is
        // that the panel does not go dark ahead of the lock.
        if let Some(deadline) = self.pending_blank {
            if is_locked(self.state) {
                self.pending_blank = None;
                self.blank_requested = true;
                info!("[device-state] lock acked — blanking");
                self.record_decision(
                    "panel-off",
                    serde_json::json!({ "waited_for": "lock-ack" }),
                    "the lock this blank asked for was acknowledged",
                );
                actions.push(Action::Blank);
            } else if now >= deadline {
                // Fail open on the panel, never on the claim. §1: a lit,
                // unlocked phone in a pocket is the worse outcome, so the
                // blank proceeds — but doctrine §8 forbids pretending the
                // session locked, so this is an error in the trail, loudly,
                // and the machine's state is left untouched.
                self.pending_blank = None;
                self.blank_requested = true;
                warn!(
                    "[device-state] lock NOT acked within {}s — blanking an UNLOCKED session",
                    self.policy.lock_ack_budget.as_secs()
                );
                self.record_error(
                    "device-state",
                    "blank-without-lock",
                    "lock ack did not arrive within the budget; panel blanked unlocked",
                );
                actions.push(Action::Blank);
            }
            return actions;
        }

        if !self.panel_on || self.blank_requested {
            return actions;
        }

        // Proximity does not blank the panel. It used to, on any locked
        // screen, which is `blueline-proximity-lock`'s old job moved inward
        // and kept too powerful: a covered sensor is a pocket, a face, a
        // table, or a thumb, and the machine cannot tell which. Turning the
        // screen off on that reading is only right during a call — and the
        // machine has no call state yet, so for now it is never right.
        //
        // What remains of proximity is evidence: it drives `Observed`, it is
        // in every snapshot, and it gates tap-to-wake (`suppress_wake`). The
        // idle budget below blanks a locked screen soon enough anyway.

        // The compositor says the user is interacting. Not our business yet —
        // and crucially this is what keeps a long swipe from being blanked
        // out from under the user's thumb.
        let Some(idle_since) = self.idle_since else {
            return actions;
        };

        let held = self.sensor_evidence.accel_moving;
        let budget = if !is_locked(self.state) {
            // An unlocked screen is not this rule's business by default; the
            // shell's IdleCoordinator owns that timer. When it IS set, the
            // blank still routes through lock-then-off below.
            self.policy.unlocked_blank_after
        } else if held {
            self.policy.lock_blank_after_held
        } else {
            self.policy.lock_blank_after
        };
        // "Never" is a legitimate setting (plugged in at the desk).
        let Some(budget) = budget else {
            return actions;
        };

        let idle_for = now.saturating_duration_since(idle_since);

        // Stage two: the grace ran out, go dark.
        if idle_for >= budget {
            info!(
                "[device-state] {}, idle {:.1}s of {}s — blanking",
                if is_locked(self.state) {
                    "locked"
                } else {
                    "unlocked"
                },
                idle_for.as_secs_f32(),
                budget.as_secs()
            );
            return self.request_blank(
                now,
                serde_json::json!({
                    "idle_secs": idle_for.as_secs(),
                    "budget_secs": budget.as_secs(),
                    "held": held,
                    "was_dimmed": self.dimmed,
                    "confidence": self.sensor_evidence.confidence(),
                }),
                "panel lit, no input within the blank budget",
            );
        }

        // Stage one: the pre-warning. The panel visibly fades and a tap
        // inside the grace window cancels the blank — the user gets told
        // what is about to happen instead of the screen simply dying.
        if self.policy.dim_warning && !self.dimmed {
            let dim_at = budget.saturating_sub(self.policy.dim_grace);
            if idle_for >= dim_at {
                self.dimmed = true;
                info!(
                    "[device-state] locked, idle {:.1}s — dimming, {}s to tap",
                    idle_for.as_secs_f32(),
                    self.policy.dim_grace.as_secs()
                );
                self.record_decision(
                    "panel-dim",
                    serde_json::json!({
                        "idle_secs": idle_for.as_secs(),
                        "blank_at_secs": budget.as_secs(),
                        "grace_secs": self.policy.dim_grace.as_secs(),
                        "held": held,
                    }),
                    "pre-warning before blanking; input inside the grace cancels it",
                );
                actions.push(Action::Dim);
            }
        }

        actions
    }

    /// Every path to a dark panel goes through here.
    ///
    /// `LOCK-DPMS-LESSONS.md` §1 is "Ordering: lock, then off", and until now
    /// that held only because hypridle's 300 s lock listener happened to fire
    /// before its own 600 s blank listener. That is an assumption, not an
    /// invariant: anything that skipped the lock (an idle inhibitor, the
    /// native coordinator disabled, a dead shell) still got blanked, unlocked
    /// and silently. Routing every blank through one function makes the
    /// ordering a property of the authority instead of a coincidence of two
    /// timers owned by a config file.
    fn request_blank(
        &mut self,
        now: Instant,
        inputs: serde_json::Value,
        reason: &str,
    ) -> Vec<Action> {
        if is_locked(self.state) {
            self.blank_requested = true;
            self.record_decision("panel-off", inputs, reason);
            return vec![Action::Blank];
        }

        info!(
            "[device-state] blanking an unlocked session — locking first, {}s to ack",
            self.policy.lock_ack_budget.as_secs()
        );
        self.pending_blank = Some(now + self.policy.lock_ack_budget);
        self.record_decision(
            "lock-before-blank",
            inputs,
            "a blank was decided for an unlocked session; the lock goes first",
        );
        vec![Action::Lock]
    }

    /// Drop evidence nobody has refreshed. Stale readings are cleared rather
    /// than trusted, and clearing errs the safe way: a stale proximity no
    /// longer suppresses a wake, and a stale accelerometer no longer buys the
    /// longer blank budget. Never the reverse.
    fn expire_stale_evidence(&mut self, now: Instant) {
        let ttl = self.policy.evidence_ttl;
        let expired = |seen: &mut Option<Instant>, flag: &mut bool| -> bool {
            let stale = match *seen {
                Some(t) => now.saturating_duration_since(t) > ttl,
                None => true,
            };
            if stale && *flag {
                *flag = false;
                *seen = None;
                return true;
            }
            false
        };

        let mut dropped: Vec<&str> = Vec::new();
        if expired(
            &mut self.evidence_seen.proximity,
            &mut self.sensor_evidence.proximity_near,
        ) {
            // Clear the raw reading and any pending debounce with it. Stale
            // means unknown, and a half-resolved edge left behind an expiry
            // would let a reading nobody has confirmed in 30 s win the moment
            // the next tick ran.
            self.proximity_raw = false;
            self.proximity_since = None;
            dropped.push("proximity");
        }
        if expired(
            &mut self.evidence_seen.accel,
            &mut self.sensor_evidence.accel_moving,
        ) {
            dropped.push("accel");
        }
        if expired(
            &mut self.evidence_seen.light,
            &mut self.sensor_evidence.light_changing,
        ) {
            // Stale lux is unknown, same as stale presence: a number from a
            // source nobody has heard in 30 s must not feed auto-brightness.
            self.sensor_evidence.lux = None;
            dropped.push("light");
        }
        if expired(
            &mut self.evidence_seen.touch,
            &mut self.sensor_evidence.touch_active,
        ) {
            dropped.push("touch");
        }

        if !dropped.is_empty() {
            warn!(
                "[device-state] evidence went stale, treating as unknown: {}",
                dropped.join(", ")
            );
            self.record_decision(
                "evidence-stale",
                serde_json::json!({ "sources": dropped, "ttl_secs": ttl.as_secs() }),
                "no reading within the evidence TTL — a dead sensor is not a quiet one",
            );
        }
    }

    /// Decide whether each evidence source is still there.
    ///
    /// Separate from `expire_stale_evidence` because it answers a different
    /// question, and because the staleness rule structurally cannot answer this
    /// one: it only acts on a source whose flag is currently `true`. A
    /// proximity sensor resting at `far` has a `false` flag, so it expires
    /// nothing, reports nothing, and dies in complete silence. That is exactly
    /// the 2026-07-25 outage — hours of dead sensors with nothing in any log.
    fn evaluate_source_health(&mut self, now: Instant) {
        let limit = self.policy.source_down_after;
        let sources = [
            (SensorSource::Proximity, self.evidence_seen.proximity),
            (SensorSource::Accelerometer, self.evidence_seen.accel),
            (SensorSource::Light, self.evidence_seen.light),
            (SensorSource::Touch, self.evidence_seen.touch),
            (SensorSource::Charge, self.evidence_seen.charge),
        ];

        let mut newly_down: Vec<&str> = Vec::new();
        for (source, seen) in sources {
            let Some(t) = seen else { continue };
            if now.saturating_duration_since(t) <= limit {
                continue;
            }
            let health = self.source_health.get_mut(source);
            if *health == SourceHealth::Down {
                continue;
            }
            *health = SourceHealth::Down;
            newly_down.push(source.as_str());
        }

        // Expected sources that have never spoken at all. `Down` above starts
        // from a last-seen stamp and so structurally cannot see these: no
        // stamp, no entry, silence forever. That is how a reporter which never
        // started produced a whole session of evidence-free decisions with
        // nothing in the trail (2026-07-27).
        let mut newly_absent: Vec<&str> = Vec::new();
        if now.saturating_duration_since(self.started_at) > self.policy.source_expected_within {
            for &source in EXPECTED_SOURCES {
                if self.evidence_seen.get(source).is_some() {
                    continue;
                }
                let health = self.source_health.get_mut(source);
                if *health == SourceHealth::Absent {
                    continue;
                }
                *health = SourceHealth::Absent;
                newly_absent.push(source.as_str());
            }
        }

        // One entry per edge, not per tick — same contract as source-down.
        for source in &newly_absent {
            warn!(
                "[device-state] evidence source {source} has never reported in {}s since start — its reporter is not running",
                self.policy.source_expected_within.as_secs()
            );
            self.record_error(
                "sensor-health",
                "source-never-reported",
                &format!(
                    "{source} is expected on this device but has never reported in the {}s since sessiond started; its reporter is not running, so every rule reading it is deciding on absence, not on a negative",
                    self.policy.source_expected_within.as_secs()
                ),
            );
        }

        // One entry per outage edge, not per tick. A source stays Down until
        // it speaks again, and a trail that repeated this every second would
        // bury the transition that actually diagnoses anything.
        for source in &newly_down {
            warn!(
                "[device-state] evidence source {source} has said nothing for {}s — treating it as DOWN, not quiet",
                limit.as_secs()
            );
            self.record_error(
                "sensor-health",
                "source-down",
                &format!(
                    "{source} reported before but has been silent for over {}s; its readings are absent, not negative",
                    limit.as_secs()
                ),
            );
        }
    }

    /// Stamp a source as freshly reported. Called alongside the reading.
    /// The bearer actually in force, for the readout. `None` until the first
    /// preference settles — which is a real state and not the same as "wifi".
    pub fn bearer_applied_str(&self) -> Option<&'static str> {
        self.bearer_applied.map(|b| b.as_str())
    }

    /// Fresh bearer evidence from the daemon's probe.
    ///
    /// Ingress only. The machine never reads the network itself for the same
    /// reason it never reads logind itself: a reader that can also act is one
    /// refactor away from being a controller, and the controller this replaces
    /// is the one that recycled the tunnel 652 times.
    pub fn note_bearer(&mut self, evidence: BearerEvidence) {
        self.bearer = evidence;
    }

    /// Charge evidence in, conclusion edge out. The trail hears about a
    /// change once, not once per report — same rule as the bearer's.
    ///
    /// Ingress only, and only through `sensor_input`: the reporter reads the
    /// supplies, the machine interprets. The reading arrives with the same
    /// freshness stamp and health accounting as any other source.
    pub fn note_charge(&mut self, evidence: ChargeFields) {
        self.charge = evidence;
        let conclusion = Self::conclude_charge(&self.charge);
        if self.charge_conclusion != Some(conclusion) {
            let previous = self.charge_conclusion;
            self.charge_conclusion = Some(conclusion);
            let mut inputs = self.charge_json();
            inputs["previous_conclusion"] = previous.map_or(serde_json::Value::Null, |p| p.into());
            self.record_decision(
                "charge-conclusion",
                inputs,
                "what the machine concludes about charging changed — one decision, made once (TASK-33)",
            );
        }
    }

    /// The one decision surfaces render instead of guessing.
    ///
    /// Pure function of the evidence — no clock, no I/O — so the mapping is
    /// testable and the same on every report. Interpretation may consume
    /// evidence but never a driver, so it lives on the machine, not in the
    /// reporter's type. "Resting" is the state nothing else could name:
    /// plugged in, not charging, not full. It is the normal hysteresis rest
    /// of a topped-up pack and is never a fault (TASK-33's do-not).
    pub fn conclude_charge(evidence: &ChargeFields) -> &'static str {
        let status = evidence.status.as_deref();
        match (evidence.plugged, status) {
            (_, Some("Full")) => "charged",
            (Some(true), Some("Charging")) => match evidence.charge_type.as_deref() {
                Some("Fast") => "charging_fast",
                Some("Trickle" | "Slow") => "charging_slow",
                _ => "charging",
            },
            (Some(true), Some("Not charging")) => "resting",
            (Some(true), Some("Discharging")) => "resting",
            (Some(false), _) => "on_battery",
            (_, Some("Discharging")) => "on_battery",
            _ => "unknown",
        }
    }

    fn charge_json(&self) -> serde_json::Value {
        serde_json::json!({
            "plugged": self.charge.plugged,
            "status": self.charge.status,
            "charge_type": self.charge.charge_type,
            "capacity": self.charge.capacity,
            "conclusion": Self::conclude_charge(&self.charge),
        })
    }

    /// Decide the bearer posture, subject to the settling window.
    ///
    /// Returns at most one `PreferLink` and one `PinTunnelUnderlay`, and only
    /// on a change that has held. A steady state produces no actions at all,
    /// which is what makes this safe to call every tick.
    fn bearer_actions(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();

        // Report the tunnel going deaf on the edge. This is the one thing that
        // is worth saying even when nothing can be done about it: every other
        // readout on the device calls a deaf tunnel "connected".
        let deaf = self.bearer.tunnel_is_deaf();
        if deaf != self.bearer_deaf {
            self.bearer_deaf = deaf;
            if deaf {
                warn!("[device-state] tunnel is up but receiving nothing");
            }
            self.record_decision(
                "bearer-tunnel-health",
                serde_json::json!({ "tunnel": self.bearer.tunnel.as_str() }),
                if deaf {
                    "tunnel up but no bytes received — up is not carrying"
                } else {
                    "tunnel handshake healthy"
                },
            );
        }

        let Some(want) = self.bearer.preferred() else {
            // Nothing usable. Deliberately not an action: with no link worth
            // preferring there is nothing to prefer it over, and issuing a
            // command here would be the machine flailing at a dead network.
            self.bearer_candidate_since = None;
            return actions;
        };

        if Some(want) == self.bearer_applied {
            self.bearer_candidate_since = None;
            return actions;
        }

        // A changed candidate has to survive the settling window. Restarting
        // the clock whenever the candidate itself changes is what stops a link
        // oscillating between two answers from ever accumulating enough time.
        let since = match self.bearer_candidate_since {
            Some((b, t)) if b == want => t,
            _ => {
                self.bearer_candidate_since = Some((want, now));
                now
            }
        };
        if now.duration_since(since) < self.policy.bearer_settle {
            return actions;
        }

        let previous = self.bearer_applied;
        self.bearer_applied = Some(want);
        self.bearer_candidate_since = None;

        info!(
            "[device-state] bearer: {} -> {}",
            previous.map(|b| b.as_str()).unwrap_or("none"),
            want.as_str()
        );
        self.record_decision(
            "bearer-preferred",
            serde_json::json!({
                "from": previous.map(|b| b.as_str()),
                "to": want.as_str(),
                "wifi": self.bearer.wifi.as_str(),
                "cellular": self.bearer.cellular.as_str(),
                "tunnel": self.bearer.tunnel.as_str(),
                "home": self.bearer.home,
                "ssid": self.bearer.ssid,
                "settled_for_secs": now.duration_since(since).as_secs(),
            }),
            "bearer preference changed and held for the settling window",
        );
        actions.push(Action::PreferLink(want));

        // The tunnel follows the underlay, and only when it is already up.
        if let Some(underlay) = self.bearer.tunnel_underlay() {
            actions.push(Action::PinTunnelUnderlay(underlay));
        }
        actions
    }

    pub fn mark_evidence_seen(&mut self, source: SensorSource) {
        self.mark_evidence_seen_at(source, Instant::now())
    }

    /// `mark_evidence_seen` with the clock passed in, so health transitions can
    /// be tested on a timeline instead of by sleeping for 90 seconds.
    pub fn mark_evidence_seen_at(&mut self, source: SensorSource, now: Instant) {
        match source {
            SensorSource::Proximity => self.evidence_seen.proximity = Some(now),
            SensorSource::Accelerometer => self.evidence_seen.accel = Some(now),
            SensorSource::Light => self.evidence_seen.light = Some(now),
            SensorSource::Touch => self.evidence_seen.touch = Some(now),
            SensorSource::Charge => self.evidence_seen.charge = Some(now),
        }

        let health = self.source_health.get_mut(source);
        let was = *health;
        *health = SourceHealth::Live;
        // Absent recovers by the same path as Down. Both mean "the machine was
        // deciding without this source", and both need the closing entry that
        // bounds the window — an outage with no end tells you when evidence
        // died and never when it came back.
        if matches!(was, SourceHealth::Down | SourceHealth::Absent) {
            // Recovery is as diagnostic as the outage: the pair of entries
            // bounds the window in which every sensor-driven rule was running
            // on nothing, which is what makes the trail usable after the fact.
            info!(
                "[device-state] evidence source {} is reporting again (was {})",
                source.as_str(),
                was.as_str()
            );
            self.record_decision(
                "source-recovered",
                serde_json::json!({ "source": source.as_str(), "was": was.as_str() }),
                "a source the machine had no evidence from has resumed reporting",
            );
        }
    }

    /// Real user input, with the machine's own veto applied first.
    ///
    /// `note_input` is unconditional by design: a power button is intent and is
    /// never refused (§4, "no — hardware signal"). A squeeze is not a button.
    /// It is a strain reading, and a phone in a tight pocket is a squeezed
    /// chassis — the same failure a covered proximity sensor already vetoes for
    /// tap-to-wake. Routing squeeze through here rather than straight to
    /// `note_input` is what stops a pocket from resetting the idle budget and
    /// opening a verb surface.
    ///
    /// The refusal is recorded, not dropped. §10 spent a section establishing
    /// that absence must never be mistaken for a negative; a producer whose
    /// reports vanish silently is indistinguishable from a dead one.
    pub fn note_input_gated(&mut self, trigger: InputTrigger) -> Option<Vec<Action>> {
        self.note_input_gated_at(trigger, Instant::now())
    }

    /// `note_input_gated` with the clock passed in, so the burst window is
    /// testable without sleeping — the same split `tick`/`tick_at` has.
    pub fn note_input_gated_at(
        &mut self,
        trigger: InputTrigger,
        now: Instant,
    ) -> Option<Vec<Action>> {
        if self.suppress_wake(trigger) {
            if matches!(trigger, InputTrigger::DoubleTapToWake) {
                while self
                    .double_tap_refusals
                    .front()
                    .is_some_and(|t| now.saturating_duration_since(*t) > DOUBLE_TAP_BURST_WINDOW)
                {
                    self.double_tap_refusals.pop_front();
                }
                self.double_tap_refusals.push_back(now);
                if self.double_tap_refusals.len() >= DOUBLE_TAP_BURST_COUNT {
                    self.double_tap_refusals.clear();
                    self.record_decision(
                        "input-burst-allowed",
                        serde_json::json!({
                            "trigger": format!("{:?}", trigger),
                            "placement": self.placement(),
                            "count": DOUBLE_TAP_BURST_COUNT,
                            "window_secs": DOUBLE_TAP_BURST_WINDOW.as_secs(),
                        }),
                        "an accelerating run of deliberate double-taps overrides the pocket belief — fail-open, and the trail says so (DUMP 2026-08-14 §2)",
                    );
                    return Some(self.note_input(trigger));
                }
            }
            let placement = self.placement();
            warn!(
                "[device-state] {:?} refused — believed {:?} at {:.2}",
                trigger, placement.belief, placement.confidence
            );
            self.record_decision(
                "input-refused",
                serde_json::json!({
                    "trigger": format!("{:?}", trigger),
                    "placement": placement,
                    "confidence": self.sensor_evidence.confidence(),
                }),
                "a placement belief above the veto threshold (DEVICE-STATE-MACHINE §4)",
            );
            return None;
        }
        Some(self.note_input(trigger))
    }

    /// Real user input. Resets the blank budget, re-arms the rule, and undoes
    /// the pre-warning dim if one is showing — that cancel is the whole point
    /// of the grace window, so it returns the action rather than waiting for
    /// the next tick to notice.
    pub fn note_input(&mut self, trigger: InputTrigger) -> Vec<Action> {
        self.idle_since = None;
        self.blank_requested = false;
        // Input inside the lock-ack window cancels the blank outright. The
        // lock request already went out and is not recalled — a lock the user
        // interrupted is a lock, and only PAM leaves it.
        if self.pending_blank.take().is_some() {
            info!("[device-state] input during the lock-ack window — blank cancelled");
        }
        let mut actions = Vec::new();
        if self.dimmed {
            self.dimmed = false;
            info!("[device-state] input during the dim warning — restoring");
            actions.push(Action::Restore);
        }
        let wake = match trigger {
            InputTrigger::PowerButton => WakeTrigger::PowerButton,
            InputTrigger::DoubleTapToWake => WakeTrigger::DoubleTapToWake,
            InputTrigger::Squeeze => WakeTrigger::Squeeze,
            InputTrigger::Touch | InputTrigger::Key => WakeTrigger::UserInput,
            InputTrigger::Unknown => WakeTrigger::Unknown,
        };
        self.record_wake(wake, &format!("input: {:?}", trigger));

        // A deliberate wake on a dark panel has to actually light it.
        //
        // This function recorded the wake and returned `Restore` — which is
        // *brightness* — so double-tap-to-wake reported to the machine and the
        // screen stayed off. It was invisible for as long as the wake never
        // came through the daemon: `hyprland.lua` bound `XF86WakeUp` straight
        // to `blueline-screen-toggle on`, so the compositor lit the panel and
        // sessiond only heard about it afterwards. viewtop consumes that key,
        // and dt2w stopped working.
        //
        // Only the *deliberate* wakes, and only when the panel is actually
        // dark. `Touch`/`Key`/`Unknown` are excluded because they are not wake
        // gestures — the touch controller is in gesture mode while the panel is
        // off, so a raw touch arriving there is not a request to wake.
        //
        // `PowerButton` is excluded for a different reason: it already has a
        // wake, from `apply_gesture` resolving the tap. Emitting one here too
        // would put two unblanks in flight for one press, and the second would
        // land against a panel the first had already lit — which is how the
        // wake loop happened in the compositor earlier today.
        //
        // Proximity has already had its say: this is reached through
        // `note_input_gated`, whose veto is the pocket check. A power button is
        // never vetoed and a double tap always is, because a pocket can produce
        // the second and not the first (§4).
        if !self.panel_on
            && matches!(
                trigger,
                InputTrigger::DoubleTapToWake | InputTrigger::Squeeze
            )
        {
            info!("[device-state] {:?} on a dark panel — waking", trigger);
            self.blank_requested = false;
            // Before any `Restore`: the brightness a panel comes back at means
            // nothing until the panel is back.
            actions.insert(0, Action::Unblank);
        }
        actions
    }

    /// What the machine currently believes the panel is doing.
    ///
    /// A belief, and named as one: the executor reports it (`set_panel`) and
    /// nothing here infers it, so this is the last thing the hardware said
    /// rather than an assumption about what it must be doing now.
    pub fn panel_on(&self) -> bool {
        self.panel_on
    }

    /// Ask for the panel, from outside the machine's own rules.
    ///
    /// The agent's verb lands here (`{"op":"screen"}`), and so does anything
    /// else that wants the glass lit or dark. It is deliberately not a
    /// shortcut past anything: **off routes through `request_blank()`** like
    /// every other path, so it locks first and waits out `LOCK_ACK_BUDGET`,
    /// and **on** is the wake that direction never had.
    ///
    /// Doctrine §13 in one function. Operation is hers — she may turn the
    /// screen off — and the ordering invariant is not a permission she is
    /// missing, it is a property of the machine that applies to every caller
    /// including itself.
    pub fn request_screen(&mut self, on: bool, why: &str) -> Vec<Action> {
        if on {
            if self.panel_on {
                return Vec::new();
            }
            self.blank_requested = false;
            let mut actions = vec![Action::Unblank];
            // A wake must never come up dim, for the same reason `set_panel`
            // restores: a blank that landed while the dim warning was showing
            // left the saved brightness low.
            if self.dimmed {
                self.dimmed = false;
                actions.push(Action::Restore);
            }
            return actions;
        }
        if !self.panel_on {
            return Vec::new();
        }
        self.request_blank(
            Instant::now(),
            serde_json::json!({ "verb": "screen", "on": false }),
            why,
        )
    }

    /// Ask for a USB gadget posture.
    ///
    /// The decision is intentionally tiny today: all advertised device-role
    /// modes are admissible. The seam matters because attachment identity and
    /// probe ownership will make that conditional; when they land, the refusal
    /// belongs here rather than in QML or usb-signaller. Every request is
    /// written now, so that future rule has a trail to inherit.
    pub fn request_usb_mode(&mut self, mode: UsbMode, why: &str) -> Vec<Action> {
        self.record_decision(
            "usb-mode-request",
            serde_json::json!({ "mode": mode.as_str() }),
            why,
        );
        vec![Action::UsbMode(mode)]
    }

    /// Ask to power the machine off, restart it, or put it to sleep.
    ///
    /// Not gated on the lock. Powering off a locked phone is a thing people do
    /// deliberately, and a refusal here would send them to the hardware button
    /// they are already holding — the same argument `PowerMenu.qml` makes for
    /// showing the sheet while locked.
    ///
    /// No state transition either. `Suspending` is entered from logind's
    /// `PrepareForSleep`, which is the event that actually means it; setting it
    /// here would be a second writer to the state the protocol owns, and a
    /// suspend that logind then refuses would leave the machine believing it
    /// had gone to sleep (doctrine §4).
    pub fn request_power(&mut self, verb: PowerVerb, why: &str) -> Result<Vec<Action>, String> {
        if let Some(in_flight) = self.power_requested {
            let refusal = format!("{} is already in flight", in_flight.as_str());
            self.record_decision(
                "power-request-refused",
                serde_json::json!({ "verb": verb.as_str(), "in_flight": in_flight.as_str() }),
                &refusal,
            );
            return Err(refusal);
        }
        self.power_requested = Some(verb);
        self.record_decision(
            "power-request",
            serde_json::json!({ "verb": verb.as_str() }),
            why,
        );
        Ok(vec![Action::Power(verb)])
    }

    /// Release the in-flight latch after the mechanism returned.
    ///
    /// This is deliberately symmetric across success and failure. `systemctl
    /// suspend` and `hibernate` return to the same daemon after resume, while a
    /// poweroff/reboot command can return zero before shutdown finishes or is
    /// later cancelled. Leaving the latch set on any return would turn an
    /// accepted request into a permanent refusal for the rest of the uptime.
    /// A stale completion cannot clear a newer request for a different verb.
    pub fn power_request_finished(&mut self, verb: PowerVerb) -> bool {
        if self.power_requested == Some(verb) {
            self.power_requested = None;
            return true;
        }
        false
    }

    /// The executor reports the panel's real state. Turning on counts as
    /// input, so a dt2w wake starts the blank budget from the wake itself.
    ///
    /// A *report*, never a request — [`Self::request_screen`] is the asking
    /// half. Keeping them apart is what stops a stale report from driving the
    /// panel, and stops an ask from quietly editing the machine's idea of what
    /// the hardware is doing.
    pub fn set_panel(&mut self, on: bool) -> Vec<Action> {
        if self.panel_on == on {
            return Vec::new();
        }
        self.panel_on = on;
        info!("[device-state] panel {}", if on { "on" } else { "off" });
        self.blank_requested = false;
        // A blank waiting on a lock ack is moot once the panel is dark by any
        // route — the power button, the proximity service, a wake that never
        // came. Leaving it armed makes the next tick emit a second `Blank` at
        // an already-dark panel, or worse, fire the `blank-without-lock` error
        // path for a blank nobody is waiting on.
        if !on {
            self.pending_blank = None;
        }
        if on {
            self.idle_since = None;
            // A wake must never come up dim. If the blank landed while the
            // warning was showing, the saved brightness is still low, so the
            // restore has to run before the user sees the panel.
            if self.dimmed {
                self.dimmed = false;
                return vec![Action::Restore];
            }
        }
        Vec::new()
    }

    /// The compositor reported a quiet period beginning at `since`.
    pub fn note_idle_start(&mut self, since: Instant) {
        if self.idle_since.is_none() {
            self.idle_since = Some(since);
        }
    }

    /// Seconds of compositor-reported quiet, or 0 while the user is active.
    pub fn idle_secs(&self) -> u64 {
        match self.idle_since {
            Some(t) => Instant::now().saturating_duration_since(t).as_secs(),
            None => 0,
        }
    }

    /// Whether the compositor currently reports the user as interacting.
    pub fn user_active(&self) -> bool {
        self.idle_since.is_none()
    }

    /// Build a forensic snapshot of the current state.
    pub fn snapshot(
        &self,
        phase: &str,
        shell_alive: bool,
        screen_locked: bool,
        screen_lock_secure: bool,
        idle_state: &str,
        inhibitor_held: bool,
    ) -> StateSnapshot {
        StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: phase.to_string(),
            shell_alive,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked,
            screen_lock_secure,
            idle_coordinator_state: idle_state.to_string(),
            sleep_inhibitor_held: inhibitor_held,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        }
    }

    pub fn state(&self) -> DeviceState {
        self.state
    }

    /// Record the brightness the panel was at when the dim fired. The executor
    /// reads the hardware; the machine is what remembers.
    pub fn note_brightness_before_dim(&mut self, value: Option<u32>) {
        self.brightness_before_dim = value;
    }

    /// The captured pre-dim brightness, consumed by the restore. Taken rather
    /// than read: a stale value surviving into the next dim cycle is exactly
    /// the class of bug this replaced.
    pub fn take_brightness_before_dim(&mut self) -> Option<u32> {
        self.brightness_before_dim.take()
    }

    /// Should this wake be refused because something is over the sensor?
    ///
    /// Tap-to-wake only. A double tap is the one wake source a pocket can
    /// produce by itself, so a covered sensor is the right veto for it. A
    /// power button press is intent (§4: "no — hardware signal") and is never
    /// refused; neither is a wake the machine itself asked for.
    ///
    /// This used to be a confidence question — the pair proximity-near +
    /// accel-moving read as 0.5 and suppressed everything. Motion cannot tell
    /// a pocket from an ear from a hand, so it was never the instrument.
    ///
    /// **Owed:** a call. Screen dark at the ear is wanted whether or not the
    /// session is locked, and it is the only case where proximity should turn
    /// a panel *off* rather than decline to turn one on. That needs a
    /// call-state input (ModemManager / callaudiod) — a factor, never an
    /// authority.
    /// May a `far` reading end `Observed` yet? See [`OBSERVED_MIN_DWELL`].
    ///
    /// Anything other than `Observed` answers yes: this gates one edge, and a
    /// state we are not in has no dwell to serve. `None` also answers yes —
    /// an `Observed` entered before this was tracked (a daemon that restarted
    /// into it) must still be able to leave, or the phone would sit in a state
    /// nothing could clear.
    fn observed_dwell_elapsed(&self, now: Instant) -> bool {
        if !matches!(self.state, DeviceState::Observed) {
            return true;
        }
        self.observed_since
            .map(|t| now.saturating_duration_since(t) >= OBSERVED_MIN_DWELL)
            .unwrap_or(true)
    }

    /// Where the phone believes it is. Interpretation, not a reading.
    ///
    /// `proximity_near` was being read as "pocket" and refusing things on that
    /// basis. §4 says it cannot carry that meaning — *"A covered sensor is a
    /// pocket, a face, a table or a thumb, and nothing in the machine can tell
    /// which"* — and measured 2026-08-05 it was drastically wrong: seven
    /// deliberate squeezes refused as pocket-dials while the phone was in a
    /// hand, unlocked, screen lit.
    ///
    /// So the four candidates §4 names get separated by the evidence that
    /// actually distinguishes them, and each answer carries a confidence,
    /// because they are not equally knowable:
    ///
    /// - **Hand** — unlocked with the panel lit. Proximity near is *compatible*
    ///   with this, not evidence against it: holding a phone is what puts a
    ///   palm over the sensor.
    /// - **Table** — lit and unlocked but nothing has touched it in a while.
    ///   Casey, 2026-08-05: *"on table is more accurate as I'm not staring it
    ///   down but if it flashed I'd notice"* — which is the useful part. Table
    ///   is *attention available*, not absence.
    /// - **Pocket** — the only one that needs all of locked, dark and covered,
    ///   and still tops out **low**. Every one of those three is a state the
    ///   phone is often in on a desk in a dark room, and the sensor that would
    ///   settle it is the one §4's table marks as able to lie.
    /// - **Face** — needs a call, and there is no call-state input yet. §4 names
    ///   it as owed; until it exists this is never answered rather than guessed.
    ///
    /// Confidence is deliberately capped below certainty everywhere. This is
    /// interpretation consuming evidence, and it gates one thing (tap-to-wake).
    /// It is on the snapshot so the trail records what was believed, and it is
    /// never an authority.
    pub fn placement(&self) -> Placement {
        let covered = self.sensor_evidence.proximity_near;
        let locked = is_locked(self.state);
        let lit = self.panel_on;
        // `idle_since` is cleared by real input, so `None` is "something touched
        // this recently" without needing a clock passed in.
        let active = self.idle_since.is_none();

        // Covered is tested first: it is the only pocket-specific evidence
        // here, and every other branch describes a phone in the open. Ordering
        // it after "unlocked and lit" meant a covered sensor never reached this
        // at all, because an unlocked lit phone matched Hand first.
        let (belief, confidence) = if covered {
            // Lock and darkness *grade* the reading rather than gate it.
            // Gating on either disarmed the one thing §4 keeps the veto for —
            // the pocket case worth stopping is a double tap about to light the
            // screen up, which happens before the panel is dark and can happen
            // before the lock hint catches up. Capped below Hand and Table
            // throughout, because a face-down phone on a desk produces this
            // exact signature and nothing here separates them.
            let c = match (locked, lit) {
                (true, false) => 0.7,
                (true, true) => 0.6,
                _ => 0.5,
            };
            (PlacementBelief::Pocket, c)
        } else if !locked && lit && active {
            (PlacementBelief::Hand, 0.8)
        } else if !locked && lit {
            // Unlocked and bright with nobody touching it. Casey's case, and
            // the one the old boolean got most wrong.
            (PlacementBelief::Table, 0.7)
        } else if locked {
            (PlacementBelief::Table, 0.5)
        } else {
            (PlacementBelief::Unknown, 0.0)
        };

        Placement {
            belief,
            confidence,
            covered,
            locked,
            lit,
            active,
        }
    }

    /// Tap-to-wake, and nothing else.
    ///
    /// §4 settles the scope in one line — *"It vetoes tap-to-wake, and nothing
    /// else"* — and the argument is that a double tap is the one input **a
    /// pocket can produce by itself**. Squeeze was in this list and does not
    /// meet that test: six strain gauges deflecting past a calibrated baseline
    /// is a hand closing on the phone, and holding it in order to squeeze it is
    /// exactly what covers the proximity sensor.
    ///
    /// If pocket squeezes turn out to be real, the answer is the producer's
    /// threshold — it owns the gauges and their per-device sensitivities — not
    /// a veto from the one sensor that cannot tell a pocket from a thumb.
    pub fn suppress_wake(&self, trigger: InputTrigger) -> bool {
        let p = self.placement();
        matches!(trigger, InputTrigger::DoubleTapToWake)
            && p.belief == PlacementBelief::Pocket
            && p.confidence >= POCKET_VETO_CONFIDENCE
    }

    /// Attempt a state transition. Returns true if the transition was
    /// legal and applied, false if refused (logged as a warning).
    /// Emits a forensic entry for every attempt (legal or not).
    pub fn transition(&mut self, next: DeviceState) -> bool {
        if self.state == next {
            return true; // idempotent
        }
        let legal = LEGAL_TRANSITIONS
            .iter()
            .any(|(from, to)| *from == self.state && *to == next);
        let prev = self.state;
        if !legal {
            warn!(
                "[device-state] ILLEGAL transition {:?} → {:?} (refused)",
                self.state, next
            );
            // Forensic: record the refused transition with a minimal snapshot
            let snapshot = StateSnapshot {
                device_state: self.state,
                locked: is_locked(self.state),
                display_active: is_display_active(self.state),
                phase: String::new(),
                shell_alive: false,
                proximity_near: self.sensor_evidence.proximity_near,
                confidence: self.sensor_evidence.confidence(),
                suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
                promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
                screen_locked: false,
                screen_lock_secure: false,
                idle_coordinator_state: String::new(),
                sleep_inhibitor_held: false,
                sensors_degraded: self.source_health.any_down(),
                placement: self.placement(),
            };
            self.forensic.append(
                ForensicEvent::Transition {
                    from: prev,
                    to: next,
                    legal: false,
                },
                snapshot,
                &format!("illegal transition {:?} → {:?} refused", prev, next),
            );
            return false;
        }
        self.state = next;
        // Cleared here, on any path out, so a later entry cannot inherit an old
        // timestamp and believe its dwell is already served. The *stamp* is set
        // by the caller instead — `Instant::now()` here would read the real
        // clock while every test drives an injected one, and the dwell would
        // then be untestable.
        if !matches!(next, DeviceState::Observed) {
            self.observed_since = None;
        }
        // Log the pair the right way round. This read
        // "Locked → Locked (from Active)" on the first live run, which looks
        // like a refused self-transition rather than the real Active → Locked.
        info!("[device-state] {:?} → {:?}", prev, next);
        // Forensic: record the successful transition
        let snapshot = StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: String::new(),
            shell_alive: false,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked: false,
            screen_lock_secure: false,
            idle_coordinator_state: String::new(),
            sleep_inhibitor_held: false,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        };
        self.forensic.append(
            ForensicEvent::Transition {
                from: prev,
                to: next,
                legal: true,
            },
            snapshot,
            &format!("{:?} → {:?}", prev, next),
        );
        true
    }

    /// Adopt logind's `LockedHint` as the lock truth.
    ///
    /// This replaces the machine deciding for itself. It had no unlock ingress
    /// outside its own fallback surface, so after the first unlock it believed
    /// the session was locked forever, and every rule gated on
    /// `is_locked(state)` — the blank budget AND the proximity blank — fired
    /// against an unlocked, in-use phone. Doctrine §4: never hold state the
    /// protocol owns.
    ///
    /// Locking moves the base tier to `Locked`; unlocking returns to `Active`
    /// from whichever locked sub-state we were in. Sub-state detail
    /// (`Observed`, doze tiers) is ours to track *within* locked, but whether
    /// we are locked at all is not.
    pub fn set_session_locked(&mut self, locked: bool) -> Vec<Action> {
        if locked == is_locked(self.state) {
            return Vec::new();
        }

        if locked {
            self.transition(DeviceState::Locked);
            return Vec::new();
        }

        // Unlocked. Anything the machine decided on the strength of believing
        // it was locked is void: a pending blank was only legitimate because
        // the screen was a lock screen, and the dim warning belongs to that
        // same rule.
        //
        // But only if the machine can actually leave where it is. There is no
        // legal Suspending → Active or Asleep → Active edge (waking goes
        // through Locked), so an unlock reported while the device is asleep is
        // either a logind race or a genuine impossibility. Voiding lock-screen
        // intent while the state stays Asleep would leave the two disagreeing,
        // which is the whole class of bug this function exists to end.
        if !self.transition(DeviceState::Active) {
            warn!(
                "[device-state] logind says unlocked but {:?} has no edge to Active — keeping lock-screen intent",
                self.state
            );
            self.record_error(
                "device-state",
                "unlock-refused",
                &format!("no legal transition from {:?} to Active", self.state),
            );
            return Vec::new();
        }

        info!("[device-state] session unlocked (logind) — clearing lock-screen intent");
        self.blank_requested = false;
        self.pending_blank = None;

        let mut actions = Vec::new();
        if self.dimmed {
            self.dimmed = false;
            actions.push(Action::Restore);
        }
        actions
    }

    /// Decide whether to believe a raw proximity reading yet.
    ///
    /// Returns the *believed* value. A raw reading that disagrees with the
    /// believed one starts a clock; it only wins once it has held for the
    /// direction's threshold. `resolve_proximity_debounce` finishes the job on
    /// the tick, for the case where the sensor reports once and goes quiet.
    fn debounce_proximity(&mut self, raw: bool, now: Instant) -> bool {
        if raw != self.proximity_raw {
            self.proximity_raw = raw;
            self.proximity_since = Some(now);
        }
        let believed = self.sensor_evidence.proximity_near;
        if raw == believed {
            // Nothing pending — the sensor agrees with what we already think.
            self.proximity_since = None;
            return believed;
        }
        let held_for = self
            .proximity_since
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        let threshold = if raw {
            self.policy.proximity_near_debounce
        } else {
            self.policy.proximity_far_debounce
        };
        if held_for >= threshold {
            self.proximity_since = None;
            raw
        } else {
            believed
        }
    }

    /// Let a held reading win once its threshold passes with no new report.
    ///
    /// Without this the debounce would only resolve when the next reading
    /// arrives, and a reporter that heartbeats every 30 s would make a real
    /// `near` take up to half a minute to be believed.
    fn resolve_proximity_debounce(&mut self, now: Instant) {
        if self.proximity_raw == self.sensor_evidence.proximity_near {
            return;
        }
        let held_for = self
            .proximity_since
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        let believed = self.debounce_proximity(self.proximity_raw, now);
        if believed != self.sensor_evidence.proximity_near {
            self.sensor_evidence.proximity_near = believed;

            // Record the edge. Without this the trail would carry a
            // Locked → Observed transition with no sensor input behind it —
            // the reading was filtered on the way in and believed a tick
            // later, so nothing would say why the machine moved. Filtered
            // blips stay out of the trail deliberately; the edge that wins
            // does not.
            let confidence = self.sensor_evidence.confidence();
            let snapshot = self.snapshot("", false, false, false, "", false);
            self.forensic.append(
                ForensicEvent::SensorInput {
                    source: SensorSource::Proximity,
                    value: SensorValue::Near(believed),
                    confidence,
                },
                snapshot,
                &format!(
                    "proximity={believed} believed after holding {} ms",
                    held_for.as_millis()
                ),
            );

            let should_be_observed = self.state == DeviceState::Locked && believed;
            if should_be_observed {
                if self.transition(DeviceState::Observed) {
                    self.observed_since = Some(now);
                }
            } else if matches!(self.state, DeviceState::Observed)
                && !believed
                && self.observed_dwell_elapsed(now)
            {
                self.transition(DeviceState::Locked);
            }
        }
    }

    /// Proximity-shaped convenience wrapper.
    pub fn update_sensors(&mut self, evidence: SensorEvidence) {
        self.update_sensors_from(SensorSource::Proximity, evidence)
    }

    /// Update sensor evidence and, if the current state is Locked,
    /// potentially transition to/from Observed.
    /// Emits forensic entries for every sensor input.
    pub fn update_sensors_from(&mut self, source: SensorSource, evidence: SensorEvidence) {
        self.update_sensors_from_at(source, evidence, Instant::now())
    }

    /// `update_sensors_from` with the clock passed in, so the debounce can be
    /// tested on a timeline instead of by sleeping.
    pub fn update_sensors_from_at(
        &mut self,
        source: SensorSource,
        mut evidence: SensorEvidence,
        now: Instant,
    ) {
        // Proximity is debounced before it is believed. The rest of the
        // evidence is taken as reported — none of it flaps the way this one
        // does, and none of it has the measurement behind it that would justify
        // picking a threshold.
        if source == SensorSource::Proximity {
            evidence.proximity_near = self.debounce_proximity(evidence.proximity_near, now);
        } else {
            evidence.proximity_near = self.sensor_evidence.proximity_near;
        }

        let was_observed = matches!(self.state, DeviceState::Observed);
        let proximity_near = evidence.proximity_near;
        let should_be_observed = self.state == DeviceState::Locked && proximity_near;
        let confidence = evidence.confidence();

        // A reading identical to the one already held is not a decision point.
        //
        // Reporters heartbeat now — they re-send their last value on a fixed
        // interval so that silence means a dead source rather than a quiet one
        // (see `SOURCE_DOWN_AFTER`). Those repeats must not each become a trail
        // entry: proximity alone would add ~2,900 lines a day saying nothing
        // changed, to a log that §0 already records as unbounded and on tmpfs.
        // Freshness is still stamped by the caller either way, which is what
        // the keepalive is actually for.
        let unchanged = self.sensor_evidence.proximity_near == evidence.proximity_near
            && self.sensor_evidence.accel_moving == evidence.accel_moving
            && self.sensor_evidence.light_changing == evidence.light_changing
            && self.sensor_evidence.touch_active == evidence.touch_active;

        self.sensor_evidence = evidence;

        // Only the trail write is skipped, never the evaluation below. An
        // identical reading can still change the answer: proximity held `near`
        // across a lock means `should_be_observed` flips from false to true
        // with no change in the evidence at all.
        if !unchanged {
            // Forensic: record the sensor input and its evaluation
            let snapshot = StateSnapshot {
                device_state: self.state,
                locked: is_locked(self.state),
                display_active: is_display_active(self.state),
                phase: String::new(),
                shell_alive: false,
                proximity_near: self.sensor_evidence.proximity_near,
                confidence,
                suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
                promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
                screen_locked: false,
                screen_lock_secure: false,
                idle_coordinator_state: String::new(),
                sleep_inhibitor_held: false,
                sensors_degraded: self.source_health.any_down(),
                placement: self.placement(),
            };
            self.forensic.append(
                ForensicEvent::SensorInput {
                    // The reporting source, not a hardcoded Proximity. Accel,
                    // light and touch readings were all being written to the trail
                    // labelled as proximity, which makes post-hoc reconstruction —
                    // the entire point of the trail — lie about what was measured.
                    source,
                    value: match source {
                        SensorSource::Proximity => SensorValue::Near(proximity_near),
                        SensorSource::Accelerometer => {
                            SensorValue::Moving(self.sensor_evidence.accel_moving)
                        }
                        SensorSource::Light => SensorValue::Light {
                            changing: self.sensor_evidence.light_changing,
                            lux: self.sensor_evidence.lux.unwrap_or(0.0),
                        },
                        SensorSource::Touch => {
                            SensorValue::Active(self.sensor_evidence.touch_active)
                        }
                        // Charge is ingested by the gate handler, never here.
                        SensorSource::Charge => unreachable!(),
                    },
                    confidence,
                },
                snapshot,
                &format!(
                    "proximity={}, confidence={:.2}, should_observe={}, was_observed={}",
                    proximity_near, confidence, should_be_observed, was_observed
                ),
            );
        }

        if should_be_observed && !was_observed {
            if self.transition(DeviceState::Observed) {
                self.observed_since = Some(now);
            }
        } else if was_observed && !proximity_near && self.observed_dwell_elapsed(now) {
            self.transition(DeviceState::Locked);
        }
    }

    /// Record a wake event (screen on, dt2w, power button, etc.).
    pub fn record_wake(&self, trigger: WakeTrigger, reason: &str) {
        let snapshot = StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: String::new(),
            shell_alive: false,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked: false,
            screen_lock_secure: false,
            idle_coordinator_state: String::new(),
            sleep_inhibitor_held: false,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        };
        self.forensic
            .append(ForensicEvent::Wake { trigger }, snapshot, reason);
    }

    /// Record an error that affected device state.
    pub fn record_error(&self, component: &str, action: &str, error: &str) {
        let snapshot = StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: String::new(),
            shell_alive: false,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked: false,
            screen_lock_secure: false,
            idle_coordinator_state: String::new(),
            sleep_inhibitor_held: false,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        };
        self.forensic.append(
            ForensicEvent::Error {
                component: component.to_string(),
                action: action.to_string(),
                error: error.to_string(),
            },
            snapshot,
            &format!("[{}] {}: {}", component, action, error),
        );
    }

    /// Record a decision (e.g., "suppress DPMS wake", "promote idle").
    pub fn record_decision(&self, decision: &str, inputs: serde_json::Value, reason: &str) {
        let snapshot = StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: String::new(),
            shell_alive: false,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked: false,
            screen_lock_secure: false,
            idle_coordinator_state: String::new(),
            sleep_inhibitor_held: false,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        };
        self.forensic.append(
            ForensicEvent::Decision {
                decision: decision.to_string(),
                inputs,
            },
            snapshot,
            reason,
        );
    }

    /// Record what the somatic plexus said. The body's events go in the same
    /// trail as the machine's, so "the light source went silent at the same
    /// time the panel blanked" is one line to read, not two logs to join.
    pub fn record_somatic(&self, event: &SomaticEvent) {
        let (kind, field, detail) = match event {
            SomaticEvent::TrendShift {
                field,
                from,
                to,
                level,
            } => (
                "trend_shift",
                field.clone(),
                format!("{from:?} -> {to:?} at {level:.2}"),
            ),
            SomaticEvent::SourceSilent {
                source,
                silent_for_secs,
                ..
            } => (
                "source_silent",
                source.clone(),
                format!("silent for {silent_for_secs} s"),
            ),
            SomaticEvent::SourceRecovered { source } => {
                ("source_recovered", source.clone(), String::new())
            }
            SomaticEvent::ViabilityThreatened {
                field,
                pressure,
                eta_secs,
            } => (
                "viability_threatened",
                field.clone(),
                format!(
                    "pressure {pressure:.2}, bound {}",
                    eta_secs.map_or_else(|| "now".to_string(), |s| format!("in {s} s"))
                ),
            ),
        };
        let snapshot = StateSnapshot {
            device_state: self.state,
            locked: is_locked(self.state),
            display_active: is_display_active(self.state),
            phase: String::new(),
            shell_alive: false,
            proximity_near: self.sensor_evidence.proximity_near,
            confidence: self.sensor_evidence.confidence(),
            suppress_dpms_wake: self.suppress_wake(InputTrigger::DoubleTapToWake),
            promote_idle_faster: self.sensor_evidence.should_promote_idle_faster(),
            screen_locked: false,
            screen_lock_secure: false,
            idle_coordinator_state: String::new(),
            sleep_inhibitor_held: false,
            sensors_degraded: self.source_health.any_down(),
            placement: self.placement(),
        };
        let reason = format!("somatic {kind}: {field}");
        self.forensic.append(
            ForensicEvent::Somatic {
                field,
                kind: kind.to_string(),
                detail,
            },
            snapshot,
            &reason,
        );
    }

    /// Serialize the current state for IPC.
    pub fn to_ipc_json(&self) -> serde_json::Value {
        serde_json::json!({
            "device_state": self.state,
            "locked": is_locked(self.state),
            "display_active": is_display_active(self.state),
            "observed": matches!(self.state, DeviceState::Observed),
            "observed_confidence": self.sensor_evidence.confidence(),
            "suppress_dpms_wake": self.suppress_wake(InputTrigger::DoubleTapToWake),
            "panel_on": self.panel_on,
            "user_active": self.user_active(),
            "idle_secs": self.idle_secs(),
            "dimmed": self.dimmed,
            "blank_requested": self.blank_requested,
            // Command admission, not physical state. Actual sleep/shutdown
            // state is learned from logind events rather than inferred from a
            // request or a successful systemctl exit.
            "power_requested": self.power_requested.map(PowerVerb::as_str),
            // Freshness means "reported within the TTL", not "has ever been
            // heard from". The `is_some()` version read as fresh for the whole
            // life of the daemon, because `expire_stale_evidence` only clears
            // the timestamp for a source whose flag was set — so a sensor
            // resting at `far` reported `fresh: true` indefinitely, including
            // through the outage this field exists to make visible.
            "evidence_fresh": {
                "proximity": self.is_fresh(self.evidence_seen.proximity),
                "accel": self.is_fresh(self.evidence_seen.accel),
                "light": self.is_fresh(self.evidence_seen.light),
                "touch": self.is_fresh(self.evidence_seen.touch),
                "charge": self.is_fresh(self.evidence_seen.charge),
            },
            // Seconds since each source last reported; null means never.
            // `fresh` is a threshold answer, this is the raw number — a page
            // rendering "last seen 4 s ago" needs the number, not the bool.
            "evidence_last_seen_secs_ago": {
                "proximity": Self::secs_since(self.evidence_seen.proximity),
                "accel": Self::secs_since(self.evidence_seen.accel),
                "light": Self::secs_since(self.evidence_seen.light),
                "touch": Self::secs_since(self.evidence_seen.touch),
                "charge": Self::secs_since(self.evidence_seen.charge),
            },
            // Health is the other axis: `fresh` says whether the reading may
            // be believed, `health` says whether the source is there at all.
            "sensor_health": self.source_health.as_json(),
            "sensors_degraded": self.source_health.any_down(),
            // Charging as the machine's own evidence — reported by sensord,
            // interpreted here (TASK-33). Surfaces render `conclusion`; they
            // do not re-derive it.
            "charge": self.charge_json(),
        })
    }

    fn is_fresh(&self, seen: Option<Instant>) -> bool {
        seen.is_some_and(|t| {
            Instant::now().saturating_duration_since(t) <= self.policy.evidence_ttl
        })
    }

    fn secs_since(seen: Option<Instant>) -> Option<u64> {
        seen.map(|t| Instant::now().saturating_duration_since(t).as_secs())
    }
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceState::Active => write!(f, "active"),
            DeviceState::Dimmed => write!(f, "dimmed"),
            DeviceState::Locked => write!(f, "locked"),
            DeviceState::Observed => write!(f, "observed"),
            DeviceState::DozeLight => write!(f, "doze_light"),
            DeviceState::DozeDeep => write!(f, "doze_deep"),
            DeviceState::Suspending => write!(f, "suspending"),
            DeviceState::Asleep => write!(f, "asleep"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessiond::bearer::{LinkHealth, TunnelHealth};

    #[test]
    fn initial_state_is_active() {
        let sm = DeviceStateMachine::new();
        assert_eq!(sm.state(), DeviceState::Active);
    }

    /// A locked machine with a lit panel, where the compositor has just
    /// reported that input stopped at `t0`.
    fn locked_and_lit() -> (DeviceStateMachine, Instant) {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Locked);
        assert!(sm.panel_on);
        let t0 = Instant::now();
        sm.note_idle_start(t0);
        (sm, t0)
    }

    /// Unlocked, panel lit, an explicit unlocked blank budget set. This is the
    /// configuration `unlocked_blank_after` exists for; the default is None.
    fn unlocked_and_lit() -> (DeviceStateMachine, Instant) {
        let mut sm = DeviceStateMachine::new();
        assert!(!is_locked(sm.state));
        assert!(sm.panel_on);
        sm.policy.unlocked_blank_after = Some(Duration::from_secs(30));
        sm.policy.dim_warning = false;
        let t0 = Instant::now();
        sm.note_idle_start(t0);
        (sm, t0)
    }

    #[test]
    fn an_unlock_reported_while_asleep_is_refused_not_half_applied() {
        // There is no Asleep → Active edge. Voiding lock-screen intent while
        // the state stays Asleep would leave the two disagreeing.
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Locked);
        sm.transition(DeviceState::Suspending);
        sm.transition(DeviceState::Asleep);
        sm.dimmed = true;
        sm.blank_requested = true;

        assert!(sm.set_session_locked(false).is_empty());
        assert_eq!(sm.state, DeviceState::Asleep, "state must not move");
        assert!(sm.dimmed, "intent must not be voided by a refused unlock");
        assert!(sm.blank_requested);
    }

    #[test]
    fn logind_unlock_releases_the_machine_from_locked() {
        // The bug this closes, measured on hardware 2026-07-26: the machine
        // sat in Locked for 40 minutes while LockedHint said no, because
        // locked_ack was the only lock ingress and there was no unlock one.
        let (mut sm, _t0) = locked_and_lit();
        assert!(is_locked(sm.state));

        sm.set_session_locked(false);
        assert_eq!(sm.state, DeviceState::Active);
        assert!(!is_locked(sm.state));
    }

    #[test]
    fn unlocking_cancels_a_pending_blank_and_restores_the_dim() {
        let (mut sm, t0) = locked_and_lit();
        // Get into the dim pre-warning.
        let dim_at = LOCK_BLANK_AFTER - LOCK_DIM_GRACE;
        assert_eq!(
            sm.tick_at(t0 + dim_at + Duration::from_millis(10)),
            vec![Action::Dim]
        );
        assert!(sm.dimmed);

        // Unlocking voids lock-screen intent and undims.
        assert_eq!(sm.set_session_locked(false), vec![Action::Restore]);
        assert!(!sm.dimmed);
        assert!(sm.pending_blank.is_none());
        assert!(!sm.blank_requested);
    }

    #[test]
    fn relocking_puts_the_machine_back() {
        let (mut sm, _t0) = locked_and_lit();
        sm.set_session_locked(false);
        assert_eq!(sm.state, DeviceState::Active);
        sm.set_session_locked(true);
        assert!(is_locked(sm.state));
    }

    #[test]
    fn an_unlocked_blank_asks_for_the_lock_first() {
        let (mut sm, t0) = unlocked_and_lit();

        // The budget expires: the machine wants the panel dark, but the
        // session is unlocked, so the lock goes first — LOCK-DPMS §1.
        assert_eq!(sm.tick_at(t0 + Duration::from_secs(31)), vec![Action::Lock]);
        assert!(sm.pending_blank.is_some());
        assert!(!sm.blank_requested, "the panel must not be dark yet");
    }

    #[test]
    fn the_agents_screen_verb_cannot_blank_an_unlocked_session() {
        // Doctrine §13 in one assertion. Operation is hers, so she may ask for
        // the screen off — and the ordering invariant is not a permission she
        // lacks, it is a property of the machine that applies to every caller.
        // She gets `Lock` first, exactly as the idle timer does.
        let (mut sm, _t0) = unlocked_and_lit();
        assert_eq!(sm.request_screen(false, "agent"), vec![Action::Lock]);
        assert!(sm.pending_blank.is_some());
        assert!(!sm.blank_requested, "the panel must not be dark yet");
    }

    #[test]
    fn double_tap_to_wake_actually_wakes_the_panel() {
        // The regression that made viewtop feel worse than Hyprland. This
        // function recorded the wake and returned `Restore`, which is
        // brightness — so dt2w reported to the machine and the screen stayed
        // dark. It only ever worked because hyprland.lua bound XF86WakeUp
        // straight to the DPMS toggle and the daemon was never consulted.
        let (mut sm, _t0) = unlocked_and_lit();
        sm.set_panel(false);
        let actions = sm.note_input(InputTrigger::DoubleTapToWake);
        assert!(
            actions.contains(&Action::Unblank),
            "a double tap on a dark panel must light it: {actions:?}"
        );
    }

    #[test]
    fn a_wake_on_a_lit_panel_does_not_unblank_it() {
        let (mut sm, _t0) = unlocked_and_lit();
        assert!(sm.panel_on());
        let actions = sm.note_input(InputTrigger::DoubleTapToWake);
        assert!(!actions.contains(&Action::Unblank));
    }

    #[test]
    fn the_power_button_does_not_unblank_twice() {
        // `apply_gesture` already wakes on a resolved tap. If `note_input`
        // also woke on the down edge there would be two unblanks in flight for
        // one press, and the second would land against a panel the first had
        // lit — which is exactly how the wake loop happened in the compositor.
        let (mut sm, _t0) = unlocked_and_lit();
        sm.set_panel(false);
        let actions = sm.note_input(InputTrigger::PowerButton);
        assert!(
            !actions.contains(&Action::Unblank),
            "the power button's wake belongs to apply_gesture: {actions:?}"
        );
    }

    #[test]
    fn a_volume_press_changes_volume_immediately() {
        // On the DOWN edge, not on a resolved gesture: BUTTON_MULTI_TAP_WINDOW
        // is 300ms and a volume key that lags a third of a second behind the
        // press feels broken.
        let (mut sm, t0) = unlocked_and_lit();
        let up = sm.button_edge_at(Button::VolumeUp, ButtonEdge::Down, t0);
        assert!(up.contains(&Action::Volume { up: true }), "{up:?}");
        let down = sm.button_edge_at(Button::VolumeDown, ButtonEdge::Down, t0);
        assert!(down.contains(&Action::Volume { up: false }), "{down:?}");
    }

    #[test]
    fn releasing_a_volume_key_does_not_change_volume_again() {
        let (mut sm, t0) = unlocked_and_lit();
        sm.button_edge_at(Button::VolumeUp, ButtonEdge::Down, t0);
        let up = sm.button_edge_at(
            Button::VolumeUp,
            ButtonEdge::Up,
            t0 + Duration::from_millis(60),
        );
        assert!(
            !up.iter().any(|a| matches!(a, Action::Volume { .. })),
            "one press is one step: {up:?}"
        );
    }

    #[test]
    fn the_screen_verb_wakes_a_dark_panel() {
        // The direction the machine did not have. Before `Action::Unblank`
        // there was no way out of a blank at all from inside the daemon: a
        // power tap on a dark panel answered `Restore`, which is brightness,
        // and the panel stayed off. It only ever worked because Hyprland's
        // keybinding woke the panel before sessiond was consulted.
        let (mut sm, _t0) = unlocked_and_lit();
        sm.set_panel(false);
        assert!(!sm.panel_on());
        assert_eq!(sm.request_screen(true, "agent"), vec![Action::Unblank]);
    }

    #[test]
    fn a_power_tap_on_a_dark_panel_unblanks_before_it_restores() {
        // Order is the whole of it: the brightness a panel comes back at means
        // nothing until the panel is back. `Restore` alone was what this
        // returned, and under viewtop that set the brightness of a screen that
        // stayed off.
        let (mut sm, t0) = unlocked_and_lit();
        sm.set_panel(false);
        sm.button_edge_at(Button::Power, ButtonEdge::Down, t0);
        let actions = sm.button_edge_at(
            Button::Power,
            ButtonEdge::Up,
            t0 + Duration::from_millis(80),
        );
        let actions = if actions.is_empty() {
            // Taps resolve on the multi-tap window expiring, not on release.
            sm.tick_at(t0 + Duration::from_secs(2))
        } else {
            actions
        };
        let unblank = actions.iter().position(|a| *a == Action::Unblank);
        let restore = actions.iter().position(|a| *a == Action::Restore);
        assert!(
            unblank.is_some(),
            "a tap on a dark panel must wake it: {actions:?}"
        );
        if let (Some(u), Some(r)) = (unblank, restore) {
            assert!(u < r, "the panel must come back before its brightness does");
        }
    }

    #[test]
    fn three_fingers_raise_the_sheet_for_the_window_they_landed_on() {
        let (mut sm, _t0) = unlocked_and_lit();
        let actions = sm.touch_gesture(3, TouchGesture::Tap, Some(7));
        assert_eq!(actions, vec![Action::WindowSheet { target: 7 }]);
    }

    #[test]
    fn holding_power_raises_the_power_menu() {
        let (mut sm, t0) = unlocked_and_lit();
        let actions = sm.apply_gesture(Button::Power, ButtonGesture::Hold, t0);
        assert_eq!(actions, vec![Action::PowerMenu]);
    }

    #[test]
    fn the_power_menu_still_opens_on_a_dark_panel() {
        // Not gated on the panel or the lock: powering off is a thing you do
        // to a locked phone, and a menu that vanished when locked would send
        // the user back to the button they are already holding.
        let (mut sm, t0) = unlocked_and_lit();
        sm.set_panel(false);
        let actions = sm.apply_gesture(Button::Power, ButtonGesture::Hold, t0);
        assert_eq!(actions, vec![Action::PowerMenu]);
    }

    #[test]
    fn a_power_request_is_latched_reported_and_refuses_a_second_verb() {
        let mut sm = DeviceStateMachine::new();

        assert_eq!(
            sm.request_power(PowerVerb::Suspend, "test request")
                .unwrap(),
            vec![Action::Power(PowerVerb::Suspend)]
        );
        assert_eq!(sm.to_ipc_json()["power_requested"], "suspend");

        let refusal = sm
            .request_power(PowerVerb::Reboot, "racing request")
            .unwrap_err();
        assert_eq!(refusal, "suspend is already in flight");
        assert_eq!(sm.to_ipc_json()["power_requested"], "suspend");
    }

    #[test]
    fn only_the_matching_terminal_outcome_releases_a_power_request() {
        let mut sm = DeviceStateMachine::new();
        sm.request_power(PowerVerb::Hibernate, "test request")
            .unwrap();

        assert!(!sm.power_request_finished(PowerVerb::Poweroff));
        assert_eq!(sm.to_ipc_json()["power_requested"], "hibernate");

        assert!(sm.power_request_finished(PowerVerb::Hibernate));
        assert_eq!(sm.to_ipc_json()["power_requested"], serde_json::Value::Null);
        assert_eq!(
            sm.request_power(PowerVerb::Poweroff, "retry after return")
                .unwrap(),
            vec![Action::Power(PowerVerb::Poweroff)]
        );
    }

    #[test]
    fn a_three_finger_tap_on_the_wallpaper_raises_nothing() {
        // The no-window case, decided rather than discovered on device: a tap
        // with no subject is inert. The alternative is a sheet with nothing to
        // act on, which is the "why did a blank sheet appear" bug.
        let (mut sm, _t0) = unlocked_and_lit();
        assert!(sm.touch_gesture(3, TouchGesture::Tap, None).is_empty());
    }

    #[test]
    fn every_other_touch_gesture_is_recognised_and_inert() {
        // Deliberate. A gesture that fires something nobody chose is worse
        // than one that fires nothing, and the binding table this is a
        // placeholder for is DeviceStatePolicy's to hold.
        let (mut sm, _t0) = unlocked_and_lit();
        for g in [
            TouchGesture::SwipeUp,
            TouchGesture::SwipeDown,
            TouchGesture::SwipeLeft,
            TouchGesture::SwipeRight,
        ] {
            assert!(sm.touch_gesture(3, g, Some(1)).is_empty(), "{g:?}");
        }
        for fingers in [1u8, 2, 4, 5] {
            assert!(
                sm.touch_gesture(fingers, TouchGesture::Tap, Some(1))
                    .is_empty(),
                "{fingers} fingers"
            );
        }
    }

    #[test]
    fn the_sheet_does_not_open_on_a_dark_panel() {
        // The sheet is content, and content on an unauthenticated glass is
        // what the disclosure rules exist to prevent. A tap on a dark panel is
        // a wake, and that is the power button's business.
        let (mut sm, _t0) = unlocked_and_lit();
        sm.set_panel(false);
        assert!(sm.touch_gesture(3, TouchGesture::Tap, Some(1)).is_empty());
    }

    #[test]
    fn a_press_that_woke_the_panel_does_not_then_blank_it() {
        // The regression Casey hit: press to wake, the lock screen appears, and
        // the screen goes black again. One press, two reports.
        //
        // `note_input` on the DOWN edge takes the device out of Locked and the
        // executor lights the panel, reporting it back through `set_panel`
        // before the finger is even off the button. The test above never
        // simulated that report, which is why it passed while the phone failed.
        let (mut sm, t0) = unlocked_and_lit();
        sm.set_panel(false);
        sm.button_edge_at(Button::Power, ButtonEdge::Down, t0);
        // The wake this very press caused, landing between the edges.
        sm.set_panel(true);

        let mut actions = sm.button_edge_at(
            Button::Power,
            ButtonEdge::Up,
            t0 + Duration::from_millis(80),
        );
        if actions.is_empty() {
            actions = sm.tick_at(t0 + Duration::from_secs(2));
        }
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Blank)),
            "the press that woke the panel must not also blank it: {actions:?}"
        );
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Lock)),
            "and it must not lock in order to blank: {actions:?}"
        );
    }

    #[test]
    fn a_volume_nudge_does_not_make_the_power_tap_blank_the_screen() {
        // The latch used to be one field for every button. A tap resolves on
        // the multi-tap window closing, up to ~1.3 s after the press, so any
        // other button's DOWN edge inside that gap overwrote it — and on a
        // Pixel 3 the volume rocker is under the same grip as power.
        let (mut sm, t0) = unlocked_and_lit();
        sm.set_panel(false);
        sm.button_edge_at(Button::Power, ButtonEdge::Down, t0);
        sm.set_panel(true); // the wake this press caused
        sm.button_edge_at(
            Button::Power,
            ButtonEdge::Up,
            t0 + Duration::from_millis(80),
        );

        // The rocker, well inside the multi-tap window.
        sm.button_edge_at(
            Button::VolumeUp,
            ButtonEdge::Down,
            t0 + Duration::from_millis(120),
        );
        sm.button_edge_at(
            Button::VolumeUp,
            ButtonEdge::Up,
            t0 + Duration::from_millis(200),
        );

        let actions = sm.tick_at(t0 + Duration::from_secs(2));
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::Blank | Action::Lock)),
            "the volume press must not decide what the power press meant: {actions:?}"
        );
    }

    #[test]
    fn a_press_against_a_lit_panel_still_blanks_it() {
        // The other half — without this the fix would simply disable the power
        // button's only binding.
        let (mut sm, t0) = unlocked_and_lit();
        sm.button_edge_at(Button::Power, ButtonEdge::Down, t0);
        let mut actions = sm.button_edge_at(
            Button::Power,
            ButtonEdge::Up,
            t0 + Duration::from_millis(80),
        );
        if actions.is_empty() {
            actions = sm.tick_at(t0 + Duration::from_secs(2));
        }
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Lock | Action::Blank)),
            "a tap on a lit panel is still lock-then-blank: {actions:?}"
        );
    }

    #[test]
    fn asking_for_the_state_the_panel_is_already_in_does_nothing() {
        let (mut sm, _t0) = unlocked_and_lit();
        assert!(sm.panel_on());
        assert!(sm.request_screen(true, "agent").is_empty());
    }

    #[test]
    fn a_pending_blank_fires_once_the_lock_is_acked() {
        let (mut sm, t0) = unlocked_and_lit();
        let t1 = t0 + Duration::from_secs(31);
        assert_eq!(sm.tick_at(t1), vec![Action::Lock]);

        // Nothing while the lock is still outstanding, well inside the budget.
        assert!(sm.tick_at(t1 + Duration::from_millis(500)).is_empty());

        // The lock lands. Now — and only now — the panel goes dark.
        sm.transition(DeviceState::Locked);
        assert_eq!(
            sm.tick_at(t1 + Duration::from_millis(600)),
            vec![Action::Blank]
        );
        assert!(sm.pending_blank.is_none());
    }

    #[test]
    fn a_lock_that_never_acks_blanks_anyway_and_says_so() {
        let (mut sm, t0) = unlocked_and_lit();
        let t1 = t0 + Duration::from_secs(31);
        assert_eq!(sm.tick_at(t1), vec![Action::Lock]);

        let before = sm.forensic.recent(200).len();

        // Past the ack budget with no lock. §1: dark-but-unlocked beats
        // lit-and-unlocked in a pocket, so the blank proceeds.
        assert_eq!(
            sm.tick_at(t1 + LOCK_ACK_BUDGET + Duration::from_millis(10)),
            vec![Action::Blank]
        );

        // But doctrine §8 forbids pretending it locked: the state is untouched
        // and the trail carries an error, not a decision.
        assert!(!is_locked(sm.state));
        let entries = sm.forensic.recent(200);
        assert!(entries.len() > before);
        assert!(
            entries.iter().any(|e| matches!(
                &e.event,
                ForensicEvent::Error { action, .. } if action == "blank-without-lock"
            )),
            "the unlocked blank must be a loud error in the trail"
        );
    }

    #[test]
    fn input_inside_the_ack_window_cancels_the_blank() {
        let (mut sm, t0) = unlocked_and_lit();
        let t1 = t0 + Duration::from_secs(31);
        assert_eq!(sm.tick_at(t1), vec![Action::Lock]);

        // The user came back within the second-or-so window.
        sm.note_input(InputTrigger::Touch);
        assert!(sm.pending_blank.is_none());

        // And the panel stays lit past where the deadline would have been.
        assert!(sm
            .tick_at(t1 + LOCK_ACK_BUDGET + Duration::from_secs(1))
            .is_empty());
        assert!(!sm.blank_requested);
    }

    #[test]
    fn a_locked_blank_still_goes_straight_to_dark() {
        // The ordering rule must not add a lock round-trip to the case that
        // was already correct: locked screens blank immediately.
        let (mut sm, t0) = locked_and_lit();
        assert_eq!(
            sm.tick_at(t0 + LOCK_BLANK_AFTER + Duration::from_millis(10)),
            vec![Action::Blank]
        );
        assert!(sm.pending_blank.is_none());
    }

    #[test]
    fn lockscreen_dims_as_a_warning_then_blanks() {
        let (mut sm, t0) = locked_and_lit();

        // Nothing at all early on.
        assert!(sm.tick_at(t0 + Duration::from_secs(1)).is_empty());

        // Dim lands one grace-window before the blank, not at it.
        let dim_at = LOCK_BLANK_AFTER - LOCK_DIM_GRACE;
        assert_eq!(
            sm.tick_at(t0 + dim_at + Duration::from_millis(10)),
            vec![Action::Dim]
        );
        // The warning is asked for once, not on every tick.
        assert!(sm.tick_at(t0 + dim_at + Duration::from_secs(1)).is_empty());

        // Then the panel goes dark when the budget runs out.
        assert_eq!(
            sm.tick_at(t0 + LOCK_BLANK_AFTER + Duration::from_millis(10)),
            vec![Action::Blank]
        );
        // And that is asked for once too.
        assert!(sm
            .tick_at(t0 + LOCK_BLANK_AFTER + Duration::from_secs(5))
            .is_empty());
    }

    #[test]
    fn tap_inside_the_grace_window_cancels_the_blank() {
        let (mut sm, t0) = locked_and_lit();
        let dim_at = LOCK_BLANK_AFTER - LOCK_DIM_GRACE;
        assert_eq!(
            sm.tick_at(t0 + dim_at + Duration::from_millis(10)),
            vec![Action::Dim]
        );

        // The user taps: brightness comes back and the budget restarts.
        assert_eq!(sm.note_input(InputTrigger::Touch), vec![Action::Restore]);
        assert!(sm.user_active(), "a tap means the user is here");

        // While the compositor reports the user as active, nothing happens at
        // all — this is the case a self-stamped clock gets wrong.
        assert!(sm.tick_at(t0 + LOCK_BLANK_AFTER * 10).is_empty());

        // Quiet resumes later; the budget runs from there, not from before.
        let tap = t0 + LOCK_BLANK_AFTER * 10;
        sm.note_idle_start(tap);
        let at_old_deadline = tap + LOCK_BLANK_AFTER - Duration::from_secs(1);
        assert!(!sm.tick_at(at_old_deadline).contains(&Action::Blank));

        // And it does eventually blank, one full budget after the tap.
        assert_eq!(
            sm.tick_at(tap + LOCK_BLANK_AFTER + Duration::from_millis(10)),
            vec![Action::Blank]
        );
    }

    #[test]
    fn proximity_alone_never_blanks_the_panel() {
        // A covered sensor is a pocket, a face, a table or a thumb, and the
        // machine cannot tell which. The only reading that justifies turning
        // the screen off is a call, and the machine has no call state yet.
        for locked in [true, false] {
            let (mut sm, t0) = locked_and_lit();
            if !locked {
                sm.set_session_locked(false);
            }
            sm.sensor_evidence.proximity_near = true;
            sm.mark_evidence_seen(SensorSource::Proximity);
            assert!(
                sm.tick_at(t0 + Duration::from_millis(10)).is_empty(),
                "locked={locked}: proximity is evidence, not an actuator"
            );
        }
    }

    #[test]
    fn an_unlocked_screen_is_never_blanked_by_this_rule() {
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.note_idle_start(t0);
        assert_eq!(sm.state(), DeviceState::Active);
        // Hours idle and unlocked: not this rule's business.
        assert!(sm.tick_at(t0 + Duration::from_secs(3600)).is_empty());
    }

    #[test]
    fn never_is_a_real_setting() {
        let (mut sm, t0) = locked_and_lit();
        sm.policy.lock_blank_after = None;
        assert!(sm.tick_at(t0 + Duration::from_secs(3600)).is_empty());
    }

    #[test]
    fn a_held_device_gets_the_longer_budget() {
        let (mut sm, t0) = locked_and_lit();
        sm.sensor_evidence.accel_moving = true;
        sm.mark_evidence_seen(SensorSource::Accelerometer);

        // Past the table budget, but a hand is on it: the panel has not gone
        // dark. (A dim warning is due here — the held budget's grace window
        // starts exactly where the table budget would have blanked.)
        assert!(!sm
            .tick_at(t0 + LOCK_BLANK_AFTER + Duration::from_secs(1))
            .contains(&Action::Blank));
        // The longer budget still ends.
        assert_eq!(
            sm.tick_at(t0 + LOCK_BLANK_AFTER_HELD + Duration::from_millis(10)),
            vec![Action::Blank]
        );
    }

    #[test]
    fn stale_evidence_stops_buying_the_longer_budget() {
        let (mut sm, t0) = locked_and_lit();
        sm.sensor_evidence.accel_moving = true;
        sm.mark_evidence_seen(SensorSource::Accelerometer);

        // The sensor dies: no further readings. Once the TTL passes the
        // reading is unknown, and unknown must not extend the budget —
        // erring toward saving the panel, never toward burning it.
        let late = t0 + EVIDENCE_TTL + LOCK_BLANK_AFTER;
        let actions = sm.tick_at(late);
        assert!(!sm.sensor_evidence.accel_moving, "stale reading must clear");
        assert_eq!(actions, vec![Action::Blank]);
    }

    #[test]
    fn a_squeeze_is_never_vetoed_by_proximity_and_neither_is_a_button() {
        let (mut sm, _t0) = locked_and_lit();
        sm.sensor_evidence.proximity_near = true;
        sm.mark_evidence_seen(SensorSource::Proximity);

        // A squeeze is NOT vetoed by a covered sensor, and this test used to
        // assert the opposite. Measured 2026-08-05: seven deliberate squeezes
        // at deflection 2509-3320 were all refused while the phone was in a
        // hand — holding it to squeeze it is what covers the sensor. Six strain
        // gauges past a calibrated baseline is the stronger signal of the two,
        // and §4 scopes the veto to tap-to-wake alone.
        assert!(!sm.suppress_wake(InputTrigger::Squeeze));
        assert!(sm.note_input_gated(InputTrigger::Squeeze).is_some());

        // Intent from a hardware button is never refused (§4).
        assert!(!sm.suppress_wake(InputTrigger::PowerButton));
        assert!(sm.note_input_gated(InputTrigger::PowerButton).is_some());
    }

    #[test]
    fn an_uncovered_squeeze_is_real_input() {
        let (mut sm, _t0) = locked_and_lit();
        sm.sensor_evidence.proximity_near = false;

        assert!(!sm.suppress_wake(InputTrigger::Squeeze));
        assert!(sm.note_input_gated(InputTrigger::Squeeze).is_some());
        // It resets the idle budget like any other real input.
        assert!(sm.idle_since.is_none());
    }

    #[test]
    fn stale_proximity_stops_suppressing_wake() {
        let (mut sm, t0) = locked_and_lit();
        sm.sensor_evidence.proximity_near = true;
        sm.mark_evidence_seen(SensorSource::Proximity);
        assert!(sm.suppress_wake(InputTrigger::DoubleTapToWake));

        // A dead proximity sensor must not keep vetoing wakes forever.
        sm.tick_at(t0 + EVIDENCE_TTL + Duration::from_secs(1));
        assert!(!sm.suppress_wake(InputTrigger::DoubleTapToWake));
    }

    #[test]
    fn a_burst_of_double_taps_fails_the_pocket_veto_open() {
        let (mut sm, t0) = locked_and_lit();
        sm.sensor_evidence.proximity_near = true;
        sm.mark_evidence_seen(SensorSource::Proximity);
        assert!(sm.suppress_wake(InputTrigger::DoubleTapToWake));

        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0)
            .is_none());
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0 + Duration::from_secs(1))
            .is_none());
        // The third inside the window is a person insisting: fail open.
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0 + Duration::from_secs(2))
            .is_some());

        let bursts: Vec<_> = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Decision { decision, .. } if decision == "input-burst-allowed"))
            .collect();
        assert_eq!(bursts.len(), 1, "the override is said, once, in the trail");

        // The memory discharged with the allow: the next tap starts a fresh
        // count rather than riding the burst.
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0 + Duration::from_secs(3))
            .is_none());
    }

    #[test]
    fn spaced_double_taps_do_not_sum_into_a_burst() {
        let (mut sm, t0) = locked_and_lit();
        sm.sensor_evidence.proximity_near = true;
        sm.mark_evidence_seen(SensorSource::Proximity);

        // 6 s apart is outside the window: each stands alone, so a pocket
        // brushing the screen twice in a minute never opens it.
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0)
            .is_none());
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0 + Duration::from_secs(6))
            .is_none());
        assert!(sm
            .note_input_gated_at(InputTrigger::DoubleTapToWake, t0 + Duration::from_secs(12))
            .is_none());
    }

    #[test]
    fn waking_from_a_dimmed_blank_restores_brightness_first() {
        let (mut sm, t0) = locked_and_lit();
        let dim_at = LOCK_BLANK_AFTER - LOCK_DIM_GRACE;
        sm.tick_at(t0 + dim_at + Duration::from_millis(10));
        sm.tick_at(t0 + LOCK_BLANK_AFTER + Duration::from_millis(10));
        sm.set_panel(false);

        // dt2w wake: the saved brightness is still the dim value, so the
        // restore has to run or the panel comes up looking dead.
        assert_eq!(sm.set_panel(true), vec![Action::Restore]);
    }

    #[test]
    fn pam_unlock_is_legal_from_every_locked_substate() {
        for from in [
            DeviceState::Locked,
            DeviceState::Observed,
            DeviceState::DozeLight,
            DeviceState::DozeDeep,
        ] {
            let mut sm = DeviceStateMachine::new();
            sm.transition(DeviceState::Locked);
            // Walk the real graph to get there; doze tiers promote in order.
            match from {
                DeviceState::Locked => {}
                DeviceState::Observed => assert!(sm.transition(DeviceState::Observed)),
                DeviceState::DozeLight => assert!(sm.transition(DeviceState::DozeLight)),
                DeviceState::DozeDeep => {
                    assert!(sm.transition(DeviceState::DozeLight));
                    assert!(sm.transition(DeviceState::DozeDeep));
                }
                other => panic!("unexpected setup state {other:?}"),
            }
            assert!(
                sm.transition(DeviceState::Active),
                "unlock from {from:?} must be legal"
            );
        }
    }

    #[test]
    fn legal_transitions_work() {
        let mut sm = DeviceStateMachine::new();
        assert!(sm.transition(DeviceState::Dimmed));
        assert_eq!(sm.state(), DeviceState::Dimmed);
        assert!(sm.transition(DeviceState::Locked));
        assert_eq!(sm.state(), DeviceState::Locked);
    }

    #[test]
    fn illegal_transitions_refused() {
        let mut sm = DeviceStateMachine::new();
        // Active → DozeLight is not legal (must go through Locked first)
        assert!(!sm.transition(DeviceState::DozeLight));
        assert_eq!(sm.state(), DeviceState::Active); // unchanged
    }

    #[test]
    fn idempotent_transition() {
        let mut sm = DeviceStateMachine::new();
        assert!(sm.transition(DeviceState::Active)); // same state
        assert_eq!(sm.state(), DeviceState::Active);
    }

    #[test]
    fn active_to_suspending_allowed() {
        let mut sm = DeviceStateMachine::new();
        // Any pre-sleep state → Suspending is legal (logind authority)
        assert!(sm.transition(DeviceState::Suspending));
        assert_eq!(sm.state(), DeviceState::Suspending);
    }

    #[test]
    fn asleep_to_locked() {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Dimmed);
        sm.transition(DeviceState::Locked);
        sm.transition(DeviceState::Suspending);
        sm.transition(DeviceState::Asleep);
        assert!(sm.transition(DeviceState::Locked));
        assert_eq!(sm.state(), DeviceState::Locked);
    }

    #[test]
    fn sensor_evidence_confidence() {
        let e = SensorEvidence {
            proximity_near: true,
            accel_moving: false,
            light_changing: false,
            touch_active: false,
            lux: None,
        };
        assert!((e.confidence() - 0.4).abs() < 0.01);

        let e2 = SensorEvidence {
            proximity_near: true,
            accel_moving: true,
            light_changing: true,
            touch_active: true,
            lux: None,
        };
        // 0.4 + 0.3 + 0.2 + 0.1 - 0.2 (disagreement) = 0.8
        assert!((e2.confidence() - 0.8).abs() < 0.01);
    }

    #[test]
    fn sensor_disagreement_reduces_confidence() {
        // Proximity near + accel moving: a pocket, an ear and a hand all read
        // this way. It lowers confidence and decides nothing.
        let e = SensorEvidence {
            proximity_near: true,
            accel_moving: true,
            light_changing: false,
            touch_active: false,
            lux: None,
        };
        // 0.4 + 0.3 - 0.2 = 0.5
        assert!((e.confidence() - 0.5).abs() < 0.01);
        // Idle promotion not accelerated (0.5 < 0.6).
        assert!(!e.should_promote_idle_faster());
    }

    // ── Proximity debounce ────────────────────────────────────────────

    /// A locked machine and a clock, for driving proximity on a timeline.
    fn locked_for_proximity() -> (DeviceStateMachine, Instant) {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Locked);
        (sm, Instant::now())
    }

    fn prox(near: bool) -> SensorEvidence {
        SensorEvidence {
            proximity_near: near,
            ..Default::default()
        }
    }

    /// Report proximity the way `server.rs` does — the reading and the
    /// freshness stamp together. A test that only does the first half has a
    /// source that is instantly stale.
    fn report_prox(sm: &mut DeviceStateMachine, near: bool, at: Instant) {
        sm.mark_evidence_seen_at(SensorSource::Proximity, at);
        sm.update_sensors_from_at(SensorSource::Proximity, prox(near), at);
    }

    /// Report `near` and let it hold long enough to be believed.
    fn hold_prox_near(sm: &mut DeviceStateMachine, at: Instant) -> Instant {
        report_prox(sm, true, at);
        let settled = at + PROXIMITY_NEAR_DEBOUNCE + Duration::from_millis(1);
        sm.mark_evidence_seen_at(SensorSource::Proximity, settled);
        sm.tick_at(settled);
        settled
    }

    #[test]
    fn a_believed_near_followed_by_far_a_second_later_does_not_flap() {
        // The measured case, 2026-08-03. The phone's trail was almost entirely
        // Locked -> Observed -> Locked, and the snapshots said why: prox=true
        // on the way in, prox=false one second later on the way out. `near`
        // holds 700 ms and is believed; `far` is believed instantly; the round
        // trip costs two transitions and two trail entries for a reading that
        // never meant anything.
        let (mut sm, t0) = locked_for_proximity();

        let settled = hold_prox_near(&mut sm, t0);
        assert_eq!(sm.state, DeviceState::Observed, "a held near is believed");

        // Far, one second after entering — inside the dwell.
        let blip_ends = settled + Duration::from_secs(1);
        report_prox(&mut sm, false, blip_ends);
        sm.tick_at(blip_ends);
        assert_eq!(
            sm.state,
            DeviceState::Observed,
            "a one-second episode must not bounce the state back"
        );

        // Past the dwell, `far` still ends it — this must not become a state
        // the phone cannot leave.
        let after = settled + OBSERVED_MIN_DWELL + Duration::from_millis(1);
        report_prox(&mut sm, false, after);
        sm.tick_at(after);
        assert_eq!(
            sm.state,
            DeviceState::Locked,
            "the dwell delays the exit, it does not remove it"
        );
    }

    #[test]
    fn a_real_proximity_episode_still_ends_when_it_ends() {
        // The nine real episodes of the 44 measured all ran >= 5 s. The dwell
        // is 3 s precisely so it sits under every one of them: covering a
        // sensor for a real interval must behave exactly as it did before.
        let (mut sm, t0) = locked_for_proximity();
        let settled = hold_prox_near(&mut sm, t0);
        assert_eq!(sm.state, DeviceState::Observed);

        let uncovered = settled + Duration::from_secs(5);
        report_prox(&mut sm, false, uncovered);
        sm.tick_at(uncovered);
        assert_eq!(
            sm.state,
            DeviceState::Locked,
            "a five-second episode ends on the reading that ends it"
        );
    }

    #[test]
    fn the_wake_veto_still_lifts_the_instant_the_sensor_clears() {
        // The dwell must not leak into `suppress_wake`. That reads the
        // debounced evidence, and PROXIMITY_FAR_DEBOUNCE is zero exactly so a
        // wake is never refused after the phone is out of the pocket. Holding
        // the *state* longer must not hold the *veto* longer.
        let (mut sm, t0) = locked_for_proximity();
        let settled = hold_prox_near(&mut sm, t0);
        assert!(
            sm.suppress_wake(InputTrigger::DoubleTapToWake),
            "covered: a pocket double-tap is vetoed"
        );

        let clear = settled + Duration::from_millis(200);
        report_prox(&mut sm, false, clear);
        assert!(
            !sm.suppress_wake(InputTrigger::DoubleTapToWake),
            "uncovered: the veto lifts at once, dwell or no dwell"
        );
        assert_eq!(
            sm.state,
            DeviceState::Observed,
            "and the state is still holding its dwell, which is the whole point"
        );
    }

    #[test]
    fn a_sub_second_near_blip_is_never_believed() {
        // The measured case, 2026-07-26: 44 near-episodes in 2.4 hours, median
        // dwell 1 s, 15 of them sub-second. Each one produced two transitions
        // and two trail entries for a reading that meant nothing.
        let (mut sm, t0) = locked_for_proximity();

        report_prox(&mut sm, true, t0);
        assert_eq!(sm.state, DeviceState::Locked, "near is not believed yet");
        assert!(!sm.sensor_evidence.proximity_near);

        report_prox(&mut sm, false, t0 + Duration::from_millis(400));
        sm.tick_at(t0 + Duration::from_millis(500));
        assert_eq!(
            sm.state,
            DeviceState::Locked,
            "the blip must leave no transition behind"
        );
    }

    #[test]
    fn a_held_near_is_believed_once_it_has_held() {
        // The nine real episodes ran 5 s to 173 s. They must survive untouched.
        let (mut sm, t0) = locked_for_proximity();
        report_prox(&mut sm, true, t0);
        assert_eq!(sm.state, DeviceState::Locked);

        // No further reading arrives — the tick has to finish the job, or a
        // reporter that heartbeats every 30 s would delay a real near by half
        // a minute.
        hold_prox_near(&mut sm, t0);
        assert!(sm.sensor_evidence.proximity_near);
        assert_eq!(sm.state, DeviceState::Observed);
    }

    #[test]
    fn far_is_believed_at_once() {
        // The reading that ends a veto is never the slow one.
        let (mut sm, t0) = locked_for_proximity();
        hold_prox_near(&mut sm, t0);
        assert_eq!(sm.state, DeviceState::Observed);

        let t1 = t0 + Duration::from_secs(10);
        report_prox(&mut sm, false, t1);
        assert!(!sm.sensor_evidence.proximity_near);
        assert_eq!(sm.state, DeviceState::Locked);
    }

    #[test]
    fn a_flapping_sensor_that_never_settles_is_never_believed() {
        // Alternating faster than the threshold: the near edge keeps restarting
        // its clock, so nothing is ever believed and the trail stays quiet.
        let (mut sm, t0) = locked_for_proximity();
        let mut t = t0;
        for _ in 0..20 {
            report_prox(&mut sm, true, t);
            t += Duration::from_millis(200);
            report_prox(&mut sm, false, t);
            t += Duration::from_millis(200);
        }
        assert_eq!(sm.state, DeviceState::Locked);
        assert!(!sm.sensor_evidence.proximity_near);
    }

    #[test]
    fn a_heartbeat_repeat_does_not_restart_the_debounce() {
        // Reporters re-send their last value every 30 s (§10). A repeat is the
        // same edge continuing, not a new one — if it reset the clock, a held
        // near would never be believed.
        let (mut sm, t0) = locked_for_proximity();
        report_prox(&mut sm, true, t0);
        report_prox(&mut sm, true, t0 + Duration::from_millis(500));
        report_prox(
            &mut sm,
            true,
            t0 + PROXIMITY_NEAR_DEBOUNCE + Duration::from_millis(1),
        );
        assert!(sm.sensor_evidence.proximity_near);
        assert_eq!(sm.state, DeviceState::Observed);
    }

    #[test]
    fn a_non_proximity_reading_does_not_disturb_the_debounce() {
        // accel/light/touch share the evidence struct; updating one must not
        // silently overwrite a proximity value mid-debounce.
        let (mut sm, t0) = locked_for_proximity();
        report_prox(&mut sm, true, t0);

        let accel = SensorEvidence {
            proximity_near: false, // the caller's stale copy — must be ignored
            accel_moving: true,
            ..Default::default()
        };
        sm.mark_evidence_seen_at(SensorSource::Accelerometer, t0 + Duration::from_millis(100));
        sm.update_sensors_from_at(
            SensorSource::Accelerometer,
            accel,
            t0 + Duration::from_millis(100),
        );

        sm.mark_evidence_seen_at(
            SensorSource::Proximity,
            t0 + PROXIMITY_NEAR_DEBOUNCE + Duration::from_millis(1),
        );
        sm.tick_at(t0 + PROXIMITY_NEAR_DEBOUNCE + Duration::from_millis(1));
        assert!(sm.sensor_evidence.accel_moving);
        assert!(
            sm.sensor_evidence.proximity_near,
            "the proximity edge must still resolve on its own clock"
        );
    }

    #[test]
    fn the_pre_dim_brightness_is_remembered_and_consumed_once() {
        // Casey's report: the panel dims, and a tap to dismiss brings it back
        // at a different level. Cause was `blueline-undim`'s floor — anything
        // restoring under 26/255 was pushed to 40%, so a phone deliberately
        // run dark came back brighter than it started. The machine remembers
        // the number now instead of the executor guessing at it.
        let mut sm = DeviceStateMachine::new();
        assert_eq!(
            sm.take_brightness_before_dim(),
            None,
            "nothing captured yet"
        );

        sm.note_brightness_before_dim(Some(18));
        assert_eq!(
            sm.take_brightness_before_dim(),
            Some(18),
            "a value below the old floor must survive the round trip unchanged"
        );
        assert_eq!(
            sm.take_brightness_before_dim(),
            None,
            "taken, not read — a stale capture must not survive into the next dim"
        );
    }

    #[test]
    fn a_failed_brightness_read_falls_through_to_the_floor() {
        // The one case the floor is right for: nothing knows what the panel
        // was at, so coming back dark is worse than coming back wrong.
        let mut sm = DeviceStateMachine::new();
        sm.note_brightness_before_dim(None);
        assert_eq!(sm.take_brightness_before_dim(), None);
    }

    #[test]
    fn only_tap_to_wake_is_vetoed_by_a_covered_sensor() {
        // A double tap is the one wake a pocket can produce by itself. The
        // power button is intent and is never refused, whatever the sensors
        // say, locked or not.
        let mut sm = DeviceStateMachine::new();
        sm.sensor_evidence.proximity_near = true;
        sm.sensor_evidence.accel_moving = true;

        for state in [DeviceState::Active, DeviceState::Locked] {
            sm.transition(state);
            assert!(sm.suppress_wake(InputTrigger::DoubleTapToWake));
            assert!(!sm.suppress_wake(InputTrigger::PowerButton));
            assert!(!sm.suppress_wake(InputTrigger::Touch));
        }
    }

    #[test]
    fn proximity_triggers_observed() {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Dimmed);
        sm.transition(DeviceState::Locked);

        // Held, not blipped — near is debounced now, so a reading that does not
        // last is a reading the machine never believed.
        hold_prox_near(&mut sm, Instant::now());
        assert_eq!(sm.state(), DeviceState::Observed);
    }

    #[test]
    fn proximity_far_returns_to_locked() {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Dimmed);
        sm.transition(DeviceState::Locked);

        // Enter observed
        let settled = hold_prox_near(&mut sm, Instant::now());
        assert_eq!(sm.state(), DeviceState::Observed);

        // Leave observed. Past OBSERVED_MIN_DWELL, because a `far` inside the
        // dwell is now deliberately ignored — see that constant. The property
        // this test is named for is unchanged: far ends Observed.
        let after = settled + OBSERVED_MIN_DWELL + Duration::from_millis(1);
        sm.update_sensors_from_at(
            SensorSource::Proximity,
            SensorEvidence {
                proximity_near: false,
                ..Default::default()
            },
            after,
        );
        assert_eq!(sm.state(), DeviceState::Locked);
    }

    #[test]
    fn is_locked_covers_all_locked_states() {
        assert!(!is_locked(DeviceState::Active));
        assert!(!is_locked(DeviceState::Dimmed));
        assert!(is_locked(DeviceState::Locked));
        assert!(is_locked(DeviceState::Observed));
        assert!(is_locked(DeviceState::DozeLight));
        assert!(is_locked(DeviceState::DozeDeep));
        assert!(is_locked(DeviceState::Suspending));
        assert!(is_locked(DeviceState::Asleep));
    }

    #[test]
    fn display_active_only_active_and_dimmed() {
        assert!(is_display_active(DeviceState::Active));
        assert!(is_display_active(DeviceState::Dimmed));
        assert!(!is_display_active(DeviceState::Locked));
        assert!(!is_display_active(DeviceState::Asleep));
    }

    #[test]
    fn full_lifecycle_with_forensic_log() {
        // Exercise the entire state machine: a phone's day in 30 seconds.
        let mut sm = DeviceStateMachine::new();

        // 1. User picks up the phone — Active
        assert_eq!(sm.state(), DeviceState::Active);
        sm.record_wake(WakeTrigger::UserInput, "user picked up phone");

        // 2. User stops touching — screen dims after 120s
        sm.transition(DeviceState::Dimmed);
        sm.record_decision(
            "dim screen",
            serde_json::json!({"idle_seconds": 120, "inhibitor": false}),
            "native IdleMonitor fired, no inhibitor held",
        );

        // 3. User still idle — lock after 300s
        sm.transition(DeviceState::Locked);
        sm.record_decision(
            "lock session",
            serde_json::json!({"idle_seconds": 300, "inhibitor": false}),
            "native IdleMonitor fired, requesting lock",
        );

        // 4. Phone goes into pocket — proximity near, held. A pocket is not a
        // one-second blip; the debounce is what tells them apart.
        hold_prox_near(&mut sm, Instant::now());
        sm.update_sensors(SensorEvidence {
            proximity_near: true,
            accel_moving: false,
            light_changing: false,
            touch_active: false,
            lux: None,
        });
        assert_eq!(sm.state(), DeviceState::Observed);
        sm.record_decision(
            "suppress DPMS wake",
            serde_json::json!({"confidence": 0.4, "proximity": "near"}),
            "proximity near, confidence 0.4 >= 0.3 threshold",
        );

        // 5. Phone stays in pocket — promote to DozeLight faster
        sm.transition(DeviceState::DozeLight);
        sm.record_decision(
            "freeze app tier",
            serde_json::json!({"idle_minutes": 5, "promoted_faster": true}),
            "observed state accelerated promotion, freezing apps.slice",
        );

        // 6. Phone still idle — promote to DozeDeep
        sm.transition(DeviceState::DozeDeep);
        sm.record_decision(
            "stop network fetchers",
            serde_json::json!({"idle_minutes": 15}),
            "deep doze, only RTC + modem IRQs active",
        );

        // 7. User presses power button — wake to lock screen
        sm.transition(DeviceState::Locked);
        sm.record_wake(WakeTrigger::PowerButton, "user pressed power button");

        // 8. Phone goes back in pocket. Held — "briefly" is precisely what the
        // debounce now refuses to believe, which is the point of it.
        let pocketed = hold_prox_near(&mut sm, Instant::now());
        assert_eq!(sm.state(), DeviceState::Observed);

        // 9. Phone comes back out — after OBSERVED_MIN_DWELL, since a pocket
        // that lasted less than that is a blip rather than a pocket.
        sm.update_sensors_from_at(
            SensorSource::Proximity,
            SensorEvidence {
                proximity_near: false,
                ..Default::default()
            },
            pocketed + OBSERVED_MIN_DWELL + Duration::from_millis(1),
        );
        assert_eq!(sm.state(), DeviceState::Locked);

        // 10. System suspends
        sm.transition(DeviceState::Suspending);
        sm.transition(DeviceState::Asleep);

        // 11. RTC alarm wakes the phone
        sm.transition(DeviceState::Locked);
        sm.record_wake(WakeTrigger::RtcAlarm, "RTC alarm for notification check");

        // 12. User unlocks with PAM
        sm.transition(DeviceState::Active);

        // Verify the full lifecycle completed
        assert_eq!(sm.state(), DeviceState::Active);

        // Dump the forensic log
        let entries = sm.forensic.recent(100);
        println!("\n=== FORENSIC LOG ({} entries) ===", entries.len());
        for entry in &entries {
            let event_name = match &entry.event {
                ForensicEvent::Transition { from, to, legal } => {
                    format!("transition {:?} → {:?} (legal={})", from, to, legal)
                }
                ForensicEvent::SensorInput {
                    source, confidence, ..
                } => {
                    format!("sensor {:?} conf={:.2}", source, confidence)
                }
                ForensicEvent::Wake { trigger } => {
                    format!("wake {:?}", trigger)
                }
                ForensicEvent::Error {
                    component, action, ..
                } => {
                    format!("error [{}] {}", component, action)
                }
                ForensicEvent::Decision { decision, .. } => {
                    format!("decision: {}", decision)
                }
                ForensicEvent::Somatic { field, kind, .. } => {
                    format!("somatic {kind}: {field}")
                }
                ForensicEvent::Heartbeat => "heartbeat".to_string(),
            };
            println!(
                "  seq={:3} ts={} {:30} state={:12} locked={} conf={:.2} reason={}",
                entry.seq,
                entry.ts,
                event_name,
                format!("{:?}", entry.snapshot.device_state),
                entry.snapshot.locked,
                entry.snapshot.confidence,
                entry.reason,
            );
        }
        println!("=== END FORENSIC LOG ===\n");

        // Verify we captured the key events
        assert!(
            entries.len() >= 12,
            "expected at least 12 forensic entries, got {}",
            entries.len()
        );
        // Verify transitions were captured
        let transitions: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.event, ForensicEvent::Transition { .. }))
            .collect();
        assert!(
            transitions.len() >= 8,
            "expected at least 8 transitions, got {}",
            transitions.len()
        );
        // Verify sensor inputs were captured
        let sensors: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.event, ForensicEvent::SensorInput { .. }))
            .collect();
        assert!(
            sensors.len() >= 2,
            "expected at least 2 sensor inputs, got {}",
            sensors.len()
        );
        // Verify wake events were captured
        let wakes: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.event, ForensicEvent::Wake { .. }))
            .collect();
        assert!(
            wakes.len() >= 2,
            "expected at least 2 wake events, got {}",
            wakes.len()
        );
    }

    #[test]
    fn illegal_transition_is_forensically_logged() {
        let mut sm = DeviceStateMachine::new();
        // Try an illegal transition: Active → DozeLight (must go through Locked)
        assert!(!sm.transition(DeviceState::DozeLight));
        let entries = sm.forensic.recent(10);
        let refused: Vec<_> = entries
            .iter()
            .filter(|e| match &e.event {
                ForensicEvent::Transition { legal, .. } => !legal,
                _ => false,
            })
            .collect();
        assert_eq!(
            refused.len(),
            1,
            "expected 1 refused transition in forensic log"
        );
        match &refused[0].event {
            ForensicEvent::Transition { from, to, legal } => {
                assert_eq!(*from, DeviceState::Active);
                assert_eq!(*to, DeviceState::DozeLight);
                assert!(!legal);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn cross_sensor_disagreement_logged() {
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Dimmed);
        sm.transition(DeviceState::Locked);
        // Proximity near + accel moving = walking with phone. Near is
        // debounced, so it has to be held before the pair is on the record.
        let t0 = Instant::now();
        hold_prox_near(&mut sm, t0);
        sm.update_sensors_from_at(
            SensorSource::Accelerometer,
            SensorEvidence {
                proximity_near: true,
                accel_moving: true,
                light_changing: false,
                touch_active: false,
                lux: None,
            },
            t0 + PROXIMITY_NEAR_DEBOUNCE + Duration::from_millis(2),
        );
        let entries = sm.forensic.recent(10);
        let sensor_entries: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.event, ForensicEvent::SensorInput { .. }))
            .collect();
        assert!(!sensor_entries.is_empty());
        // The confidence should reflect the disagreement (0.4 + 0.3 - 0.2 = 0.5)
        match &sensor_entries[sensor_entries.len() - 1].event {
            ForensicEvent::SensorInput { confidence, .. } => {
                assert!(
                    (confidence - 0.5).abs() < 0.01,
                    "expected 0.5 confidence, got {}",
                    confidence
                );
            }
            _ => unreachable!(),
        }
    }

    // ── Evidence-source health ───────────────────────────────────────
    // The distinction these cover is DEVICE-STATE-MACHINE.md §0's: "'No
    // evidence' and 'evidence says nothing is happening' must not be the same
    // state." The 2026-07-25 outage ran four hours with nothing in any log.

    #[test]
    fn a_source_that_never_reported_is_unknown_not_down() {
        // A light sensor with no reporter installed is silent, correctly,
        // forever. Calling that an outage would make the signal worthless.
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(60));
        assert_eq!(sm.source_health.light, SourceHealth::Unknown);
        assert_eq!(sm.source_health.proximity, SourceHealth::Unknown);
        assert!(!sm.source_health.any_down());
    }

    #[test]
    fn a_source_that_reported_and_went_silent_is_down_and_loud() {
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.mark_evidence_seen_at(SensorSource::Proximity, t0);

        // Still within the window: quiet is allowed.
        sm.tick_at(t0 + SOURCE_DOWN_AFTER - Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Live);

        sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Down);
        assert!(sm.source_health.any_down());

        // The outage is an error in the trail, not just a state field —
        // otherwise nothing after the fact can find the window.
        let errors: Vec<_> = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Error { action, .. } if action == "source-down"))
            .collect();
        assert_eq!(errors.len(), 1, "expected exactly one source-down entry");
    }

    #[test]
    fn an_expected_source_that_never_reports_is_absent_and_loud() {
        // The regression this exists for: souveraine-sensord was `enabled` but
        // never started (its unit hung off a target nothing activates), so
        // proximity/light/accel had no stamp at all. `Down` starts from a
        // stamp, so it could not see this, and the machine ran a whole session
        // on no evidence with nothing in the trail.
        let mut sm = DeviceStateMachine::new();
        let t0 = sm.started_at;

        // Inside the grace window: a reporter is allowed to arrive late.
        sm.tick_at(t0 + SOURCE_EXPECTED_WITHIN - Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Unknown);
        assert!(!sm.source_health.any_down());

        sm.tick_at(t0 + SOURCE_EXPECTED_WITHIN + Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Absent);
        assert_eq!(sm.source_health.light, SourceHealth::Absent);
        assert_eq!(sm.source_health.charge, SourceHealth::Absent);
        assert!(
            sm.source_health.any_down(),
            "absent evidence is degraded evidence"
        );

        // Touch has no reporter on this device, so it must stay silent — a
        // permanent false alarm is the same defect in the other direction.
        assert_eq!(sm.source_health.touch, SourceHealth::Unknown);

        // Neither does the accelerometer, since `6b67512` unclaimed it. This
        // assert is the one that was missing: listing it as expected lit the
        // degraded banner on every snapshot for ten days.
        assert_eq!(sm.source_health.accel, SourceHealth::Unknown);

        let errors: Vec<_> = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Error { action, .. } if action == "source-never-reported"))
            .collect();
        assert_eq!(errors.len(), 3, "one entry per expected source, once");
    }

    #[test]
    fn an_absent_source_is_recorded_once_not_every_tick() {
        let mut sm = DeviceStateMachine::new();
        let t0 = sm.started_at;
        for i in 0..10 {
            sm.tick_at(t0 + SOURCE_EXPECTED_WITHIN + Duration::from_secs(1 + i));
        }
        let errors = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Error { action, .. } if action == "source-never-reported"))
            .count();
        assert_eq!(errors, 3, "three expected sources, one entry each");
    }

    #[test]
    fn a_late_reporter_clears_absent_and_bounds_the_gap() {
        let mut sm = DeviceStateMachine::new();
        let t0 = sm.started_at;
        sm.tick_at(t0 + SOURCE_EXPECTED_WITHIN + Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Absent);

        // The reporter finally starts. Absent must close like Down does, or
        // the trail says when evidence went missing and never when it returned.
        sm.mark_evidence_seen_at(
            SensorSource::Proximity,
            t0 + SOURCE_EXPECTED_WITHIN + Duration::from_secs(5),
        );
        assert_eq!(sm.source_health.proximity, SourceHealth::Live);

        let recovered = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Decision { decision, .. } if decision == "source-recovered"))
            .count();
        assert_eq!(recovered, 1);
    }

    #[test]
    fn a_down_source_is_recorded_once_not_every_tick() {
        // A tick loop that re-logged this every second would bury the
        // transition under 3600 identical lines an hour.
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.mark_evidence_seen_at(SensorSource::Proximity, t0);
        for i in 0..10 {
            sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(1 + i));
        }
        let errors = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Error { action, .. } if action == "source-down"))
            .count();
        assert_eq!(errors, 1);
    }

    #[test]
    fn a_recovered_source_goes_live_and_bounds_the_outage() {
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.mark_evidence_seen_at(SensorSource::Proximity, t0);
        sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(1));
        assert_eq!(sm.source_health.proximity, SourceHealth::Down);

        sm.mark_evidence_seen_at(SensorSource::Proximity, t0 + Duration::from_secs(200));
        assert_eq!(sm.source_health.proximity, SourceHealth::Live);
        assert!(!sm.source_health.any_down());

        // The recovery entry is what closes the interval. Without it the trail
        // says when the sensors died and never says when they came back.
        let recovered = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| matches!(&e.event, ForensicEvent::Decision { decision, .. } if decision == "source-recovered"))
            .count();
        assert_eq!(recovered, 1);
    }

    #[test]
    fn decisions_taken_during_an_outage_are_stamped_degraded() {
        // The point of the whole feature: a panel-off recorded during four
        // dead hours must not read like a healthy one.
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.mark_evidence_seen_at(SensorSource::Proximity, t0);
        sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(1));

        sm.record_decision("panel-off", serde_json::json!({}), "test");
        let entry = sm
            .forensic
            .recent(1)
            .into_iter()
            .next()
            .expect("a decision was just recorded");
        assert!(
            entry.snapshot.sensors_degraded,
            "a decision taken while a source is down must say so"
        );
    }

    #[test]
    fn freshness_expires_rather_than_meaning_ever_seen() {
        // `evidence_fresh` used to be `is_some()`, which read true for the
        // life of the daemon and stayed true straight through an outage.
        let mut sm = DeviceStateMachine::new();
        sm.mark_evidence_seen_at(
            SensorSource::Proximity,
            Instant::now() - (EVIDENCE_TTL + Duration::from_secs(5)),
        );
        let json = sm.to_ipc_json();
        assert_eq!(
            json["evidence_fresh"]["proximity"],
            serde_json::json!(false)
        );

        sm.mark_evidence_seen(SensorSource::Proximity);
        let json = sm.to_ipc_json();
        assert_eq!(json["evidence_fresh"]["proximity"], serde_json::json!(true));
    }

    #[test]
    fn last_seen_crosses_the_wire_as_seconds_ago() {
        // TASK-08(f): the machine held last-seen internally and nothing
        // outside could read it. null is "never reported", a different claim
        // from "long ago".
        let mut sm = DeviceStateMachine::new();
        let json = sm.to_ipc_json();
        assert_eq!(
            json["evidence_last_seen_secs_ago"]["proximity"],
            serde_json::Value::Null
        );

        sm.mark_evidence_seen_at(
            SensorSource::Proximity,
            Instant::now() - Duration::from_secs(30),
        );
        let json = sm.to_ipc_json();
        let secs = json["evidence_last_seen_secs_ago"]["proximity"]
            .as_u64()
            .expect("a stamp 30 s old crosses as a number");
        assert!((30..=31).contains(&secs), "got {secs}");
        assert_eq!(
            json["evidence_last_seen_secs_ago"]["light"],
            serde_json::Value::Null,
            "a source that never reported stays null"
        );
    }

    #[test]
    fn a_repeated_reading_is_not_a_trail_entry() {
        // Reporters heartbeat every 30s so silence is meaningful. If each
        // repeat were logged, proximity alone would add ~2,900 lines a day to
        // an unbounded tmpfs file, all of them saying nothing happened.
        let mut sm = DeviceStateMachine::new();
        sm.transition(DeviceState::Locked);
        let near = SensorEvidence {
            proximity_near: true,
            ..Default::default()
        };
        sm.update_sensors(near.clone());
        let after_first = sm.forensic.recent(100).len();

        for _ in 0..5 {
            sm.update_sensors(near.clone());
        }
        assert_eq!(
            sm.forensic.recent(100).len(),
            after_first,
            "identical readings must not each append to the trail"
        );
    }

    #[test]
    fn a_repeated_reading_still_reevaluates_observed() {
        // The dedupe skips the trail write, never the decision. Proximity held
        // `near` across a lock flips should_be_observed with no change in the
        // evidence at all — an early return here would strand the machine.
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        let settled = hold_prox_near(&mut sm, t0);
        assert_eq!(sm.state, DeviceState::Active);
        assert!(sm.sensor_evidence.proximity_near, "near is believed by now");

        sm.transition(DeviceState::Locked);
        report_prox(&mut sm, true, settled + Duration::from_secs(1));
        assert_eq!(
            sm.state,
            DeviceState::Observed,
            "an unchanged reading must still be re-evaluated against the new state"
        );
    }

    #[test]
    fn a_policy_round_trips_through_json() {
        // The bug this closes: SetPolicy mutated memory and nothing wrote it
        // down, so every timer set in Settings reverted on the next restart.
        let mut p = DeviceStatePolicy::default();
        p.lock_blank_after = Some(Duration::from_secs(300));
        p.dim_warning = false;
        let raw = serde_json::to_string(&p).expect("policy serializes");
        let back: DeviceStatePolicy = serde_json::from_str(&raw).expect("policy parses");
        assert_eq!(back.lock_blank_after, Some(Duration::from_secs(300)));
        assert!(!back.dim_warning);
    }

    #[test]
    fn a_policy_file_missing_new_fields_still_loads() {
        // An older sessiond's file must not make the device fall back to
        // built-in timers on upgrade. `serde(default)` at the container level
        // is what guarantees it; this is the test that keeps it there.
        let raw = r#"{"lock_blank_after":{"secs":300,"nanos":0},"dim_warning":true}"#;
        let p: DeviceStatePolicy = serde_json::from_str(raw).expect("partial policy parses");
        assert_eq!(p.lock_blank_after, Some(Duration::from_secs(300)));
        // Filled from Default, not left at zero.
        assert_eq!(p.lock_ack_budget, LOCK_ACK_BUDGET);
        assert_eq!(p.source_down_after, SOURCE_DOWN_AFTER);
        assert_eq!(p.evidence_ttl, EVIDENCE_TTL);
    }

    // ── The trail itself: durable, bounded, chained ───────────────────

    fn a_snapshot() -> StateSnapshot {
        DeviceStateMachine::new().snapshot("test", false, false, false, "", false)
    }

    fn write_n(log: &ForensicLog, n: usize) {
        let snap = a_snapshot();
        for i in 0..n {
            log.append(
                ForensicEvent::Heartbeat,
                snap.clone(),
                &format!("entry {i}"),
            );
        }
    }

    /// Recompute a line's hash the way an external verifier must: strip the
    /// trailing `hash` key, close the object, SHA-256 what is left.
    fn rehash(line: &str) -> String {
        let cut = line.rfind(",\"hash\":").expect("the hash is the last key");
        sha256_hex(&format!("{}}}", &line[..cut]))
    }

    fn chain_of(path: &std::path::Path) -> Vec<(u64, String, String)> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).expect("a parseable entry");
                let hash = v["hash"].as_str().expect("every entry carries its hash");
                assert_eq!(rehash(line), hash, "the hash must cover the entry body");
                (
                    v["seq"].as_u64().expect("seq"),
                    v["prev"].as_str().unwrap_or_default().to_string(),
                    hash.to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn every_entry_chains_to_the_one_before_it() {
        // §5 called the trail tamper-evident while it carried only a seq,
        // which detects a deleted line and nothing else. This is the claim.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        write_n(&ForensicLog::with_path(path.clone()), 4);

        let chain = chain_of(&path);
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0].1, "", "the head of a new chain has no predecessor");
        for pair in chain.windows(2) {
            assert_eq!(pair[1].0, pair[0].0 + 1, "seq is contiguous");
            assert_eq!(pair[1].1, pair[0].2, "prev is the previous entry's hash");
        }
    }

    #[test]
    fn the_trail_survives_a_reopen_and_keeps_one_chain() {
        // The whole point of leaving tmpfs: a reboot must not erase what the
        // machine decided, and the chain must not restart at zero either.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        write_n(&ForensicLog::with_path(path.clone()), 3);
        write_n(&ForensicLog::with_path(path.clone()), 3);

        let chain = chain_of(&path);
        assert_eq!(chain.len(), 6, "the earlier run is still there");
        assert_eq!(chain[3].0, 3, "seq continues across the restart");
        assert_eq!(
            chain[3].1, chain[2].2,
            "the new run chains onto the old one rather than starting over"
        );
    }

    #[test]
    fn the_trail_rotates_and_the_set_stays_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        // Small enough that a handful of entries fills a generation.
        write_n(&ForensicLog::with_path_bounded(path.clone(), 2048), 400);

        let live = std::fs::metadata(&path).expect("a live file").len();
        assert!(live <= 2048, "the live file is bounded: {live}");

        let mut total = live;
        for n in 1..=FORENSIC_KEEP {
            let g = path.with_extension(format!("jsonl.{n}"));
            total += std::fs::metadata(&g).map(|m| m.len()).unwrap_or(0);
        }
        assert!(
            total <= 2048 * (FORENSIC_KEEP as u64 + 1),
            "the whole set is bounded: {total}"
        );
        // And nothing beyond the kept generations survives.
        assert!(
            !path
                .with_extension(format!("jsonl.{}", FORENSIC_KEEP + 1))
                .exists(),
            "generations past the keep count are deleted, not accumulated"
        );
    }

    #[test]
    fn the_chain_runs_across_a_rotation() {
        // A rotated set must verify as one chain, or bounding the trail would
        // have quietly cost the property that durability was for.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        write_n(&ForensicLog::with_path_bounded(path.clone(), 2048), 12);

        let rotated = chain_of(&path.with_extension("jsonl.1"));
        let live = chain_of(&path);
        assert!(
            !rotated.is_empty() && !live.is_empty(),
            "a rotation happened"
        );
        let last_rotated = rotated.last().expect("rotated entries");
        assert_eq!(
            live[0].1, last_rotated.2,
            "the first live entry chains onto the last rotated one"
        );
        assert_eq!(live[0].0, last_rotated.0 + 1, "seq does not restart");
    }

    #[test]
    fn a_torn_tail_is_discarded_and_the_chain_resumes() {
        // Measured on hardware 2026-07-26: a hard reboot left the trail at
        // exactly 4096 bytes — a page boundary — with the final entry cut in
        // half. The first version treated that as tampering and rotated the
        // whole file aside, so every hard reboot started a new chain at the
        // one moment continuity is worth most.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        write_n(&ForensicLog::with_path(path.clone()), 3);

        // Tear the last write the way losing power does: a partial line, no
        // newline, mid-object.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open");
            write!(f, "{{\"seq\":3,\"prev\":\"abc\",\"ts\":17850").expect("tear it");
        }

        write_n(&ForensicLog::with_path(path.clone()), 1);

        assert!(
            !path.with_extension("jsonl.1").exists(),
            "a torn tail is not a rotation event"
        );
        let chain = chain_of(&path);
        assert_eq!(chain.len(), 4, "three intact entries plus the new one");
        assert_eq!(chain[3].0, 3, "seq continues rather than restarting");
        assert_eq!(
            chain[3].1, chain[2].2,
            "the entry after the tear chains onto the last intact one"
        );
    }

    #[test]
    fn a_file_with_nothing_parseable_is_still_rotated_aside() {
        // Appending onto a line we cannot parse would produce a chain that
        // fails verification forever after, which reads as tampering. The
        // damaged file is evidence, so it is kept, not deleted.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        std::fs::write(&path, "this is not JSON at all\nnor is this\n").expect("write junk");

        write_n(&ForensicLog::with_path(path.clone()), 1);

        let kept = path.with_extension("jsonl.1");
        assert!(kept.exists(), "the unusable trail is kept as evidence");
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap().lines().count(),
            2,
            "kept whole, not truncated"
        );
        let fresh = chain_of(&path);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].0, 0, "the new chain starts clean");
        assert_eq!(
            fresh[0].1, "",
            "and does not claim a predecessor it cannot verify"
        );
    }

    #[test]
    fn an_intent_stamps_every_entry_it_covers_and_no_others() {
        // The point of the field: a chain is one decision and several verbs,
        // and a trail that records only the leaves cannot tell a considered
        // sequence from four accidents.
        let log = ForensicLog::new();
        let snap = a_snapshot();

        log.append(ForensicEvent::Heartbeat, snap.clone(), "before");
        log.set_intent(Some("quiet the room".into()));
        log.append(ForensicEvent::Heartbeat, snap.clone(), "during");
        log.append(ForensicEvent::Heartbeat, snap.clone(), "still during");
        log.set_intent(None);
        log.append(ForensicEvent::Heartbeat, snap, "after");

        let got: Vec<_> = log
            .recent(10)
            .into_iter()
            .map(|e| (e.reason, e.intent))
            .collect();
        assert_eq!(got[0].1, None, "entries before the intent are untouched");
        assert_eq!(got[1].1.as_deref(), Some("quiet the room"));
        assert_eq!(got[2].1.as_deref(), Some("quiet the room"));
        assert_eq!(
            got[3].1, None,
            "an intent that outlived its request would mislabel whatever came next"
        );
    }

    #[test]
    fn an_entry_without_an_intent_serializes_exactly_as_it_did_before() {
        // `skip_serializing_if` is what keeps the hash contract from moving:
        // an entry with no intent must not gain an `"intent":null`, or every
        // line written before today would fail verification against the code
        // that wrote it.
        let entry = ForensicEntry {
            seq: 0,
            prev: String::new(),
            ts: 0,
            event: ForensicEvent::Heartbeat,
            snapshot: a_snapshot(),
            reason: "x".into(),
            intent: None,
        };
        let json = serde_json::to_string(&entry).expect("serializes");
        assert!(!json.contains("intent"), "absent must mean absent: {json}");
    }

    #[test]
    fn a_trail_with_nowhere_to_write_still_answers_ipc() {
        // Memory-only is the honest answer when there is no home; it must not
        // take the in-memory buffer down with it, because the IPC query is how
        // the shell reads the trail.
        let log = ForensicLog::new();
        assert_eq!(log.chain_state(), "memory-only");
        write_n(&log, 3);
        assert_eq!(log.recent(10).len(), 3);
    }

    // ---- bearer: the anti-flap rule is the whole point ----

    fn machine_with_bearer(wifi: LinkHealth, cellular: LinkHealth) -> DeviceStateMachine {
        let mut m = DeviceStateMachine::new();
        m.note_bearer(BearerEvidence {
            wifi,
            cellular,
            ..Default::default()
        });
        m
    }

    #[test]
    fn a_bearer_change_waits_for_the_settling_window() {
        let mut m = machine_with_bearer(LinkHealth::Carrying, LinkHealth::Associated);
        let t0 = Instant::now();

        // First sight of a preference is not enough. This is the property the
        // 652-recycle gate did not have: it acted on every event it saw.
        let a = m.tick_at(t0);
        assert!(
            !a.iter().any(|x| matches!(x, Action::PreferLink(_))),
            "acted before the window: {a:?}"
        );

        let early = t0 + BEARER_SETTLE - Duration::from_secs(1);
        let a = m.tick_at(early);
        assert!(!a.iter().any(|x| matches!(x, Action::PreferLink(_))));

        let late = t0 + BEARER_SETTLE + Duration::from_secs(1);
        let a = m.tick_at(late);
        assert!(
            a.contains(&Action::PreferLink(Bearer::Wifi)),
            "should have settled on wifi: {a:?}"
        );
    }

    #[test]
    fn a_preference_that_reverses_inside_the_window_never_acts() {
        // The flap case, stated exactly: wifi appears, then goes away again
        // before it settled. No command may be produced by that at all.
        let mut m = machine_with_bearer(LinkHealth::Carrying, LinkHealth::Carrying);
        let t0 = Instant::now();
        let _ = m.tick_at(t0);

        m.note_bearer(BearerEvidence {
            wifi: LinkHealth::Down,
            cellular: LinkHealth::Carrying,
            ..Default::default()
        });
        let mid = t0 + BEARER_SETTLE / 2;
        assert!(!m
            .tick_at(mid)
            .iter()
            .any(|x| matches!(x, Action::PreferLink(_))));

        // Back to wifi before either candidate matured.
        m.note_bearer(BearerEvidence {
            wifi: LinkHealth::Carrying,
            cellular: LinkHealth::Carrying,
            ..Default::default()
        });
        let later = t0 + BEARER_SETTLE + Duration::from_secs(1);
        let a = m.tick_at(later);
        assert!(
            !a.iter().any(|x| matches!(x, Action::PreferLink(_))),
            "the clock must restart when the candidate changes, not accumulate: {a:?}"
        );
    }

    #[test]
    fn a_settled_preference_is_not_re_emitted_every_tick() {
        // Idempotence at the decision, not at the executor. A command per tick
        // would be the same disease as a command per event.
        let mut m = machine_with_bearer(LinkHealth::Carrying, LinkHealth::Absent);
        let t0 = Instant::now();
        let _ = m.tick_at(t0);
        let settled = t0 + BEARER_SETTLE + Duration::from_secs(1);
        assert!(m
            .tick_at(settled)
            .contains(&Action::PreferLink(Bearer::Wifi)));

        for i in 1..5 {
            let a = m.tick_at(settled + Duration::from_secs(i));
            assert!(
                !a.iter().any(|x| matches!(x, Action::PreferLink(_))),
                "re-emitted on a steady state at +{i}s: {a:?}"
            );
        }
    }

    #[test]
    fn no_usable_link_produces_no_action() {
        let mut m = machine_with_bearer(LinkHealth::Down, LinkHealth::Absent);
        let t0 = Instant::now();
        let _ = m.tick_at(t0);
        let a = m.tick_at(t0 + BEARER_SETTLE + Duration::from_secs(1));
        assert!(
            !a.iter().any(|x| matches!(x, Action::PreferLink(_))),
            "flailed at a dead network: {a:?}"
        );
    }

    #[test]
    fn the_tunnel_is_pinned_with_the_link_but_never_switched_on() {
        let mut m = DeviceStateMachine::new();
        m.note_bearer(BearerEvidence {
            wifi: LinkHealth::Carrying,
            cellular: LinkHealth::Associated,
            tunnel: TunnelHealth::Handshaking,
            ..Default::default()
        });
        let t0 = Instant::now();
        let _ = m.tick_at(t0);
        let a = m.tick_at(t0 + BEARER_SETTLE + Duration::from_secs(1));
        assert!(a.contains(&Action::PinTunnelUnderlay(Bearer::Wifi)));

        // Tunnel off: the preference still lands, the pin does not. Deciding
        // to bring a tunnel up is the user's call, not this machine's.
        let mut m = DeviceStateMachine::new();
        m.note_bearer(BearerEvidence {
            wifi: LinkHealth::Carrying,
            tunnel: TunnelHealth::Off,
            ..Default::default()
        });
        let _ = m.tick_at(t0);
        let a = m.tick_at(t0 + BEARER_SETTLE + Duration::from_secs(1));
        assert!(a.contains(&Action::PreferLink(Bearer::Wifi)));
        assert!(
            !a.iter().any(|x| matches!(x, Action::PinTunnelUnderlay(_))),
            "pinned an underlay for a tunnel the user switched off: {a:?}"
        );
    }

    fn charge_fields(
        plugged: Option<bool>,
        status: Option<&str>,
        charge_type: Option<&str>,
    ) -> ChargeFields {
        ChargeFields {
            plugged,
            status: status.map(str::to_owned),
            charge_type: charge_type.map(str::to_owned),
            capacity: None,
        }
    }

    #[test]
    fn the_conclusion_is_one_decision() {
        // Moved here with the interpretation: it lives on the machine that
        // consumes the evidence, never in the reporter's type.
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(Some(true), Some("Full"), None)),
            "charged"
        );
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(
                Some(true),
                Some("Charging"),
                Some("Fast")
            )),
            "charging_fast"
        );
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(
                Some(true),
                Some("Charging"),
                Some("Trickle")
            )),
            "charging_slow"
        );
        // The state nothing else could name: cable in, pack topped up,
        // charger resting between hysteresis top-ups. Not a fault.
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(
                Some(true),
                Some("Not charging"),
                None
            )),
            "resting"
        );
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(
                Some(true),
                Some("Discharging"),
                None
            )),
            "resting",
            "discharging while plugged is the resting state, never an alarm"
        );
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(Some(false), None, None)),
            "on_battery"
        );
        assert_eq!(
            DeviceStateMachine::conclude_charge(&charge_fields(None, None, None)),
            "unknown"
        );
    }

    #[test]
    fn charge_enters_through_the_sensor_gate_and_gets_the_same_health() {
        // The reading arrives like any other source's: evidence in, freshness
        // stamped, health Live — and only the machine's conclusion on the
        // wire, never the reporter's.
        let mut sm = DeviceStateMachine::new();
        sm.mark_evidence_seen_at(SensorSource::Charge, Instant::now());
        assert_eq!(
            sm.source_health.get_mut(SensorSource::Charge),
            &mut SourceHealth::Live
        );
        assert!(sm.evidence_seen.charge.is_some());

        sm.note_charge(charge_fields(Some(true), Some("Charging"), Some("Fast")));
        let ipc = sm.to_ipc_json();
        assert_eq!(ipc["charge"]["conclusion"], "charging_fast");
        assert_eq!(ipc["charge"]["status"], "Charging");
        assert_eq!(ipc["sensor_health"]["charge"], "live");
        assert_eq!(
            ipc["evidence_last_seen_secs_ago"]["charge"].is_number(),
            true
        );
        assert_eq!(ipc["sensors_degraded"], false);

        // A conclusion edge writes one trail entry, not one per report — the
        // keepalive re-sends the identical reading and must not repeat it.
        sm.note_charge(charge_fields(Some(true), Some("Charging"), Some("Fast")));
        let conclusion_edges: Vec<_> = sm
            .forensic
            .recent(50)
            .into_iter()
            .filter(|e| {
                matches!(
                    &e.event,
                    ForensicEvent::Decision { decision, .. } if decision == "charge-conclusion"
                )
            })
            .collect();
        assert_eq!(conclusion_edges.len(), 1);
    }

    #[test]
    fn a_charge_source_that_stops_going_down_is_visible_as_degraded() {
        // The reporter dies: the source says nothing for longer than
        // SOURCE_DOWN_AFTER, and the machine calls it Down like any other —
        // a battery readout frozen at its last value is an outage, not a
        // reading.
        let mut sm = DeviceStateMachine::new();
        let t0 = Instant::now();
        sm.mark_evidence_seen_at(SensorSource::Charge, t0);
        sm.tick_at(t0 + SOURCE_DOWN_AFTER + Duration::from_secs(1));
        assert_eq!(
            sm.source_health.get_mut(SensorSource::Charge),
            &mut SourceHealth::Down
        );
        assert!(sm.to_ipc_json()["sensors_degraded"] == true);
    }
}
