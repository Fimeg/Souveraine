//! souveraine-sensord — one reporter for every source the machine expects.
//!
//! REPORTER, NOT AN AUTHORITY. It reads sensors and tells sessiond. It decides
//! nothing, actuates nothing, and reads no lock state. Every decision belongs
//! to the device state machine (`DEVICE-STATE-MACHINE.md` §1) — the whole point
//! of that document is that we had seven actors each seeing one facet and
//! acting on it blindly.
//!
//! ## Why this replaces the scripts
//!
//! `blueline-proximity-lock` is a 113-line shell script implementing a contract
//! that is genuinely subtle: heartbeat inside `SOURCE_DOWN_AFTER`, seed the
//! last value from the startup probe banner, report both edges, pass the last
//! reading between a subshell and a background loop through a file in
//! `$XDG_RUNTIME_DIR`. All of that is correct and all of it would have to be
//! copied, verbatim and by hand, into a second script for light and a third for
//! accel — §12 says decide this before writing the second one, not after the
//! third.
//!
//! Copies drift. The `blueline-*` units are exactly the "script-based random
//! stuff" that has to become one orchestrated thing.
//!
//! ## The contract it implements (§10)
//!
//! `monitor-sensor` emits only on CHANGE, so a phone on a table is silent for
//! hours and is byte-for-byte indistinguishable from a dead SLPI. Silence must
//! therefore be made meaningful: every source re-sends its last known reading
//! every `KEEPALIVE`, comfortably inside the machine's 90 s `SOURCE_DOWN_AFTER`.
//! As long as this process and the sensor behind it live, sessiond hears from
//! each source on a schedule. If it stops hearing, something is actually wrong.
//!
//! Repeats are idempotent: the machine re-derives identical evidence and skips
//! the trail write, so a keepalive can never move the state.
//!
//! Health is deliberately NOT read from `net.hadess.SensorProxy`'s
//! `HasProximity`/`HasAccelerometer`. §10 records it answering wrongly in both
//! directions — `true` for hours after the stack died, `false` after remoteproc
//! had already recovered. What can be trusted is our own experience of whether
//! readings arrive.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Re-send interval for each source's last known reading. Must stay well under
/// the machine's `SOURCE_DOWN_AFTER` (90 s) so a healthy source gets several
/// chances to be heard before it is called down.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// How long to wait for sessiond to accept and answer. Short: it answers in
/// microseconds when healthy, and a reporter must never block on the authority.
const SESSIOND_TIMEOUT: Duration = Duration::from_secs(5);

/// Backoff when `monitor-sensor` exits or the proxy has no sensors yet.
const RESPAWN_DELAY: Duration = Duration::from_secs(5);

/// How often the charge source reads the supplies.
///
/// The battery moves on a minutes scale; charging decisions (the resting
/// hysteresis of a topped-up pack) move slower still. A 30 s cadence is an
/// order of magnitude faster than anything charge does, and it lines the
/// reading up with the keepalive so the machine hears from the source on one
/// schedule. The keepalive re-sends the last reading between polls, which is
/// what keeps the source Live in the machine's health table even when the
/// value itself never changes.
const CHARGE_POLL: Duration = Duration::from_secs(30);

/// The last reading we sent per source, in the exact JSON shape sessiond's
/// `SensorInput` expects. Shared with the keepalive thread.
type LastSeen = Arc<Mutex<HashMap<&'static str, serde_json::Value>>>;

/// When each source last went out on the wire, so MIN_REPORT_INTERVAL can be
/// enforced per source rather than globally — a busy light sensor must not
/// delay a proximity edge.
type LastSent = Arc<Mutex<HashMap<&'static str, Instant>>>;

fn socket_path() -> PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc_getuid() }));
    PathBuf::from(runtime).join("souveraine/sessiond.sock")
}

/// Avoid a libc dependency for one call.
unsafe fn libc_getuid() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1000)
}

/// Send one reading. Returns false when sessiond could not be reached — which
/// is information, not a fatal error: the daemon restarts independently of us.
fn report(sock: &PathBuf, source: &str, value: &serde_json::Value) -> bool {
    let req = serde_json::json!({
        "op": "sensor_input",
        "source": source,
        "value": value,
    });

    let attempt = || -> std::io::Result<()> {
        let stream = UnixStream::connect(sock)?;
        stream.set_read_timeout(Some(SESSIOND_TIMEOUT))?;
        stream.set_write_timeout(Some(SESSIOND_TIMEOUT))?;
        let mut w = stream.try_clone()?;
        w.write_all(format!("{req}\n").as_bytes())?;
        w.flush()?;
        // Read the reply so the daemon is not left writing into a closed pipe.
        let mut buf = [0u8; 4096];
        let mut r = stream;
        let _ = r.read(&mut buf)?;
        Ok(())
    };

    match attempt() {
        Ok(()) => true,
        Err(e) => {
            eprintln!("sessiond unreachable; {source} reading dropped: {e}");
            false
        }
    }
}

/// One source's parse rules: the monitor-sensor flag, the change line it emits,
/// and how to turn that line into the value sessiond wants.
struct Source {
    name: &'static str,
    flag: &'static str,
    /// Substring identifying a change line for this source.
    change_marker: &'static str,
    /// Substring identifying this source's startup banner, used to seed the
    /// keepalive before the first change arrives. Without a seed, a phone that
    /// booted onto a table stays silent and a stack that was dead from boot is
    /// invisible.
    banner_marker: &'static str,
    parse: fn(&str, &mut ParserState) -> Option<serde_json::Value>,
}

/// `Proximity value changed: 0` / banner `=== Has proximity sensor (near: 0)`
fn parse_proximity(line: &str, _state: &mut ParserState) -> Option<serde_json::Value> {
    let near = !(line.trim_end().ends_with(": 0") || line.contains("near: 0"));
    Some(serde_json::json!({ "near": near }))
}

/// `Light changed: 240.000000 lux`
///
/// The wire carries `SensorValue::Light { changing, lux }` — the number and
/// the debounced edge together. The edge is what the machine's confidence
/// table believes (§9); the number is what the plexus accumulates as a level
/// and what auto-brightness will act on (TASK-47). Levels ride, edges decide,
/// and nobody upstream throws a measurement away.
fn parse_light(line: &str, state: &mut ParserState) -> Option<serde_json::Value> {
    let lux: f64 = line
        .rsplit(':')
        .next()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let now = Instant::now();
    let changing = match state.last_lux {
        // The startup banner is a level, not a change. Reporting "changing" on
        // it would hand the confidence table a presence signal produced by
        // nothing more than the reporter starting up.
        None => {
            state.last_lux = Some(lux);
            false
        }
        Some(prev) => {
            let delta = lux - prev;
            if delta.abs() <= LIGHT_CHANGE_LUX {
                // Back inside the band: whatever excursion was building did not
                // last, so it never happened. This is the branch that kills the
                // 0.2 pulse.
                state.pending_since = None;
                false
            } else {
                let brightening = delta > 0.0;
                let needed = if brightening {
                    LIGHT_BRIGHTENING_DEBOUNCE
                } else {
                    LIGHT_DARKENING_DEBOUNCE
                };
                // A reversal restarts the clock — an excursion that changes
                // direction is a new excursion, and the new direction may owe a
                // longer wait than the one already served.
                let continuing =
                    state.pending_since.is_some() && state.pending_brightening == brightening;
                if !continuing {
                    state.pending_since = Some(now);
                    state.pending_brightening = brightening;
                    false
                } else if now.duration_since(state.pending_since.unwrap_or(now)) >= needed {
                    // Held past the debounce. Believe it, and move the
                    // reference so the next threshold measures from here.
                    state.last_lux = Some(lux);
                    state.pending_since = None;
                    true
                } else {
                    false
                }
            }
        }
    };
    Some(serde_json::json!({
        "light": { "changing": changing, "lux": lux }
    }))
}

// The accelerometer parser and its motion decay lived here. They went with the
// accel claim itself — see SOURCES for the measurement — because both existed
// only to turn iio-sensor-proxy's *orientation* into a motion edge, and there
// is no orientation stream when the source is not claimed. The inference was
// always weak (a phone carried face-up in a steady hand reported nothing),
// which is why §4 weighted it 0.3. TASK-36's SLPI batching reports motion
// directly and does not need any of this.

/// Per-source memory the parsers need to turn levels into edges.
#[derive(Default)]
struct ParserState {
    /// The lux level we last REPORTED as a change. The threshold measures from
    /// here, not from the last sample.
    last_lux: Option<f64>,
    /// An excursion past the threshold that has not yet lasted long enough to
    /// be believed: when it started and whether it was brightening. Cleared the
    /// moment the level falls back inside the band.
    pending_since: Option<Instant>,
    pending_brightening: bool,
}

/// Shared between the read loop and the keepalive snapshot.
type Parser = Arc<Mutex<ParserState>>;

/// How much lux must move before it counts as "changing", measured against the
/// last value we REPORTED, not the last sample. Comparing consecutive samples
/// is hysteresis in name only: room light drifting 200 lux over a minute moves
/// a fraction of a lux per sample and never trips any threshold, so the machine
/// would be told nothing is changing while the room visibly changes.
const LIGHT_CHANGE_LUX: f64 = 15.0;

/// How long a lux excursion must PERSIST before it counts as a change.
///
/// The level threshold alone is not hysteresis: one sample crossing it emits
/// `changing: true`, the next sample sits inside the new band and emits
/// `changing: false`, and §4's confidence table gets a 0.2 pulse that clears
/// before anything can act on it. Measured on device 2026-07-27: that pulse
/// moved `observed_confidence` 0.50 → 0.70 → 0.50 in 1.2 s, toggling
/// `promote_idle` across the 0.6 band twice.
///
/// These are not invented numbers. They are what Android ships for THIS
/// device — `config_autoBrightnessBrighteningLightDebounce` (2000) and
/// `config_autoBrightnessDarkeningLightDebounce` (4000), read out of
/// `framework-res__lineage_blueline__auto_generated_rro_vendor.apk` in the
/// LineageOS blueline image. The asymmetry is the point: a shadow crossing the
/// sensor must not be believed as fast as a lamp switching on, which is the
/// same shape as `PROXIMITY_NEAR_DEBOUNCE` / `PROXIMITY_FAR_DEBOUNCE`.
const LIGHT_BRIGHTENING_DEBOUNCE: Duration = Duration::from_millis(2000);
const LIGHT_DARKENING_DEBOUNCE: Duration = Duration::from_millis(4000);

// MOTION_WINDOW lived here: how long after an orientation change the device
// still counted as moving. Kept in the history rather than the binary, because
// the finding behind it survives the source being dropped and TASK-36 will need
// it again — measured on device 2026-07-27, boot 0 carried 165 `Moving(true)`
// against 8 `Moving(false)`, all the falses at startup. An edge-derived
// "moving" can never fall back to false on its own, so the keepalive re-sent a
// stuck `true` every 30 s, pinning accel's +0.3 into `observed_confidence` for
// the life of the process and holding the machine at or above the 0.3
// "suppress DPMS wake" band. Whatever reports motion next owes a decay.

/// No source may report more often than this.
///
/// Not a nicety. monitor-sensor emits a line per lux sample and ambient light
/// never holds still, so without a floor the reporter opened ~8 sessiond
/// connections a second, each one re-deriving identical evidence. A sensor feed
/// with no buffering is the same defect §9.5 found in proximity — a raw signal
/// handed to a consumer at whatever rate the hardware happens to produce it.
///
/// 250 ms is the shortest interval at which a change still feels immediate to a
/// person, which is the only consumer that cares about latency here.
const MIN_REPORT_INTERVAL: Duration = Duration::from_millis(250);

const SOURCES: &[Source] = &[
    Source {
        name: "proximity",
        flag: "--proximity",
        change_marker: "Proximity value changed: ",
        banner_marker: "Has proximity",
        parse: parse_proximity,
    },
    Source {
        name: "light",
        flag: "--light",
        change_marker: "Light changed: ",
        banner_marker: "Has ambient light sensor",
        parse: parse_light,
    },
    // The accelerometer is NOT claimed, and that is a power decision.
    //
    // Measured on the phone 2026-08-04, idle, screen on, same session, one flag
    // apart:
    //
    //   monitor-sensor --proximity --light --accel   iio-sensor-proxy  15.3%
    //   monitor-sensor --proximity --light            iio-sensor-proxy   1.1%
    //
    // Claiming the accelerometer makes iio-sensor-proxy poll the IIO device
    // continuously; nothing else here does. So ~14 points of a core were being
    // spent, forever, on the one reading §4 gives the least weight (+0.3) and
    // that §9's own note calls a weak signal — and which the keepalive had to
    // actively decay because monitor-sensor only speaks on orientation change,
    // meaning a stationary phone paid the full poll cost to report nothing.
    //
    // This is the cheap half of TASK-15. It does not say accel is unwanted: it
    // says a subprocess holding a continuous claim is the wrong way to get it.
    // TASK-36's SLPI batching is the right one — the sensor hub already
    // aggregates motion and can report on an interval instead of being polled.
    // Restore this entry only together with that, or the cost comes back.
    //
    // "accelerometer" was the name on the wire, not "accel": SensorSource
    // derives its JSON from snake_case variant names, and the shorter form is
    // the trail/health key only. Noted here so a future restore does not
    // rediscover the refusal.
];

/// Re-send every source's last reading forever. This is the half that makes
/// silence meaningful; see the module docs.
fn spawn_keepalive(sock: PathBuf, last: LastSeen) {
    std::thread::spawn(move || loop {
        std::thread::sleep(KEEPALIVE);
        // The motion decay that used to run here went with the accelerometer
        // claim (see SOURCES). It existed because monitor-sensor only speaks on
        // orientation change, so a stale `moving: true` would otherwise be
        // re-sent forever and pin accel's +0.3 into the confidence sum. With
        // the source unclaimed there is no accelerometer entry to decay, and
        // leaving the decay in would have been a loop deriving a value nothing
        // reports. It comes back with the source, not before.
        let snapshot: Vec<(&'static str, serde_json::Value)> = {
            let guard = match last.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.iter().map(|(k, v)| (*k, v.clone())).collect()
        };
        for (source, value) in snapshot {
            report(&sock, source, &value);
        }
    });
}

fn read_trimmed(dir: &std::path::Path, attr: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(attr))
        .ok()
        .map(|v| v.trim().to_owned())
}

/// Read `/sys/class/power_supply` into the wire shape of `SensorValue::Charge`.
///
/// Classify each node by its own `type` attribute — device names are not the
/// contract (blueline's are `qcom-battery` and `pmi8998-charger`; another
/// body differs). Every read is independent: one missing attribute degrades
/// one field, never the whole probe.
fn probe_charge() -> serde_json::Value {
    let mut fields = serde_json::json!({
        "plugged": serde_json::Value::Null,
        "status": serde_json::Value::Null,
        "charge_type": serde_json::Value::Null,
        "capacity": serde_json::Value::Null,
    });
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return fields;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(kind) = std::fs::read_to_string(dir.join("type")).ok() else {
            continue;
        };
        if kind.trim() == "Battery" {
            if let Some(status) = read_trimmed(&dir, "status") {
                fields["status"] = status.into();
            }
            if let Ok(capacity) = std::fs::read_to_string(dir.join("capacity")) {
                if let Ok(v) = capacity.trim().parse::<u8>() {
                    fields["capacity"] = v.into();
                }
            }
        } else {
            // Any non-battery supply asserting online counts as plugged —
            // USB, mains, wireless all mean the same thing to policy.
            if read_trimmed(&dir, "online").as_deref() == Some("1") {
                fields["plugged"] = serde_json::Value::Bool(true);
            } else if fields["plugged"].is_null() {
                fields["plugged"] = serde_json::Value::Bool(false);
            }
            // The charger carries charge_type; the first real answer wins.
            // "Unknown"/"N/A" are the driver's silence, not a reading.
            if fields["charge_type"].is_null() {
                if let Some(ct) = read_trimmed(&dir, "charge_type") {
                    if ct != "Unknown" && ct != "N/A" {
                        fields["charge_type"] = ct.into();
                    }
                }
            }
        }
    }
    serde_json::json!({ "charge": fields })
}

/// Poll the supplies on a fixed cadence and report on change.
///
/// Charge is not a monitor-sensor source: it has no change lines, so it gets
/// a reader of its own instead of a parser. The last reading lands in the
/// same map the keepalive serves, so sessiond hears from the source on the
/// same schedule as proximity and light and its health table stays honest.
fn spawn_charge_poller(sock: PathBuf, last: LastSeen) {
    std::thread::spawn(move || loop {
        std::thread::sleep(CHARGE_POLL);
        let value = probe_charge();
        let unchanged = {
            let mut guard = match last.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let same = guard.get("charge") == Some(&value);
            guard.insert("charge", value.clone());
            same
        };
        if unchanged {
            continue;
        }
        report(&sock, "charge", &value);
    });
}

fn spawn_monitor() -> std::io::Result<Child> {
    let mut cmd = Command::new("monitor-sensor");
    for s in SOURCES {
        cmd.arg(s.flag);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
}

fn main() {
    let sock = socket_path();
    let last: LastSeen = Arc::new(Mutex::new(HashMap::new()));
    let last_sent: LastSent = Arc::new(Mutex::new(HashMap::new()));
    // Parser memory outlives each monitor-sensor generation: a respawn must not
    // reset the lux reference or re-arm motion, or every SLPI blip would look
    // like a fresh change.
    let parser: Parser = Arc::new(Mutex::new(ParserState::default()));
    spawn_keepalive(sock.clone(), Arc::clone(&last));
    spawn_charge_poller(sock.clone(), Arc::clone(&last));

    // One monitor-sensor for every source, respawned if it dies. Restarting the
    // process is the recovery path for a sensor stack that came back after an
    // SLPI reset; the keepalive keeps reporting the stale-but-last value in the
    // meantime, and the machine's own freshness rules decide what that is worth.
    loop {
        let child = match spawn_monitor() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("could not start monitor-sensor: {e}");
                std::thread::sleep(RESPAWN_DELAY);
                continue;
            }
        };
        let stdout = match child.stdout {
            Some(s) => s,
            None => {
                std::thread::sleep(RESPAWN_DELAY);
                continue;
            }
        };

        for line in BufReader::new(stdout).lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            for s in SOURCES {
                // Seed from the banner so the keepalive has something to send
                // before the first change ever arrives.
                let is_banner = line.contains(s.banner_marker);
                if !is_banner && !line.contains(s.change_marker) {
                    continue;
                }
                let value = {
                    let mut state = match parser.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    match (s.parse)(&line, &mut state) {
                        Some(v) => v,
                        None => continue,
                    }
                };
                // Only report when the value the machine sees actually
                // changes. Lux rides in that value, so a lux drift that does
                // not trip the changing edge still gets reported — that is the
                // level reaching the plexus, and MIN_REPORT_INTERVAL caps the
                // resulting cadence at ~4/second instead of monitor-sensor's
                // ~8. Only identical reports fall out here: a repeated value
                // would otherwise be a socket round trip saying nothing
                // happened, and the keepalive already guarantees sessiond
                // hears from every source inside SOURCE_DOWN_AFTER.
                let unchanged = {
                    let mut guard = match last.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    let same = guard.get(s.name) == Some(&value);
                    guard.insert(s.name, value.clone());
                    same
                };
                if unchanged {
                    break;
                }
                // The buffer. A change still has to wait its turn, so a sensor
                // oscillating around its own threshold cannot flood the
                // machine — the keepalive re-sends the settled value anyway,
                // so nothing is lost by dropping an intermediate edge.
                {
                    let mut sent = match last_sent.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    if let Some(prev) = sent.get(s.name) {
                        if prev.elapsed() < MIN_REPORT_INTERVAL {
                            break;
                        }
                    }
                    sent.insert(s.name, Instant::now());
                }
                // Report both edges. "Far" matters as much as "near": it is
                // what re-enables a wake, and it is what proves the sensor is
                // still alive.
                report(&sock, s.name, &value);
                break;
            }
        }

        eprintln!("monitor-sensor exited; respawning");
        std::thread::sleep(RESPAWN_DELAY);
    }
}
