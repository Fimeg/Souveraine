pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/*
 * Server-owned conversation control opened by the footer chip.
 *
 * A bubble array is not a thread. Every row here names a server conversation
 * and selecting it backfills that transcript before the next send. New and
 * offered-resume are explicit acts; opening the menu never attaches by itself.
 */
Rectangle {
    id: root

    signal picked

    readonly property string query: searchField.text.trim().toLowerCase()
    readonly property var filteredConversations: Souveraine.conversations
        .filter(conversation => {
            if (root.query.length === 0) return true;
            const date = conversation.updated_at ?? conversation.created_at ?? "";
            return String(conversation.id).toLowerCase().includes(root.query)
                || String(date).toLowerCase().includes(root.query);
        })
        .slice(0, 50)

    function shortId(id) {
        const value = String(id ?? "");
        return value.length > 12 ? value.slice(0, 12) : value;
    }

    function when(conversation) {
        const raw = conversation.updated_at ?? conversation.created_at ?? "";
        if (raw.length === 0) return Translation.tr("date unknown");
        const date = new Date(raw);
        if (isNaN(date.getTime())) return raw;
        return date.toLocaleString(Qt.locale(), "MMM d · HH:mm");
    }

    onVisibleChanged: {
        if (!visible) return;
        searchField.text = "";
        Souveraine.refreshConversations();
    }

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
        spacing: 5

        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            MaterialSymbol {
                text: "forum"
                iconSize: Appearance.font.pixelSize.normal
                color: Appearance.colors.colPrimary
            }
            StyledText {
                Layout.fillWidth: true
                text: Translation.tr("Conversations")
                color: Appearance.colors.colOnLayer2
                font.pixelSize: Appearance.font.pixelSize.small
                font.bold: true
            }
            MaterialSymbol {
                visible: Souveraine.conversationsLoading
                text: "sync"
                iconSize: Appearance.font.pixelSize.normal
                color: Appearance.colors.colPrimary
                RotationAnimation on rotation {
                    running: Souveraine.conversationsLoading
                    from: 0
                    to: 360
                    duration: 900
                    loops: Animation.Infinite
                }
            }
            MaterialSymbol {
                visible: Souveraine.conversationsStale
                text: "sync_problem"
                iconSize: Appearance.font.pixelSize.normal
                color: Appearance.colors.colError
            }
        }

        Rectangle {
            Layout.fillWidth: true
            visible: Souveraine.offeredConversationId.length > 0
            Layout.preferredHeight: visible ? 38 : 0
            radius: Appearance.rounding.small
            color: Appearance.colors.colSecondaryContainer

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 6
                spacing: 7

                MaterialSymbol {
                    text: "history"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnSecondaryContainer
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0

                    StyledText {
                        Layout.fillWidth: true
                        text: Translation.tr("Continue latest")
                        color: Appearance.colors.colOnSecondaryContainer
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        font.bold: true
                    }
                    StyledText {
                        Layout.fillWidth: true
                        text: root.shortId(Souveraine.offeredConversationId)
                        color: Appearance.colors.colOnSecondaryContainer
                        opacity: 0.72
                        font.pixelSize: Appearance.font.pixelSize.smallest
                        elide: Text.ElideRight
                    }
                }
                StyledText {
                    text: Translation.tr("dismiss")
                    color: Appearance.colors.colOnSecondaryContainer
                    opacity: dismissOffer.containsMouse ? 1 : 0.65
                    font.pixelSize: Appearance.font.pixelSize.smallest

                    MouseArea {
                        id: dismissOffer
                        anchors.fill: parent
                        anchors.margins: -6
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: Souveraine.dismissOfferedResume()
                    }
                }
            }

            MouseArea {
                anchors.fill: parent
                anchors.rightMargin: 58
                cursorShape: Qt.PointingHandCursor
                onClicked: if (Souveraine.acceptOfferedResume()) root.picked()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 34
            radius: Appearance.rounding.small
            color: newThread.containsMouse
                ? Appearance.colors.colLayer2Hover
                : Appearance.colors.colLayer2
            border.width: 1
            border.color: Appearance.colors.colOutlineVariant

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 7

                MaterialSymbol {
                    text: "add_comment"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colPrimary
                }
                StyledText {
                    Layout.fillWidth: true
                    text: Translation.tr("New conversation")
                    color: Appearance.colors.colOnLayer2
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
            }

            MouseArea {
                id: newThread
                anchors.fill: parent
                hoverEnabled: true
                enabled: !Souveraine.turnActive
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: {
                    Ai.clearMessages();
                    root.picked();
                }
            }
        }

        TextField {
            id: searchField
            Layout.fillWidth: true
            visible: Souveraine.conversations.length > 5
            Layout.preferredHeight: visible ? 32 : 0
            placeholderText: Translation.tr("Filter by id or date")
            color: Appearance.colors.colOnLayer2
            placeholderTextColor: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            selectByMouse: true
            leftPadding: 9
            rightPadding: 9
            background: Rectangle {
                radius: Appearance.rounding.small
                color: Appearance.colors.colLayer1
                border.width: searchField.activeFocus ? 1 : 0
                border.color: Appearance.colors.colPrimary
            }
        }

        ListView {
            id: conversationList
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, 210)
            clip: true
            spacing: 3
            model: root.filteredConversations
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: conversationRow
                required property var modelData
                readonly property bool selected: modelData.id === Souveraine.conversationId

                width: ListView.view.width
                implicitHeight: 38
                radius: Appearance.rounding.small
                color: selected ? Appearance.colors.colSecondaryContainer
                    : chooseThread.containsMouse ? Appearance.colors.colLayer2Hover
                    : Appearance.colors.colLayer2

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 7

                    MaterialSymbol {
                        text: conversationRow.selected ? "chat" : "chat_bubble_outline"
                        iconSize: Appearance.font.pixelSize.smaller
                        color: conversationRow.selected
                            ? Appearance.colors.colOnSecondaryContainer
                            : Appearance.colors.colSubtext
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        StyledText {
                            Layout.fillWidth: true
                            text: root.when(conversationRow.modelData)
                            color: conversationRow.selected
                                ? Appearance.colors.colOnSecondaryContainer
                                : Appearance.colors.colOnLayer2
                            font.pixelSize: Appearance.font.pixelSize.smaller
                            elide: Text.ElideRight
                        }
                        StyledText {
                            Layout.fillWidth: true
                            text: root.shortId(conversationRow.modelData.id)
                            color: Appearance.colors.colSubtext
                            font.pixelSize: Appearance.font.pixelSize.smallest
                            elide: Text.ElideRight
                        }
                    }
                    StyledText {
                        visible: conversationRow.selected
                        text: Translation.tr("attached")
                        color: Appearance.colors.colOnSecondaryContainer
                        font.pixelSize: Appearance.font.pixelSize.smallest
                    }
                }

                MouseArea {
                    id: chooseThread
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: !Souveraine.turnActive && !conversationRow.selected
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: if (Souveraine.loadConversationById(conversationRow.modelData.id)) root.picked()
                }
            }
        }

        StyledText {
            Layout.fillWidth: true
            visible: !Souveraine.conversationsLoading
                && root.filteredConversations.length === 0
            text: root.query.length > 0
                ? Translation.tr("No matching conversations")
                : Translation.tr("No saved conversations")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
