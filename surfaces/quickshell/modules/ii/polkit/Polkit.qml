import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Wayland

// Souveraine: on a phone deploy, surface the on-screen keyboard when a polkit
// prompt appears. The polkit window is a wlr-layer-shell Overlay and the OSK
// is a separate layer-shell surface (squeekboard), so the OSK won't auto-rise
// to it. Since both run under this shell, we drive the OSK explicitly.
// Laptop deploys (souveraine.phone == false) are unchanged — hardware
// keyboard handles it.
//
// This was a poke — `oskOpen = PolkitService.active` on the transition — and
// a poke is not enough for two reasons, both of which left a password field
// on screen with no way to type into it:
//
//   1. squeekboard hides itself when input-method focus drops, which this
//      layer-shell surface does not reliably hold. The keyboard appeared and
//      then left while the prompt was still waiting.
//   2. The transition is all a poke sees. pkexec run before Config.ready —
//      an update on login, a boot-time authorization — has its prompt up
//      before this file is even loaded, so no transition ever arrives.
//
// A hold fixes both: the keyboard is re-asserted for as long as the prompt
// is up (GlobalStates.oskHold), and onCompleted takes the hold for a prompt
// that was already waiting.
FullscreenPolkitWindow {
    id: root
    contentComponent: Component {
        PolkitContent {}
    }

    readonly property bool phone: Config.options?.souveraine?.phone ?? false

    function syncKeyboard() {
        if (!root.phone) return;
        if (PolkitService.active) GlobalStates.oskHold("polkit");
        else GlobalStates.oskRelease("polkit");
    }

    Component.onCompleted: root.syncKeyboard()

    Connections {
        target: PolkitService
        function onActiveChanged() {
            root.syncKeyboard();
        }
    }
}
