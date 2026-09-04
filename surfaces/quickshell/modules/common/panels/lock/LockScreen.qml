// Souveraine patch to ii's stock LockScreen.qml.
//
// Two changes, both for phone duty where the lock is the only gate on the
// device:
//   1. The lock state is mirrored into Persistent.states.lock.locked, so a
//      quickshell crash/restart while locked comes back locked instead of
//      dropping the session lock on the floor.
//   2. initIfReady() also locks when the persisted flag says we died locked,
//      not only on a fresh Hyprland instance.
// Everything else is unchanged stock ii.
pragma ComponentBehavior: Bound
import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland

Scope {
    id: root

    required property Component lockSurface
    property alias context: lockContext
    property Component sessionLockSurface: WlSessionLockSurface {
        id: sessionLockSurface
        color: "transparent"
        Loader {
            active: GlobalStates.screenLocked
            anchors.fill: parent
            opacity: active ? 1 : 0
            Behavior on opacity {
                animation: Appearance.animation.elementMoveFast.colorAnimation.createObject(this)
            }
            sourceComponent: root.lockSurface
        }
    }

    Process {
        id: unlockKeyringProc
        onExited: (exitCode, exitStatus) => {
            KeyringStorage.fetchKeyringData();
        }
    }
    // Set once, the first time the lock surface goes secure after boot, to
    // dismiss the BootBloom overlay. The C splash is already gone by now (it
    // released at the Act III handoff); nothing to poke here anymore.
    property bool bootDismissed: false
    function unlockKeyring() {
        unlockKeyringProc.exec({
            environment: ({
                "UNLOCK_PASSWORD": lockContext.currentText
            }),
            command: ["bash", "-c", Quickshell.shellPath("scripts/keyring/unlock.sh")]
        })
    }

    // This stores all the information shared between the lock surfaces on each screen.
    // https://github.com/quickshell-mirror/quickshell-examples/tree/master/lockscreen
    LockContext {
        id: lockContext

        Connections {
            target: GlobalStates
            function onScreenLockedChanged() {
                Persistent.states.lock.locked = GlobalStates.screenLocked;
                // Persistent is a file and answers asynchronously, so it can
                // never be the thing a reload reads at construction. This is
                // the same fact held in-process, where the reload can see it.
                lockContinuity.held = GlobalStates.screenLocked;
                if (GlobalStates.screenLocked) {
                    lockContext.reset();
                    lockContext.tryFingerUnlock();
                }
            }
        }

        onUnlocked: (targetAction) => {
            // Perform the target action if it's not just unlocking
            if (targetAction == LockContext.ActionEnum.Poweroff) {
                Session.poweroff();
                return;
            } else if (targetAction == LockContext.ActionEnum.Reboot) {
                Session.reboot();
                return;
            }

            // Unlock the keyring if configured to do so
            if (Config.options.lock.security.unlockKeyring) root.unlockKeyring(); // Async

            // Unlock the screen before exiting, or the compositor will display a
            // fallback lock you can't interact with.
            GlobalStates.screenLocked = false;

            // Reset
            lockContext.reset();

            // Post-unlock actions
            if (lockContext.alsoInhibitIdle) {
                lockContext.alsoInhibitIdle = false;
                Idle.toggleInhibit(true);
            }
        }
    }

    // TASK-48. The lock REQUEST, carried across a quickshell scene reload by
    // quickshell's own reload machinery — in-process and synchronous, which is
    // what this has to be. Declared before the WlSessionLock on purpose:
    // `Scope` is a ReloadPropagator and reloads its children in declaration
    // order, so `held` is restored and applied before the lock below reloads.
    //
    // Why any of this is needed, from reading quickshell's session_lock.cpp:
    // on reload the new WlSessionLock adopts the outgoing one's
    // SessionLockManager and then calls realizeLockTarget() with whatever
    // `locked` evaluated to at construction. GlobalStates is a fresh singleton
    // by then, so screenLocked is false, so lockTarget is false — and the
    // false branch is `unlock()`, which on an adopted manager that IS locked
    // sends ext_session_lock_v1.unlock_and_destroy. A scene reload therefore
    // UNLOCKS the session. What is left is a compositor with no lock surface,
    // a phone that reads black, and `{"locked":false,"lockRequested":true}`.
    // If instead the adoption does not match, the new manager's lock() is
    // refused (the outgoing lock is still the process-global holder) and
    // updateSurfaces(true) is called anyway — that is the FATAL this task was
    // raised for. Same root cause, two faces.
    //
    // With the request true at construction, realizeLockTarget takes the adopt
    // branch: surfaces are re-created against the SAME compositor lock,
    // manager->lock() declines harmlessly because we already hold it, and
    // updateSurfaces sees an active lock. No re-request, no denial, no
    // unlock_and_destroy. This is the task's "adopting beats re-requesting",
    // and it is the only branch that never opens the panel.
    PersistentProperties {
        id: lockContinuity
        reloadableId: "souveraineLockContinuity"

        property bool held: false

        onLoaded: {
            if (lockContinuity.held && !GlobalStates.screenLocked) {
                console.log("[lock] scene reload owed a lock — adopting, not re-requesting");
                GlobalStates.screenLocked = true;
            }
        }
    }

    WlSessionLock {
        id: lock
        // Explicit so the adoption survives being reparented out of a Scope.
        // Under a Scope children match by index and this is unused; anywhere
        // else an empty reloadableId means oldInstance is ALWAYS null, the
        // manager is never adopted, and the lock cannot be re-acquired for the
        // life of the process.
        reloadableId: "souveraineSessionLock"
        locked: GlobalStates.screenLocked
        surface: root.sessionLockSurface

        // `secure` is the compositor's acknowledgement that a real
        // session-lock surface is up — not our requested bool. Keep it
        // separate so an IPC caller cannot mistake a queued lock for a secure
        // surface when it is deciding whether personal content may be exposed.
        // It is also the true "lockscreen is ready" event: the one moment we
        // dismiss the boot bloom. The C splash released DRM master back at the
        // Act III→IV handoff (~6.8s); the quickshell BootBloom overlay has
        // covered all of Hyprland's startup since. Now our own lock surface is
        // mapped and secure, so fade the bloom out to reveal it. Firing on the
        // request edge (screenLocked) was too early — the lock surface wasn't
        // on screen yet. bootDismissed keeps it once-only so re-locks after
        // unlock never re-hide a bloom that's already gone.
        onSecureChanged: {
            GlobalStates.screenLockSecure = secure;
            console.log("[lock] session lock secure=" + secure);
            root.dismissBloomIfSecure();
        }

        // The dismissal above is an EDGE, and a scene reload is exactly the
        // case where the edge is in the past. `GlobalStates.bootBloomActive`
        // defaults to true on every scene construction and `bootDismissed`
        // resets with it, but a reload during an already-secure lock never
        // moves `secure` — so nothing ever cleared the bloom and the phone sat
        // under a full-screen white overlay until the shell was restarted.
        // Observed 2026-07-29: a reload at 10:44:10 with no secure transition
        // after it, and a solid white `grim` capture.
        //
        // Same shape as the locked_ack edge that never re-fired after a
        // sessiond restart, and as the ChargeRate stale-scene reload. Check the
        // LEVEL at construction as well as the edge.
        Component.onCompleted: root.dismissBloomIfSecure()

        // The compositor can end our lock without us asking: ext-session-lock
        // `finished` (denied because another client held it) makes quickshell
        // drop `locked` to false C++-side. Our request bool never hears about
        // it, so it lingers true — which lies to every gate reading it
        // (redaction, capability tiers, session state IPC) and blocks
        // re-locking, because the `locked:` binding only re-fires on a
        // false->true edge of screenLocked. Resync on that path. A normal
        // unlock clears screenLocked *before* the binding drops `locked`,
        // so this guard stays quiet there.
        onLockStateChanged: {
            if (!lock.locked && GlobalStates.screenLocked) {
                console.log("[lock] compositor ended our session lock while still requested — resyncing");
                GlobalStates.screenLocked = false;
            }
        }
    }

    // `secure` is an EDGE, and after a reload that adopts the lock that edge
    // is in the past: the compositor acked before this tree existed, so
    // onSecureChanged never fires and GlobalStates.screenLockSecure would sit
    // false on a session that is genuinely secure. Everything gating on it —
    // redaction, capability tiers, and the locked_ack this handoff owes
    // sessiond — would then be wrong in the dangerous direction.
    //
    // Publish the LEVEL once the reload has actually run. Component.onCompleted
    // is too early: Reloadable defers onReload to after component completion,
    // so `lock.secure` is still false there. Setting screenLockSecure is enough
    // to send the ack — SessiondBridge is already listening for it.
    Connections {
        target: Quickshell
        function onReloadCompleted() {
            GlobalStates.screenLockSecure = lock.secure;
            root.dismissBloomIfSecure();
        }
    }

    // Idempotent, and deliberately NOT a timeout. A bloom that outlives its
    // reason is a bug to locate, not something to paper over with a timer — if
    // this is still up while the lock is secure, the caller is missing and the
    // fix belongs where the call is missing.
    function dismissBloomIfSecure() {
        if (!lock.secure || root.bootDismissed)
            return;
        root.bootDismissed = true;
        GlobalStates.bootBloomActive = false;
    }

    function lock() {
        if (Config.options.lock.useHyprlock) {
            Quickshell.execDetached(["bash", "-c", "pidof hyprlock || hyprlock"]);
            return;
        }
        GlobalStates.screenLocked = true;
    }

    IpcHandler {
        target: "lock"

        function activate(): void {
            root.lock();
        }
        function focus(): void {
            lockContext.shouldReFocus();
        }
    }

    GlobalShortcut {
        name: "lock"
        description: "Locks the screen"

        onPressed: {
            root.lock()
        }
    }

    GlobalShortcut {
        name: "lockFocus"
        description: "Re-focuses the lock screen. This is because Hyprland after waking up for whatever reason"
            + "decides to keyboard-unfocus the lock screen"

        onPressed: {
            lockContext.shouldReFocus();
        }
    }

    property bool initDone: false
    function initIfReady() {
        if (!Config.ready || !Persistent.ready || root.initDone) return;
        root.initDone = true;
        // Register with sessiond (and start the heartbeat) before deciding
        // the startup lock state. If sessiond holds the session lock, it
        // releases on our shell_ready and we MUST lock immediately — the
        // compositor is holding an abandoned lock for us to inherit
        // (misc:allow_session_lock_restore). No sessiond = cb(false) and
        // the legacy launchOnStartup rules decide alone.
        SessiondBridge.shellReady(function(mustLock) {
            if (mustLock
                || (Config.options.lock.launchOnStartup
                    && (Persistent.isNewHyprlandInstance || Persistent.states.lock.locked))) {
                root.lock();
            } else {
                KeyringStorage.fetchKeyringData();
            }
        });
    }
    Connections {
        target: Config
        function onReadyChanged() {
            root.initIfReady();
        }
    }
    Connections {
        target: Persistent
        function onReadyChanged() {
            root.initIfReady();
        }
    }
}
