pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

/*
 * The footer's agent menu.
 *
 * The first section controls this pane and therefore lists only Souveraine
 * agents. AgentSessions' Claude/Codex records are observations, not selectable
 * backends for this conversation; active external sessions remain visible in
 * a separately labelled read-only section so status cannot masquerade as a
 * control again.
 */
Rectangle {
    id: root

    signal picked

    readonly property var externalSessions: AgentSessions.sessions.filter(session =>
        session.provider !== "souveraine" && session.state === "active")

    implicitHeight: content.implicitHeight + 14
    radius: Appearance.rounding.normal
    color: Appearance.colors.colLayer2
    border.width: 1
    border.color: Appearance.colors.colLayer0Border
    clip: true

    ColumnLayout {
        id: content
        anchors.fill: parent
        anchors.margins: 7
        spacing: 4

        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            MaterialSymbol {
                text: "neurology"
                iconSize: Appearance.font.pixelSize.normal
                color: Appearance.colors.colPrimary
            }
            StyledText {
                Layout.fillWidth: true
                text: Translation.tr("This conversation")
                color: Appearance.colors.colOnLayer2
                font.pixelSize: Appearance.font.pixelSize.small
                font.bold: true
            }
            StyledText {
                visible: Souveraine.turnActive
                text: Translation.tr("turn running")
                color: Appearance.colors.colPrimary
                font.pixelSize: Appearance.font.pixelSize.smallest
            }
        }

        Repeater {
            model: Ai.modelList

            delegate: Rectangle {
                id: agentRow
                required property var modelData
                readonly property var agent: Ai.models[modelData] ?? null
                readonly property bool selected: modelData === Souveraine.currentAgentId

                Layout.fillWidth: true
                implicitHeight: 34
                radius: Appearance.rounding.small
                color: selected ? Appearance.colors.colSecondaryContainer
                    : picker.containsMouse ? Appearance.colors.colLayer2Hover
                    : Appearance.colors.colLayer2

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 7

                    Rectangle {
                        Layout.alignment: Qt.AlignVCenter
                        implicitWidth: 7
                        implicitHeight: 7
                        radius: 4
                        color: agentRow.selected && Souveraine.turnActive
                            ? Appearance.colors.colPrimary
                            : agentRow.selected
                                ? Appearance.colors.colOnSecondaryContainer
                                : Appearance.colors.colOutlineVariant
                    }
                    StyledText {
                        Layout.fillWidth: true
                        text: agentRow.agent?.name ?? agentRow.modelData
                        color: agentRow.selected
                            ? Appearance.colors.colOnSecondaryContainer
                            : Appearance.colors.colOnLayer2
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        font.bold: agentRow.selected
                        elide: Text.ElideRight
                    }
                    StyledText {
                        text: agentRow.selected ? Translation.tr("selected") : ""
                        color: Appearance.colors.colSubtext
                        font.pixelSize: Appearance.font.pixelSize.smallest
                    }
                }

                MouseArea {
                    id: picker
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: !Souveraine.turnActive && !agentRow.selected
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: {
                        Ai.setModel(agentRow.modelData, false);
                        root.picked();
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            visible: root.externalSessions.length > 0
            color: Appearance.colors.colLayer0Border
        }

        StyledText {
            Layout.fillWidth: true
            visible: root.externalSessions.length > 0
            text: Translation.tr("Running elsewhere · observed only")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smallest
        }

        Repeater {
            model: root.externalSessions

            delegate: RowLayout {
                id: externalRow
                required property var modelData
                Layout.fillWidth: true
                Layout.leftMargin: 8
                Layout.rightMargin: 8
                spacing: 7

                MaterialSymbol {
                    text: AgentSessions.providerIcon(externalRow.modelData.provider)
                    iconSize: Appearance.font.pixelSize.smaller
                    color: Appearance.colors.colSubtext
                }
                StyledText {
                    Layout.fillWidth: true
                    text: AgentSessions.sessionLabel(externalRow.modelData)
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smallest
                    elide: Text.ElideRight
                }
                StyledText {
                    text: AgentSessions.providerLabel(externalRow.modelData.provider)
                    color: Appearance.colors.colOutlineVariant
                    font.pixelSize: Appearance.font.pixelSize.smallest
                }
            }
        }
    }
}
