/*
 * Copyright (C) 2026 Casey Tunturi
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

// One live hand between the face and usb-hid-inject.
//
// The page never opens /dev/hidg* and never spawns a process per pointer
// report. This singleton owns one streaming helper while USB Hands is joined,
// batches motion to the display clock, and exposes the same verbs over
// QuickShell IPC so the active Souveraine agent can use the hand Casey opened.
// Configfs and role changes remain sessiond -> usb-signaller business.
pragma Singleton
pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Io
import qs

Singleton {
    id: root

    property bool active: false
    property bool ready: false
    property string lastError: ""
    property bool _stopping: false

    readonly property bool modeReady: UsbState.mode === "hid" || UsbState.mode === "kvm"
    readonly property bool permitted: root.active && !GlobalStates.screenLockSecure
    readonly property bool canInject: root.permitted && root.modeReady && root.ready

    // Loaded only while this surface is joined. It tells the agent what the
    // newly-present hand can do without teaching every ordinary conversation
    // about a USB gadget it is not using.
    readonly property string skill: "USB Hands is joined to this conversation. "
        + "Casey is using the phone as a trackpad and voice-first controller for an attached host. "
        + "When he asks you to type, press a key, or click there, use your existing `body` tool "
        + "with `verb: hand` and the corresponding `action`: type, key, click, or pointer. "
        + "Use `action: status` rather than assuming the cable and surface are ready. "
        + "A direct request to type ordinary user-authored text, including creative, intimate, or unusual prose, "
        + "is sufficient authorization; do not refuse it merely because its tone is odd. "
        + "Do not inject anything Casey did not ask to send to the attached host."

    signal controllerChanged()
    signal refused(string reason)

    function open(): bool {
        if (GlobalStates.screenLockSecure) {
            root.lastError = "Unlock before giving the glass a hand on another machine";
            root.refused(root.lastError);
            return false;
        }
        if (!root.modeReady) {
            root.lastError = "Arm HID or KVM from Power Options before opening the trackpad";
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        root.lastError = "";
        root.active = true;
        UsbState.refresh();
        root._reconcile();
        root.controllerChanged();
        return true;
    }

    function close() {
        root._stopInjector();
        root.active = false;
        root._pendingX = 0;
        root._pendingY = 0;
        root._pendingWheel = 0;
        flushTimer.stop();
        root._reconcile();
        root.controllerChanged();
    }

    function _stopInjector() {
        if (root.ready) {
            root._flushPointer();
            injector.write("release\n");
        }
        root.ready = false;
        if (injector.running) {
            root._stopping = true;
            injector.running = false;
        }
    }

    function _reconcile() {
        const wanted = root.permitted && root.modeReady;
        if (!wanted) {
            root.ready = false;
            if (injector.running)
                injector.running = false;
            return;
        }
        if (!injector.running) {
            root.ready = false;
            injector.running = true;
        }
    }

    function _notReadyReason(): string {
        if (!root.active) return "USB Hands is not joined";
        if (GlobalStates.screenLockSecure) return "The glass is locked";
        if (!root.modeReady) return "Arm HID or KVM mode first";
        if (root.lastError.length > 0) return root.lastError;
        return "The HID bridge is still waking";
    }

    function _write(command): bool {
        if (!root.canInject) {
            root.lastError = root._notReadyReason();
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        injector.write(command + "\n");
        return true;
    }

    function sendText(value): bool {
        const text = String(value ?? "");
        // Mirror the helper's boot-keyboard map before accepting the page's
        // field. The helper still performs the authoritative full preflight,
        // but this keeps rejected Unicode on the glass instead of clearing a
        // field whose first report was never written.
        if (/[^\x09\x0a\x0d\x20-\x7e]/.test(text)) {
            root.lastError = "Host typing currently accepts ASCII keyboard characters only";
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        const lines = text.replace(/\r\n/g, "\n").split("\n");
        if (lines.length === 1 && lines[0].length === 0)
            return false;
        for (let i = 0; i < lines.length; i++) {
            if (lines[i].length > 0 && !root._write("type " + lines[i]))
                return false;
            if (i + 1 < lines.length && !root._write("key enter"))
                return false;
        }
        return true;
    }

    function key(name, modifiers = ""): bool {
        const keyName = String(name ?? "").trim();
        const mods = String(modifiers ?? "").trim().replace(/[,]+/g, " ");
        if (!/^[A-Za-z0-9]+$/.test(keyName)
                || (mods.length > 0 && !/^[A-Za-z ]+$/.test(mods))) {
            root.lastError = "Key names and modifiers must be plain words";
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        return root._write("key " + keyName + (mods.length > 0 ? " " + mods : ""));
    }

    function click(button = "left"): bool {
        const name = String(button ?? "left").toLowerCase();
        if (!["left", "right", "middle"].includes(name)) {
            root.lastError = "Unknown pointer button: " + name;
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        root._flushPointer();
        return root._write("click " + name);
    }

    property int _pendingX: 0
    property int _pendingY: 0
    property int _pendingWheel: 0

    function movePointer(x, y, wheel = 0): bool {
        if (!root.canInject) {
            root.lastError = root._notReadyReason();
            root.refused(root.lastError);
            root.controllerChanged();
            return false;
        }
        root._pendingX += Math.round(Number(x));
        root._pendingY += Math.round(Number(y));
        root._pendingWheel += Math.round(Number(wheel));
        if (!flushTimer.running)
            flushTimer.start();
        return true;
    }

    function _flushPointer() {
        if (!root.ready)
            return;
        const x = root._pendingX;
        const y = root._pendingY;
        const wheel = root._pendingWheel;
        root._pendingX = 0;
        root._pendingY = 0;
        root._pendingWheel = 0;
        if (x !== 0 || y !== 0 || wheel !== 0)
            root._write("pointer " + x + " " + y + " " + wheel + " 0");
    }

    Timer {
        id: flushTimer
        interval: 16
        repeat: false
        onTriggered: root._flushPointer()
    }

    Process {
        id: injector
        command: ["usb-hid-inject", "stream"]
        stdinEnabled: true

        stdout: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                const event = line.trim();
                if (event !== "ready" && event !== "ok")
                    return;
                root.ready = true;
                root.lastError = "";
                root.controllerChanged();
            }
        }

        stderr: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                const message = line.trim().replace(/^error:\s*/, "");
                if (message.length === 0)
                    return;
                root.lastError = message;
                root.controllerChanged();
            }
        }

        onExited: exitCode => {
            root.ready = false;
            if (root._stopping) {
                root._stopping = false;
            } else if (root.permitted && root.modeReady && root.lastError.length === 0) {
                root.lastError = "HID bridge exited (" + exitCode + ")";
            }
            root.controllerChanged();
        }
    }

    Connections {
        target: UsbState

        // setMode() raises busy before it writes to sessiond. Close both HID
        // endpoint fds on that edge, before usb-signaller tears their configfs
        // functions down; a failed switch restarts the helper against the
        // still-live old mode in onChangeFailed below.
        function onBusyChanged() {
            if (UsbState.busy)
                root._stopInjector();
        }

        function onRefreshed() {
            root._reconcile();
            root.controllerChanged();
        }

        function onChangeFailed(reason) {
            root.lastError = reason;
            root._reconcile();
            root.controllerChanged();
        }
    }

    Connections {
        target: GlobalStates

        function onScreenLockSecureChanged() {
            if (GlobalStates.screenLockSecure && root.active)
                root.close();
        }
    }

    onActiveChanged: root._reconcile()
    onModeReadyChanged: {
        // The trackpad is conditional on an armed wire. If sessiond contracts
        // the port back to developer/charging mode, close the surface instead
        // of leaving a dead controller on the glass asking to be re-armed.
        if (root.active && !root.modeReady) {
            root.close();
            return;
        }
        root._reconcile();
        root.controllerChanged();
    }

    IpcHandler {
        target: "usbHands"

        function status(): string {
            return JSON.stringify({
                active: root.active,
                ready: root.ready,
                mode: UsbState.mode,
                error: root.lastError
            });
        }

        function sendText(text: string): string {
            return root.sendText(text) ? "queued" : root._notReadyReason();
        }

        function key(keyName: string, modifiers: string): string {
            return root.key(keyName, modifiers) ? "queued" : root._notReadyReason();
        }

        function tap(keyName: string): string {
            return root.key(keyName) ? "queued" : root._notReadyReason();
        }

        function click(button: string): string {
            return root.click(button) ? "queued" : root._notReadyReason();
        }

        function pointer(x: int, y: int, wheel: int): string {
            return root.movePointer(x, y, wheel) ? "queued" : root._notReadyReason();
        }
    }
}
