//! Orchestration for souveraine-sessiond: the control socket, the shell
//! heartbeat, and the retake state machine.
//!
//! Sync, std-only, thread-per-connection — the same auditable shape as
//! machined's server. The daemon's whole job is to make sure that at every
//! moment either (a) it holds the session lock, (b) a live shell does, or
//! (c) the user deliberately unlocked at the fallback surface. Transitions
//! between those states are the only logic here.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::{info, warn};

use crate::belief::NervousNode;
use crate::plexus::{self, SensorReading, SomaticPlexus};
use crate::sessiond::device_state::{
    is_notable, Action, DeviceState, DeviceStateMachine, SensorEvidence,
};
use crate::sessiond::idle;
use crate::sessiond::lock::{self, LockController, Msg, SessionOutcome};
use crate::sessiond::lockhint;
use crate::sessiond::protocol::{
    Envelope, InputTrigger, Phase, PowerVerb, RefusalCode, Request, SensorSource, SensorValue,
    UsbMode, LOCKED_ACK_TIMEOUT_SECS, MAX_REQUEST_BYTES, VERBS,
};

/// How often the machine's clock runs. One second is fine: the shortest
/// budget it enforces is measured in seconds, and a locked idle phone should
/// not be woken more often than it must be.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Sample bearer posture every Nth tick. See the comment at the call site for
/// why this rides the existing clock instead of getting a timer.
const BEARER_PROBE_EVERY: u64 = 5;

/// Ambient lux below which `attended` cannot be believed, whatever the other
/// sensors say. **Provisional** — 10 lux is a lit room, far from a pocket's
/// zero and from daylight's thousands; set it from a real trail, not a guess.
const ATTENDED_MIN_LUX: f64 = 10.0;

/// Lux to a belief magnitude. Saturating, so daylight (10^4) and a bright
/// room (10^2) land 0.99 and 0.50 instead of linear crushing both to one:
/// levels ride three orders of magnitude without a threshold owning them.
fn lux_to_belief(lux: f64) -> f32 {
    (lux / (lux + 100.0)) as f32
}

/// Whether the phone is being attended this instant: the panel is lit, the
/// ambient light is high enough to see, nothing covers the sensor, and the
/// phone is either being touched or is in motion — a hand, not a pocket, and
/// not a face-down table. Fed as the `attended` field at 1 Hz; the plexus
/// does the believing and the decaying.
fn attended_amount(d: &Daemon) -> f32 {
    let e = &d.device_state.sensor_evidence;
    let lit = e.lux.map_or(false, |lux| lux >= ATTENDED_MIN_LUX);
    let attended =
        d.device_state.panel_on() && lit && !e.proximity_near && (e.touch_active || e.accel_moving);
    if attended {
        1.0
    } else {
        0.0
    }
}

/// How long a connection may sit without sending a request before it is closed.
///
/// Exempts the registered shell heartbeat, whose whole job is to stay silent.
/// Generous on purpose: this is a leak backstop, not a policy. Every real
/// client here either speaks immediately or is the heartbeat.
const IDLE_CONNECTION_TIMEOUT_SECS: u64 = 120;

/// The one userspace authority for panel DPMS on blueline. sessiond decides;
/// this serializes the transition and owns the FTS pacing lesson.
const DPMS_EXECUTOR: &str = "blueline-screen-toggle";

/// Restores brightness after a dim, with the 10% floor. Never a bare
/// `brightnessctl -r` — see Pixel3Arch b9f83f0.
const UNDIM_EXECUTOR: &str = "blueline-undim";

struct Daemon {
    phase: Phase,
    /// Present exactly while a lock-session thread is running.
    controller: Option<LockController>,
    /// Bumped on every shell registration; a heartbeat EOF only counts if
    /// its generation is still current (a restarted shell supersedes).
    heartbeat_gen: u64,
    shell_alive: bool,
    /// SO_PEERCRED pid of the process holding the authority lease. The lease
    /// is per-connection, but the *shell* is a process, and a quickshell scene
    /// reload opens a second connection from the same pid while the first is
    /// still open. Without this the daemon cannot tell its own shell's
    /// successor from an impostor, and refuses the successor forever (TASK-48).
    shell_pid: Option<i32>,
    /// Write handle to the live shell's heartbeat connection, so the authority
    /// can push a directive down it instead of only answering requests. The
    /// shell owns the rich lock surface in steady state; this is how the
    /// daemon that owns the *decision* reaches the process that owns the
    /// *surface*.
    shell_directives: Option<UnixStream>,
    /// Connections that asked to be told, rather than to ask.
    ///
    /// The shell gets directives because it owns a surface the authority needs
    /// driven. These get *events*, and own nothing — an observer that cannot
    /// act cannot be a second authority by accident. Dead ones are pruned on
    /// the write that fails; there is no reaper, because a subscriber that
    /// never receives anything is indistinguishable from a quiet device and
    /// pruning on silence would be §10's mistake again.
    subscribers: Vec<UnixStream>,
    /// Highest trail seq already pushed to subscribers.
    last_pushed_seq: u64,
    /// The unified device state machine. The single authority for device
    /// power state — idle, lock, doze, sleep. All actors route through it.
    device_state: DeviceStateMachine,
    /// The somatic plexus. The body's afferents believed into fields and
    /// ranked pressures; sessiond feeds it from the sensor lane and the
    /// clock, and reads its `body` section back to the shell.
    plexus: SomaticPlexus,
}

pub struct Shared {
    state: Mutex<Daemon>,
    cond: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Daemon> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn socket_path() -> Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join(crate::sessiond::protocol::SOCKET_RELPATH))
}

pub fn run(initial_lock: bool, socket_path: &Path) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).ok();
    }
    if socket_path.exists() {
        if UnixStream::connect(socket_path).is_ok() {
            anyhow::bail!(
                "another souveraine-sessiond is already serving {}",
                socket_path.display()
            );
        }
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding {}", socket_path.display()))?;
    // Same-user only today. When agents get their own accounts, this widens
    // to a group grant deliberately, machined-style — never by default.
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;

    let shared = Arc::new(Shared {
        state: Mutex::new(Daemon {
            phase: Phase::Idle,
            controller: None,
            heartbeat_gen: 0,
            shell_alive: false,
            shell_pid: None,
            shell_directives: None,
            subscribers: Vec::new(),
            last_pushed_seq: 0,
            device_state: DeviceStateMachine::new(),
            plexus: SomaticPlexus::phone_plexus(Utc::now()),
        }),
        cond: Condvar::new(),
    });

    if initial_lock {
        spawn_lock_session(&shared);
    }
    spawn_clock(&shared);
    spawn_idle_watcher(&shared);
    spawn_lock_hint_watcher(&shared);
    info!("sessiond serving on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || handle_connection(s, shared));
            }
            Err(e) => warn!("accept failed: {e}"),
        }
    }
    Ok(())
}

/// The machine's clock. Ticks the state machine and carries out whatever it
/// decided.
///
/// The daemon had no periodic anything before this: every `Duration` in it was
/// a one-shot handshake deadline. That is the whole reason the state machine
/// never moved — it could answer requests, but nothing ever told it that time
/// had passed, so it could not be the proactive half of the nervous system.
fn spawn_clock(shared: &Arc<Shared>) {
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let mut tick_count: u64 = 0;
        loop {
            std::thread::sleep(TICK_INTERVAL);
            tick_count = tick_count.wrapping_add(1);

            // Bearer evidence is sampled off the existing clock rather than given a
            // timer of its own — TASK-49 acceptance #6, "no new timer". Every fifth
            // tick, because the probe costs five subprocesses and the settling
            // window is 20 s: four samples per window is enough to establish that a
            // change held, and 1 Hz would be five processes a second forever.
            //
            // Probed with the lock *released*. Holding the state mutex across
            // subprocess spawns is exactly the self-deadlock shape that put 347
            // threads in __futex_wait on 2026-07-26.
            if tick_count.is_multiple_of(BEARER_PROBE_EVERY) {
                let home = shared.lock().device_state.policy.home_ssids.clone();
                let evidence = crate::sessiond::bearer::probe(&home);
                shared.lock().device_state.note_bearer(evidence);
            }

            // Hold the lock only to decide, never while running a command.
            let actions = {
                let mut d = shared.lock();
                // The body's clock runs on the machine's: `attended` is not a
                // sensor anyone reports, it is a relation between panel,
                // proximity, light and touch that sessiond is the only actor
                // who can see — so sessiond feeds it, at 1 Hz.
                let now = Utc::now();
                let attended = attended_amount(&d);
                d.plexus.ingest(
                    SensorReading {
                        source: "attended".into(),
                        amount: attended,
                    },
                    now,
                );
                d.plexus.tick(now);
                // The body's events land in the machine's trail, so "the
                // light source went silent" and "the panel blanked" read as
                // one timeline instead of two logs to join.
                let events = d.plexus.notable();
                for event in &events {
                    d.device_state.record_somatic(event);
                }
                d.device_state.tick()
            };
            for action in actions {
                execute(&shared, action);
            }
            // After the actions, so an executor's own `error-operational` is in the
            // trail before the tick's events go out. A subscriber that hears the
            // blank but not the brightnessctl failure underneath it has been told a
            // tidier story than what happened.
            fan_out(&shared);
        }
    });
}

/// Feed compositor input truth into the machine.
///
/// Without this the machine has a clock but no idea whether anyone is
/// touching the device, and the only other actor holding that knowledge is
/// hypridle — which is why the lock screen's timeout lived in hypridle's
/// config with a single 600 s timer instead of in the authority that owns the
/// lock.
fn spawn_idle_watcher(shared: &Arc<Shared>) {
    let shared = Arc::clone(shared);
    idle::spawn(Arc::new(move |event| match event {
        idle::IdleEvent::Idled { since } => {
            shared.lock().device_state.note_idle_start(since);
        }
        idle::IdleEvent::Resumed => {
            let actions = {
                let mut d = shared.lock();
                d.device_state.note_input(InputTrigger::Unknown)
            };
            for action in actions {
                execute(&shared, action);
            }
        }
    }));
}

/// Feed logind's lock truth into the machine.
///
/// Without this the machine tracked `locked` itself and had no unlock ingress,
/// so after the first unlock it believed the session was locked forever — and
/// then blanked an in-use phone on proximity, because the proximity rule is
/// gated on `is_locked`. Doctrine §4: never hold state the protocol owns.
fn spawn_lock_hint_watcher(shared: &Arc<Shared>) {
    let shared = Arc::clone(shared);
    lockhint::spawn(Arc::new(move |locked| {
        let actions = {
            let mut d = shared.lock();
            d.device_state.set_session_locked(locked)
        };
        for action in actions {
            execute(&shared, action);
        }
    }));
}

/// Carry out one decision. Failures are recorded in the forensic trail — a
/// blank that did not happen is a state divergence, not a log line to lose.
/// Ask logind whether this session is locked. Fail closed: if the question
/// cannot be answered, the answer is "not locked", because the failure this
/// guards is a dark screen on an open session.
fn session_is_locked() -> bool {
    match crate::sessiond::lockhint::resolve_session_path()
        .and_then(|p| crate::sessiond::lockhint::read_locked_hint(&p))
    {
        Ok(locked) => locked,
        Err(e) => {
            warn!("could not read LockedHint ({e:#}); treating the session as unlocked");
            false
        }
    }
}

fn execute(shared: &Arc<Shared>, action: Action) {
    // The lock is not a shell command, so it does not go through the executor
    // table below. It is the authority acting as the authority: the shell owns
    // the rich lock surface in steady state and executes the directive, and
    // sessiond takes the lock itself when no shell is alive to do it. Either
    // way the decision is the daemon's — doctrine §11, the session authority
    // and the binary authority are the same authority, and it is not the UID
    // that makes it one (§1: everything running as you is you).
    if action == Action::Lock {
        request_session_lock(shared);
        return;
    }

    if let Action::UsbMode(mode) = action {
        if let Err(error) = set_usb_mode(mode) {
            warn!("USB mode {} failed: {error:#}", mode.as_str());
            shared
                .lock()
                .device_state
                .record_error("device-state", "usb-mode", &error.to_string());
        }
        return;
    }

    // Dim and restore carry a number, so they are not table entries. The
    // machine remembers the exact brightness the panel was at; the executor
    // reads the hardware and sets it back. Neither `brightnessctl -s/-r` nor
    // the floor in `blueline-undim` can tell a mistakenly-saved dim value from
    // a phone the user deliberately runs dark — that guess is what made a
    // tap-to-dismiss come back at a different level than it started.
    if action == Action::Dim {
        let before = read_brightness();
        if before.is_none() {
            warn!("could not read brightness before dimming — restore will use the floor");
        }
        shared
            .lock()
            .device_state
            .note_brightness_before_dim(before);
    }
    if action == Action::Restore {
        // Bind FIRST, then branch. `if let Some(v) = shared.lock()...` keeps the
        // temporary MutexGuard alive for the whole if-let body on edition 2021,
        // and the body calls run_executor, which locks again — a self-deadlock
        // on a non-reentrant mutex. It held the state lock forever, so every
        // later request thread piled up behind it: measured on the phone
        // 2026-07-26, 347 threads in __futex_wait and 691 of 1024 fds, with the
        // accept loop still healthy and not one request ever answered.
        let saved = shared.lock().device_state.take_brightness_before_dim();
        if let Some(value) = saved {
            run_executor(
                shared,
                "brightnessctl",
                &["set", &value.to_string()],
                "panel-restore",
                action,
            );
            return;
        }
        // No capture. This is the only case the floor is right for: the panel
        // may be sitting at the dim level with nothing that knows better.
        run_executor(shared, UNDIM_EXECUTOR, &[], "panel-restore", action);
        return;
    }

    // Bearer posture is expressed to NetworkManager, never written with `ip
    // route`. NM already writes every route on this device; a second writer is
    // how the metric fight that started TASK-49 became unresolvable, and
    // acceptance #6 forbids adding one. `connection modify` + `device reapply`
    // leaves NM the sole author and makes the preference survive a reconnect.
    if let Action::PreferLink(bearer) = action {
        apply_link_preference(shared, bearer);
        return;
    }
    // Pinning the tunnel's endpoint is the one route sessiond does own: it is
    // a /32 host route for the WireGuard peer, which NM has no opinion about
    // because the endpoint is not part of any connection's config.
    if let Action::PinTunnelUnderlay(bearer) = action {
        pin_tunnel_underlay(shared, bearer);
        return;
    }

    // The sheet's target, rendered once and outliving the match so the table's
    // `&str` arguments can borrow it. Every other row's arguments are static;
    // this is the first action that carries a value into its own command line.
    let sheet_target: String;

    let (program, args, label): (&str, Vec<&str>, &str) = match action {
        // sessiond guarantees this runs once per dim, which is what the old
        // hypridle `-s`/`-r` listener could not: a second save while already
        // dim is what pinned brightness at 10/255.
        // Percent, not an absolute. `set 10` is 10/255 on blueline and 10/2047
        // on the iPhone 7's Apple DWI backlight, where it reads as fully off —
        // measured 2026-08-09, brightness=1 with the panel still powered.
        Action::Dim => ("brightnessctl", vec!["set", "4%"], "panel-dim"),
        Action::Restore => unreachable!("handled above; the restore carries a value"),
        // The other direction, through the same executor.
        //
        // No lock check and no proximity gate. `Blank`'s guard below exists
        // because a dark panel on an unlocked session is a disclosure hazard
        // that outlives the moment; lighting one is not, and the lock surface
        // is what gets lit when the session is locked. §4's table settles the
        // veto question separately: a power button "can it lie? no — hardware
        // signal", which is why the one wake a pocket can produce by itself
        // (tap-to-wake) is gated at the toggle and this is not.
        Action::Unblank => (DPMS_EXECUTOR, vec!["on"], "panel-on"),
        // One step of output volume. `wpctl` is the same call `hyprland.lua`
        // made; what changed is that the machine makes it, so the volume keys
        // are not a keybinding a compositor swap can delete — which is exactly
        // how they stopped working. `-l 1` caps at 100% on the way up; the
        // down direction needs no cap.
        Action::Volume { up: true } => (
            "wpctl",
            vec!["set-volume", "-l", "1", "@DEFAULT_AUDIO_SINK@", "5%+"],
            "volume-up",
        ),
        Action::Volume { up: false } => (
            "wpctl",
            vec!["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
            "volume-down",
        ),
        // The shell's own indicator, raised by name. `osdVolume` is the
        // `IpcHandler` beside the `osdVolumeTrigger` global shortcut the shell
        // registers — the shortcut is unreachable because viewtop takes these
        // keys as device buttons before a keystroke exists, so the machine
        // knocks on the other door instead.
        Action::VolumeOsd => (
            "qs",
            vec!["-c", "souveraine", "ipc", "call", "osdVolume", "trigger"],
            "volume-osd",
        ),
        // The shell owns the sheet; the machine owns what opens it, and for
        // which window. Same shape as every other row — a named tool with
        // named arguments, one place, in the trail — except that the window is
        // an argument, because a sheet with no subject is the bug this replaced.
        Action::WindowSheet { target } => {
            sheet_target = target.to_string();
            (
                "qs",
                vec![
                    "-c",
                    "souveraine",
                    "ipc",
                    "call",
                    "windowSheet",
                    "open",
                    &sheet_target,
                ],
                "window-sheet-open",
            )
        }
        // The device's own verbs. No argument: unlike the sheet, the subject is
        // the machine itself and there is only one of those.
        Action::PowerMenu => (
            "qs",
            vec!["-c", "souveraine", "ipc", "call", "powerMenu", "open"],
            "power-menu-open",
        ),
        Action::UsbMode(_) => unreachable!("handled above; expressed through usb-signaller"),
        Action::Power(_) => unreachable!("power requests use the result-bearing power executor"),
        Action::Blank => {
            // The invariant, enforced where it cannot be reasoned around: the
            // panel does not go dark unless logind says this session is
            // locked, right now. The machine's own `Locked` is a belief, and a
            // belief that entered by one route (locked_ack) and can only leave
            // by another (logind's falling edge) gets stuck — observed
            // 2026-07-26: machine `locked`, LockedHint `no`, panel off, and
            // double-tap waking straight to an unlocked screen.
            //
            // Checked at the actuator, not at the decision, because every path
            // to a dark panel ends here and only here.
            if !session_is_locked() {
                warn!("refusing to blank: logind says this session is not locked");
                shared.lock().device_state.record_error(
                    "device-state",
                    "panel-off",
                    "refused: session not locked per logind",
                );
                request_session_lock(shared);
                return;
            }
            (DPMS_EXECUTOR, vec!["off"], "panel-off")
        }
        Action::Lock => unreachable!("handled above; the lock is not a shell command"),
        Action::PreferLink(_) => unreachable!("handled above; expressed to NetworkManager"),
        Action::PinTunnelUnderlay(_) => unreachable!("handled above; a host route, not a command"),
    };

    run_executor(shared, program, &args, label, action);
}

const USB_MODED_DESTINATION: &str = "com.meego.usb_moded";
const USB_MODED_PATH: &str = "/com/meego/usb_moded";
const USB_MODED_INTERFACE: &str = "com.meego.usb_moded";

fn usb_moded_call(member: &str, argument: Option<&str>) -> Result<String> {
    let mut command = std::process::Command::new("busctl");
    command.args([
        "--system",
        "call",
        USB_MODED_DESTINATION,
        USB_MODED_PATH,
        USB_MODED_INTERFACE,
        member,
    ]);
    if let Some(argument) = argument {
        command.args(["s", argument]);
    }
    let output = command
        .output()
        .with_context(|| format!("calling usb-signaller {member}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "usb-signaller {member}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("usb-signaller returned non-UTF8")?;
    let start = stdout
        .find('"')
        .context("usb-signaller reply had no string")?
        + 1;
    let end = stdout[start..]
        .find('"')
        .map(|offset| start + offset)
        .context("usb-signaller reply had an unterminated string")?;
    Ok(stdout[start..end].to_owned())
}

fn read_usb_role() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/usb_role").ok()?;
    for entry in entries.flatten() {
        if let Ok(role) = std::fs::read_to_string(entry.path().join("role")) {
            return Some(role.trim().to_owned());
        }
    }
    None
}

fn usb_snapshot() -> Result<serde_json::Value> {
    let raw_mode = usb_moded_call("mode_request", None)?;
    let available = usb_moded_call("get_modes", None)?
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let charger_online = std::fs::read_to_string("/sys/class/power_supply/pmi8998-charger/online")
        .ok()
        .map(|value| value.trim() == "1");
    Ok(serde_json::json!({
        "ok": true,
        "mode": raw_mode.strip_suffix("_mode").unwrap_or(&raw_mode),
        "raw_mode": raw_mode,
        "available_modes": available,
        "data_role": read_usb_role(),
        // Power evidence comes from the charger/upower lane, adjacent to the
        // data-mode mechanism. A true Type-C source/sink role stays unknown
        // until TCPM binds; online is not dressed up as more than it says.
        "charger_online": charger_online,
        "power_role": serde_json::Value::Null,
        // Stable fields now, evidence later. Federation supplies identity;
        // the probe registry supplies ownership. Neither is inferred from a
        // VID/PID and neither absence is reported as "unclaimed".
        "attached_identity": serde_json::Value::Null,
        "probe_owner": serde_json::Value::Null,
    }))
}

fn set_usb_mode(mode: UsbMode) -> Result<()> {
    let _ = usb_moded_call("set_mode", Some(mode.as_usb_moded()))?;
    let after = usb_moded_call("mode_request", None)?;
    if after != mode.as_usb_moded() {
        anyhow::bail!(
            "requested {}, usb-signaller reports {after}",
            mode.as_usb_moded()
        );
    }
    Ok(())
}

/// The cellular v4 default's metric, which is a fixed reference point rather
/// than something to set.
///
/// The modem is v6-only; the v4 default for the carrier is installed by the
/// CLAT daemon on the `clat` tun as `default dev clat scope link metric 2048`,
/// not by the `gsm` connection. So the *only* lever needed is wifi's metric:
/// below 2048 it wins, above 2048 the carrier does. Setting anything on the
/// gsm connection would be adjusting a number nothing reads.
const CLAT_DEFAULT_METRIC: u32 = 2048;

/// Wifi's metric when it should win: NetworkManager's own default, and
/// comfortably under the clat default.
const METRIC_PREFERRED: &str = "600";

/// Wifi's metric when the carrier should win. Above 2048, and clear of NM's
/// +20000 connectivity penalty so a penalised link cannot accidentally land
/// back under the CLAT default. That penalty is what inverted things on
/// 2026-07-31: wifi went to 20600 and the clat default won on a link that
/// could not resolve.
const METRIC_DEMOTED: &str = "30000";

/// Tell NetworkManager which link ordinary traffic should take.
///
/// **Only the wifi connection is touched, and only `wlan0` is reapplied.** An
/// earlier version also modified the `gsm` connection and reapplied `clat`, and
/// that reapply wiped every route the CLAT daemon had installed out-of-band —
/// including the `205.151.11.13/32` pin that is the only path to the MMS proxy.
/// Measured on the phone 2026-08-01: after one reapply the clat device was up
/// with an address and *zero* routes, and the carrier path was gone until
/// `blueline-clat.service` was restarted. `nmcli device reapply` resets a
/// device to its connection's config, so it is destructive to exactly the
/// routes a sidecar daemon owns. Do not reapply a device whose routes NM did
/// not write.
fn apply_link_preference(shared: &Arc<Shared>, bearer: crate::sessiond::bearer::Bearer) {
    use crate::sessiond::bearer::Bearer;
    debug_assert!(METRIC_PREFERRED.parse::<u32>().unwrap() < CLAT_DEFAULT_METRIC);
    debug_assert!(METRIC_DEMOTED.parse::<u32>().unwrap() > CLAT_DEFAULT_METRIC);

    let metric = match bearer {
        Bearer::Wifi => METRIC_PREFERRED,
        Bearer::Cellular => METRIC_DEMOTED,
    };
    let conns = active_connections_of_type("802-11-wireless");
    let mut failed: Vec<String> = Vec::new();
    for conn in &conns {
        let ok = std::process::Command::new("nmcli")
            .args(["connection", "modify", conn, "ipv4.route-metric", metric])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            failed.push(conn.clone());
        }
    }
    // Reapply rather than up/down. Cycling the connection is what made the old
    // gate's corrections into NM events that re-entered it; `reapply` changes
    // the live config without a state transition, so nothing observing NM sees
    // a link flap.
    let reapplied = std::process::Command::new("nmcli")
        .args(["device", "reapply", "wlan0"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let mut d = shared.lock();
    if failed.is_empty() && reapplied {
        // A success is a decision, not an error. Recording it as an error put
        // the daemon's own working actions in the trail's error channel, which
        // makes the one signal a reader scans for useless.
        d.device_state.record_decision(
            "bearer-applied",
            serde_json::json!({
                "bearer": bearer.as_str(),
                "wifi_route_metric": metric,
                "connections": conns,
            }),
            "wifi route metric set and reapplied",
        );
    } else {
        d.device_state.record_error(
            "bearer",
            "prefer-link",
            &format!(
                "nmcli failed (modify: {failed:?}, reapply wlan0: {reapplied}) — \
                 the metric may not match the decision"
            ),
        );
    }
}

/// List active connection names of a given `connection.type`.
fn active_connections_of_type(kind: &str) -> Vec<String> {
    let out = match std::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let (name, ty) = l.rsplit_once(':')?;
            (ty == kind).then(|| name.to_string())
        })
        .collect()
}

/// Pin the WireGuard endpoint's /32 to the chosen underlay.
///
/// Over clat the tunnel was measured sending and never receiving; pinning the
/// endpoint via wlan0 produced a handshake in seconds. The endpoint is read
/// from `wg show`, so a profile change does not need this code changed.
fn pin_tunnel_underlay(shared: &Arc<Shared>, bearer: crate::sessiond::bearer::Bearer) {
    use crate::sessiond::bearer::Bearer;
    let Some(endpoint) = wg_endpoint_ip() else {
        return;
    };
    let dev = match bearer {
        Bearer::Wifi => "wlan0",
        Bearer::Cellular => "clat",
    };
    // `replace` rather than `add`: this runs on every settled change, and an
    // `add` that collides leaves the old pin in place, which is the stale
    // route the whole exercise is trying not to inherit.
    let ok = std::process::Command::new("ip")
        .args(["route", "replace", &format!("{endpoint}/32"), "dev", dev])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let mut d = shared.lock();
    if ok {
        d.device_state.record_decision(
            "bearer-tunnel-pinned",
            serde_json::json!({ "endpoint": endpoint, "dev": dev }),
            "tunnel endpoint pinned to the chosen underlay",
        );
    } else {
        // Expected without CAP_NET_ADMIN. Worth an error rather than silence:
        // the tunnel still works over the default route, but it is no longer
        // pinned to the link the machine chose, and a reader should know the
        // difference between "pinned" and "left to the default".
        d.device_state.record_error(
            "bearer",
            "pin-tunnel",
            &format!("could not pin {endpoint} via {dev}; tunnel follows the default route"),
        );
    }
}

/// The WireGuard peer's endpoint address, as an IP.
///
/// Not from `wg show`: that needs `CAP_NET_ADMIN` and sessiond is a user unit,
/// so it can only ever report "Operation not permitted" here. NetworkManager
/// will hand the peer line to an unprivileged caller, and the endpoint is not
/// a secret — the key on the same line is, which is why only the endpoint is
/// taken.
///
/// The endpoint is configured as a *hostname*, not an address, so it has
/// to be resolved before it can be pinned. That is the one circular step in
/// this file: if DNS is broken — which is precisely the failure this task
/// exists to fix — resolution fails and no pin is written. Skipping is the
/// right failure: a pin to a wrong or stale address is worse than none, and
/// the tunnel still works over whatever the default route is.
fn wg_endpoint_ip() -> Option<String> {
    let out = std::process::Command::new("nmcli")
        .args(["-g", "wireguard.peers", "connection", "show", "--active"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let host = text.lines().find_map(|l| {
        l.split_whitespace()
            .find_map(|f| f.strip_prefix("endpoint="))
            .and_then(|e| {
                e.replace("\\:", ":")
                    .rsplit_once(':')
                    .map(|(h, _)| h.to_string())
            })
    })?;

    // Already an address? Then there is nothing to resolve.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host);
    }
    let out = std::process::Command::new("getent")
        .args(["ahostsv4", &host])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
}

/// Act on the machine's decision that the session must lock before the panel
/// goes dark.
///
/// Two paths, one authority. While a live shell owns steady state it holds the
/// rich lock surface, so the directive goes down the heartbeat connection and
/// the shell executes it — the same relationship §6 of `DEVICE-STATE-MACHINE.md`
/// gives the DPMS executor ("receives commands from the unified state machine,
/// not from independent actors"). With no shell alive there is nothing to
/// delegate to and sessiond raises its own surface.
///
/// A failure here is never swallowed. If the directive cannot be written the
/// daemon takes the lock itself rather than letting the pending blank time out
/// into a dark unlocked screen, and the attempt is recorded either way.
fn request_session_lock(shared: &Arc<Shared>) {
    let directive = serde_json::json!({ "directive": "lock", "why": "blank-requires-lock" });
    let mut line = directive.to_string();
    line.push('\n');

    let mut d = shared.lock();
    if d.shell_alive {
        if let Some(conn) = d.shell_directives.as_mut() {
            match conn.write_all(line.as_bytes()).and_then(|_| conn.flush()) {
                Ok(()) => {
                    info!("[device-state] lock directive sent to the shell");
                    d.device_state.record_decision(
                        "lock-directive",
                        serde_json::json!({ "target": "shell" }),
                        "authority told the shell to raise its lock surface",
                    );
                    return;
                }
                Err(e) => {
                    warn!("could not push lock directive to the shell: {e}");
                    d.device_state
                        .record_error("device-state", "lock-directive", &e.to_string());
                }
            }
        } else {
            warn!("shell is alive but no directive channel — taking the lock here");
            d.device_state.record_error(
                "device-state",
                "lock-directive",
                "shell registered without a directive channel",
            );
        }
    }
    drop(d);

    if !spawn_lock_session(shared) {
        warn!("could not start a lock session for a pending blank");
        shared.lock().device_state.record_error(
            "device-state",
            "lock-for-blank",
            "no lock session could be started",
        );
    }
}

/// Drive the machine to a tier. Kept separate from the lock plumbing so the
/// call sites read as one line each.
fn set_tier(d: &mut Daemon, next: DeviceState, why: &str) {
    if d.device_state.state() == next {
        return;
    }
    if !d.device_state.transition(next) {
        warn!("[device-state] {why}: refused {:?}", next);
    }
}

/// Start a lock session unless one is already running. Returns whether a
/// session is running when we leave (either ours or a pre-existing one).
fn spawn_lock_session(shared: &Arc<Shared>) -> bool {
    let mut d = shared.lock();
    if d.controller.is_some() {
        return true;
    }
    let (controller, rx, wake_read) = match lock::channel() {
        Ok(t) => t,
        Err(e) => {
            warn!("cannot create lock channel: {e:#}");
            return false;
        }
    };
    d.controller = Some(controller);
    d.phase = Phase::Holding;
    // sessiond itself now holds the lock — boot lock, retake after a shell
    // death, or an explicit `lock` request. All three are the locked tier.
    set_tier(&mut d, DeviceState::Locked, "sessiond holds the lock");
    drop(d);

    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let outcome = lock::run(rx, wake_read);
        let mut d = shared.lock();
        d.controller = None;
        match outcome {
            Ok(SessionOutcome::Released) => {
                d.phase = Phase::AwaitingShellLock;
                let gen = d.heartbeat_gen;
                drop(d);
                start_ack_timer(&shared, gen);
                shared.cond.notify_all();
                return;
            }
            Ok(SessionOutcome::Denied) => {
                info!("lock denied/revoked — another locker owns the session");
                d.phase = if d.shell_alive {
                    Phase::Released
                } else {
                    Phase::Idle
                };
            }
            Err(e) => {
                warn!("lock session failed: {e:#}");
                d.phase = if d.shell_alive {
                    Phase::Released
                } else {
                    Phase::Idle
                };
            }
        }
        drop(d);
        shared.cond.notify_all();
    });
    true
}

/// If the shell never acks its lock after a handoff, take the lock back.
/// A retake against a shell that DID lock is refused by the compositor
/// (Denied outcome), so this is safe against a late or lost ack.
fn start_ack_timer(shared: &Arc<Shared>, gen: u64) {
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(LOCKED_ACK_TIMEOUT_SECS));
        let d = shared.lock();
        if d.phase == Phase::AwaitingShellLock && d.heartbeat_gen == gen {
            warn!("shell never confirmed its lock after handoff — retaking");
            drop(d);
            spawn_lock_session(&shared);
        }
    });
}

/// A refusal a caller can branch on.
///
/// `code` is the class, `reason` is this particular one. Doctrine §13: an
/// agent composing verbs needs to know *which kind* of no it got — not now,
/// never, or wrong arguments are three different next moves and a sentence
/// does not distinguish them.
/// The panel's current brightness, or `None` if it cannot be read.
///
/// Read rather than remembered from the last set: the user, the shell, or an
/// agent may have changed it since, and restoring a value the machine assumed
/// is how the panel ends up somewhere nobody asked for.
fn read_brightness() -> Option<u32> {
    let out = std::process::Command::new("brightnessctl")
        .arg("get")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

/// Run one executor and record what it did. Shared by the paths that build
/// their arguments dynamically and the table below.
fn run_executor(shared: &Arc<Shared>, program: &str, args: &[&str], label: &str, action: Action) {
    match std::process::Command::new(program).args(args).status() {
        Ok(status) if status.success() => {
            if action == Action::Blank {
                let follow_up = shared.lock().device_state.set_panel(false);
                for a in follow_up {
                    execute(shared, a);
                }
            }
        }
        Ok(status) => {
            let code = status.code().unwrap_or(-1).to_string();
            warn!("{program} {args:?} exited {code}");
            shared
                .lock()
                .device_state
                .record_error("device-state", label, &code);
        }
        Err(e) => {
            warn!("could not run {program}: {e}");
            shared
                .lock()
                .device_state
                .record_error("device-state", label, &e.to_string());
        }
    }
}

/// Every answer login1 documents for its `Can*` power methods.
///
/// These are deliberately not flattened to a bool. `no` is a permission
/// refusal, `na` is missing mechanism, and an inhibitor is current machine
/// state; callers composing verbs need those to remain different answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerCapability {
    Yes,
    No,
    Challenge,
    NotAvailable,
    Inhibited,
    InhibitorBlocked,
    ChallengeInhibitorBlocked,
}

impl PowerCapability {
    fn parse_busctl(text: &str) -> Result<Self> {
        // busctl prints `s "challenge"`. Take the second field and strip the
        // quotes here rather than in the shell — an earlier version parsed it
        // behind two escaping layers, matched a backslash-quote busctl never
        // emits, and left every capability reading "unknown" on the phone.
        let answer = text
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .trim_matches('"');
        match answer {
            "yes" => Ok(Self::Yes),
            "no" => Ok(Self::No),
            "challenge" => Ok(Self::Challenge),
            "na" => Ok(Self::NotAvailable),
            "inhibited" => Ok(Self::Inhibited),
            "inhibitor-blocked" => Ok(Self::InhibitorBlocked),
            "challenge-inhibitor-blocked" => Ok(Self::ChallengeInhibitorBlocked),
            _ => anyhow::bail!("unrecognised login1 power capability {answer:?}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Challenge => "challenge",
            Self::NotAvailable => "na",
            Self::Inhibited => "inhibited",
            Self::InhibitorBlocked => "inhibitor-blocked",
            Self::ChallengeInhibitorBlocked => "challenge-inhibitor-blocked",
        }
    }

    fn refusal_code(self) -> Option<RefusalCode> {
        match self {
            Self::Yes | Self::Challenge => None,
            Self::No => Some(RefusalCode::NotPermitted),
            Self::NotAvailable => Some(RefusalCode::Unavailable),
            Self::Inhibited | Self::InhibitorBlocked | Self::ChallengeInhibitorBlocked => {
                Some(RefusalCode::RefusedByState)
            }
        }
    }
}

/// Injectable boundary around the two process calls a power request needs.
/// Unit tests use a fake implementation, so no test can invoke a real power
/// mechanism even if it runs on a developer's live session.
trait PowerBackend {
    fn capability(&self, verb: PowerVerb) -> Result<PowerCapability>;
    fn execute(&self, verb: PowerVerb) -> Result<()>;
}

struct SystemPowerBackend;

impl PowerBackend for SystemPowerBackend {
    fn capability(&self, verb: PowerVerb) -> Result<PowerCapability> {
        let capability = verb.logind_capability();
        let output = std::process::Command::new("busctl")
            .args([
                "--system",
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                capability,
            ])
            .output()
            .with_context(|| format!("running busctl {capability}"))?;
        if !output.status.success() {
            anyhow::bail!(
                "busctl {capability} exited {}",
                output.status.code().unwrap_or(-1)
            );
        }
        PowerCapability::parse_busctl(&String::from_utf8_lossy(&output.stdout))
    }

    fn execute(&self, verb: PowerVerb) -> Result<()> {
        let status = std::process::Command::new("systemctl")
            .arg(verb.as_systemctl())
            .status()
            .with_context(|| format!("running systemctl {}", verb.as_systemctl()))?;
        if !status.success() {
            anyhow::bail!(
                "systemctl {} exited {}",
                verb.as_systemctl(),
                status.code().unwrap_or(-1)
            );
        }
        Ok(())
    }
}

fn refuse(code: RefusalCode, reason: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "code": code.as_str(), "reason": reason })
}

fn power_label(verb: PowerVerb) -> &'static str {
    match verb {
        PowerVerb::Poweroff => "power-off",
        PowerVerb::Reboot => "power-reboot",
        PowerVerb::Suspend => "power-suspend",
        PowerVerb::Hibernate => "power-hibernate",
    }
}

/// Admit, gate, execute, and finish one power request.
///
/// The latch is taken before the live capability probe, so two callers cannot
/// both pass policy and race different terminal actions. Every path after
/// admission releases it. A zero exit is reported as `accepted`: it proves the
/// mechanism returned success, not that shutdown or sleep was observed.
fn handle_power_request<B: PowerBackend + ?Sized>(
    shared: &Arc<Shared>,
    verb: PowerVerb,
    backend: &B,
) -> serde_json::Value {
    let actions = {
        let mut d = shared.lock();
        d.device_state
            .request_power(verb, "power requested through the session authority")
    };
    let action_verb = match actions {
        Ok(actions) if actions == vec![Action::Power(verb)] => verb,
        Ok(actions) => {
            let reason = format!("power admission returned unexpected actions: {actions:?}");
            let mut d = shared.lock();
            d.device_state.power_request_finished(verb);
            d.device_state
                .record_error("device-state", power_label(verb), &reason);
            return refuse(RefusalCode::Unavailable, &reason);
        }
        Err(reason) => return refuse(RefusalCode::RefusedByState, &reason),
    };

    let capability = match backend.capability(action_verb) {
        Ok(capability) => capability,
        Err(error) => {
            let reason = format!(
                "could not ask logind about {}: {error:#}",
                action_verb.as_str()
            );
            let mut d = shared.lock();
            d.device_state.power_request_finished(action_verb);
            d.device_state
                .record_error("device-state", "power-capability", &reason);
            return refuse(RefusalCode::Unavailable, &reason);
        }
    };

    if let Some(code) = capability.refusal_code() {
        let reason = format!(
            "logind says {}: {}",
            action_verb.logind_capability(),
            capability.as_str()
        );
        let mut d = shared.lock();
        d.device_state.power_request_finished(action_verb);
        d.device_state.record_decision(
            "power-request-refused",
            serde_json::json!({
                "verb": action_verb.as_str(),
                "capability": capability.as_str(),
            }),
            &reason,
        );
        return refuse(code, &reason);
    }

    match backend.execute(action_verb) {
        Ok(()) => {
            let mut d = shared.lock();
            d.device_state.power_request_finished(action_verb);
            d.device_state.record_decision(
                "power-command-accepted",
                serde_json::json!({ "verb": action_verb.as_str() }),
                "the power mechanism returned success; physical state is not inferred",
            );
            serde_json::json!({
                "ok": true,
                "verb": action_verb.as_str(),
                "status": "accepted",
            })
        }
        Err(error) => {
            let reason = error.to_string();
            warn!("power command {} failed: {reason}", action_verb.as_str());
            let mut d = shared.lock();
            d.device_state.power_request_finished(action_verb);
            d.device_state
                .record_error("device-state", power_label(action_verb), &reason);
            refuse(RefusalCode::Unavailable, &reason)
        }
    }
}

fn respond(writer: &mut UnixStream, value: serde_json::Value) -> std::io::Result<()> {
    writer.write_all(value.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn handle_connection(stream: UnixStream, shared: Arc<Shared>) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    // A read timeout as well as a write one. Without it a client that connects
    // and says nothing parks this thread and its two fds forever, and 691 of
    // them reached two thirds of the fd limit before anyone noticed. Idle
    // connections are cheap to re-establish; leaked ones are not.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(IDLE_CONNECTION_TIMEOUT_SECS)));
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            warn!("could not clone stream: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    // Whether THIS connection is the registered shell heartbeat, and under
    // which generation.
    let mut heartbeat: Option<u64> = None;
    // Whether it asked to be told rather than to ask. Read back off our own
    // response rather than threaded through `handle_request`, because that
    // field IS the wire contract — a caller learns it is subscribed the same
    // way this loop does, and there is no second place for the two to disagree.
    let mut subscribed = false;

    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_REQUEST_BYTES).read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            // A read timeout is not an error for the shell's heartbeat. That
            // connection is SUPPOSED to sit silent — its silence is the
            // liveness signal and its EOF is shell death, so dropping it here
            // would manufacture exactly the false shell-death lock the
            // registration path guards against. Any other silent connection is
            // a leak and gets closed.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // A subscriber is silent for the same reason the heartbeat is:
                // silence is the normal case. A quiet device is exactly when
                // nothing notable is happening, so reaping on silence would
                // disconnect every observer of a healthy phone — §10's mistake
                // with the roles reversed.
                if heartbeat.is_some() || subscribed {
                    continue;
                }
                warn!(
                    "closing idle connection after {IDLE_CONNECTION_TIMEOUT_SECS}s with no request"
                );
                break;
            }
            Err(e) => {
                warn!("read error on control socket: {e}");
                break;
            }
        }
        if line.len() as u64 >= MAX_REQUEST_BYTES {
            let _ = respond(
                &mut writer,
                refuse(RefusalCode::InvalidArgument, "request exceeds size limit"),
            );
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Envelope>(line) {
            Ok(env) => {
                // The intent is set for exactly this request and cleared after,
                // so an entry recorded by the clock thread between two requests
                // is never mislabelled with a chain it had nothing to do with.
                shared
                    .lock()
                    .device_state
                    .forensic
                    .set_intent(env.intent.clone());
                let out = handle_request(env.request, &shared, Some(&writer), &mut heartbeat);
                shared.lock().device_state.forensic.set_intent(None);
                out
            }
            Err(e) => refuse(
                RefusalCode::UnsupportedOp,
                &format!("malformed request: {e}"),
            ),
        };
        if response.get("subscribed").and_then(|v| v.as_bool()) == Some(true) {
            subscribed = true;
        }
        if respond(&mut writer, response).is_err() {
            break;
        }
        // After the answer, never before it: a request that moved the machine
        // has its own reply on the wire before the event describing it, so a
        // caller that is also a subscriber sees cause and then effect.
        fan_out(&shared);
    }

    // Connection gone. If it was the live heartbeat, the shell died —
    // fail closed: take the session lock regardless of prior lock state.
    if let Some(gen) = heartbeat {
        let mut d = shared.lock();
        if d.heartbeat_gen == gen && d.shell_alive {
            d.shell_alive = false;
            d.shell_pid = None;
            d.shell_directives = None;
            let holding = d.controller.is_some();
            drop(d);
            warn!("shell heartbeat lost — locking the session");
            if !holding {
                spawn_lock_session(&shared);
            }
        }
    }
}

/// Push every notable trail entry recorded since the last push to every
/// subscriber, and drop the ones that have gone away.
///
/// Called after anything that can move the machine — a request, a tick, a
/// logind edge — rather than from inside the recording path. Recording happens
/// under the state lock, and writing to a socket under that lock is how this
/// daemon deadlocked itself once already (`4d652bb`): 347 threads parked in
/// __futex_wait behind one guard, accept loop healthy, not a single request
/// answered. A slow reader must never be able to stop the machine deciding.
fn fan_out(shared: &Arc<Shared>) {
    let (entries, mut subs) = {
        let mut d = shared.lock();
        if d.subscribers.is_empty() {
            // Still advance, or the first subscriber to attach inherits every
            // notable edge since boot as a burst of stale news.
            d.last_pushed_seq = d.device_state.forensic.head_seq();
            return;
        }
        let from = d.last_pushed_seq;
        let fresh: Vec<_> = d
            .device_state
            .forensic
            .since(from)
            .into_iter()
            .filter(|e| is_notable(&e.event))
            .collect();
        d.last_pushed_seq = d.device_state.forensic.head_seq();
        if fresh.is_empty() {
            return;
        }
        (fresh, std::mem::take(&mut d.subscribers))
    };

    subs.retain_mut(|sock| {
        entries.iter().all(|entry| {
            let Ok(line) = serde_json::to_string(entry) else {
                return true; // our fault, not the subscriber's — keep it
            };
            sock.write_all(line.as_bytes())
                .and_then(|_| sock.write_all(b"\n"))
                .and_then(|_| sock.flush())
                .is_ok()
        })
    });

    let mut d = shared.lock();
    // Anything that attached while we were writing is still in the list.
    d.subscribers.append(&mut subs);
}

/// SO_PEERCRED pid of the process on the other end, or None if the kernel
/// would not say. None is never treated as a match: the lease is exclusive and
/// "I could not prove who you are" has to fail closed.
///
/// Deliberately the same getsockopt as machined's `peer_cred`, not
/// `UnixStream::peer_cred` — that one is still unstable
/// (`peer_credentials_unix_socket`, rust#42839) and only fails at the aarch64
/// build, after the host test job has already gone green.
fn peer_pid(stream: &UnixStream) -> Option<i32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED fills a ucred struct of the size we pass; the fd is
    // live for the duration of the call because we hold &UnixStream.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc == 0 {
        Some(cred.pid)
    } else {
        None
    }
}

fn handle_request(
    req: Request,
    shared: &Arc<Shared>,
    directives: Option<&UnixStream>,
    heartbeat: &mut Option<u64>,
) -> serde_json::Value {
    match req {
        Request::Describe => {
            serde_json::json!({
                "ok": true,
                "verbs": VERBS.iter().map(|v| serde_json::json!({
                    "op": v.op,
                    "mutates": v.mutates,
                    "summary": v.summary,
                    "refuses": v.refuses.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                    "example": v.example,
                })).collect::<Vec<_>>(),
            })
        }
        Request::Status => {
            let d = shared.lock();
            serde_json::json!({
                "ok": true,
                "phase": d.phase,
                "shell_alive": d.shell_alive,
            })
        }
        Request::ShellReady => {
            let peer = directives.and_then(peer_pid);
            let mut d = shared.lock();

            // Session authority is an exclusive lease. A utility QML process
            // or duplicate shell must never supersede the live supervised
            // shell merely by sending shell_ready: doing so turns that
            // process's EOF into a false shell-death lock. The current
            // authority must disconnect first; its connection handler clears
            // shell_alive before a replacement can register.
            //
            // One exception, and it is half of TASK-48: a quickshell scene
            // reload is the SAME process. The QML tree is rebuilt and opens a
            // second connection while the outgoing tree's socket is still open
            // — same pid, unit still `active`, NRestarts still 0 — so the old
            // connection's EOF is minutes away or never comes at all. Refusing
            // that registration refuses the shell's own successor, and the
            // measured result is a lease that never frees: "shell_ready
            // refused ... retrying (24)" every 5s forever, black screen, no
            // crash to notice. Only a service restart clears it.
            //
            // SO_PEERCRED is the kernel's answer to who is on the other end,
            // and it is the one thing in this exchange that cannot lie — the
            // same move lockhint.rs makes by reading logind instead of
            // mirroring it. Same pid as the holder means reload; supersede.
            // Anything we cannot PROVE is the same process is still refused,
            // so the lease keeps every case it was written for.
            if d.shell_alive {
                let is_reload =
                    matches!((peer, d.shell_pid), (Some(new), Some(held)) if new == held);
                if !is_reload {
                    return refuse(
                        RefusalCode::RefusedByState,
                        "shell authority is already registered",
                    );
                }
                // Supersede in place. `heartbeat_gen` already exists to make a
                // stale connection's EOF harmless — the outgoing handler checks
                // `heartbeat_gen == gen` before declaring the shell dead — so
                // bumping it here is exactly what stops the old socket, closing
                // whenever the old scene is finally collected, from locking a
                // session the new tree is already holding.
                d.heartbeat_gen += 1;
                d.shell_directives = directives.and_then(|s| s.try_clone().ok());
                let gen = d.heartbeat_gen;
                *heartbeat = Some(gen);
                // The daemon is not holding the lock across a reload — the
                // shell is, and misc:allow_session_lock_restore is what lets
                // the new tree adopt it. So `held` is false and there is no
                // handoff to wait on. `must_lock` still reports the session's
                // real state, because a tree that came up without a lock
                // request has to raise one and this is the answer it acts on.
                let locked = crate::sessiond::device_state::is_locked(d.device_state.state());
                info!(
                    "shell re-registered after scene reload (pid {}, gen {gen}, session locked={locked})",
                    peer.unwrap_or(-1)
                );
                return serde_json::json!({
                    "ok": true, "held": false, "reload": true, "must_lock": locked
                });
            }
            d.heartbeat_gen += 1;
            d.shell_alive = true;
            d.shell_pid = peer;
            d.shell_directives = directives.and_then(|s| s.try_clone().ok());
            let gen = d.heartbeat_gen;
            *heartbeat = Some(gen);

            let holding = d.controller.is_some();
            if holding {
                if let Some(ctl) = &d.controller {
                    ctl.send(Msg::Release);
                }
                // Wait until the lock-session thread actually dropped its
                // Wayland connection; only then may the shell lock (the
                // compositor refuses a second locker while ours is alive).
                let deadline = Duration::from_secs(5);
                let (_guard, timed_out) = shared
                    .cond
                    .wait_timeout_while(d, deadline, |d| d.controller.is_some())
                    .unwrap_or_else(|e| e.into_inner());
                if timed_out.timed_out() {
                    return refuse(
                        RefusalCode::Unavailable,
                        "lock session did not release in time",
                    );
                }
                info!("handoff: lock released to shell (gen {gen})");
                serde_json::json!({ "ok": true, "held": true, "must_lock": true })
            } else {
                info!("shell registered (gen {gen}), no lock held");
                if d.phase == Phase::Idle {
                    d.phase = Phase::Released;
                }
                serde_json::json!({ "ok": true, "held": false, "must_lock": false })
            }
        }
        Request::LockedAck => {
            let mut d = shared.lock();
            if heartbeat.is_none() {
                return refuse(
                    RefusalCode::RefusedByState,
                    "locked_ack from a connection that never sent shell_ready",
                );
            }
            d.phase = Phase::Released;
            info!("shell lock confirmed");
            // The compositor has acknowledged a secure lock surface. THIS is
            // the transition nothing ever made: without it the machine stayed
            // in Active for the life of the daemon, so every locked-state rule
            // below it was unreachable.
            set_tier(&mut d, DeviceState::Locked, "shell lock confirmed");
            // The handoff is a deliberate connection drop, but Hyprland
            // announces any dying lock client as a crashed lockscreen.
            // The shell's lock is confirmed, so clear that banner.
            let _ = std::process::Command::new("hyprctl")
                .arg("dismissnotify")
                .spawn();
            serde_json::json!({ "ok": true })
        }
        Request::Lock => {
            let d = shared.lock();
            match d.phase {
                Phase::Holding => serde_json::json!({ "ok": true, "already": true }),
                Phase::Released | Phase::AwaitingShellLock if d.shell_alive => refuse(
                    RefusalCode::RefusedByState,
                    "a live shell owns the session lock; use the shell's lock IPC",
                ),
                _ => {
                    drop(d);
                    if spawn_lock_session(shared) {
                        serde_json::json!({ "ok": true })
                    } else {
                        refuse(RefusalCode::Unavailable, "could not start a lock session")
                    }
                }
            }
        }
        Request::DeviceState => {
            let d = shared.lock();
            let mut resp = d.device_state.to_ipc_json();
            resp["ok"] = serde_json::Value::Bool(true);
            resp["phase"] = serde_json::to_value(d.phase).unwrap_or_default();
            resp["shell_alive"] = serde_json::Value::Bool(d.shell_alive);
            resp["body"] = d.plexus.to_json(Utc::now());
            resp
        }
        Request::SensorInput(input) => {
            let mut d = shared.lock();
            // Charge enters through the same gate as every other source — the
            // reporter reads the supplies, the machine interprets. It carries
            // no presence evidence, so it skips the SensorEvidence lane below.
            if let (SensorSource::Charge, SensorValue::Charge(fields)) =
                (&input.source, &input.value)
            {
                d.device_state.note_charge(fields.clone());
                d.device_state.mark_evidence_seen(input.source);
                // The fuel gauge feeds the plexus's charge gland directly —
                // same number, same cadence, no machine in between. Levels
                // ride.
                if let Some(capacity) = fields.capacity {
                    d.plexus.ingest(
                        SensorReading {
                            source: "charge".into(),
                            amount: capacity as f32 / 100.0,
                        },
                        Utc::now(),
                    );
                }
                info!("[device-state] sensor charge = {:?}", d.device_state.charge);
                return serde_json::json!({ "ok": true });
            }
            let evidence = &mut d.device_state.sensor_evidence;
            match (&input.source, &input.value) {
                (SensorSource::Proximity, SensorValue::Near(v)) => {
                    evidence.proximity_near = *v;
                }
                (SensorSource::Accelerometer, SensorValue::Moving(v)) => {
                    evidence.accel_moving = *v;
                }
                (SensorSource::Light, SensorValue::Light { changing, lux }) => {
                    evidence.light_changing = *changing;
                    evidence.lux = Some(*lux);
                }
                (SensorSource::Touch, SensorValue::Active(v)) => {
                    evidence.touch_active = *v;
                }
                _ => {
                    return refuse(RefusalCode::InvalidArgument, "sensor source/value mismatch");
                }
            }
            let conf = evidence.confidence();
            let promote = evidence.should_promote_idle_faster();
            // Copy the readings out, ending the borrow before the machine is
            // touched again.
            let evidence_copy = SensorEvidence {
                proximity_near: evidence.proximity_near,
                accel_moving: evidence.accel_moving,
                light_changing: evidence.light_changing,
                lux: evidence.lux,
                touch_active: evidence.touch_active,
            };
            // The body hears what the machine sees, each afferent through its
            // own kind: the number for light, the polarity for presence.
            let reading = match &input.value {
                SensorValue::Near(v) => Some(("proximity", if *v { 1.0 } else { 0.0 })),
                SensorValue::Moving(v) => Some(("motion", if *v { 1.0 } else { 0.0 })),
                SensorValue::Light { lux, .. } => Some(("light", lux_to_belief(*lux))),
                SensorValue::Active(v) => Some(("touch", if *v { 1.0 } else { 0.0 })),
                SensorValue::Charge(_) => None,
            };
            if let Some((source, amount)) = reading {
                d.plexus.ingest(
                    SensorReading {
                        source: source.into(),
                        amount,
                    },
                    Utc::now(),
                );
            }
            // Stamp the source as fresh. Without this every reading expires
            // on the next tick, because "never reported" and "reported long
            // ago" are the same thing to the staleness rule.
            d.device_state.mark_evidence_seen(input.source);
            // Let the state machine evaluate Observed transitions.
            d.device_state
                .update_sensors_from(input.source, evidence_copy);
            // The tap-to-wake answer specifically; a power button is never
            // refused. Asked after the update, not before.
            let suppress = d.device_state.suppress_wake(InputTrigger::DoubleTapToWake);
            info!(
                "[device-state] sensor {:?} = {:?} (confidence={:.2}, suppress_dpms={}, promote_idle={})",
                input.source, input.value, conf, suppress, promote
            );
            serde_json::json!({
                "ok": true,
                "confidence": conf,
                "suppress_dpms_wake": suppress,
                "promote_idle_faster": promote,
                "device_state": d.device_state.state(),
            })
        }
        Request::Input { trigger } => {
            let trigger = trigger.unwrap_or(InputTrigger::Unknown);
            let gated = {
                let mut d = shared.lock();
                d.device_state.note_input_gated(trigger)
            };
            // A refusal, in the vocabulary callers already branch on: a gesture
            // producer must not act on an input the authority refused, and
            // `refused_by_state` says not-now rather than never (doctrine §13).
            //
            // Today this is cooperation, not enforcement — an unattested
            // producer can ignore the answer and call the shell anyway. §10 is
            // explicit that the policy layer means something before the
            // enforcement lands; TASK-41 is the enforcement.
            let Some(actions) = gated else {
                // Report the belief and its confidence, not a conclusion. This
                // said "proximity near — treated as a pocket", which was a
                // reading wearing an interpretation's name and was wrong on
                // every deliberate squeeze (see `placement`).
                let p = { shared.lock().device_state.placement() };
                return refuse(
                    RefusalCode::RefusedByState,
                    &format!(
                        "believed {:?} at {:.2} (covered={}, locked={}, lit={})",
                        p.belief, p.confidence, p.covered, p.locked, p.lit
                    ),
                );
            };
            let cancelled = !actions.is_empty();
            for action in actions {
                execute(shared, action);
            }
            serde_json::json!({ "ok": true, "dim_cancelled": cancelled })
        }
        Request::Button { button, edge } => {
            let actions = {
                let mut d = shared.lock();
                d.device_state.button_edge(button, edge)
            };
            for action in actions {
                execute(shared, action);
            }
            serde_json::json!({ "ok": true })
        }
        Request::Gesture {
            fingers,
            gesture,
            target,
        } => {
            let actions = {
                let mut d = shared.lock();
                d.device_state.touch_gesture(fingers, gesture, target)
            };
            let bound = !actions.is_empty();
            for action in actions {
                execute(shared, action);
            }
            // Whether it meant anything, reported. A gesture the machine
            // recognises and has no binding for is a legitimate answer and a
            // different one from a gesture it did not understand — the caller
            // can tell them apart instead of guessing.
            serde_json::json!({ "ok": true, "bound": bound })
        }
        Request::Panel { on } => {
            let actions = {
                let mut d = shared.lock();
                d.device_state.set_panel(on)
            };
            for action in actions {
                execute(shared, action);
            }
            serde_json::json!({ "ok": true, "panel_on": on })
        }
        Request::Screen { on } => {
            let actions = {
                let mut d = shared.lock();
                d.device_state.request_screen(on, "screen verb")
            };
            let acted = !actions.is_empty();
            for action in actions {
                execute(shared, action);
            }
            // `panel_on` is what the machine currently believes, not what was
            // asked: a blank that is waiting on a lock ack has not happened
            // yet, and saying otherwise would be the report lying about the
            // hardware.
            let panel_on = shared.lock().device_state.panel_on();
            serde_json::json!({ "ok": true, "requested": on, "panel_on": panel_on, "acted": acted })
        }
        Request::Usb => match usb_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => refuse(
                RefusalCode::Unavailable,
                &format!("USB posture is unavailable: {error:#}"),
            ),
        },
        Request::SetUsbMode { mode } => {
            let actions = {
                let mut d = shared.lock();
                d.device_state
                    .request_usb_mode(mode, "USB mode requested through the session authority")
            };
            for action in actions {
                execute(shared, action);
            }
            match usb_snapshot() {
                Ok(snapshot) if snapshot["raw_mode"] == mode.as_usb_moded() => snapshot,
                Ok(snapshot) => refuse(
                    RefusalCode::Unavailable,
                    &format!(
                        "USB mode did not settle at {} (reported {})",
                        mode.as_usb_moded(),
                        snapshot["raw_mode"]
                    ),
                ),
                Err(error) => refuse(
                    RefusalCode::Unavailable,
                    &format!("USB mode changed but could not be verified: {error:#}"),
                ),
            }
        }
        Request::Power { verb } => handle_power_request(shared, verb, &SystemPowerBackend),
        Request::Bearer => {
            let d = shared.lock();
            let b = &d.device_state.bearer;
            serde_json::json!({
                "ok": true,
                "wifi": b.wifi.as_str(),
                "cellular": b.cellular.as_str(),
                "tunnel": b.tunnel.as_str(),
                // Reported separately from `tunnel` because it is the claim a
                // reader most needs and the one every other readout gets
                // wrong: a tunnel can be up and not be carrying anything.
                "tunnel_deaf": b.tunnel_is_deaf(),
                "home": b.home,
                "ssid": b.ssid,
                "preferred": b.preferred().map(|x| x.as_str()),
                "applied": d.device_state.bearer_applied_str(),
            })
        }
        Request::GetPolicy => {
            let d = shared.lock();
            let p = &d.device_state.policy;
            serde_json::json!({
                "ok": true,
                "lock_blank_after_secs": p.lock_blank_after.map(|v| v.as_secs()).unwrap_or(0),
                "lock_blank_after_held_secs":
                    p.lock_blank_after_held.map(|v| v.as_secs()).unwrap_or(0),
                "dim_grace_secs": p.dim_grace.as_secs(),
                "evidence_ttl_secs": p.evidence_ttl.as_secs(),
                "dim_warning": p.dim_warning,
                "lock_ack_budget_secs": p.lock_ack_budget.as_secs(),
                "unlocked_blank_after_secs":
                    p.unlocked_blank_after.map(|v| v.as_secs()).unwrap_or(0),
                "home_ssids": p.home_ssids,
                "bearer_settle_secs": p.bearer_settle.as_secs(),
            })
        }
        Request::SetPolicy {
            lock_blank_after_secs,
            lock_blank_after_held_secs,
            dim_grace_secs,
            evidence_ttl_secs,
            dim_warning,
            lock_ack_budget_secs,
            unlocked_blank_after_secs,
            home_ssids,
            bearer_settle_secs,
        } => {
            // 0 means "never" — the desk-clock case, and iOS's Auto-Lock has
            // the same option for the same reason.
            let as_budget = |secs: u64| {
                if secs == 0 {
                    None
                } else {
                    Some(Duration::from_secs(secs))
                }
            };
            let mut d = shared.lock();
            let p = &mut d.device_state.policy;
            if let Some(v) = lock_blank_after_secs {
                p.lock_blank_after = as_budget(v);
            }
            if let Some(v) = lock_blank_after_held_secs {
                p.lock_blank_after_held = as_budget(v);
            }
            if let Some(v) = dim_grace_secs {
                p.dim_grace = Duration::from_secs(v);
            }
            if let Some(v) = evidence_ttl_secs {
                p.evidence_ttl = Duration::from_secs(v);
            }
            if let Some(v) = dim_warning {
                p.dim_warning = v;
            }
            // A zero ack budget would blank before the lock could possibly
            // land, which is precisely the ordering this closes. Refuse it.
            if let Some(v) = lock_ack_budget_secs {
                if v == 0 {
                    drop(d);
                    return refuse(
                        RefusalCode::InvalidArgument,
                        "lock_ack_budget_secs must be at least 1",
                    );
                }
                p.lock_ack_budget = Duration::from_secs(v);
            }
            if let Some(v) = unlocked_blank_after_secs {
                p.unlocked_blank_after = as_budget(v);
            }
            if let Some(v) = home_ssids {
                p.home_ssids = v;
            }
            // Zero is the event-speed controller this whole mechanism exists
            // to not be. Refuse it the same way a zero ack budget is refused.
            if let Some(v) = bearer_settle_secs {
                if v == 0 {
                    drop(d);
                    return refuse(
                        RefusalCode::InvalidArgument,
                        "bearer_settle_secs must be at least 1",
                    );
                }
                p.bearer_settle = Duration::from_secs(v);
            }
            // A dim warning longer than the budget it warns about is not a
            // warning — `dim_at` saturates to zero and the panel dims the
            // instant it goes idle, with no lit period at all. Refuse rather
            // than accept a setting whose behaviour contradicts its label.
            if let Some(budget) = p.lock_blank_after {
                if p.dim_warning && p.dim_grace >= budget {
                    drop(d);
                    return refuse(
                        RefusalCode::InvalidArgument,
                        "dim_grace_secs must be shorter than lock_blank_after_secs",
                    );
                }
            }
            let applied = serde_json::json!({
                "lock_blank_after_secs": p.lock_blank_after.map(|v| v.as_secs()).unwrap_or(0),
                "lock_blank_after_held_secs":
                    p.lock_blank_after_held.map(|v| v.as_secs()).unwrap_or(0),
                "dim_grace_secs": p.dim_grace.as_secs(),
                "evidence_ttl_secs": p.evidence_ttl.as_secs(),
                "dim_warning": p.dim_warning,
                "lock_ack_budget_secs": p.lock_ack_budget.as_secs(),
                "unlocked_blank_after_secs":
                    p.unlocked_blank_after.map(|v| v.as_secs()).unwrap_or(0),
            });
            info!("[device-state] policy set: {applied}");
            d.device_state.record_decision(
                "policy-set",
                applied.clone(),
                "settings changed the policy",
            );

            // Persist, and say so if it fails. A setting that took effect for
            // this run but will not survive a reboot is a success-shaped
            // switch (TASK-19's rule), so the failure goes back to the caller
            // rather than only into the log.
            if let Err(e) = d.device_state.policy.save() {
                warn!("[device-state] policy applied but NOT saved: {e}");
                d.device_state.record_error(
                    "device-state",
                    "policy-save-failed",
                    &format!("the policy is live for this run only: {e}"),
                );
                return serde_json::json!({
                    "ok": false,
                    "reason": format!("applied for this session, but not saved: {e}"),
                    "policy": applied,
                });
            }
            serde_json::json!({ "ok": true, "policy": applied })
        }
        Request::Subscribe => {
            let Some(stream) = directives.and_then(|s| s.try_clone().ok()) else {
                return refuse(
                    RefusalCode::Unavailable,
                    "subscribe needs a connection this daemon can hold open",
                );
            };
            let mut d = shared.lock();
            // Start the cursor at the current head, not at 0. A new subscriber
            // is told what happens NEXT; replaying the buffer would hand it a
            // history it has no way to date and would re-fire hours-old edges
            // as if they were now. `forensic_log` is the verb for the past.
            let from = d.device_state.forensic.head_seq();
            d.subscribers.push(stream);
            let count = d.subscribers.len();
            info!("event subscriber attached (from seq {from}, {count} total)");
            serde_json::json!({ "ok": true, "subscribed": true, "from_seq": from })
        }
        Request::ForensicLog { count } => {
            let d = shared.lock();
            let entries = d.device_state.forensic.recent(count.unwrap_or(50));
            serde_json::json!({
                "ok": true,
                "count": entries.len(),
                "entries": entries,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn idle_shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(Daemon {
                phase: Phase::Idle,
                controller: None,
                heartbeat_gen: 0,
                shell_alive: false,
                shell_pid: None,
                shell_directives: None,
                subscribers: Vec::new(),
                last_pushed_seq: 0,
                device_state: DeviceStateMachine::new(),
                plexus: SomaticPlexus::phone_plexus(Utc::now()),
            }),
            cond: Condvar::new(),
        })
    }

    struct FakePowerBackend {
        capability: std::result::Result<PowerCapability, &'static str>,
        execution: std::result::Result<(), &'static str>,
        capability_calls: Cell<usize>,
        execution_calls: Cell<usize>,
        inspect_latch: Option<(Arc<Shared>, PowerVerb)>,
        saw_latch_before_probe: Cell<bool>,
    }

    impl FakePowerBackend {
        fn new(
            capability: std::result::Result<PowerCapability, &'static str>,
            execution: std::result::Result<(), &'static str>,
        ) -> Self {
            Self {
                capability,
                execution,
                capability_calls: Cell::new(0),
                execution_calls: Cell::new(0),
                inspect_latch: None,
                saw_latch_before_probe: Cell::new(false),
            }
        }

        fn inspecting(mut self, shared: &Arc<Shared>, verb: PowerVerb) -> Self {
            self.inspect_latch = Some((Arc::clone(shared), verb));
            self
        }
    }

    impl PowerBackend for FakePowerBackend {
        fn capability(&self, _verb: PowerVerb) -> Result<PowerCapability> {
            self.capability_calls.set(self.capability_calls.get() + 1);
            if let Some((shared, expected)) = &self.inspect_latch {
                let value = shared.lock().device_state.to_ipc_json()["power_requested"].clone();
                self.saw_latch_before_probe
                    .set(value == serde_json::json!(expected.as_str()));
            }
            self.capability.map_err(|reason| anyhow::anyhow!(reason))
        }

        fn execute(&self, _verb: PowerVerb) -> Result<()> {
            self.execution_calls.set(self.execution_calls.get() + 1);
            self.execution.map_err(|reason| anyhow::anyhow!(reason))
        }
    }

    #[test]
    fn login1_power_capability_parser_names_every_documented_answer() {
        let cases = [
            ("yes", PowerCapability::Yes),
            ("no", PowerCapability::No),
            ("challenge", PowerCapability::Challenge),
            ("na", PowerCapability::NotAvailable),
            ("inhibited", PowerCapability::Inhibited),
            ("inhibitor-blocked", PowerCapability::InhibitorBlocked),
            (
                "challenge-inhibitor-blocked",
                PowerCapability::ChallengeInhibitorBlocked,
            ),
        ];

        for (wire, expected) in cases {
            assert_eq!(
                PowerCapability::parse_busctl(&format!("s \"{wire}\"\n")).unwrap(),
                expected
            );
            assert_eq!(expected.as_str(), wire);
        }
        assert!(PowerCapability::parse_busctl("").is_err());
        assert!(PowerCapability::parse_busctl("s \"future-answer\"").is_err());
    }

    #[test]
    fn yes_and_challenge_execute_after_the_latch_is_taken() {
        for capability in [PowerCapability::Yes, PowerCapability::Challenge] {
            let shared = idle_shared();
            let backend = FakePowerBackend::new(Ok(capability), Ok(()))
                .inspecting(&shared, PowerVerb::Suspend);

            let reply = handle_power_request(&shared, PowerVerb::Suspend, &backend);

            assert_eq!(reply["ok"], true);
            assert_eq!(reply["status"], "accepted");
            assert_eq!(reply["verb"], "suspend");
            assert!(
                reply.get("state").is_none(),
                "request must not claim sleep state"
            );
            assert!(backend.saw_latch_before_probe.get());
            assert_eq!(backend.capability_calls.get(), 1);
            assert_eq!(backend.execution_calls.get(), 1);
            assert_eq!(
                shared.lock().device_state.to_ipc_json()["power_requested"],
                serde_json::Value::Null
            );
        }
    }

    #[test]
    fn capability_refusals_keep_permission_state_and_availability_distinct() {
        let cases = [
            (PowerCapability::No, "not_permitted"),
            (PowerCapability::NotAvailable, "unavailable"),
            (PowerCapability::Inhibited, "refused_by_state"),
            (PowerCapability::InhibitorBlocked, "refused_by_state"),
            (
                PowerCapability::ChallengeInhibitorBlocked,
                "refused_by_state",
            ),
        ];

        for (capability, expected_code) in cases {
            let shared = idle_shared();
            let backend = FakePowerBackend::new(Ok(capability), Ok(()));

            let reply = handle_power_request(&shared, PowerVerb::Poweroff, &backend);

            assert_eq!(reply["ok"], false);
            assert_eq!(reply["code"], expected_code);
            assert_eq!(backend.capability_calls.get(), 1);
            assert_eq!(backend.execution_calls.get(), 0);
            assert_eq!(
                shared.lock().device_state.to_ipc_json()["power_requested"],
                serde_json::Value::Null
            );
        }
    }

    #[test]
    fn probe_and_command_errors_are_honest_and_release_the_latch_for_retry() {
        let shared = idle_shared();
        let probe_failure = FakePowerBackend::new(Err("busctl unavailable"), Ok(()));
        let reply = handle_power_request(&shared, PowerVerb::Reboot, &probe_failure);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["code"], "unavailable");
        assert_eq!(probe_failure.execution_calls.get(), 0);

        let command_failure =
            FakePowerBackend::new(Ok(PowerCapability::Yes), Err("systemctl reboot exited 1"));
        let reply = handle_power_request(&shared, PowerVerb::Reboot, &command_failure);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["code"], "unavailable");
        assert_eq!(command_failure.execution_calls.get(), 1);

        let retry = FakePowerBackend::new(Ok(PowerCapability::Yes), Ok(()));
        let reply = handle_power_request(&shared, PowerVerb::Reboot, &retry);
        assert_eq!(
            reply["ok"], true,
            "both terminal errors must release admission"
        );
        assert_eq!(reply["status"], "accepted");
    }

    #[test]
    fn second_shell_ready_cannot_steal_authority_lease() {
        let shared = idle_shared();
        let mut authority_heartbeat = None;
        let first = handle_request(Request::ShellReady, &shared, None, &mut authority_heartbeat);
        assert_eq!(first["ok"], true);
        assert_eq!(authority_heartbeat, Some(1));

        let mut utility_heartbeat = None;
        let second = handle_request(Request::ShellReady, &shared, None, &mut utility_heartbeat);
        assert_eq!(second["ok"], false);
        assert_eq!(second["reason"], "shell authority is already registered");
        assert_eq!(utility_heartbeat, None);

        let state = shared.lock();
        assert!(state.shell_alive);
        assert_eq!(state.heartbeat_gen, 1);
        assert_eq!(state.phase, Phase::Released);
    }

    /// TASK-48. A scene reload is the same pid on a second connection while
    /// the first is still open. It must be admitted, or the lease never frees
    /// and the phone sits black until the unit is restarted.
    #[test]
    fn scene_reload_from_the_same_pid_supersedes_the_lease() {
        let shared = idle_shared();
        let (outgoing, _o) = UnixStream::pair().unwrap();
        let (incoming, _i) = UnixStream::pair().unwrap();

        let mut first_heartbeat = None;
        let first = handle_request(
            Request::ShellReady,
            &shared,
            Some(&outgoing),
            &mut first_heartbeat,
        );
        assert_eq!(first["ok"], true);
        assert_eq!(first_heartbeat, Some(1));
        assert_eq!(shared.lock().shell_pid, Some(std::process::id() as i32));

        // The outgoing connection has NOT closed — this is the whole point.
        let mut reload_heartbeat = None;
        let reload = handle_request(
            Request::ShellReady,
            &shared,
            Some(&incoming),
            &mut reload_heartbeat,
        );
        assert_eq!(reload["ok"], true, "same-pid reload must not be refused");
        assert_eq!(reload["reload"], true);
        assert_eq!(reload_heartbeat, Some(2));

        let state = shared.lock();
        assert!(state.shell_alive, "the shell never stopped being alive");
        assert_eq!(state.heartbeat_gen, 2);
    }

    /// The generation bump is what disarms the outgoing connection: when the
    /// old scene is finally collected and its socket EOFs, that handler must
    /// not declare the shell dead and lock a session the new tree is holding.
    #[test]
    fn stale_connection_eof_after_a_reload_does_not_kill_the_shell() {
        let shared = idle_shared();
        let (outgoing, _o) = UnixStream::pair().unwrap();
        let (incoming, _i) = UnixStream::pair().unwrap();

        let mut stale_heartbeat = None;
        handle_request(
            Request::ShellReady,
            &shared,
            Some(&outgoing),
            &mut stale_heartbeat,
        );
        let mut live_heartbeat = None;
        handle_request(
            Request::ShellReady,
            &shared,
            Some(&incoming),
            &mut live_heartbeat,
        );

        // Replays the EOF branch of handle_connection for the OLD connection.
        let stale_gen = stale_heartbeat.unwrap();
        let mut d = shared.lock();
        let would_lock = d.heartbeat_gen == stale_gen && d.shell_alive;
        assert!(!would_lock, "a superseded connection's EOF must be inert");
        assert!(d.shell_alive);
        assert_eq!(live_heartbeat, Some(d.heartbeat_gen));
        d.shell_pid = None; // silence the unused-mut lint path
    }

    /// A subscriber is told what happens next, not what already happened.
    /// Replaying the buffer would re-fire hours-old edges as if they were now.
    #[test]
    fn subscribing_starts_at_the_current_head_not_at_zero() {
        let shared = idle_shared();
        let (sock, _peer) = UnixStream::pair().unwrap();

        // Move the machine so the trail is non-empty.
        let mut hb = None;
        handle_request(Request::Panel { on: false }, &shared, None, &mut hb);
        handle_request(Request::Panel { on: true }, &shared, None, &mut hb);
        let head = shared.lock().device_state.forensic.head_seq();

        let mut sub_hb = None;
        let reply = handle_request(Request::Subscribe, &shared, Some(&sock), &mut sub_hb);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["subscribed"], true);
        assert_eq!(reply["from_seq"], head, "must not replay the buffer");
        assert_eq!(shared.lock().subscribers.len(), 1);
    }

    /// The filter is the product. A mind that receives every reading is
    /// reading drivers, which is the thing doctrine forbids and P1 fears.
    #[test]
    fn only_edges_cross_to_a_subscriber() {
        use crate::sessiond::device_state::ForensicEvent;

        assert!(is_notable(&ForensicEvent::Transition {
            from: DeviceState::Active,
            to: DeviceState::Locked,
            legal: true,
        }));
        assert!(
            is_notable(&ForensicEvent::Transition {
                from: DeviceState::Asleep,
                to: DeviceState::Active,
                legal: false,
            }),
            "a REFUSED transition is the more interesting one — the machine \
             wanted to move and its own guard said no"
        );
        assert!(is_notable(&ForensicEvent::Error {
            component: "session-events".into(),
            action: "source-down".into(),
            error: "no reading in 90s".into(),
        }));
        assert!(is_notable(&ForensicEvent::Decision {
            decision: "source-recovered".into(),
            inputs: serde_json::json!({ "source": "proximity" }),
        }));

        // The two that would make it tinnitus.
        assert!(!is_notable(&ForensicEvent::Heartbeat));
        assert!(!is_notable(&ForensicEvent::SensorInput {
            source: crate::sessiond::protocol::SensorSource::Proximity,
            value: crate::sessiond::protocol::SensorValue::Near(true),
            confidence: 0.4,
        }));
    }

    /// A caller we cannot identify never inherits the reload exemption, even
    /// while a real shell holds the lease. Unprovable identity fails closed.
    #[test]
    fn unidentified_caller_is_still_refused_while_a_shell_holds_the_lease() {
        let shared = idle_shared();
        let (shell, _s) = UnixStream::pair().unwrap();

        let mut shell_heartbeat = None;
        handle_request(
            Request::ShellReady,
            &shared,
            Some(&shell),
            &mut shell_heartbeat,
        );

        let mut anon_heartbeat = None;
        let anon = handle_request(Request::ShellReady, &shared, None, &mut anon_heartbeat);
        assert_eq!(anon["ok"], false);
        assert_eq!(anon["reason"], "shell authority is already registered");
        assert_eq!(anon_heartbeat, None);
        assert_eq!(shared.lock().heartbeat_gen, 1);
    }
}
