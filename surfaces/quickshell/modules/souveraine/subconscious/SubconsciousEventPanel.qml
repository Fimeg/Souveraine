import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions

// Tier 2 of the subconscious three-tier visibility design (see
// docs/tasks/subconscious-surfacing-threshold.md): a persistent, interactive
// event log — the shell's equivalent of the TUI "cockpit". Shows surfaced
// subconscious events (pass snapshots, reflections, archivist syntheses,
// surfacings) from past turns. Each entry click-expands to its full text,
// closing docs/bugs.md B-005 ("subconscious messages not expandable").
// Fed by Ai.subconsciousEvents; the live Tier-1 ticker is snapshotted into
// it on pass end.
//
// Souveraine-original, relocated out of the ii tree on 2026-07-30. It was
// previously a StyledOverlayWidget (qs.modules.ii.overlay) riveted into the
// ii overlay system; that base is intentionally NOT ported, because we are
// exiting ii. This is now plain content — a host (a souveraine overlay
// surface, a sidebar, or a future viewtop scene node) instantiates and places
// it. The pin/close/drag affordances were the overlay host's job, not the
// content's, so they are not here. GAP NAMED: no souveraine host mounts this
// yet, so it does not reach the glass until one exists.
Item {
    id: root

    // A host may read these to label/chrome the panel.
    property string title: Translation.tr("Subconscious")
    property int contentRadius: Appearance.rounding.normal

    implicitWidth: body.implicitWidth
    implicitHeight: body.implicitHeight

    Rectangle {
        id: body
        anchors.fill: parent
        radius: root.contentRadius
        color: Appearance.colors.colLayer1
        implicitWidth: 340
        implicitHeight: 420

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 6

            // Header
            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                MaterialSymbol {
                    iconSize: Appearance.font.pixelSize.larger
                    text: "psychology"
                    color: Appearance.colors.colSubtext
                }
                StyledText {
                    Layout.fillWidth: true
                    text: Translation.tr("Subconscious event log")
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.small
                    font.bold: true
                }
                StyledText {
                    text: Ai.subconsciousEvents.length
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.small
                    visible: Ai.subconsciousEvents.length > 0
                }
            }

            // Event list
            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true

                ListView {
                    id: eventList
                    model: Ai.subconsciousEvents
                    spacing: 4
                    boundsBehavior: Flickable.StopAtBounds

                    delegate: Rectangle {
                        width: eventList.width
                        height: entryCol.implicitHeight + 12
                        radius: Appearance.rounding.small
                        color: Appearance.colors.colLayer2
                        border.width: 1
                        border.color: Appearance.colors.colLayer1Active

                        property bool expanded: false

                        ColumnLayout {
                            id: entryCol
                            anchors.fill: parent
                            anchors.margins: 6
                            spacing: 3

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 4

                                MaterialSymbol {
                                    iconSize: Appearance.font.pixelSize.small
                                    text: modelData.kind === "halt" ? "error"
                                        : modelData.kind === "reflection" ? "lightbulb"
                                        : modelData.kind === "archivist" ? "auto_stories"
                                        : "psychology"
                                    color: Appearance.colors.colSubtext
                                }
                                StyledText {
                                    Layout.fillWidth: true
                                    text: modelData.source ?? Translation.tr("Subconscious")
                                    color: Appearance.colors.colOnLayer1
                                    font.pixelSize: Appearance.font.pixelSize.small
                                    font.bold: true
                                    elide: Text.ElideRight
                                }
                                MaterialSymbol {
                                    iconSize: Appearance.font.pixelSize.small
                                    text: expanded ? "expand_less" : "expand_more"
                                    color: Appearance.colors.colSubtext
                                }
                            }

                            StyledText {
                                Layout.fillWidth: true
                                visible: !expanded
                                text: (modelData.content ?? "").split("\n")[0]
                                color: Appearance.colors.colSubtext
                                font.pixelSize: Appearance.font.pixelSize.small
                                elide: Text.ElideRight
                            }

                            StyledText {
                                Layout.fillWidth: true
                                visible: expanded
                                wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                                text: modelData.content ?? ""
                                color: Appearance.colors.colOnLayer1
                                font.pixelSize: Appearance.font.pixelSize.small
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: parent.expanded = !parent.expanded
                        }
                    }
                }
            }

            // Empty state
            StyledText {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: Ai.subconsciousEvents.length === 0
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                text: Translation.tr("No subconscious events yet.\nThey appear here after a pass.")
                color: Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.small
                wrapMode: Text.WrapAtWordBoundaryOrAnywhere
            }

            // Clear log
            StyledText {
                Layout.alignment: Qt.AlignRight
                text: Translation.tr("Clear")
                color: Appearance.colors.colSubtext
                font.pixelSize: Appearance.font.pixelSize.small
                visible: Ai.subconsciousEvents.length > 0

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Ai.subconsciousEvents = []
                }
            }
        }
    }
}
