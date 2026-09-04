// Shared lock-surface glance host. Credentials are supplied by the parent
// lock surface; this component owns only ambient cards so every form factor
// shares the same policy and data model.
import QtQuick
import QtQuick.Layouts
import Quickshell.Services.UPower
import qs.services
import qs.modules.common
import "."

Item {
    id: root

    readonly property bool compact: width < 700
    implicitHeight: content.implicitHeight

    Timer {
        interval: 1000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: clock.now = new Date()
    }

    QtObject {
        id: clock
        property date now: new Date()
    }

    ColumnLayout {
        id: content
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(parent.width, root.compact ? 460 : 620)
        spacing: root.compact ? 10 : 16

        Text {
            Layout.fillWidth: true
            text: Qt.formatTime(clock.now,
                Config.options.lock.twelveHourClock ? "h:mm ap" : "hh:mm")
            color: "white"
            font.pixelSize: root.compact ? 60 : 76
            font.weight: Font.Light
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            Layout.fillWidth: true
            text: Qt.formatDate(clock.now, "dddd, MMMM d")
            color: "#d9ffffff"
            font.pixelSize: root.compact ? 17 : 21
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            Layout.alignment: Qt.AlignHCenter
            visible: LockContentPolicy.batteryVisible && UPower.displayDevice?.isPresent
            text: {
                const dev = UPower.displayDevice;
                // UPowerDevice.percentage is a 0–1 fraction (TouchLockSurface
                // multiplies too) — rounding it raw is what rendered the
                // eternal "Charging 0%" here.
                const pct = Math.round((dev?.percentage ?? 0) * 100);
                // State is authoritative — don't infer "Charging" from
                // onBattery alone. onBattery is false for fully-charged,
                // pending-charge, and unknown, which would read "Charging N%"
                // forever on a topped-off pack.
                switch (dev?.state ?? UPowerDeviceState.Unknown) {
                    case UPowerDeviceState.FullyCharged:
                        return "Full " + pct + "%";
                    case UPowerDeviceState.Charging:
                        // ChargeRate.label is "Charging" unless the charger
                        // is doing something worth naming — fast, trickle,
                        // long-life. A charger that has quietly dropped to
                        // trickle used to look exactly like one that hadn't.
                        // Right after a driver rebind the pack can briefly
                        // report 0% while charging. Drop the number rather
                        // than lie.
                        return pct > 0 ? ChargeRate.label + " " + pct + "%"
                                       : ChargeRate.label;
                    case UPowerDeviceState.Discharging:
                        // Discharging with the cable still in is the charger
                        // resting between top-ups, not the battery draining.
                        // Saying "Battery 94%" there — and watching it count
                        // down while plugged in — reads as a broken charger.
                        if (ChargeRate.pluggedNotCharging)
                            return pct >= 95 ? "Charged " + pct + "%"
                                             : "Plugged in " + pct + "%";
                        return "Battery " + pct + "%";
                    default:
                        // Unknown / Empty / Pending* — show the number
                        // without a misleading verb.
                        return pct + "%";
                }
            }
            color: "#d9ffffff"
            font.pixelSize: 14
        }

        LockMediaCard {
            Layout.topMargin: root.compact ? 8 : 14
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
        }

        LockAgentCard {
            Layout.topMargin: root.compact ? 6 : 10
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
        }

        LockNotifyCard {
            Layout.topMargin: root.compact ? 6 : 10
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
        }
    }
}
