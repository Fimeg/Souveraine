// Haptic feedback, through feedbackd.
//
// Not a direct write to /dev/input/eventN. feedbackd already owns the force
// feedback device (it claimed event4 at boot the moment the kernel exposed
// pmi8998_haptics), it already arbitrates between callers, and it already
// carries the user's profile — full / quiet / silent — which is where "do not
// buzz right now" is supposed to be decided. A second writer to the same
// device would be the competing-writer mistake again, in a new subsystem.
//
// The event names are feedbackd's own vocabulary, so the theme decides what
// each one feels like. `button-pressed` maps to VibraPattern in the `quiet`
// profile, which is the same event squeekboard fires per key.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    // Off switch that does not depend on reaching feedbackd to honour it.
    property bool enabled: true

    function trigger(event) {
        if (!root.enabled)
            return;
        Quickshell.execDetached(["busctl", "call", "--user",
            "org.sigxcpu.Feedback", "/org/sigxcpu/Feedback",
            "org.sigxcpu.Feedback", "TriggerFeedback",
            "ssa{sv}i", "souveraine", event, "0", "-1"]);
    }

    // A detent. Short and light: this fires once per entry the thumb crosses
    // on the dial, so anything longer would smear into the next one.
    function tick() {
        root.trigger("button-pressed");
    }

    // Something completed.
    function confirm() {
        root.trigger("button-released");
    }

    // Something was refused. Distinct on purpose — a refusal that feels like a
    // success is worse than no feedback at all.
    function refuse() {
        root.trigger("bell-terminal");
    }
}
