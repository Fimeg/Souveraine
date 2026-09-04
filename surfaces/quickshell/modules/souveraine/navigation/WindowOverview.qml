// The window overview: every open window as a card, scaled down.
//
// WHY THIS REPLACES THE WORKSPACE GRID. `ii-base/modules/ii/overview/
// OverviewWidget.qml` draws a grid of *workspaces* and places windows inside
// them by Hyprland coordinates — `HyprlandData.windowList`, `monitorData`,
// `Hyprland.monitorFor`. Under viewtop none of that resolves: HyprlandData
// cannot parse a compositor that is not Hyprland (the shell logs exactly that
// on every refresh). So the old overview cannot be repaired by pointing it at a
// different data source.
//
// This header used to add "and viewtop has no workspaces at all", written the
// same night `workspace.rs` landed. viewtop has them, and they are not
// Hyprland's: a *continuous* strip that grows when a window needs a room and
// shrinks when the last one leaves, read over the control socket as
// `{"op":"workspaces"}` — count, active, and the float the strip is scrolled
// to. That is a thing this overview could draw and does not yet. What remains
// true is the original point: TASK-14 calls a grid of workspace thumbnails dead
// weight on a phone, so the cards are windows.
//
// What does exist is windows, and the reference Flutter shell's overview is
// built on exactly that: equal-sized cards, each holding a live texture of one
// window, flick-up to dismiss. Its layout search tries every column count for
// the largest cards that fit — on its landscape target that spreads wide, and
// on our 540x1080 portrait the same search collapses to one column of
// full-width cards, which is what this lays out directly rather than
// rediscovering by search.
//
// The previews are `ScreencopyView` against a `Toplevel`, which needs the
// compositor to serve a per-window capture source. viewtop did not until
// 2026-08-03 — only whole-output capture — which is why a card could never have
// shown anything but the screen it was covering.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import qs
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root

    // Newest first. `ToplevelManager.toplevels.values` is the same list the
    // dock and TaskbarApps already read, so there is one idea of what is open.
    readonly property var windows: {
        const all = ToplevelManager.toplevels?.values ?? [];
        return all.slice().reverse();
    }
    readonly property int count: windows.length

    signal activated(var toplevel)
    signal closed(var toplevel)

    // How far open, 0..1. The reference shell drives its whole overview from
    // one `progress` — cards read it, offset their own entry from it, and the
    // strip slides on it — rather than each piece animating independently on a
    // visible/not-visible flag. That is what makes it read as one surface
    // arriving instead of several things appearing at once.
    //
    // It is also Phosh's model, which SHELL-ECOSYSTEM names as the one we
    // study: state gates visibility, and the canonical line is
    // `use_top_layer = !locked`. Visibility follows state; state is never set
    // from a visibility handler.
    property real progress: (GlobalStates.overviewOpen || GlobalStates.missionControlOpen) ? 1 : 0
    Behavior on progress {
        NumberAnimation {
            duration: 260
            // Standard-curve: quick to commit, slow to settle. A symmetric
            // ease makes an overview feel like it is deciding.
            easing.type: Easing.OutCubic
        }
    }

    implicitWidth: parent ? parent.width : 540
    implicitHeight: parent ? parent.height : 800

    // Nothing open. Said plainly rather than rendering an empty frame, because
    // an overview that draws nothing and an overview that is broken look
    // identical — which is the failure this whole surface was in.
    StyledText {
        anchors.centerIn: parent
        visible: root.count === 0
        text: qsTr("No open windows")
        opacity: 0.6
    }

    ListView {
        id: list
        anchors.fill: parent
        anchors.margins: 12
        // The whole strip slides up as it opens, which is what ties the cards
        // together into one surface. 24px is small on purpose — a long throw
        // reads as a modal, and this is a place you pass through.
        transform: Translate { y: (1 - root.progress) * 24 }
        visible: root.count > 0
        model: root.windows
        // Horizontal, per TASK-14 ("horizontally swiped live app cards") and
        // per the reference shell's own carousel, whose comment reads "the
        // whole strip slides in horizontally". A vertical strip was the first
        // draft and it is the wrong axis on a phone: the thumb travels
        // sideways, and a vertical list fights the flick-up that dismisses.
        orientation: ListView.Horizontal
        snapMode: ListView.SnapOneItem
        highlightRangeMode: ListView.StrictlyEnforceRange
        preferredHighlightBegin: 0
        preferredHighlightEnd: width
        spacing: 12
        clip: true
        // Cards are tall; a phone shows one and a bit at a time and flicks
        // between them. Clamped at zero because `height` is -1 until the first
        // layout pass and ListView rejects a negative buffer outright.
        cacheBuffer: Math.max(0, width * 2)

        delegate: Item {
            id: card
            required property var modelData
            required property int index

            // --- Entry, dismissal and parallax --------------------------------
            //
            // The numbers are the reference shell's, read out of
            // `overview_window_card.dart` rather than guessed, because they are
            // the part that took someone a long time to get right:
            //
            //   stagger      index * 0.045 of the run, capped at the 5th card
            //   intro scale  0.86 -> 1.0
            //   dismiss at   32% of the card's height
            //   fling        -760 px/s
            //   rubber band  56 px downward, past which it stops following
            //   settle       90..240 ms
            //
            // The cap matters: without it the tenth card starts its entry half
            // a second after the first, and the overview feels like it is
            // loading rather than opening.
            property real dismissY: 0
            property bool dismissed: false
            readonly property real dismissDistance: height * 0.32

            // Each card's own share of the opening, staggered by index and
            // capped at the fifth — `interval(progress, i*0.045, 1)` in the
            // reference. The cap is the part that matters: uncapped, the tenth
            // card starts half a second after the first and the overview reads
            // as loading rather than opening.
            readonly property real intro: {
                const begin = Math.min(index, 4) * 0.045;
                if (root.progress <= begin) return 0;
                return Math.min(1, (root.progress - begin) / (1 - begin));
            }

            opacity: dismissed
                ? 0
                : root.progress * (1 - Math.min(1, Math.abs(dismissY) / (height * 0.8)))
            transform: Translate { y: card.dismissY }
            // 0.86 -> 1.0, the reference's numbers, driven by the shared
            // progress rather than a timer per card.
            scale: 0.86 + 0.14 * card.intro
            Behavior on opacity {
                NumberAnimation { duration: 160 }
            }
            NumberAnimation {
                id: settle
                target: card
                property: "dismissY"
                to: 0
                duration: 180
                easing.type: Easing.OutCubic
            }
            NumberAnimation {
                id: fling
                target: card
                property: "dismissY"
                to: -card.height * 1.2
                duration: 200
                easing.type: Easing.InCubic
                onFinished: {
                    card.dismissed = true;
                    root.closed(card.modelData);
                }
            }

            width: list.width
            height: list.height

            Rectangle {
                anchors.fill: parent
                radius: Appearance?.rounding?.normal ?? 12
                color: Appearance?.colors?.colLayer1 ?? "#1e1e24"
                border.width: 1
                border.color: modelData?.activated
                    ? (Appearance?.colors?.colPrimary ?? "#9ec5ff")
                    : "transparent"
            }

            ScreencopyView {
                id: preview
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 6
                anchors.bottom: label.top
                anchors.bottomMargin: 6
                // Only while the overview is up. A live capture per window is
                // a render of that window every frame, and leaving them running
                // behind a closed overview is the battery cost of drawing
                // things nobody can see.
                captureSource: root.progress > 0 ? card.modelData : null
                live: root.progress > 0

                // The preview is the target: tapping the picture of a window is
                // how you go to it.
                MouseArea {
                    anchors.fill: parent
                    // Vertical drag dismisses, tap activates. Threshold left to
                    // the platform so it agrees with every other scroller.
                    property real pressY: 0
                    property bool dragging: false
                    onPressed: mouse => {
                        pressY = mouse.y;
                        dragging = false;
                    }
                    onPositionChanged: mouse => {
                        const dy = mouse.y - pressY;
                        if (!dragging && Math.abs(dy) > 8)
                            dragging = true;
                        if (!dragging)
                            return;
                        // Follows the finger upward; downward it rubber-bands
                        // and stops at 56, so a card cannot be dragged into the
                        // one below it.
                        card.dismissY = dy < 0 ? dy : Math.min(56, dy * 0.4);
                    }
                    onReleased: {
                        if (!dragging) {
                            root.activated(card.modelData);
                            return;
                        }
                        if (card.dismissY < -card.dismissDistance)
                            fling.start();
                        else
                            settle.start();
                    }
                    onCanceled: settle.start()
                }
            }

            StyledText {
                id: label
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.margins: 8
                horizontalAlignment: Text.AlignHCenter
                elide: Text.ElideRight
                text: card.modelData?.title || card.modelData?.appId || qsTr("Window")
                opacity: 0.85
            }

            // Close, on the card rather than behind a gesture. The reference
            // shell flicks a card up to dismiss; that wants a drag controller
            // and an animation budget, and a visible target is the thing that
            // works on the first try.
            RippleButton {
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: 10
                implicitWidth: 34
                implicitHeight: 34
                buttonRadius: 17
                onClicked: root.closed(card.modelData)
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    text: "close"
                    iconSize: 18
                }
            }
        }
    }
}
