pragma Singleton
pragma ComponentBehavior: Bound

// Cellular/modem status via mmcli (ModemManager CLI), following the same
// process-based-service pattern as services/Network.qml (nmcli). No
// existing service in this shell talks to ModemManager — this is new,
// added for Pixel 3 (blueline) telephony support.
//
// Pattern sources (see Pixel3Arch/references/shells/):
// - Marathon-Image's CellularManager.qml/NetworkManager.qml gave the
//   property shape (operatorName, networkType, signalStrength, roaming)
//   but proxy a native ModemManagerCpp backend that doesn't exist in that
//   repo — not directly portable.
// - Phosh's src/wwan/phosh-wwan-mm.c is the real, shipping reference: it
//   reads ModemManager's base `Modem` interface (SignalQuality,
//   AccessTechnologies, State) and the separate `Modem.Modem3gpp`
//   interface (OperatorName) as two distinct D-Bus interfaces on the same
//   modem object path, and updates via PropertiesChanged signals rather
//   than polling. mmcli -J surfaces both interfaces' fields in one call,
//   so we replicate the *signal-driven refresh*, not the polling
//   Marathon-Image's own comments call out as a placeholder.

import Quickshell
import Quickshell.Io
import QtQuick

Singleton {
    id: root

    property bool available: false
    property string modemPath: ""
    property string state: "unknown" // registered, searching, denied, unknown, disabled
    property int signalQuality: 0     // 0-100
    property string operatorName: ""
    property string accessTech: "" // GSM, EDGE, 3G, HSPA, LTE, 5G (Phosh's user-friendly mapping)
    property bool roaming: false

    // "has service" = registered or better. A modem with an active data
    // bearer reports state "connected" (not "registered"), so matching only
    // the exact string "registered" made a working 4G data link render the
    // "connected, no internet" error glyph. Phosh's phosh_wwan_mm treats
    // registered/connecting/connected identically — mirror that.
    readonly property bool hasService: ["registered", "connecting", "connected"].indexOf(root.state) >= 0

    readonly property string materialSymbol: !root.available
        ? "signal_cellular_off"
        : !root.hasService
            ? "signal_cellular_0_bar"   // searching / denied / disabled
            : (
                root.signalQuality > 80 ? "signal_cellular_4_bar" :
                root.signalQuality > 60 ? "signal_cellular_3_bar" :
                root.signalQuality > 40 ? "signal_cellular_2_bar" :
                root.signalQuality > 20 ? "signal_cellular_1_bar" :
                "signal_cellular_0_bar"
            )

    // MMModemAccessTechnology bitmask -> label, mirrors Phosh's
    // phosh_wwan_mm_user_friendly_access_tec() threshold order
    // (5GNR > LTE > HSPA+ > HSPA > UMTS/3G > EDGE > GSM), applied to the
    // string mmcli already prints for `generic.access-technologies[0]`.
    function friendlyAccessTech(raw) {
        if (!raw)
            return "";
        const t = raw.toLowerCase();
        if (t.includes("5gnr"))
            return "5G";
        if (t.includes("lte"))
            return "LTE";
        if (t.includes("hspa+"))
            return "H+";
        if (t.includes("hspa"))
            return "H";
        if (t.includes("umts"))
            return "3G";
        if (t.includes("edge"))
            return "E";
        if (t.includes("gsm"))
            return "G";
        return raw.toUpperCase();
    }

    function update() {
        findModem.running = true;
    }

    Process {
        id: findModem
        command: ["mmcli", "-L", "-J"]
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const data = JSON.parse(text);
                    const modems = data["modem-list"] || [];
                    if (modems.length === 0) {
                        root.available = false;
                        root.modemPath = "";
                        return;
                    }
                    root.modemPath = modems[0];
                    modemDetail.command = ["mmcli", "-m", modems[0], "-J"];
                    modemDetail.running = true;
                } catch (e) {
                    root.available = false;
                }
            }
        }
    }

    Process {
        id: modemDetail
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const data = JSON.parse(text);
                    const modem = data.modem;
                    const generic = modem["generic"];
                    const threegpp = modem["3gpp"];

                    root.available = true;
                    root.state = generic["state"] || "unknown";
                    root.signalQuality = parseInt((generic["signal-quality"] && generic["signal-quality"]["value"]) || "0", 10);

                    const techs = generic["access-technologies"] || [];
                    root.accessTech = techs.length > 0 ? root.friendlyAccessTech(techs[0]) : "";

                    root.operatorName = (threegpp && threegpp["operator-name"]) ? threegpp["operator-name"] : "";
                    root.roaming = !!(threegpp && threegpp["registration-state"] === "roaming");
                } catch (e) {
                    root.available = false;
                }
            }
        }
    }

    // Signal-driven refresh: subscribe to ModemManager's PropertiesChanged
    // on both interfaces it actually emits changes on (base Modem +
    // Modem.Modem3gpp — same two-interface split Phosh's real code uses),
    // re-running the mmcli read on any change instead of polling on a
    // fixed timer. This is the equivalent of Network.qml's long-running
    // `nmcli monitor` subscriber process.
    Process {
        id: subscriber
        running: true
        // gdbus (not dbus-monitor): unprivileged users can't get monitor
        // rights on the system bus, and dbus-monitor's eavesdrop fallback
        // silently receives nothing there. gdbus subscribes with normal
        // match rules, which broadcast signals like PropertiesChanged
        // always reach.
        command: ["gdbus", "monitor", "-y", "-d", "org.freedesktop.ModemManager1"]
        stdout: SplitParser {
            onRead: line => {
                if (line.includes("PropertiesChanged"))
                    root.update();
            }
        }
    }

    // Fallback: the initial update can race ModemManager's own startup
    // (modem appears seconds after the shell), and a missed signal would
    // otherwise stick forever. Slow re-read, not the primary mechanism.
    Timer {
        interval: 30000
        running: true
        repeat: true
        onTriggered: root.update()
    }

    Component.onCompleted: update()
}
