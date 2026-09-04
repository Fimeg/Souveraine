import QtQuick
import qs.services
import qs.modules.common
import Quickshell
import Quickshell.Io

/**
 * A WireGuard tunnel, toggled deliberately.
 *
 * DELIBERATE, NEVER AUTOMATIC — and that is the design, not a limitation.
 * An autonomous gate held this tunnel on 2026-07-30: it recycled 652 times in
 * 90 minutes, through doze tiers whose contract is "network fetchers stopped",
 * and the user could not switch it off because `nmcli connection down` fired
 * the NM dispatcher that immediately brought it back up. A control surface the
 * owner cannot overrule is not a control surface.
 *
 * So there is no daemon, no timer and no dispatcher hook deciding this. The
 * toggle is the whole of it. If tunnel posture later becomes something the
 * device decides for itself, it belongs in sessiond's state machine as an
 * Action (DEVICE-STATE-MACHINE §12) and reachable as a verb (doctrine §13) —
 * not as another actor off to the side.
 *
 * No privilege needed: polkit already lets the seat user activate a system
 * connection, verified on device.
 */
QuickToggleModel {
    id: root

    /// The NetworkManager connection this drives. A property rather than a
    /// constant so a second profile does not need a second component.
    ///
    /// Give each device its OWN peer. A profile that shares an address and a
    /// private key between two machines lets WireGuard connect only one of
    /// them at a time, and one dialling a port with a key the server no longer
    /// lists will send and never receive. Set this per deployment.
    property string connectionName: ""

    /// True while an up/down is in flight, so a double tap cannot race itself.
    property bool busy: false

    /// A host that only exists at the far end of the tunnel. Reaching it is the
    /// only honest proof the tunnel carries traffic.
    property string probeHost: ""

    /// "off" | "connecting" | "limited" | "online".
    ///
    /// NOT a bool, and that is the point. On 2026-07-31 this toggle read "On"
    /// for an hour while the tunnel was deaf: NetworkManager said `activated`,
    /// `wg` said 368 B received against 5.3 KiB sent, and every service behind
    /// it — STT, TTS, gitea — was unreachable. "The interface is up" and "the
    /// tunnel carries traffic" are different claims and only one of them is
    /// worth showing.
    ///
    /// Sailfish models exactly this distinction (`MobileDataConnection.status`
    /// is Online / Limited / Connecting, never a bool) and Android refuses to
    /// let an unvalidated network win at all. `limited` is the state both have
    /// and we did not.
    ///
    /// The probe is deliberately NOT an internet check. This is a split tunnel
    /// whose whole purpose is a private range that no public probe can
    /// see; a link that fails a general-internet probe may be exactly the one
    /// we want. The question is per-consumer — "can this carry *this*" — so we
    /// ask the far end directly.
    property string status: "off"

    name: Translation.tr("VPN")
    icon: root.status === "online" ? "vpn_lock"
        : root.status === "limited" ? "vpn_lock_off"
        : "vpn_key"
    statusText: {
        if (root.busy)
            return Translation.tr("…");
        switch (root.status) {
        case "online":  return Translation.tr("On");
        case "limited": return Translation.tr("No route");
        default:        return Translation.tr("Off");
        }
    }
    tooltipText: {
        switch (root.status) {
        case "online":
            return Translation.tr("VPN — the private range routed over the tunnel");
        case "limited":
            return Translation.tr("VPN — connected but not carrying traffic. The tunnel is up and the far side is unreachable; check which link the handshake is leaving by.");
        default:
            return Translation.tr("VPN — off");
        }
    }

    mainAction: () => {
        if (root.busy)
            return;
        root.busy = true;
        if (root.toggled)
            downProc.running = true;
        else
            upProc.running = true;
    }

    function notify(body) {
        Quickshell.execDetached(["notify-send", Translation.tr("VPN"), body, "-a", "Shell"]);
    }

    Process {
        id: upProc
        command: ["nmcli", "connection", "up", root.connectionName]
        onExited: (exitCode, exitStatus) => {
            root.busy = false;
            refreshProc.running = true;
            if (exitCode !== 0)
                root.notify(Translation.tr("Could not connect."));
        }
    }

    Process {
        id: downProc
        command: ["nmcli", "connection", "down", root.connectionName]
        onExited: (exitCode, exitStatus) => {
            root.busy = false;
            refreshProc.running = true;
            if (exitCode !== 0)
                root.notify(Translation.tr("Could not disconnect."));
        }
    }

    // Read NM rather than tracking a local bool: the connection can also be
    // brought up or down from nmcli, from Settings, or by NM itself, and a
    // shadow copy that disagrees with the owning subsystem is the failure
    // doctrine §4 names ("never a hand-tracked bool").
    // Two questions, asked together because the answer to the first does not
    // imply the second: is the connection active, and does it carry traffic.
    // A single `nmcli` reply can only answer the first, which is how this
    // reported "On" through an hour of a dead tunnel.
    //
    // The probe is one UDP DNS round-trip to the far-side resolver with a 2 s
    // deadline — cheap enough for a 15 s poll, and it fails in exactly the case
    // that matters (interface up, nothing crossing it). `getent` is not used:
    // it would consult the whole resolver list and could be answered by a
    // nameserver on another link, which is the same conflation this is here to
    // end.
    Process {
        id: refreshProc
        running: true
        command: ["bash", "-c",
            `if ! nmcli -t -f NAME connection show --active | grep -qx '${root.connectionName}'; then echo off; exit 0; fi;`
            + ` if timeout 2 bash -c 'echo > /dev/udp/${root.probeHost}/53' 2>/dev/null`
            + ` && timeout 2 ping -c1 -W2 ${root.probeHost} >/dev/null 2>&1; then echo online; else echo limited; fi`]
        stdout: StdioCollector {
            id: stateCollector
            onStreamFinished: {
                const reply = stateCollector.text.trim();
                if (reply.length === 0)
                    return;
                root.status = reply;
                // `toggled` stays the user-facing "is it switched on", so the
                // button still lights while the tunnel is limited — the state
                // is reported in statusText rather than by silently un-toggling
                // something the user turned on. Sailfish does the same: the
                // switch stays checked and the description says "Limited".
                root.toggled = (reply !== "off");
            }
        }
    }

    // Cheap resync for changes made outside the panel. One nmcli call every
    // 15 s while the shell runs; deliberately not a decision loop.
    Timer {
        interval: 15000
        running: true
        repeat: true
        onTriggered: {
            if (!root.busy)
                refreshProc.running = true;
        }
    }
}
