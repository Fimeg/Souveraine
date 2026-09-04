// Phone Polkit content with a first-class FPC1020 temporary factor.
//
// The PAM module owns acceptance.  This surface only recognizes its prompt,
// waits for a pulse that came from blueline-fingerprintd (not generic wake
// input), and submits the blank PAM response after the visible confirmation
// interval.  The normal PIN/password conversation stays intact as fallback.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Widgets
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root

    readonly property bool usePasswordChars: !PolkitService.flow?.responseVisible ?? true
    readonly property bool fpcPrompt:
        FingerprintPreview.polkitEnabled
        && PolkitService.cleanPrompt === "Touch and hold the fingerprint reader"

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Escape)
            PolkitService.cancel();
    }

    function submitPassword() {
        PolkitService.submit(inputField.text);
    }

    function usePinInstead() {
        // An empty response tells pam_souveraine_fpc to decline. PAM then
        // reaches the ordinary system-auth conversation, where this same
        // window shows the normal password field.
        PolkitService.submit("");
    }

    Connections {
        target: PolkitService
        function onInteractionAvailableChanged() {
            if (!PolkitService.interactionAvailable || root.fpcPrompt)
                return;
            inputField.text = "";
            inputField.forceActiveFocus();
        }
    }

    Connections {
        target: FingerprintPreview
        function onPulseObserved() {
            if (PolkitService.active && PolkitService.interactionAvailable && root.fpcPrompt)
                FingerprintPreview.beginHold("polkit");
        }
        function onHoldConfirmed(purpose) {
            if (purpose === "polkit" && root.fpcPrompt && PolkitService.interactionAvailable)
                PolkitService.submit("");
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Appearance.colors.colScrim
        opacity: 0
        Component.onCompleted: opacity = 1
        Behavior on opacity {
            animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
        }
    }

    WindowDialog {
        anchors.centerIn: parent
        backgroundWidth: 450
        show: false
        Component.onCompleted: show = true

        MaterialSymbol {
            Layout.alignment: Qt.AlignHCenter
            iconSize: 26
            text: root.fpcPrompt ? "fingerprint" : "security"
            color: Appearance.colors.colSecondary
        }

        WindowDialogTitle {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: Translation.tr("Authentication")
        }

        WindowDialogParagraph {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignLeft
            text: PolkitService.cleanMessage
        }

        Item {
            Layout.fillWidth: true
            visible: root.fpcPrompt
            implicitHeight: visible ? 104 : 0

            Rectangle {
                anchors.fill: parent
                radius: Appearance.rounding.normal
                color: FingerprintPreview.confirmedPurpose === "polkit"
                    ? Appearance.colors.colPrimary : "#2a000000"
                border.width: FingerprintPreview.pulseSeen ? 2 : 1
                border.color: FingerprintPreview.pulseSeen
                    ? Appearance.colors.colPrimary : "#66ffffff"
            }

            Rectangle {
                anchors.left: parent.left
                anchors.bottom: parent.bottom
                width: parent.width * (FingerprintPreview.activePurpose === "polkit"
                    ? FingerprintPreview.holdProgress : 0)
                height: 3
                radius: 2
                color: Appearance.colors.colPrimary
            }

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 4

                MaterialSymbol {
                    Layout.alignment: Qt.AlignHCenter
                    text: "fingerprint"
                    iconSize: 32
                    color: FingerprintPreview.confirmedPurpose === "polkit"
                        ? Appearance.colors.colOnPrimary : Appearance.colors.colOnLayer1
                }
                StyledText {
                    Layout.alignment: Qt.AlignHCenter
                    text: FingerprintPreview.activePurpose === "polkit"
                        ? Translation.tr("Reader contact received — confirming")
                        : Translation.tr("Touch and hold the fingerprint reader")
                    color: FingerprintPreview.confirmedPurpose === "polkit"
                        ? Appearance.colors.colOnPrimary : Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
                StyledText {
                    Layout.alignment: Qt.AlignHCenter
                    visible: FingerprintPreview.activePurpose === "polkit"
                    text: Translation.tr("%1 seconds").arg(Math.round(FingerprintPreview.holdMs / 1000))
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
            }
        }

        MaterialTextField {
            id: inputField
            Layout.fillWidth: true
            visible: !root.fpcPrompt
            focus: visible
            enabled: PolkitService.interactionAvailable
            placeholderText: PolkitService.cleanPrompt
            echoMode: root.usePasswordChars ? TextInput.Password : TextInput.Normal
            onAccepted: root.submitPassword()

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Escape)
                    PolkitService.cancel();
            }
        }

        WindowDialogButtonRow {
            Layout.bottomMargin: 10
            Item { Layout.fillWidth: true }
            DialogButton {
                buttonText: Translation.tr("Cancel")
                onClicked: PolkitService.cancel()
            }
            DialogButton {
                visible: root.fpcPrompt
                enabled: PolkitService.interactionAvailable
                buttonText: Translation.tr("Use PIN instead")
                onClicked: root.usePinInstead()
            }
            DialogButton {
                visible: !root.fpcPrompt
                enabled: PolkitService.interactionAvailable
                buttonText: Translation.tr("OK")
                onClicked: root.submitPassword()
            }
        }
    }

    onFpcPromptChanged: {
        if (!fpcPrompt)
            FingerprintPreview.reset("polkit");
    }
}
