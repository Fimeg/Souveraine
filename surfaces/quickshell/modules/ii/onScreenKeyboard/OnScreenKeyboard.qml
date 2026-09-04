// Pixel3Arch replacement for ii's stock OnScreenKeyboard.qml (2026-07-07,
// reworked 2026-07-10: wvkbd -> squeekboard).
//
// ii's own on-screen keyboard is retired — squeekboard (3-finger swipe-up
// gesture, or the pill's long-hold) is the real keyboard now. It also
// auto-shows/hides itself on text-field focus via input-method-v2, which
// wvkbd never could. See docs/phone-shell-ux.md.
//
// This file keeps ii's OSK *plumbing* (GlobalStates.oskOpen, the "osk" IPC
// target, the oskToggle/oskOpen/oskClose global shortcuts — both the pill's
// long-hold and the 3-finger swipe-up gesture call `osk toggle`) all still
// working, but no longer renders a keyboard itself. It drives squeekboard
// over its D-Bus visibility interface (sm.puri.OSK0.SetVisible).
//
// Tapping the OSK while search/overview (or a sidebar) is open used to close
// it out from under you — that's `GlobalFocusGrab`'s HyprlandFocusGrab
// (a real Wayland protocol, hyprland_focus_grab_v1) clearing because the
// tap landed outside its whitelisted surfaces. Two "shield window" attempts
// to make wvkbd count as "inside" that whitelist both failed on real
// device testing:
//   1. mask: Region {} (empty, no item) — theory was this claims zero
//      input area so taps pass through to wvkbd underneath while the
//      window still counts toward the grab. Wrong in practice: search
//      still closed on every tap, meaning an empty Region does NOT behave
//      like zero input — it behaves like an unmasked/default full-window
//      area, and the shield silently ate the taps itself.
//   2. mask: Region { item: <full-rect Item> } — mirrors the stock OSK's
//      own working mask pattern exactly, but the stock OSK's mask matched
//      ITS OWN visible key grid (so taps landed on real buttons). Our
//      shield has no buttons — a full-covering mask would swallow every
//      tap meant for wvkbd, making the keyboard untappable. Never shipped;
//      caught before deploying back to the phone.
// There is no Quickshell or hyprctl API to add an arbitrary external
// process's Wayland surface (wvkbd is not a Quickshell QObject) to
// HyprlandFocusGrab's whitelist — confirmed via the actual
// hyprland-focus-grab-v1 protocol docs, which only expose whitelisting via
// the compositor-side protocol request, not anything scriptable from here
// without writing a standalone Wayland client. Not worth it for this.
//
// The actual fix lives in Overview.qml (and would need the same pattern in
// any other GlobalFocusGrab-dismissable surface if this bites there too):
// stop registering as dismissable at all while GlobalStates.oskOpen is
// true, so there's nothing for an outside tap to clear in the first place.
// See the comment there for details.
import qs
import qs.services
import qs.modules.common
import QtQuick
import Quickshell.Io
import Quickshell
import Quickshell.Hyprland

Scope {
    id: root

    // squeekboard exposes sm.puri.OSK0.SetVisible(b) on the session bus.
    // Unlike wvkbd's signal dance this is an absolute set, so show/hide
    // can't flip the wrong way. Known ceiling: squeekboard also shows and
    // hides ITSELF on input-method focus, and oskOpen doesn't hear about
    // that — the gesture toggle can need two swipes after an auto-show.
    // Ask the bus first. If no owner exists, start the package-owned service;
    // the owner monitor below pushes the pending visibility intent as soon as
    // Squeekboard claims the name.
    function showOsk() {
        Quickshell.execDetached(["sh", "-c",
            "busctl --user call sm.puri.OSK0 /sm/puri/OSK0 sm.puri.OSK0 " +
            "SetVisible b true 2>/dev/null || " +
            "systemctl --user start squeekboard.service >/dev/null 2>&1"])
    }
    function hideOsk() {
        Quickshell.execDetached(["busctl", "call", "--user",
            "sm.puri.OSK0", "/sm/puri/OSK0", "sm.puri.OSK0", "SetVisible", "b", "false"])
    }

    Connections {
        target: GlobalStates
        function onOskOpenChanged() {
            if (GlobalStates.oskOpen) {
                root.showOsk();
            } else {
                root.hideOsk();
            }
        }
    }

    // squeekboard auto-shows/hides ITSELF on input-method focus (e.g. a
    // sidebar taking or dropping a text field) without telling us. oskOpen
    // then lies: the dock stays suppressed and the gesture rail floats at
    // keyboard height over nothing until someone manually toggles. Mirror
    // squeekboard's real Visible property back into oskOpen so there is
    // exactly one truth. Setting oskOpen from here re-triggers SetVisible
    // with the value squeekboard already has, which is a harmless no-op.
    // A self-hide is squeekboard's opinion, not the user's. While a surface
    // holds the keyboard (GlobalStates.oskHolds — polkit's password field is
    // the first) that opinion is overridden and the keyboard comes straight
    // back. Bounded: after this many re-asserts within one hold we stop and
    // say so, rather than trading SetVisible calls with squeekboard forever.
    property int maxReasserts: 5
    property int reassertCount: 0

    // An owner change is not a visibility event.
    //
    // sm.puri.OSK0 is a *name*, not a process. Boot and a crash-restart can
    // both put a fresh Squeekboard process behind it.
    // Each new process announces its own idea of Visible, and until now this
    // monitor mirrored that into oskOpen — indistinguishable from the
    // keyboard deciding to show itself. Measured on 2026-08-05: shell up at
    // 03:03:38, `[osk] squeekboard self-showed` at 03:03:43, nine seconds
    // after boot with nobody near the phone.
    //
    // A process that just started has no history. It cannot know whether the
    // user wanted a keyboard. The shell does — oskOpen is the continuity of
    // that intent across the keyboard's whole lifetime. So on an owner change
    // we PUSH intent onto the new owner instead of PULLING state out of it,
    // and we ignore its opening claim while our push is in flight.
    //
    // This is also why "the keyboard comes back on wake" needs no compositor
    // change to stop hurting: wake perturbs the display connection, the OSK
    // restarts, and it is the mirror — not the compositor and not the
    // keyboard — that turns that restart into a keyboard on the user's
    // screen.
    property string oskOwner: ""
    property int ownerSettleMs: 2000
    // NOT a binding on Date.now() — QML bindings do not re-evaluate because
    // time passed, so a `Date.now() - t < ms` property latches at creation
    // and never clears. A timer is the only honest way to express "for a
    // moment after".
    property bool ownerSettling: false

    Timer {
        id: ownerSettleTimer
        interval: root.ownerSettleMs
        onTriggered: {
            root.ownerSettling = false;
            // Push once more on the way out. The first push can lose a race to
            // Squeekboard self-showing after startup. Whoever speaks last
            // during the settle window does not get to win; intent does.
            root.assertIntent();
        }
    }

    function assertIntent(): void {
        if (GlobalStates.oskOpen) root.showOsk();
        else root.hideOsk();
    }

    function adoptOwner(owner: string): void {
        if (owner === root.oskOwner) return;
        const previous = root.oskOwner;
        root.oskOwner = owner;
        root.ownerSettling = true;
        ownerSettleTimer.restart();
        if (owner === "") {
            console.log("[osk] keyboard left the bus (was " + previous + ")");
            return;
        }
        console.log("[osk] keyboard is now " + owner
            + (previous === "" ? "" : " (was " + previous + ")")
            + "; re-asserting oskOpen=" + GlobalStates.oskOpen);
        // Push our intent, not theirs. A fresh keyboard claiming Visible=true
        // when nobody asked for one gets closed here — and again when the
        // settle window closes, in case it spoke after we did.
        root.assertIntent();
    }

    Connections {
        target: GlobalStates
        function onOskHoldsChanged() {
            if (GlobalStates.oskHolds.length > 0) root.reassertCount = 0;
        }
    }

    Process {
        id: oskVisMonitor
        running: true
        command: ["gdbus", "monitor", "--session",
                  "--dest", "sm.puri.OSK0", "--object-path", "/sm/puri/OSK0"]
        stdout: SplitParser {
            onRead: line => {
                // gdbus prints the owner of --dest at startup and again on
                // every NameOwnerChanged. These lines were being dropped by
                // the 'Visible' filter below; they are the evidence we need.
                //   The name sm.puri.OSK0 is owned by :1.41
                //   The name sm.puri.OSK0 does not have an owner
                const owned = line.match(/is owned by (\S+)/);
                if (owned) {
                    root.adoptOwner(owned[1]);
                    return;
                }
                if (line.includes("does not have an owner")) {
                    root.adoptOwner("");
                    return;
                }

                if (!line.includes("'Visible'")) return;
                const vis = line.includes("<true>");

                // Still inside an owner change: this is the new process
                // introducing itself, or the echo of the intent we just
                // pushed at it. Either way it is not the user speaking.
                if (root.ownerSettling) {
                    if (vis !== GlobalStates.oskOpen)
                        console.log("[osk] ignoring Visible=" + vis
                            + " from freshly-arrived " + root.oskOwner
                            + "; oskOpen=" + GlobalStates.oskOpen + " stands");
                    return;
                }
                if (!vis && GlobalStates.oskHolds.length > 0) {
                    if (root.reassertCount >= root.maxReasserts) {
                        console.log("[osk] squeekboard self-hid under hold ["
                            + GlobalStates.oskHolds.join(",") + "] more than "
                            + root.maxReasserts + " times; giving up the hold");
                        GlobalStates.oskDropHolds();
                        GlobalStates.oskOpen = false;
                        return;
                    }
                    root.reassertCount++;
                    console.log("[osk] squeekboard self-hid while held by ["
                        + GlobalStates.oskHolds.join(",") + "]; re-asserting ("
                        + root.reassertCount + "/" + root.maxReasserts + ")");
                    root.showOsk();
                    return;
                }
                if (GlobalStates.oskOpen !== vis) {
                    console.log("[osk] squeekboard self-" + (vis ? "showed" : "hid") + ", syncing oskOpen");
                    GlobalStates.oskOpen = vis;
                }
            }
        }
    }

    // Every deliberate close — pill, gesture, shortcut, IPC — goes through
    // here, because closing by hand is what drops a hold.
    function userClose() {
        GlobalStates.oskDropHolds();
        GlobalStates.oskOpen = false;
    }

    // Unlocking must not leave a keyboard behind.
    //
    // Nothing asks for one — the lock module never touches `oskOpen`, and only
    // polkit takes a hold. Squeekboard self-shows whenever input-method
    // focus lands on them (GlobalStates' own note: "self-showed / self-hid
    // every couple of seconds, ending in Visible=true with no keyboard in front
    // of the user"), and the surfaces coming back at unlock are exactly such a
    // focus change. So this is not a hold to release, it is a keyboard nobody
    // requested — closed on the unlock edge, the same place the state machine
    // treats as "the session is yours again".
    //
    // `userClose()`, not `oskOpen = false`: it drops holds too, so a stale
    // polkit hold taken before the lock cannot re-assert the keyboard the
    // instant this clears.
    Connections {
        target: GlobalStates
        function onScreenLockSecureChanged() {
            if (!GlobalStates.screenLockSecure && GlobalStates.oskOpen)
                root.userClose();
        }
    }

    IpcHandler {
        target: "osk"

        function toggle(): void {
            if (GlobalStates.oskOpen) root.userClose();
            else GlobalStates.oskOpen = true;
        }

        function close(): void {
            root.userClose();
        }

        function open(): void {
            GlobalStates.oskOpen = true;
        }

        // Which surfaces are holding the keyboard open, if any. The pill and
        // the agent both ask "why won't this close"; this answers it.
        function holds(): string {
            return JSON.stringify(GlobalStates.oskHolds);
        }

        // Double-tapping the pill while the keyboard is open calls this —
        // pops the (suppressed, see Dock.qml) dock back up for a few
        // seconds without having to close the keyboard first.
        function pulseDock(): void {
            GlobalStates.pulseDockReveal();
        }
    }

    GlobalShortcut {
        name: "oskToggle"
        description: "Toggles on screen keyboard on press"

        onPressed: {
            if (GlobalStates.oskOpen) root.userClose();
            else GlobalStates.oskOpen = true;
        }
    }

    GlobalShortcut {
        name: "oskOpen"
        description: "Opens on screen keyboard on press"

        onPressed: {
            GlobalStates.oskOpen = true;
        }
    }

    GlobalShortcut {
        name: "oskClose"
        description: "Closes on screen keyboard on press"

        onPressed: {
            root.userClose();
        }
    }
}
