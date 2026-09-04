// Souveraine addition: touch-first lock surface for the phone.
//
// A PIN keypad instead of ii's keyboard-driven LockSurface — on the phone
// the OSK is a layershell surface and can never appear above the session
// lock, so the lock surface must carry its own input. Auth goes through the
// same LockContext/PAM machinery as the desktop surface; hardware keyboards
// still work via the Keys handlers. Ambient glance content is supplied by the
// Souveraine-owned LockSurfaceHost; ii Background remains only a temporary
// compatibility layer while the shell is progressively brought in-tree.
//
// Sized for 540x1080 logical (1080x2160 @ scale 2).
import QtQuick
import QtQuick.Layouts
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import qs.modules.common.panels.lock
import qs.modules.souveraine.lock
import Quickshell
import Quickshell.Services.UPower

MouseArea {
    id: root
    required property LockContext context
    readonly property bool requirePasswordToPower: Config.options.lock.security.requirePasswordToPower
    readonly property bool allowPowerFromLock: Config.options.lock.security.allowPowerFromLock

    readonly property int keySize: 96
    readonly property int keySpacing: 18

    // Two-stage lock: glance (ambient cards + swipe hint) and PIN. The pad
    // is not always up — a swipe up (or any hardware key) reveals it, and it
    // retreats after sitting idle with nothing typed. Credentials machinery
    // is untouched; this is purely which stage is presented.
    property bool pinRevealed: false
    property real pressY: 0
    // Live drag: the pad follows the finger during the swipe instead of
    // snapping at a threshold. dragOffset is px of upward travel; release
    // past commitDistance commits the reveal, anything less springs back.
    property bool dragging: false
    property real dragOffset: 0
    readonly property real commitDistance: 120

    // Lock wallpaper. The session-lock surface is transparent and nothing
    // else paints behind this MouseArea, so the surface owns its own
    // backdrop: the lock's pinned wallpaper when set, else the system
    // wallpaper, else a plain dark field. The scrim keeps the glance text
    // readable over any image.
    Rectangle {
        anchors.fill: parent
        z: -3
        color: "#0b0d10"
    }
    Image {
        anchors.fill: parent
        z: -2
        source: {
            const p = Config.options.lock.wallpaperPath
                || Config.options.background.wallpaperPath || "";
            return p ? "file://" + p : "";
        }
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        visible: status === Image.Ready
    }
    Rectangle {
        anchors.fill: parent
        z: -1
        color: "#000000"
        opacity: 0.32
    }

    function forceFieldFocus() {
        root.forceActiveFocus();
    }
    // Note: shouldReFocus does NOT reveal the pad — hypridle's after_sleep_cmd
    // fires it on every wake to fix Hyprland's keyboard-focus loss, and wake
    // must land on glance, not the keypad. Typing reveals via Keys below.
    Connections {
        target: root.context
        function onShouldReFocus() {
            forceFieldFocus();
        }
    }
    Component.onCompleted: forceFieldFocus()
    onPressed: mouse => {
        root.pressY = mouse.y;
        forceFieldFocus();
    }
    onPositionChanged: mouse => {
        if (!root.pinRevealed) {
            const d = root.pressY - mouse.y;
            if (d > 8) {
                root.dragging = true;
                root.dragOffset = Math.max(0, d);
            }
        } else if (mouse.y - root.pressY > 80
                 && root.context.currentText.length === 0
                 && !root.context.unlockInProgress) {
            root.pinRevealed = false;
        }
    }
    onReleased: {
        // Order matters: dragging must drop first so the settle (up on
        // commit, back down on abort) animates from the finger's position.
        const commit = !root.pinRevealed && root.dragOffset > root.commitDistance;
        root.dragging = false;
        if (commit) root.pinRevealed = true;
        root.dragOffset = 0;
    }
    onCanceled: {
        root.dragging = false;
        root.dragOffset = 0;
    }

    // Retreat to glance when the pad sits unused and empty.
    Timer {
        interval: 25000
        running: root.pinRevealed && root.context.currentText.length === 0
                 && !root.context.unlockInProgress
        onTriggered: root.pinRevealed = false
    }

    // Souveraine-owned ambient content. Credentials stay below this host, so
    // phone, laptop, and desktop can rearrange the same cards later without
    // inventing separate lock/session behavior.
    LockSurfaceHost {
        anchors {
            top: parent.top
            topMargin: 56
            horizontalCenter: parent.horizontalCenter
        }
        width: Math.max(0, parent.width - 40)
        z: 1
    }

    function pressDigit(d) {
        root.context.resetClearTimer();
        root.context.currentText += d;
    }

    // Hardware keyboard entry (USB-C keyboard); mirrors the desktop surface.
    // Any key is also a reveal gesture, so typing works from glance.
    focus: true
    Keys.onPressed: event => {
        root.pinRevealed = true;
        root.context.resetClearTimer();
        if (event.key === Qt.Key_Backspace) {
            root.context.currentText = (event.modifiers & Qt.ControlModifier)
                ? "" : root.context.currentText.slice(0, -1);
        } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            if (root.context.currentText.length > 0) root.context.tryUnlock();
        } else if (event.key === Qt.Key_Escape) {
            root.context.currentText = "";
        } else if (event.text.length === 1 && event.text >= " ") {
            root.context.currentText += event.text;
        }
    }

    // Glance-stage hint; tapping it is an alternative to the swipe.
    ColumnLayout {
        id: swipeHint
        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: parent.bottom
            bottomMargin: 64
        }
        z: 2
        spacing: 2
        visible: opacity > 0
        opacity: root.pinRevealed ? 0
            : 1 - Math.min(1, root.dragOffset / root.commitDistance)
        Behavior on opacity {
            enabled: !root.dragging
            NumberAnimation { duration: 180 }
        }

        MaterialSymbol {
            Layout.alignment: Qt.AlignHCenter
            text: "keyboard_arrow_up"
            iconSize: 34
            color: Appearance.colors.colOnSurfaceVariant
        }
        StyledText {
            Layout.alignment: Qt.AlignHCenter
            text: Translation.tr("Swipe up to unlock")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.normal
        }

        TapHandler {
            onTapped: root.pinRevealed = true
        }

        Item {
            id: fingerprintPreview
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 18
            visible: root.context.provisionalFingerprintEnabled
            implicitWidth: 196
            implicitHeight: 78

            Rectangle {
                anchors.fill: parent
                radius: Appearance.rounding.normal
                color: root.context.provisionalFingerprintConfirmed
                    ? Appearance.colors.colPrimary
                    : "#2a000000"
                border.width: root.context.provisionalFingerprintPulseSeen ? 2 : 1
                border.color: root.context.provisionalFingerprintPulseSeen
                    ? Appearance.colors.colPrimary : "#66ffffff"
            }

            Rectangle {
                anchors.left: parent.left
                anchors.bottom: parent.bottom
                width: parent.width * root.context.provisionalFingerprintHoldProgress
                height: 3
                radius: 2
                color: Appearance.colors.colPrimary
            }

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 2

                MaterialSymbol {
                    Layout.alignment: Qt.AlignHCenter
                    text: "fingerprint"
                    iconSize: 30
                    color: root.context.provisionalFingerprintConfirmed
                        ? Appearance.colors.colOnPrimary : Appearance.colors.colOnLayer1
                }
                StyledText {
                    Layout.alignment: Qt.AlignHCenter
                    text: root.context.provisionalFingerprintConfirmed
                        ? Translation.tr("Hold recorded — PIN still required")
                        : root.context.provisionalFingerprintPulseSeen
                            ? Translation.tr("Sensor pulse received — hold to confirm")
                            : Translation.tr("Hold %1 seconds to test fingerprint wiring")
                                .arg(Math.round(root.context.provisionalFingerprintHoldMs / 1000))
                    color: root.context.provisionalFingerprintConfirmed
                        ? Appearance.colors.colOnPrimary : Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                    horizontalAlignment: Text.AlignHCenter
                }
            }

            MouseArea {
                anchors.fill: parent
                enabled: root.context.provisionalFingerprintEnabled
                preventStealing: true
                pressAndHoldInterval: root.context.provisionalFingerprintHoldMs
                onPressed: {
                    root.context.beginProvisionalFingerprintHold();
                    mouse.accepted = true;
                }
                onReleased: root.context.cancelProvisionalFingerprintHold()
                onCanceled: root.context.cancelProvisionalFingerprintHold()
                onPressAndHold: root.context.confirmProvisionalFingerprintHold()
            }
        }
    }

    ColumnLayout {
        id: padColumn
        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: parent.bottom
            bottomMargin: 48
        }
        z: 2
        spacing: 20
        visible: opacity > 0
        opacity: root.pinRevealed ? 1
            : Math.min(1, root.dragOffset / root.commitDistance)
        Behavior on opacity {
            enabled: !root.dragging
            NumberAnimation { duration: 200 }
        }
        // Slides with the finger during the drag; on release the Behavior
        // takes over and settles it (fully up on commit, back down on abort).
        transform: Translate {
            y: root.pinRevealed ? 0
                : Math.max(0, (padColumn.height + 48) - root.dragOffset)
            Behavior on y {
                enabled: !root.dragging
                NumberAnimation {
                    duration: 260
                    easing.type: Easing.OutCubic
                }
            }
        }

        // Entered-PIN dots, with the empty-state hint behind them.
        // In a ColumnLayout so ErrorShakeAnimation's Layout.leftMargin works.
        Item {
            id: dotsArea
            Layout.alignment: Qt.AlignHCenter
            implicitWidth: Math.max(dotsRow.width, hintText.width, 1)
            implicitHeight: 36
            opacity: root.context.unlockInProgress ? 0.5 : 1

            Row {
                id: dotsRow
                anchors.centerIn: parent
                spacing: 13
                Repeater {
                    model: Math.min(root.context.currentText.length, 14)
                    Rectangle {
                        width: 13
                        height: 13
                        radius: 6.5
                        color: GlobalStates.screenUnlockFailed
                            ? Appearance.colors.colError : Appearance.colors.colOnLayer1
                    }
                }
            }
            StyledText {
                id: hintText
                anchors.centerIn: parent
                visible: root.context.currentText.length === 0
                text: GlobalStates.screenUnlockFailed
                    ? Translation.tr("Incorrect PIN") : Translation.tr("Enter PIN")
                color: GlobalStates.screenUnlockFailed
                    ? Appearance.colors.colError : Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.normal
            }

            ErrorShakeAnimation {
                id: wrongPinShakeAnim
                target: dotsArea
            }
            Connections {
                target: GlobalStates
                function onScreenUnlockFailedChanged() {
                    if (GlobalStates.screenUnlockFailed) wrongPinShakeAnim.restart();
                }
            }
        }

        GridLayout {
            Layout.alignment: Qt.AlignHCenter
            columns: 3
            columnSpacing: root.keySpacing
            rowSpacing: root.keySpacing

            Repeater {
                model: ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
                KeypadButton {
                    id: digitKey
                    required property string modelData
                    onClicked: root.pressDigit(digitKey.modelData)
                    contentItem: StyledText {
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        text: digitKey.modelData
                        font.pixelSize: 30
                        color: Appearance.colors.colOnLayer1
                    }
                }
            }

            KeypadButton {
                enabled: root.context.currentText.length > 0 && !root.context.unlockInProgress
                colBackground: "transparent"
                onClicked: {
                    root.context.resetClearTimer();
                    root.context.currentText = root.context.currentText.slice(0, -1);
                }
                onPressAndHold: root.context.currentText = ""
                contentItem: KeypadIcon {
                    text: "backspace"
                    color: Appearance.colors.colOnLayer1
                }
            }

            KeypadButton {
                onClicked: root.pressDigit("0")
                contentItem: StyledText {
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    text: "0"
                    font.pixelSize: 30
                    color: Appearance.colors.colOnLayer1
                }
            }

            KeypadButton {
                id: confirmKey
                enabled: root.context.currentText.length > 0 && !root.context.unlockInProgress
                toggled: true
                onClicked: root.context.tryUnlock()
                contentItem: KeypadIcon {
                    text: "arrow_right_alt"
                    color: confirmKey.enabled ? Appearance.colors.colOnPrimary : Appearance.colors.colSubtext
                }
            }
        }

        // Utility row: power / battery / reboot, kept small and away from digits
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 6
            spacing: 36

            UtilityButton {
                visible: root.allowPowerFromLock
                iconName: "power_settings_new"
                targetAction: LockContext.ActionEnum.Poweroff
            }

            RowLayout {
                spacing: 5
                visible: Battery.available
                MaterialSymbol {
                    fill: 1
                    text: Battery.isCharging ? "bolt" : "battery_android_full"
                    iconSize: Appearance.font.pixelSize.huge
                    color: (Battery.isLow && !Battery.isCharging)
                        ? Appearance.colors.colError : Appearance.colors.colOnSurfaceVariant
                }
                StyledText {
                    text: Math.round(Battery.percentage * 100) + "%"
                    color: Appearance.colors.colOnSurfaceVariant
                }
            }

            UtilityButton {
                visible: root.allowPowerFromLock
                iconName: "restart_alt"
                targetAction: LockContext.ActionEnum.Reboot
            }
        }
    }

    component KeypadButton: RippleButton {
        implicitWidth: root.keySize
        implicitHeight: root.keySize
        buttonRadius: root.keySize / 2
        enabled: !root.context.unlockInProgress
        colBackground: ColorUtils.transparentize(Appearance.colors.colLayer1, 0.4)
    }

    component KeypadIcon: MaterialSymbol {
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        iconSize: 30
    }

    // Same semantics as the desktop surface's password-guarded power buttons:
    // with requirePasswordToPower the action arms and the PIN confirms it,
    // otherwise it fires immediately.
    component UtilityButton: RippleButton {
        id: utilBtn
        required property string iconName
        required property var targetAction

        implicitWidth: 56
        implicitHeight: 56
        buttonRadius: 28
        toggled: root.context.targetAction === utilBtn.targetAction
        colBackground: "transparent"

        onClicked: {
            if (!root.requirePasswordToPower) {
                root.context.unlocked(utilBtn.targetAction);
                return;
            }
            if (root.context.targetAction === utilBtn.targetAction) {
                root.context.resetTargetAction();
            } else {
                root.context.targetAction = utilBtn.targetAction;
                root.context.shouldReFocus();
            }
        }

        contentItem: KeypadIcon {
            text: utilBtn.iconName
            color: utilBtn.toggled ? Appearance.colors.colOnPrimary : Appearance.colors.colOnSurfaceVariant
        }
    }
}
