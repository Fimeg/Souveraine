pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.functions
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/*
 * Reasoning, drawn as itself.
 *
 * Owned Souveraine surface. The thing this replaces marked reasoning with
 * literal <think> fences inside one flattened markdown string, which is how a
 * think-fence collision could eat a reply (renderer-think-fence, 2026-08-11).
 * A segment kind cannot collide with punctuation, so the whole class of bug
 * goes away by construction rather than by escaping harder.
 *
 * Visual intent: quieter than speech, and quieter than any tool. Thinking is
 * not addressed to anyone. It is legible when wanted and out of the way
 * otherwise — open while it is still happening, folded once it is done.
 */
Item {
    id: root

    property string segmentContent: ""
    property bool done: false
    property bool completed: false
    property bool renderMarkdown: true
    property bool enableMouseSelection: false

    // Open while the thought is still forming; fold it once it has landed.
    // Deliberately not user-sticky yet: a fold state that survives a delegate
    // recycle needs to live in the model, not here.
    property bool expanded: !root.completed

    onCompletedChanged: if (root.completed) root.expanded = false

    readonly property color accent: Appearance.colors.colSubtext

    Layout.fillWidth: true
    implicitHeight: body.implicitHeight

    Rectangle {
        id: body
        anchors.left: parent.left
        anchors.right: parent.right
        implicitHeight: column.implicitHeight + 12
        radius: Appearance.rounding.small
        color: Appearance.colors.colLayer2
        border.width: 1
        border.color: ColorUtils.transparentize(root.accent, 0.75)

        ColumnLayout {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 6
            spacing: 4

            MouseArea {
                Layout.fillWidth: true
                implicitHeight: header.implicitHeight
                cursorShape: Qt.PointingHandCursor
                onClicked: root.expanded = !root.expanded

                RowLayout {
                    id: header
                    anchors.left: parent.left
                    anchors.right: parent.right
                    spacing: 6

                    MaterialSymbol {
                        text: "neurology"
                        iconSize: Appearance.font.pixelSize.normal
                        color: root.accent
                    }

                    StyledText {
                        Layout.fillWidth: true
                        text: root.completed ? Translation.tr("Thought") : Translation.tr("Thinking\u2026")
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        font.italic: true
                        color: root.accent
                        elide: Text.ElideRight
                    }

                    MaterialSymbol {
                        text: root.expanded ? "expand_less" : "expand_more"
                        iconSize: Appearance.font.pixelSize.normal
                        color: root.accent
                    }
                }
            }

            StyledText {
                Layout.fillWidth: true
                visible: root.expanded && root.segmentContent.length > 0
                text: root.segmentContent
                font.pixelSize: Appearance.font.pixelSize.smaller
                font.italic: true
                color: Appearance.colors.colSubtext
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }
        }
    }
}
