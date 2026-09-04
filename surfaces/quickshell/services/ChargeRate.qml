// How fast the battery is actually charging — fast, standard, or trickle.
//
// Android surfaced this and the phone did not, so a charger that had quietly
// fallen back to trickle looked exactly like one that hadn't: "Charging 41%"
// either way, for hours.
//
// The value comes from the kernel's `charge_type`, which on SDM845 lives on
// the *charger* (`pmi8998-charger`), not the fuel gauge (`qcom-battery`) —
// the fuel gauge has no such attribute at all. Our UPower fork walks the
// supplier device link to find it and publishes it as the `ChargeType`
// property on the battery device (see packaging/upower-souveraine,
// up_device_supply_get_supplier_charge_type_str).
//
// Quickshell's UPowerDevice binds a fixed set of properties in C++ and
// `ChargeType` is not among them, so this reads D-Bus directly: one initial
// get-property, then gdbus monitor for changes (the property is declared
// emits-change, so the signal is real, not a poll). Same pattern as the
// squeekboard visibility monitor in OnScreenKeyboard.qml.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.UPower

Singleton {
    id: root

    // UpDeviceChargeType, from libupower-glib/up-types.h. Kept as plain ints
    // because the enum crosses as a uint32 and Quickshell has no binding for
    // it — if the fork ever renumbers, this list is the one place to fix.
    readonly property int unknown: 0
    readonly property int none: 1
    readonly property int trickle: 2
    readonly property int fast: 3
    readonly property int standard: 4
    readonly property int adaptive: 5
    readonly property int custom: 6
    readonly property int longlife: 7
    readonly property int bypass: 8

    // Raw charge type as UPower last reported it. -1 until the first read
    // lands, so "not asked yet" is distinguishable from "driver says unknown".
    property int chargeType: -1

    readonly property bool charging:
        UPower.displayDevice?.state === UPowerDeviceState.Charging

    // The cable is in and the charger has stopped anyway. This is normal
    // charge-termination hysteresis, not a fault: the charger terminates at
    // full, the pack self-discharges to its recharge threshold, and the
    // charger starts again. UPower reports `discharging` throughout, so a
    // surface that trusts state alone shows "Battery 94%" ticking down with
    // the cable plugged in — which reads as a failing charger and isn't one.
    readonly property bool pluggedNotCharging:
        !UPower.onBattery && !root.charging
        && UPower.displayDevice?.state !== UPowerDeviceState.FullyCharged

    // True when the rate is something worth saying out loud. `standard` is
    // the unremarkable case and gets the plain verb; none/unknown mean the
    // driver told us nothing and must not be dressed up as information.
    readonly property bool rateKnown: root.charging
        && root.chargeType !== root.none
        && root.chargeType !== root.unknown
        && root.chargeType !== -1

    // The phrase a surface shows in place of "Charging". Empty when the
    // device is not charging — a charge *rate* while discharging is a stale
    // reading of the last session, not a fact about now.
    readonly property string label: {
        if (!root.charging) return "";
        switch (root.chargeType) {
            case root.fast:     return "Fast charging";
            case root.trickle:  return "Slow charging";
            case root.adaptive: return "Adaptive charging";
            case root.longlife: return "Charging (long life)";
            case root.bypass:   return "Bypass charging";
            // standard, custom, none, unknown, and not-yet-read all fall
            // through to the verb with no adverb attached.
            default:            return "Charging";
        }
    }

    // Initial value. The monitor below only carries *changes*, so without
    // this a session that starts already plugged in shows nothing until the
    // charger next shifts gear — which on a topped-off pack may be never.
    Process {
        id: initialRead
        running: true
        command: ["sh", "-c",
            "p=$(upower -e | grep -m1 -i batt) || exit 0; " +
            "busctl get-property org.freedesktop.UPower \"$p\" " +
            "org.freedesktop.UPower.Device ChargeType"]
        stdout: SplitParser {
            // `busctl get-property` prints "u 4".
            onRead: line => {
                const m = line.match(/^u\s+(\d+)/);
                if (m) root.chargeType = parseInt(m[1], 10);
            }
        }
    }

    // PropertiesChanged carries the new value inline:
    //   /org/…/battery_qcom_battery: org.freedesktop.DBus.Properties
    //   ::PropertiesChanged ('org.freedesktop.UPower.Device',
    //   {'ChargeType': <uint32 3>}, @as [])
    // Broadcast signals are receivable without eavesdrop privileges, so this
    // needs no root and no polling.
    Process {
        id: chargeTypeMonitor
        running: true
        command: ["gdbus", "monitor", "--system",
                  "--dest", "org.freedesktop.UPower"]
        stdout: SplitParser {
            onRead: line => {
                const m = line.match(/'ChargeType':\s*<uint32\s+(\d+)>/);
                if (!m) return;
                const next = parseInt(m[1], 10);
                if (next === root.chargeType) return;
                console.log("[charge-rate] charge type " + root.chargeType
                    + " -> " + next);
                root.chargeType = next;
            }
        }
    }

    // A charger swap can change the rate without UPower re-emitting if the
    // battery device is re-added rather than updated. Re-read on every
    // plug/unplug edge; it is one busctl call, not a poll.
    Connections {
        target: UPower.displayDevice
        function onStateChanged() {
            initialRead.running = false;
            initialRead.running = true;
        }
    }
}
