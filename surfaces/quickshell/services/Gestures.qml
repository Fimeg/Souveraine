// Gesture consumer — the userspace side of squeeze, and the seam every other
// physical gesture arrives through.
//
// This is deliberately written before the producer exists. TASK-13 has the
// Active Edge rail powered (PM8998 GPIO 2, held from boot) and the SSC
// registry admitting `sns_touch_gesture`, but nothing had anywhere to deliver
// a squeeze TO — and a bring-up with no consumer is a sensor that fires into
// nothing, which is exactly why grip stalled. The contract goes first now, so
// the producer has a defined target and can be tested the moment it works.
//
// The producer is not specified here on purpose. Anything that can reach the
// shell's IPC socket can deliver a gesture: a libssc client, a udev-spawned
// helper, an evdev reader, or a human running `qs ipc call gesture squeeze`
// to test the routing without any of that existing. That is the same posture
// the sensor reporters take — the source is replaceable, the contract is not.
//
// Routing is NOT hardcoded per gesture. `squeezeAction` names an action, and
// the action table below is the one place a gesture's meaning is decided, so
// a squeeze can be re-pointed without editing call sites. See the Keyboard
// System note in TASK-13 step 4: a squeeze must register as an input, not as
// a hardcoded binding, or it collides with everything else that wants it.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.modules.common

Singleton {
    id: root

    // What each gesture does. Names, not closures, so the choice is data a
    // settings page or an agent can read and change.
    property string squeezeAction: "dial"
    property string squeezeHoldAction: "assistant"
    property string edgeAction: "dial"
    // Last gesture seen, so a surface can show that the hardware is alive even
    // before anything is bound to it — the difference between "grip does
    // nothing" and "grip is not reaching us" is the whole debugging story.
    property string lastGesture: ""
    property double lastGestureAt: 0

    signal gestureReceived(string name)

    readonly property var actions: ({
        "dial": () => {
            Haptics.tick();
            Quickshell.execDetached(["qs", "-c", "souveraine", "ipc",
                                     "--any-display", "call", "dial", "toggle"]);
        },
        "assistant": () => {
            Haptics.tick();
            GlobalStates.sidebarLeftOpen = !GlobalStates.sidebarLeftOpen;
        },
        "keyboard": () => {
            Haptics.tick();
            GlobalStates.oskOpen = !GlobalStates.oskOpen;
        },
        "screenshot": () => {
            Haptics.confirm();
            Quickshell.execDetached(["sh", "-c",
                "grim ~/Pictures/screenshot-$(date +%Y%m%d-%H%M%S).png"]);
        },
        "none": () => {}
    })

    // One entry point for every gesture, so the trail of "what arrived" is in
    // one place and a refusal is legible rather than a silent no-op.
    function deliver(name, action) {
        root.lastGesture = name;
        root.lastGestureAt = Date.now();
        console.log("[gesture] " + name + " -> " + action);
        root.gestureReceived(name);

        const fn = root.actions[action];
        if (!fn) {
            console.log("[gesture] no action named '" + action + "'");
            Haptics.refuse();
            return false;
        }
        fn();
        return true;
    }

    IpcHandler {
        target: "gesture"

        // The squeeze. Called by whatever ends up producing it.
        function squeeze(): string {
            return JSON.stringify({
                ok: root.deliver("squeeze", root.squeezeAction),
                action: root.squeezeAction
            });
        }

        // A held squeeze is a different gesture, not a longer one.
        function squeezeHold(): string {
            return JSON.stringify({
                ok: root.deliver("squeeze-hold", root.squeezeHoldAction),
                action: root.squeezeHoldAction
            });
        }

        function edge(): string {
            return JSON.stringify({
                ok: root.deliver("edge", root.edgeAction),
                action: root.edgeAction
            });
        }

        // What is bound to what, and what was last seen. An agent or a
        // settings page reads this instead of guessing.
        function state(): string {
            return JSON.stringify({
                squeeze: root.squeezeAction,
                squeezeHold: root.squeezeHoldAction,
                edge: root.edgeAction,
                available: Object.keys(root.actions),
                lastGesture: root.lastGesture,
                lastGestureAt: root.lastGestureAt
            });
        }

        // Re-point a gesture without editing code.
        function bind(gesture: string, action: string): string {
            if (!root.actions[action])
                return JSON.stringify({ ok: false, reason: "no such action: " + action });
            switch (gesture) {
                case "squeeze":      root.squeezeAction = action; break;
                case "squeeze-hold": root.squeezeHoldAction = action; break;
                case "edge":         root.edgeAction = action; break;
                default:
                    return JSON.stringify({ ok: false, reason: "no such gesture: " + gesture });
            }
            return JSON.stringify({ ok: true, gesture: gesture, action: action });
        }
    }
}
