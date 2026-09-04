// Pixel3Arch patch to ii's stock Overview.qml. History:
//
// 2026-07-07 — opened the search/overview with the on-screen keyboard
// (GlobalStates.oskOpen) and closed it with the keyboard too.
// 2026-08-05 — finesse pass (TASK-14): the keyboard no longer auto-raises
// with the drawer; it belongs to the search pane and is summoned by tapping
// it (see oskSummoner). The drawer gained a translucent theme sheet
// (panelBg), the dock is suppressed while it is open, and both bodies fill
// 0.78 of the panel height instead of 0.7.
//
// While the OSK is open, this panel does NOT register with
// GlobalFocusGrab.addDismissable — see the onOskOpenChanged handler below.
// Reason: GlobalFocusGrab's HyprlandFocusGrab (hyprland_focus_grab_v1, a
// real Wayland protocol) clears whenever a tap lands outside its
// whitelisted surfaces, and wvkbd (the real OSK — see OnScreenKeyboard.qml)
// is a separate process that CANNOT be added to that whitelist — Quickshell
// only exposes whitelisting its own in-process windows, and there's no
// hyprctl or other lever either. Two attempts at a "shield window" to fake
// wvkbd's inclusion both failed on real device testing (see
// OnScreenKeyboard.qml's history for what was tried and why). So instead of
// fighting the grab, this panel just stops being dismissable-by-outside-tap
// while the keyboard is up — it still closes normally via Escape, the
// explicit toggle, or picking a search result. Everything else in this file
// is unchanged stock ii — diff against upstream before re-applying this
// patch if ii updates.
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions as CF
import Qt.labs.synchronizer
import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland
import qs.modules.souveraine.navigation

Scope {
    id: overviewScope
    property bool dontAutoCancelSearch: false
    // Whether panelWindow is currently registered with GlobalFocusGrab so an
    // outside tap dismisses the drawer. Gated on the OSK (see onOskOpenChanged)
    // and deliberately tracked rather than add/remove blind: adding the same
    // window twice is the double-add race the registration comment below warns
    // about, and re-opening the drawer while already registered would re-add.
    property bool panelDismissableRegistered: false

    // Mission control is on the glass — committed, or previewed under a dwelling
    // thumb. Presentation gates read this; focus, dismissal and every commit
    // path keep reading `missionControlOpen`, so a peek draws and decides
    // nothing.
    readonly property bool missionShown: GlobalStates.missionControlOpen || GlobalStates.missionPeek

    PanelWindow {
        id: panelWindow
        property string searchingText: ""
        readonly property HyprlandMonitor monitor: Hyprland.monitorFor(panelWindow.screen)
        property bool monitorIsFocused: (Hyprland.focusedMonitor?.id == monitor?.id)
        // Two ways in, one surface. The pill's second-stage swipe has set
        // `missionControlOpen` since 2026-07, and TASK-14 records that NO
        // SURFACE CONSUMES IT — the Auxo-like card surface it was meant to
        // raise was never built. WindowOverview is that surface, so the state
        // finally reaches something instead of being set and dropped.
        //
        // Mission control is the cards alone: no search, because it is a
        // "switch to what is running" gesture, not a "find something" one.
        visible: GlobalStates.overviewOpen || overviewScope.missionShown

        WlrLayershell.namespace: "quickshell:overview"
        // Overlay, not Top: the second-stage edge swipe must bring the
        // overview up over a fullscreen app (fullscreen renders above Top).
        WlrLayershell.layer: WlrLayer.Overlay
        // Only the search half wants the keyboard. Mission control taking it
        // would summon the OSK over the cards for a surface with no text field.
        WlrLayershell.keyboardFocus: GlobalStates.overviewOpen ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None
        color: "transparent"

        mask: Region {
            // The window only exists where the drawer does: panelBg (which
            // wraps the column with a breathing margin) is the drawn surface
            // AND the input region. This replaced `columnLayout` when the
            // panel sheet was added — the mask must cover what panelBg
            // covers, or the sheet renders clipped and its margin is dead
            // input space.
            //
            // Mission control covers the whole glass (the blurred home
            // backdrop), so there it is the whole window or the cards take no
            // taps at all.
            // A peek maps this surface while the thumb is still down on the
            // rail. Masking to a zero-size item makes it take no input at all,
            // so the in-flight gesture stays the rail's and cannot be stolen by
            // a full-glass region appearing mid-drag.
            item: GlobalStates.missionControlOpen ? missionBackdrop
                : GlobalStates.missionPeek ? noInput : panelBg
        }

        Item {
            id: noInput
            width: 0
            height: 0
        }

        anchors {
            top: true
            bottom: true
            left: true
            right: true
        }

        Connections {
            target: GlobalStates
            function onOverviewOpenChanged() {
                if (!GlobalStates.overviewOpen) {
                    searchWidget.disableExpandAnimation();
                    overviewScope.dontAutoCancelSearch = false;
                    GlobalFocusGrab.dismiss();
                    GlobalStates.oskOpen = false;
                    // Undo the drawer-time suppression (see below) so the dock
                    // returns to its pre-drawer posture: its own swipe-down
                    // state, or the empty-desktop reveal it had coming.
                    GlobalStates.dockSuppressed = false;
                } else {
                    if (!overviewScope.dontAutoCancelSearch) {
                        searchWidget.cancelSearch();
                    }
                    // Keyboard-less on open — 2026-08-05: the OSK no longer
                    // auto-raises with the drawer. The keyboard belongs to the
                    // search pane: tapping it summons the OSK (see oskSummoner
                    // below), tapping anything else doesn't. The field still
                    // auto-focuses so the first tap has somewhere to go.
                    GlobalStates.oskOpen = false;
                    // No OSK: the drawer is dismissable by outside tap, and it
                    // must register right here — onOskOpenChanged only fires on
                    // a keyboard toggle, which no longer happens on open.
                    if (!overviewScope.panelDismissableRegistered) {
                        GlobalFocusGrab.addDismissable(panelWindow);
                        overviewScope.panelDismissableRegistered = true;
                    }
                    // The drawer takes the space, so the dock goes down while
                    // it is open — and suppression, not dockRevealed=false:
                    // the dock is pinned on the phone, and a pinned dock
                    // ignores dockRevealed entirely (Dock.qml computeDockState:
                    // only dockSuppressed beats effectivePinned). The overview
                    // button lives on the dock, which is why the drawer needs
                    // other doors in (pill swipe home / IPC). The close branch
                    // above restores the flag.
                    GlobalStates.dockSuppressed = true;
                }
            }
        }

        // Registering as dismissable is gated on the OSK's state, not
        // just overviewOpen — see the file-header comment for why. The
        // drawer registers itself in onOverviewOpenChanged (the OSK no
        // longer flips on open); this handler only re-tracks when the OSK
        // is toggled independently while the drawer stays open.
        // One surface at a time. Home and the running-work view are opened by
        // different gestures, but they are not modes to be stacked: a swipe to
        // mission control while the drawer is up should replace it, not float
        // the cards over the grid. Neither flag's writers enforce this, so the
        // body's owner does.
        Connections {
            target: GlobalStates
            function onOverviewOpenChanged() {
                if (GlobalStates.overviewOpen)
                    GlobalStates.missionControlOpen = false;
            }
            function onMissionControlOpenChanged() {
                if (GlobalStates.missionControlOpen)
                    GlobalStates.overviewOpen = false;
            }
        }

        Connections {
            target: GlobalStates
            function onOskOpenChanged() {
                if (!GlobalStates.overviewOpen) return;
                if (GlobalStates.oskOpen) {
                    GlobalFocusGrab.removeDismissable(panelWindow);
                    overviewScope.panelDismissableRegistered = false;
                } else if (!overviewScope.panelDismissableRegistered) {
                    GlobalFocusGrab.addDismissable(panelWindow);
                    overviewScope.panelDismissableRegistered = true;
                }
            }
        }

        Connections {
            target: GlobalFocusGrab
            function onDismissed() {
                GlobalStates.overviewOpen = false;
            }
        }
        implicitWidth: columnLayout.implicitWidth
        implicitHeight: columnLayout.implicitHeight

        function setSearchingText(text) {
            searchWidget.setSearchingText(text);
            searchWidget.focusFirstItem();
        }

        // Tap-to-dismiss for the dead space INSIDE the overview page: the
        // window's input mask only covers the panel sheet (panelBg), and the
        // grid's gaps (between tiles, around the grid) consume nothing —
        // taps there used to be silently ignored, which read as the page
        // being stuck. Declared before (= stacked below) the column, sized to
        // the overview grid only, so the search bar's padding stays inert and
        // every real control (workspaces, windows, search) wins the tap.
        // While the OSK is up this is the ONLY outside-tap dismiss — the
        // focus-grab route is deliberately disabled then (see header).
        MouseArea {
            enabled: (GlobalStates.overviewOpen || GlobalStates.missionControlOpen)
                && overviewLoader.active
            x: columnLayout.x + overviewLoader.x
            y: columnLayout.y + overviewLoader.y
            width: overviewLoader.width
            height: overviewLoader.height
            // Closes whichever one is up. Mission control has no focus grab to
            // fall back on — it takes no keyboard focus — so this is its only
            // outside-tap dismissal, and leaving it out stranded the surface
            // with no way back but the rail.
            onClicked: {
                GlobalStates.overviewOpen = false;
                GlobalStates.missionControlOpen = false;
            }
        }

        // The drawer's sheet. A solid, slightly translucent layer of the
        // theme surface behind the search bar and grid — the "blur/look"
        // finesse item. Deliberately NOT a real GaussianBlur: that needs
        // Qt5Compat and dies on the phone's GLES-ish backend (2026-08-05),
        // so this is the phone-safe substitute — an opaque-enough sheet that
        // the wallpaper reads as a soft base behind it. Declared before the
        // column (stacked below it) so nothing here can eat taps; it is
        // input-transparent anyway.
        // Multitasking sits on the home zone, blurred — not on the app you came
        // from, and not on nothing. MultiEffect (Qt6), not GaussianBlur: that
        // one needs Qt5Compat and dies on the phone's GLES path (2026-08-05).
        // The sheet under it is the known-good look if the effect no-ops.
        Item {
            id: missionBackdrop
            visible: overviewScope.missionShown
            anchors.fill: parent
            z: -1

            // The backdrop arrives with the climb rather than at the end of it.
            //
            // This was a hard on/off gated on a 140 ms dwell, so an ordinary
            // swipe shrank the real window over the live app and then the whole
            // destination — blur, sheet, cards — appeared in one frame. That
            // discontinuity is the "swipe to this swap is awkward" Casey has
            // named repeatedly, and it is TASK-60's own acceptance: *"the
            // scale-on-drag either continues into the view or is gone. Not
            // both."*
            //
            // Same curve as the cards, from the same owner, so at the commit
            // frame the backdrop is already opaque and the card's picture is
            // already exactly under the real window — releasing the carry
            // changes nothing on the glass. A fast flick to Home shows a faint
            // wash rather than a hard flash, because `presence` recedes past
            // the multitasking detent instead of latching on.
            opacity: GlobalStates.missionControlOpen ? 1 : ZoneTransition.presence
            Behavior on opacity {
                enabled: GlobalStates.missionControlOpen || ZoneTransition.travel === 0
                NumberAnimation {
                    duration: 260
                    easing.type: Easing.OutCubic
                }
            }

            Image {
                id: homeWall
                anchors.fill: parent
                source: Config.options.background.wallpaperPath ?? ""
                fillMode: Image.PreserveAspectCrop
                cache: true
                asynchronous: true
                visible: false
            }

            MultiEffect {
                anchors.fill: parent
                source: homeWall
                visible: homeWall.status === Image.Ready
                blurEnabled: true
                blurMax: 64
                blur: 1
                saturation: -0.2
                brightness: -0.25
            }

            Rectangle {
                anchors.fill: parent
                color: CF.ColorUtils.transparentize(Appearance?.colors?.colLayer0 ?? "#101010", 0.25)
            }

            // The backdrop is the input region while mission control is up, so
            // it owes the way out. Without this a tap outside a card lands on
            // the overlay and does nothing, which is a screen you cannot leave.
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    GlobalStates.missionControlOpen = false;
                    GlobalStates.overviewOpen = false;
                }
            }
        }

        Rectangle {
            id: panelBg
            visible: columnLayout.visible && !overviewScope.missionShown
            anchors.fill: columnLayout
            // The sheet breathes past the column so the drawer reads as a
            // panel, not as widgets floating on the wallpaper. The mask
            // follows this same box (see `mask` above).
            anchors.leftMargin: -18
            anchors.rightMargin: -18
            anchors.topMargin: -6
            anchors.bottomMargin: -14
            radius: Appearance?.rounding?.large ?? 20
            // colLayer1 at ~88% — opaque enough that wallpaper detail behind
            // the grid is noise, translucent enough to still be "surface".
            color: CF.ColorUtils.transparentize(Appearance?.colors?.colLayer1 ?? "#1a1a1a", 0.12)
            border.width: 1
            border.color: CF.ColorUtils.transparentize(Appearance?.colors?.colOnLayer1 ?? "#ffffff", 0.88)
        }

        Column {
            id: columnLayout
            // Both ways in. The panel, the mask and the loader were all moved
            // to `overviewOpen || missionControlOpen` when mission control
            // landed and this was left behind, so the second-stage pill swipe
            // raised a panel whose entire contents were invisible — the cards
            // were built, instantiated, and never shown.
            visible: GlobalStates.overviewOpen || overviewScope.missionShown
            anchors {
                horizontalCenter: parent.horizontalCenter
                top: parent.top
            }
            spacing: -8

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Escape) {
                    GlobalStates.overviewOpen = false;
                }
            }

            SearchWidget {
                id: searchWidget
                // Search belongs to the overview, not to mission control:
                // "switch to what is running" has nothing to type into, and
                // the panel deliberately takes no keyboard focus in that mode
                // (see `keyboardFocus` above), so a field here would be one
                // you could see and not use.
                visible: GlobalStates.overviewOpen
                anchors.horizontalCenter: parent.horizontalCenter
                Synchronizer on searchingText {
                    property alias source: panelWindow.searchingText
                }
            }

            Loader {
                id: overviewLoader
                anchors.horizontalCenter: parent.horizontalCenter
                active: (GlobalStates.overviewOpen || overviewScope.missionShown)
                    && (Config?.options.overview.enable ?? true)
                // Two ways in, two bodies — TASK-14's split. The overview
                // (Home) is the app drawer: search above (this file's
                // SearchWidget) and AppGrid below. Mission control, raised by
                // the pill's second-stage swipe, is the running-work view:
                // WindowOverview's cards alone, deliberately no search and no
                // keyboard (see `keyboardFocus` above). Previously this one
                // body rendered the running cards for BOTH flags, which left
                // Home showing what is running instead of what can run.
                sourceComponent: overviewScope.missionShown
                    ? windowOverviewComponent : appGridComponent
            }

            Component {
                id: windowOverviewComponent
                // WindowOverview, not OverviewWidget. The latter draws a grid
                // of workspaces from HyprlandData; viewtop has neither, so it
                // rendered an empty frame and read as the overview being
                // broken. See WindowOverview.qml's header.
                // ZoneOverview, not WindowOverview (TASK-60). The flat list
                // of window cards drew the two halves of a split as unrelated
                // things, and could not show a zone you were not on. A zone is
                // the destination; its windows live inside its card.
                ZoneOverview {
                    id: zoneOverview

                    // Tell the owner where this surface actually is, rather than
                    // letting it assume the screen. The panel respects exclusive
                    // zones, so the bar's 40 px makes it 1040 tall on a 1080
                    // screen and puts its origin 40 px down the output — and the
                    // compositor's carry targets are in output coordinates.
                    //
                    // The inset is what was lost to reservations, which are
                    // top-anchored for this surface. It should equal the `at.y`
                    // the compositor reports for any tiled window; if a future
                    // surface reserves along the bottom this becomes wrong, and
                    // the cross-check is how it would be caught.
                    function report() {
                        ZoneTransition.measuredAt(0,
                            (panelWindow.screen?.height ?? panelWindow.height) - panelWindow.height,
                            width, height);
                    }
                    onWidthChanged: zoneOverview.report()
                    onHeightChanged: zoneOverview.report()
                    Component.onCompleted: zoneOverview.report()

                    // The panel's width, NOT the column's.
                    //
                    // `overviewLoader.parent` is the Column, and a Column is
                    // as wide as its widest child. In the drawer the search
                    // widget supplies that width; mission control is the cards
                    // alone and has no search — so the only child left was
                    // this loader, whose width came from the column, whose
                    // width came from this loader. The cycle resolves to zero:
                    // measured `column.w: 0, item.w: 0` with `h: 842`.
                    //
                    // Full-height, zero-width cards draw nothing, so mission
                    // control was the blurred backdrop and no cards — while
                    // every other signal read healthy (zones 2, windows 1,
                    // subscribed true, loader active, item present, opacity 1,
                    // progress 1). Nothing was broken except one number.
                    //
                    // Full width is also what this surface wants: mission
                    // control covers the whole glass — its own mask is
                    // `missionBackdrop`, not the drawer's sheet.
                    width: panelWindow.width
                    height: panelWindow.height * 0.78
                    visible: (panelWindow.searchingText == "")
                    onActivated: zone => {
                        // The compositor moves the canvas; the shell only asks.
                        // Going to a zone is not "activate a toplevel" — that
                        // was the old model's verb and it could not express
                        // "this place, which happens to hold two windows".
                        //
                        // Through the end-target table, which is what makes the
                        // strand repro pass (TASK-60). This used to move the
                        // canvas and clear two flags by hand — three steps, none
                        // of which released a transform, and this tap never
                        // touches the rail where every release-the-pose path had
                        // been built. So btop came back as a small card in the
                        // middle of the screen. A tapped card is now the same
                        // machinery as a released pill: one destination, and
                        // arriving at it is what lets go.
                        ZoneTransition.commit(ZoneTransition.zoneTarget(zone));
                    }
                    onClosed: id => ViewtopControl.close(id)
                }
            }

            Component {
                id: appGridComponent
                // The phone app drawer. TASK-14: a paged grid from the
                // desktop-entry list, alphabetical; search stays exactly
                // as-is on top. See AppGrid.qml's header.
                AppGrid {
                    width: overviewLoader.parent.width
                    height: panelWindow.height * 0.78
                    visible: (panelWindow.searchingText == "")
                }
            }
        }
        // The search pane is now the keyboard's door (2026-08-05): the OSK
        // no longer auto-raises with the drawer, it raises when this pane is
        // TAPPED. This overlay sits above the search bar but outside its
        // widget tree, so it can add behavior without forking SearchWidget:
        // it re-focuses the field first (a tap on the grid may have left it)
        // and only then opens the keyboard — the OSK's input redirection
        // returns keys to whatever had focus before it opened, so focus must
        // land BEFORE the keyboard does. The press propagates on afterwards,
        // so the field still gets its normal tap.
        MouseArea {
            id: oskSummoner
            enabled: GlobalStates.overviewOpen && !GlobalStates.oskOpen
            visible: columnLayout.visible
            // searchWidget is inside the column, not a sibling, so anchors
            // cannot bind it — same idiom as the tap-to-dismiss MouseArea:
            // geometry in columnLayout-local terms, offset by the column's
            // own position within the panel.
            x: columnLayout.x + searchWidget.x
            y: columnLayout.y + searchWidget.y
            width: searchWidget.width
            height: searchWidget.height
            acceptedButtons: Qt.LeftButton
            propagateComposedEvents: true

            // A tap, not a press.
            //
            // Summoning on `onPressed` meant any gesture that merely *began*
            // over the search field raised the keyboard — including the swipe
            // that scrolls the app grid, which starts at the top of the drawer
            // more often than not. Casey, 2026-08-07: "keyboard still opens
            // when I swipe on the app drawer."
            //
            // So the press only remembers where it landed and the release
            // decides. The slop is deliberately generous: a thumb reaching the
            // top of a 540px panel is not steady, and a few pixels of drift
            // while tapping a text field is a tap.
            property real pressX: 0
            property real pressY: 0
            readonly property real tapSlop: 12

            onPressed: (mouse) => {
                oskSummoner.pressX = mouse.x;
                oskSummoner.pressY = mouse.y;
                mouse.accepted = false;
            }

            onReleased: (mouse) => {
                const moved = Math.hypot(mouse.x - oskSummoner.pressX,
                                         mouse.y - oskSummoner.pressY);
                if (moved <= oskSummoner.tapSlop && !GlobalStates.oskOpen) {
                    searchWidget.focusSearchInput();
                    GlobalStates.oskOpen = true;
                }
                mouse.accepted = false;
            }
        }
    }

    function toggleClipboard() {
        if (GlobalStates.overviewOpen && overviewScope.dontAutoCancelSearch) {
            GlobalStates.overviewOpen = false;
            return;
        }
        overviewScope.dontAutoCancelSearch = true;
        panelWindow.setSearchingText(Config.options.search.prefix.clipboard);
        GlobalStates.overviewOpen = true;
    }

    function toggleEmojis() {
        if (GlobalStates.overviewOpen && overviewScope.dontAutoCancelSearch) {
            GlobalStates.overviewOpen = false;
            return;
        }
        overviewScope.dontAutoCancelSearch = true;
        panelWindow.setSearchingText(Config.options.search.prefix.emojis);
        GlobalStates.overviewOpen = true;
    }

    // sessiond calls `ipc call overview toggle` for `Action::Overview` — the
    // three-finger-tap binding (`device_state.rs`) and anything else that
    // reaches for the overview by name. Nothing answered to `overview`: the
    // only target here was `search`, so the executor shelled out to a handler
    // that did not exist and the tap did nothing, silently. Two things were
    // hiding that — the shipped sessiond has no `gesture` verb at all and
    // refuses the op before it gets this far, so the second half of the break
    // could not be seen from the device.
    //
    // This makes the existing binding reach the surface it already names. It
    // does not decide what the tap *should* raise; that is TASK-55's Q1, and
    // the answer there may retarget this without changing it.
    IpcHandler {
        target: "overview"

        function toggle(): void {
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
        function open(): void {
            GlobalStates.overviewOpen = true;
        }
        function close(): void {
            GlobalStates.overviewOpen = false;
        }

        // Multitasking. Raised only by the pill's first-stage swipe until now,
        // which meant the running-work view was the one surface on the device
        // that neither an agent nor a test could reach — and it is the one
        // that has been reported broken.
        function missionControl(): void {
            GlobalStates.missionControlOpen = !GlobalStates.missionControlOpen;
        }

        // The multitasking gesture, without a finger.
        //
        // TASK-60's acceptance: *"The gesture is reachable as a verb. An agent
        // can drive `shift` and pick an end target well enough to demo
        // multitasking."* Casey, 2026-08-07 — there is a point where she is
        // asked for a tour of the phone, *"and that does mean even these little
        // gesture steps will be possible."*
        //
        // Three steps rather than one opaque call, because a tour narrates: she
        // can hold the windows half-carried while she says what multitasking
        // is, and then land them. `swipe` is the whole motion for when the
        // narration is not the point.
        //
        // No contact is synthesized, so the compositor never sees input at all
        // and the run cannot raise `observed_confidence` or feed the idle
        // budget — the machine must not believe a human is present because she
        // moved a window. Step-up stays human-only by the same absence:
        // `input.rs:432` routes `Origin::Agent` to `Route::Withheld`, and this
        // path does not go near it.
        function carryBegin(): string {
            return ZoneTransition.begin() ? "carrying"
                : "nothing on this zone to carry";
        }

        function carryShift(shift: real): void {
            ZoneTransition.progress(shift);
        }

        // `target` is an end target by name: home, overview, last_zone, or a
        // zone number. Same four the pill commits, same table.
        function carryCommit(target: string): void {
            const n = parseInt(target, 10);
            ZoneTransition.commit(isNaN(n) ? target : ZoneTransition.zoneTarget(n));
        }

        // Begin, travel, and land — the whole gesture in one call.
        function swipe(target: string): void {
            ZoneTransition.begin();
            tour.target = target;
            tour.step = 0;
            tour.restart();
        }

        function carryState(): string {
            return JSON.stringify({
                inFlight: ZoneTransition.inFlight,
                cardScale: ZoneTransition.cardScale,
                card: {
                    x: ZoneTransition.cardX,
                    y: ZoneTransition.cardY,
                    w: ZoneTransition.cardWidth,
                    h: ZoneTransition.cardHeight
                },
                panel: { w: ZoneTransition.panelWidth, h: ZoneTransition.panelHeight },
                // What the surface reported, beside what the panel actually is.
                // The card rect is derived from these, so a card in the wrong
                // place is diagnosed by reading them rather than by theorising —
                // the same reason `state` exists at all (TASK-60: `item.w: 0`
                // beside `column.w: 0` located the zero-width card in one step).
                surface: {
                    left: ZoneTransition.surfaceLeft,
                    top: ZoneTransition.surfaceTop,
                    w: ZoneTransition.surfaceWidth,
                    h: ZoneTransition.surfaceHeight
                },
                panelWindow: { w: panelWindow.width, h: panelWindow.height },
                loaderItem: overviewLoader.item
                    ? { w: overviewLoader.item.width, h: overviewLoader.item.height }
                    : null
            });
        }

        function state(): string {
            return JSON.stringify({
                overview: GlobalStates.overviewOpen,
                missionControl: GlobalStates.missionControlOpen,
                zones: ViewtopControl.zoneCount,
                windows: ViewtopControl.windows.length,
                subscribed: ViewtopControl.subscribed,
                searchingText: panelWindow.searchingText,
                loaderActive: overviewLoader.active,
                loaderHasItem: overviewLoader.item !== null,
                activeZone: ViewtopControl.activeZone,
                item: overviewLoader.item ? {
                    w: overviewLoader.item.width,
                    h: overviewLoader.item.height,
                    opacity: overviewLoader.item.opacity,
                    visible: overviewLoader.item.visible,
                    zones: overviewLoader.item.zones ? overviewLoader.item.zones.length : -1,
                    progress: overviewLoader.item.progress ?? -1
                } : null,
                column: { w: columnLayout.width, h: columnLayout.height, visible: columnLayout.visible },
                loaderPos: { x: overviewLoader.x, y: overviewLoader.y, w: overviewLoader.width, h: overviewLoader.height }
            });
        }
    }

    // The travel half of `overview swipe`. Sixteen steps over ~320 ms is the
    // settle `ZoneOverview` already animates against, so an agent's swipe and a
    // thumb's arrive at the same speed — a demo that moved at a different rate
    // than the real gesture would be showing something the phone does not do.
    Timer {
        id: tour
        property string target: "overview"
        property int step: 0
        readonly property int steps: 16
        interval: 20
        repeat: true
        onTriggered: {
            tour.step += 1;
            if (tour.step >= tour.steps) {
                tour.stop();
                const n = parseInt(tour.target, 10);
                ZoneTransition.commit(isNaN(n) ? tour.target
                    : ZoneTransition.zoneTarget(n));
                ZoneTransition.rest();
                return;
            }
            // Drives the clock, exactly as the rail does — so the carry, the
            // backdrop and the cards all move on the demo for the same reason
            // they move under a thumb. A tour that ran its own animation would
            // be showing something the phone does not actually do.
            //
            // A detent of `steps` makes one step one unit of travel, so the
            // demo lands precisely on the multitasking detent at the last step.
            ZoneTransition.pullTo(tour.step, tour.steps);
        }
    }

    IpcHandler {
        target: "search"

        function toggle() {
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
        function workspacesToggle() {
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
        function close() {
            GlobalStates.overviewOpen = false;
        }
        function open() {
            GlobalStates.overviewOpen = true;
        }
        // Device-side acceptance/debug surface: lets deploy checks exercise
        // the exact query path the text field uses without synthesizing keys.
        function setQuery(query: string): void {
            overviewScope.dontAutoCancelSearch = true;
            GlobalStates.overviewOpen = true;
            panelWindow.setSearchingText(query);
        }
        function status(): string {
            return JSON.stringify({
                open: GlobalStates.overviewOpen,
                query: LauncherSearch.query,
                results: LauncherSearch.results.length,
                desktopEntries: DesktopEntries.applications.values.length,
                appSearchEntries: AppSearch.list.length,
                widget: searchWidget.debugState()
            });
        }
        function toggleReleaseInterrupt() {
            GlobalStates.superReleaseMightTrigger = false;
        }
        function clipboardToggle() {
            overviewScope.toggleClipboard();
        }
    }

    GlobalShortcut {
        name: "searchToggle"
        description: "Toggles search on press"

        onPressed: {
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
    }
    GlobalShortcut {
        name: "overviewWorkspacesClose"
        description: "Closes overview on press"

        onPressed: {
            GlobalStates.overviewOpen = false;
        }
    }
    GlobalShortcut {
        name: "overviewWorkspacesToggle"
        description: "Toggles overview on press"

        onPressed: {
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
    }
    GlobalShortcut {
        name: "searchToggleRelease"
        description: "Toggles search on release"

        onPressed: {
            GlobalStates.superReleaseMightTrigger = true;
        }

        onReleased: {
            if (!GlobalStates.superReleaseMightTrigger) {
                GlobalStates.superReleaseMightTrigger = true;
                return;
            }
            GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
        }
    }
    GlobalShortcut {
        name: "searchToggleReleaseInterrupt"
        description: "Interrupts possibility of search being toggled on release. " + "This is necessary because GlobalShortcut.onReleased in quickshell triggers whether or not you press something else while holding the key. " + "To make sure this works consistently, use binditn = MODKEYS, catchall in an automatically triggered submap that includes everything."

        onPressed: {
            GlobalStates.superReleaseMightTrigger = false;
        }
    }
    GlobalShortcut {
        name: "overviewClipboardToggle"
        description: "Toggle clipboard query on overview widget"

        onPressed: {
            overviewScope.toggleClipboard();
        }
    }

    GlobalShortcut {
        name: "overviewEmojiToggle"
        description: "Toggle emoji query on overview widget"

        onPressed: {
            overviewScope.toggleEmojis();
        }
    }
}
