pragma Singleton
pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Io

/**
 * DeviceEvidence — the shell's ingress to the device state machine.
 *
 * DEVICE-STATE-MACHINE §1 is a list of seven actors that each saw one facet of
 * the device and could not see the others. Every shell surface that takes user
 * input is a candidate for becoming an eighth. This singleton exists so that
 * "report that the user did something" is one line, and a new surface has no
 * excuse to be an isolated unknown.
 *
 * The wire is `Request::Input { trigger }` (souveraine/src/sessiond/protocol.rs
 * — that file is the contract): `{"op":"input","trigger":"touch"}`. It resets
 * the idle budget the lock/blank rules count against, which is what lets the
 * machine distinguish "the user is looking at this" from "this has been lit for
 * ten minutes."
 *
 * `intent` rides along in the Envelope. protocol.rs is explicit that it is
 * "declared, never verified... evidence in exactly the sense doctrine §9 means —
 * useful for reconstruction, never a basis for a decision. Nothing branches on
 * it." So it is safe to be honest in, and it is what §11 wants recorded: the
 * intent, not only the leaf. Never put user content in it — a surface name, not
 * what the surface was showing.
 *
 * No sessiond on the socket (laptop, or bring-up) = every call no-ops quietly.
 * This is evidence, not an authority: a dropped report must never be an error
 * the user sees.
 */
Singleton {
    id: root

    // Reports are coalesced: a keyboard would otherwise emit one request per
    // keystroke to reset a budget measured in tens of seconds. The machine only
    // needs to know the user is still there.
    readonly property int _throttleMs: 2000

    property double _lastSentAt: 0
    property string _pendingTrigger: ""
    property string _pendingIntent: ""

    /**
     * Report real user input.
     *
     * trigger: "touch" | "key" | "power_button" | "double_tap_to_wake"
     *          | "squeeze" | "unknown"  (InputTrigger, snake_case)
     * intent:  short surface label, e.g. "selection-menu". No user content.
     */
    function report(trigger, intent) {
        const t = String(trigger ?? "unknown");
        const now = Date.now();
        if (now - root._lastSentAt < root._throttleMs) {
            // Keep the newest label; the budget reset is idempotent so dropping
            // the intervening reports costs nothing.
            root._pendingTrigger = t;
            root._pendingIntent = String(intent ?? "");
            flushTimer.running = true;
            return;
        }
        root._send(t, String(intent ?? ""));
    }

    /** Convenience for the common case: a tap on one of our own surfaces. */
    function touched(intent) {
        root.report("touch", intent);
    }

    Timer {
        id: flushTimer
        interval: root._throttleMs
        repeat: false
        onTriggered: {
            if (root._pendingTrigger.length === 0) return;
            root._send(root._pendingTrigger, root._pendingIntent);
            root._pendingTrigger = "";
            root._pendingIntent = "";
        }
    }

    property var _queued: null

    function _send(trigger, intent) {
        root._lastSentAt = Date.now();
        const msg = { op: "input", trigger: trigger };
        if (intent.length > 0) msg.intent = intent;
        if (sock.connected) {
            sock.write(JSON.stringify(msg) + "\n");
            return;
        }
        root._queued = msg;
        sock.connected = true;
    }

    // ── Ingress: the machine's own account of itself ─────────────────────
    //
    // Everything above is egress — the shell telling sessiond that something
    // happened. This half is the other direction, and until now it did not
    // exist: the state machine computes its state, its evidence, its
    // confidence and its per-source health, and **no surface could see any of
    // it** (TASK-08(f), TASK-19). The trail knew and the glass did not.
    //
    // Strictly a projection. It reads `device_state`, holds nothing the
    // protocol owns, and decides nothing — DEVICE-STATE-MACHINE §1's whole
    // complaint is actors that saw one facet and acted on it, and a readout
    // that started branching would be the eighth. Doctrine §4: read the
    // authority, never mirror it into a second source of truth.
    //
    // Polled only while a surface is actually looking (watch/unwatch). A
    // settings page open on the desk should not cost a request per second for
    // the rest of the day.

    /// True once sessiond has answered at least once. False on the laptop,
    /// where there is no daemon — surfaces must render that as "unavailable",
    /// never as healthy-looking zeroes.
    property bool available: false
    /// The last `device_state` reply, verbatim. Read-only to every consumer.
    property var state: ({})
    /// Recent forensic entries (the decision trail), newest last.
    property var recentDecisions: []
    /// ms epoch of the last successful read; 0 = never.
    property double lastReadAt: 0

    property int _watchers: 0

    /** Begin polling. Pair every call with unwatch(). */
    function watch() {
        root._watchers += 1;
        if (root._watchers === 1) {
            readTimer.running = true;
            root._query();
        }
    }

    function unwatch() {
        root._watchers = Math.max(0, root._watchers - 1);
        if (root._watchers === 0) {
            readTimer.running = false;
            readSock.connected = false;
        }
    }

    /** One-shot refresh, whether or not anything is watching. */
    function refresh() {
        root._query();
    }

    property bool _queryPending: false

    function _query() {
        if (readSock.connected) {
            readSock.write(JSON.stringify({ op: "device_state" }) + "\n");
            readSock.write(JSON.stringify({ op: "forensic_log", count: 20 }) + "\n");
            return;
        }
        root._queryPending = true;
        readSock.connected = true;
    }

    Timer {
        id: readTimer
        interval: 2000
        repeat: true
        running: false
        onTriggered: root._query()
    }

    // A SECOND connection, deliberately. The egress socket above is
    // fire-and-forget and throttled; interleaving request/response traffic on
    // it would mean correlating replies to writes that may never come. This
    // one only ever asks questions. It does NOT register shell authority —
    // that is SessiondBridge's job, and a second registration is what
    // deadlocks the lease.
    Socket {
        id: readSock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/sessiond.sock"

        onConnectionStateChanged: {
            if (connected && root._queryPending) {
                root._queryPending = false;
                readSock.write(JSON.stringify({ op: "device_state" }) + "\n");
                readSock.write(JSON.stringify({ op: "forensic_log", count: 20 }) + "\n");
            } else if (!connected) {
                root._queryPending = false;
                // No daemon is the laptop's normal state. Say unavailable and
                // let the surface show that, rather than leaving stale values
                // on screen that look current.
                root.available = false;
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (e) {
                    return;
                }
                if (reply.ok !== true) {
                    console.log("[device-evidence] read refused:",
                        reply.code ?? "?", reply.reason ?? "");
                    return;
                }
                // device_state carries the state field; forensic_log carries
                // entries. One parser, two shapes, told apart by content
                // rather than by a correlation id the protocol does not have.
                if (reply.device_state !== undefined) {
                    root.state = reply;
                    root.available = true;
                    root.lastReadAt = Date.now();
                } else if (reply.entries !== undefined) {
                    root.recentDecisions = reply.entries;
                }
            }
        }
    }

    Socket {
        id: sock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/sessiond.sock"

        onConnectionStateChanged: {
            if (connected && root._queued) {
                const m = root._queued;
                root._queued = null;
                sock.write(JSON.stringify(m) + "\n");
            } else if (!connected && root._queued) {
                // Quiet on purpose. Evidence is best-effort; a missing daemon is
                // the laptop's normal state and must not look like a fault.
                root._queued = null;
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                // Nothing to do with a reply — this is fire-and-forget. Only a
                // refusal is worth a line, so a protocol drift is not silent.
                try {
                    const reply = JSON.parse(message);
                    if (reply.ok !== true)
                        console.log("[device-evidence] refused:",
                            reply.code ?? "?", reply.reason ?? "");
                } catch (e) {
                    // Malformed reply is not worth escalating for a fire-and-forget.
                }
            }
        }
    }
}
