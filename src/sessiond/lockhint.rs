//! logind's `LockedHint` — the machine's lock truth.
//!
//! The state machine used to decide for itself whether the session was locked,
//! from its own transition history: `locked_ack` from the shell put it in
//! `Locked` and nothing ever took it out. There was no unlock ingress at all
//! except sessiond's own fallback PIN surface, which is the crash path, not
//! the one anybody uses daily. So after the first unlock the machine believed
//! the session was locked forever.
//!
//! Measured on hardware 2026-07-26, 40 minutes after a cold boot:
//! `device_state: locked, locked: true, panel_on: false` while
//! `loginctl show-session 1 -p LockedHint` said `LockedHint=no`. The screen was
//! dark on an unlocked session, and both the blank-budget rule and the
//! proximity rule had been firing against `is_locked(state) == true` the whole
//! time — which is why proximity was blanking an unlocked phone and why a
//! blank never asked for a lock first (it thought one was already held).
//!
//! Doctrine §4 named this exactly: "`locked` comes from `LockedHint` /
//! `Lock()` / `Unlock()` — never a hand-tracked bool. A shadow copy can
//! disagree with logind, and anything asking logind directly sees something
//! different than the shell shows." The machine was keeping the shadow copy the
//! doctrine forbids, and it disagreed precisely as predicted.
//!
//! So we read the authority instead. Whoever locks or unlocks — the shell's
//! rich surface, sessiond's fallback surface, `loginctl lock-session`, a future
//! greeter — logind is told, so we are told. The machine stops guessing.
//!
//! Transport is `gdbus monitor`, the pattern doctrine §8 names for the sibling
//! problem ("a persistent `gdbus monitor --system` process, parsed
//! line-by-line — **not** polling. Polling latency eats the delay budget
//! directly"). sessiond has no D-Bus crate: `zbus` is behind the `secrets`
//! feature and drags in a tokio runtime this thread-based daemon does not want.

use std::io::{BufRead, BufReader};
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tracing::{info, warn};

/// Resolve the graphical session's REAL object path.
///
/// Two traps here, both measured.
///
/// **Not `auto`.** `GetSession("auto")` resolves to *the caller's* session, and
/// sessiond is a systemd user unit — it runs under `user@1000.service`, outside
/// any session scope. Asked over ssh it answers the ssh session
/// (`session/_310` on 2026-07-26 while the phone's graphical session was `_31`);
/// asked from a session-less unit it is at best a fallback. The user's
/// `Display` session is the graphical one by definition, whoever asks.
///
/// **Not the `auto` path either.** `/org/freedesktop/login1/session/auto` is an
/// alias, not an object: `PropertiesChanged` only ever fires on the concrete
/// path, so monitoring the alias yields a subscription that is silently never
/// delivered. That is the failure the shell already hit
/// (`[session-events] no session path returned`).
pub fn resolve_session_path() -> Result<String> {
    let uid = std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .context("reading our own uid")?;

    // The seat's active session first — the one on the glass.
    //
    // `User.Display` is not that, and the difference is not academic. Measured
    // 2026-08-02: an ssh login made `Display` resolve to a *remote* session
    // with no seat, while the compositor sat on `seat0`/tty1. The hint was then
    // written to one session and read from another, `LockedHint` never moved,
    // and every blank took `request_blank`'s fail-open branch and darkened an
    // unlocked panel. Display also went stale the moment that ssh session
    // ended, so the resolver's answer depended on who happened to be logged in.
    //
    // A phone has one seat and the lock is about the panel on it, so ask the
    // seat. `Session.qml` resolves the same way for the write side; the two
    // must agree or this is a hint nobody reads.
    //
    // `(so) "76" "/org/freedesktop/login1/session/_376"` — the object path is
    // the second quoted field, same shape as `Display` below.
    if let Ok(out) = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/seat/seat0",
            "org.freedesktop.login1.Seat",
            "ActiveSession",
        ])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(p) = text
                .split('"')
                .nth(3)
                .filter(|p| p.starts_with("/org/freedesktop/login1/session/"))
            {
                return Ok(p.to_string());
            }
        }
    }

    // `(so) "1" "/org/freedesktop/login1/session/_31"` — the object path is the
    // second quoted field.
    let user_obj = format!("/org/freedesktop/login1/user/_{uid}");
    if let Ok(out) = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.login1",
            &user_obj,
            "org.freedesktop.login1.User",
            "Display",
        ])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(p) = text
                .split('"')
                .nth(3)
                .filter(|p| p.starts_with("/org/freedesktop/login1/session/"))
            {
                return Ok(p.to_string());
            }
        }
    }

    warn!("[lock-hint] no Display session for uid {uid}; falling back to GetSession(auto), which may resolve to the wrong session");
    let out = Command::new("busctl")
        .args([
            "--system",
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "GetSession",
            "s",
            "auto",
        ])
        .output()
        .context("busctl GetSession")?;

    if !out.status.success() {
        bail!(
            "busctl GetSession failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // `o "/org/freedesktop/login1/session/_31"`
    let text = String::from_utf8_lossy(&out.stdout);
    let path = text
        .split('"')
        .nth(1)
        .map(str::to_string)
        .filter(|p| p.starts_with("/org/freedesktop/login1/session/"))
        .context("could not parse a session path out of busctl output")?;
    Ok(path)
}

/// One-shot read, so the machine starts from the truth rather than from a
/// default that happens to be wrong until the first signal arrives.
pub fn read_locked_hint(path: &str) -> Result<bool> {
    let out = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
            "LockedHint",
        ])
        .output()
        .context("busctl get-property LockedHint")?;

    if !out.status.success() {
        bail!(
            "busctl get-property failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // `b true`
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.split_whitespace().nth(1) == Some("true"))
}

/// Pull the LockedHint value out of a `gdbus monitor` PropertiesChanged line.
///
/// The line looks like:
/// ```text
/// /org/freedesktop/login1/session/_31: org.freedesktop.DBus.Properties.PropertiesChanged
///   ('org.freedesktop.login1.Session', {'LockedHint': <true>}, @as [])
/// ```
/// Returns `None` for every other property change on the session, of which
/// there are many (`IdleHint`, `Active`, `IdleSinceHint`…).
pub(crate) fn parse_locked_hint(line: &str) -> Option<bool> {
    let idx = line.find("'LockedHint':")?;
    let rest = &line[idx..];
    let open = rest.find('<')?;
    let close = rest[open..].find('>')? + open;
    match rest[open + 1..close].trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Say so, loudly and once, when this session can never hold a lock hint.
///
/// logind refuses `SetLockedHint` for any session that is not `Class=user` —
/// "Session does not support lock screen". When that happens `LockedHint` is
/// pinned at `no` forever, so `is_locked()` is permanently false, every
/// `request_blank()` times out its ack budget, and the panel goes dark on a
/// session nobody could confirm was locked. The machine keeps working and keeps
/// blanking; only the security half is gone.
///
/// That state lasted weeks undetected on blueline (greetd's `default_session`
/// runs as the greeter, so any `systemctl restart greetd` produced it), and the
/// only trace was one `blank-without-lock` per blank — an entry that reads like
/// a timing problem rather than a structural one. This turns it into a single
/// line naming the cause.
///
/// A warning and not a refusal to start: a phone that will not boot because its
/// session class is wrong is worse than one that boots and says the lock hint is
/// unavailable.
fn warn_if_session_cannot_lock(path: &str) {
    let Ok(out) = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
            "Class",
        ])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    // `s "user"`
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(class) = text.split('"').nth(1) else {
        return;
    };
    if class == "user" {
        return;
    }
    warn!(
        "[lock-hint] session {path} is Class={class}, not user — logind will refuse SetLockedHint \
         for it, so LockedHint can never become true, `locked` stays false, and EVERY blank will \
         go out on a session that could not be confirmed locked (blank-without-lock). On greetd \
         this means the desktop is running in the greeter slot; see LOCK-DPMS-LESSONS §1."
    );
}

/// Watch `LockedHint` forever, reporting every change.
///
/// The callback fires once at startup with the current value, then on each
/// transition. It is NOT called for repeats — the machine only needs edges.
///
/// A watcher that dies is loud and fatal to the guarantee, not silently
/// retried into a state where the machine is guessing again: if the monitor
/// exits we say so at `warn` and stop, leaving `is_locked` pinned at the last
/// known truth rather than drifting.
pub fn spawn(on_change: Arc<dyn Fn(bool) + Send + Sync>) {
    std::thread::spawn(move || {
        let path = match resolve_session_path() {
            Ok(p) => p,
            Err(e) => {
                warn!("[lock-hint] could not resolve the session path: {e:#} — the device state machine will fall back to its own lock tracking, which drifts");
                return;
            }
        };
        info!("[lock-hint] watching {path}");
        warn_if_session_cannot_lock(&path);

        let mut last = match read_locked_hint(&path) {
            Ok(v) => {
                info!("[lock-hint] initial LockedHint={v}");
                on_change(v);
                v
            }
            Err(e) => {
                warn!("[lock-hint] initial read failed: {e:#}");
                false
            }
        };

        let mut child = match Command::new("gdbus")
            .args([
                "monitor",
                "--system",
                "--dest",
                "org.freedesktop.login1",
                "--object-path",
                &path,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "[lock-hint] could not start gdbus monitor: {e} — lock truth will not update"
                );
                return;
            }
        };

        let Some(stdout) = child.stdout.take() else {
            warn!("[lock-hint] gdbus monitor produced no stdout");
            return;
        };

        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(v) = parse_locked_hint(&line) {
                if v != last {
                    last = v;
                    info!("[lock-hint] LockedHint={v}");
                    on_change(v);
                }
            }
        }

        warn!("[lock-hint] gdbus monitor exited — lock truth is now STALE; the machine will keep its last known value");
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_locked_hint_change() {
        let line = "/org/freedesktop/login1/session/_31: org.freedesktop.DBus.Properties.PropertiesChanged ('org.freedesktop.login1.Session', {'LockedHint': <true>}, @as [])";
        assert_eq!(parse_locked_hint(line), Some(true));
    }

    #[test]
    fn parses_an_unlock() {
        let line = "/org/freedesktop/login1/session/_31: org.freedesktop.DBus.Properties.PropertiesChanged ('org.freedesktop.login1.Session', {'LockedHint': <false>}, @as [])";
        assert_eq!(parse_locked_hint(line), Some(false));
    }

    #[test]
    fn ignores_other_properties() {
        // IdleHint churns constantly; treating it as a lock change would make
        // the machine flap between Active and Locked on every idle edge.
        let line = "/org/freedesktop/login1/session/_31: org.freedesktop.DBus.Properties.PropertiesChanged ('org.freedesktop.login1.Session', {'IdleHint': <true>}, @as [])";
        assert_eq!(parse_locked_hint(line), None);
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert_eq!(parse_locked_hint("something else entirely"), None);
    }
}
