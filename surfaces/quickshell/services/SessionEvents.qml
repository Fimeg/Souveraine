// Souveraine logind event ingress — the shell's ears for external lock and
// suspend requests.
//
// logind is the system session manager. Other software (a phone's power
// button daemon, a remote SSH session, `loginctl lock-session`) can ask it
// to lock or suspend at any time. Without this file the shell only knows
// about locks it initiates itself; an external lock request would arrive as
// a D-Bus signal that nothing listens to, and the phone would suspend into
// an unlocked session.
//
// Architecture: two `gdbus monitor` processes (one for the manager bus, one
// for the session bus) emit lines that SplitParser splits into individual
// D-Bus signal frames. This is the same Process + SplitParser pattern that
// Souveraine.qml uses for the SSE turn stream — a long-lived child whose
// stdout is line-delimited and parsed in-process.
//
// The sleep delay inhibitor is a `systemd-inhibit --what=sleep --mode=delay
// sleep infinity` process that holds the logind delay inhibitor slot from
// shell startup. Kill it to release (allowing suspend); restart it to
// reacquire (blocking suspend again after wake). This is the same pattern
// Idle.qml uses for hypridle: a background Process whose lifecycle IS the
// inhibitor's lifecycle.
//
// The trust boundary is the same as Session.qml: this surface is local,
// single-user, reachable only over quickshell's IPC socket. The D-Bus
// signals it listens to are system-bus signals that any session member can
// see; this file simply acts on them before logind times out its inhibitor
// delay (typically 5s).
//
// See: session-inhibitors.md, suspend-before-lock.md, capability-tiers.md
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions

Singleton {
    id: root

    // --- Properties --------------------------------------------------------

    // The resolved logind session path (e.g. "/org/freedesktop/login1/session/_3").
    // Empty until the resolver Process completes. The session monitor does not
    // start until this is set, because the D-Bus object path depends on it.
    property string sessionPath: ""

    // Whether the sleep delay inhibitor is currently held. True from shell
    // startup; released only when PrepareForSleep(true) arrives and the
    // session is locked (or about to be). Reacquired on PrepareForSleep(false).
    property bool sleepInhibitorHeld: true

    // --- Signals -----------------------------------------------------------

    // Emitted on every PrepareForSleep signal. `suspending` is true when the
    // system is about to sleep, false when it has woken. Listeners can use
    // this to pause/resume network activity, dim screens, etc.
    signal prepareForSleep(bool suspending)

    // Emitted when an external Lock signal arrives from logind (e.g.
    // `loginctl lock-session` from another process, or a power-button daemon
    // that locks before suspend).
    signal sessionLockRequested()

    // Emitted when the delay inhibitor is released to let suspend proceed.
    // This happens after WlSessionLock.secure confirms the compositor has
    // locked the session, or immediately if the session was already secure.
    signal sleepInhibitorReleased()

    // Emitted when the delay inhibitor is reacquired after wake. The system
    // is no longer suspending, and the shell is blocking suspend again until
    // the next PrepareForSleep(true) cycle.
    signal sleepInhibitorAcquired()

    // --- Session path resolution -------------------------------------------
    // Resolve the graphical session's REAL object path via the user's `Display`
    // session. sessiond resolves exactly this way; see the two measured traps
    // documented on `resolve_session_path` in src/sessiond/lockhint.rs:
    //
    //   Not `auto`. `GetSession("auto")` resolves to the CALLER's session, and
    //   this shell is a systemd user unit under user@1000.service, outside any
    //   session scope — it has no session of its own to name.
    //
    //   Not the `auto` path either. /org/freedesktop/login1/session/auto is an
    //   alias, not an object: PropertiesChanged only fires on the concrete
    //   path, so monitoring the alias subscribes to something never delivered.
    //
    // The user's `Display` session is the graphical one by definition, whoever
    // asks. The previous probe here asked for a `-p ObjectPath` property that
    // loginctl does not have, so it returned empty on every boot and external
    // lock signals were never monitored at all.
    Process {
        id: sessionResolver
        running: true
        command: ["sh", "-c",
            "busctl --system get-property org.freedesktop.login1 "
            + "/org/freedesktop/login1/user/_$(id -u) "
            + "org.freedesktop.login1.User Display 2>/dev/null"]
        stdout: StdioCollector {
            onStreamFinished: {
                const raw = text.trim();
                // `(so) "1" "/org/freedesktop/login1/session/_31"` — the object
                // path is the second quoted field.
                const path = raw.split('"')[3];
                if (path && path.startsWith("/org/freedesktop/login1/session/")) {
                    root.sessionPath = path;
                    console.log("[session-events] session path:", root.sessionPath);
                    sessionLockMonitor.running = true;
                } else if (raw.length > 0) {
                    console.error("[session-events] could not parse Display session from:", raw,
                        "— retrying; external lock signals are NOT being monitored");
                } else {
                    // Do not settle for this. logind may simply not have the
                    // graphical session yet at the moment we asked, and giving
                    // up leaves the shell permanently blind to external lock and
                    // unlock — the exact silent gap that made a fallback-surface
                    // unlock leave a stale lock surface on screen.
                    console.error("[session-events] no Display session for this user yet; "
                        + "retrying — external lock signals are NOT being monitored");
                }
            }
        }
        onExited: (exitCode) => {
            if (exitCode !== 0) {
                console.error("[session-events] session path probe failed (exit " + exitCode
                    + "); retrying — external lock monitoring is NOT active");
            }
        }
    }

    // Keep asking until the graphical session exists. Idle the moment it does:
    // `running` is false once sessionPath is set, so this costs nothing in the
    // normal case and closes the window where a shell that started before
    // logind published the session would never monitor lock signals at all.
    Timer {
        id: sessionResolveRetry
        interval: 5000
        repeat: true
        running: root.sessionPath.length === 0
        onTriggered: sessionResolver.running = true
    }

    // --- Sleep delay inhibitor ---------------------------------------------
    // systemd-inhibit with --mode=delay holds a delay inhibitor on the sleep
    // verb. logind allows a configurable delay (typically 5s) before forcing
    // suspend, giving the shell time to lock the session securely. The
    // process runs `sleep infinity` so it stays alive until we kill it.
    //
    // Kill this process = release the inhibitor = logind may proceed with
    // suspend. Restart it = reacquire = suspend is blocked again.
    //
    // This is deliberately NOT wired through Session.inhibit() yet. Session.qml
    // now supports "sleep" kind, but the delay-mode inhibitor must be held from
    // startup and released reactively on PrepareForSleep — it does not follow
    // the cookie-based request/release pattern that Session.inhibit() exposes.
    // Managing the process here keeps the sleep path working without coupling
    // the reactive suspend flow to the IPC-facing inhibitor API.
    Process {
        id: sleepInhibitor
        running: true
        command: ["systemd-inhibit", "--what=sleep", "--mode=delay",
            "--who=Souveraine Shell",
            "--why=Delay suspend until WlSessionLock.secure confirms session is locked",
            "sleep", "infinity"]
        onExited: (exitCode, exitStatus) => {
            // Unexpected exit while we think the inhibitor is held. This can
            // happen if systemd-inhibit is not installed, or if logind
            // restarted and invalidated the inhibitor fd. Log it; the next
            // PrepareForSleep(true) will find no inhibitor and suspend will
            // proceed immediately — which is the safe degradation: the phone
            // sleeps instead of draining its battery blocking a suspend that
            // will never complete.
            if (root.sleepInhibitorHeld) {
                console.log("[session-events] sleep inhibitor exited unexpectedly (exit "
                    + exitCode + "); suspend will no longer be delayed");
                root.sleepInhibitorHeld = false;
            }
        }
    }

    function releaseSleepInhibitor() {
        if (!root.sleepInhibitorHeld) return;
        // Setting running=false sends SIGTERM to the process. `sleep infinity`
        // exits cleanly on SIGTERM, which releases the systemd-inhibit fd.
        sleepInhibitor.running = false;
        root.sleepInhibitorHeld = false;
        root.sleepInhibitorReleased();
        console.log("[session-events] sleep inhibitor released; suspend may proceed");
    }

    function reacquireSleepInhibitor() {
        if (root.sleepInhibitorHeld) return;
        sleepInhibitor.running = true;
        root.sleepInhibitorHeld = true;
        root.sleepInhibitorAcquired();
        console.log("[session-events] sleep inhibitor reacquired; suspend blocked");
    }

    // --- D-Bus monitor: PrepareForSleep ------------------------------------
    // Watches org.freedesktop.login1.Manager for the PrepareForSleep signal.
    // This signal carries a boolean: true = suspending, false = waking.
    //
    // gdbus monitor output format:
    //   /org/freedesktop/login1.Manager: org.freedesktop.login1.Manager.PrepareForSleep (true,)
    //
    // We parse the boolean from the signal arguments. The object path line
    // is also emitted by gdbus but SplitParser gives us one line at a time,
    // so the signal name line is what carries the argument.
    Process {
        id: sleepMonitor
        running: true
        command: ["gdbus", "monitor", "--system",
            "--dest", "org.freedesktop.login1",
            "--object-path", "/org/freedesktop/login1"]
        stdout: SplitParser {
            onRead: data => {
                if (data.length === 0) return;
                if (!data.includes("PrepareForSleep")) return;
                // Extract the boolean from the parenthesized argument list.
                // The format is "(true,)" or "(false,)" — the trailing comma
                // is gdbus's tuple representation of a single-argument signal.
                const suspending = data.includes("(true,");
                root.prepareForSleep(suspending);
                console.log("[session-events] PrepareForSleep:", suspending ? "suspending" : "waking");

                if (suspending) {
                    root.onSuspendRequested();
                } else {
                    root.onWoken();
                }
            }
        }
        onExited: (exitCode) => {
            console.log("[session-events] sleep monitor exited (exit " + exitCode
                + "); PrepareForSleep signals will not be received");
        }
    }

    // --- D-Bus monitor: Session Lock ---------------------------------------
    // Watches the current session's object path for the Lock signal. This
    // signal has no arguments — it is a notification that something asked
    // logind to lock this session.
    //
    // The monitor does not start until sessionPath is resolved, because the
    // object path is session-specific. If resolution fails, this monitor
    // stays disabled and only shell-initiated locks work.
    Process {
        id: sessionLockMonitor
        command: root.sessionPath.length > 0
            ? ["gdbus", "monitor", "--system",
                "--dest", "org.freedesktop.login1",
                "--object-path", root.sessionPath]
            : ["false"] // never runs; guarded by sessionPath check
        stdout: SplitParser {
            onRead: data => {
                if (data.length === 0) return;
                if (!data.includes("org.freedesktop.login1.Session.Lock")) return;
                // Avoid reacting to our own lock-session notification.
                // Session.lock() calls loginctl lock-session, which emits
                // this same signal back at us. If the session is already
                // locked (or lock was requested), this is an echo, not a
                // new external request.
                if (GlobalStates.screenLocked) return;
                console.log("[session-events] external Lock signal received");
                root.sessionLockRequested();
                Session.lock();
            }
        }
        onExited: (exitCode) => {
            if (exitCode !== 0) {
                console.log("[session-events] session lock monitor exited (exit " + exitCode
                    + "); external lock signals will not be received");
            }
        }
    }

    // --- Suspend/lock coordination -----------------------------------------
    // The core logic: when logind tells us the system is about to suspend,
    // we must ensure the session is locked BEFORE we release the delay
    // inhibitor. If the session is already secure (WlSessionLock confirmed),
    // release immediately. If not, request a Wayland lock and wait for
    // LockScreen.qml to confirm secure — then release.
    //
    // This is the suspend-before-lock protocol documented in
    // suspend-before-lock.md: PrepareForSleep(true) -> request lock ->
    // wait for screenLockSecure -> release inhibitor -> logind proceeds.

    // Whether we requested a lock as part of the suspend path. Used to
    // distinguish "we locked for suspend" from "we woke up and should not
    // unlock". The lock persists across suspend/resume; only the human
    // unlocks via the credential gate.
    property bool _lockForSuspend: false

    function onSuspendRequested() {
        if (GlobalStates.screenLockSecure) {
            // Session is already secure — the compositor has confirmed the
            // lock surface. Release the inhibitor immediately so logind can
            // proceed with suspend within its delay window.
            root.releaseSleepInhibitor();
        } else {
            // Session is not yet secure. Request a Wayland lock; the
            // Connections block below watches for screenLockSecure and will
            // release the inhibitor when it arrives. We set the flag so the
            // Connections handler knows this lock came from the suspend path.
            root._lockForSuspend = true;
            if (!GlobalStates.screenLocked) {
                Session.lock();
            }
            // If screenLocked is true but screenLockSecure is not yet true,
            // the lock has been requested but the compositor has not acked.
            // The Connections handler will catch the ack.
            console.log("[session-events] lock requested for suspend; "
                + "waiting for WlSessionLock.secure");
        }
    }

    function onWoken() {
        // The system has woken from suspend. Reacquire the delay inhibitor
        // so the next PrepareForSleep(true) is again blocked until the
        // session is secure. The lock persists — we do not unlock on wake.
        // Only the human credential gate (PIN pad) unlocks.
        root._lockForSuspend = false;
        root.reacquireSleepInhibitor();
    }

    // Watch for the compositor confirming the lock surface. This is the
    // signal from LockScreen.qml's WlSessionLock.onSecureChanged handler,
    // mediated through GlobalStates.screenLockSecure. If we are on the
    // suspend path (lock requested for suspend, inhibitor still held), the
    // secure confirmation means the session is now safe to suspend.
    Connections {
        target: GlobalStates
        function onScreenLockSecureChanged() {
            if (!GlobalStates.screenLockSecure) return;
            if (!root._lockForSuspend) return;
            if (!root.sleepInhibitorHeld) return;
            // WlSessionLock.secure confirms the compositor has replaced the
            // session surfaces with the lock surface. The session is now
            // secure; release the delay inhibitor so logind can suspend.
            root._lockForSuspend = false;
            root.releaseSleepInhibitor();
        }
    }

    Timer {
        id: initLogTimer
        interval: 0
        repeat: false
        running: true
        onTriggered: console.log("[session-events] initialized; sleep inhibitor held="
            + root.sleepInhibitorHeld)
    }
}
