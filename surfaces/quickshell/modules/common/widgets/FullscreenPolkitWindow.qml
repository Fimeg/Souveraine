// Souveraine override of ii's FullscreenPolkitWindow.
//
// One change: on a phone the prompt takes keyboard focus exclusively.
//
// Upstream uses WlrKeyboardFocus.OnDemand, which is right for a desktop —
// the prompt is one surface among many and focus follows the pointer. On the
// phone it made the password field untypable in a way that looked like a
// keyboard bug and wasn't. Verified live 2026-07-26: the on-screen keyboard
// rose with the prompt and its keys went nowhere.
//
// The chain: squeekboard delivers keystrokes through the input-method
// protocol to whatever surface holds keyboard focus, and Qt only activates a
// text-input context on a surface that HAS that focus. With OnDemand a
// layer-shell surface is granted focus by a click — but the field the user
// needs to click is inside the very surface that has no context yet, and
// there is no pointer on a phone to hover one into existence. Nothing ever
// activated, so the keys had no destination.
//
// Exclusive is also simply what this surface is: a modal authorization
// prompt on the Overlay layer, the same posture the lock surface takes. Esc
// and the cancel button both still dismiss it (see PolkitContent), so the
// grab is not a trap.
//
// Desktop keeps OnDemand — there is a hardware keyboard there and no reason
// to change a working path.
pragma ComponentBehavior: Bound
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Wayland

Scope {
    id: root
    required property Component contentComponent

    readonly property bool phone: Config.options?.souveraine?.phone ?? false

    Loader {
        active: PolkitService.active
        sourceComponent: Variants {
            model: Quickshell.screens
            delegate: PanelWindow {
                id: panelWindow
                required property var modelData
                screen: modelData

                // Do not cover the on-screen keyboard. squeekboard is a
                // layer-shell surface on `top`; this prompt is on `overlay`,
                // so a full-screen prompt sits above it. The dialog is
                // transparent, which made that look survivable — the keyboard
                // was plainly visible through it — but this surface owned the
                // whole screen's input region, so every tap in the bottom
                // third landed on the dialog and the keys never saw a touch.
                // Visible and inert, exactly as reported.
                //
                // Measured on device: osk is 0 732 540 348 on a 540x1080
                // panel, so the keyboard is the bottom ~32%. While it is up,
                // stop anchoring the bottom edge and end the surface above it.
                readonly property bool yieldToOsk: root.phone && GlobalStates.oskOpen

                anchors {
                    top: true
                    left: true
                    right: true
                    bottom: !panelWindow.yieldToOsk
                }
                // Only consulted when the bottom anchor is released.
                implicitHeight: panelWindow.yieldToOsk
                    ? Math.round((panelWindow.screen?.height ?? 1080) * 0.65)
                    : 0

                color: "transparent"
                WlrLayershell.namespace: "quickshell:polkit"
                // Exclusive was tried on 2026-07-26 and made it worse: with an
                // exclusive layer grab the on-screen keyboard stopped taking
                // touch at all — keys did not even highlight. OnDemand is what
                // the working text fields elsewhere in this shell use, and
                // squeekboard follows them fine.
                WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand
                WlrLayershell.layer: WlrLayer.Overlay
                exclusionMode: ExclusionMode.Ignore

                Loader {
                    anchors.fill: parent
                    sourceComponent: root.contentComponent
                }
            }
        }
    }
}
