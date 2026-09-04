import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "volume_up"
        title: Translation.tr("Output")

        StyledText {
            Layout.fillWidth: true
            text: Audio.sink ? Audio.friendlyDeviceName(Audio.sink)
                : Translation.tr("No audio output")
            color: Appearance.colors.colSubtext
            elide: Text.ElideRight
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            RippleButton {
                implicitWidth: 44
                implicitHeight: 44
                enabled: Audio.sink ? Audio.sink.ready : false
                buttonRadius: Appearance.rounding.full
                onClicked: Audio.toggleMute()
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    text: Audio.sink && Audio.sink.audio.muted ? "volume_off" : "volume_up"
                    iconSize: 22
                }
            }
            StyledSlider {
                Layout.fillWidth: true
                enabled: Audio.sink ? Audio.sink.ready : false
                value: Audio.sink ? Audio.sink.audio.volume : 0
                from: 0
                to: 1
                onMoved: {
                    if (Audio.sink) Audio.sink.audio.volume = value;
                }
            }
        }
    }

    ContentSection {
        icon: "mic"
        title: Translation.tr("Microphone")

        StyledText {
            Layout.fillWidth: true
            text: Audio.source ? Audio.friendlyDeviceName(Audio.source)
                : Translation.tr("No microphone")
            color: Appearance.colors.colSubtext
            elide: Text.ElideRight
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            RippleButton {
                implicitWidth: 44
                implicitHeight: 44
                enabled: Audio.source ? Audio.source.ready : false
                buttonRadius: Appearance.rounding.full
                onClicked: Audio.toggleMicMute()
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    text: Audio.source && Audio.source.audio.muted ? "mic_off" : "mic"
                    iconSize: 22
                }
                StyledToolTip {
                    text: Audio.source && Audio.source.audio.muted
                        ? Translation.tr("Unmute microphone") : Translation.tr("Mute microphone")
                }
            }
            StyledSlider {
                Layout.fillWidth: true
                enabled: Audio.source ? Audio.source.ready : false
                value: Audio.source ? Audio.source.audio.volume : 0
                from: 0
                to: 1
                onMoved: {
                    if (Audio.source) Audio.source.audio.volume = value;
                }
            }
        }
    }
}
