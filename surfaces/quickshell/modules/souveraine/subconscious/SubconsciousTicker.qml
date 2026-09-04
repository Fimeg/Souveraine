import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import QtQuick
import QtQuick.Layouts
import Quickshell

// Tier 1 of the subconscious three-tier visibility design (see
// docs/tasks/subconscious-surfacing-threshold.md): a transient, fading line
// that shows the subconscious's live reasoning/tools while its N+1 pass runs.
// Visible only while Ai.subconsciousActive; fed by Ai.subconsciousStream;
// cleared (and snapshotted to the Tier-2 log) when the pass ends.
//
// This is Souveraine-original work, not anything inherited from the upstream
// ii shell. It lived under the ii tree only because it had not been re-homed;
// relocated into modules/souveraine/ on 2026-07-30 so it is no longer tangled
// with a shell we are exiting. The model it reads (Ai.qml, in services/) is
// already substrate-neutral and never moved.
//
// Interaction: the psychology icon is a real affordance, not decoration.
//   • tap    → expand/collapse an inline mini-log (last few entries) in place
//   • long-press → ask the host to open the Tier-2 event panel
//
// The long-press emits requestOpenPanel() rather than reaching into an overlay
// manager. Whoever mounts the panel connects that signal; the view itself owns
// no overlay state. That decoupling is the point: this component renders
// wherever a host places it — sidebar, scene element, or a future viewtop node.
Rectangle {
    id: root
    Layout.fillWidth: true
    implicitHeight: col.implicitHeight + 2 * root.padding
    radius: Appearance.rounding.small
    color: Appearance.colors.colLayer2
    visible: Ai.subconsciousActive && Ai.subconsciousStream.length > 0
    opacity: visible ? 1 : 0

    property real padding: 6
    property bool expanded: false

    /// Ask the host to open the Tier-2 subconscious event panel. The view does
    /// not know how panels are mounted; it only declares the intent. Unconnected
    /// until a souveraine overlay host exists — see SubconsciousEventPanel.qml.
    signal requestOpenPanel()

    Behavior on opacity {
        animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
    }
    Behavior on implicitHeight {
        animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
    }

    // Fader: retire the oldest line a few seconds after it arrives so the
    // ticker reads as a live stream, not an accumulating log. The full stream
    // is snapshotted to Ai.subconsciousEvents on pass end regardless.
    Timer {
        interval: 4000
        repeat: true
        running: root.visible && !root.expanded
        onTriggered: {
            if (Ai.subconsciousStream.length > 1)
                Ai.subconsciousStream = Ai.subconsciousStream.slice(1);
        }
    }

    ColumnLayout {
        id: col
        anchors.fill: parent
        anchors.margins: root.padding
        spacing: root.padding

        RowLayout {
            Layout.fillWidth: true
            spacing: root.padding

            // The icon is the affordance: tap = expand inline, long-press =
            // open the event panel. Press-and-hold beats a separate button
            // — keeps the ticker visually quiet.
            Item {
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: icon.implicitWidth
                implicitHeight: icon.implicitHeight

                MaterialSymbol {
                    id: icon
                    anchors.centerIn: parent
                    iconSize: Appearance.font.pixelSize.larger
                    text: "psychology"
                    color: Appearance.colors.colSubtext
                    NumberAnimation on opacity {
                        // gentle pulse while the pass runs
                        from: 0.5; to: 1; duration: 1200
                        running: root.visible; loops: Animation.Infinite
                        easing.type: Easing.InOutSine
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: root.expanded = !root.expanded
                    onPressAndHold: root.requestOpenPanel()
                    cursorShape: Qt.PointingHandCursor
                }
            }

            StyledText {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                text: {
                    const s = Ai.subconsciousStream;
                    if (s.length === 0) return "";
                    const last = s[s.length - 1];
                    const isTool = last.kind === "tool_call" || last.kind === "tool_result";
                    const tag = isTool ? Translation.tr("tool") : Translation.tr("thinking");
                    // For a forming thought (tokens), show the tail of the sentence
                    // so the newest words are always visible as it grows, rather
                    // than the head eliding away. Tools are short — show in full.
                    const body = (!isTool && last.text.length > 120)
                        ? "…" + last.text.slice(-120) : last.text;
                    return `${tag} · ${body}`;
                }
                color: Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.small
                elide: Text.ElideRight
                maximumLineCount: 2
                wrapMode: Text.WrapAtWordBoundaryOrAnywhere
            }

            // Hint that there's more — expand/collapse chevron, and a long-
            // press affordance tooltip.
            MaterialSymbol {
                Layout.alignment: Qt.AlignVCenter
                iconSize: Appearance.font.pixelSize.small
                text: root.expanded ? "expand_less" : "expand_more"
                color: Appearance.colors.colSubtext
                visible: Ai.subconsciousStream.length > 0
                MouseArea {
                    anchors.fill: parent
                    onClicked: root.expanded = !root.expanded
                    onPressAndHold: root.requestOpenPanel()
                    cursorShape: Qt.PointingHandCursor
                }
                StyledToolTip {
                    text: Translation.tr("Tap to expand · Hold to open the event panel")
                }
            }
        }

        // Inline mini-log: the last several entries (newest at the bottom),
        // shown when expanded. Read-only — the full searchable log lives in
        // the Tier-2 panel.
        ColumnLayout {
            Layout.fillWidth: true
            visible: root.expanded && Ai.subconsciousStream.length > 0
            spacing: 2

            Repeater {
                // Show up to the last 6 entries, oldest first so the forming
                // thought reads top-to-bottom.
                model: ScriptModel {
                    values: Ai.subconsciousStream.slice(-6)
                }
                delegate: StyledText {
                    required property var modelData
                    Layout.fillWidth: true
                    wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                    font.pixelSize: Appearance.font.pixelSize.small
                    color: Appearance.colors.colSubtext
                    text: {
                        const isTool = modelData.kind === "tool_call" || modelData.kind === "tool_result";
                        const tag = isTool ? Translation.tr("tool") : Translation.tr("thinking");
                        return `${tag} · ${modelData.text}`;
                    }
                }
            }
        }
    }
}
