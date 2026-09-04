// The dial's surface and its entries.
//
// RadialDial.qml is the component; this is the panel it lives on and the list
// it renders. Split so the entries can move to the verb tables (TASK-30)
// without touching the ring, and so the ring can be reused by anything else
// that wants a thumb-picker.
//
// Entries are the ones Casey named: kill the current window, screenshot,
// system health, the agent — plus lock, which is the one every phone needs
// within a thumb's reach.
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland
import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions
import "." as DialLocal

Scope {
    id: scope

    property bool dialOpen: false

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: win
            required property var modelData
            screen: win.modelData

            anchors { top: true; left: true; right: true; bottom: true }
            color: "transparent"
            visible: scope.dialOpen
            WlrLayershell.namespace: "souveraine:dial"
            WlrLayershell.layer: WlrLayer.Overlay
            // OnDemand, not Exclusive: an exclusive grab was tried on the
            // polkit surface 2026-07-26 and stopped touch reaching the layer
            // below it entirely.
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand
            exclusionMode: ExclusionMode.Ignore

            // `open` is assigned, never bound: openAt() writes it, and a
            // binding here would be broken by that write on the first call.
            // The host drives it in one direction and listens for the close.
            DialLocal.RadialDial {
                id: dial
                entries: scope.entries
                onOpenChanged: {
                    if (!dial.open)
                        scope.dialOpen = false;
                }
            }

            Connections {
                target: scope
                function onDialOpenChanged() {
                    // No touch point to hand over — and none can be computed
                    // here, because this fires on the same signal that maps
                    // the surface, when win.width/height are still 0.
                    if (scope.dialOpen)
                        dial.openAt();
                    else
                        dial.close();
                }
            }
        }
    }

    // ## Screen recording
    //
    // Hold the camera to start, tap or hold it again to stop. There is no
    // wf-recorder or wl-screenrec on this device, so the capture is `grim` in
    // a loop piped into ffmpeg — measured on blueline 2026-08-06 at 1080x2160,
    // 41 frames in 3.4s, about 12 fps. Coarse, and honest about it: the right
    // tool is a compositor-side encoder fed by the capture protocol viewtop
    // already serves, and this is what ships today with what is installed.
    //
    // **Its own process group, and SIGINT to the group.** The pipeline is two
    // processes and only ffmpeg can finish the file — it writes the mp4 index
    // on the way out, so a killed ffmpeg leaves an unplayable recording.
    // Interrupting the group stops the grim loop, ffmpeg sees its input end,
    // and it closes the file properly. Killing the shell alone would orphan
    // both halves and leave the phone quietly capturing the screen forever,
    // which is the failure worth engineering against here.
    property bool recording: false
    readonly property string recPid: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine-screenrec.pid"

    function startRecording() {
        if (scope.recording)
            return;
        Haptics.confirm();
        scope.recording = true;
        // `setsid` run directly as argv, not through an outer `sh -c`. Wrapped,
        // the inner script needs a second level of shell quoting and the whole
        // pipeline silently produced no file while every state flag said it was
        // recording — the failure looked like a bug in the verb rather than in
        // the quoting. One shell, one level of quotes.
        Quickshell.execDetached(["setsid", "sh", "-c",
            'mkdir -p "$HOME/Videos"; echo $$ > "$1"; '
            // Half scale: `grim` at the full 1080x2160 managed only ~3.5 fps
            // on this device (measured 2026-08-06), and a recording that
            // captures a third of the frames it claims plays back at triple
            // speed. Halving the pixels roughly doubles the rate and the
            // framerate below is set to match what it actually achieves.
            + 'while :; do grim -s 0.5 -t ppm - || break; done '
            + '| ffmpeg -y -loglevel error -f image2pipe -framerate 8 -i - '
            + '-c:v libx264 -preset ultrafast -pix_fmt yuv420p '
            + '"$HOME/Videos/screenrec-$(date +%Y%m%d-%H%M%S).mp4"',
            "_", scope.recPid]);
    }

    function stopRecording() {
        if (!scope.recording)
            return;
        Haptics.confirm();
        scope.recording = false;
        Quickshell.execDetached(["sh", "-c",
            'G=$(cat "$1" 2>/dev/null); [ -n "$G" ] && kill -INT -- -"$G" 2>/dev/null; rm -f "$1"',
            "_", scope.recPid]);
    }

    // The list. `enabled: false` carries a `reason` so a refusal explains
    // itself in the middle of the ring rather than greying out silently.
    readonly property var entries: [
        {
            icon: "lock",
            label: "Lock",
            verb: () => { Haptics.confirm(); Session.lock(); }
        },
        {
            icon: "photo_camera",
            label: scope.recording ? "Stop recording" : "Screenshot",
            verb: () => {
                // While recording, the same seat is the way out. A separate
                // stop entry would mean the ring changes length mid-session,
                // and the dial is muscle memory — the thumb goes where the
                // camera was, so that is where stopping lives.
                if (scope.recording) {
                    scope.stopRecording();
                    return;
                }
                Haptics.confirm();
                Quickshell.execDetached(["sh", "-c",
                    "grim ~/Pictures/screenshot-$(date +%Y%m%d-%H%M%S).png"]);
            },
            hold: () => {
                if (scope.recording) {
                    scope.stopRecording();
                    return;
                }
                scope.startRecording();
            }
        },
        {
            icon: "cancel",
            label: "Kill window",
            enabled: !!Hyprland.activeToplevel,
            reason: "no focused window to close",
            verb: () => {
                const addr = Hyprland.activeToplevel?.address;
                if (!addr) { Haptics.refuse(); return; }
                Haptics.confirm();
                const a = addr.startsWith("0x") ? addr : "0x" + addr;
                Quickshell.execDetached(["hyprctl", "dispatch",
                    `hl.dsp.window.close({ window = "address:${a}" })`]);
            }
        },
        {
            icon: "monitor_heart",
            label: "Health",
            verb: () => { Haptics.confirm(); GlobalStates.sidebarRightOpen = true; }
        },
        {
            icon: "smart_toy",
            label: "Agent",
            verb: () => { Haptics.confirm(); GlobalStates.sidebarLeftOpen = true; }
        },
        {
            icon: "keyboard",
            label: "Keyboard",
            verb: () => { Haptics.confirm(); GlobalStates.oskOpen = !GlobalStates.oskOpen; }
        }
    ]

    // Anything that can reach this can raise the dial: the pill's long-hold,
    // an edge gesture, a squeeze once grip produces events, or an agent.
    IpcHandler {
        target: "dial"

        function open(): void {
            scope.dialOpen = true;
            console.log("[dial] open -> dialOpen=" + scope.dialOpen
                + " screens=" + Quickshell.screens.length);
        }

        function close(): void {
            scope.dialOpen = false;
        }

        function toggle(): void {
            scope.dialOpen = !scope.dialOpen;
        }

        // Recording, reachable without the ring. The hold gesture is the
        // hand's way in; this is hers, and doctrine §13 is explicit that a
        // verb only the thumb can reach is a defect. It is also the only way
        // to test the pipeline without a finger on the glass.
        function record(): void {
            scope.startRecording();
        }

        function stopRecord(): void {
            scope.stopRecording();
        }

        function recording(): string {
            return scope.recording ? "recording" : "idle";
        }

        // What the ring currently offers, so a caller can see the entries
        // without opening it. JSON-over-string: quickshell marshals a `var`
        // return as VOID.
        function entries(): string {
            return JSON.stringify(scope.entries.map(e => ({
                label: e.label,
                icon: e.icon,
                enabled: e.enabled !== false,
                reason: e.reason ?? ""
            })));
        }
    }

    GlobalShortcut {
        name: "dialToggle"
        description: "Toggle the radial dial"
        onPressed: scope.dialOpen = !scope.dialOpen
    }
}
