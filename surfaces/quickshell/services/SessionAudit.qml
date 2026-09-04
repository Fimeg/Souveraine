// Session audit trail — tamper-evident log of session state transitions.
pragma Singleton
//
// Every lock/unlock, sleep/wake, auth event, and break-glass grant is
// appended to an append-only log file with a hash chain. Each entry
// includes the hash of the previous entry, making the log tamper-evident:
// altering any past entry invalidates every subsequent hash.
//
// The log is JSONL (one JSON object per line). The hash chain uses
// SHA-256 via the `sha256sum` binary. The tamper-evidence is advisory —
// it detects casual modification, not a determined attacker with access
// to the file.
//
// Log location: ~/.local/share/souveraine/session-audit.jsonl
//
// Entry format:
// {
//   "seq": 42,
//   "prev": "sha256-of-previous-entry",
//   "ts": 1234567890,
//   "event": "lock-requested",
//   "data": { ... },
//   "hash": "sha256-of-this-entry-without-hash-field"
// }
//
// The hash is computed over the JSON string of the entry WITHOUT the hash
// field. This is: sha256(JSON.stringify({seq, prev, ts, event, data})).
// Computed by piping the entry JSON through sha256sum in the same shell
// command that appends it to the log, so the hash and write are atomic.
//
// Integration points:
//   - GlobalStates: lock/unlock transitions
//   - IdleCoordinator: state machine transitions
//   - SessionEvents: PrepareForSleep, session Lock signal
//   - StepUpAuth: auth succeeded/failed, break-glass issued/consumed
//   - Session: action failures, verb refusals
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs.services
import qs.modules.common

Singleton {
    id: root

    // The audit log file path. Uses the standard XDG data directory.
    readonly property string auditPath: Quickshell.env("HOME")
        + "/.local/share/souveraine/session-audit.jsonl"

    // The hash of the last entry in the chain. Empty for the first entry
    // (genesis block). Updated after every append.
    property string lastHash: ""

    // Sequence number. Incremented with every entry. Combined with the
    // hash chain, this detects gaps (skipped entries) as well as mutations.
    property int nextSeq: 0

    // Ensure the parent directory exists on startup.
    Process {
        id: dirCreator
        running: true
        command: ["mkdir", "-p",
            Quickshell.env("HOME") + "/.local/share/souveraine"]
    }

    // --- Append logic -------------------------------------------------------
    // Computes SHA-256 and appends in one shell command so the hash and
    // write are atomic. The entry JSON (without hash field) is piped to
    // sha256sum; the final entry (with hash) is appended to the log, and
    // the hex hash is emitted to stdout so the StdioCollector can update
    // lastHash for the next entry in the chain.
    function append(event, data) {
        const entry = {
            seq: root.nextSeq,
            prev: root.lastHash,
            ts: Math.floor(Date.now() / 1000),
            event: event,
            data: data || {}
        };
        const hashInput = JSON.stringify(entry);
        const escaped = _escape(hashInput);

        // Shell: compute sha256, inject hash into the JSON via sed
        // (available on all target devices), append to log, echo hash
        // to stdout. The sed command inserts "hash":"<hex>" before the
        // LAST closing brace (anchored with $), which is the JSON root.
        const cmd =
            `h=$(printf '%s' '${escaped}' | sha256sum | cut -d' ' -f1); ` +
            `printf '%s\\n' '${escaped}' | sed "s/}$/,\\"hash\\":\\"$h\\"}/" >> ${root.auditPath}; ` +
            `echo "$h"`;
        appendProcHash.command = ["sh", "-c", cmd];
        appendProcHash.running = true;

        root.nextSeq++;
        console.log("[audit] " + event + " (seq=" + entry.seq + ")");
    }

    // Escape single quotes for shell embedding.
    function _escape(s) {
        return s.replace(/'/g, "'\\''");
    }

    Process {
        id: appendProcHash
        stdout: StdioCollector {
            onStreamFinished: {
                const hash = text.trim();
                if (hash.length === 64) {
                    root.lastHash = hash;
                } else {
                    console.log("[audit] unexpected sha256 output: " + text);
                }
            }
        }
        onExited: (exitCode) => {
            if (exitCode !== 0) {
                console.log("[audit] append failed (exit " + exitCode + ")");
            }
        }
    }

    // --- Load existing chain on startup -------------------------------------
    // Read the last entry from the log to restore the hash chain state.
    // If the log doesn't exist or is empty, start fresh.
    Process {
        id: chainLoader
        running: true
        command: ["sh", "-c",
            `if [ -f ${root.auditPath} ]; then tail -1 ${root.auditPath}; else echo ""; fi`]
        stdout: StdioCollector {
            onStreamFinished: {
                const line = text.trim();
                if (line.length === 0) {
                    root.lastHash = "";
                    root.nextSeq = 0;
                    root.append("audit-started", { reason: "new chain" });
                    return;
                }
                try {
                    const last = JSON.parse(line);
                    root.lastHash = last.hash || "";
                    root.nextSeq = (last.seq || 0) + 1;
                    root.append("audit-started", { reason: "chain resumed" });
                } catch (e) {
                    console.log("[audit] could not parse last entry: " + e);
                    root.lastHash = "";
                    root.nextSeq = 0;
                    root.append("audit-started", { reason: "chain reset (parse error)" });
                }
            }
        }
    }

    // --- Event wiring -------------------------------------------------------
    // Lock/unlock transitions.
    Connections {
        target: GlobalStates
        function onScreenLockedChanged() {
            root.append(GlobalStates.screenLocked
                ? "lock-requested" : "lock-cleared",
                { screenLocked: GlobalStates.screenLocked });
        }
        function onScreenLockSecureChanged() {
            root.append(GlobalStates.screenLockSecure
                ? "lock-secure" : "lock-insecure",
                { screenLockSecure: GlobalStates.screenLockSecure });
        }
    }

    // Idle state transitions.
    Connections {
        target: IdleCoordinator
        function onStateTransitioned(state) {
            const names = ["active", "dimmed", "lock-requested",
                "lock-secure", "suspending", "asleep", "waking"];
            root.append("idle-transition", {
                state: names[state] || String(state)
            });
        }
    }

    // Sleep/wake events.
    Connections {
        target: typeof SessionEvents !== "undefined" ? SessionEvents : null
        function onPrepareForSleep(suspending) {
            root.append(suspending ? "sleep-requested" : "wake",
                { sleepInhibitorHeld: SessionEvents.sleepInhibitorHeld });
        }
        function onSleepInhibitorReleased() {
            root.append("sleep-inhibitor-released", {});
        }
        function onSleepInhibitorAcquired() {
            root.append("sleep-inhibitor-acquired", {});
        }
        function onSessionLockRequested() {
            root.append("external-lock-signal", {});
        }
    }

    // Auth events.
    Connections {
        target: typeof StepUpAuth !== "undefined" ? StepUpAuth : null
        function onAuthSucceeded(family) {
            root.append("auth-succeeded", { family: family });
        }
        function onAuthFailed(family) {
            root.append("auth-failed", { family: family });
        }
        function onBreakGlassIssued(reason, expiresAt) {
            root.append("break-glass-issued", {
                reason: reason,
                expiresAt: expiresAt
            });
        }
        function onBreakGlassConsumed(reason) {
            root.append("break-glass-consumed", { reason: reason });
        }
        function onBreakGlassExpired(reason) {
            root.append("break-glass-expired", { reason: reason });
        }
        function onGrantExpired(family) {
            root.append("grant-expired", { family: family });
        }
        function onGrantRevoked(family) {
            root.append("grant-revoked", { family: family });
        }
    }

    // ── Forensic: device state transitions ────────────────────────
    // These entries capture the decision context at each state change.
    // The Rust-side forensic log (forensic.jsonl) has the full sensor
    // snapshots; this trail records the same events in the
    // tamper-evident hash chain for cross-referencing.

    // Brightness / idle coordinator errors.
    Connections {
        target: typeof IdleCoordinator !== "undefined" ? IdleCoordinator : null
        function onStateTransitioned(state) {
            const names = ["active", "dimmed", "lock-requested",
                "lock-secure", "suspending", "asleep", "waking"];
            root.append("device-state-transition", {
                state: names[state] || String(state),
                source: "idle-coordinator"
            });
        }
    }

    // DPMS / screen power errors (from blueline-screen-toggle or
    // brightnessctl failures). These are the operational errors that
    // the old code logged to console.log only.
    function logDeviceError(component, action, error) {
        root.append("device-error", {
            component: component,
            action: action,
            error: error,
            device_state: typeof IdleCoordinator !== "undefined"
                ? IdleCoordinator.state : -1,
            screen_locked: typeof GlobalStates !== "undefined"
                ? GlobalStates.screenLocked : false,
            screen_lock_secure: typeof GlobalStates !== "undefined"
                ? GlobalStates.screenLockSecure : false,
        });
    }

    // Sensor input that affected device state (proximity, accel, etc.).
    function logSensorInput(source, value, confidence, decision) {
        root.append("sensor-input", {
            source: source,
            value: value,
            confidence: confidence,
            decision: decision,
            device_state: typeof IdleCoordinator !== "undefined"
                ? IdleCoordinator.state : -1,
        });
    }

    // Wake event — what triggered the screen to turn on.
    function logWakeEvent(trigger, details) {
        root.append("wake-event", {
            trigger: trigger,
            details: details || {},
            device_state: typeof IdleCoordinator !== "undefined"
                ? IdleCoordinator.state : -1,
            proximity: typeof GlobalStates !== "undefined"
                ? GlobalStates.proximityNear : null,
        });
    }

    Timer {
        id: initLogTimer
        interval: 0
        repeat: false
        running: true
        onTriggered: console.log("[audit] initialized; log at " + root.auditPath)
    }
}
