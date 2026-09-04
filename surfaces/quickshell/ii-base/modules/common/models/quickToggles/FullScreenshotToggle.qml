import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.services
import qs.modules.common

/**
 * Whole-screen capture, as opposed to ScreenSnipToggle's region select.
 *
 * `fullScreenshot` had been listed in the phone's config for long enough that
 * everyone assumed a component existed; none ever did, in any tree or any
 * commit. It rendered nothing because there was nothing to render.
 */
QuickToggleModel {
    id: root
    name: Translation.tr("Screenshot")
    hasStatusText: false
    toggled: false
    icon: "screenshot_monitor"

    // Close the panel first, or the screenshot is a picture of the panel.
    // Same 300 ms the region snip uses — the close animation has to finish.
    mainAction: () => {
        GlobalStates.sidebarRightOpen = false;
        captureDelay.start();
    }

    Timer {
        id: captureDelay
        interval: 300
        repeat: false
        onTriggered: captureProc.running = true
    }

    // To a file AND the clipboard: on the phone there is rarely anywhere to
    // paste it right away, and on the laptop the file is the annoying half.
    Process {
        id: captureProc
        command: ["bash", "-c",
            "mkdir -p \"$HOME/Pictures\"; "
            + "f=\"$HOME/Pictures/screenshot-$(date +%Y%m%d-%H%M%S).png\"; "
            + "grim \"$f\" || { notify-send Screenshot 'Capture failed.' -a Shell; exit 1; }; "
            + "wl-copy -t image/png < \"$f\"; "
            + "notify-send Screenshot \"${f##*/} — saved and copied\" -a Shell"]
    }

    tooltipText: Translation.tr("Whole screen to ~/Pictures and the clipboard")
}
