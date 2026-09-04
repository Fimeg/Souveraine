pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/*
 * One tool call, drawn as itself.
 *
 * Owned Souveraine surface — the vendor snapshot's ToolCallBlock is what this
 * replaces. Differences that are deliberate, not incidental:
 *
 *   - every tool in the registry has a name, an icon and a summary, including
 *     the interiority verbs the borrowed card could not see;
 *   - acts on the machine and acts on herself are visually distinct families,
 *     because they are not the same kind of event;
 *   - status is legible without opening the payload (TASK-72 ask 3);
 *   - collapsed by default, except while running or on failure — the two
 *     cases where the payload is the point (TASK-72 ask 2).
 *
 * It draws only. It reads no state and calls nothing; the segment is handed
 * to it whole. Ai.qml stays the data boundary and does not choose pixels.
 */
Item {
    id: root

    property var segment: ({})

    readonly property string name: String(segment.name ?? "tool")
    readonly property string status: String(segment.status ?? "running")
    readonly property string output: String(segment.output ?? "")
    readonly property bool failed: segment.failed === true
    readonly property bool running: status === "running"

    readonly property string kind: ToolVocabulary.kindOf(name)
    readonly property string summary: ToolVocabulary.summarize(name, segment.arguments)

    // Acts on herself read in the secondary accent; acts on the machine in the
    // primary. Sensors are deliberately quiet — she looks constantly, and a
    // card per glance should not shout.
    readonly property color accent: {
        if (root.failed) return Appearance.colors.colError;
        switch (root.kind) {
        case "sensor": return Appearance.colors.colSubtext;
        case "memory":
        case "presence":
        case "intent":
        case "reach": return Appearance.colors.colSecondary;
        case "control": return Appearance.colors.colError;
        default: return Appearance.colors.colPrimary;
        }
    }

    property bool expanded: root.running || root.failed

    Layout.fillWidth: true
    // The itinerary ribbon owns successful route state persistently beside
    // the composer. Keep failures in the transcript: they are evidence, not
    // duplicate chrome.
    visible: root.name !== "itinerary" || root.failed
    implicitHeight: visible ? card.implicitHeight : 0

    function statusLabel() {
        if (root.running) return Translation.tr("running");
        if (root.status === "unresolved") return Translation.tr("unresolved");
        if (root.failed) return Translation.tr("failed");
        return Translation.tr("done");
    }

    Rectangle {
        id: card
        width: parent.width
        implicitHeight: cardLayout.implicitHeight + 14
        radius: Appearance.rounding.small
        color: root.failed ? Appearance.colors.colErrorContainer : Appearance.colors.colLayer2
        border.width: 1
        border.color: root.failed ? Appearance.colors.colError : Appearance.colors.colOutlineVariant

        // The family stripe. Cheaper to read than an icon and it survives
        // being glanced at sideways in a scrolling column.
        Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            anchors.margins: 1
            width: 2
            radius: 1
            color: root.accent
        }

        ColumnLayout {
            id: cardLayout
            anchors.fill: parent
            anchors.margins: 7
            anchors.leftMargin: 10
            spacing: 5

            MouseArea {
                id: header
                Layout.fillWidth: true
                implicitHeight: headerRow.implicitHeight
                hoverEnabled: true
                // Nothing to open is not the same as refusing to open.
                enabled: root.output.length > 0
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: root.expanded = !root.expanded

                RowLayout {
                    id: headerRow
                    anchors.fill: parent
                    spacing: 7

                    MaterialSymbol {
                        text: ToolVocabulary.iconOf(root.name)
                        iconSize: Appearance.font.pixelSize.large
                        color: root.accent
                    }
                    StyledText {
                        font.pixelSize: Appearance.font.pixelSize.small
                        font.bold: true
                        text: root.name
                        color: Appearance.colors.colOnLayer2
                    }
                    StyledText {
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        font.pixelSize: Appearance.font.pixelSize.small
                        text: root.summary
                        color: Appearance.colors.colSubtext
                    }
                    MaterialSymbol {
                        visible: root.running
                        text: "sync"
                        iconSize: Appearance.font.pixelSize.normal
                        color: root.accent
                        RotationAnimation on rotation {
                            running: root.running
                            from: 0
                            to: 360
                            duration: 900
                            loops: Animation.Infinite
                        }
                    }
                    StyledText {
                        visible: !root.running
                        font.pixelSize: Appearance.font.pixelSize.small
                        text: root.statusLabel()
                        color: root.failed ? Appearance.colors.colError : Appearance.colors.colSubtext
                    }
                    MaterialSymbol {
                        visible: header.enabled
                        text: root.expanded ? "expand_less" : "expand_more"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colSubtext
                    }
                }
            }

            // A height-capped TextArea is clipped, not scrollable. Put it in a
            // real ScrollView so a long grep/build result can be inspected in
            // place without expanding one card over the whole conversation.
            ScrollView {
                id: outputScroll
                Layout.fillWidth: true
                visible: root.expanded && root.output.length > 0
                Layout.preferredHeight: visible ? Math.min(outputEditor.implicitHeight, 240) : 0
                clip: true
                ScrollBar.vertical.policy: ScrollBar.AsNeeded
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                TextArea {
                    id: outputEditor
                    width: outputScroll.availableWidth
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.WrapAnywhere
                    textFormat: TextEdit.PlainText
                    text: root.output
                    font.family: Appearance.font.family.monospace
                    font.pixelSize: Appearance.font.pixelSize.smaller
                    color: root.failed ? Appearance.colors.colOnErrorContainer : Appearance.colors.colOnLayer2
                    background: Rectangle {
                        radius: Appearance.rounding.small / 2
                        color: Appearance.colors.colLayer1
                    }
                }
            }
        }
    }
}
