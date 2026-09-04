// Multitasking: every zone that has something on it, as one card each.
//
// TASK-60. This replaces a flat list of window cards, which was the wrong
// model and was told so on device — Casey, 2026-08-05: *"it's supposed to be
// showing me all sorta backgrounded zones; including split zones. You've seen
// macOS. That whole multitasking view is a function of it's own."*
//
// ## Why a zone is the card, and a window is not
//
// viewtop's canvas is a strip of zones that are minted when a window needs
// somewhere to be and destroyed when the last one leaves (`workspace.rs`). A
// zone holding two tiled windows is **one place you can go**, not two things
// you can pick between — and a flat window list said the opposite: it drew the
// halves of a split as two unrelated cards, and nothing on screen said they
// shared a destination. macOS's Mission Control is two levels for this reason,
// spaces above and windows within; on a phone there is room for one level at a
// time, so the zone wins and its windows are composited inside it.
//
// Home (`HOME_ZONE`) is always present and always empty. It is drawn as a card
// anyway, because "go back to square one" is the one destination that must be
// reachable from here even when nothing is open.
//
// ## Pictures, except the one you are looking at
//
// TASK-60 Q2 asked whether cards are live pictures or real posed windows, and
// the answer is *both, in their own half of the motion*. Casey, 2026-08-05:
// *"real windows is nicer… or if the multitasking grid pictures all but the
// 'viewed/observed' one and as we scroll through the one we rest on can be
// live."*
//
// So: the **transition** is real windows. The rail shrinks them through `pose`
// as the thumb climbs (`SystemGestureRail`), which is the part that has to be
// the actual app because it is the app you are still holding.
//
// The **destination** is this, and here only the card you have come to rest on
// runs a live capture. Every other card holds a single frame. A live
// `ScreencopyView` is a render of that window every frame; N of them is N
// windows redrawing to fill a strip where you can only look at one. Casey's own
// framing — *"why we're drawing ones not in 'frame'"* — is also why
// `cacheBuffer` is 0: a card off the side of the screen is not drawn at all,
// not drawn cheaply.
//
// ## Where the facts come from
//
// The compositor, pushed (`ViewtopControl.subscribed`) or asked synchronously
// when this opens — never from the poll that made the first version wrong. A
// view that opens stale is wrong at exactly the moment it is used, since the
// reason to open it is that something just changed.
//
// Cards are joined to their pictures by `app_id` + `title`, which the
// compositor reports beside each window. That is a bridge, not an identity:
// quickshell sees foreign-toplevel handles and viewtop sees `SurfaceId`, and
// the two namespaces have never met. Two terminals with the same title
// collide. The sound fix is a shared id in the protocol; this is enough to
// group cards and is deliberately written down as approximate rather than
// presented as correct.
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root

    signal activated(int zone)
    // Carries the compositor's own id, not a toplevel handle. Closing is a
    // scene verb like every other one the hand can reach (TASK-55), so it goes
    // out the one door in `ViewtopControl` — a second path would be a second
    // thing that can refuse differently, log differently, and be missed by the
    // trail when the agent is the one closing the window.
    signal closed(int id)

    // One entry per zone that exists, each carrying the windows on it.
    // Rebuilt from `ViewtopControl.windows`, which is the compositor's answer
    // rather than the shell's own bookkeeping — there is one idea of what is
    // where, and it is not this file's.
    readonly property var zones: {
        const byZone = {};
        const count = Math.max(1, ViewtopControl.zoneCount);
        for (let z = 0; z < count; z++)
            byZone[z] = { zone: z, windows: [], focused: false };
        for (const w of ViewtopControl.windows) {
            if (byZone[w.workspace] === undefined)
                byZone[w.workspace] = { zone: w.workspace, windows: [], focused: false };
            byZone[w.workspace].windows.push(w);
            if (w.focused)
                byZone[w.workspace].focused = true;
        }
        const out = [];
        for (const k in byZone)
            out.push(byZone[k]);
        out.sort((a, b) => a.zone - b.zone);
        return out;
    }

    // The live toplevel whose app_id+title matches a reported window, so a
    // card can show a picture of it. Null when nothing matches, which draws a
    // placeholder rather than an empty rectangle — an overview that renders
    // nothing and one that is broken must not look the same.
    //
    // A window the compositor has not named at all cannot be joined, and must
    // not be: with both fields absent this would match the first toplevel that
    // also reports neither, and put one app's picture on every card. That is
    // the state on a compositor older than the commit that added them, which is
    // exactly when a wrong picture would be least explicable.
    function toplevelFor(w) {
        if (!w || (!w.app_id && !w.title))
            return null;
        const all = ToplevelManager.toplevels?.values ?? [];
        for (const t of all) {
            if ((t.appId || "") === (w.app_id || "") && (t.title || "") === (w.title || ""))
                return t;
        }
        return null;
    }

    // One progress drives the whole surface — the reference shell's trick, and
    // Phosh's: state gates visibility, visibility never sets state.
    //
    // While the rail is still pulling, this *is* the rail's progress, so the
    // cards grow at exactly the rate the real windows behind them are
    // shrinking. Once the gesture commits the state flag takes over and the
    // animation carries it the rest of the way. TASK-60's acceptance is the
    // reason: *"no frame where a window is scaled by one and laid out by the
    // other."*
    readonly property bool open: GlobalStates.overviewOpen || GlobalStates.missionControlOpen
    property real progress: root.open ? 1 : ZoneTransition.presence
    Behavior on progress {
        // Only the settle is animated. Following the thumb is not an animation
        // and must not be smoothed, or the cards lag the fingers that are
        // moving them.
        enabled: root.open || ZoneTransition.travel === 0
        // `elementMoveEnter` rather than a hand-typed OutCubic. The house
        // curve bank is Material 3 Expressive and every one of its spatial
        // curves overshoots — the shell has carried it since ii and only
        // `SubconsciousTicker` and `AgentMessage` had ever called it, so every
        // navigation surface animated on a flat cubic. That is the missing
        // whoosh, and it was a two-line import away the whole time.
        animation: Appearance.animation.elementMoveEnter.numberAnimation.createObject(this)
    }

    // Ask the moment this becomes visible. Cheap, and the whole answer to why
    // the previous version showed a second app as absent. Redundant when the
    // compositor is pushing, and kept anyway: it costs one request and it is
    // what makes the surface correct on a phone whose compositor predates the
    // push channel.
    onOpenChanged: {
        if (root.open) {
            ViewtopControl.refreshWindows();
            list.seat();
        }
    }

    // A peek draws these cards without ever setting `open`, so it owes the same
    // ask — otherwise the preview is the 2 s poll's idea of what is running.
    Connections {
        target: GlobalStates
        function onMissionPeekChanged() {
            if (GlobalStates.missionPeek)
                ViewtopControl.refreshWindows();
        }
    }

    implicitWidth: parent ? parent.width : 540
    implicitHeight: parent ? parent.height : 800

    // --- The line she speaks on ----------------------------------------------
    //
    // Deliberately **not** model-dependent. It carries a real, locally computed
    // fact about what is open, and it is a place a model can later write to
    // rather than a place that only exists once one can. Casey, 2026-08-16:
    // *"It should sorta initially be less than model dependant but we'll build
    // out that whole system."*
    //
    // Doctrine §13 is why the band is here at all: multitasking is the one
    // surface that is purely about operation — what is running, where it lives —
    // and operation is hers. A verb she cannot reach is a defect; a screen
    // about her half of the device with nowhere for her to speak is the same
    // defect wearing a different face.
    property string spoken: ""

    readonly property string note: {
        if (root.spoken.length > 0)
            return root.spoken;
        const zones = Math.max(1, ViewtopControl.zoneCount);
        const wins = ViewtopControl.windows.length;
        const floats = ViewtopControl.windows.filter(w => w.floating === true).length;
        const parts = [qsTr("%n zone(s)", "", zones), qsTr("%n window(s)", "", wins)];
        if (floats > 0)
            parts.push(qsTr("%n floating", "", floats));
        return parts.join(" · ");
    }

    // `qs -c souveraine ipc call overviewNote say "…"`. One verb, so the thing
    // that writes here is a caller with a name rather than a global somebody
    // sets from four places.
    IpcHandler {
        target: "overviewNote"

        function say(text: string): void {
            root.spoken = text;
        }

        function clear(): void {
            root.spoken = "";
        }

        function state(): string {
            return JSON.stringify({
                spoken: root.spoken,
                shown: root.note,
                subject: root.subject ? root.subject.id : null,
                restingZone: root.restingZone ? root.restingZone.zone : null
            });
        }
    }

    // The zone the strip has come to rest on, and the window inside it a verb
    // would act on. Focused first, else the first one there — a split has two
    // and the tray has to name one of them without asking.
    readonly property var restingZone: root.zones.length === 0 ? null
        : root.zones[Math.max(0, Math.min(root.zones.length - 1, list.currentIndex))]

    readonly property var subject: {
        const z = root.restingZone;
        if (!z || z.windows.length === 0)
            return null;
        for (const w of z.windows)
            if (w.focused)
                return w;
        return z.windows[0];
    }

    ListView {
        id: list
        anchors.fill: parent
        anchors.margins: 12
        anchors.bottomMargin: 12 + ZoneTransition.trayHeight
        transform: Translate { y: (1 - root.progress) * 24 }
        model: root.zones
        orientation: ListView.Horizontal
        snapMode: ListView.SnapOneItem
        highlightRangeMode: ListView.StrictlyEnforceRange
        // Centred, not pinned left. The delegate is one card wide plus its
        // gutter, so both neighbours show and the strip reads as cards you
        // scroll between rather than screens you page through. `ZoneTransition`
        // owns these numbers because it also owns where the carry lands.
        preferredHighlightBegin: ZoneTransition.highlightBegin
        preferredHighlightEnd: ZoneTransition.highlightBegin + ZoneTransition.delegateWidth
        // The gutter is inside the delegate; two sources of separation would be
        // two numbers to keep agreeing.
        spacing: 0
        clip: true
        // Nothing off the sides is built. A card that is not in frame is a
        // window capture nobody can see — see the header.
        cacheBuffer: 0

        // Open on the zone you are actually on, so the first card is where you
        // came from rather than wherever the strip happens to start.
        //
        // Re-seated on every model change, not set once at construction.
        // `root.zones` rebuilds into a **new array** whenever the compositor's
        // window list moves, a ListView resets `currentIndex` to 0 on a model
        // change, and `onOpenChanged` asks for a refresh the instant this
        // opens — so the reset was guaranteed and the strip always came to rest
        // on Home. Measured 2026-08-16: three zones, Firefox in front, and the
        // card you were shown was the empty one.
        //
        // `callLater` so the delegates exist to be seated.
        function seat(): void {
            list.currentIndex = Math.max(0,
                root.zones.findIndex(z => z.zone === ViewtopControl.activeZone));
        }
        onModelChanged: Qt.callLater(list.seat)
        Component.onCompleted: list.seat()

        delegate: Item {
            id: zoneCard
            required property var modelData
            required property int index

            width: ZoneTransition.delegateWidth
            height: list.height

            // How far this card is from the middle of the strip, in delegates.
            // 0 is the one you are resting on.
            readonly property real offCentre: {
                const centre = list.contentX + list.width / 2;
                const mine = zoneCard.x + zoneCard.width / 2;
                return Math.abs(mine - centre)
                    / Math.max(1, ZoneTransition.delegateWidth);
            }

            readonly property bool isHome: zoneCard.modelData.zone === ViewtopControl.homeZone
            readonly property bool isActive: zoneCard.modelData.zone === ViewtopControl.activeZone
            // The card the strip has come to rest on. This one, and only this
            // one, runs live captures.
            readonly property bool resting: zoneCard.index === list.currentIndex

            // Staggered entry, capped at the fifth card so a long strip does
            // not read as loading.
            readonly property real share: {
                const start = Math.min(zoneCard.index, 5) * 0.045;
                return Math.max(0, Math.min(1, (root.progress - start) / (1 - start)));
            }
            // Depth, and only where it cannot become a second writer.
            //
            // A card off to the side sits back and dims, so the strip reads as
            // cards floating at different distances rather than a filmstrip.
            // **Gated on `inFlight`:** while the compositor is carrying a real
            // window onto one of these rects, a card that also scales is
            // exactly the fault TASK-60 was written to remove, so during a
            // carry every card sits at its true rect and the depth eases in on
            // the commit frame through the Behavior below.
            readonly property real depth: ZoneTransition.inFlight
                ? 0 : Math.min(1.4, zoneCard.offCentre)
            scale: 1 - 0.07 * zoneCard.depth
            opacity: zoneCard.share * (1 - 0.4 * Math.min(1, zoneCard.depth))
            Behavior on scale {
                animation: Appearance.animation.elementMoveSmall.numberAnimation.createObject(this)
            }

            // The card arrives rather than fading in place.
            //
            // A separate transform, and deliberately with **no** Behavior of its
            // own: `share` is continuous — it is `progress` remapped — so it is
            // already smoothed by the Behavior on `progress` above. Folding it
            // into `scale` would put a 350 ms animation under a value that
            // changes every frame, which restarts the animation every frame and
            // reads as lag rather than as motion. `depth` is step-like and is
            // the one that wants a Behavior.
            //
            // Gated on `inFlight` for the same reason `depth` is: while the
            // compositor carries a real window onto this rect, a card that also
            // scales is TASK-60's two-writer fault exactly.
            transform: Scale {
                origin.x: zoneCard.width / 2
                origin.y: zoneCard.height / 2
                xScale: ZoneTransition.inFlight ? 1 : 0.90 + 0.10 * zoneCard.share
                yScale: xScale
            }

            // No `scale` tied to the *gesture*, and that absence is the point of
            // TASK-60.
            //
            // This used to read `1.0 - 0.4 * root.progress`, tuned to match the
            // curve the rail was running through `pose` on the real windows.
            // Two writers over one geometry: the numbers were made to agree by
            // hand, and at the handoff they did not — the window sat at 0.6 of
            // the glass while the card arrived near full bleed. A transform one
            // surface applies and another must remember to undo always has a
            // path that forgets, and tapping a card was that path.
            //
            // The compositor now carries the real window onto `frame`'s rect
            // (`ZoneTransition`). A card that also scaled would be the second
            // writer all over again, so the card is chrome — the frame, the
            // label, the close control, the neighbours either side — and never
            // a competing transform.

            Rectangle {
                id: frame
                // The zone, drawn small — the same rect the compositor is
                // carrying this zone's windows onto, from the same arithmetic,
                // because it *is* the same arithmetic. `ZoneTransition` owns it;
                // nothing here recomputes a card size.
                //
                // Keeping the zone's aspect is not a taste call: `carried` takes
                // its scale from the width alone and applies it uniformly, so a
                // card of any other shape would letterbox the live window inside
                // the card it is supposed to be.
                //
                // Centred in the delegate, which leaves both neighbours showing
                // through — full bleed reads as "you are looking at that app"
                // rather than "here are the places you can go".
                // The list is inset by `listMargin` inside this surface, so the
                // delegate's origin is that much in from the panel's. Both
                // offsets come off the shared rect rather than being re-derived,
                // so the card and the carry stay the same rectangle even if the
                // frame constants move.
                x: ZoneTransition.cardInsetX
                y: ZoneTransition.cardInsetY
                width: ZoneTransition.cardWidth
                height: ZoneTransition.cardHeight
                radius: 18
                // Opaque, deliberately, and the theme colour is not.
                //
                // The backdrop behind this is the wallpaper blurred — which
                // stays, it is the room these sit in — but a translucent card
                // let it through, so every card was a coloured smear of the
                // wallpaper instead of a card with something on it. Measured
                // 2026-08-16 on the Home card: the whole frame read as one
                // pink-and-blue blur. Cards float *on* the room; they are not
                // made of it.
                color: Qt.rgba(Appearance.colors.colLayer1.r,
                    Appearance.colors.colLayer1.g,
                    Appearance.colors.colLayer1.b, 1)
                // The zone you are on is named by its frame rather than by
                // moving it: a card that jumps out of the row is a card whose
                // neighbours shift under the thumb mid-swipe.
                border.width: zoneCard.isActive ? 2 : 1
                border.color: zoneCard.isActive
                    ? Appearance.colors.colOnLayer1
                    : Appearance.colors.colLayer1Active
                clip: true

                // The windows of this zone, at the compositor's own tiling
                // scaled by the card's own factor — not a layout that guesses
                // at it.
                //
                // This was a `ColumnLayout` with an 8 px margin, which was a
                // second opinion about where a window sits inside its zone: one
                // fills, two split, and a margin nudges both off. Now each pane
                // is the window's reported rect through `cardScale`, so the card
                // is a photograph of the zone rather than a reconstruction of
                // it — and the live window the compositor carries onto this same
                // rect lands exactly on its own picture.
                //
                // `at` is zone-local (measured 2026-08-07: two windows on
                // different zones both report `x: 0` with the second zone in
                // front), so this one formula is right for every card, not just
                // the one in front.
                Repeater {
                    model: zoneCard.modelData.windows

                    delegate: Item {
                        id: pane
                        required property var modelData

                        // `contentTop` off the y before scaling: the card is a
                        // picture of the usable zone, so a window that starts
                        // below the bar's reservation starts at the card's top
                        // edge. `ZoneTransition.targetFor` makes the identical
                        // subtraction — that is what puts the carried window
                        // exactly on its own picture.
                        x: (pane.modelData.at?.x ?? 0) * ZoneTransition.cardScale
                        y: ((pane.modelData.at?.y ?? 0) - ZoneTransition.contentTop)
                            * ZoneTransition.cardScale
                        width: (pane.modelData.size?.width ?? 0) * ZoneTransition.cardScale
                        height: (pane.modelData.size?.height ?? 0) * ZoneTransition.cardScale

                        readonly property var toplevel: root.toplevelFor(pane.modelData)
                        // A window that has left its zone's confinement. Said
                        // on the card because a float is precisely the window
                        // you can no longer find by remembering which zone you
                        // put it on.
                        readonly property bool floating: pane.modelData.floating === true

                        ScreencopyView {
                            id: shot
                            anchors.fill: parent
                            // Never captures behind a closed overview, and only
                            // *keeps* capturing on the card being looked at. The
                            // others hold the frame they arrived with, which is
                            // what a card off to the side is worth.
                            captureSource: root.progress > 0 ? pane.toplevel : null
                            live: root.progress > 0 && zoneCard.resting
                            visible: pane.toplevel !== null
                        }

                        // The join failed. Said out loud, because a blank card
                        // and a broken overview must not look alike.
                        StyledText {
                            anchors.centerIn: parent
                            visible: pane.toplevel === null
                            text: pane.modelData.app_id || qsTr("window")
                            opacity: 0.6
                        }

                        StyledText {
                            anchors.left: parent.left
                            anchors.bottom: parent.bottom
                            anchors.margins: 8
                            visible: pane.floating
                            text: qsTr("(float)")
                            opacity: 0.75
                        }

                        RippleButton {
                            anchors.top: parent.top
                            anchors.right: parent.right
                            anchors.margins: 8
                            implicitWidth: 34
                            implicitHeight: 34
                            buttonRadius: 17
                            onClicked: root.closed(pane.modelData.id)
                            contentItem: MaterialSymbol {
                                anchors.centerIn: parent
                                text: "close"
                                iconSize: 18
                            }
                        }
                    }
                }

                // Home, and any zone that is empty. Home is empty on purpose —
                // it is the widget space — so this is a destination, not a
                // failure.
                StyledText {
                    anchors.centerIn: parent
                    visible: zoneCard.modelData.windows.length === 0
                    text: zoneCard.isHome ? qsTr("Home") : qsTr("Empty")
                    opacity: 0.6
                    font.pixelSize: Appearance.font.pixelSize.large
                }

                // Tapping the card goes to that zone. The whole card, not just
                // a picture inside it: the destination is the zone, so the
                // target should be the thing that represents it.
                MouseArea {
                    anchors.fill: parent
                    z: -1
                    onClicked: root.activated(zoneCard.modelData.zone)
                }
            }

            // Under the card, not under the delegate: the frame is a fixed rect
            // now rather than the delegate less a bottom margin, so the label
            // follows the thing it names.
            StyledText {
                anchors.top: frame.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.margins: 6
                horizontalAlignment: Text.AlignHCenter
                elide: Text.ElideRight
                opacity: 0.85
                text: {
                    if (zoneCard.isHome)
                        return qsTr("Home");
                    const n = zoneCard.modelData.windows.length;
                    if (n === 0)
                        return qsTr("Zone %1").arg(zoneCard.modelData.zone);
                    if (n === 1)
                        return zoneCard.modelData.windows[0].title
                            || zoneCard.modelData.windows[0].app_id
                            || qsTr("Zone %1").arg(zoneCard.modelData.zone);
                    // A split names itself as one place holding two things,
                    // which is the distinction this whole surface exists for.
                    return qsTr("Split · %1 windows").arg(n);
                }
            }
        }
    }

    // --- The tray ------------------------------------------------------------
    //
    // A predicate, not a launcher. Every control here acts on the card in
    // front — Android's instinct with the screenshot button under recents, and
    // the reason it reads as a sentence rather than a toolbar. An app grid
    // would make this a second home screen; the drawer already owns that
    // (TASK-14), and a verb in two places is two places to fix it.
    //
    // Screenshot is deliberately absent for the same reason: it is already in
    // the right-hand sidebar. Casey noticed that before I did.
    Item {
        id: tray
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 12
        height: ZoneTransition.trayHeight - 12
        // The verbs carry their own staggered fade now, so this band no longer
        // multiplies one over the top of them — squaring the ramp made the row
        // arrive late and all at once, which is the opposite of the stagger.
        transform: Translate { y: (1 - root.progress) * 24 }

        Row {
            id: verbs
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            spacing: 18

            // Float is first because it is the verb this surface owes. A float
            // has left its zone's confinement, so the strip stops counting it
            // and it becomes the one window you cannot find by remembering
            // where you left it — which makes multitasking the only place it is
            // findable, and therefore the only honest place to turn it off.
            TrayVerb {
                order: 0
                symbol: root.subject && root.subject.floating === true
                    ? "picture_in_picture_off" : "picture_in_picture"
                caption: root.subject && root.subject.floating === true
                    ? qsTr("Unfloat") : qsTr("Float")
                active: root.subject !== null
                onTriggered: {
                    if (root.subject.floating === true)
                        ViewtopControl.unfloat(root.subject.id);
                    else
                        ViewtopControl.float(root.subject.id);
                    ViewtopControl.refreshWindows();
                }
            }

            // Minting a zone has never had a control anywhere in this OS,
            // because zones are born from need rather than created — and that
            // left "put this somewhere of its own" unreachable. The compositor
            // has always answered it: `Request::Workspace` reads
            // `Some(w) if w >= count => self.workspaces.push()`, so asking for
            // the zone past the end *is* the mint. One button, no new verb.
            TrayVerb {
                order: 1
                symbol: "add_to_queue"
                caption: qsTr("New zone")
                active: root.subject !== null
                    && root.restingZone.windows.length > 0
                onTriggered: {
                    ViewtopControl.moveToZone(root.subject.id,
                        Math.max(1, ViewtopControl.zoneCount));
                    ViewtopControl.refreshWindows();
                }
            }

            // Auxo's one contribution that everybody kept. `close` and not
            // `kill`: this is a request the client may refuse or prompt on, and
            // a row of cards is the wrong place to take that choice away.
            // Force lives on the sheet, behind a hold, where it is deliberate.
            TrayVerb {
                order: 2
                symbol: "clear_all"
                caption: qsTr("Close all")
                active: root.restingZone !== null
                    && root.restingZone.windows.length > 0
                onTriggered: {
                    for (const w of root.restingZone.windows)
                        ViewtopControl.close(w.id);
                }
            }
        }

        StyledText {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            // Times `progress`, which the band used to apply for it.
            opacity: root.progress * (root.spoken.length > 0 ? 0.95 : 0.55)
            text: root.note
        }
    }

    // One verb in the tray: a symbol, a caption under it, and an off state that
    // reads as unavailable rather than as broken. Local because it is the
    // tray's own idiom and nothing else wants it yet; promote it to
    // `common/widgets` the second something does.
    component TrayVerb: Item {
        id: verb
        required property string symbol
        required property string caption
        // Where in the row this one sits, so the band arrives as a sentence
        // rather than as a block. Named at the call site rather than taken from
        // a Repeater index because the three verbs are three deliberate things,
        // not a model.
        required property int order
        property bool active: true
        signal triggered

        width: 84
        height: 78

        // Each verb starts 6% of the climb after the one before it, so the row
        // lands left to right. Capped by the same `min(1, …)` shape the cards
        // use, so a fully open tray is fully opaque and not 94% of one.
        readonly property real share: {
            const start = verb.order * 0.06;
            return Math.max(0, Math.min(1, (root.progress - start) / (1 - start)));
        }

        opacity: verb.share * (verb.active ? 1 : 0.35)
        transform: Translate { y: (1 - verb.share) * 14 }

        RippleButton {
            id: hit
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            implicitWidth: 52
            implicitHeight: 52
            buttonRadius: 26
            enabled: verb.active
            onClicked: verb.triggered()
            // The bank's own bounce, which nothing outside the subconscious
            // ticker had ever used. `expressiveDefaultSpatial` overshoots to
            // 1.21, so the button comes back past its resting size and settles
            // — the difference between a control that acknowledges a thumb and
            // one that merely changes colour under it.
            scale: hit.down ? 0.90 : 1
            Behavior on scale {
                animation: Appearance.animation.clickBounce.numberAnimation.createObject(this)
            }
            contentItem: MaterialSymbol {
                anchors.centerIn: parent
                text: verb.symbol
                iconSize: 24
            }
        }

        StyledText {
            anchors.top: hit.bottom
            anchors.topMargin: 4
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            opacity: 0.8
            text: verb.caption
        }
    }
}
