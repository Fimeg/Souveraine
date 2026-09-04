// Souveraine-owned ambient MPRIS card. Transport is safe while locked; title,
// artist, and artwork are personal by default and require an explicit opt-in.
import QtQuick
import QtQuick.Layouts
import Quickshell.Services.Mpris
import qs.services

Item {
    id: root

    function candidates() {
        return Mpris.players.values.filter(player =>
            player && (player.canTogglePlaying || player.canGoPrevious || player.canGoNext));
    }

    readonly property var player: {
        const players = root.candidates();
        return players.find(player => player.isPlaying) || players[0] || null;
    }
    readonly property bool hasPlayer: root.player !== null
    readonly property bool revealMetadata: LockContentPolicy.mediaMetadataVisible

    visible: LockContentPolicy.mediaControlsVisible && root.hasPlayer
    implicitHeight: visible ? card.implicitHeight : 0

    Rectangle {
        id: card
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width
        implicitHeight: content.implicitHeight + 24
        radius: 20
        color: "#b3181b20"
        border.width: 1
        border.color: "#55ffffff"

        ColumnLayout {
            id: content
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8

            Text {
                Layout.fillWidth: true
                visible: root.revealMetadata
                text: root.player?.trackTitle || "Media"
                color: "white"
                font.pixelSize: 17
                font.bold: true
                elide: Text.ElideRight
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                Layout.fillWidth: true
                visible: root.revealMetadata && (root.player?.trackArtist || "").length > 0
                text: root.player?.trackArtist || ""
                color: "#d9ffffff"
                font.pixelSize: 13
                elide: Text.ElideRight
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                Layout.fillWidth: true
                visible: !root.revealMetadata
                text: "Media controls"
                color: "#d9ffffff"
                font.pixelSize: 13
                horizontalAlignment: Text.AlignHCenter
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: 18

                TransportButton {
                    glyph: "‹‹"
                    available: root.player?.canGoPrevious ?? false
                    onClicked: root.player.previous()
                }
                TransportButton {
                    glyph: root.player?.isPlaying ? "Ⅱ" : "▶"
                    available: root.player?.canTogglePlaying ?? false
                    primary: true
                    onClicked: root.player.togglePlaying()
                }
                TransportButton {
                    glyph: "››"
                    available: root.player?.canGoNext ?? false
                    onClicked: root.player.next()
                }
            }
        }
    }

    component TransportButton: Rectangle {
        required property string glyph
        required property bool available
        property bool primary: false
        signal clicked()

        implicitWidth: primary ? 50 : 40
        implicitHeight: primary ? 50 : 40
        radius: implicitWidth / 2
        color: primary ? "#e6ffffff" : "#26ffffff"
        opacity: available ? 1 : 0.35

        Text {
            anchors.centerIn: parent
            text: parent.glyph
            color: parent.primary ? "#1c1b20" : "white"
            font.pixelSize: parent.primary ? 19 : 16
            font.bold: true
        }
        MouseArea {
            anchors.fill: parent
            enabled: parent.available
            onClicked: parent.clicked()
        }
    }
}
