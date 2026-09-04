import qs
import qs.services
import qs.modules.common
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Pam

Scope {
    id: root

    enum ActionEnum { Unlock, Poweroff, Reboot }

    signal shouldReFocus()
    signal unlocked(targetAction: var)
    signal failed()

    // These properties are in the context and not individual lock surfaces
    // so all surfaces can share the same state.
    property string currentText: ""
    property bool unlockInProgress: false
    property bool showFailure: false
    property bool fingerprintsConfigured: false
    property var targetAction: LockContext.ActionEnum.Unlock
    property bool alsoInhibitIdle: false

    // FingerprintPreview owns the FPC pulse and hold state for both lock and
    // step-up. This context only exposes it to the lock surface; it never
    // decides whether a hold unlocks the session.
    readonly property bool provisionalFingerprintEnabled:
        FingerprintPreview.previewEnabled
    readonly property int provisionalFingerprintHoldMs:
        FingerprintPreview.holdMs
    readonly property bool provisionalFingerprintHolding:
        FingerprintPreview.holding && FingerprintPreview.activePurpose === "lock"
    readonly property bool provisionalFingerprintConfirmed:
        FingerprintPreview.confirmed && FingerprintPreview.confirmedPurpose === "lock"
    readonly property bool provisionalFingerprintPulseSeen: FingerprintPreview.pulseSeen
    readonly property real provisionalFingerprintHoldProgress:
        FingerprintPreview.activePurpose === "lock" ? FingerprintPreview.holdProgress : 0

    function resetTargetAction() {
        root.targetAction = LockContext.ActionEnum.Unlock;
    }

    function clearText() {
        root.currentText = "";
    }

    function resetClearTimer() {
        passwordClearTimer.restart();
    }

    function reset() {
        root.resetTargetAction();
        root.clearText();
        root.unlockInProgress = false;
        stopFingerPam();
        root.resetProvisionalFingerprint();
    }

    function beginProvisionalFingerprintHold() {
        FingerprintPreview.beginHold("lock");
    }

    function cancelProvisionalFingerprintHold() {
        FingerprintPreview.cancelHold("lock");
    }

    function confirmProvisionalFingerprintHold() {
        FingerprintPreview.confirmHold("lock");
    }

    // Called by the diagnostic `fingerprint.signal` IPC seam. It reaches the
    // same one-owner state as the root-owned FPC producer record.
    function noteProvisionalFingerprintPulse() {
        return FingerprintPreview.notePulse();
    }

    function resetProvisionalFingerprint() {
        FingerprintPreview.reset("lock");
    }

    Timer {
        id: passwordClearTimer
        interval: 10000
        onTriggered: {
            root.reset();
        }
    }

    onCurrentTextChanged: {
        if (currentText.length > 0) {
            showFailure = false;
            GlobalStates.screenUnlockFailed = false;
        }
        GlobalStates.screenLockContainsCharacters = currentText.length > 0;
        passwordClearTimer.restart();
    }

    function tryUnlock(alsoInhibitIdle = false) {
        root.alsoInhibitIdle = alsoInhibitIdle;
        root.unlockInProgress = true;
        pam.start();
    }

    function tryFingerUnlock() {
        if (root.fingerprintsConfigured) {
            fingerPam.start();
        }
    }

    function stopFingerPam() {
        if (fingerPam.active) {
            fingerPam.abort();
        }
    }

    Process {
        id: fingerprintCheckProc
        running: true
        command: ["bash", "-c", "fprintd-list $(whoami)"]
        stdout: StdioCollector {
            id: fingerprintOutputCollector
            onStreamFinished: {
                root.fingerprintsConfigured = fingerprintOutputCollector.text.includes("Fingerprints for user");
            }
        }
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0) {
                // console.warn("[LockContext] fprintd-list command exited with error:", exitCode, exitStatus);
                root.fingerprintsConfigured = false;
            }
        }
    }
    
    PamContext {
        id: pam

        // pam_unix will ask for a response for the password prompt
        onPamMessage: {
            if (this.responseRequired) {
                this.respond(root.currentText);
            }
        }

        // pam_unix won't send any important messages so all we need is the completion status.
        onCompleted: result => {
            if (result == PamResult.Success) {
                root.unlocked(root.targetAction);
                stopFingerPam();
            } else {
                root.clearText();
                root.unlockInProgress = false;
                GlobalStates.screenUnlockFailed = true;
                root.showFailure = true;
            }
        }
    }

    PamContext {
        id: fingerPam

        configDirectory: "pam"
        config: "fprintd.conf"

        onCompleted: result => {
            if (result == PamResult.Success) {
                root.unlocked(root.targetAction);
                stopFingerPam();
            } else if (result == PamResult.Error) { // if timeout or etc..
                tryFingerUnlock()
            }
        }
    }
}
