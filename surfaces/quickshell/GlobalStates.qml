// Souveraine patch to ii's stock GlobalStates.qml.
//
// Adds dockRevealed: the explicit, persistent dock state driven by the
// Souveraine's integrated navigation rail. Everything else in this file is
// unchanged stock ii — diff against upstream before re-applying this patch if
// ii updates.
import qs.modules.common
import qs.services
import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io
pragma Singleton
pragma ComponentBehavior: Bound

Singleton {
    id: root

    // ── Write authority ────────────────────────────────────────────
    // Each property below names its single authorized writer. Other
    // files MUST NOT write to these directly — route through the
    // writer's IPC or signal instead. This is a convention, not
    // enforced at runtime; a lint rule or future QML analyzer should
    // flag writes from files other than the named writer.
    //
    // Stock ii properties (barOpen, crosshairOpen, sidebarLeftOpen,
    // sidebarRightOpen, mediaControlsOpen, osdBrightnessOpen,
    // osdVolumeOpen, oskOpen, overlayOpen, overviewOpen,
    // regionSelectorOpen, searchOpen, screenTranslatorOpen,
    // sessionOpen, wallpaperSelectorOpen, workspaceShowNumbers) are
    // written by their respective ii modules. Souveraine does not
    // own these — they follow ii's own conventions.

    // WRITER: ii bar module
    property bool barOpen: true
    // WRITER: ii crosshair module
    property bool crosshairOpen: false
    // WRITER: ii sidebar module
    property bool sidebarLeftOpen: false
    // WRITER: ii sidebar module
    property bool sidebarRightOpen: false
    // WRITER: ii media controls module
    property bool mediaControlsOpen: false
    // WRITER: ii OSD module
    property bool osdBrightnessOpen: false
    // WRITER: ii OSD module
    property bool osdVolumeOpen: false
    // WRITER: ii OSK module (OnScreenKeyboard.qml)
    property bool oskOpen: false
    // WRITER: OnScreenKeyboard.qml (via oskHold/oskRelease/oskDropHolds).
    // The names of surfaces that need the keyboard for as long as they are
    // up, rather than for the instant they appeared.
    //
    // Setting oskOpen once is not enough. squeekboard hides ITSELF whenever
    // input-method focus drops (the journal is full of `self-hid`), and a
    // layer-shell password prompt does not reliably hold that focus — so a
    // surface that pokes the keyboard open gets a keyboard that vanishes
    // while the field is still waiting for input. A hold survives that: the
    // keyboard is re-asserted for as long as the holder is up.
    //
    // A user close outranks every hold (oskDropHolds). The person in front
    // of the phone gets to dismiss the keyboard even mid-prompt.
    property var oskHolds: []
    // oskOpen as it stood when the first hold was taken, so releasing the
    // last hold restores rather than clobbers a keyboard the user had open.
    property bool oskHoldRestoreOpen: false

    function oskHold(reason) {
        if (root.oskHolds.indexOf(reason) >= 0) return;
        if (root.oskHolds.length === 0)
            root.oskHoldRestoreOpen = root.oskOpen;
        root.oskHolds = root.oskHolds.concat([reason]);
        root.oskOpen = true;
    }

    function oskRelease(reason) {
        const next = root.oskHolds.filter(r => r !== reason);
        if (next.length === root.oskHolds.length) return;
        root.oskHolds = next;
        if (next.length === 0)
            root.oskOpen = root.oskHoldRestoreOpen;
    }

    // The user closed the keyboard by hand. Every hold drops — otherwise the
    // re-assert below would fight them key for key.
    function oskDropHolds() {
        if (root.oskHolds.length === 0) return;
        root.oskHolds = [];
        root.oskHoldRestoreOpen = false;
    }

    // Locking closes the keyboard, and unlocking does not bring it back.
    //
    // `oskHoldRestoreOpen` remembers whether the keyboard was up when the
    // first hold was taken, so releasing the last hold restores rather than
    // clobbers it. Across a lock that restore is wrong: the keyboard was up
    // because of something the user was typing into before they put the phone
    // down, and the first thing they see on unlocking is a keyboard over
    // whatever they actually came back for.
    //
    // Intent to type does not survive the screen going away. Anything that
    // still wants the keyboard — a field taking focus again — will ask for it,
    // which is the path that already works.
    onScreenLockedChanged: {
        if (!root.screenLocked) return;
        root.oskHolds = [];
        root.oskHoldRestoreOpen = false;
        root.oskOpen = false;
    }
    // WRITER: ii overlay module
    property bool overlayOpen: false
    // WRITER: ii overview module + Dock.qml IPC
    property bool overviewOpen: false
    // WRITER: ii region selector module
    property bool regionSelectorOpen: false
    // WRITER: ii search module
    property bool searchOpen: false
    // WRITER: LockScreen.qml — the shell's lock *request*. Drives
    // WlSessionLock and hides ordinary surfaces immediately. Deliberately
    // separate from screenLockSecure (compositor ack). Consumers that
    // disclose personal data must gate on secure, not merely request.
    property bool screenLocked: false
    // WRITER: IdleCoordinator.qml — true while the display is genuinely
    // in use (Active or Waking). Widgets/pollers whose data may go stale
    // while the screen is dimmed/locked/asleep gate their timers on this
    // instead of importing IdleCoordinator into ii-base.
    property bool displayActive: true
    // WRITER: LockScreen.qml — WlSessionLock.secure. Only true once the
    // compositor has acknowledged the lock surface. This is the real
    // "session is locked" signal; screenLocked is just the request.
    property bool screenLockSecure: false
    // WRITER: LockScreen.qml
    property bool screenLockContainsCharacters: false
    // WRITER: LockScreen.qml
    property bool screenUnlockFailed: false
    // WRITER: ii translator module
    property bool screenTranslatorOpen: false
    // WRITER: ii session module
    property bool sessionOpen: false
    // WRITER: GlobalShortcut handler in this file
    property bool superDown: false
    // WRITER: GlobalShortcut handler in this file
    property bool superReleaseMightTrigger: true
    // WRITER: GlobalShortcut handler in this file
    property real superPressTime: 0
    // WRITER: GlobalShortcut handler in this file
    property real superLastPressDuration: -1
    // WRITER: GlobalShortcut handler in this file
    property real superLastReleaseTime: 0
    // WRITER: ii wallpaper selector module
    property bool wallpaperSelectorOpen: false
    // WRITER: ii workspace module
    property bool workspaceShowNumbers: false
    // WRITER: LockScreen.qml — true from shell start until the lock
    // surface is secure. Covers Hyprland's boot render, continuing the
    // C splash bloom animation. LockScreen.onSecureChanged clears it.
    property bool bootBloomActive: true
    // WRITER: Dock.qml IPC (swipeUp/reveal) + navigation rail. The
    // fullscreen dock toggle. Deliberately has no timer.
    property bool dockRevealed: false

    // WRITER: navigation rail (triple swipe-up). The Souveraine
    // process/task surface. Deliberately distinct from overviewOpen
    // (app launcher/search) — this is the running-work view.
    property bool missionControlOpen: false

    // GONE: zonePullProgress. Its job moved to `ZoneTransition` (TASK-60).
    //
    // It was the right idea — one level the transition and the destination
    // share — and the wrong home. The motion needs *two* curves over one clock:
    // the carry keeps travelling as the thumb climbs past the multitasking
    // detent toward home, while the destination has to recede, or the preview
    // shows somewhere the release will not take you. Two curves derived in two
    // files is how this task's original bug was built, so both now live with
    // the owner: `ZoneTransition.clock`, `.shift`, `.presence`.
    //
    // Not left here as a mirror. A state property nobody writes is a shadow
    // copy waiting for a reader, and this file has a rule about those.

    // WRITER: navigation rail, for the whole of an upward pull. The cards are
    // DRAWN, nothing is committed — release still decides. Gates presentation
    // only (visibility, mask, which body the loader builds); never focus,
    // dismissal or state, or a preview would be a decision.
    //
    // Any pull, not a dwell. It used to wait 140 ms of holding still, which
    // meant an ordinary swipe shrank the real window over the live app and then
    // the entire destination appeared in one frame — the "swipe to this swap is
    // awkward" complaint, and TASK-60's unmet acceptance. The old worry, that a
    // fast flick to Home would flash the blur, is answered by the fade being
    // continuous and receding past the detent rather than by hiding the surface
    // until the thumb stops.
    property bool missionPeek: false

    // WRITER: Dock.qml IPC (swipeDown). A rail swipe-down on a visible
    // dock dismisses it in ANY state — including pinned and
    // shown-on-empty-desktop. Swipe up clears it.
    property bool dockSuppressed: false

    // WRITER: this file (pulseDockReveal) + OnScreenKeyboard.qml IPC.
    // Transient dock reveal: shows for 3s and restores prior state.
    // Restored 2026-07-16 — definition was lost in a refactor while
    // its IPC caller survived, so pulseDock threw.
    property bool dockRevealPulse: false
    function pulseDockReveal() {
        root.dockRevealPulse = true;
        dockRevealPulseTimer.restart();
    }
    Timer {
        id: dockRevealPulseTimer
        interval: 3000
        onTriggered: root.dockRevealPulse = false
    }

    // WRITER: DockAppButton drag lifecycle. True while a dock icon drag
    // is in flight. Read by DockManifest's state checks so structural
    // edits (pin/stack mutations) can't land mid-drag.
    property bool dockDragInProgress: false

    function superPressDuration() {
        const now = Date.now();
        if (root.superPressTime > 0)
            return now - root.superPressTime;
        if (root.superLastReleaseTime > 0 && now - root.superLastReleaseTime < 250)
            return root.superLastPressDuration;
        return -1;
    }

    function shouldSuppressSuperReleaseSearch() {
        const autoHide = Config?.options?.bar?.autoHide;
        const showWhenPressingSuper = autoHide?.showWhenPressingSuper;
        if (!autoHide?.enable || !showWhenPressingSuper?.enable || !showWhenPressingSuper?.suppressSearchOnHold)
            return false;

        const duration = root.superPressDuration();
        return duration >= (showWhenPressingSuper?.suppressSearchDelay ?? showWhenPressingSuper?.delay ?? 140);
    }

    onSidebarRightOpenChanged: {
        if (GlobalStates.sidebarRightOpen) {
            Notifications.timeoutAll();
            Notifications.markAllRead();
        }
    }

    GlobalShortcut {
        name: "workspaceNumber"
        description: "Hold to show workspace numbers, release to show icons"

        onPressed: {
            root.superDown = true
            root.superPressTime = Date.now()
            root.superLastPressDuration = -1
        }
        onReleased: {
            const now = Date.now()
            if (root.superPressTime > 0)
                root.superLastPressDuration = now - root.superPressTime
            root.superLastReleaseTime = now
            root.superPressTime = 0
            root.superDown = false
        }
    }
}
