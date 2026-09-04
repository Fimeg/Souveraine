// Ambient "notifications arrived while locked" card. Same posture as
// LockAgentCard: the lock surface announces that something landed, the
// content waits for unlock. App identity and count are ambient; the summary
// line is personal and renders only when promoted
// (lock.content.notificationContentAmbient). Bodies never render here.
import QtQuick
import QtQuick.Layouts
import qs
import qs.services

Item {
    id: root

    // Newest-first, only what arrived during this lock. Transient
    // notifications are excluded — they asked not to persist.
    property var entries: []

    implicitHeight: card.visible ? card.implicitHeight : 0
    visible: LockContentPolicy.notificationsVisible && entries.length > 0

    Connections {
        target: NotifyEvents
        function onLanded(evt) {
            if (!GlobalStates.screenLocked || evt.isTransient) return;
            root.entries = [evt, ...root.entries].slice(0, 16);
        }
    }
    Connections {
        target: GlobalStates
        // Fresh slate each lock cycle; once unlocked the shell's own
        // surfaces own notification history.
        function onScreenLockedChanged() { root.entries = []; }
    }

    // Grouped by app, newest app first: [{ app, count, latest }]
    readonly property var groups: {
        const byApp = {};
        const order = [];
        for (const e of entries) {
            const key = e.app || qsTr("App");
            if (!byApp[key]) {
                byApp[key] = { app: key, count: 0, latest: e };
                order.push(key);
            }
            byApp[key].count++;
        }
        return order.map(k => byApp[k]);
    }

    Rectangle {
        id: card
        anchors.fill: parent
        visible: root.entries.length > 0
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
                text: qsTr("Arrived while locked")
                color: "#ccffffff"
                font.pixelSize: 12
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }

            Repeater {
                model: root.groups.slice(0, 4)
                delegate: ColumnLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 1

                    Text {
                        Layout.fillWidth: true
                        text: modelData.count > 1
                            ? modelData.app + " · " + modelData.count
                            : modelData.app
                        color: "#e6ffffff"
                        font.pixelSize: 14
                        elide: Text.ElideRight
                        maximumLineCount: 1
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: LockContentPolicy.notificationContentVisible
                            && (modelData.latest.summary ?? "").length > 0
                        text: "• " + modelData.latest.summary
                        color: "#b3ffffff"
                        font.pixelSize: 13
                        elide: Text.ElideRight
                        maximumLineCount: 1
                    }
                }
            }

            Text {
                visible: root.groups.length > 4
                text: qsTr("+%1 more").arg(root.groups.length - 4)
                color: "#99ffffff"
                font.pixelSize: 12
            }
        }
    }
}
