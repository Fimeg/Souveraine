// Ambient "the agent replied" card. While the session is locked, personal-tier
// agent output is redacted in the chat (Ai.qml). This surface announces that a
// reply landed — one-line preview only, never the body — so the lock screen
// behaves like a notification: you see that she answered, and the full text
// restores in the chat on unlock. See SESSION-TRUST-ARCHITECTURE.md (ambient
// output is safe on the lock surface; personal is not).
import QtQuick
import QtQuick.Layouts
import qs
import qs.services

Item {
    id: root

    property var previews: []

    implicitHeight: card.visible ? card.implicitHeight : 0
    visible: previews.length > 0

    function refresh() {
        root.previews = Ai.redactedPreviews();
    }

    Connections {
        target: Ai
        function onRedactedMessagesChanged() { root.refresh() }
    }
    Connections {
        target: GlobalStates
        // Repopulate on lock, clear on unlock (the chat takes over).
        function onScreenLockedChanged() {
            if (GlobalStates.screenLocked) root.refresh();
            else root.previews = [];
        }
    }

    Rectangle {
        id: card
        anchors.fill: parent
        visible: root.previews.length > 0
        radius: 16
        color: "#22000000"
        border.color: "#33ffffff"
        border.width: 1
        implicitHeight: layout.implicitHeight + 24

        ColumnLayout {
            id: layout
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.margins: 12
            spacing: 4

            Text {
                Layout.fillWidth: true
                text: qsTr("Souveraine replied while locked")
                color: "#ccffffff"
                font.pixelSize: 12
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }

            Repeater {
                model: root.previews.slice(0, 3)
                delegate: Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: "• " + modelData
                    color: "#e6ffffff"
                    font.pixelSize: 14
                    elide: Text.ElideRight
                    maximumLineCount: 1
                }
            }
        }
    }
}
