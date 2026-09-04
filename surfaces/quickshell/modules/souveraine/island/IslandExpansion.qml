import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

// The island opened: every session the collector sees, newest first.
//
// Deliberately a flat list rather than provider-grouped tabs. The question this
// answers is "what is running right now", and that is chronological, not
// taxonomic — grouping by provider would bury a live Codex run under three idle
// Souveraine threads. The provider is a glyph on each row instead.
Rectangle {
    id: root

    signal dismissed

    implicitWidth: 320
    implicitHeight: Math.min(column.implicitHeight + 16, 360)
    radius: Appearance.rounding.small
    color: Appearance.colors.colLayer2
    border.width: 1
    border.color: Appearance.colors.colLayer0Border

    opacity: visible ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 140 } }

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: 8
        spacing: 4

        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            StyledText {
                Layout.fillWidth: true
                text: qsTr("Agent sessions")
                color: Appearance.colors.colOnLayer0
                font.pixelSize: Appearance.font.pixelSize.smaller
                font.weight: Font.DemiBold
            }

            // Provider health belongs here, not on a row: a provider that is
            // unavailable has no rows to hang the message on, and "no sessions"
            // must never be confused with "not looking". Same family as every
            // empty-result bug this project has hit.
            //
            // Stated visibly rather than in a tooltip. A MaterialSymbol has no
            // `hovered`, and our StyledToolTip fix deliberately treats a
            // non-hoverable parent as "never show" -- so a tooltip here could
            // never have appeared, which is exactly the silent-absence failure
            // this block exists to prevent.
            Repeater {
                model: ["souveraine", "claude", "codex"]
                delegate: RowLayout {
                    id: providerHealthRow
                    required property string modelData
                    readonly property var p: AgentSessions.providers?.[modelData] ?? null
                    visible: p !== null && p.available === false
                    spacing: 3
                    MaterialSymbol {
                        text: AgentSessions.providerIcon(providerHealthRow.modelData)
                        iconSize: Appearance.font.pixelSize.smaller
                        color: Appearance.colors.colOutlineVariant
                    }
                    StyledText {
                        text: qsTr("%1 unavailable").arg(AgentSessions.providerLabel(providerHealthRow.modelData))
                        color: Appearance.colors.colOutlineVariant
                        font.pixelSize: Appearance.font.pixelSize.smallest
                        elide: Text.ElideRight
                    }
                }
            }
        }

        StyledText {
            Layout.fillWidth: true
            visible: AgentSessions.stale
            text: qsTr("stale — %1").arg(AgentSessions.lastError)
            color: Appearance.colors.colOutlineVariant
            font.pixelSize: Appearance.font.pixelSize.smallest
            elide: Text.ElideRight
        }

        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredHeight: contentHeight
            clip: true
            spacing: 2
            model: AgentSessions.sessions

            delegate: Item {
                required property var modelData
                width: ListView.view.width
                implicitHeight: 30

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 4
                    spacing: 6

                    Rectangle {
                        Layout.alignment: Qt.AlignVCenter
                        implicitWidth: 6
                        implicitHeight: 6
                        radius: 3
                        color: modelData.state === "active" ? Appearance.colors.colPrimary
                             : modelData.state === "recent" ? Appearance.colors.colOnLayer0
                             : Appearance.colors.colOutlineVariant
                    }

                    MaterialSymbol {
                        Layout.alignment: Qt.AlignVCenter
                        text: AgentSessions.providerIcon(modelData.provider)
                        iconSize: Appearance.font.pixelSize.smaller
                        color: Appearance.colors.colOnLayer0
                    }

                    StyledText {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        text: AgentSessions.sessionLabel(modelData)
                        color: Appearance.colors.colOnLayer0
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        elide: Text.ElideRight
                    }

                    // "—" where a provider does not report tokens. Rendering a
                    // 0 would be a measurement claim we cannot back. The
                    // substrate persists per-turn usage, but this presence
                    // projection does not aggregate it. An em dash is the
                    // honest glyph for "not measured".
                    StyledText {
                        Layout.alignment: Qt.AlignVCenter
                        readonly property int tok: AgentSessions.sessionTokens(modelData)
                        text: tok < 0 ? "—"
                            : tok > 1000 ? (Math.round(tok / 100) / 10) + "k"
                            : String(tok)
                        color: Appearance.colors.colOutlineVariant
                        font.pixelSize: Appearance.font.pixelSize.smallest
                    }
                }
            }
        }

        StyledText {
            Layout.fillWidth: true
            visible: AgentSessions.sessions.length === 0
            text: AgentSessions.available ? qsTr("No recent sessions")
                                          : qsTr("Collecting…")
            color: Appearance.colors.colOutlineVariant
            font.pixelSize: Appearance.font.pixelSize.smaller
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
