// The garden's micro-view — the vault card in the right panel.
//
// The micro view is a window into the lens window, not a second editor:
// it shows the freshest notes, the sync truth, a daily note entry point
// and a quick capture — and every tap either opens the garden at that
// note or plants one in it. The full app is the lens window itself; this
// is the pulse.
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

Item {
    id: root

    implicitHeight: column.implicitHeight
    property var shown: Lens.joined ? Lens.notes.slice(0, 5) : []

    function dailyRel() {
        return new Date().toISOString().slice(0, 10) + ".md";
    }

    function openDaily() {
        if (!Lens.joined) {
            Lens.join();
            return;
        }
        const rel = root.dailyRel();
        const exists = Lens.notes.some(n => n.rel === rel);
        if (exists)
            Lens.openNote(rel);
        else
            Lens.createNote(rel.replace(/\.md$/, ""));
    }

    function relTime(mtime) {
        const s = Math.max(0, Date.now() / 1000 - mtime);
        if (s < 60) return Translation.tr("just now");
        if (s < 3600) return Translation.tr("%1m ago").arg(Math.floor(s / 60));
        if (s < 86400) return Translation.tr("%1h ago").arg(Math.floor(s / 3600));
        if (s < 604800) return Translation.tr("%1d ago").arg(Math.floor(s / 86400));
        return new Date(mtime * 1000).toLocaleDateString(Qt.locale(), "dd MMM");
    }

    onVisibleChanged: {
        if (root.visible && Lens.joined)
            Lens.requestState();
    }

    ColumnLayout {
        id: column
        anchors.fill: parent
        spacing: 4

        // ── header ──
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            MaterialSymbol {
                text: "hub"
                iconSize: Appearance.font.pixelSize.larger
                color: Appearance.colors.colPrimary
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                StyledText {
                    font.pixelSize: Appearance.font.pixelSize.larger
                    color: Appearance.colors.colOnLayer1
                    text: Translation.tr("Garden")
                }
                StyledText {
                    visible: Lens.joined && Lens.head.length > 0
                    font.pixelSize: Appearance.font.pixelSize.small
                    color: Appearance.colors.colOnLayer1Inactive
                    text: Lens.branch + " @" + Lens.head
                }
            }

            Rectangle {
                id: syncPill
                Layout.alignment: Qt.AlignVCenter
                radius: 8
                color: Appearance.colors.colLayer0
                implicitHeight: pillText.implicitHeight + 6
                implicitWidth: pillText.implicitWidth + 14

                Row {
                    anchors.centerIn: parent
                    spacing: 5

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 7
                        height: 7
                        radius: width / 2
                        color: root.pillColor
                    }
                    StyledText {
                        id: pillText
                        font.pixelSize: Appearance.font.pixelSize.small
                        color: Appearance.colors.colOnLayer0
                        text: Lens.stateText
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: 2
            height: 1
            color: Appearance.colors.colLayer0Border
        }

        // ── the daily note ──
        Item {
            Layout.fillWidth: true
            Layout.topMargin: 2
            implicitHeight: dailyRow.implicitHeight

            RowLayout {
                id: dailyRow
                anchors.fill: parent
                spacing: 6

                MaterialSymbol {
                    text: "today"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer1Inactive
                }
                StyledText {
                    Layout.fillWidth: true
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer1
                    text: Translation.tr("Today's note")
                }
                StyledText {
                    font.pixelSize: Appearance.font.pixelSize.small
                    color: Appearance.colors.colPrimary
                    text: Translation.tr("open")
                }
            }
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openDaily()
            }
        }

        // ── fresh notes ──
        Repeater {
            model: root.shown

            delegate: Item {
                required property var modelData
                required property int index
                Layout.fillWidth: true
                implicitHeight: noteRow.implicitHeight

                RowLayout {
                    id: noteRow
                    anchors.fill: parent
                    spacing: 6

                    Rectangle {
                        Layout.alignment: Qt.AlignVCenter
                        width: 4
                        height: 4
                        radius: width / 2
                        color: index === 0 ? Appearance.colors.colPrimary : Appearance.colors.colOnLayer1Inactive
                    }
                    StyledText {
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        font.pixelSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colOnLayer1
                        text: modelData.title
                    }
                    StyledText {
                        font.pixelSize: Appearance.font.pixelSize.small
                        color: Appearance.colors.colOnLayer1Inactive
                        text: root.relTime(modelData.mtime)
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Lens.openNote(modelData.rel)
                }
            }
        }

        // ── cold card ──
        Item {
            Layout.fillWidth: true
            visible: !Lens.joined
            implicitHeight: coldRow.implicitHeight

            RowLayout {
                id: coldRow
                anchors.fill: parent
                spacing: 6

                MaterialSymbol {
                    text: "crop_free"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer1Inactive
                }
                StyledText {
                    Layout.fillWidth: true
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer1Inactive
                    text: Translation.tr("The garden is closed")
                }
                StyledText {
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colPrimary
                    text: Translation.tr("Open")
                }
            }
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: Lens.join()
            }
        }

        // ── push the truth ──
        Item {
            Layout.fillWidth: true
            visible: Lens.joined && (Lens.syncState === "ahead" || Lens.syncState === "diverged" || Lens.syncState === "dirty")
            implicitHeight: pushRow.implicitHeight

            RowLayout {
                id: pushRow
                anchors.fill: parent
                spacing: 6

                MaterialSymbol {
                    text: "upload"
                    iconSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colPrimary
                }
                StyledText {
                    Layout.fillWidth: true
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colOnLayer1
                    text: Translation.tr("The garden is ahead — push it")
                }
                StyledText {
                    font.pixelSize: Appearance.font.pixelSize.normal
                    color: Appearance.colors.colPrimary
                    text: Translation.tr("Push")
                }
            }
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: Lens.syncNow()
            }
        }
    }

    // Pill color: one dot, one meaning.
    readonly property color pillColor: {
        if (!Lens.joined) return Appearance.colors.colOnLayer1Inactive;
        switch (Lens.syncState) {
        case "clean": return Appearance.colors.colPrimary;
        case "dirty": return Appearance.m3colors.m3error;
        case "ahead":
        case "diverged": return Appearance.m3colors.m3tertiary;
        case "behind": return Appearance.m3colors.m3error;
        default: return Appearance.colors.colOnLayer1Inactive;
        }
    }
}
