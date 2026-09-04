// A window's scale, travelling instead of jumping.
//
// `pose` is instant on the far side. `set_pose` in the compositor is one
// registry update — the doctrine is that it "honours rather than adjudicates",
// so easing must not live there or the compositor would be deciding how the
// agent's own verb looks. The curve is the shell's, exactly as `ZoneTransition`
// owns the carry's curve and the compositor only carries.
//
// So Zoom fired one call and the window changed size between two frames. Casey,
// 2026-08-16: *"zoom is still snapping"*.
//
// Android never steps a scale. `RectFSpringAnim` gives scale a spring of its
// own — `SpringAnimation(this, RECT_SCALE_PROGRESS)` with
// `swipe_up_rect_scale_stiffness` 200 and `swipe_up_rect_scale_damping_ratio`
// **0.75**, separate from the position spring's 0.8 and deliberately under one:
// scale overshoots a little and settles, and that is the whole of why theirs
// reads better than a step.
//
// Same numbers here. Integrated on `FrameAnimation` — the render clock — for
// the same reason the settle is: one send per painted frame rather than one per
// whatever the socket managed.
import QtQuick
import Quickshell
import qs.services

Item {
    id: ramp

    property int target: 0
    // Where the scale is now, and where it is going. `scale` is authoritative
    // between calls: the compositor does not report pose back, so a caller that
    // re-read it would be reading its own last write anyway.
    property real scale: 1.0
    property real to: 1.0
    readonly property bool running: tick.running

    // sqrt(200) in rad/s, expressed per millisecond to match the rest of the
    // shell's integrators. Damping under 1 on purpose — see the header.
    readonly property real omega: 0.01414
    readonly property real damping: 0.75
    // Nothing outlives this. A spring that failed to converge still has to
    // leave the window at a scale someone asked for.
    readonly property int maxMs: 900
    // Below this the change is not visible on a 540 px panel and is not worth a
    // round trip.
    readonly property real epsilon: 0.0015

    property real _velocity: 0
    property real _elapsed: 0
    property real _sent: -1

    // Seeded at rest by default: Zoom is a tap, and a tap has no throw in it.
    function springTo(value, velocity) {
        if (ramp.target <= 0)
            return;
        ramp.to = value;
        ramp._velocity = velocity ?? 0;
        ramp._elapsed = 0;
        tick.running = true;
    }

    function reset(id) {
        tick.running = false;
        ramp.target = id;
        ramp.scale = 1.0;
        ramp.to = 1.0;
        ramp._velocity = 0;
        ramp._sent = -1;
    }

    function _push(force) {
        if (ramp.target <= 0)
            return;
        if (!force && Math.abs(ramp.scale - ramp._sent) < ramp.epsilon)
            return;
        ramp._sent = ramp.scale;
        ViewtopControl.pose(ramp.target, ramp.scale, 0.0, 0.5, 0.5);
    }

    FrameAnimation {
        id: tick
        running: false

        onTriggered: {
            // Clamped: a dropped frame must not integrate one large step and
            // throw the window through its own target.
            const dt = Math.min(48, tick.frameTime * 1000);
            ramp._elapsed += dt;

            const w = ramp.omega;
            const offset = ramp.scale - ramp.to;
            ramp._velocity += (-2 * ramp.damping * w * ramp._velocity - w * w * offset) * dt;
            ramp.scale += ramp._velocity * dt;

            if ((Math.abs(ramp.scale - ramp.to) < 0.001
                    && Math.abs(ramp._velocity) < 0.0001)
                || ramp._elapsed >= ramp.maxMs) {
                tick.running = false;
                ramp.scale = ramp.to;
                ramp._push(true);
                return;
            }
            ramp._push(false);
        }
    }
}
