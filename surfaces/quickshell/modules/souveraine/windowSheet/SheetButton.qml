// One circular verb on the window action sheet.
//
// Sized against the selection chip's failure rather than against a desktop
// idea of a button: 76 px of circle and a 34 px glyph, because the chip
// shipped at 44 px with 18-20 px icons and was "completely untappable" on
// device, twice. Android's own floor for a touch target is 48 dp and that is
// a *minimum*, not a target, for a control the hand reaches for mid-gesture.
//
// The hold is optional and only Close uses it. It is a real press-and-hold
// rather than a second button because the polite and forceful versions of
// "close this" are one intent at two levels of insistence — and a separate
// always-kill button is one a hurried thumb presses meaning the safe one.
// The ring filling is the whole feedback story: a hold you only learn about
// on release cannot be abandoned halfway.
import QtQuick
import Quickshell
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root

    property string icon: ""
    property string label: ""
    property string holdLabel: ""
    // 0 disables the hold entirely, which is the default: most verbs have no
    // second level and should not appear to have one.
    property int holdMs: 0

    signal tapped
    signal held

    readonly property int diameter: 76

    implicitWidth: diameter
    implicitHeight: diameter + 34

    Rectangle {
        id: circle
        width: root.diameter
        height: root.diameter
        radius: width / 2
        anchors.horizontalCenter: parent.horizontalCenter
        color: area.pressed
            ? Appearance.colors.colLayer2
            : Appearance.colors.colLayer1
        border.width: 1
        // `colLayer1Active`, not `colLayer1Inactive` — the latter does not
        // exist in Appearance and resolves to undefined, which Qt reports as
        // "Unable to assign [undefined] to QColor" and then draws borderless.
        border.color: Appearance.colors.colLayer1Active

        scale: area.pressed ? 0.94 : 1.0
        // The house bounce, which overshoots on the way back — a flat 90 ms
        // ramp is a size change, not an acknowledgement.
        Behavior on scale {
            animation: Appearance.animation.clickBounce.numberAnimation.createObject(this)
        }
        Behavior on color {
            animation: Appearance.animation.elementMoveFast.colorAnimation.createObject(this)
        }

        MaterialSymbol {
            anchors.centerIn: parent
            text: root.icon
            iconSize: 34
            color: Appearance.colors.colOnLayer1
        }

        // The hold's progress, drawn as a ring that closes. Only appears once
        // a hold is actually running, so a tap never flashes it.
        Canvas {
            id: ring
            anchors.fill: parent
            visible: root.holdMs > 0 && hold.running
            property real progress: 0

            onProgressChanged: requestPaint()
            onPaint: {
                const ctx = getContext("2d");
                ctx.reset();
                const r = width / 2 - 2;
                ctx.beginPath();
                ctx.arc(width / 2, height / 2, r, -Math.PI / 2,
                        -Math.PI / 2 + Math.PI * 2 * ring.progress);
                ctx.lineWidth = 3;
                ctx.strokeStyle = Appearance.colors.colOnLayer1;
                ctx.stroke();
            }
        }
    }

    StyledText {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: circle.bottom
        anchors.topMargin: 8
        text: hold.running && root.holdLabel !== "" ? root.holdLabel : root.label
        color: "#e6ffffff"
        font.pixelSize: Appearance.font.pixelSize.small
    }

    // Drives the ring and, at the end, the forceful verb. Started on press and
    // stopped on release or on leaving the button, so sliding a thumb off is
    // an abort rather than a commit.
    NumberAnimation {
        id: hold
        target: ring
        property: "progress"
        from: 0
        to: 1
        duration: root.holdMs
        onFinished: {
            root.heldFired = true;
            root.held();
        }
    }

    property bool heldFired: false

    MouseArea {
        id: area
        anchors.fill: parent

        onPressed: {
            root.heldFired = false;
            if (root.holdMs > 0)
                hold.start();
        }

        onReleased: {
            if (root.holdMs > 0)
                hold.stop();
            // A hold that completed already fired the forceful verb; releasing
            // afterwards must not also fire the polite one.
            if (!root.heldFired && containsMouse)
                root.tapped();
            ring.progress = 0;
        }

        onCanceled: {
            hold.stop();
            ring.progress = 0;
        }

        hoverEnabled: true
    }
}
