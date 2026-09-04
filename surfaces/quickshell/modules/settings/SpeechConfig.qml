import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Speech services — dictation (STT) and speech synthesis (TTS).
// The endpoints live in Config so the souveraine-stt CLI (and the
// keyboard's mic key that shells out to it) read exactly what's set
// here. The probe hits the STT /health route so "it's configured" and
// "it's answering" are visibly different states.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "mic"
        title: Translation.tr("Dictation (speech to text)")

        ConfigSwitch {
            buttonIcon: "record_voice_over"
            text: Translation.tr("Enable dictation")
            checked: Config.options.speech.stt.enable
            onCheckedChanged: {
                Config.options.speech.stt.enable = checked;
            }
            StyledToolTip {
                text: Translation.tr("Voice input through the transcription server. The keyboard's mic key and the souveraine-stt command both use this.")
            }
        }

        MaterialTextField {
            Layout.fillWidth: true
            text: Config.options.speech.stt.endpoint
            placeholderText: Translation.tr("Transcription endpoint (http://host:port/transcribe)")
            onEditingFinished: {
                if (text !== Config.options.speech.stt.endpoint) {
                    Config.options.speech.stt.endpoint = text;
                    healthProbe.refresh();
                }
            }
        }

        RowLayout {
            spacing: 8

            StyledText {
                text: healthProbe.statusText
                color: healthProbe.healthy ? Appearance.colors.colOnLayer1
                                           : Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.smaller
            }

            RippleButton {
                implicitWidth: 32
                implicitHeight: 32
                buttonRadius: Appearance.rounding.full
                onClicked: healthProbe.refresh()
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    horizontalAlignment: Text.AlignHCenter
                    text: "refresh"
                    iconSize: 18
                }
                StyledToolTip {
                    text: Translation.tr("Check the server")
                }
            }
        }
    }

    ContentSection {
        icon: "text_to_speech"
        title: Translation.tr("Speech synthesis (text to speech)")

        ConfigSwitch {
            buttonIcon: "campaign"
            text: Translation.tr("Enable speech output")
            checked: Config.options.speech.tts.enable
            onCheckedChanged: {
                Config.options.speech.tts.enable = checked;
            }
            StyledToolTip {
                text: Translation.tr("Spoken responses through the synthesis server. Off until a TTS endpoint exists.")
            }
        }

        MaterialTextField {
            Layout.fillWidth: true
            text: Config.options.speech.tts.endpoint
            placeholderText: Translation.tr("Synthesis endpoint (empty = none yet)")
            onEditingFinished: {
                if (text !== Config.options.speech.tts.endpoint)
                    Config.options.speech.tts.endpoint = text;
            }
        }
    }

    // STT /health probe. Derives the health URL from the transcribe
    // endpoint (…/transcribe -> …/health) rather than storing a second URL.
    QtObject {
        id: healthProbe
        property bool healthy: false
        property string statusText: Translation.tr("Checking server…")

        function refresh() {
            statusText = Translation.tr("Checking server…");
            healthy = false;
            probeProc.running = false;
            probeProc.running = true;
        }
    }

    Process {
        id: probeProc
        running: true
        command: ["curl", "-s", "--max-time", "5",
            Config.options.speech.stt.endpoint.replace(/\/[^\/]*$/, "/health")]
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const h = JSON.parse(text);
                    healthProbe.healthy = h.status === "ok";
                    healthProbe.statusText = healthProbe.healthy
                        ? Translation.tr("Server up · %1 on %2").arg(h.model ?? "?").arg(h.device ?? "?")
                        : Translation.tr("Server answered but not ok");
                } catch (e) {
                    healthProbe.healthy = false;
                    healthProbe.statusText = Translation.tr("Server unreachable");
                }
            }
        }
    }
}
