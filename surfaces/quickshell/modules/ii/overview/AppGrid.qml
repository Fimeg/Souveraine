// The phone app drawer: every desktop entry as a paged icon grid.
//
// TASK-14. The overview pane is stock ii's *desktop* overview — workspace
// thumbnails + search. On a 5.5" phone the workspace grid is dead weight, so
// the body becomes what a phone Home key opens: an app grid. This lives in its
// own file so Overview.qml only swaps the body widget and ii updates stay
// mergeable (the header comment in Overview.qml, "diff against upstream before
// re-applying"; the original stock body is `ii-base/modules/ii/overview/
// OverviewWidget.qml`).
//
// Deliberately NOT the workspace grid's sibling: the reference shell's
// overview is built on windows, and running windows have their own surface —
// WindowOverview, raised by mission control (the pill's second-stage swipe). An
// app drawer is for *finding* something, not for *switching* to what is
// running, so this grid has no notion of workspaces or of what is currently
// focused. It is the launcher half of the split; WindowOverview is the task
// half. Do not merge them: that conflation is what landed the running-cards on
// the Home page (the `overviewOpen || missionControlOpen` body in Overview.qml
// that this file replaces).
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import Quickshell
import Quickshell.Widgets

Item {
    id: root

    // --- Layout ----------------------------------------------------------
    // 4x5 on the 540x1080 portrait. The reference shell's layout search
    // collapses to a single column of full-width cards in portrait — that is
    // the *task* surface. An app drawer is density, not drama, so this is a
    // dense grid: 4 columns keeps a thumb travel across a page, 5 rows fills
    // the 0.78-height body without needing the whole 1080. Columns, rows and
    // icon size are user-settable in the settings app via
    // Config.options.overview.appGrid (see Config.qml); the fallbacks match
    // the phone's measured defaults. `property JsonObject` reads resolve
    // against the active config file at load, not against Config.qml's base
    // values — so these are fallbacks, not overrides.
    readonly property int columns: Math.max(1, Math.min(10,
        Config?.options?.overview?.appGrid?.columns ?? 4))
    readonly property int rows: Math.max(1, Math.min(10,
        Config?.options?.overview?.appGrid?.rows ?? 5))
    readonly property int iconSize: Math.max(24, Math.min(96,
        Config?.options?.overview?.appGrid?.iconSize ?? 44))
    readonly property int perPage: columns * rows
    readonly property real pageMargin: 14
    readonly property real cellSpacing: 6

    // --- Data ------------------------------------------------------------
    // AppSearch.list is the canonical deduped DesktopEntry list (the same one
    // the search backend reads), sorted here alphabetically. Paging is a
    // property, not a ListModel, so a desktop-entry change recomputes cleanly.
    readonly property var apps: {
        const all = AppSearch.list.slice()
            .filter(app => app?.name)
            .sort((a, b) => (a.name || "").localeCompare(b.name || ""));
        return all;
    }
    readonly property var pages: {
        const result = [];
        for (let i = 0; i < root.apps.length; i += root.perPage)
            result.push(root.apps.slice(i, i + root.perPage));
        return result;
    }
    readonly property int pageCount: root.pages.length

    // How far open, 0..1 — the same single-progress idiom WindowOverview uses
    // so this surface reads as arriving, not appearing. The overview's own
    // gate; mission control drives its own surface.
    property real progress: GlobalStates.overviewOpen ? 1 : 0
    Behavior on progress {
        NumberAnimation {
            duration: 260
            easing.type: Easing.OutCubic
        }
    }
    transform: Translate { y: (1 - root.progress) * 24 }
    opacity: root.progress

    // Nothing installed worth showing. Kept distinct from a broken grid on
    // purpose — the same silence-was-the-bug discipline WindowOverview's
    // "No open windows" label exists for.
    StyledText {
        anchors.centerIn: parent
        visible: root.apps.length === 0
        text: qsTr("No apps")
        opacity: 0.6
    }

    ListView {
        id: list
        anchors.fill: parent
        // Room for the page dots below the grid.
        anchors.bottomMargin: 22
        // Paged horizontally, per TASK-14 ("paged"), matching the task
        // surface's axis so the thumb does one learned motion for both.
        orientation: ListView.Horizontal
        snapMode: ListView.SnapOneItem
        highlightRangeMode: ListView.StrictlyEnforceRange
        preferredHighlightBegin: 0
        preferredHighlightEnd: width
        spacing: 0
        clip: true
        visible: root.apps.length > 0
        model: root.pages

        // The page the strip is on, live (not ListView.currentIndex, which
        // only follows keyboard/programmatic selection); feeds the dots.
        readonly property int pageIndex: Math.max(0, Math.min(root.pageCount - 1,
            Math.round(list.contentX / list.width)))

        delegate: Item {
            id: page
            required property var modelData
            required property int index
            width: list.width
            height: list.height

            readonly property real tileW: (width - root.pageMargin * 2
                - root.cellSpacing * (root.columns - 1)) / root.columns
            readonly property real tileH: (height - root.cellSpacing * (root.rows - 1)) / root.rows

            Grid {
                anchors.fill: parent
                anchors.margins: root.pageMargin
                columns: root.columns
                rows: root.rows
                columnSpacing: root.cellSpacing
                rowSpacing: root.cellSpacing

                Repeater {
                    model: page.modelData
                    delegate: RippleButton {
                        required property var modelData
                        implicitWidth: page.tileW
                        implicitHeight: page.tileH
                        buttonRadius: Appearance?.rounding?.normal ?? 12
                        // Launch closes the drawer; the tapped app takes the
                        // screen. Same close-before-launch order SearchItem
                        // uses — the entry must not execute into an overview
                        // still covering the monitor.
                        onClicked: {
                            GlobalStates.overviewOpen = false;
                            modelData.execute();
                        }
                        contentItem: Column {
                            anchors.fill: parent
                            anchors.topMargin: 12
                            spacing: 6

                            IconImage {
                                anchors.horizontalCenter: parent.horizontalCenter
                                source: Quickshell.iconPath(modelData.icon, "image-missing")
                                implicitWidth: root.iconSize
                                implicitHeight: root.iconSize
                            }

                            StyledText {
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                verticalAlignment: Text.AlignTop
                                elide: Text.ElideRight
                                maximumLineCount: 1
                                text: modelData.name
                                font.pixelSize: Appearance?.font?.pixelSize?.small ?? 12
                                color: Appearance?.colors?.colOnLayer0 ?? "#fff"
                                opacity: 0.85
                            }
                        }
                    }
                }
            }
        }
    }

    // Page dots. One per page, the current one in the accent; only shown when
    // there is more than one page (a single dot that cannot move is noise).
    Row {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 4
        visible: root.pageCount > 1
        spacing: 6
        Repeater {
            model: root.pageCount
            delegate: Rectangle {
                required property int index
                width: 7
                height: 7
                radius: width / 2
                color: index === list.pageIndex
                    ? (Appearance?.colors?.colPrimary ?? "#a0c8ff")
                    : "#66ffffff"
                Behavior on color { ColorAnimation { duration: 150 } }
            }
        }
    }
}