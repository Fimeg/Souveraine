import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import Qt5Compat.GraphicalEffects
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Widgets

DockButton {
    id: root
    property var appToplevel
    property var appListRoot
    property int modelIndex: -1   // set by the delegate Loader (parent.index)
    property int lastFocused: -1
    property real iconSize: Config.options?.dock.iconSize ?? 44
    property real countDotWidth: 10
    property real countDotHeight: 4
    property bool appIsActive: appToplevel.toplevels.find(t => (t.activated == true)) !== undefined

    readonly property bool isSeparator: appToplevel.appId === "SEPARATOR"
    // Suffix-tolerant resolve: a bare heuristicLookup returns null for app_ids
    // that carry an instance suffix (Firefox reports "firefox-default"), and a
    // null entry makes tapping a pinned-but-not-running icon a silent no-op.
    property var desktopEntry: AppSearch.resolveEntry(appToplevel.appId)
    enabled: !isSeparator
    implicitWidth: isSeparator ? 1 : implicitHeight - topInset - bottomInset

    Connections {
        target: DesktopEntries

        function onApplicationsChanged() {
            root.desktopEntry = AppSearch.resolveEntry(appToplevel.appId);
        }
    }

    // --- Insertion gap indicator (drag-to-reorder) ----------------------
    // A thin vertical line at the LEFT edge of this delegate when it is
    // the current reorder target. Sits above the button content so it's
    // always visible during a drag.
    Rectangle {
        visible: appListRoot.dragInsertIndex >= 0
            && root.modelIndex === appListRoot.dragInsertIndex
        anchors.left: parent.left
        anchors.leftMargin: -1   // straddle the ListView spacing
        anchors.verticalCenter: parent.verticalCenter
        width: 2
        height: parent.height * 0.65
        radius: 1
        color: Appearance.colors.colPrimary
        z: 10
        opacity: visible ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 80 } }
    }

    // --- Drag-to-combine (Tier 1) --------------------------------------
    // Pattern copied from this codebase's OverviewWidget.qml (drag a window
    // onto a workspace): a MouseArea with drag.target moves a ghost Item;
    // Drag.active/source/hotSpot are set imperatively on press; the release
    // reads which DropArea the drag ended over (appListRoot.dragTargetAppId,
    // set by the DropArea's onEntered) and commits. NOT Qt's DropArea.onDropped
    // — the anchored-proxy version never moved, so no DropArea ever saw it.
    // The ghost therefore carries NO anchors: Qt refuses to move an anchored
    // drag.target, and an unmoving ghost generates no enter events — the
    // exact bug the line above describes. Position is set on press instead.
    // GNOME's dwell rule (500ms before a hover counts as "combine") gates the
    // highlight so a quick brush-past doesn't read as an intended stack.
    property bool formingStack: false   // this icon is the current drop target, dwell fired
    readonly property string dragAppId: appToplevel.appId

    // The ghost that actually moves under the finger (real Drag source).
    Item {
        id: dragGhost
        width: root.iconSize
        height: root.iconSize
        visible: dragMouse.dragging
        Drag.hotSpot.x: width / 2
        Drag.hotSpot.y: height / 2
        z: 100
        IconImage {
            anchors.fill: parent
            source: Quickshell.iconPath(AppSearch.guessIcon(root.appToplevel.appId), "image-missing")
            opacity: 0.9
        }
    }

    // Single interaction surface (mirrors OverviewWidget's dragArea owning
    // all gestures): tap -> activate/launch, long-press -> menu, drag ->
    // combine. Sits above the visual button; the button's own onClicked is
    // routed here via root.activate() so nothing double-fires.
    // Hold/drag arbitration follows VLC's DelegateTouchTapHandler rule: a
    // long-press may open the menu, but the moment movement turns into a
    // drag the menu is cancelled and the drag owns the gesture (Android
    // home-screen semantics).
    MouseArea {
        id: dragMouse
        anchors.fill: parent
        enabled: !root.isSeparator
        acceptedButtons: Qt.LeftButton | Qt.MiddleButton
        hoverEnabled: true
        property bool dragging: false
        drag.target: dragGhost
        drag.threshold: 12
        pressAndHoldInterval: 450
        onPressed: (mouse) => {
            // Start the (not yet visible) ghost centered under the finger.
            // drag.target then moves it 1:1 with the pointer, so its center
            // hotspot tracks the finger across sibling DropAreas.
            dragGhost.x = mouse.x - dragGhost.width / 2
            dragGhost.y = mouse.y - dragGhost.height / 2
        }
        drag.onActiveChanged: {
            if (drag.active) {
                dragging = true
                dragGhost.Drag.source = root
                dragGhost.Drag.active = true
                // Structural dock edits (manifest pin/stack mutations) are
                // refused while a drag is in flight — see DockManifest.
                GlobalStates.dockDragInProgress = true
                // Store source index for reorder computation.
                appListRoot.dragSourceIndex = root.modelIndex
                appListRoot.dragInsertIndex = -1
                // Hold-then-move means drag, not menu: a menu that opened on
                // the hold gets dismissed the moment real movement starts.
                root.menuOpen = false
            }
        }
        onPressAndHold: (mouse) => {
            if (!dragging && mouse.button === Qt.LeftButton) root.menuOpen = true
        }
        onReleased: (mouse) => {
            if (dragging) {
                const target = appListRoot.dragTargetAppId
                const targetStack = appListRoot.dragTargetStackId
                dragGhost.Drag.active = false
                dragging = false
                GlobalStates.dockDragInProgress = false
                if (target && target.toLowerCase() !== root.appToplevel.appId.toLowerCase()) {
                    // Dwell fired → combine into stack (existing path).
                    TaskbarApps.combineIntoStack(target, root.appToplevel.appId, targetStack)
                } else if (appListRoot.dragInsertIndex >= 0
                    && appListRoot.dragInsertIndex !== appListRoot.dragSourceIndex) {
                    // No dwell, valid insertion gap → reorder pinned app.
                    const stacksCount = TaskbarApps.stacksList().length
                    const targetPinnedIdx = appListRoot.dragInsertIndex - stacksCount
                    if (targetPinnedIdx >= 0) {
                        TaskbarApps.reorderPinned(root.appToplevel.appId, targetPinnedIdx)
                    }
                }
                appListRoot.dragTargetAppId = ""
                appListRoot.dragTargetStackId = ""
                appListRoot.dragInsertIndex = -1
                appListRoot.dragSourceIndex = -1
                return
            }
            if (root.menuOpen) return   // long-press already handled it
            if (mouse.button === Qt.MiddleButton) {
                root.desktopEntry?.execute()
            } else {
                root.activate()
            }
        }
        onCanceled: {
            dragging = false
            dragGhost.Drag.active = false
            GlobalStates.dockDragInProgress = false
            appListRoot.dragInsertIndex = -1
            appListRoot.dragSourceIndex = -1
        }
    }

    DropArea {
        anchors.fill: parent
        onEntered: (drag) => {
            if (drag.source === root) return
            dwellTimer.restart()
            // --- Reorder: compute insertion gap position ---------------
            // Only pinned apps (not stacks, not running apps) participate
            // in reordering. The gap appears BEFORE the target if dragging
            // left (source > target), AFTER if dragging right (source <
            // target) — matching the physical intuition of sliding an icon
            // into a new slot.
            const srcIdx = appListRoot.dragSourceIndex
            const tgtIdx = root.modelIndex
            const stacksCount = TaskbarApps.stacksList().length
            const pinnedCount = Config.options?.dock.pinnedApps?.length ?? 0
            if (srcIdx >= stacksCount && srcIdx < stacksCount + pinnedCount
                && tgtIdx >= stacksCount && tgtIdx < stacksCount + pinnedCount
                && srcIdx !== tgtIdx
                && root.appToplevel.pinned && !root.appToplevel.isStack) {
                appListRoot.dragInsertIndex = srcIdx < tgtIdx ? tgtIdx + 1 : tgtIdx
            }
        }
        onExited: {
            dwellTimer.stop()
            root.formingStack = false
            if (appListRoot.dragTargetAppId === root.appToplevel.appId) {
                appListRoot.dragTargetAppId = ""
                appListRoot.dragTargetStackId = ""
            }
        }
        Timer {
            id: dwellTimer
            interval: Config.options?.dock.dragDwellMs ?? 500   // GNOME's dwell — intent, not accident
            onTriggered: {
                root.formingStack = true
                appListRoot.dragTargetAppId = root.appToplevel.appId
                appListRoot.dragTargetStackId = ""   // plain app -> new stack
                // Dwell = combine intent, not reorder — clear the gap.
                appListRoot.dragInsertIndex = -1
            }
        }
    }

    // Pre-commit preview: a rounded container fades in behind the icon once
    // the dwell fires, so you SEE the stack forming before releasing.
    Rectangle {
        anchors.fill: parent
        anchors.margins: -2
        z: -1
        radius: Appearance.rounding.normal
        visible: opacity > 0
        opacity: root.formingStack ? 1 : 0
        color: ColorUtils.transparentize(Appearance.colors.colPrimary, 0.55)
        border.width: 1
        border.color: Appearance.colors.colPrimary
        Behavior on opacity { NumberAnimation { duration: 120 } }
        scale: root.formingStack ? 1.08 : 1.0
        Behavior on scale { NumberAnimation { duration: 120; easing.type: Easing.OutBack } }
    }

    Loader {
        active: isSeparator
        anchors {
            fill: parent
            topMargin: dockVisualBackground.margin + dockRow.padding + Appearance.rounding.normal
            bottomMargin: dockVisualBackground.margin + dockRow.padding + Appearance.rounding.normal
        }
        sourceComponent: DockSeparator {}
    }

    Loader {
        anchors.fill: parent
        active: appToplevel.toplevels.length > 0
        sourceComponent: MouseArea {
            id: mouseArea
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
            onEntered: {
                appListRoot.lastHoveredButton = root
                appListRoot.buttonHovered = true
                lastFocused = appToplevel.toplevels.length - 1
            }
            onExited: {
                if (appListRoot.lastHoveredButton === root) {
                    appListRoot.buttonHovered = false
                }
            }
        }
    }

    // Tap behaviour, called by the unified dragMouse handler above.
    function activate() {
        if (appToplevel.toplevels.length === 0) {
            root.desktopEntry?.execute();
            return;
        }
        // Cycle from the window that is actually focused, not the stored
        // index — that index goes stale as windows open/close and the hover
        // handler also writes it, so tapping could focus the wrong window.
        const cur = appToplevel.toplevels.findIndex(t => t.activated);
        lastFocused = ((cur >= 0 ? cur : Math.max(lastFocused, -1)) + 1) % appToplevel.toplevels.length
        appToplevel.toplevels[lastFocused].activate()
    }

    middleClickAction: () => {
        root.desktopEntry?.execute();
    }

    altAction: () => {
        TaskbarApps.togglePin(appToplevel.appId);
    }

    // Long-press -> per-app context menu (pin, stack membership, settings
    // stub). Opened by dragMouse.onPressAndHold. This is settings-surface
    // entry (b); "App settings…" fires the dockSettings IPC stub.
    property bool menuOpen: false

    property real menuCenterX: 0
    onMenuOpenChanged: {
        if (menuOpen && QsWindow)
            menuCenterX = QsWindow.mapFromItem(root, root.width / 2, 0).x;
    }

    PopupWindow {
        id: ctxMenu
        visible: root.menuOpen || ctxContent.opacity > 0
        color: "transparent"
        anchor {
            window: root.QsWindow?.window ?? null
            adjustment: PopupAdjustment.None
            gravity: Edges.Top | Edges.Right
            edges: Edges.Top | Edges.Left
        }
        implicitWidth: root.QsWindow?.window?.width ?? 1
        implicitHeight: ctxColumn.implicitHeight + 18

        MouseArea {
            anchors.fill: parent
            enabled: root.menuOpen
            onClicked: root.menuOpen = false // tap-away closes
        }

        Rectangle {
            id: ctxContent
            anchors.bottom: parent.bottom
            x: root.menuCenterX - width / 2
            width: 200
            height: ctxColumn.implicitHeight + 12
            radius: Appearance.rounding.normal
            color: Appearance.m3colors.m3surfaceContainer
            border.width: 1
            border.color: Appearance.colors.colLayer0Border
            opacity: root.menuOpen ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 100 } }

            ColumnLayout {
                id: ctxColumn
                anchors.fill: parent
                anchors.margins: 6
                spacing: 2

                component MenuItem: RippleButton {
                    Layout.fillWidth: true
                    implicitHeight: 34
                    property string label: ""
                    contentItem: StyledText {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        verticalAlignment: Text.AlignVCenter
                        text: parent.label
                        color: Appearance.m3colors.m3onSurface
                        font.pixelSize: Appearance.font.pixelSize.small
                    }
                }

                // Stack membership is drag-only now: drag onto an icon/stack
                // to combine, drag a member off the arc to unstack. The menu
                // keeps only what drag can't express.
                MenuItem {
                    label: TaskbarApps.isPinned(root.appToplevel.appId) ? "Unpin" : "Pin to dock"
                    onClicked: { TaskbarApps.togglePin(root.appToplevel.appId); root.menuOpen = false }
                }
                MenuItem {
                    label: "App settings…"
                    onClicked: {
                        // Stub: hand off to the future souveraine-settings app.
                        Quickshell.execDetached(["qs", "-c", "ii", "ipc", "call",
                            "dockSettings", "openApp", root.appToplevel.appId]);
                        root.menuOpen = false;
                    }
                }
            }
        }
    }

    contentItem: Loader {
        active: !isSeparator
        // The Loader stretches its loaded item to the Loader's own size, so
        // width/height/centerIn declared ON the loaded item are overridden —
        // that stretched the block to the content area and the top-anchored
        // icon rode the top of the bar. The stretched item is a passthrough;
        // the real fixed-size block centers INSIDE it (same structure as
        // DockStack — keep them identical).
        sourceComponent: Item {
            Item {
                anchors.centerIn: parent
                // Center the icon+dots block. Reserve only HALF the dot strip
                // below the icon: reserving the full strip pushed the block's
                // center below the icon's center, so centerIn left extra gap
                // above the icon (icons read a few px high). Half-reserve splits
                // the difference so the glyph sits visually centered.
                width: root.iconSize
                height: root.iconSize + (root.countDotHeight + 2) / 2

                Loader {
                    id: iconImageLoader
                    anchors {
                        left: parent.left
                        right: parent.right
                        top: parent.top
                    }
                    height: root.iconSize
                    active: !root.isSeparator
                    sourceComponent: IconImage {
                        source: Quickshell.iconPath(AppSearch.guessIcon(appToplevel.appId), "image-missing")
                        implicitSize: root.iconSize
                    }
                }

                Loader {
                    active: Config.options.dock.monochromeIcons
                    anchors.fill: iconImageLoader
                    sourceComponent: Item {
                        Desaturate {
                            id: desaturatedIcon
                            visible: false // There's already color overlay
                            anchors.fill: parent
                            source: iconImageLoader
                            desaturation: 0.8
                        }
                        ColorOverlay {
                            anchors.fill: desaturatedIcon
                            source: desaturatedIcon
                            color: ColorUtils.transparentize(Appearance.colors.colPrimary, 0.9)
                        }
                    }
                }

                RowLayout {
                    spacing: 3
                    anchors {
                        top: iconImageLoader.bottom
                        topMargin: 2
                        horizontalCenter: parent.horizontalCenter
                    }
                    Repeater {
                        model: Math.min(appToplevel.toplevels.length, 3)
                        delegate: Rectangle {
                            required property int index
                            radius: Appearance.rounding.full
                            implicitWidth: (appToplevel.toplevels.length <= 3) ? 
                                root.countDotWidth : root.countDotHeight // Circles when too many
                            implicitHeight: root.countDotHeight
                            color: appIsActive ? Appearance.colors.colPrimary : ColorUtils.transparentize(Appearance.colors.colOnLayer0, 0.4)
                        }
                    }
                }
            }
        }
    }
}
