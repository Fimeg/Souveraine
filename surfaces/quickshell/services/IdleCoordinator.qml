// Souveraine's staged idle projection.
//
// It does not replace logind or hypridle. It gives all surfaces one state
// vocabulary while target-specific adapters evolve. Native idle-notify stays
// opt-in until it is verified on the Pixel compositor.
//
// The state graph extends into sleep/suspend when SessionEvents is present:
//   Active → Dimmed → LockRequested → LockSecure → Suspending → Asleep → Waking → Active
// The sleep states are driven by logind's PrepareForSleep signal via
// SessionEvents.qml; they are not reachable from idle timers alone.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs
import qs.modules.common
import qs.modules.common.functions

Singleton {
    id: root

    enum State { Active, Dimmed, LockRequested, LockSecure, Suspending, Asleep, Waking }

    property int state: IdleCoordinator.Active
    readonly property bool nativeEnabled: Config.options.lock.idle.nativeCoordinatorEnabled
    readonly property bool lockSecure: GlobalStates.screenLockSecure
    readonly property bool lockRequested: GlobalStates.screenLocked

    signal dimRequested()
    signal activeRequested()
    signal stateTransitioned(int state)

    // Whether we lowered the backlight, so Active only restores what
    // Dimmed saved — never a stale brightnessctl snapshot.
    property bool displayDimmed: false

    // Legal transitions. Each key maps to the set of states it may
    // move to. Anything not in this map is a bug — two event sources
    // racing in the same frame, a stale timer firing after a lock, or
    // a new code path that forgot to check preconditions.
    //
    // The graph:
    //   Active → Dimmed → LockRequested → LockSecure → Suspending → Asleep → Waking → Active
    //   Any locked state can return to Active on unlock.
    //   Suspending is reachable from any pre-sleep state (logind is the authority).
    readonly property var legalTransitions: ({
        0: [1, 2, 5],                // Active → Dimmed, LockRequested, Suspending
        1: [0, 2, 5],                // Dimmed → Active, LockRequested, Suspending
        2: [0, 3, 5],                // LockRequested → Active, LockSecure, Suspending
        3: [0, 5],                   // LockSecure → Active, Suspending
        4: [5, 6],                   // Suspending → Asleep, Waking
        5: [6],                      // Asleep → Waking
        6: [0],                      // Waking → Active
    })

    function setState(next) {
        if (root.state === next) return;
        const allowed = root.legalTransitions[root.state];
        if (allowed && allowed.indexOf(next) === -1) {
            console.warn("[idle-coordinator] ILLEGAL transition "
                + root.state + " → " + next + " (ignored)");
            return;
        }
        const prev = root.state;
        root.state = next;
        // Publish the coarse in-use bool the ii-base pollers gate on
        // (quickshell-idle-power task 4). Waking counts as active so stats
        // are fresh by the time the screen is visible again.
        GlobalStates.displayActive =
            (next === IdleCoordinator.Active || next === IdleCoordinator.Waking);
        root.stateTransitioned(next);
        console.log("[idle-coordinator] state=" + next + " (from=" + prev + ")");

        if (next === IdleCoordinator.Dimmed) {
            dimProc.action = "dim";
            dimProc.command = ["brightnessctl", "-q", "-s", "set",
                               Config.options.lock.idle.dimBrightness];
            dimProc.running = true;
            root.displayDimmed = true;
        } else if (next === IdleCoordinator.Active && root.displayDimmed) {
            dimProc.action = "restore";
            dimProc.command = ["brightnessctl", "-q", "-r"];
            dimProc.running = true;
            root.displayDimmed = false;
        }
    }

    Process {
        id: dimProc
        property string action: ""
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0)
                console.log("[idle-coordinator] brightness " + dimProc.action
                    + " failed (exit " + exitCode + ")");
        }
    }

    function returnActive() {
        // Only return to Active from states that are legitimately
        // "waiting for user input" — Dimmed or Waking. Never from
        // locked/sleeping states; those are guarded by the transition
        // table, but this check makes the intent explicit.
        if (root.state !== IdleCoordinator.Dimmed
            && root.state !== IdleCoordinator.Waking) return;
        root.setState(IdleCoordinator.Active);
        root.activeRequested();
    }

    // Keep System Awake is checked inside the handlers, NOT bound to
    // `enabled`: flipping enabled destroys/recreates the ext-idle-notify
    // object, and doing that during lock teardown (ii's LockScreen toggles
    // Idle.inhibit) races the compositor into a fatal "invalid object"
    // protocol error that kills the whole shell. The Wayland idle-inhibitor
    // surface is not honored on the Pixel compositor, so respectInhibitors
    // alone can't see the toggle either (see Idle.qml).
    IdleMonitor {
        id: dimMonitor
        enabled: root.nativeEnabled
        // Derived, never stored: the dim is a grace before the lock, so it
        // cannot be set past it. Clamped to leave at least a second of dim —
        // a grace >= the lock budget would otherwise mean dimming before the
        // user stopped touching the phone.
        timeout: Math.max(1, Math.min(
            Config.options.lock.idle.lockAfterSeconds - 1,
            Config.options.lock.idle.lockAfterSeconds
                - Config.options.lock.idle.dimBeforeLockSeconds)) * 1000
        respectInhibitors: true
        onIsIdleChanged: {
            if (isIdle && !root.lockRequested && !Idle.inhibit) {
                root.setState(IdleCoordinator.Dimmed);
                root.dimRequested();
            } else if (!isIdle) {
                root.returnActive();
            }
        }
    }

    IdleMonitor {
        id: lockMonitor
        enabled: root.nativeEnabled
        timeout: Math.max(1, Config.options.lock.idle.lockAfterSeconds) * 1000
        respectInhibitors: true
        onIsIdleChanged: {
            if (isIdle && !root.lockRequested && !Idle.inhibit) {
                root.setState(IdleCoordinator.LockRequested);
                Session.lock();
            } else if (!isIdle) {
                root.returnActive();
            }
        }
    }

    Connections {
        target: GlobalStates
        function onScreenLockedChanged() {
            if (GlobalStates.screenLocked)
                root.setState(IdleCoordinator.LockRequested);
            else
                root.returnActive();
        }
        function onScreenLockSecureChanged() {
            if (GlobalStates.screenLockSecure)
                root.setState(IdleCoordinator.LockSecure);
        }
    }

    // Logind sleep/suspend lifecycle. SessionEvents drives these states
    // when PrepareForSleep fires; they are unreachable without it.
    // Wires to SessionEvents once that singleton exists.
    Connections {
        target: typeof SessionEvents !== "undefined" ? SessionEvents : null
        function onPrepareForSleep(suspending) {
            if (suspending) {
                root.setState(IdleCoordinator.Suspending);
            } else {
                // Waking from sleep. The lock may or may not still be
                // held — returnActive() checks that before clearing.
                root.setState(IdleCoordinator.Waking);
                // Brief waking state before returning to the idle graph.
                // Surfaces can animate a wake transition during this window.
                wakeResetTimer.start();
            }
        }
    }

    Timer {
        id: wakeResetTimer
        interval: 1500
        repeat: false
        onTriggered: {
            if (root.state === IdleCoordinator.Waking)
                root.returnActive();
        }
    }
}
