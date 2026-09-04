pragma Singleton
pragma ComponentBehavior: Bound

import qs
import qs.modules.common
import Quickshell
import Quickshell.Io
import QtQuick

/**
 * Selection — compositor-wide text selection, for the TASK-18 action menu.
 *
 * Mechanism (verified on blueline 2026-07-29): the phone's Hyprland advertises
 * `zwp_primary_selection_device_manager_v1` plus BOTH data-control managers
 * (`zwlr_data_control_manager_v1`, `ext_data_control_manager_v1`).  data-control
 * is what lets an unfocused client observe selection changes, so
 * `wl-paste --primary --watch` sees every selection with no per-app hooks and no
 * compositor patch.  That answers TASK-18's open "selection-detection mechanism"
 * question: compositor-level, not per-app.  viewtop is not required for this.
 *
 * What the protocol does NOT give us is the selection's bounding rectangle.
 * Android solves this at the toolkit layer (ActionMode/FloatingToolbar) and Apple
 * in-app; neither is available to us.  So `anchorX/anchorY` is the pointer
 * position at selection time, which on a phone is where the finger lifted — the
 * same place Android puts its floating toolbar anyway.  Consumers should treat it
 * as a hint and clamp themselves on screen.
 *
 * ── Privacy, and why this service is written defensively ──────────────────
 *
 * A primary-selection watcher is a firehose of personal content: it receives
 * every selection on the device, including a password highlighted inside a
 * password manager.  Nothing in the doctrine's tier table (SESSION-AUTHORITY
 * §2) covers an ambient capability of that shape, so this service takes the
 * conservative reading of §9 (a reading is evidence, and this one is sensitive):
 *
 *   - The watcher only runs while `enabled` AND the session is genuinely
 *     unlocked.  On lock it is KILLED, not paused-and-buffered — there is
 *     nothing to leak from a process that isn't running.
 *   - `text` is cleared on lock, and never written to disk, never logged, and
 *     never put in the forensic trail.  DEVICE-STATE-MACHINE §11 wants intent
 *     recorded, not leaves; the leaf here is the user's private content.
 *   - Lock state is read live from GlobalStates (which mirrors
 *     WlSessionLock.secure) rather than cached, per doctrine §4: never hold
 *     state the protocol owns.  Gating on `screenLockSecure` and not
 *     `screenLocked` is deliberate — GlobalStates.qml:109 says disclosure gates
 *     on the compositor ack, not on the request.
 *   - console.log NEVER receives selection text, only its length.
 */
Singleton {
    id: root

    // User kill switch (settings → Selection). Default off: this capability
    // reads everything the user highlights, so it is opt-in, not opt-out.
    readonly property bool enabled: Config.options?.selection?.enable ?? false

    // The live selection. Empty when there is none, or whenever the session is
    // not genuinely unlocked. Never persisted.
    property string text: ""
    readonly property bool hasSelection: root.text.length > 0

    // Pointer position when the selection last changed — a hint for anchoring,
    // not the selection's real geometry (see the header). -1 when unknown.
    property int anchorX: -1
    property int anchorY: -1

    // True only when it is safe to observe selections at all.
    readonly property bool _permitted: root.enabled && !GlobalStates.screenLockSecure

    // Selections shorter than this are almost always an accidental drag.
    readonly property int _minChars: 2
    // A drag fires primary-selection repeatedly as it grows. Settle before
    // announcing, so consumers see one selection and not thirty.
    readonly property int _settleMs: 220

    signal selectionSettled(string text, int x, int y)
    signal selectionCleared()

    onEnabledChanged: root._reconcile()

    Connections {
        target: GlobalStates
        function onScreenLockSecureChanged() {
            root._reconcile();
        }
    }

    // Bring the watcher in line with policy, and scrub on the way down.
    function _reconcile() {
        if (root._permitted) {
            if (!watcher.running) {
                root._primed = false;
                watcher.running = true;
                console.log("[Selection] watching primary selection");
            }
            return;
        }
        if (watcher.running || root.text.length > 0) {
            console.log("[Selection] stopping watcher and clearing (permitted=false)");
        }
        watcher.running = false;
        settleTimer.running = false;
        root._clear();
    }

    function _clear() {
        const had = root.text.length > 0;
        root.text = "";
        root.anchorX = -1;
        root.anchorY = -1;
        if (had) root.selectionCleared();
    }

    // Called by the menu when the user dismisses it or an action consumes the
    // selection. Does not touch the compositor's selection — only our view of it.
    //
    // The dismissed text is remembered: the compositor's selection is unchanged
    // by dismissing, so any later re-read of it (a watcher restart) would
    // otherwise resurrect the chip the user just closed.
    function dismiss() {
        settleTimer.running = false;
        root._dismissedText = root.text;
        root._clear();
    }

    property string _pending: ""
    property string _dismissedText: ""

    // wl-paste --watch fires ONCE IMMEDIATELY with whatever the selection
    // already holds, before any new user action. That replay is not a fresh
    // selection: it resurrected an hour-old selection every time the watcher
    // restarted (i.e. on every unlock), leaving a chip that could never be
    // cleared because the primary buffer never changed again. The first
    // emission after a start only establishes the baseline.
    property bool _primed: false

    Component.onCompleted: root._reconcile()

    // ── the watcher ──────────────────────────────────────────────────────
    // `--watch cat` makes wl-paste run `cat` per change with the selection on
    // that child's stdin; wl-paste relays it to our stdout, so one long-lived
    // process yields a stream of selections. -n keeps trailing newlines off.
    // WAYLAND_DISPLAY is set explicitly: remote/`systemctl --user` contexts do
    // not inherit it reliably on this device.
    Process {
        id: watcher
        command: ["sh", "-c",
            "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1} " +
            "exec wl-paste --primary --watch sh -c 'cat; printf \"\\036\"'"]
        // \036 (record separator) terminates each selection: selections contain
        // newlines, so a line-based split would fragment multi-line text.
        stdout: SplitParser {
            splitMarker: "\u001e"
            onRead: data => root._onSelection(data)
        }
        onExited: (exitCode) => {
            if (!root._permitted) return; // intentional stop
            if (exitCode !== 0 && exitCode !== 15)
                console.log("[Selection] watcher exited", exitCode, "- selection menu is inert");
        }
    }

    function _onSelection(raw) {
        if (!root._permitted) return;
        const t = String(raw ?? "");
        // The startup replay: record it as already-seen and show nothing.
        if (!root._primed) {
            root._primed = true;
            root._dismissedText = t;
            console.log(`[Selection] baseline ${t.trim().length} chars (not shown)`);
            return;
        }
        // A re-read of something the user already dismissed is not a new
        // selection. Only an actual change re-opens the chip.
        if (t.length > 0 && t === root._dismissedText) return;
        if (t.trim().length < root._minChars) {
            // A cleared or trivial selection retires the menu rather than
            // leaving a stale one anchored over nothing.
            settleTimer.running = false;
            root._clear();
            return;
        }
        // A genuinely new selection clears the dismissal memory.
        root._dismissedText = "";
        root._pending = t;
        // Restart the settle window on every growth tick.
        settleTimer.restart();
    }

    Timer {
        id: settleTimer
        interval: root._settleMs
        repeat: false
        onTriggered: {
            if (!root._permitted || root._pending.length === 0) return;
            // Ask for the pointer position only once the selection has settled —
            // one hyprctl call per selection, not one per drag tick.
            cursorPos.running = true;
        }
    }

    // Anchor hint. A query, not a dispatch — the phone's hyprctl is a Lua-eval
    // variant where classic dispatch syntax fails, but queries are unaffected.
    // Failure is non-fatal: the menu falls back to its own placement.
    Process {
        id: cursorPos
        command: ["sh", "-c",
            "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1} hyprctl cursorpos 2>/dev/null"]
        stdout: StdioCollector {
            onStreamFinished: {
                let x = -1, y = -1;
                const m = String(text).match(/(-?\d+)\s*,\s*(-?\d+)/);
                if (m) { x = parseInt(m[1]); y = parseInt(m[2]); }
                root._announce(x, y);
            }
        }
        onExited: (exitCode) => {
            // StdioCollector already announced on success; only cover the case
            // where the process died without producing parseable output.
            if (exitCode !== 0 && root._pending.length > 0)
                root._announce(-1, -1);
        }
    }

    function _announce(x, y) {
        if (!root._permitted || root._pending.length === 0) return;
        root.text = root._pending;
        root._pending = "";
        root.anchorX = x;
        root.anchorY = y;
        // Length only — never the content.
        console.log(`[Selection] settled: ${root.text.length} chars, anchor ${x},${y}`);
        root.selectionSettled(root.text, x, y);
    }
}
