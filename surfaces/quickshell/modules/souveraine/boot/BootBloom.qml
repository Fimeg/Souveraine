// Souveraine boot bloom overlay — the quickshell half of the boot animation.
//
// The C splash (souveraine-splash) owns DRM master at boot and plays Act I-III
// (cursor blink/slide, types the two lines) on bare GPU, then holds. This
// overlay is Hyprland's first surface: a fullscreen layershell Overlay that
// resumes the animation (Act IV+V — Souvie + bloom flower) seamlessly, over the
// live compositor, covering ALL of Hyprland's own boot render. It:
//
//   1. maps as early as possible (top-level in ShellRoot, NOT behind the
//      Config.ready LazyLoader) so nothing of Hyprland's default output shows;
//   2. on its first painted frame, pokes `splash-signal` — telling the C splash
//      "I'm up, you may release DRM master now." The splash fades and exits;
//      Hyprland (already waiting) grabs the CRTC; this overlay is already
//      painting black+bloom over the swap, so the handoff is invisible;
//   3. drives u_time from the boot monotonic clock so the bloom phase continues
//      exactly where the splash left off (no visible seam / restart);
//   4. is dismissed by LockScreen.onSecureChanged (fade out) once the real lock
//      surface is up.
//
// Clock continuity: the C shader's t is CLOCK_MONOTONIC seconds since its own
// start (≈ boot). We read /proc/uptime once at map time to recover that same
// clock, so u_time here matches the splash's u_time. FLOWER_START (8.0s) is
// absolute in that clock; the bloom begins on its own regardless of when this
// overlay happens to map.
pragma ComponentBehavior: Bound
import qs
import qs.services
import qs.modules.common
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

Scope {
    id: root

    // Held true from boot until the lockscreen is secure. LockScreen flips it
    // off (onSecureChanged) to fade the overlay out and reveal the lock.
    property bool active: GlobalStates.bootBloomActive
    // Monotonic seconds at the moment we captured the boot clock. u_time =
    // bootBase + (now - captureWall). Filled from /proc/uptime once.
    property real bootBase: 0
    property bool clockReady: false
    property bool splashPoked: false

    // Read the boot monotonic clock once, as early as possible.
    Process {
        id: uptimeProc
        command: ["cat", "/proc/uptime"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                // /proc/uptime: "<seconds-since-boot> <idle>"
                const secs = parseFloat(text.trim().split(/\s+/)[0]);
                if (!isNaN(secs)) {
                    root.bootBase = secs;
                    root.clockReady = true;
                }
            }
        }
    }

    // Pokes the C splash to release DRM master. Fires once, on first frame.
    Process { id: splashSignalProc }

    // Watchdog: the overlay is normally dismissed by LockScreen.onSecureChanged.
    // That event only ever arrives where something locks the session at boot —
    // on SouveraineOS, sessiond. Running this shell on a plain distro (the
    // laptop still boots EndeavourOS), nothing locks, secure never fires, and
    // the overlay covers the whole screen forever with no input path out. Boot
    // is over long before this fires; if the lock hasn't taken by now it isn't
    // coming, so reveal the desktop rather than trapping the session.
    Timer {
        interval: 15000
        repeat: false
        running: root.active
        onTriggered: {
            if (!GlobalStates.screenLockSecure) {
                console.log("[bootbloom] no lock surface after 15s — dismissing overlay");
                GlobalStates.bootBloomActive = false;
            }
        }
    }

    // Wall-clock reference captured when the /proc/uptime read completed, so
    // elapsed = frameClock.currentTime-based delta. We drive u_time off an
    // Animator-free FrameAnimation for a monotonic per-frame tick.
    FrameAnimation {
        id: frameClock
        running: root.active
        // elapsedTime is seconds since this animation started running.
    }

    Variants {
        model: root.active ? Quickshell.screens : []

        PanelWindow {
            id: win
            required property var modelData
            screen: modelData

            WlrLayershell.namespace: "quickshell:bootbloom"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.exclusionMode: ExclusionMode.Ignore
            // No keyboard grab — we're purely visual; the lock takes input.
            color: "black"

            anchors { top: true; left: true; right: true; bottom: true }

            // u_time in the splash's boot clock: base + seconds elapsed since
            // we captured the base. Continuous with the C shader.
            property real bootTime: root.bootBase + frameClock.elapsedTime

            ShaderEffect {
                id: bloom
                anchors.fill: parent
                // qsb-compiled at deploy time from BootBloom.frag
                fragmentShader: Qt.resolvedUrl("BootBloom.frag.qsb")

                property real u_time: win.bootTime
                property vector2d u_resolution: Qt.vector2d(width, height)
                property real u_fade: 1.0

                // Fade out on dismiss.
                Behavior on u_fade {
                    NumberAnimation { duration: 450; easing.type: Easing.InOutQuad }
                }
            }

            // ShaderEffect has no per-frame signal; poke the splash once the
            // shader is compiled AND the boot clock is read, one tick later, so
            // the C splash only releases DRM master after we're actually able to
            // paint. window.frameSwapped would be ideal but doesn't surface
            // through PanelWindow — a single-shot armed Timer is the reliable
            // "we're up" edge (a frame or two of overlap with the splash is
            // harmless; both draw the same continuous bloom).
            Timer {
                interval: 32   // ~2 frames at 60Hz
                repeat: false
                running: bloom.status === ShaderEffect.Compiled
                         && root.clockReady && !root.splashPoked
                onTriggered: {
                    root.splashPoked = true;
                    splashSignalProc.exec({ command: ["splash-signal"] });
                }
            }

            // Souvie — real PNG logo, drawn above the bloom, fading in on the
            // same SOUVIE_FADE window (7.4-8.6s) the shader used for the atlas
            // text. Falls back to invisible if the asset is missing.
            Image {
                id: souvie
                anchors.horizontalCenter: parent.horizontalCenter
                y: parent.height * 0.14
                width: Math.min(parent.width * 0.42, sourceSize.width)
                fillMode: Image.PreserveAspectFit
                source: Qt.resolvedUrl("souvie.png")
                smooth: true
                // fade in over 7.4-8.6s in boot clock
                opacity: bloom.u_fade * Math.max(0, Math.min(1,
                    (win.bootTime - 7.4) / 1.2))
                visible: status === Image.Ready
            }
        }
    }
}
