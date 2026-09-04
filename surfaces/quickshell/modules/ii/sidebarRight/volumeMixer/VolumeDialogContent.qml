import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

// Native PulseAudio publishes the Pixel 3 board endpoints directly: playback
// on hw:0,0 and UCM handset capture on hw:0,1.  Present both without depending
// on PipeWire's node model.
ColumnLayout {
    id: root
    required property bool isSink
    readonly property bool outputAvailable: Audio.ready && Audio.sink?.name.length > 0
    readonly property bool inputAvailable: Audio.sourceReady && Audio.source?.name.length > 0
    readonly property var currentNode: root.isSink ? Audio.sink : Audio.source
    spacing: 16

    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: deviceRow.implicitHeight + 24
        radius: Appearance.rounding.small
        color: Appearance.colors.colLayer2

        RowLayout {
            id: deviceRow
            anchors.fill: parent
            anchors.margins: 12
            spacing: 12

            MaterialSymbol {
                text: root.isSink ? "speaker" : "mic"
                iconSize: Appearance.font.pixelSize.hugeass
                color: Appearance.colors.colOnLayer2
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2

                StyledText {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer2
                    text: root.isSink
                        ? (root.outputAvailable ? Audio.friendlyDeviceName(Audio.sink) : Translation.tr("Connecting to PulseAudio…"))
                        : (root.inputAvailable ? Audio.friendlyDeviceName(Audio.source) : Translation.tr("Connecting to PulseAudio…"))
                }

                StyledText {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    font.pixelSize: Appearance.font.pixelSize.smaller
                    color: Appearance.m3colors.m3outline
                    text: root.isSink
                        ? Translation.tr("Default output • Pixel 3 internal stereo speakers")
                        : (Audio.micActive
                            ? Translation.tr("Default input • recording in progress")
                            : Translation.tr("Default input • Pixel 3 handset microphone"))
                }
            }
        }
    }

    StyledSlider {
        Layout.fillWidth: true
        visible: root.isSink ? root.outputAvailable : root.inputAvailable
        value: root.currentNode?.audio?.volume ?? 0
        onMoved: root.currentNode.audio.volume = value
        configuration: StyledSlider.Configuration.M
    }

    StyledText {
        Layout.fillWidth: true
        visible: root.isSink ? root.outputAvailable : root.inputAvailable
        horizontalAlignment: Text.AlignHCenter
        color: Appearance.colors.colSubtext
        text: root.currentNode?.audio?.muted
            ? Translation.tr("Muted")
            : `${Math.round((root.currentNode?.audio?.volume ?? 0) * 100)}%`
    }

    Item { Layout.fillHeight: true }
}
