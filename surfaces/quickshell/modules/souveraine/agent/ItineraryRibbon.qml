pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/*
 * The itinerary's persistent surface.
 *
 * A tool card says an itinerary verb happened. This ribbon says where the
 * agent is now. It consumes the substrate's structured read-only projection,
 * remains beside the composer while chat scrolls, and opens to reveal linked
 * todo stops without becoming a second commitment store.
 */
Rectangle {
    id: root

    property var itinerary: ({})
    property bool stale: false
    property bool expanded: false

    readonly property var stops: root.itinerary?.stops ?? []
    readonly property int currentIndex: Number(root.itinerary?.current ?? 0)
    readonly property var currentStop: currentIndex >= 0 && currentIndex < stops.length
        ? stops[currentIndex] : null
    readonly property int doneCount: {
        let count = 0;
        for (const stop of stops) if (stop.status === "done") count++;
        return count;
    }
    readonly property string phase: stops.length < 1 ? ""
        : root.itinerary?.active
            ? `${Math.min(currentIndex + 1, stops.length)}/${stops.length}`
            : `${stops.length}/${stops.length}`

    implicitHeight: layout.implicitHeight + 2
    radius: Appearance.rounding.normal
    color: Appearance.colors.colSecondaryContainer
    border.width: 1
    border.color: Appearance.colors.colSecondaryContainerActive
    clip: true

    ColumnLayout {
        id: layout
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 1
        spacing: 0

        Item {
            Layout.fillWidth: true
            implicitHeight: 34

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 9
                anchors.rightMargin: 7
                spacing: 7

                MaterialSymbol {
                    text: root.itinerary?.active ? "route" : "task_alt"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnSecondaryContainer
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0

                    StyledText {
                        Layout.fillWidth: true
                        text: root.itinerary?.title ?? Translation.tr("Itinerary")
                        color: Appearance.colors.colOnSecondaryContainer
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        font.bold: true
                        elide: Text.ElideRight
                    }
                    StyledText {
                        Layout.fillWidth: true
                        text: root.currentStop?.name
                            ?? (root.itinerary?.active ? Translation.tr("In progress") : Translation.tr("Route complete"))
                        color: Appearance.colors.colOnSecondaryContainer
                        opacity: 0.72
                        font.pixelSize: Appearance.font.pixelSize.smallest
                        elide: Text.ElideRight
                    }
                }
                StyledText {
                    text: root.phase
                    color: Appearance.colors.colOnSecondaryContainer
                    font.pixelSize: Appearance.font.pixelSize.smallest
                    font.bold: true
                }
                MaterialSymbol {
                    visible: root.stale
                    text: "sync_problem"
                    iconSize: Appearance.font.pixelSize.smaller
                    color: Appearance.colors.colError
                }
                MaterialSymbol {
                    text: root.expanded ? "expand_less" : "expand_more"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnSecondaryContainer
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.expanded = !root.expanded
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.leftMargin: 8
            Layout.rightMargin: 8
            implicitHeight: 2
            radius: 1
            color: Appearance.colors.colLayer2

            Rectangle {
                width: parent.width * (root.stops.length > 0 ? root.doneCount / root.stops.length : 0)
                height: parent.height
                radius: parent.radius
                color: Appearance.colors.colPrimary
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: root.expanded ? Math.min(stopList.contentHeight, 190) : 0
            visible: root.expanded
            clip: true
            ScrollBar.vertical.policy: ScrollBar.AsNeeded

            ListView {
                id: stopList
                model: root.stops
                spacing: 3
                boundsBehavior: Flickable.StopAtBounds

                delegate: Rectangle {
                    id: stopCard
                    required property var modelData
                    width: stopList.width
                    implicitHeight: stopRow.implicitHeight + 10
                    radius: Appearance.rounding.small
                    color: stopCard.modelData.status === "current"
                        ? Appearance.colors.colSecondaryContainerActive
                        : Appearance.colors.colLayer2

                    RowLayout {
                        id: stopRow
                        anchors.fill: parent
                        anchors.margins: 5
                        spacing: 7

                        MaterialSymbol {
                            text: stopCard.modelData.status === "done" ? "check_circle"
                                : stopCard.modelData.status === "current" ? "radio_button_checked"
                                : "radio_button_unchecked"
                            iconSize: Appearance.font.pixelSize.smaller
                            color: stopCard.modelData.status === "current"
                                ? Appearance.colors.colPrimary
                                : Appearance.colors.colSubtext
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0

                            StyledText {
                                Layout.fillWidth: true
                                text: stopCard.modelData.name ?? ""
                                color: Appearance.colors.colOnLayer2
                                font.pixelSize: Appearance.font.pixelSize.smaller
                                font.strikeout: stopCard.modelData.status === "done"
                                elide: Text.ElideRight
                            }
                            StyledText {
                                Layout.fillWidth: true
                                visible: (stopCard.modelData.description?.length ?? 0) > 0
                                text: stopCard.modelData.description ?? ""
                                color: Appearance.colors.colSubtext
                                font.pixelSize: Appearance.font.pixelSize.smallest
                                elide: Text.ElideRight
                            }
                        }
                        MaterialSymbol {
                            visible: (stopCard.modelData.todo_id?.length ?? 0) > 0
                            text: "checklist"
                            iconSize: Appearance.font.pixelSize.smaller
                            color: Appearance.colors.colSubtext
                        }
                        StyledText {
                            visible: (stopCard.modelData.nature?.length ?? 0) > 0
                            text: stopCard.modelData.nature ?? ""
                            color: Appearance.colors.colSubtext
                            font.pixelSize: Appearance.font.pixelSize.smallest
                        }
                        MaterialSymbol {
                            visible: stopCard.modelData.energy === "generative"
                            text: "bolt"
                            iconSize: Appearance.font.pixelSize.smaller
                            color: Appearance.colors.colPrimary
                        }
                    }
                }
            }
        }
    }

    Behavior on implicitHeight {
        NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
    }
}
