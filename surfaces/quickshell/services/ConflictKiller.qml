// Souveraine patch to ii's stock ConflictKiller.qml.
//
// Stock ii flags kded6 as a "conflicting tray". On this phone kded6 is not
// a competing Plasma tray — it is the StatusNotifierWatcher that ii's OWN
// tray (Quickshell.Services.SystemTray) registers as a host with. Killing
// it just makes D-Bus re-activate it on ii's next tray call, and leaving
// autoKillTrays off pops the kill dialog on every shell start. So: drop
// the kded6 check entirely; keep the notification-daemon check unchanged.
pragma Singleton

import qs.modules.common
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    property string killDialogQmlPath: FileUtils.trimFileProtocol(Quickshell.shellPath("killDialog.qml"))

    function load() {
        // dummy to force init
    }

    Connections {
        target: Config
        function onReadyChanged() {
            if (Config.ready) checkConflictsProc.running = true
        }
    }

    Process {
        id: checkConflictsProc
        command: ["bash", "-c", `pidof mako dunst`]
        stdout: StdioCollector {
            onStreamFinished: {
                const conflictingNotifications = this.text.trim().length > 0;
                if (!conflictingNotifications) return;
                if (Config.options.conflictKiller.autoKillNotificationDaemons)
                    Quickshell.execDetached(["killall", "mako", "dunst"])
                else
                    Quickshell.execDetached(["qs", "-p", root.killDialogQmlPath])
            }
        }
    }
}
