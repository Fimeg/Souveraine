// One typed view over sessiond's USB posture and mode verbs.
//
// usb-signaller owns configfs, souveraine-upower supplies adjacent charging
// evidence, and federation/probe leases will supply who is attached and who is
// already working there. The shell owns none of it; it asks sessiond and shows
// the answer.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    property bool available: false
    property bool busy: false
    property string lastError: ""
    property string mode: "unknown"
    property string rawMode: "unknown_mode"
    property var availableModes: []
    property string dataRole: "unknown"
    property bool chargerKnown: false
    property bool chargerOnline: false
    property var attachedIdentity: null
    property var probeOwner: null

    signal refreshed()
    signal changeFailed(string reason)

    function refresh() {
        root._send({ op: "usb" });
    }

    function setMode(mode) {
        if (!["developer", "hid", "kvm", "charging_only"].includes(mode)) {
            root.changeFailed("unsupported USB mode: " + mode);
            return;
        }
        root.busy = true;
        root._send({ op: "set_usb_mode", mode: mode });
    }

    function hasMode(rawMode) {
        return root.availableModes.includes(rawMode);
    }

    property var _queued: null

    function _send(message) {
        if (sock.connected) {
            sock.write(JSON.stringify(message) + "\n");
            return;
        }
        root._queued = message;
        sock.connected = true;
    }

    Socket {
        id: sock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/sessiond.sock"

        onConnectionStateChanged: {
            if (connected && root._queued) {
                const message = root._queued;
                root._queued = null;
                sock.write(JSON.stringify(message) + "\n");
            } else if (!connected && root._queued) {
                root._queued = null;
                root.available = false;
                root.busy = false;
                root.lastError = "sessiond socket unavailable";
                root.changeFailed(root.lastError);
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (error) {
                    root.lastError = "unparseable USB reply";
                    root.busy = false;
                    root.changeFailed(root.lastError);
                    return;
                }
                if (reply.ok !== true) {
                    root.lastError = reply.reason || "USB request refused";
                    root.busy = false;
                    root.changeFailed(root.lastError);
                    return;
                }

                root.mode = reply.mode ?? "unknown";
                root.rawMode = reply.raw_mode ?? "unknown_mode";
                root.availableModes = reply.available_modes ?? [];
                root.dataRole = reply.data_role ?? "unknown";
                root.chargerKnown = reply.charger_online !== null
                    && reply.charger_online !== undefined;
                root.chargerOnline = reply.charger_online === true;
                root.attachedIdentity = reply.attached_identity ?? null;
                root.probeOwner = reply.probe_owner ?? null;
                root.available = true;
                root.busy = false;
                root.lastError = "";
                root.refreshed();
            }
        }
    }
}
