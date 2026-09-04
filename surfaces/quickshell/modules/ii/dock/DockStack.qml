// DockStack — macOS-Dock "Fan" mode for a stack of apps (2026-07-11).
//
// Collapsed: a single stacked-cards icon (the topmost member) with a count
// dot, like a pinned app. Tap or long-press to expand.
//
// Expanded: member icons arc UP off the dock along a polar curve. Angle
// sweeps ~90deg (straight up) to ~55deg across N members; radius grows per
// item so they don't overlap. Slide a thumb up the arc — the icon under the
// finger scales/highlights; release launches it. Release off the arc (or a
// plain tap on empty space) collapses without launching.
//
// The arc lives in its own PopupWindow (like previewPopup in DockApps.qml)
// so it can draw ABOVE the dock's own bounds. Icons animate x/y from the
// collapsed origin out to their arc slots, driven by one `expanded` bool.
pragma ComponentBehavior: Bound
import qs.services
import qs.modules.common
import qs.modules.common.functions
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Widgets

DockButton {
    id: root
    property var appToplevel            // the STACK entry (isStack === true)
    property var appListRoot
    property real iconSize: Config.options?.dock.iconSize ?? 44
    property real countDotWidth: 10
    property real countDotHeight: 4

    // Arc geometry.
    readonly property real arcRadiusBase: 46   // first item's distance up
    readonly property real arcRadiusStep: 30   // extra distance per item
    readonly property real arcAngleStart: 90   // degrees, straight up
    readonly property real arcAngleEnd: 55      // degrees, last item leans out

    readonly property var members: appToplevel?.members ?? []
    property bool expanded: false
    // Index highlighted by the current drag, or -1 for none.
    property int highlightedIndex: -1

    // Polar slot for member i (0-based), as an offset from the collapsed
    // icon centre. y is negative = upward. Single-member stacks go straight up.
    function arcAngleFor(i) {
        if (members.length <= 1) return root.arcAngleStart;
        const t = i / (members.length - 1);
        return root.arcAngleStart + t * (root.arcAngleEnd - root.arcAngleStart);
    }
    function arcRadiusFor(i) {
        return root.arcRadiusBase + i * root.arcRadiusStep;
    }
    function slotX(i) {
        const a = arcAngleFor(i) * Math.PI / 180;
        return arcRadiusFor(i) * Math.cos(a);
    }
    function slotY(i) {
        const a = arcAngleFor(i) * Math.PI / 180;
        return -arcRadiusFor(i) * Math.sin(a);
    }

    function collapse() {
        root.expanded = false;
        root.highlightedIndex = -1;
    }

    // Last open window for a member appId, or null. The stack entry carries
    // its members' toplevels (TaskbarApps routes them here instead of
    // making standalone icons).
    function runningToplevelFor(appId) {
        const tls = root.appToplevel?.toplevels ?? [];
        const low = (appId ?? "").toLowerCase();
        let last = null;
        for (let k = 0; k < tls.length; k++) {
            if ((tls[k].appId ?? "").toLowerCase() === low) last = tls[k];
        }
        return last;
    }

    // Tap a member: focus its open window if it has one; launch otherwise.
    function launchMember(i) {
        if (i < 0 || i >= members.length) return;
        const running = runningToplevelFor(members[i]);
        if (running) {
            running.activate();
            return;
        }
        // Suffix-tolerant — see DockAppButton.desktopEntry.
        const entry = AppSearch.resolveEntry(members[i]);
        entry?.execute();
    }

    implicitWidth: implicitHeight - topInset - bottomInset

    // --- Drop target: drag a loose app onto this stack to add it ---------
    // Same dwell rule as DockAppButton (500ms of hover = intent). Sets the
    // shared drag state with dragTargetStackId non-empty so the release in
    // DockAppButton takes combineIntoStack's add-into-existing branch. The
    // "STACK:" sentinel keeps dragTargetAppId from ever colliding with a
    // plain appId.
    property bool formingStack: false
    DropArea {
        anchors.fill: parent
        onEntered: (drag) => {
            dwellTimer.restart()
        }
        onExited: {
            dwellTimer.stop()
            root.formingStack = false
            if (appListRoot.dragTargetStackId === root.appToplevel.appId) {
                appListRoot.dragTargetAppId = ""
                appListRoot.dragTargetStackId = ""
            }
        }
        Timer {
            id: dwellTimer
            interval: Config.options?.dock.dragDwellMs ?? 500
            onTriggered: {
                root.formingStack = true
                appListRoot.dragTargetAppId = "STACK:" + root.appToplevel.appId
                appListRoot.dragTargetStackId = root.appToplevel.appId
            }
        }
    }

    // Pre-commit preview, mirrors DockAppButton: the stack lights up once
    // the dwell fires so you SEE it will absorb the drop.
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

    // Collapsed icon: GNOME-folder style — a rounded plate holding a mini
    // grid of the member icons (up to 4), so it reads as "a group of these
    // apps" at a glance instead of a mystery blob. Count dots underneath.
    contentItem: Item {
        // Same block pattern as DockAppButton: icon+dots block centered as a
        // unit, reserving half the dot strip, so the stack rides the bar
        // exactly like a plain app icon instead of being placed by its own
        // rules.
        Item {
            anchors.centerIn: parent
            width: root.iconSize
            height: root.iconSize + (root.countDotHeight + 2) / 2

            Item {
                id: collapsedStack
                anchors {
                    left: parent.left
                    right: parent.right
                    top: parent.top
                }
                height: root.iconSize

                Rectangle {
                    anchors.fill: parent
                    radius: Appearance.rounding.small
                    color: ColorUtils.transparentize(Appearance.m3colors.m3surfaceContainer, 0.15)
                    border.width: 1
                    border.color: ColorUtils.transparentize(Appearance.colors.colOnLayer0, 0.8)
                }
                Grid {
                    anchors.centerIn: parent
                    columns: 2
                    spacing: 3
                    Repeater {
                        model: Math.min(root.members.length, 4)
                        delegate: IconImage {
                            required property int index
                            // The members were 12px inside a 35px box — a
                            // stack you could not read at a glance, which is
                            // the whole job of the collapsed form. 11 was
                            // border, padding and grid spacing all taken off
                            // the icon; only the spacing genuinely has to be.
                            implicitSize: (root.iconSize - 7) / 2
                            source: Quickshell.iconPath(AppSearch.guessIcon(root.members[index] ?? ""), "image-missing")
                        }
                    }
                }
            }

            // Count dot(s) under the icon — same pattern as DockAppButton.
            RowLayout {
                spacing: 3
                anchors {
                    top: collapsedStack.bottom
                    topMargin: 2
                    horizontalCenter: parent.horizontalCenter
                }
                Repeater {
                    model: Math.min(root.members.length, 3)
                    delegate: Rectangle {
                        required property int index
                        radius: Appearance.rounding.full
                        implicitWidth: (root.members.length <= 3) ? root.countDotWidth : root.countDotHeight
                        implicitHeight: root.countDotHeight
                        color: (root.appToplevel?.toplevels?.length ?? 0) > 0
                            ? Appearance.colors.colPrimary
                            : ColorUtils.transparentize(Appearance.colors.colOnLayer0, 0.4)
                    }
                }
            }
        }
    }

    // Tap expands; tapping again (while expanded) collapses.
    onClicked: {
        if (root.expanded) root.collapse();
        else root.expanded = true;
    }

    // Horizontal centre of this button, in its window's coordinates —
    // recomputed when the popup opens (same trick as previewPopup).
    property real cachedCenterX: 0
    onExpandedChanged: {
        if (expanded && QsWindow)
            cachedCenterX = QsWindow.mapFromItem(root, root.width / 2, 0).x;
    }

    // The arc: a full-width PopupWindow anchored above the dock (same anchor
    // pattern as previewPopup — gravity Top on the dock window). The arc
    // MouseArea floats at cachedCenterX so it sits over this stack icon.
    PopupWindow {
        id: arcPopup
        visible: root.expanded || arcContent.opacity > 0
        color: "transparent"

        // Reach = furthest member's radius + icon + margin.
        readonly property real reach: root.arcRadiusFor(Math.max(root.members.length - 1, 0)) + root.iconSize + 24

        anchor {
            window: root.QsWindow?.window ?? null
            adjustment: PopupAdjustment.None
            gravity: Edges.Top | Edges.Right
            edges: Edges.Top | Edges.Left
        }

        implicitWidth: root.QsWindow?.window?.width ?? 1
        implicitHeight: reach

        MouseArea {
            id: arcArea
            anchors.bottom: parent.bottom
            implicitWidth: arcPopup.reach * 2
            implicitHeight: arcPopup.reach
            x: root.cachedCenterX - width / 2
            enabled: root.expanded
            hoverEnabled: false

            // Collapsed-icon origin inside this MouseArea = bottom centre.
            readonly property real originX: width / 2
            readonly property real originY: height - root.iconSize / 2

            function indexUnder(px, py) {
                var best = -1;
                var bestD = 44; // px pick radius
                for (var i = 0; i < root.members.length; i++) {
                    const cx = originX + root.slotX(i);
                    const cy = originY + root.slotY(i);
                    const d = Math.hypot(px - cx, py - cy);
                    if (d < bestD) { bestD = d; best = i; }
                }
                return best;
            }

            // --- Member drag: reorder along the arc / drag out to unstack.
            // Press-and-hold a member lifts it; sliding then moves it between
            // slots (siblings part to make room — previewOrder is the live
            // slot assignment). Release on a slot commits the new order;
            // release off the arc entirely unstacks the member back to a
            // standalone pinned app. Quick press-release still launches.
            property int dragMemberIndex: -1     // members[] index being dragged
            property var previewOrder: []        // member index per slot, during drag
            property bool unstackPending: false
            property real dragPX: 0
            property real dragPY: 0
            pressAndHoldInterval: 350

            onPressAndHold: (mouse) => {
                const i = indexUnder(mouse.x, mouse.y);
                if (i < 0) return;
                dragMemberIndex = i;
                previewOrder = Array.from({length: root.members.length}, (_, k) => k);
                dragPX = mouse.x; dragPY = mouse.y;
                unstackPending = false;
                root.highlightedIndex = -1;
            }
            onPositionChanged: (mouse) => {
                if (dragMemberIndex >= 0) {
                    dragPX = mouse.x; dragPY = mouse.y;
                    const s = indexUnder(mouse.x, mouse.y);
                    unstackPending = (s < 0);
                    if (s >= 0) {
                        const cur = previewOrder.indexOf(dragMemberIndex);
                        if (s !== cur) {
                            var po = previewOrder.slice();
                            po.splice(cur, 1);
                            po.splice(s, 0, dragMemberIndex);
                            previewOrder = po;
                        }
                    }
                    return;
                }
                root.highlightedIndex = indexUnder(mouse.x, mouse.y);
            }
            onReleased: (mouse) => {
                if (dragMemberIndex >= 0) {
                    const stackId = root.appToplevel.appId;
                    if (unstackPending) {
                        TaskbarApps.unstackMember(stackId, root.members[dragMemberIndex]);
                    } else if (previewOrder.some((m, s) => m !== s)) {
                        TaskbarApps.setStackOrder(stackId, previewOrder.map(k => root.members[k]));
                    }
                    dragMemberIndex = -1;
                    unstackPending = false;
                    return;   // stay expanded — show the result
                }
                const i = indexUnder(mouse.x, mouse.y);
                if (i >= 0) root.launchMember(i);
                root.collapse();
            }
            onCanceled: {
                dragMemberIndex = -1;
                unstackPending = false;
                root.collapse();
            }
            // A plain tap on empty popup space collapses.
            onClicked: (mouse) => {
                if (indexUnder(mouse.x, mouse.y) < 0) root.collapse();
            }

            Item {
                id: arcContent
                anchors.fill: parent
                opacity: root.expanded ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: 120 } }

                Repeater {
                    model: root.members.length
                    delegate: Item {
                        id: memberIcon
                        required property int index
                        width: root.iconSize
                        height: root.iconSize
                        readonly property bool highlighted: root.highlightedIndex === index
                        readonly property bool beingDragged: arcArea.dragMemberIndex === index
                        // Slot this member occupies: its own index normally,
                        // its previewOrder position while a drag is live.
                        readonly property int slotIndex: {
                            if (arcArea.dragMemberIndex < 0) return index;
                            const s = arcArea.previewOrder.indexOf(index);
                            return s < 0 ? index : s;
                        }

                        // Animate from the collapsed origin out to the slot;
                        // a dragged member tracks the finger instead.
                        x: (root.expanded
                            ? (beingDragged ? arcArea.dragPX
                                            : arcArea.originX + root.slotX(slotIndex))
                            : arcArea.originX) - width / 2
                        y: (root.expanded
                            ? (beingDragged ? arcArea.dragPY
                                            : arcArea.originY + root.slotY(slotIndex))
                            : arcArea.originY) - height / 2
                        scale: (highlighted || beingDragged) ? 1.25 : 1.0
                        z: beingDragged ? 2 : (highlighted ? 1 : 0)
                        // Off-arc = release will unstack; telegraph it.
                        opacity: (beingDragged && arcArea.unstackPending) ? 0.55 : 1

                        Behavior on x { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
                        Behavior on y { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
                        Behavior on scale { NumberAnimation { duration: 90 } }

                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: -4
                            radius: Appearance.rounding.normal
                            color: memberIcon.highlighted
                                ? ColorUtils.transparentize(Appearance.colors.colPrimary, 0.6)
                                : "transparent"
                        }
                        IconImage {
                            anchors.fill: parent
                            source: Quickshell.iconPath(AppSearch.guessIcon(root.members[memberIcon.index] ?? ""), "image-missing")
                        }
                        // Running dot: this member has an open window.
                        Rectangle {
                            visible: root.runningToplevelFor(root.members[memberIcon.index]) !== null
                            anchors {
                                top: parent.bottom
                                topMargin: 1
                                horizontalCenter: parent.horizontalCenter
                            }
                            width: 8; height: 3
                            radius: Appearance.rounding.full
                            color: Appearance.colors.colPrimary
                        }
                    }
                }
            }
        }
    }
}
