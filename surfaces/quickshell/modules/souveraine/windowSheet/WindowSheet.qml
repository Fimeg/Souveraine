// The window action sheet — the hand's verbs on one window.
//
// TASK-55. Three fingers tapped a window; the compositor named which one, the
// machine decided what that means, and this draws the answer. The split is
// `gesture.rs`'s own: "the compositor recognises, the shell draws, and the
// answer comes back as the ToCompositor intents that already work". This file
// is the drawing half, and ViewtopControl is the answering half.
//
// ## Why a scrim and not a blur
//
// GaussianBlur fails on the phone's GLES path — the Home drawer hit exactly
// this on 2026-08-05 and fell back to a flat translucent panel (`panelBg`).
// Repeating a known-broken effect here would trade a working sheet for a
// black rectangle on the one device that matters. The scrim reads as "the app
// is behind this and paused" without asking the GPU for something it refuses.
//
// ## Why the buttons are this big
//
// The selection chip is the house precedent for a floating action surface and
// it failed twice on device — "tiny, illegible, not working properly at all" —
// at 44 px tall with 18-20 px icons. TASK-55 Q6 makes not inheriting that a
// condition of this shipping. So: 76 px circles, 34 px glyphs, 28 px between
// them, and the row sits above the vertical centre so a thumb reaching from
// the bottom bezel does not cover the window being acted on.
//
// ## Close, and the floor under it
//
// A tap is `close` — the protocol asking, which a client may refuse or answer
// with a save-your-work prompt. A press-and-hold is `kill`, which always
// works and loses unsaved work. Two verbs on one button because they are the
// same intent at two levels of insistence, and because a separate always-kill
// button would get pressed by someone who meant the polite one.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Scope {
    id: scope

    // The window this sheet is about. 0 is "none" — the sheet is never open
    // without a subject, which is what keeps the no-window case from becoming
    // a blank sheet nobody can explain.
    property int target: 0
    property bool sheetOpen: false
    // Split has pinned a window and is waiting for the other half to be filled.
    property bool choosing: false
    // A window is grabbed and following the finger. The sheet is closed while
    // this is true — it would be covering the thing being moved — so the only
    // surface up is the Done bar.
    property bool moving: false
    // Which pose Zoom last applied. Kept on the scope rather than the button so
    // it survives the sheet closing and reopening on the same window — tapping
    // Zoom twice in two visits should continue the cycle, not restart it and
    // appear to do nothing.
    property int poseStep: 0

    // Whether the subject has left its zone's confinement, as the compositor
    // last reported it. Read, never remembered — a shell that kept its own idea
    // of this would draw Unfloat on a window something else had already put
    // back.
    readonly property bool targetFloating: {
        for (const w of ViewtopControl.windows)
            if (w.id === scope.target)
                return w.floating === true;
        return false;
    }

    // How long a hold on Close means kill. Longer than a tap could ever be,
    // short enough that a stuck app does not feel like a negotiation.
    readonly property int killHoldMs: 1000

    function openFor(id) {
        if (!id || id <= 0) {
            console.log("[window-sheet] refusing to open without a target");
            return;
        }
        // A different window starts both cycles over. They survive the sheet
        // closing on the *same* window on purpose; carrying them onto another
        // one meant the first tap of Zoom or Resize appeared to do nothing.
        if (id !== poseRamp.target) {
            scope.poseStep = 0;
            poseRamp.reset(id);
        }
        scope.target = id;
        // Ask where that window is, now. The poll runs every two seconds, and
        // a sheet that opened against two-second-old geometry would scrim the
        // wrong rectangle for exactly as long as it took to notice.
        ViewtopControl.refreshWindows();
        scope.sheetOpen = true;
    }

    function dismiss() {
        scope.sheetOpen = false;
        scope.choosing = false;
        // A grab outlives the sheet on purpose — Move closes the sheet to get
        // out of the way — so dismissing must not orphan one. Anything else
        // would leave the compositor holding a window for a finger that has
        // nothing left on screen telling it how to let go.
        if (scope.moving) {
            ViewtopControl.drop();
            scope.moving = false;
        }
        scope.target = 0;
    }

    // sessiond's `Action::WindowSheet` lands here: `qs -c souveraine ipc call
    // windowSheet open <id>`. The id arrives as a string because the executor
    // builds a command line; parsing is this side's job.
    IpcHandler {
        target: "windowSheet"

        function open(id: string): void {
            scope.openFor(parseInt(id, 10));
        }

        function close(): void {
            scope.dismiss();
        }
    }

    // The subject went away while the sheet was up.
    //
    // TASK-55's acceptance wants the sheet dismissed "without leaving a latch
    // behind". `onSucceeded` covers the exits this sheet asked for; it cannot
    // cover the app quitting on its own, the tray's Close all, or a card being
    // closed from the overview. The sheet stayed open over a dead id and every
    // button then refused with `gone`, which reads as six broken buttons.
    //
    // The compositor's `cancel` deliberately emits nothing — "a cancelled
    // gesture did not happen", and there is a test pinning that — so the
    // canvas is the honest signal, and it is already pushed.
    Connections {
        target: ViewtopControl
        function onWindowsChanged_() {
            if (scope.target <= 0)
                return;
            for (const w of ViewtopControl.windows)
                if (w.id === scope.target)
                    return;
            // Ends the grab too. A move whose window died leaves the compositor
            // holding nothing and the Done bar up with no way to reach it.
            //
            // The ramp is stopped by hand rather than by `dismiss`, which is
            // also the ordinary close and must leave the Zoom cycle where it
            // is — a spring still ticking at a dead id refuses once per frame.
            if (poseRamp.target === scope.target)
                poseRamp.reset(0);
            scope.dismiss();
        }
    }

    // A refusal the hand can see. `close` is a request, and a client that says
    // no must not look like a button that did nothing — that silence is the
    // complaint this whole task came from.
    Connections {
        target: ViewtopControl
        function onRefused(intent, reason) {
            if (!scope.sheetOpen)
                return;
            refusal.text = intent + " refused: " + reason;
            refusal.opacity = 1;
            refusalFade.restart();
        }
        function onSucceeded(intent) {
            // The window is gone or moved; the sheet has nothing left to be
            // about. Kept open for `pose`-style verbs would mean a sheet
            // pointing at a window that is no longer where it was.
            //
            // Except mid-Split: that `place` is the *first half* of a flow, and
            // dismissing on it would close the sheet before the chooser could
            // ask which window fills the other half.
            //
            // And except with the sheet already closed: `WindowResize` emits a
            // `place` per frame, and a closed sheet dismissing itself once per
            // frame would clear `target` under a mode that is still running.
            if (!scope.sheetOpen || scope.choosing)
                return;
            if (intent === "close" || intent === "kill" || intent === "place")
                scope.dismiss();
        }
    }

    // Resize's own mode, on its own surface. Outlives the sheet the way a move
    // does, and ends by its own Done bar.
    WindowResize {
        id: resize
        onFinished: ViewtopControl.refreshWindows()
    }

    // Zoom's travel. Outlives the sheet for the same reason `poseStep` does —
    // a ramp that stopped when the sheet closed would leave the window
    // part-scaled at whatever frame the dismissal landed on.
    PoseRamp {
        id: poseRamp
    }

    // The way out of move mode. A separate, small surface rather than part of
    // the sheet: while moving, the whole screen has to stay reachable by the
    // finger that is dragging the window, so a full-screen scrim would eat the
    // gesture it exists to support.
    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: moveBar
            required property var modelData
            screen: moveBar.modelData

            anchors { bottom: true; left: true; right: true }
            implicitHeight: 86
            color: "transparent"
            visible: scope.moving
            WlrLayershell.namespace: "souveraine:windowmove"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            exclusionMode: ExclusionMode.Ignore

            Rectangle {
                anchors.centerIn: parent
                implicitWidth: 220
                implicitHeight: 58
                radius: 29
                color: Appearance.colors.colLayer1
                border.width: 1
                border.color: Appearance.colors.colLayer1Active

                StyledText {
                    anchors.centerIn: parent
                    text: "Done moving"
                    color: Appearance.colors.colOnLayer1
                    font.pixelSize: Appearance.font.pixelSize.normal
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        ViewtopControl.drop();
                        scope.moving = false;
                        scope.target = 0;
                    }
                }
            }
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: win
            required property var modelData
            screen: win.modelData

            anchors { top: true; left: true; right: true; bottom: true }
            color: "transparent"
            visible: scope.sheetOpen
            WlrLayershell.namespace: "souveraine:windowsheet"
            WlrLayershell.layer: WlrLayer.Overlay
            // OnDemand rather than Exclusive: an exclusive grab on the polkit
            // surface stopped touch reaching the layer below it entirely
            // (2026-07-26), and this surface must not repeat that.
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand
            exclusionMode: ExclusionMode.Ignore

            // The tap-away target. Transparent and full-screen: dismissal by
            // tapping outside is load-bearing on a phone and is the only exit
            // that needs no explanation, so the *catcher* still spans the
            // glass even though the *scrim* no longer does.
            MouseArea {
                anchors.fill: parent
                onClicked: scope.dismiss()
            }

            // The scrim, over the target window only.
            //
            // Casey, 2026-08-05: *"Three finger tap overlay should overlay the
            // app it's talking about; not all apps."* Dimming everything said
            // the sheet was about the device; it is about one window, and on a
            // split screen it was covering the app you were not asking about.
            //
            // Geometry comes from the compositor's `workspaces` reply rather
            // than being computed here — the strip offset and any pose are
            // facts only it holds, which is the same reason the tap carries its
            // target instead of a raw point.
            Rectangle {
                id: scrim
                readonly property var target: {
                    for (const w of ViewtopControl.windows)
                        if (w.id === scope.target && w.at && w.size)
                            return w;
                    return null;
                }
                // Falls back to the whole screen when the compositor reported
                // no geometry. A sheet with no visible subject is confusing;
                // one that dims everything is merely less precise, and the
                // buttons still work.
                x: scrim.target ? scrim.target.at.x : 0
                y: scrim.target ? scrim.target.at.y : 0
                width: scrim.target ? scrim.target.size.width : parent.width
                height: scrim.target ? scrim.target.size.height : parent.height
                color: "#99000000"
                radius: scrim.target ? 12 : 0

                Behavior on opacity { NumberAnimation { duration: 120 } }
            }

            // The chooser for Split's other half. Sits in the bottom half —
            // where the window it is choosing for will land — so the gesture
            // reads as "put something here" rather than as a menu that happens
            // to be on screen.
            ColumnLayout {
                visible: scope.choosing
                anchors.horizontalCenter: parent.horizontalCenter
                y: parent.height * 0.58
                spacing: 14

                StyledText {
                    Layout.alignment: Qt.AlignHCenter
                    text: ViewtopControl.windows.length > 1
                        ? "fill the other half"
                        : "nothing else is open"
                    color: "#e6ffffff"
                    font.pixelSize: Appearance.font.pixelSize.normal
                }

                Repeater {
                    model: ViewtopControl.windows.filter(w => w.id !== scope.target)

                    delegate: Rectangle {
                        required property var modelData
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: 220
                        implicitHeight: 56
                        radius: 14
                        color: pick.pressed
                            ? Appearance.colors.colLayer2
                            : Appearance.colors.colLayer1
                        border.width: 1
                        border.color: Appearance.colors.colLayer1Active

                        StyledText {
                            anchors.centerIn: parent
                            elide: Text.ElideRight
                            width: parent.width - 24
                            horizontalAlignment: Text.AlignHCenter
                            // The strip carries app_id and title now, so this
                            // names the window instead of numbering it. Falls
                            // back to the id rather than to blank: a chooser
                            // row you cannot identify is worse than an ugly one.
                            text: {
                                const w = parent.modelData;
                                const name = w.title || w.app_id || ("window " + w.id);
                                return w.floating ? name + "  (float)" : name;
                            }
                            color: Appearance.colors.colOnLayer1
                            font.pixelSize: Appearance.font.pixelSize.small
                        }

                        MouseArea {
                            id: pick
                            anchors.fill: parent
                            onClicked: {
                                const pickId = parent.modelData.id;
                                // Bring it here first. The chooser offers every
                                // open window, and most of them are on another
                                // zone — placing one without moving it puts the
                                // other half of your split on a zone you are not
                                // looking at, which reads as the pick doing
                                // nothing.
                                const here = ViewtopControl.windows
                                    .find(w => w.id === scope.target)?.workspace;
                                if (here !== undefined && parent.modelData.workspace !== here)
                                    ViewtopControl.moveToZone(pickId, here);
                                ViewtopControl.place(pickId, 0,
                                    win.height / 2, win.width, win.height / 2);
                                scope.choosing = false;
                                scope.dismiss();
                            }
                        }
                    }
                }
            }

            // The row, centred on the window it acts on rather than on the
            // panel — a sheet floating mid-screen while the app it names is in
            // the top half reads as belonging to neither.
            RowLayout {
                visible: !scope.choosing
                x: scrim.x + (scrim.width - width) / 2
                y: scrim.y + scrim.height * 0.38
                spacing: 28

                SheetButton {
                    icon: "drag_pan"
                    label: "Move"
                    onTapped: {
                        // Hands the window to the finger and gets out of the
                        // way: the sheet closes, one-finger drags move the
                        // window, and `drop` ends it. A one-shot `place` here
                        // would be the compositor guessing where you wanted it.
                        ViewtopControl.grab(scope.target);
                        scope.moving = true;
                        scope.sheetOpen = false;
                    }
                }

                SheetButton {
                    icon: "aspect_ratio"
                    label: "Resize"
                    // The corner handles the tap-cycle owed. That cycle placed
                    // every step at the zone's origin, so the first tap always
                    // read as "take the whole top" — Casey, 2026-08-16.
                    // `WindowResize` owns the mode; the sheet gets out of the
                    // way exactly as it does for Move.
                    onTapped: {
                        if (resize.beginFor(scope.target))
                            scope.sheetOpen = false;
                    }
                }

                SheetButton {
                    icon: "zoom_out_map"
                    label: "Zoom"
                    // The `pose` verb, reaching a window for the first time.
                    //
                    // `pose` has existed in the wire and the compositor since
                    // 2026-08-05 — it genuinely scales content, not only moves
                    // it — and **nothing in the shell has ever called it**.
                    // `poseActiveZone` did, and TASK-60 deleted that wrapper as
                    // the shell's half of the two-writer bug, which took the
                    // only caller with it. Casey, 2026-08-16: *"I can move
                    // windows but the zooming them in and out didn't work
                    // yet."* It did not work because nobody asked.
                    //
                    // Distinct from Resize, and the difference is load-bearing:
                    // Resize `place`s a different *rectangle* and the client
                    // relays out inside it. Zoom composes the pixels it already
                    // drew — the client is never told, its layout does not
                    // move, and the compositor maps input back through the
                    // inverse so a shrunken window is still touchable where it
                    // is drawn.
                    //
                    // Identity is a step in the cycle rather than a separate
                    // Reset: a pose the user cannot get out of with the button
                    // they got into it with is a trap.
                    // Sprung, not stepped. One `pose` call changed the window's
                    // size between two frames — Casey, 2026-08-16: *"zoom is
                    // still snapping"*. `PoseRamp` carries it on Android's own
                    // scale spring; the compositor still only honours what it
                    // is told, once per painted frame.
                    onTapped: {
                        scope.poseStep = (scope.poseStep + 1) % 3;
                        poseRamp.springTo([1.0, 0.85, 0.7][scope.poseStep], 0);
                    }
                }

                SheetButton {
                    icon: scope.targetFloating ? "picture_in_picture_off"
                        : "picture_in_picture"
                    label: scope.targetFloating ? "Unfloat" : "Float"
                    // A float leaves the zone's confinement — it stops being
                    // the zone's business, so the strip no longer counts it
                    // when deciding whether a zone still holds anything.
                    //
                    // Which is exactly why the label has to flip: a float is
                    // the one window you can no longer find by remembering
                    // which zone you left it on, so the way back out must be
                    // the same button, in the same place, saying so.
                    onTapped: {
                        if (scope.targetFloating)
                            ViewtopControl.unfloat(scope.target);
                        else
                            ViewtopControl.float(scope.target);
                        ViewtopControl.refreshWindows();
                    }
                }

                SheetButton {
                    icon: "splitscreen"
                    label: "Split"
                    // Pin this window to the top half, then offer the other
                    // half to something already open. Casey's shape: "the
                    // second waiting one possibly having a button to add an
                    // already opened one from the multitasking section".
                    //
                    // The list is queried, not assumed — a chooser offering a
                    // window that has since closed would `place` a dead id and
                    // refuse, which reads as the button being broken.
                    onTapped: {
                        ViewtopControl.place(scope.target, 0, 0, win.width, win.height / 2);
                        ViewtopControl.refreshWindows();
                        scope.choosing = true;
                    }
                }

                SheetButton {
                    icon: "close"
                    label: "Close"
                    holdLabel: "hold to force"
                    holdMs: scope.killHoldMs
                    onTapped: ViewtopControl.close(scope.target)
                    onHeld: ViewtopControl.kill(scope.target)
                }
            }

            StyledText {
                id: refusal
                anchors.horizontalCenter: parent.horizontalCenter
                y: parent.height * 0.38 + 150
                width: parent.width * 0.8
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                color: "#e6ffffff"
                font.pixelSize: Appearance.font.pixelSize.small
                opacity: 0
                text: ""

                Behavior on opacity { NumberAnimation { duration: 150 } }

                Timer {
                    id: refusalFade
                    interval: 2600
                    onTriggered: refusal.opacity = 0
                }
            }
        }
    }
}
