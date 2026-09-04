// Pixel3Arch addition: stop the Waydroid session. Phone-only — waydroid is
// installed on the phone and not on the laptop, so this lives in the overlay.
// Deploy to modules/common/models/quickToggles/.
import QtQuick
import Quickshell
import Quickshell.Io
import qs.services
import qs.modules.common

/**
 * Terminate the Android container's session.
 *
 * `waydroidTerminate` sat in the phone's config with no component behind it in
 * any tree or commit — a configured verb that did nothing, silently.
 */
QuickToggleModel {
    id: root

    /// True while a stop is in flight, so a double tap cannot race itself.
    property bool busy: false

    name: Translation.tr("Stop Android")
    hasStatusText: false
    toggled: false
    icon: "android"

    // Deliberate, and always pressable. There is deliberately NO status poll:
    // `waydroid status` talks to the container and can block for seconds, and
    // a button that greys itself out on a stale read is a verb the owner
    // cannot reach. Press it; it reports what actually happened.
    mainAction: () => {
        if (root.busy)
            return;
        root.busy = true;
        stopProc.running = true;
    }

    Process {
        id: stopProc
        command: ["bash", "-c",
            "waydroid session stop >/dev/null 2>&1; "
            + "waydroid status 2>/dev/null | grep -q 'Session:.*RUNNING' && echo running || echo stopped"]
        stdout: StdioCollector {
            id: stopResult
            onStreamFinished: {
                root.busy = false;
                const stillRunning = stopResult.text.trim() === "running";
                Quickshell.execDetached(["notify-send", Translation.tr("Android"),
                    stillRunning
                        ? Translation.tr("Session did not stop.")
                        : Translation.tr("Session stopped."),
                    "-a", "Shell"]);
            }
        }
        onExited: root.busy = false
    }

    tooltipText: Translation.tr("Stop the Waydroid session")
}
