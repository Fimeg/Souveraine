// The Settings view over souveraine-sessiond's device-state policy.
//
// TASK-19's rule is that every control is a view over the owning service — "no
// success-shaped switches". The idle timers are owned by the state machine in
// sessiond, not by a JSON file in ~/.config, so a spinbox that writes
// Config.options and stops there is exactly the lie that rule forbids. It was
// that lie until 2026-07-25: the settings page moved dimAfterSeconds and
// lockAfterSeconds while the daemon that actually blanks the panel ran on its
// compiled-in defaults, because SetPolicy existed in the protocol with zero
// callers anywhere in the tree.
//
// This talks to the daemon on a SEPARATE, short-lived connection. The
// SessiondBridge socket is the heartbeat: its EOF is how sessiond learns the
// shell died, and its open line is how the authority pushes directives. Settings
// traffic does not belong on it. A second connection is harmless — only
// `shell_ready` claims the authority lease, and this never sends it.
//
// Failures here are LOUD. A policy write that silently did nothing would
// recreate the exact bug this file exists to close.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    // True once a reply has been parsed — until then the page must not claim
    // to be showing the daemon's values.
    property bool available: false
    property string lastError: ""

    // Mirrors DeviceStatePolicy. Seconds; 0 means "never" for the budgets.
    property int lockBlankAfterSecs: 0
    property int lockBlankAfterHeldSecs: 0
    property int dimGraceSecs: 0
    property int evidenceTtlSecs: 0
    property bool dimWarning: true
    property int lockAckBudgetSecs: 0
    property int unlockedBlankAfterSecs: 0

    signal refreshed()
    signal applyFailed(string reason)

    function refresh() {
        root._send({ op: "get_policy" });
    }

    // Every field is optional daemon-side; omitted fields keep their value.
    function apply(fields) {
        const msg = { op: "set_policy" };
        for (const k in fields) msg[k] = fields[k];
        root._send(msg);
    }

    property var _queued: null

    function _send(msg) {
        if (sock.connected) {
            sock.write(JSON.stringify(msg) + "\n");
            return;
        }
        // One in flight is enough; Settings is a single page and the daemon
        // answers in microseconds.
        root._queued = msg;
        sock.connected = true;
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
                root.lastError = "sessiond socket unavailable";
                root.available = false;
                console.error("[sessiond-policy] could not reach the daemon at "
                    + sock.path + " — the idle timers shown are NOT authoritative");
                root._queued = null;
                root.applyFailed(root.lastError);
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (e) {
                    console.error("[sessiond-policy] unparseable reply: " + message);
                    root.lastError = "unparseable reply";
                    root.applyFailed(root.lastError);
                    return;
                }

                if (reply.ok !== true) {
                    const why = reply.reason || "refused without a reason";
                    console.error("[sessiond-policy] daemon refused: " + why);
                    root.lastError = why;
                    root.applyFailed(why);
                    return;
                }

                // get_policy answers flat; set_policy answers under `policy`.
                const p = reply.policy !== undefined ? reply.policy : reply;
                root.lockBlankAfterSecs = p.lock_blank_after_secs ?? 0;
                root.lockBlankAfterHeldSecs = p.lock_blank_after_held_secs ?? 0;
                root.dimGraceSecs = p.dim_grace_secs ?? 0;
                root.evidenceTtlSecs = p.evidence_ttl_secs ?? 0;
                root.dimWarning = p.dim_warning === true;
                root.lockAckBudgetSecs = p.lock_ack_budget_secs ?? 0;
                root.unlockedBlankAfterSecs = p.unlocked_blank_after_secs ?? 0;
                root.available = true;
                root.lastError = "";
                root.refreshed();
            }
        }
    }
}
