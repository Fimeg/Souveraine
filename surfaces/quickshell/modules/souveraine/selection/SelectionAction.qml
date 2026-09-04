// One row in the selection menu.
//
// A disabled row still renders and still explains itself — the same choice
// RadialDial makes with its `reason` in the middle of the ring. A greyed-out
// entry teaches nothing; a row that says "Needs the side-shoot seam in Ai.qml"
// tells you what is missing.
import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root

    property string icon: "radio_button_unchecked"
    property string label: ""
    property string sublabel: ""
    /** LockContentPolicy tier this action's effect belongs to. */
    property string tier: LockContentPolicy.ambient
    /** Why this is unavailable. Shown in place of the sublabel when disabled. */
    property string reason: ""

    // `enabled` is the Item property; a disabled row is visible but inert.
    signal triggered()

    Layout.fillWidth: true
    implicitHeight: root.sublabel.length > 0 || (!root.enabled && root.reason.length > 0)
        ? 56 : 44

    MouseArea {
        anchors.fill: parent
        enabled: root.enabled
        onClicked: {
            Haptics.trigger("button-pressed");
            root.triggered();
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 14
        anchors.rightMargin: 14
        spacing: 12

        MaterialSymbol {
            text: root.icon
            iconSize: 20
            opacity: root.enabled ? 1.0 : 0.4
            color: Appearance.colors.colOnLayer1
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 1

            StyledText {
                Layout.fillWidth: true
                text: root.label
                elide: Text.ElideRight
                opacity: root.enabled ? 1.0 : 0.4
                color: Appearance.colors.colOnLayer1
                font.pixelSize: Appearance.font.pixelSize.small
            }

            StyledText {
                Layout.fillWidth: true
                visible: text.length > 0
                // The refusal replaces the description: when a row cannot act,
                // what it would have done is less useful than why it cannot.
                text: root.enabled ? root.sublabel : root.reason
                elide: Text.ElideRight
                wrapMode: Text.NoWrap
                color: Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.smaller
            }
        }
    }
}
