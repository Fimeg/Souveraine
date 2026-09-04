// Pixel3Arch patch to ii's stock Dock.qml (2026-07-07).
//
// Problem: the dock (pinned, exclusiveZone claiming real screen space) and
// wvkbd (the on-screen keyboard, see OnScreenKeyboard.qml) both anchor to
// the bottom edge and both claim exclusive zones — their reservations
// stack instead of one giving way, so the keyboard/its focus-grab shield
// drift out of alignment with each other depending on what else is
// reserving space. Simplest real fix: the dock gets out of the way
// entirely while the keyboard is open, instead of trying to make the
// keyboard-side math account for wherever the dock happens to be.
//
// GlobalStates.oskOpen suppresses both reveal-when-idle and the pinned
// exclusive zone. In fullscreen Souveraine's navigation rail explicitly
// reveals or hides the dock; it never times out or opens another surface.
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import "." as DockLocal
import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Quickshell.Io
import Quickshell.Hyprland
import Quickshell
import Quickshell.Widgets
import Quickshell.Wayland
import Quickshell.Hyprland

Scope { // Scope
    id: root
    property bool pinned: Config.options?.dock.pinnedOnStartup ?? false

    // Dock visibility as one explicit state instead of overlapping booleans.
    //   HIDDEN  - fully tucked below the edge
    //   SHOWN   - visible without claiming exclusive space
    //   PINNED  - visible AND reserving an exclusive zone
    // OSK-open and previewPopup-hover are *inputs* to this, not states.
    enum DockState { Hidden, Shown, Pinned }

    // The normal dock pin is suppressed while the OSK is open. Fullscreen
    // always takes precedence, so fullscreen remains genuinely edge-to-edge
    // until the navigation rail explicitly reveals the dock.
    property bool effectivePinned: root.pinned && !GlobalStates.oskOpen
    property bool autoHide: Config.options?.dock.autoHide ?? false
    property bool pointerAtDock: false
    property bool pointerReveal: false

    function notePointerAtDock(present) {
        root.pointerAtDock = present;
        if (present) {
            hideTimer.stop();
            revealTimer.restart();
        } else {
            revealTimer.stop();
            hideTimer.restart();
        }
    }

    Timer {
        id: revealTimer
        interval: Config.options?.dock.revealDelayMs ?? 120
        onTriggered: root.pointerReveal = root.pointerAtDock
    }

    Timer {
        id: hideTimer
        interval: Config.options?.dock.hideDelayMs ?? 350
        onTriggered: if (!root.pointerAtDock) root.pointerReveal = false
    }

    // The dock's state projection + guarded mutation surface for external
    // callers (the agent via Souveraine's harness). Lives here, not as a
    // qs.services singleton, to avoid a circular import with GlobalStates.
    DockLocal.DockManifest { id: dockManifest; visibility: root.visibility }

    // requestDockShow (previewPopup hover) is threaded up from DockApps via
    // this alias so the state computation can see it in one place.
    property bool previewShowing: false

    // App mode: any fullscreen window on the focused monitor owns the
    // display, so the dock hides. Two previous checks both had blind spots:
    //   (1) ws.toplevels scan via ext-foreign-toplevel-list used
    //       wayland?.fullscreen which is unreliable, and never saw
    //       standalone qs -p windows (souveraine-settings).
    //   (2) HyprlandData.activeWindow?.fullscreen === 2 only saw the
    //       focused window — missed fullscreen apps that lost focus to a
    //       layer-shell surface (the dock itself, notifications, OSK).
    // HyprlandData.windowList (hyprctl clients -j) has ALL windows with
    // their real fullscreen mode. Scan it for any fullscreen === 2 window
    // on the focused monitor. Reactive: HyprlandData updates on every
    // Hyprland event.
    readonly property bool activeMonitorHasFullscreen: {
        const focusedId = HyprlandData.monitors.find(m => m.focused)?.id;
        if (focusedId === undefined) return false;
        return HyprlandData.windowList.some(w => w.fullscreen === 2 && w.monitor === focusedId);
    }

    function computeDockState() {
        // Multitasking owns the screen. The dock is *home's* furniture, and
        // home is one of the cards — a dock over the strip is one destination's
        // furniture drawn on top of the picker for all of them. Casey,
        // 2026-08-16: *"remove the dock it should not be there."*
        //
        // First in the ladder, above the home check, because the gesture is
        // most often made **from** home: `activeZone` is still 0 for the whole
        // time the overview is up, so every rung below this returns PINNED.
        //
        // The peek counts. The destination draws from the first climbing pixel
        // (`SystemGestureRail.peekWanted`), so the dock leaving as the cards
        // arrive is one motion rather than two events.
        if (GlobalStates.missionControlOpen || GlobalStates.overviewOpen
            || GlobalStates.missionPeek)
            return Dock.DockState.Hidden;

        // USB Hands owns the bottom of home while its trackpad is actually
        // open. Arming the wire alone changes no furniture; opening the
        // conditional controller makes the dock yield until it closes.
        if (HidController.active)
            return Dock.DockState.Hidden;

        // A visible keyboard owns the bottom edge. This must precede the home
        // zone's unconditional PINNED return below; otherwise home stacks the
        // dock's exclusive zone under the OSK and charges the display twice.
        // The deliberate pulse remains reachable, but is SHOWN rather than
        // PINNED so it does not reserve another strip of screen.
        if (GlobalStates.oskOpen)
            return GlobalStates.dockRevealPulse
                ? Dock.DockState.Shown : Dock.DockState.Hidden;

        // Zone one is home, and home has a dock. Casey, 2026-08-05: *"The dock
        // should just always be on zone one."*
        //
        // Checked before everything below because on home it is not a reveal,
        // a pulse, or a consequence of nothing being focused — it is furniture
        // that is simply there, the way the widget space around it is. The
        // whole ladder underneath decides when to show a dock that is normally
        // absent; on home there is nothing to decide.
        //
        // The compositor is the authority on which zone is active — the shell
        // asking `{"op":"workspaces"}` and believing its own copy is the
        // second-decider shape — so this reads ViewtopControl's last answer and
        // treats "unknown" as not-home rather than guessing.
        if (ViewtopControl.activeZone === ViewtopControl.homeZone) {
            if (GlobalStates.dockSuppressed)
                return Dock.DockState.Hidden;
            if (root.autoHide)
                return (root.pointerReveal || root.previewShowing
                        || GlobalStates.dockRevealed || GlobalStates.dockRevealPulse)
                    ? Dock.DockState.Shown : Dock.DockState.Hidden;
            return root.effectivePinned
                ? Dock.DockState.Pinned : Dock.DockState.Shown;
        }
        // And off home it is gone — including when `pinned` is set, which is
        // the config that used to win. Casey, 2026-08-05: *"when I tap on an
        // app the dock shouldn't be there anymore."* The positive check above
        // is not enough on its own: `effectivePinned` sits further down the
        // ladder and returned Pinned on every zone, so opening an app left the
        // dock exactly where it was.
        //
        // `-1` is "not asked yet" and deliberately falls through rather than
        // hiding: a compositor that has not answered must not take the dock
        // away, or a slow first reply reads as the dock being broken.
        if (ViewtopControl.activeZone >= 0)
            return Dock.DockState.Hidden;
        if (root.activeMonitorHasFullscreen)
            return (GlobalStates.dockRevealed || GlobalStates.dockRevealPulse)
                ? Dock.DockState.Shown : Dock.DockState.Hidden;
        // An explicit pulse outranks suppression and the OSK — it exists to
        // glance at the dock while the keyboard is up.
        if (GlobalStates.dockRevealPulse)
            return Dock.DockState.Shown;
        // NOT gated on `oskOpen` here, and that is the point.
        //
        // Suppressing the dock whenever `oskOpen` was true — above
        // `effectivePinned`, so it applied in every state — hid the dock
        // PERMANENTLY on the device. `oskOpen` is not "the keyboard is on
        // screen": GlobalStates' own comment says squeekboard hides itself
        // whenever input-method focus drops and a hold re-asserts it, and the
        // journal shows exactly that — self-showed / self-hid every couple of
        // seconds, ending in `Visible=true` with no keyboard in front of the
        // user. Gating a persistent surface on a flag that flaps turns a
        // cosmetic overlap into a dock nobody can reach.
        //
        // The real signal is the keyboard's exclusive zone, which the
        // compositor already applies: an unpinned dock declares zone 0 and is
        // placed above the keyboard for free, the same mechanism that fixed the
        // pill. What remains is the PINNED case, where the dock reserves space
        // of its own and the two reservations stack. That wants fixing where
        // the zones are arbitrated, not by reading a D-Bus property the
        // keyboard flaps at us.
        //
        // Rail swipe-down dismissed a visible dock; swipe up brings it back.
        if (GlobalStates.dockSuppressed)
            return Dock.DockState.Hidden;
        if (root.effectivePinned)
            return Dock.DockState.Pinned;
        if (root.previewShowing)
            return Dock.DockState.Shown;
        // Empty desktop (nothing focused) reveals the dock, unless the OSK took
        // the bottom edge. Kept here, where it was: on an empty desktop a
        // spuriously-true `oskOpen` costs a reveal that would have been
        // cosmetic anyway, which is a very different price from hiding a pinned
        // dock the user relies on.
        if (!GlobalStates.oskOpen && !ToplevelManager.activeToplevel?.activated)
            return Dock.DockState.Shown;
        return Dock.DockState.Hidden;
    }

    property int dockState: computeDockState()

    // The one authored answer to "is the dock there?".
    //
    // `DockManifest` and `ShellModel` each recomputed this from GlobalStates
    // alone, which cannot see the zone rule above — so both reported "hidden"
    // while the dock sat pinned on home, and that is what the agent read.
    readonly property string visibility: root.dockState === Dock.DockState.Pinned
        ? "pinned"
        : root.dockState === Dock.DockState.Shown ? "shown" : "hidden"

    // The navigation rail's dock contract is intentionally just two operations:
    // swipe up reveals the dock; swipe down hides it. No timer, no overview.
    // The manifest + guarded pin/stack methods below extend the same target
    // so the agent (via Souveraine's harness, not a new integration) reaches
    // the dock through one IPC name. See services/DockManifest.qml and
    // docs/tasks/souveraine-shell-ecosystem.md.
    IpcHandler {
        target: "dock"

        function swipeUp(): void {
            GlobalStates.dockSuppressed = false;
            GlobalStates.dockRevealed = true;
        }

        function swipeDown(): void {
            GlobalStates.dockRevealed = false;
            GlobalStates.dockSuppressed = true;
        }

        function reveal(): void {
            GlobalStates.dockSuppressed = false;
            GlobalStates.dockRevealed = true;
        }

        // Toggle app-mode fullscreen on the REAL active window. The rail
        // can't use a bare `hyprctl dispatch fullscreen` because tapping it
        // makes the shell (org.quickshell) the focused surface, so hyprctl
        // would fullscreen the rail, not the app. The mechanism is
        // the one in the code below: Hyprland.activeToplevel stays the real
        // app window across a layer-shell tap, and we dispatch AT its
        // address. (An earlier draft went through ToplevelManager +
        // ToplevelHandle::setFullscreen — that path is not what runs.)
        function fullscreen(): void {
            // Use Hyprland 0.55's named fullscreen API. Numeric modes select
            // legacy/fake fullscreen behavior on this Lua dispatcher.
            // Clear a revealed dock before either direction of the toggle.
            GlobalStates.dockRevealed = false;
            // Hyprland.activeToplevel is Hyprland's real active APP window and
            // its .address is a stable window handle — a layer-shell rail tap
            // never becomes a Hyprland toplevel, so this stays the app even
            // after the tap focuses the shell. Dispatch AT that address so we
            // fullscreen the app, not whatever hyprctl thinks is focused.
            const raw = Hyprland.activeToplevel?.address;
            if (!raw) return;
            // .address may or may not carry the 0x prefix; normalize to exactly
            // one. Selector "address:0x..." is verified working on this fork.
            const addr = raw.startsWith("0x") ? raw : "0x" + raw;
            Quickshell.execDetached(["hyprctl", "dispatch",
                `hl.dsp.window.fullscreen({ window = "address:${addr}", mode = "fullscreen", action = "toggle" })`]);
        }

        // --- Manifest projection (read-only) + guarded mutation ----------
        // These delegate to DockManifest, which owns the projection shape
        // and the state checks. The agent calls `dock.manifest`, `dock.pin`,
        // etc. — never parsing QML. Refusals return {ok:false, reason}, not
        // errors, so a refused mutation is information the agent learns from.
        //
        // Returns are `string` (JSON), not `var`: quickshell marshals only
        // string/int/bool/double/color across IPC and silently maps a `var`
        // return to VOID (src/io/ipc.cpp ipcType()). Declared `: var`, these
        // registered as `(): void` and returned nothing at all — the {ok,
        // reason} contract never reached the caller. JSON-over-string is what
        // actually crosses the socket.

        function manifest(): string {
            return JSON.stringify(dockManifest.manifest());
        }

        function pin(appId: string): string {
            return JSON.stringify(dockManifest.pin(appId));
        }

        function unpin(appId: string): string {
            return JSON.stringify(dockManifest.unpin(appId));
        }

        function addToStack(stackId: string, appId: string): string {
            return JSON.stringify(dockManifest.addToStack(stackId, appId));
        }

        function removeFromStack(stackId: string, appId: string): string {
            return JSON.stringify(dockManifest.removeFromStack(stackId, appId));
        }

        function renameStack(stackId: string, newName: string): string {
            return JSON.stringify(dockManifest.renameStack(stackId, newName));
        }
    }

    // Settings-surface stub (entry point b). souveraine-settings doesn't
    // exist yet; the long-hold "App settings…" menu calls dockSettings.openApp
    // here. For now it just pulses the dock and logs, so nothing errors and
    // the future settings app has a stable IPC name to take over.
    IpcHandler {
        target: "dockSettings"

        function openApp(appId: string): void {
            console.log("[dockSettings] openApp stub for", appId,
                "- souveraine-settings not yet installed");
            GlobalStates.dockRevealed = true;
        }

        function open(): void {
            console.log("[dockSettings] open stub - souveraine-settings not yet installed");
            GlobalStates.dockRevealed = true;
        }
    }

    // Shell layer/state registry — declarative surface model + read
    // projection. Lives here (beside the dock, not as a qs.services
    // singleton) for the same circular-import reason as DockManifest.
    // See modules/common/ShellModel.qml and
    // docs/tasks/souveraine-shell-ecosystem.md section 1.
    ShellModel { id: shellModel; dockVisibility: root.visibility }

    IpcHandler {
        target: "shell"

        // JSON-over-string, not `var` — see the note on dock.manifest above.
        // A `var` return marshals as VOID and silently drops the payload.

        // surfaces() — registry list, one entry per meaningful surface with
        // layer, gating state, config gate, and a live `active` flag.
        function surfaces(): string {
            return JSON.stringify(shellModel.surfaces());
        }

        // state() — the GlobalStates bits that matter for layer gating, plus
        // the shell mode. Read-only snapshot.
        function state(): string {
            return JSON.stringify(shellModel.state());
        }
    }

    Variants {
        // For each monitor
        model: Quickshell.screens

        PanelWindow {
            id: dockRoot
            // Window
            required property var modelData
            screen: modelData

            // The dock window maps only when it should paint. This is what
            // actually hides it over a fullscreen app (a Hidden dockState
            // unmounts the layer entirely) — the earlier `visible` was
            // always-true and only `reveal` flipped, so the dock kept
            // painting over fullscreen. hoverToReveal (mouse, off by default)
            // keeps the strip alive for desktop pointer use.
            property bool reveal: root.dockState !== Dock.DockState.Hidden
                || root.pointerReveal
            // This space belongs to the always-on navigation rail. It is
            // visually empty and must also be absent from the dock's *input*
            // region; otherwise the dock receives touches before the rail.
            readonly property int gestureRailHeight: Config.options?.dock.gestureRailHeight ?? 32
            // The rail draws nothing on home — `SystemGestureRail` takes the
            // handle's opacity to 0 there, which is Casey's own call from
            // 2026-08-05: *"above the dock it has no function."* The dock went
            // on reserving the strip anyway, so home had 32 px of gap under the
            // bar holding space for a pill that was never going to appear.
            //
            // Only the *drawing* collapses. The input reservation above stays
            // exactly where it is, because the rail still takes both swipes on
            // home and a dock that claimed those pixels would eat the gesture.
            readonly property int railVisualHeight:
                ViewtopControl.activeZone === ViewtopControl.homeZone
                    ? 0 : gestureRailHeight
            visible: !GlobalStates.screenLocked && (reveal || root.autoHide
                || Config.options?.dock.hoverToReveal)

            anchors {
                bottom: true
                left: true
                right: true
            }

            exclusiveZone: root.dockState === Dock.DockState.Pinned ? implicitHeight - (Appearance.sizes.hyprlandGapsOut) - (Appearance.sizes.elevationMargin - Appearance.sizes.hyprlandGapsOut) : 0

            implicitWidth: dockBackground.implicitWidth
            WlrLayershell.namespace: "quickshell:dock"
            // Overlay, not Top: fullscreen windows render above the Top
            // layer, and the whole point of the edge swipe is to reach the
            // dock from a fullscreen app.
            WlrLayershell.layer: WlrLayer.Overlay
            color: "transparent"

            // Content-driven: tall enough for one 64px dock button + the
            // row's 8px bottom margin + the navigation rail, with Config
            // dock.height as a floor. A fixed config height (72) left the
            // visible bar ~36px for 64px buttons — icons poked out the
            // bottom and count dots landed under the bar. Size off the
            // BUTTON's implicit height, NOT dockRow.implicitHeight: the
            // separator's Layout margins inflate the row's implicit and
            // made the bar overshoot (content then top-aligned with a dead
            // band underneath).
            implicitHeight: Math.max(Config.options?.dock.height ?? 70,
                    overviewButton.implicitHeight + 8 + gestureRailHeight)
                + Appearance.sizes.elevationMargin + Appearance.sizes.hyprlandGapsOut

            mask: Region {
                item: dockInputRegion
            }

            // Deliberately smaller than dockMouseArea. The full MouseArea is
            // still useful for laying out and hovering dock content, while the
            // layer-shell only advertises the bar itself as touchable.
            Item {
                id: dockInputRegion
                anchors.horizontalCenter: parent.horizontalCenter
                width: dockMouseArea.width
                y: dockRoot.reveal
                    ? dockRoot.height - dockRoot.railVisualHeight
                        - Appearance.sizes.hyprlandGapsOut - dockRoot.visualHeight
                    : dockRoot.height - (Config.options?.dock.hoverRegionHeight ?? 2)
                height: dockRoot.reveal ? dockRoot.visualHeight
                    : (Config.options?.dock.hoverRegionHeight ?? 2)
            }

            readonly property real visualHeight:
                Math.max(Config.options?.dock.height ?? 60,
                    overviewButton.implicitHeight + 8)

            MouseArea {
                id: dockMouseArea
                height: parent.height
                anchors {
                    top: parent.top
                    topMargin: dockRoot.reveal ? 0 : Config.options?.dock.hoverToReveal ? (dockRoot.implicitHeight - Config.options.dock.hoverRegionHeight) : (dockRoot.implicitHeight + 1)
                    horizontalCenter: parent.horizontalCenter
                }
                implicitWidth: dockHoverRegion.implicitWidth + Appearance.sizes.elevationMargin * 2
                hoverEnabled: true
                onContainsMouseChanged: root.notePointerAtDock(containsMouse)

                Behavior on anchors.topMargin {
                    animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
                }

                Item {
                    id: dockHoverRegion
                    anchors.fill: parent
                    implicitWidth: dockBackground.implicitWidth

                    Item { // Wrapper for the dock background
                        id: dockBackground
                        anchors {
                            // Reserve the rail: bottom-anchor short of the
                            // window's bottom so the dock never covers it.
                            bottom: parent.bottom
                            bottomMargin: dockRoot.railVisualHeight
                            horizontalCenter: parent.horizontalCenter
                        }

                        Behavior on anchors.bottomMargin {
                            animation: Appearance.animation.elementMove.numberAnimation.createObject(this)
                        }

                        implicitWidth: dockRow.implicitWidth + 5 * 2
                        height: dockRoot.visualHeight + Appearance.sizes.elevationMargin

                        StyledRectangularShadow {
                            target: dockVisualBackground
                        }
                        Rectangle { // The real rectangle that is visible
                            id: dockVisualBackground
                            property real margin: Appearance.sizes.elevationMargin
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: Appearance.sizes.hyprlandGapsOut
                            height: dockRoot.visualHeight
                            color: Appearance.colors.colLayer0
                            border.width: 1
                            border.color: Appearance.colors.colLayer0Border
                            radius: Appearance.rounding.large
                        }

                        RowLayout {
                            id: dockRow
                            // Anchored to the visible bar. Split top/bottom
                            // so the icons ride UP inside the bar instead of
                            // drooping out the bottom (uniform margins left
                            // them low; less top + more bottom lifts them).
                            anchors.fill: dockVisualBackground
                            anchors.topMargin: 0
                            anchors.bottomMargin: 8
                            // The background is built 2*padding wider than the
                            // row (dockBackground.implicitWidth); inset the row
                            // so that width actually becomes side padding —
                            // without it the first icon sat ON the rounded corner.
                            anchors.leftMargin: padding
                            anchors.rightMargin: padding
                            spacing: 3
                            property real padding: 5

                            DockApps {
                                id: dockApps
                                buttonPadding: dockRow.padding
                                // Keep the whole dock on-screen: the app list
                                // may take at most what's left after the fixed
                                // separator + overview button + paddings. Past
                                // that it scrolls horizontally.
                                maxWidth: dockRoot.width
                                    - Appearance.sizes.elevationMargin * 2
                                    - dockSeparator.implicitWidth
                                    - overviewButton.implicitWidth
                                    - dockRow.spacing * 2 - 5 * 2
                                onRequestDockShowChanged: root.previewShowing = requestDockShow
                            }
                            DockSeparator {
                                id: dockSeparator
                            }
                            DockButton {
                                id: overviewButton
                                Layout.fillHeight: true
                                onClicked: GlobalStates.overviewOpen = !GlobalStates.overviewOpen
                                topInset: Appearance.sizes.hyprlandGapsOut + dockRow.padding
                                bottomInset: Appearance.sizes.hyprlandGapsOut + dockRow.padding
                                contentItem: MaterialSymbol {
                                    anchors.fill: parent
                                    horizontalAlignment: Text.AlignHCenter
                                    font.pixelSize: parent.width / 2
                                    text: "apps"
                                    color: Appearance.colors.colOnLayer0
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
