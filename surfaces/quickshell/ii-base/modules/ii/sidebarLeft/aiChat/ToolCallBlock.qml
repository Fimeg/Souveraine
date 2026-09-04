pragma ComponentBehavior: Bound

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property var segment: ({})
    property bool expanded: segment.status === "running" || segment.failed === true
    property string name: String(segment.name ?? "tool")
    property string status: String(segment.status ?? "running")
    property string output: String(segment.output ?? "")
    property bool failed: segment.failed === true
    property string summary: toolSummary()

    Layout.fillWidth: true
    implicitHeight: card.implicitHeight

    function clip(text, max) {
        const value = String(text ?? "");
        return value.length > max ? value.slice(0, Math.max(0, max - 1)) + "…" : value;
    }

    function argumentsObject() {
        try {
            const parsed = JSON.parse(String(segment.arguments ?? "{}"));
            return parsed && typeof parsed === "object" ? parsed : {};
        } catch (error) {
            return {};
        }
    }

    function toolSummary() {
        const args = argumentsObject();
        switch (name) {
        case "bash": return "$ " + clip(args.command ?? segment.arguments, 88);
        case "read": return clip(args.path ?? segment.arguments, 88);
        case "write": return "write → " + clip(args.path ?? segment.arguments, 78);
        case "edit": return "edit → " + clip(args.path ?? segment.arguments, 80);
        case "grep": return "\"" + clip(args.pattern ?? segment.arguments, 48) + "\" " + clip(args.path ?? "", 30);
        case "glob": return clip(args.pattern ?? segment.arguments, 88);
        case "list_dir": return clip(args.path ?? ".", 88);
        case "memory": return clip((args.command ?? "list") + " " + (args.path ?? ""), 88);
        default: return clip(segment.arguments ?? "", 88);
        }
    }

    function iconForTool() {
        switch (name) {
        case "bash": return "terminal";
        case "read": return "article";
        case "write":
        case "edit": return "edit_note";
        case "grep": return "manage_search";
        case "glob":
        case "list_dir": return "folder";
        case "memory": return "psychology";
        default: return "sensors";
        }
    }

    function statusLabel() {
        if (status === "running") return Translation.tr("running");
        if (status === "unresolved") return Translation.tr("unresolved");
        if (failed) return Translation.tr("failed");
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

        ColumnLayout {
            id: cardLayout
            anchors.fill: parent
            anchors.margins: 7
            spacing: 5

            MouseArea {
                id: header
                Layout.fillWidth: true
                implicitHeight: headerRow.implicitHeight
                hoverEnabled: true
                enabled: root.status !== "running" || root.output.length > 0
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: root.expanded = !root.expanded

                RowLayout {
                    id: headerRow
                    anchors.fill: parent
                    spacing: 7

                    MaterialSymbol {
                        text: root.iconForTool()
                        iconSize: Appearance.font.pixelSize.large
                        color: root.failed ? Appearance.colors.colError : Appearance.colors.colPrimary
                    }
                    StyledText {
                        Layout.fillWidth: false
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
                        visible: root.status === "running"
                        text: "sync"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colPrimary
                        RotationAnimation on rotation {
                            running: root.status === "running"
                            from: 0
                            to: 360
                            duration: 900
                            loops: Animation.Infinite
                        }
                    }
                    StyledText {
                        visible: root.status !== "running"
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

            TextArea {
                Layout.fillWidth: true
                visible: root.expanded && root.output.length > 0
                implicitHeight: visible ? Math.min(contentHeight + topPadding + bottomPadding, 240) : 0
                readOnly: true
                selectByMouse: true
                wrapMode: TextArea.Wrap
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
