// The colour ramp: dimming below the backlight's floor, and the evening warmth.
//
// **The name is upstream's and it is now a lie.** There is no hyprsunset here,
// and no `hyprctl` to reach one. It keeps the name because every consumer in the
// tree — `QuickSliders`, `BarContent`, `NightLightToggle`, `BrightnessIndicator`,
// `OnScreenDisplay` — spells it, and renaming a singleton across the base tree
// and the phone overlay is a separate sweep from making the control work. What
// this file is: the same public surface (`gamma`, `gammaLowerLimit`, `setGamma`,
// `temperatureActive`, `toggleTemperature`, `fetchState`, `load`, and the
// `gammaChangeAttempt` signal) pointed at the authority that actually owns the
// LUT.
//
// What it replaces: `hyprctl hyprsunset gamma|temperature`, plus a `pidof
// hyprsunset || hyprsunset` that spawned a daemon on every call. Under viewtop
// that binary does not exist, so **the slider and the night-light have done
// nothing at all since the viewtop move** — silently, because `execDetached`
// cannot fail loudly. Gamma is a pixel claim on the glass, so it goes to the
// compositor, which serves it as a verb and applies it to the CRTC's own LUT.
// TASK-61 Part 1/5.
//
// One writer, one ramp. Brightness scaling and the evening warmth are *not* two
// controls here — they are two curve generators composed into a single LUT on
// the far side, which is why every path below funnels through `_apply()` and
// sends both numbers at once. Two callers each owning half of one hardware slot
// is trap #5 in START-HERE, and it has cost this project three sessions.
pragma Singleton

import Quickshell
import QtQuick
import qs.modules.common

Singleton {
    id: root
    signal gammaChangeAttempt()

    // The panel's backlight has a floor, and below it the only way further down
    // is the ramp. That is what the bottom 30% of the brightness slider drives,
    // so gamma is a *dimming* control here before it is a colour one.
    readonly property real gammaLowerLimit: 25

    property string from: Config.options?.light?.night?.from ?? "19:00"
    property string to: Config.options?.light?.night?.to ?? "06:30"
    property bool automatic: Config.options?.light?.night?.automatic && (Config?.ready ?? true)
    property int colorTemperature: Config.options?.light?.night?.colorTemperature ?? 5000
    // Upstream sent 6000 K to mean "off", because hyprsunset had no way to say
    // *balanced*. The verb does: omitting the temperature leaves the channels
    // untouched. Kept because the config surface still names it, but nothing
    // below sends it as a value any more — off is absence, not a warm-ish number.
    property int defaultColorTemperature: 6000
    property int gamma: 100
    property bool shouldBeOn
    property bool firstEvaluation: true
    property bool temperatureActive: false

    property int fromHour: Number(from.split(":")[0])
    property int fromMinute: Number(from.split(":")[1])
    property int toHour: Number(to.split(":")[0])
    property int toMinute: Number(to.split(":")[1])

    property int clockHour: DateTime.clock.hours
    property int clockMinute: DateTime.clock.minutes

    property var manualActive
    property int manualActiveHour
    property int manualActiveMinute

    onClockMinuteChanged: reEvaluate()
    onAutomaticChanged: {
        root.manualActive = undefined;
        root.firstEvaluation = true;
        reEvaluate();
    }

    function inBetween(t, from, to) {
        if (from < to) {
            return (t >= from && t <= to);
        } else {
            // Wrapped around midnight
            return (t >= from || t <= to);
        }
    }

    function reEvaluate() {
        const t = clockHour * 60 + clockMinute;
        const from = fromHour * 60 + fromMinute;
        const to = toHour * 60 + toMinute;
        const manualActive = manualActiveHour * 60 + manualActiveMinute;

        if (root.manualActive !== undefined && (inBetween(from, manualActive, t) || inBetween(to, manualActive, t))) {
            root.manualActive = undefined;
        }
        root.shouldBeOn = inBetween(t, from, to);
        if (firstEvaluation) {
            firstEvaluation = false;
            root.ensureState();
        }
    }

    onShouldBeOnChanged: ensureState()
    function ensureState() {
        if (!root.automatic || root.manualActive !== undefined)
            return;
        if (root.shouldBeOn) {
            root.enableTemperature();
        } else {
            root.disableTemperature();
        }
    }

    // Push the ramp we believe in to the compositor.
    //
    // The single write path. Both halves travel together because the far side
    // composes them into one curve — sending them separately would mean the
    // second call overwriting the first's contribution.
    function _apply() {
        ViewtopControl.gamma(root.gamma, root.temperatureActive ? root.colorTemperature : 0);
    }

    function load() {
        // No daemon to start any more; the compositor is already running, and if
        // it is not there is nothing a shell could do about it. Ask what ramp is
        // in force before deciding anything, then let the calendar rule.
        ViewtopControl.refreshState();
        root.ensureState();
    }

    // Take the compositor's answer as the truth about the ramp.
    //
    // Rule 2 of the house style: never hold state the protocol owns. The
    // compositor is the LUT's only writer and its ramp starts at identity, so a
    // compositor restart — an ordinary event, and how every viewtop upgrade
    // lands — leaves this singleton believing in a warmth the panel no longer
    // has. It would then refuse to re-warm, because it already thought it had.
    //
    // Adopting rather than re-asserting is also what keeps this to one writer:
    // if the adopted state is not what the calendar wants, `ensureState()` sets
    // it right on the next minute tick. The two ends converge instead of
    // fighting over the slot.
    Connections {
        target: ViewtopControl
        function onGammaValueChanged() { root._adopt(); }
        function onGammaTemperatureChanged() { root._adopt(); }
    }

    function _adopt() {
        // -1 is "never answered". Adopting it would drive the slider to a value
        // no panel ever had.
        if (ViewtopControl.gammaValue < 0)
            return;
        root.gamma = ViewtopControl.gammaValue;
        root.temperatureActive = ViewtopControl.gammaTemperature > 0;
    }

    function enableTemperature() {
        root.temperatureActive = true;
        root._apply();
    }

    function disableTemperature() {
        root.temperatureActive = false;
        root._apply();
    }

    function setGamma(gamma) {
        root.gamma = Math.max(root.gammaLowerLimit, Math.min(100, gamma));

        root.gammaChangeAttempt();

        root._apply();
    }

    // Ask the compositor what is in force. Answers asynchronously, into
    // `_adopt()` above — the toggle that calls this wants the panel's truth, not
    // this file's memory of it.
    function fetchState() {
        ViewtopControl.refreshState();
    }

    function toggleTemperature(active = undefined) {
        if (root.manualActive === undefined) {
            root.manualActive = root.temperatureActive;
            root.manualActiveHour = root.clockHour;
            root.manualActiveMinute = root.clockMinute;
        }

        root.manualActive = active !== undefined ? active : !root.manualActive;
        if (root.manualActive) {
            root.enableTemperature();
        } else {
            root.disableTemperature();
        }
    }

    // Change temp while the evening is already on. Re-sends the whole ramp
    // rather than a temperature alone, for the reason `_apply()` gives.
    Connections {
        target: Config.options.light.night
        function onColorTemperatureChanged() {
            if (!root.temperatureActive) return;
            root._apply();
        }
    }
}
