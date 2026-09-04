import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Idle & sleep — the staged idle projection, exposed honestly.
//
// The governing idea is REFERENCE-EXTRACTION.md's "idle is a transition graph,
// not a timer": the page shows the live stage the shell is actually in, not
// just three timeout knobs pretending idle is linear.
//
// Two honesty constraints drive the layout, and both come straight from the
// extraction's build order (truth before visuals):
//
//   1. Two authorities, two sections, never blended. The shell's own
//      IdleMonitors decide when a session IN USE dims and locks; sessiond's
//      device state machine decides how long a LOCKED panel may burn. The
//      second set is read live from the daemon (SessiondPolicy) rather than
//      from a config file, because the daemon is what actuates them —
//      TASK-19's "no success-shaped switches".
//
//      hypridle no longer owns screen-off: its idle listeners were deleted
//      2026-07-25 once sessiond got real actuators, because "lock, then off"
//      held only while its 300s lock happened to precede its 600s blank.
//
//   2. Settings is a window in the authoritative shell process, so this page
//      reads the idle/session singletons directly. It must not spawn `qs ipc`
//      children, which can become accidental shell instances when display
//      selection is ambiguous.
ContentPage {
    id: page
    forceWidth: true

    // --- Live stage readout ----------------------------------------------
    // 0 Active · 1 Dimmed · 2 Lock requested · 3 Lock secure ·
    // 4 Suspending · 5 Asleep · 6 Waking — the IdleCoordinator.State
    // enum, read directly from the shell-owned coordinator.
    readonly property int liveStage: IdleCoordinator.state
    readonly property bool liveNative: IdleCoordinator.nativeEnabled
    readonly property bool probeOk: true
    readonly property bool sleepInhibitorHeld: SessionEvents.sleepInhibitorHeld
    readonly property bool stepUpEnabled: StepUpAuth.grantTtlMs > 0

    // Seconds on the wire, human words on screen. "Never" is 0, which is what
    // the daemon already means by it.
    readonly property var blankPresets: [
        { displayName: Translation.tr("15s"),    icon: "timer",      value: 15 },
        { displayName: Translation.tr("30s"),    icon: "timer",      value: 30 },
        { displayName: Translation.tr("1 min"),  icon: "timer",      value: 60 },
        { displayName: Translation.tr("5 min"),  icon: "timer",      value: 300 },
        { displayName: Translation.tr("Never"),  icon: "timer_off",  value: 0 }
    ]
    readonly property var idlePresets: [
        { displayName: Translation.tr("30s"),    icon: "timer", value: 30 },
        { displayName: Translation.tr("1 min"),  icon: "timer", value: 60 },
        { displayName: Translation.tr("2 min"),  icon: "timer", value: 120 },
        { displayName: Translation.tr("5 min"),  icon: "timer", value: 300 },
        { displayName: Translation.tr("10 min"), icon: "timer", value: 600 }
    ]
    // ONE dim setting, for both authorities.
    //
    // This used to be two: a session dim (15s–5min, before the lock) and a
    // separate lock-screen dim grace (5–20s, before the blank). They are two
    // daemons, but they are not two questions — the user is answering "how
    // much warning do I get before the screen goes away", once. Splitting it
    // made the page describe our architecture instead of their screen.
    //
    // Written to both, unchanged: the shell's dimBeforeLockSeconds and
    // sessiond's dim_grace_secs.
    readonly property var dimPresets: [
        { displayName: Translation.tr("5s"),    icon: "brightness_medium", value: 5 },
        { displayName: Translation.tr("10s"),   icon: "brightness_medium", value: 10 },
        { displayName: Translation.tr("30s"),   icon: "brightness_medium", value: 30 },
        { displayName: Translation.tr("1 min"), icon: "brightness_medium", value: 60 },
        { displayName: Translation.tr("3 min"), icon: "brightness_medium", value: 180 }
    ]

    // Write the chosen value through, unchanged, to both authorities.
    //
    // An earlier version clamped the lock-screen grace to `blank - 1` so a
    // long dim would still "fit". That was invented policy nobody asked for
    // and it inverted the setting: a 30 s dim against a 15 s blank became a
    // 14 s grace, which starts the dim one second after you stop touching the
    // phone. A setting that silently means something else is worse than one
    // that does not apply.
    function applyDim(v) {
        Config.options.lock.idle.dimBeforeLockSeconds = v;
        if (SessiondPolicy.available)
            SessiondPolicy.apply({ dim_grace_secs: v });
    }

    // "2 min" reads better than "120 s" in a sentence about when things happen.
    function clockText(secs) {
        if (secs >= 60 && secs % 60 === 0)
            return Translation.tr("%1 min").arg(secs / 60);
        if (secs >= 60)
            return Translation.tr("%1 min %2 s").arg(Math.floor(secs / 60)).arg(secs % 60);
        return Translation.tr("%1 s").arg(secs);
    }

    readonly property var stageNames: [
        Translation.tr("Active"),
        Translation.tr("Dimmed"),
        Translation.tr("Lock requested"),
        Translation.tr("Lock secure"),
        Translation.tr("Suspending"),
        Translation.tr("Asleep"),
        Translation.tr("Waking")
    ]
    function stageLabel(s) {
        return (s >= 0 && s < stageNames.length) ? stageNames[s] : Translation.tr("unknown");
    }

    ContentSection {
        icon: "motion_sensor_active"
        title: Translation.tr("Current state")

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            StyledText {
                text: page.probeOk
                    ? Translation.tr("Idle stage: %1").arg(page.stageLabel(page.liveStage))
                    : Translation.tr("Idle stage: (shell not reachable)")
                font.pixelSize: Appearance.font.pixelSize.normal
            }
        }

        StyledText {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: page.liveNative
                ? Translation.tr("The native coordinator is driving idle transitions. Dim and lock fire on the timers below.")
                : Translation.tr("The native coordinator is off, so the session timers below do not run. Nothing else locks on idle now that hypridle's listeners are gone — turn it on, or the phone only locks when you lock it.")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            StyledText {
                text: page.sleepInhibitorHeld
                    ? Translation.tr("Sleep inhibitor: held (suspend blocked until lock is secure)")
                    : Translation.tr("Sleep inhibitor: released (suspend may proceed)")
                font.pixelSize: Appearance.font.pixelSize.normal
                color: page.sleepInhibitorHeld
                    ? Appearance.colors.colOnLayer1
                    : Appearance.colors.colSubtext
            }
        }
    }

    ContentSection {
        icon: "experiment"
        title: Translation.tr("Native idle coordinator")

        ConfigSwitch {
            buttonIcon: "science"
            text: Translation.tr("Enable native coordinator (experimental)")
            checked: Config.options.lock.idle.nativeCoordinatorEnabled
            onCheckedChanged: {
                Config.options.lock.idle.nativeCoordinatorEnabled = checked;
            }
            StyledToolTip {
                text: Translation.tr("Use Wayland idle-notify to drive the dim/lock timers. Unverified on the Pixel 3 compositor build — leave off unless you are testing it. When off, hypridle handles idle and screen-off.")
            }
        }
    }

    ContentSection {
        icon: "timer"
        title: Translation.tr("Session timers")

        // Config keys, effective only while the native coordinator is on.
        // Presets rather than a seconds spinner, same reasoning as below.
        ContentSubsectionLabel {
            text: Translation.tr("Lock the session after")
        }
        ConfigSelectionArray {
            currentValue: Config.options.lock.idle.lockAfterSeconds
            onSelected: v => Config.options.lock.idle.lockAfterSeconds = v
            options: page.idlePresets
        }

        StyledText {
            Layout.fillWidth: true
            visible: !Config.options.lock.idle.nativeCoordinatorEnabled
            wrapMode: Text.WordWrap
            color: Appearance.colors.colError
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: Translation.tr("The native coordinator is off, so these do not run.")
        }
    }

    // --- Brightness dimming, one question ---------------------------------
    // Deliberately not split by authority. Two daemons own the actuation, and
    // the page used to say so by giving each its own dim control — which made
    // the user answer the same question twice and left them to work out that
    // the two interact. One control, written to both. See applyDim().
    ContentSection {
        icon: "brightness_medium"
        title: Translation.tr("Brightness dimming")

        ConfigSwitch {
            buttonIcon: "brightness_low"
            text: Translation.tr("Dim before the screen goes away")
            checked: SessiondPolicy.dimWarning
            enabled: SessiondPolicy.available
            onCheckedChanged: {
                if (SessiondPolicy.available && checked !== SessiondPolicy.dimWarning)
                    SessiondPolicy.apply({ dim_warning: checked });
            }
        }

        ContentSubsectionLabel {
            text: Translation.tr("Start dimming this long before")
        }
        ConfigSelectionArray {
            currentValue: Config.options.lock.idle.dimBeforeLockSeconds
            onSelected: v => page.applyDim(v)
            options: page.dimPresets
        }

        // Say what it actually does, on both surfaces, in one sentence each.
        // A relationship the user has to compute in their head is one they
        // will get wrong.
        StyledText {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: {
                const lock = Config.options.lock.idle.lockAfterSeconds;
                const grace = Config.options.lock.idle.dimBeforeLockSeconds;
                const dimAt = Math.max(1, Math.min(lock - 1, lock - grace));
                return Translation.tr("In use: dims at %1, locks at %2.")
                    .arg(page.clockText(dimAt))
                    .arg(page.clockText(lock));
            }
        }

        StyledText {
            Layout.fillWidth: true
            visible: SessiondPolicy.available
            wrapMode: Text.WordWrap
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: {
                const blank = SessiondPolicy.lockBlankAfterSecs;
                if (blank === 0)
                    return Translation.tr("On the lock screen: never blanks, so nothing dims.");
                const g = SessiondPolicy.dimGraceSecs;
                return Translation.tr("On the lock screen: dims %1 before blanking at %2.")
                    .arg(page.clockText(g))
                    .arg(page.clockText(blank));
            }
        }
    }

    // --- Lock screen timers, owned by sessiond ----------------------------
    // A separate section because these are a different authority. The two
    // above are the shell's own IdleMonitors deciding when to dim and lock a
    // session in use; these are the device state machine deciding how long a
    // LOCKED, lit panel may burn before it goes dark. That machine has its own
    // clock and its own actuators, so its numbers must come from it — which is
    // what SessiondPolicy is for, and what this page did not do until
    // 2026-07-25 (SetPolicy had zero callers; the daemon ran on its built-in
    // 15 s while this page showed whatever was in the JSON file).
    Component.onCompleted: SessiondPolicy.refresh()

    ContentSection {
        icon: "phonelink_lock"
        title: Translation.tr("Lock screen (device authority)")

        StyledText {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: SessiondPolicy.available
                ? Appearance.colors.colSubtext
                : Appearance.colors.colError
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: SessiondPolicy.available
                ? Translation.tr("Read live from souveraine-sessiond. Changes here go straight to the daemon that blanks the panel.")
                : Translation.tr("sessiond is not answering — these values are NOT authoritative. %1").arg(SessiondPolicy.lastError)
        }

        // Presets, not a seconds spinner. The policy struct's own comment
        // names the shape — "iOS's Auto-Lock: a user-chosen timeout with a
        // visible dim shortly before it, and a 'never' option for the
        // desk-clock case" — and nobody reasons about a lock screen in
        // 5-second increments. Values are still seconds on the wire; the
        // daemon's vocabulary does not change because the UI got legible.
        // One blank timeout, not two.
        //
        // The daemon still has a separate held-vs-resting budget, and it is
        // still the right idea — a phone in your hand should not blank on the
        // same schedule as one face-up on a desk. It is not a *setting*
        // though. Asking the user to pick two numbers made them responsible
        // for arbitrating a guess the accelerometer was making on their
        // behalf, and the machine already has a better vocabulary for that:
        // §4's confidence arithmetic. Held-ness belongs there, as an
        // adjustment to one budget, not as a second budget on this page.
        //
        // Until that lands, both fields get the same value, so the behaviour
        // is uniform and predictable rather than silently forking on a sensor
        // reading nothing surfaces.
        ContentSubsectionLabel {
            text: Translation.tr("Blank after")
        }
        ConfigSelectionArray {
            currentValue: SessiondPolicy.lockBlankAfterSecs
            onSelected: v => SessiondPolicy.apply({
                lock_blank_after_secs: v,
                lock_blank_after_held_secs: v
            })
            options: page.blankPresets
        }

        // Only claim the ordering guarantee when the daemon actually reports
        // the field. An older sessiond has no lock_ack_budget_secs, and
        // rendering that absence as "waits 0s" would be the page inventing a
        // number — the same class of lie as the timers this section replaced.
        StyledText {
            Layout.fillWidth: true
            visible: SessiondPolicy.available && SessiondPolicy.lockAckBudgetSecs > 0
            wrapMode: Text.WordWrap
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: Translation.tr("The screen never goes dark on an unlocked session. sessiond locks first and waits %1s for the compositor to acknowledge; if it cannot, it blanks anyway and records a security error rather than pretending the session locked.")
                .arg(SessiondPolicy.lockAckBudgetSecs)
        }

        StyledText {
            Layout.fillWidth: true
            visible: SessiondPolicy.available && SessiondPolicy.lockAckBudgetSecs === 0
            wrapMode: Text.WordWrap
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            text: Translation.tr("This sessiond predates the lock-before-blank ordering. Update the souveraine package to get it.")
        }
    }
}
