// Souveraine patch to ii's stock Lock.qml.
//
// Selects the lock surface by config: lock.touchKeypad picks the
// TouchLockSurface (PIN pad for the phone's touch panel) instead of the
// stock keyboard-driven LockSurface. Everything else is unchanged stock ii.
pragma ComponentBehavior: Bound
import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions
import qs.modules.common.panels.lock
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Hyprland

LockScreen {
    id: root

    // Monitor name -> workspace id to restore on unlock (set when locking)
    property var savedWorkspaces: ({})

    // Session arbiter surface. Lives here beside the lock because the lock IS
    // the session gate on this device — LockScreen already owns WlSessionLock,
    // and the session verbs (suspend/poweroff/inhibit) are the other half of
    // the same lifecycle. The logic lives in the Session singleton
    // (modules/common/functions/Session.qml); this is only the IPC face.
    //
    // Every mutating method returns {ok, reason?} rather than throwing, so a
    // refusal is information the agent learns from — same contract as dock.*,
    // shell.* and apps.*. `lock` (target: "lock") stays as it was: it is the
    // surface's own activate/focus pair, not the session lifecycle.
    //
    // `session` is the one lifecycle authority across phone, laptop, and
    // desktop. The visual power-menu toggles live at `sessionMenu`; a menu is
    // a presentation concern, whereas this target is the system contract.
    // The ii screen is vendored with that one target rename so Quickshell
    // never has to silently choose between duplicate `session` handlers.
    // Every method returns `string`, not `var`, and the payload is JSON.
    // This is not a style choice — quickshell marshals exactly five types over
    // IPC (string, int, bool, double, color; see src/io/ipc.cpp ipcType()) and
    // maps a `var` return to VOID, discarding the value with no error. A
    // method declared `: var` therefore looks correct in QML, registers as
    // `(): void`, and silently returns nothing to the caller. JSON-over-string
    // is the only way a structured {ok, reason} result actually crosses.
    IpcHandler {
        target: "session"

        // Read-only projection: lock state (read through from the compositor,
        // never a cached bool), idle inhibitors with their reasons, and what
        // this machine can actually do.
        function state(): string {
            return JSON.stringify(Session.state());
        }

        function capabilities(): string {
            return JSON.stringify(Session.caps());
        }

        function lock(): string {
            return JSON.stringify(Session.lock());
        }

        // Refuses by design — unlocking is the credential gate.
        function unlock(): string {
            return JSON.stringify(Session.unlock());
        }

        function suspend(): string {
            return JSON.stringify(Session.suspend());
        }

        function hibernate(): string {
            return JSON.stringify(Session.hibernate());
        }

        function poweroff(): string {
            return JSON.stringify(Session.poweroff());
        }

        function reboot(): string {
            return JSON.stringify(Session.reboot());
        }

        function logout(): string {
            return JSON.stringify(Session.logout());
        }

        // Reason is mandatory: an inhibitor nobody can explain is exactly the
        // thing that leaves the phone awake in a pocket at 3am.
        function inhibit(what: string, reason: string): string {
            return JSON.stringify(Session.inhibit(what, reason));
        }

        function uninhibit(cookie: string): string {
            return JSON.stringify(Session.uninhibit(cookie));
        }
    }

    // Preview-only diagnostic ingress. The physical FPC1020 producer publishes
    // its root-owned pulse record for LockContext to watch; it cannot use
    // generic XF86WakeUp here because the touch controller emits that key too.
    // This target lets us exercise the same visual-only path manually. It
    // never unlocks, reveals Personal content, or mints step-up. Task 41 owns
    // replacing this with an attested, source-specific producer.
    IpcHandler {
        target: "fingerprint"

        function signal(): string {
            return JSON.stringify(root.context.noteProvisionalFingerprintPulse());
        }
    }

    Timer {
        id: restoreTimer
        interval: 150
        repeat: false
        onTriggered: {
            var batch = ""
            for (var j = 0; j < Quickshell.screens.length; ++j) {
                var monName = Quickshell.screens[j].name
                var wsId = root.savedWorkspaces[monName]
                if (wsId !== undefined) {
                    batch += `hyprctl dispatch 'hl.dsp.focus({monitor="${monName}"})'; hyprctl dispatch 'hl.dsp.focus({workspace=${wsId}})';`
                }
            }
            if (batch.length > 0) {
                Quickshell.execDetached(["bash", "-c", batch])
            }
        }
    }

    // Which surface, decided WITHOUT waiting for the config file.
    //
    // `Config.options` is a JsonAdapter, so it answers with its QML defaults
    // from the instant it exists — `touchKeypad` reads `false` until `onLoaded`
    // flips `ready`. That is survivable at boot, where `initIfReady()` waits for
    // `Config.ready` before requesting a lock, and fatal on a reload, where the
    // lock is adopted at construction: the surface would bind to the DESKTOP
    // keypad on the phone, and `WlSessionLock.surfaceComponent` cannot be
    // changed while the lock is active — quickshell qCritical's and keeps the
    // old one. The result is a lock screen that will not take your PIN, which
    // is the worst of the three failures on this path because you cannot get
    // back in.
    //
    // So the last known-good answer rides through the reload in-process, and
    // Config takes over the moment it is genuinely loaded.
    //
    // `PersistentProperties` is in-process, so it covers a reload and not a
    // cold start. sessiond takes the lock before the shell exists, so a shell
    // started by greetd adopts a live lock at construction with `ready` still
    // false — the fatal path above, reached without any reload. `SOUVERAINE_
    // TOUCH_KEYPAD` is read from the environment, which is answerable at that
    // instant; unset it reads exactly as before.
    PersistentProperties {
        id: lockPrefs
        reloadableId: "souveraineLockPrefs"
        property bool touchKeypad: Quickshell.env("SOUVERAINE_TOUCH_KEYPAD") === "1"
    }

    Connections {
        target: Config
        function onReadyChanged() {
            if (Config.ready)
                lockPrefs.touchKeypad = Config.options.lock.touchKeypad;
        }
    }

    lockSurface: (Config.ready ? Config.options.lock.touchKeypad : lockPrefs.touchKeypad)
        ? touchSurfaceComponent
        : desktopSurfaceComponent

    property Component desktopSurfaceComponent: LockSurface {
        context: root.context
    }
    property Component touchSurfaceComponent: TouchLockSurface {
        context: root.context
    }

    // Single batch for lock and unlock so we don't race multiple hyprctl calls
    Connections {
        target: GlobalStates
        function onScreenLockedChanged() {
            if (GlobalStates.screenLocked) {
                // Lock: save workspace per monitor and move all to temp workspace in one batch
                var next = {}
                var batch = "keyword animation workspaces,1,7,menu_decel,slidevert; "
                for (var i = 0; i < Quickshell.screens.length; ++i) {
                    var mon = Quickshell.screens[i].name
                    var mData = HyprlandData.monitors.find(m => m.name === mon)
                    if (mData?.activeWorkspace == undefined) {
                        return;
                    }
                    var ws = (mData?.activeWorkspace?.id ?? 1)
                    next[mon] = ws
                    batch += `hyprctl dispatch 'hl.dsp.focus({monitor="${mon}"})'; hyprctl dispatch 'hl.dsp.focus({workspace=${2147483647 - ws}})';`
                }
                root.savedWorkspaces = next
                Quickshell.execDetached(["bash", "-c", batch])
            } else {
                restoreTimer.start()
            }
        }
    }

    // Push everything down (visual only; workspace switch is in Connections above)
    Variants {
        model: Quickshell.screens
        delegate: Scope {
            required property ShellScreen modelData
            property bool shouldPush: GlobalStates.screenLocked
            property string targetMonitorName: modelData.name
            property int verticalMovementDistance: modelData.height
            property int horizontalSqueeze: modelData.width * 0.2
        }
    }
}
