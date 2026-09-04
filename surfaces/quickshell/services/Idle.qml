pragma Singleton
import qs.modules.common
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

/**
 * A nice wrapper for date and time strings.
 */
Singleton {
    id: root

    property alias inhibit: idleInhibitor.enabled
    inhibit: false

    // ii-stock consumers (IdleInhibitorToggle.qml) read Idle.autoIdleInhibit to
    // label the "Keep awake" toggle. Souveraine's idle path routes through
    // hypridle, not a Wayland idle-inhibitor, so there's no separate "auto"
    // mode — but the stock toggle expects the property to exist. Expose it as
    // a plain bool so the toggle binds cleanly instead of throwing
    // [undefined] -> bool at startup.
    property bool autoIdleInhibit: false

    // Pixel 3: the Wayland idle-inhibitor on an invisible 0x0 surface is
    // not honored by our compositor build, and screen-off is driven by
    // hypridle scripts anyway — so Keep System Awake stops/starts hypridle.
    onInhibitChanged: {
        // Keep System Awake toggles the systemd-managed hypridle unit.
        // (Was pkill/setsid, which orphaned hypridle across Hyprland restarts
        // and wedged wake — see hyprland.lua hyprland.start.)
        //
        // Runs through a Process, not execDetached, because this is the
        // mechanism that decides whether the phone sleeps: if the unit fails
        // to come back we need to know, not discover it via a flat battery.
        // reset-failed is expected to be a no-op when the unit is healthy, so
        // its failure is not interesting — but the restart's is, so the two
        // are separated rather than hidden behind one silencing redirect.
        hypridleProc.action = root.inhibit ? "stop" : "restart";
        hypridleProc.command = ["sh", "-c", root.inhibit
            ? "systemctl --user stop hypridle.service"
            : "systemctl --user reset-failed hypridle.service || true; systemctl --user restart hypridle.service"];
        hypridleProc.running = true;
    }

    Process {
        id: hypridleProc
        property string action: ""
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0)
                console.log(`[idle] hypridle ${hypridleProc.action} failed (exit ${exitCode}); `
                    + `idle handling may be wedged — inhibit=${root.inhibit}`);
        }
    }

    Connections {
        target: Persistent
        function onReadyChanged() {
            if (!Persistent.isNewHyprlandInstance) {
                root.inhibit = Persistent.states.idle.inhibit;
            } else {
                Persistent.states.idle.inhibit = root.inhibit;
            }
        }
    }

    function toggleInhibit(active = null) {
        if (active !== null) {
            root.inhibit = active;
        } else {
            root.inhibit = !root.inhibit;
        }
        Persistent.states.idle.inhibit = root.inhibit;
    }

    IdleInhibitor {
        id: idleInhibitor
        window: PanelWindow {
            // Inhibitor requires a "visible" surface
            // Actually not lol
            implicitWidth: 0
            implicitHeight: 0
            color: "transparent"
            // Just in case...
            anchors {
                right: true
                bottom: true
            }
            // Make it not interactable
            mask: Region {
                item: null
            }
        }
    }
}
