// One owner for the temporary FPC1020 interaction path.
//
// The privileged reader daemon publishes only an IRQ pulse. Until the
// match-on-chip daemon exists, a user-visible hold is the configured temporary
// confirmation. The Polkit surface may submit the configured temporary PAM
// factor after a completed hold; it is never a lock-screen unlock path.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs.modules.common

Singleton {
    id: root

    readonly property bool previewEnabled:
        (Config.options && Config.options.lock && Config.options.lock.fingerprintPreview
            && Config.options.lock.fingerprintPreview.enabled) ?? false
    readonly property bool polkitEnabled:
        (Config.options && Config.options.lock && Config.options.lock.fingerprintPolkit
            && Config.options.lock.fingerprintPolkit.enabled) ?? true
    readonly property bool enabled: previewEnabled || polkitEnabled
    readonly property int holdMs:
        (Config.options && Config.options.lock && Config.options.lock.fingerprintPreview
            && Config.options.lock.fingerprintPreview.holdMs) ?? 3000
    property bool holding: false
    property bool confirmed: false
    property bool pulseSeen: false
    property real holdProgress: 0
    property double holdStartedAt: 0
    property string activePurpose: ""
    property string confirmedPurpose: ""
    property string pulseToken: ""

    signal pulseObserved()
    signal holdConfirmed(string purpose)

    function beginHold(purpose) {
        if (!root.enabled || !purpose) return false;
        root.confirmed = false;
        root.confirmedPurpose = "";
        root.activePurpose = purpose;
        root.holding = true;
        root.holdStartedAt = Date.now();
        root.holdProgress = 0;
        holdTimer.start();
        return true;
    }

    function cancelHold(purpose = "") {
        if (purpose && root.activePurpose !== purpose) return;
        root.holding = false;
        root.holdProgress = 0;
        root.activePurpose = "";
        holdTimer.stop();
    }

    function confirmHold(purpose = "") {
        if (!root.holding || (purpose && root.activePurpose !== purpose)) return;
        const confirmedPurpose = root.activePurpose;
        root.holding = false;
        root.holdProgress = 1;
        root.activePurpose = "";
        holdTimer.stop();
        root.confirmed = true;
        root.confirmedPurpose = confirmedPurpose;
        root.holdConfirmed(confirmedPurpose);
        confirmTimer.restart();
    }

    // The diagnostic IPC and the reader file meet here. Neither can unlock a
    // session; Polkit alone may use the explicit temporary confirmation path.
    function notePulse() {
        if (!root.enabled)
            return { ok: false, code: "not_enabled", reason: "fingerprint preview is disabled" };
        root.pulseSeen = true;
        root.pulseObserved();
        pulseTimer.restart();
        return { ok: true, status: "observed" };
    }

    function reset(purpose = "") {
        if (purpose && root.activePurpose !== purpose && root.confirmedPurpose !== purpose)
            return;
        root.cancelHold();
        root.confirmed = false;
        root.confirmedPurpose = "";
        root.pulseSeen = false;
        confirmTimer.stop();
        pulseTimer.stop();
    }

    FileView {
        id: pulseFile
        path: "/run/blueline-fingerprintd/preview-pulse"
        watchChanges: true
        printErrors: false
        onFileChanged: reload()
        onLoaded: {
            try {
                const record = JSON.parse(pulseFile.text());
                const token = `${record.sequence}:${record.at_ms}`;
                if (token === root.pulseToken) return;
                root.pulseToken = token;
                const ageMs = Date.now() - Number(record.at_ms);
                if (Number(record.sequence) > 0 && ageMs >= 0 && ageMs < 5000)
                    root.notePulse();
            } catch (error) {
                console.warn("[fingerprint-preview] invalid FPC pulse record:", error);
            }
        }
    }

    Timer {
        id: holdTimer
        interval: 50
        repeat: true
        onTriggered: {
            const elapsed = Date.now() - root.holdStartedAt;
            root.holdProgress = Math.min(1, elapsed / Math.max(1, root.holdMs));
            if (root.holdProgress >= 1) root.confirmHold();
        }
    }

    Timer {
        id: confirmTimer
        interval: 3500
        onTriggered: {
            root.confirmed = false;
            root.confirmedPurpose = "";
        }
    }

    Timer {
        id: pulseTimer
        interval: 3500
        onTriggered: root.pulseSeen = false
    }
}
