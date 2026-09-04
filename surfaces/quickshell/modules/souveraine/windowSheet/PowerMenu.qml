// The device's own verbs, on a held power button.
//
// The counterpart to WindowSheet: that sheet is about a window, this is about
// the machine. Casey, 2026-08-05 — "when we hold the power button ... we'd be
// given another context overlay", and "that whole power usb state probably
// belongs in the power switch options".
//
// ## Why the USB state lives here
//
// The USB-C port is not a preference, it is what the device currently *is*:
// host or device, sourcing power or sinking it, and what is on the other end.
// It changes the meaning of every peripheral attached, which makes it
// proprioception and puts it in sessiond beside panel, lock and idle. This
// surface is the view over that, not a second owner of it.
//
// ## What is honest here today
//
// The data role and charger attachment are measured rather than guessed.
// UsbState reads sessiond's combined snapshot: usb-signaller owns the configfs
// mechanism, souveraine-upower is adjacent charging evidence, and sessiond is
// the authority that can turn a requested posture into an audited Action.
// This surface reads and asks; it never writes sysfs or calls the gadget daemon.
//
// There is no `/sys/class/typec/` on this device — the tcpm driver is not
// bound — so the *power* role genuinely is not knowable here yet. The row
// reports what the charger says and claims nothing more.
//
// ## Why it is not gated on the lock
//
// Powering off is something you do to a locked phone. A menu that disappeared
// when locked would send the user back to the hardware button they are already
// holding. It shows device verbs and never session content, so disclosure is
// unaffected.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import "." as SheetLocal

Scope {
    id: scope

    property bool menuOpen: false
    property bool powerOptionsOpen: false
    property bool attachmentOptionsOpen: false
    property bool batteryOptionsOpen: false
    property bool hidOptionsOpen: false

    // Measured, not assumed. Re-read each time the menu opens rather than
    // polled: the port's state only matters while someone is looking at it,
    // and a timer here would be a background reader of a file nobody needs.
    readonly property string dataRole: UsbState.dataRole
    readonly property bool chargerOnline: UsbState.chargerOnline

    function refresh() {
        UsbState.refresh();
    }

    function open() {
        scope.refresh();
        scope.powerOptionsOpen = false;
        scope.attachmentOptionsOpen = false;
        scope.batteryOptionsOpen = false;
        scope.hidOptionsOpen = false;
        scope.menuOpen = true;
    }

    function dismiss() {
        scope.menuOpen = false;
    }

    IpcHandler {
        target: "powerMenu"

        function open(): void {
            scope.open();
        }

        function close(): void {
            scope.dismiss();
        }

        function state(): string {
            return scope.dataRole + " charger_online=" + scope.chargerOnline;
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: win
            required property var modelData
            screen: win.modelData

            anchors { top: true; left: true; right: true; bottom: true }
            color: "transparent"
            visible: scope.menuOpen
            WlrLayershell.namespace: "souveraine:powermenu"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand
            exclusionMode: ExclusionMode.Ignore

            Rectangle {
                anchors.fill: parent
                color: "#99000000"

                MouseArea {
                    anchors.fill: parent
                    onClicked: scope.dismiss()
                }
            }

            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(win.width - 32, 500)
                spacing: 22

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 28

                    SheetLocal.SheetButton {
                        icon: "lock"
                        label: "Lock"
                        onTapped: {
                            scope.dismiss();
                            Session.lock();
                        }
                    }

                    SheetLocal.SheetButton {
                        icon: "restart_alt"
                        label: "Restart"
                        holdLabel: "hold to restart"
                        holdMs: 1000
                        // Hold, not tap. A restart reached by a single tap on a
                        // menu you opened by holding a button is one slip away
                        // from losing whatever was open.
                        onHeld: {
                            scope.dismiss();
                            Session.reboot();
                        }
                    }

                    SheetLocal.SheetButton {
                        icon: "power_settings_new"
                        label: "Power off"
                        holdLabel: "hold to power off"
                        holdMs: 1000
                        onHeld: {
                            scope.dismiss();
                            Session.poweroff();
                        }
                    }
                }

                // One gentle door into the more peculiar things the phone can
                // become. Live leaves ask sessiond; unfinished leaves keep their
                // future shape without pretending that a tap did anything.
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 9

                    SheetLocal.PowerOptionRow {
                        Layout.fillWidth: true
                        icon: "power"
                        title: "Power Options"
                        detail: "USB roles, KVM and hardware modes"
                        expandable: true
                        expanded: scope.powerOptionsOpen
                        onTapped: {
                            scope.powerOptionsOpen = !scope.powerOptionsOpen;
                            if (!scope.powerOptionsOpen) {
                                scope.attachmentOptionsOpen = false;
                                scope.batteryOptionsOpen = false;
                                scope.hidOptionsOpen = false;
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: scope.powerOptionsOpen
                        spacing: 8

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "usb"
                            title: "Port & attachment"
                            detail: scope.dataRole
                                + (!UsbState.chargerKnown ? " · power unknown"
                                    : scope.chargerOnline ? " · drawing power" : " · no charger")
                            badge: UsbState.available ? "live" : "waiting"
                            inset: 10
                            expandable: true
                            expanded: scope.attachmentOptionsOpen
                            onTapped: {
                                scope.attachmentOptionsOpen = !scope.attachmentOptionsOpen;
                                if (scope.attachmentOptionsOpen) {
                                    scope.batteryOptionsOpen = false;
                                    scope.hidOptionsOpen = false;
                                }
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            visible: scope.attachmentOptionsOpen
                            spacing: 7

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "fingerprint"
                                title: "Attached identity"
                                detail: UsbState.attachedIdentity
                                    ? UsbState.attachedIdentity : "USB enumeration + federation handshake"
                                badge: UsbState.attachedIdentity ? "known" : "next"
                                inset: 28
                            }

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "hub"
                                title: "Probe ownership"
                                detail: UsbState.probeOwner
                                    ? UsbState.probeOwner : "Which trusted node already has instruments here"
                                badge: UsbState.probeOwner ? "claimed" : "next"
                                inset: 28
                            }
                        }

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "battery_android_frame_shield"
                            title: "Battery care"
                            detail: "Charge ceiling, resume floor, agent discretion"
                            badge: "policy"
                            inset: 10
                            expandable: true
                            expanded: scope.batteryOptionsOpen
                            onTapped: {
                                scope.batteryOptionsOpen = !scope.batteryOptionsOpen;
                                if (scope.batteryOptionsOpen) {
                                    scope.attachmentOptionsOpen = false;
                                    scope.hidOptionsOpen = false;
                                }
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            visible: scope.batteryOptionsOpen
                            spacing: 7

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "battery_full_alt"
                                title: "Charge ceiling"
                                detail: "Stay below 100% inside a human safety envelope"
                                badge: "draft"
                                inset: 28
                            }

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "battery_horiz_050"
                                title: "Resume floor"
                                detail: "Wide hysteresis instead of twitching at 98/99"
                                badge: "draft"
                                inset: 28
                            }

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "neurology"
                                title: "Agent-managed"
                                detail: "May move the pair; every change is bounded and written"
                                badge: "planned"
                                inset: 28
                            }
                        }

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "keyboard"
                            title: "USB hand"
                            detail: HidController.active
                                ? "Wire armed · trackpad open"
                                : HidController.modeReady
                                    ? "Keyboard and pointer wire armed"
                                    : "Arm the wire, then open its trackpad"
                            badge: UsbState.busy ? "switching"
                                : HidController.active ? "open"
                                : HidController.modeReady ? "armed"
                                : UsbState.hasMode("hid_mode") ? "ready" : "building"
                            inset: 10
                            expandable: true
                            expanded: scope.hidOptionsOpen
                            onTapped: {
                                scope.hidOptionsOpen = !scope.hidOptionsOpen;
                                if (scope.hidOptionsOpen) {
                                    scope.attachmentOptionsOpen = false;
                                    scope.batteryOptionsOpen = false;
                                }
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            visible: scope.hidOptionsOpen
                            spacing: 7

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "cable"
                                title: HidController.modeReady
                                    ? "Disarm USB hand" : "Arm USB hand"
                                detail: HidController.modeReady
                                    ? "Restore developer USB and close its controls"
                                    : "Expose keyboard, pointer and developer network"
                                badge: UsbState.busy ? "switching"
                                    : HidController.modeReady ? "armed"
                                    : UsbState.hasMode("hid_mode") ? "ready" : "not installed"
                                inset: 28
                                available: UsbState.hasMode("hid_mode") && !UsbState.busy
                                onTapped: UsbState.setMode(
                                    HidController.modeReady ? "developer" : "hid")
                            }

                            SheetLocal.PowerOptionRow {
                                Layout.fillWidth: true
                                icon: "keyboard_mouse"
                                title: HidController.active
                                    ? "Close trackpad" : "Open trackpad"
                                detail: HidController.active
                                    ? "Leave the wire armed; put its controls away"
                                    : "Voice, pointer, scroll and clicks"
                                badge: HidController.ready ? "awake"
                                    : HidController.active ? "waking"
                                    : GlobalStates.screenLockSecure ? "locked" : "closed"
                                inset: 28
                                available: HidController.modeReady && !UsbState.busy
                                    && !GlobalStates.screenLockSecure
                                onTapped: {
                                    if (HidController.active) {
                                        Face.leaveHands();
                                        scope.dismiss();
                                    } else if (Face.joinHands()) {
                                        scope.dismiss();
                                    }
                                }
                            }
                        }

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "desktop_windows"
                            title: "Full USB KVM"
                            detail: UsbState.mode === "kvm"
                                ? "Active · tap to restore developer USB"
                                : "GUD display · HID input · NCM control · smoo"
                            badge: UsbState.busy ? "switching"
                                : UsbState.mode === "kvm" ? "active"
                                : UsbState.hasMode("kvm_mode") ? "ready" : "not installed"
                            inset: 10
                            available: UsbState.hasMode("kvm_mode") && !UsbState.busy
                            onTapped: UsbState.setMode(
                                UsbState.mode === "kvm" ? "developer" : "kvm")
                        }

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "hard_drive"
                            title: "Host-backed volume"
                            detail: "Mount storage offered by the attached host"
                            badge: UsbState.mode === "kvm" ? "active" : "smoo"
                            inset: 10
                        }

                        SheetLocal.PowerOptionRow {
                            Layout.fillWidth: true
                            icon: "electrical_services"
                            title: "USB host + power share"
                            detail: "Become the host and source 5 V"
                            badge: "kernel"
                            inset: 10
                        }
                    }

                    // The port, as measured. Not a control yet — see the
                    // architecture note at the top of this file.
                    StyledText {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.maximumWidth: win.width * 0.8
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.WordWrap
                        color: "#aaffffff"
                        font.pixelSize: Appearance.font.pixelSize.small
                        text: "USB-C: " + scope.dataRole
                            + (!UsbState.chargerKnown ? " · power unknown"
                                : scope.chargerOnline ? " · charger attached" : " · no charger")
                    }
                }
            }
        }
    }
}
