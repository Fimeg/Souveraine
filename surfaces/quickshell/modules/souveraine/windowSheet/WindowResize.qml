// Resize as a drag, not as a cycle.
//
// The sheet's Resize button used to tap through half → two-thirds → full,
// always anchored at the zone's top-left. Casey, 2026-08-16: *"defaults to
// taking the whole top? I was thinking it'd be a draggable resize"*. So this is
// the corner handle the old comment owed.
//
// Shaped like Move rather than like a dialog: the sheet closes, this takes the
// glass, and a Done bar is the way out. Two finger-driven modes never run at
// once, which is what let Move stay a compositor grab and this stay a shell
// computation.
//
// The shell owns the rect while a handle is held. Seeded from the compositor's
// report at entry and pushed back on every frame, but never re-read mid-drag —
// the push channel would otherwise hand back a rect one round-trip stale and
// the corner would stutter against the finger.
import QtQuick
import Quickshell
import Quickshell.Wayland
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Scope {
    id: scope

    property int target: 0
    property bool active: false
    signal finished

    // Output-space, the same coordinates the compositor reports `at`/`size` in.
    property real rx: 0
    property real ry: 0
    property real rw: 0
    property real rh: 0

    // Small enough to be a corner, large enough to still be an app.
    readonly property real minWidth: 180
    readonly property real minHeight: 140
    // How near a detent counts as on it. One thumb-width of slop; below about
    // 12 px the snap cannot be felt and above about 24 it fights free dragging.
    readonly property real snapSlop: 18

    // What the bar reserved. `ZoneTransition` measured it from the overview
    // surface and is the better number; until that surface has been realised
    // once it has not measured anything, and the shell's own bar height is what
    // produced the reservation in the first place.
    readonly property real zoneTop: ZoneTransition.surfaceReported
        ? ZoneTransition.contentTop : Appearance.sizes.barHeight
    readonly property real zoneLeft: 0
    readonly property real zoneWidth: ZoneTransition.panelWidth
    readonly property real zoneHeight: ZoneTransition.panelHeight - scope.zoneTop

    function beginFor(id) {
        for (const w of ViewtopControl.windows) {
            if (w.id === id && w.at && w.size) {
                scope.target = id;
                scope.rx = w.at.x;
                scope.ry = w.at.y;
                scope.rw = w.size.width;
                scope.rh = w.size.height;
                scope.active = true;
                pump.running = true;
                return true;
            }
        }
        console.log("[window-resize] no geometry for " + id + "; not entering");
        return false;
    }

    function done() {
        pump.running = false;
        // The last frame's rect may never have been sent — the pump only fires
        // on a change, and a release that lands on the same pixel as the last
        // frame would leave the window one step behind the handle.
        scope._push(true);
        scope.active = false;
        scope.target = 0;
        Haptics.confirm();
        scope.finished();
    }

    // Edges the corner is allowed to rest on: the zone's own, its middle, and
    // the thirds. A window that lands exactly on a half is the one thing you
    // cannot hit reliably by hand, and it is what most resizes are reaching for.
    function _detents(from, span) {
        return [from, from + span / 3, from + span / 2,
            from + 2 * span / 3, from + span];
    }

    function _snap(value, from, span) {
        let best = value;
        let bestGap = scope.snapSlop;
        for (const d of scope._detents(from, span)) {
            const gap = Math.abs(value - d);
            if (gap < bestGap) {
                bestGap = gap;
                best = d;
            }
        }
        return best;
    }

    // Whether the last snap actually moved the value, so the tick fires on
    // arriving at a detent rather than once per frame while resting on one.
    property bool _onDetent: false

    function _dragTo(anchorX, anchorY, px, py) {
        const sx = scope._snap(px, scope.zoneLeft, scope.zoneWidth);
        const sy = scope._snap(py, scope.zoneTop, scope.zoneHeight);
        const snapped = (sx !== px) || (sy !== py);
        if (snapped && !scope._onDetent)
            Haptics.tick();
        scope._onDetent = snapped;

        const x = Math.max(scope.zoneLeft,
            Math.min(scope.zoneLeft + scope.zoneWidth, sx));
        const y = Math.max(scope.zoneTop,
            Math.min(scope.zoneTop + scope.zoneHeight, sy));

        let left = Math.min(anchorX, x);
        let right = Math.max(anchorX, x);
        let top = Math.min(anchorY, y);
        let bottom = Math.max(anchorY, y);

        // The minimum grows away from the anchor, never toward it — clamping
        // symmetrically would walk the anchored corner across the glass as soon
        // as the drag hit the floor.
        if (right - left < scope.minWidth) {
            if (x < anchorX)
                left = right - scope.minWidth;
            else
                right = left + scope.minWidth;
        }
        if (bottom - top < scope.minHeight) {
            if (y < anchorY)
                top = bottom - scope.minHeight;
            else
                bottom = top + scope.minHeight;
        }

        scope.rx = left;
        scope.ry = top;
        scope.rw = right - left;
        scope.rh = bottom - top;
    }

    // One `place` per painted frame, and only when the rect moved.
    //
    // The socket is not the constraint — 0.47 ms median, measured on blueline
    // 2026-08-16 — but a send per motion event is still a send per motion
    // event, and `ViewtopControl` holds one request in flight at a time. The
    // render clock is the rate the eye can use and it is the rate the carry
    // already runs at (`ZoneTransition`'s settle), so this pumps the same way.
    property real _sentX: -1
    property real _sentY: -1
    property real _sentW: -1
    property real _sentH: -1

    function _push(force) {
        if (scope.target <= 0)
            return;
        if (!force
            && Math.abs(scope.rx - scope._sentX) < 1
            && Math.abs(scope.ry - scope._sentY) < 1
            && Math.abs(scope.rw - scope._sentW) < 1
            && Math.abs(scope.rh - scope._sentH) < 1)
            return;
        scope._sentX = scope.rx;
        scope._sentY = scope.ry;
        scope._sentW = scope.rw;
        scope._sentH = scope.rh;
        ViewtopControl.place(scope.target, Math.round(scope.rx),
            Math.round(scope.ry), Math.round(scope.rw), Math.round(scope.rh));
    }

    FrameAnimation {
        id: pump
        running: false
        onTriggered: scope._push(false)
    }

    // The window went away under the drag. Leave rather than keep resizing a
    // dead id, which would refuse once per frame.
    Connections {
        target: ViewtopControl
        function onRefused(intent, reason) {
            if (scope.active && intent === "place") {
                console.log("[window-resize] place refused (" + reason + "); leaving");
                pump.running = false;
                scope.active = false;
                scope.target = 0;
                Haptics.refuse();
                scope.finished();
            }
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: surface
            required property var modelData
            screen: surface.modelData

            anchors { top: true; left: true; right: true; bottom: true }
            color: "transparent"
            visible: scope.active
            WlrLayershell.namespace: "souveraine:windowresize"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            exclusionMode: ExclusionMode.Ignore

            // Everything outside the rect, dimmed — so the thing being sized
            // reads as the subject and the zone behind it as the room it has to
            // fit in. Four bands rather than a full-screen scrim with a hole,
            // because a hole needs a mask and the phone's GLES path refuses the
            // effects that would draw one (the sheet's own header says why).
            Repeater {
                model: [
                    { x: 0, y: 0, w: surface.width, h: scope.ry },
                    { x: 0, y: scope.ry + scope.rh, w: surface.width,
                      h: Math.max(0, surface.height - scope.ry - scope.rh) },
                    { x: 0, y: scope.ry, w: Math.max(0, scope.rx), h: scope.rh },
                    { x: scope.rx + scope.rw, y: scope.ry,
                      w: Math.max(0, surface.width - scope.rx - scope.rw),
                      h: scope.rh }
                ]

                delegate: Rectangle {
                    required property var modelData
                    x: modelData.x
                    y: modelData.y
                    width: modelData.w
                    height: modelData.h
                    color: "#8c000000"
                }
            }

            // The rect itself: an outline and four brackets. The window keeps
            // rendering underneath, live, so what you are sizing is the app and
            // not a placeholder of it.
            Item {
                id: frame
                x: scope.rx
                y: scope.ry
                width: scope.rw
                height: scope.rh

                Rectangle {
                    anchors.fill: parent
                    color: "transparent"
                    border.width: 2
                    border.color: Appearance.colors.colOnLayer1
                    radius: 12
                }

                StyledText {
                    anchors.centerIn: parent
                    text: Math.round(scope.rw) + " × " + Math.round(scope.rh)
                    color: "#e6ffffff"
                    font.pixelSize: Appearance.font.pixelSize.large
                    opacity: 0.9
                }

                // One corner. `ox`/`oy` are which corner it is, as 0 or 1, so
                // the anchor is simply the other one.
                component Corner: Item {
                    id: corner
                    required property int ox
                    required property int oy

                    readonly property real hit: 64

                    x: corner.ox * frame.width - corner.hit / 2
                    y: corner.oy * frame.height - corner.hit / 2
                    width: corner.hit
                    height: corner.hit

                    Canvas {
                        id: bracket
                        anchors.fill: parent
                        onPaint: {
                            const ctx = getContext("2d");
                            ctx.reset();
                            const c = corner.hit / 2;
                            const arm = 16;
                            const dx = corner.ox === 0 ? 1 : -1;
                            const dy = corner.oy === 0 ? 1 : -1;
                            ctx.beginPath();
                            ctx.moveTo(c, c + dy * arm);
                            ctx.lineTo(c, c);
                            ctx.lineTo(c + dx * arm, c);
                            ctx.lineWidth = 4;
                            ctx.lineCap = "round";
                            ctx.strokeStyle = Appearance.colors.colOnLayer1;
                            ctx.stroke();
                        }
                    }

                    Rectangle {
                        anchors.centerIn: parent
                        width: 30
                        height: 30
                        radius: 15
                        color: Appearance.colors.colOnLayer1
                        opacity: grip.pressed ? 0.30 : 0
                        Behavior on opacity {
                            NumberAnimation {
                                duration: Appearance.animation.elementMoveFast.duration
                                easing.type: Appearance.animation.elementMoveFast.type
                                easing.bezierCurve: Appearance.animation.elementMoveFast.bezierCurve
                            }
                        }
                    }

                    scale: grip.pressed ? 1.18 : 1
                    Behavior on scale {
                        NumberAnimation {
                            duration: Appearance.animation.clickBounce.duration
                            easing.type: Appearance.animation.clickBounce.type
                            easing.bezierCurve: Appearance.animation.clickBounce.bezierCurve
                        }
                    }

                    MouseArea {
                        id: grip
                        anchors.fill: parent
                        // Latched at the press: the anchor is the opposite
                        // corner of the rect as it was when the finger landed,
                        // and re-reading it from the live rect would let it
                        // chase its own output.
                        property real anchorX: 0
                        property real anchorY: 0

                        onPressed: {
                            grip.anchorX = corner.ox === 0
                                ? scope.rx + scope.rw : scope.rx;
                            grip.anchorY = corner.oy === 0
                                ? scope.ry + scope.rh : scope.ry;
                            scope._onDetent = false;
                            Haptics.tick();
                        }

                        onPositionChanged: mouse => {
                            const p = grip.mapToItem(surface.contentItem,
                                mouse.x, mouse.y);
                            scope._dragTo(grip.anchorX, grip.anchorY, p.x, p.y);
                        }
                    }
                }

                Corner { ox: 0; oy: 0 }
                Corner { ox: 1; oy: 0 }
                Corner { ox: 0; oy: 1 }
                Corner { ox: 1; oy: 1 }
            }

            // The way out. Same shape and same place as Move's, because they
            // are the same kind of mode and a second idea of "how do I leave
            // this" is one the thumb has to learn twice.
            Rectangle {
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 14
                implicitWidth: 220
                implicitHeight: 58
                radius: 29
                color: Appearance.colors.colLayer1
                border.width: 1
                border.color: Appearance.colors.colLayer1Active

                StyledText {
                    anchors.centerIn: parent
                    text: qsTr("Done resizing")
                    color: Appearance.colors.colOnLayer1
                    font.pixelSize: Appearance.font.pixelSize.normal
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: scope.done()
                }
            }
        }
    }
}
