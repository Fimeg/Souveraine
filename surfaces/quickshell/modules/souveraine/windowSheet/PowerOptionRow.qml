// One nested choice in the held-power-button sheet.
//
// `available` is deliberately separate from Item.enabled. An unavailable
// option still opens when it owns children, so the hand can see the shape of
// the mode being built; a leaf that cannot act simply has no click target and
// wears its status instead of pretending a tap did something.
import QtQuick
import QtQuick.Layouts
import qs.modules.common
import qs.modules.common.widgets

Rectangle {
    id: root

    property string icon: ""
    property string title: ""
    property string detail: ""
    property string badge: ""
    property bool expandable: false
    property bool expanded: false
    property bool available: false
    property int inset: 0

    signal tapped

    implicitHeight: detail === "" ? 58 : 70
    radius: 18
    color: hit.pressed
        ? Appearance.colors.colLayer2
        : Appearance.colors.colLayer1
    border.width: 1
    border.color: Appearance.colors.colLayer1Active
    opacity: available || expandable ? 1.0 : 0.72

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 16 + root.inset
        anchors.rightMargin: 16
        spacing: 13

        MaterialSymbol {
            text: root.icon
            iconSize: 27
            color: Appearance.colors.colOnLayer1
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            StyledText {
                Layout.fillWidth: true
                text: root.title
                color: Appearance.colors.colOnLayer1
                font.pixelSize: Appearance.font.pixelSize.normal
                elide: Text.ElideRight
            }

            StyledText {
                Layout.fillWidth: true
                visible: root.detail !== ""
                text: root.detail
                color: "#aaffffff"
                font.pixelSize: Appearance.font.pixelSize.small
                elide: Text.ElideRight
            }
        }

        Rectangle {
            visible: root.badge !== ""
            implicitWidth: badgeText.implicitWidth + 18
            implicitHeight: 26
            radius: 13
            color: Appearance.colors.colLayer2

            StyledText {
                id: badgeText
                anchors.centerIn: parent
                text: root.badge
                color: "#c8ffffff"
                font.pixelSize: Appearance.font.pixelSize.smaller
            }
        }

        MaterialSymbol {
            visible: root.expandable
            text: root.expanded ? "expand_less" : "expand_more"
            iconSize: 25
            color: Appearance.colors.colOnLayer1
        }
    }

    MouseArea {
        id: hit
        anchors.fill: parent
        enabled: root.available || root.expandable
        onClicked: root.tapped()
    }
}
