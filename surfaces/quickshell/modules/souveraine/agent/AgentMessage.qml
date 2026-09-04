pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import qs.modules.ii.sidebarLeft.aiChat
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

/*
 * One message, drawn from typed segments.
 *
 * Owned Souveraine surface (TASK-72 step 2). This replaces the vendor
 * snapshot's AiMessage as the agent-surface delegate.
 *
 * The structural point: it consumes the segment list, never a flattened
 * markdown blob. `services/Ai.qml` stays the data boundary — it preserves wire
 * segment type and call identity and does not choose pixels; this file chooses
 * pixels and reads no state.
 *
 * Deliberately reused from the vendor snapshot: MessageTextBlock and
 * MessageCodeBlock. Those are markdown renderers, not agent vocabulary — the
 * ii-base rule is that no further *agent-surface feature* lands in the vendor
 * tree, not that we owe ourselves a second markdown engine. Reasoning and tool
 * calls are ours because those are the parts that are about her.
 *
 * Known parity gaps against the vendor delegate, stated rather than hidden:
 *   - in-place message editing (Ctrl+S save) is not carried over;
 *   - the attached-file indicator is not drawn yet — it belongs with the image
 *     intake half of TASK-72, which has no picker yet either.
 */
Rectangle {
    id: root

    property int messageIndex
    property var messageData
    property var messageInputField

    property real messagePadding: 7
    property real contentSpacing: 3
    property bool renderMarkdown: true
    property bool enableMouseSelection: false

    // Typed segments are the contract. The markdown splitter is the fallback
    // for legacy/provider messages that never carried segments at all.
    readonly property var messageBlocks: {
        const segments = root.messageData ? root.messageData.segments : [];
        return (segments && segments.length > 0)
            ? segments
            : StringUtils.splitMarkdownBlocks(root.messageData?.content);
    }

    readonly property bool isAssistant: (root.messageData?.role ?? "") === "assistant"
    readonly property bool done: root.messageData?.done ?? false

    // Delete is armed rather than immediate — see the control row.
    property bool deleteArmed: false
    onMessageIndexChanged: root.deleteArmed = false

    readonly property bool isSpeakingThis: Speech.speaking
        && Ai.speakingMessageIndex === root.messageIndex

    // What `copy` puts on the clipboard: everything the message actually says.
    readonly property string plainText: root.messageData?.content ?? ""

    // What `speak` sends to TTS. NOT the same thing, deliberately.
    //
    // The vendor spoke messageData.content verbatim, which on a segmented
    // message means the synthesizer reads reasoning and tool payloads aloud.
    // Typed segments let us say what a voice should say: prose only. Code is
    // excluded because reading a diff aloud is noise, not speech; `think` and
    // `tool` are excluded because they are not addressed to anyone.
    //
    // Falls back to the whole content when a legacy message carries no
    // segments, which is the pre-existing behaviour rather than silence.
    readonly property string spokenText: {
        const segments = root.messageData?.segments;
        if (!segments || segments.length < 1) return root.plainText;
        return segments
            .filter(s => (s?.kind ?? "text") === "text")
            .map(s => s?.text ?? "")
            .join("\n\n")
            .trim();
    }

    anchors.left: parent?.left
    anchors.right: parent?.right

    // While streaming, never let the bubble shrink: markdown reflow transiently
    // collapses height and jolts the auto-scroll. Floor it until done.
    readonly property real naturalHeight: columnLayout.implicitHeight + root.messagePadding * 2
    property real streamingHeightFloor: 0
    implicitHeight: !root.done ? Math.max(naturalHeight, streamingHeightFloor) : naturalHeight
    onNaturalHeightChanged: if (!root.done) streamingHeightFloor = Math.max(streamingHeightFloor, naturalHeight)
    onMessageDataChanged: streamingHeightFloor = 0

    radius: Appearance.rounding.normal
    color: Appearance.colors.colLayer1

    ColumnLayout {
        id: columnLayout

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: root.messagePadding
        spacing: root.contentSpacing

        Rectangle { // Header
            Layout.fillWidth: true
            implicitHeight: headerRow.implicitHeight + 8
            radius: Appearance.rounding.small
            color: Appearance.colors.colSecondaryContainer

            RowLayout {
                id: headerRow
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                spacing: 8

                MaterialSymbol {
                    text: root.isAssistant ? "auto_awesome" : "person"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnSecondaryContainer
                }

                StyledText {
                    Layout.fillWidth: true
                    text: root.messageData?.name ?? (root.isAssistant ? Translation.tr("Souveraine") : Translation.tr("You"))
                    font.pixelSize: Appearance.font.pixelSize.small
                    color: Appearance.colors.colOnSecondaryContainer
                    elide: Text.ElideRight
                }

                // ── Controls ──────────────────────────────────────────────
                // Deliberately NOT carried over from the vendor row:
                //
                //   regenerate — the conversation is forward-only. Ai.qml's
                //     regenerate() is already a no-op that returns advice.
                //     A button that only ever explains it does nothing is
                //     worse than no button. Audio re-synthesis is the real
                //     verb, and it lives on `replay` below.
                //   edit — there is no in-place edit. The vendor's saved to
                //     a local array the server never sees, so the message
                //     you read back was not the message the agent holds.
                //
                // `delete` is kept but gated, because it is a *view filter*
                // wearing a delete icon: removeMessage() splices two local
                // arrays and the server transcript is untouched. /resume
                // brings it straight back.
                ButtonGroup {
                    id: controlRow
                    spacing: 5
                    visible: !root.deleteArmed

                    AiMessageControlButton {
                        id: speakButton
                        // stop icon only while THIS message is the speaker
                        buttonIcon: root.isSpeakingThis ? "stop_circle" : "volume_up"
                        visible: Speech.enabled && root.isAssistant
                        enabled: visible

                        onClicked: {
                            if (root.isSpeakingThis) {
                                Speech.stop()
                            } else {
                                Ai.speakingMessageIndex = root.messageIndex
                                Speech.speak(StringUtils.ttsClean(root.spokenText))
                            }
                        }

                        StyledToolTip {
                            text: root.isSpeakingThis ? Translation.tr("Stop") : Translation.tr("Speak")
                        }
                    }

                    AiMessageControlButton {
                        id: respeakButton
                        buttonIcon: "replay"
                        visible: Speech.enabled && root.isAssistant
                        enabled: visible

                        onClicked: {
                            // Re-synthesize when the previous synth came out
                            // wrong. Does NOT re-run the agent.
                            //
                            // The legacy path below cannot actually do this:
                            // speak() opens with a cache check on the text,
                            // and re-synthesis is by definition the same
                            // text — so it replays the identical broken file.
                            // Speech.resynthesize() is the honest verb
                            // (cancel live job, bypass cache, re-request);
                            // until it lands we degrade rather than lie.
                            Ai.speakingMessageIndex = root.messageIndex
                            const text = StringUtils.ttsClean(root.spokenText)
                            if (typeof Speech.resynthesize === "function") {
                                Speech.resynthesize(text)
                            } else {
                                Speech.stop()
                                Speech.speak(text)
                            }
                        }

                        StyledToolTip {
                            text: Translation.tr("Re-synthesize audio")
                        }
                    }

                    AiMessageControlButton {
                        id: copyButton
                        buttonIcon: activated ? "inventory" : "content_copy"

                        onClicked: {
                            Quickshell.clipboardText = root.plainText
                            copyButton.activated = true
                            copyIconTimer.restart()
                        }

                        Timer {
                            id: copyIconTimer
                            interval: 1500
                            repeat: false
                            onTriggered: copyButton.activated = false
                        }

                        StyledToolTip {
                            text: Translation.tr("Copy")
                        }
                    }

                    AiMessageControlButton {
                        id: toggleMarkdownButton
                        activated: !root.renderMarkdown
                        buttonIcon: "code"
                        onClicked: root.renderMarkdown = !root.renderMarkdown
                        StyledToolTip {
                            text: Translation.tr("View Markdown source")
                        }
                    }

                    AiMessageControlButton {
                        id: deleteButton
                        buttonIcon: "close"
                        onClicked: root.deleteArmed = true
                        StyledToolTip {
                            text: Translation.tr("Hide from view")
                        }
                    }
                }

                // Armed state replaces the row in place rather than opening a
                // modal: this panel is a layer-shell surface and a grabbing
                // popup here fights the compositor for focus. The words are
                // the point, not the chrome.
                RowLayout {
                    id: deleteConfirmRow
                    visible: root.deleteArmed
                    spacing: 6

                    MaterialSymbol {
                        text: "visibility_off"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colOnSecondaryContainer
                    }

                    StyledText {
                        text: Translation.tr("Hides it here only — the transcript keeps it.")
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        color: Appearance.colors.colOnSecondaryContainer
                        elide: Text.ElideRight
                    }

                    DialogButton {
                        buttonText: Translation.tr("Cancel")
                        onClicked: root.deleteArmed = false
                    }

                    DialogButton {
                        buttonText: Translation.tr("Hide")
                        onClicked: {
                            root.deleteArmed = false
                            Ai.removeMessage(root.messageIndex)
                        }
                    }
                }
            }
        }

        Item { // Waiting, before any segment has arrived
            Layout.fillWidth: true
            implicitHeight: waitingLoader.shown ? waitingLoader.implicitHeight : 0
            visible: implicitHeight > 0

            Behavior on implicitHeight {
                animation: Appearance.animation.elementMove.numberAnimation.createObject(this)
            }

            FadeLoader {
                id: waitingLoader
                anchors.centerIn: parent
                shown: (root.messageBlocks.length < 1) && !root.done
                sourceComponent: MaterialLoadingIndicator {
                    loading: true
                }
            }
        }

        Repeater {
            model: ScriptModel {
                values: root.messageBlocks
            }

            delegate: DelegateChooser {
                role: "type"

                DelegateChoice {
                    roleValue: "tool"
                    ToolCard {
                        required property var modelData
                        segment: modelData
                    }
                }

                DelegateChoice {
                    roleValue: "think"
                    ThinkingCard {
                        required property var modelData
                        segmentContent: modelData.content ?? ""
                        renderMarkdown: root.renderMarkdown
                        enableMouseSelection: root.enableMouseSelection
                        done: root.done
                        completed: modelData.completed ?? root.done
                    }
                }

                DelegateChoice {
                    roleValue: "code"
                    MessageCodeBlock {
                        required property var modelData
                        renderMarkdown: root.renderMarkdown
                        enableMouseSelection: root.enableMouseSelection
                        segmentContent: modelData.content ?? ""
                        segmentLang: modelData.lang ?? ""
                        messageData: root.messageData
                    }
                }

                DelegateChoice {
                    roleValue: "text"
                    MessageTextBlock {
                        required property var modelData
                        renderMarkdown: root.renderMarkdown
                        enableMouseSelection: root.enableMouseSelection
                        segmentContent: modelData.content ?? ""
                        messageData: root.messageData
                        done: root.done
                        forceDisableChunkSplitting: root.messageData?.content?.includes("```") ?? true
                    }
                }
            }
        }

        Flow { // Annotations
            visible: (root.messageData?.annotationSources?.length ?? 0) > 0
            spacing: 5
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignLeft

            Repeater {
                model: ScriptModel {
                    values: root.messageData?.annotationSources ?? []
                }
                delegate: AnnotationSourceButton {
                    required property var modelData
                    displayText: modelData.text
                    url: modelData.url
                }
            }
        }

        Flow { // Search queries
            visible: (root.messageData?.searchQueries?.length ?? 0) > 0
            spacing: 5
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignLeft

            Repeater {
                model: ScriptModel {
                    values: root.messageData?.searchQueries ?? []
                }
                delegate: SearchQueryButton {
                    required property var modelData
                    query: modelData
                }
            }
        }
    }
}
