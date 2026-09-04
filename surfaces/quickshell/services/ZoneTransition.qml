// The multitasking transition, and the one thing that owns it.
//
// TASK-60. The fault this replaces was **two writers over one geometry**: the
// rail scaled the real windows through `pose` while `ZoneOverview` scaled its
// cards through its own `scale`, both reading `zonePullProgress` and running
// opposite curves. At the handoff the window was at 0.6 of the glass and the
// card arrived near full bleed. Every symptom — the size pop, the doubled
// frame, and the strand — was that one fault.
//
// So there is one owner and it is this file. It holds:
//
// 1. **The card rect**, computed once here and read by both ends. `ZoneOverview`
//    lays its card out at `cardRect`; the carry names `cardRect` as the
//    destination. They cannot disagree, because there is nothing to keep in
//    agreement — it is the same number.
// 2. **The four verbs**, so the compositor carries the *real* window onto that
//    rect. Nothing in the shell transforms a window any more.
// 3. **The end-target table**, so arriving somewhere is one function with one
//    branch per destination rather than a set of booleans each caller sets in
//    its own order.
//
// ## Releasing is arriving
//
// quickstep's `GestureState` resolves every swipe to exactly one of
// `HOME / RECENTS / NEW_TASK / LAST_TASK / ALL_APPS`, and that is structurally
// why Android cannot strand a window and we could. The old shape was a
// transform one surface applied and a *different* surface had to remember to
// undo — so tapping a card, which never touches the rail, left a window shrunk
// with no way back (Casey's strand repro: btop, multitasking, Home,
// multitasking, tap btop, and btop is a small card in the middle of the
// screen).
//
// Here the transform is released by `commit`/`cancel` inside the compositor,
// and *every* exit is one of those two. `LastZone` is a destination with a
// name, not an abandon branch, for the same reason.
//
// ## Why the geometry is a scaled zone and not a rectangle that looks nice
//
// `Viewtop::carried` takes its scale from the width alone
// (`leg.size.width / size.width`) and applies it uniformly. A destination rect
// of some other aspect would therefore letterbox the real window inside the
// card it is supposed to *be*. So a card is the whole zone drawn small — one
// scale factor, origin included — and a window's target is its own position
// and size run through that same factor. The window lands exactly where the
// card's picture of it is, by construction rather than by tuning.
//
// ## It is a verb, because she gives the tour
//
// Casey, 2026-08-07: an agent asked for a tour of the phone means *"even these
// little gesture steps will be possible."* `shift` and the end target are both
// reachable from the compositor's verb table, and `Overview.qml`'s `swipe` IPC
// drives this whole file without a finger. An agent-driven run stays
// `Origin::Agent` throughout — the compositor never sees a contact — so it
// raises no evidence and buys no idle budget, which is the rule TASK-60 sets
// for the tour and `input.rs` already enforces for her hand.
pragma Singleton

import QtQuick
import Quickshell
import qs
import qs.services

Singleton {
    id: root

    // --- The geometry, defined once ------------------------------------------
    //
    // Logical pixels, in the panel's coordinates — which are the output's,
    // because the overview's `PanelWindow` is anchored to all four edges. The
    // compositor's `at`/`size` are in the same space, so a rect computed here
    // is directly a `TransitionTarget` with no conversion to get wrong.
    //
    // `Quickshell.screens[0]`, not a window's `screen`: this is a singleton and
    // has no window, and the phone has one output. The fallbacks are the
    // Pixel's logical size so a first frame before the screen list populates
    // lays out at the right scale rather than at zero.
    readonly property var screen: Quickshell.screens.length > 0 ? Quickshell.screens[0] : null
    readonly property real panelWidth: root.screen ? root.screen.width : 540
    readonly property real panelHeight: root.screen ? root.screen.height : 1080

    // The overview surface's *measured* geometry, reported by the surface
    // itself. Assuming it equalled the screen was wrong and wrong in the way
    // that hides: the overview `PanelWindow` respects exclusive zones, so the
    // status bar's 40 px makes it 1040 tall on a 1080 screen and puts its origin
    // 40 px down the output. Computing the card against the raw screen made it
    // about 4% too large and 40 px low — close enough to look right in a still
    // frame and wrong in exactly the way that reads as a bad hand-off in motion.
    //
    // Measured 2026-08-07: `loaderPos.h = 811.2`, which is `1040 * 0.78`, not
    // `1080 * 0.78`. The surface knows; nothing here should guess.
    //
    // `surfaceTop` is the output-space y of the surface's origin. Derived from
    // what the surface lost to exclusive zones, which is top-anchored here (the
    // bar reserves, the pill reserves nothing, the dock is on Overlay). It
    // should equal the `at.y` the compositor reports for any tiled window — a
    // free cross-check if this is ever suspected.
    property real surfaceWidth: root.panelWidth
    property real surfaceHeight: root.panelHeight
    property real surfaceTop: 0
    property real surfaceLeft: 0
    // Whether any of the four above came from the surface rather than from the
    // defaults. `dragLength` is the one consumer that must know the difference.
    property bool surfaceReported: false

    // Report the overview surface's real geometry. Called by the surface; the
    // only writer of these four.
    function measuredAt(left, top, width, height) {
        root.surfaceLeft = left;
        root.surfaceTop = top;
        root.surfaceWidth = Math.max(1, width);
        root.surfaceHeight = Math.max(1, height);
        root.surfaceReported = true;
    }

    // `ZoneOverview`'s own frame, restated here because the destination and the
    // layout have to be the same arithmetic. Changing one of these moves the
    // card and the window it carries together; that is the point.
    //
    // `listMargin` is the `ListView`'s inset, `labelHeight` the strip under the
    // card that names the zone. The loader's height is reported rather than
    // derived from a share, for the reason above.
    readonly property real listMargin: 12
    readonly property real labelHeight: 34
    // The band under the strip: verbs that act on the card in front, and the
    // line she speaks on. Reserved here rather than overlaid, so the cards
    // shrink to make room and `dragLength` re-derives itself — the tray moves
    // the destination, and the gesture has to know that.
    readonly property real trayHeight: 132

    // The box a card is laid out inside, in surface-local coordinates.
    readonly property real _availX: root.listMargin
    readonly property real _availY: root.listMargin
    readonly property real _availW: root.surfaceWidth - 2 * root.listMargin
    readonly property real _availH: root.surfaceHeight
        - 2 * root.listMargin - root.labelHeight - root.trayHeight

    // --- What a card is a picture *of* ---------------------------------------
    //
    // The zone's usable area, not the whole output. A card drawn as the full
    // panel carries the bar's reserved strip inside it as dead space, and a
    // tiled window — whose `at.y` starts below that reservation — therefore
    // sits with a band of empty card above it. Casey, 2026-08-16: *"each window
    // currently has a gap on the top for where the top bar section would go."*
    // That gap is `surfaceTop * cardScale` and it was drawn faithfully; the
    // card was just a picture of the wrong rectangle.
    //
    // `surfaceTop` is what this surface lost to exclusive zones, and the header
    // above already states the invariant that makes it the right number: it
    // equals the `at.y` the compositor reports for any tiled window. Assumes
    // reservations stay top-anchored, which is true today (the bar reserves,
    // the pill reserves nothing, the dock is on Overlay). A bottom reservation
    // would need its own term, and the cross-check that would catch it is the
    // same one — a card whose windows no longer reach its bottom edge.
    readonly property real contentTop: root.surfaceTop
    readonly property real contentWidth: root.panelWidth
    readonly property real contentHeight: Math.max(1, root.panelHeight - root.contentTop)

    // How much of the space a card is allowed to fill.
    //
    // Fit-to-box made a card that nearly touched its neighbours, so the strip
    // read as one thing at a time with slivers either side. Casey, 2026-08-16:
    // *"scaled a little bit more / at least tighter together… we should be able
    // to smoothly scroll between them, floating cards and such."* Under one is
    // what makes them cards floating in a strip rather than a stack of screens.
    readonly property real cardFill: 0.84
    // Between neighbours. The ListView's own `spacing` is 0 so this is the one
    // number that says how far apart cards sit.
    readonly property real cardGutter: 16

    // One factor. `min` so the card keeps the *content's* aspect — which is what
    // a window's reported rect is measured against — and the carried window
    // fills it exactly. See the header on why a uniform scale is not a choice
    // here but a consequence of how `carried` works.
    readonly property real cardScale: Math.max(0.05, root.cardFill * Math.min(
        root._availW / Math.max(1, root.contentWidth),
        root._availH / Math.max(1, root.contentHeight)))

    readonly property real cardWidth: root.contentWidth * root.cardScale
    readonly property real cardHeight: root.contentHeight * root.cardScale

    // --- Where the resting card sits -----------------------------------------
    //
    // The delegate is the card plus its gutter, and the `ListView` holds the
    // current one centred (`StrictlyEnforceRange`, highlight range below), so a
    // neighbour shows on each side and the strip scrolls between them. It used
    // to be a full-width delegate pinned at x = 0 with the card centred inside
    // it, which is why the cards sat a screen apart.
    //
    // All four numbers live here and nowhere else. `ZoneOverview` lays the card
    // out at `cardInset*` and the carry names `cardX`/`cardY`; they are the same
    // rectangle in two coordinate spaces, not two rectangles kept in agreement.
    readonly property real delegateWidth: root.cardWidth + root.cardGutter
    readonly property real highlightBegin: Math.max(0,
        (root._availW - root.delegateWidth) / 2)

    // Delegate-local: what `ZoneOverview` positions the frame at.
    readonly property real cardInsetX: (root.delegateWidth - root.cardWidth) / 2
    readonly property real cardInsetY: (root._availH - root.cardHeight) / 2

    // Surface-local: what the carry's destination is measured in.
    readonly property real cardX: root._availX + root.highlightBegin + root.cardInsetX
    readonly property real cardY: root._availY + root.cardInsetY

    // How far the thumb has to climb for the window to *become* its card.
    //
    // The destination and the distance to it are one question, and quickstep
    // answers them in one call — `getSwipeUpDestinationAndLength(dp, ctx,
    // TEMP_RECT, …)` returns the task rect and the drag length together, and
    // `LauncherActivityInterface` computes that length as `dp.heightPx -
    // outRect.bottom`: how far the window's bottom edge travels.
    //
    // The rail asked `screen.height * 0.18` instead — TASK-60 named it *"a
    // number with no relationship to where the card actually is"* and left it.
    // Measured on blueline 2026-08-16: the card's bottom sits at 805 logical px
    // of 1080, so the true travel is 275 px and the old detent fired at 194.
    // The gesture committed with the window 40% short of its card and the
    // hand-off covered the rest in one frame — Casey's *"the swipe to this swap
    // is awkward"*, reported six times and tuned at from every direction except
    // this one.
    //
    // Until the surface has reported, this is the old 18% — not a floor, and
    // not a guess dressed up as arithmetic. `ZoneOverview` lives inside a gated
    // Loader, so nothing has measured anything until the overview has been
    // realised once, and computing the card against the *defaults* yields a
    // card the size of the panel and a drag length near zero. Behaving exactly
    // as yesterday until the real number exists is the honest fallback; the
    // rail latches whichever it got at the press, so no gesture ever changes
    // detent halfway through.
    readonly property real dragLength: root.surfaceReported
        ? Math.max(120, root.panelHeight
            - (root.surfaceTop + root.cardY + root.cardHeight))
        : root.panelHeight * 0.18

    // Where one window goes: its own place in the zone, drawn small.
    //
    // `cardX`/`cardY` are surface-local because that is what `ZoneOverview` lays
    // out in; the compositor speaks output coordinates, so the surface's origin
    // is added here. One conversion, at the one boundary where the two spaces
    // meet — the alternative is every caller remembering an offset, which is
    // the shape of bug this file exists to stop.
    // `contentTop` comes off the window's own `at.y` before scaling, because the
    // card is a picture of the usable zone and not of the whole output — the
    // same subtraction `ZoneOverview` makes when it lays the pane out. Miss it
    // in one place and the carried window lands a bar's height off its picture.
    function targetFor(w) {
        return {
            id: w.id,
            at: {
                x: root.surfaceLeft + root.cardX + w.at.x * root.cardScale,
                y: root.surfaceTop + root.cardY
                    + (w.at.y - root.contentTop) * root.cardScale
            },
            size: {
                width: w.size.width * root.cardScale,
                height: w.size.height * root.cardScale
            }
        };
    }

    // --- One clock, two strategies -------------------------------------------
    //
    // The gesture reports one number: how far the thumb has climbed. Everything
    // visible is a curve over it, and every curve lives here.
    //
    // This is TASK-52's rule — *one attention model, one clock, effects as
    // strategies over it* — and it is the same rule as TASK-60's "one owner",
    // one level up. The carry's `shift` and the destination's `presence` are
    // genuinely different curves: the window keeps travelling as the climb
    // continues past the multitasking detent toward home, while the destination
    // has to *recede*, because a preview that stayed would be showing somewhere
    // the release is no longer going to take you. Two curves is correct. Two
    // *places* is the bug, and it is exactly how the original was built — a
    // scale on the rail and a scale on the card, tuned by hand to agree.
    //
    // `pose` is a gravity well borrowed from the atmosphere primitives, so this
    // is her felt environment and not merely navigation chrome. It answers to
    // the same discipline.

    // How far the thumb has climbed, and where the multitasking detent sits.
    // The rail owns the gesture's geometry and reports both; nothing here
    // measures a finger.
    property real travel: 0
    property real detent: 1

    // The clock. 0 at rest, 1 at the multitasking detent, and it keeps counting
    // past it — the climb toward home is more of the same motion, not a second
    // gesture.
    readonly property real clock: root.travel / Math.max(1, root.detent)

    // Strategy one: what the compositor carries the window on.
    //
    // Past the detent this used to be a hard `Math.min(1, …)` — the thumb kept
    // travelling toward home and the window stopped dead. Android never goes
    // dead there, it goes *heavy*: `AnimatorControllerWithResistance` is
    // literally two playback controllers, one running 0→1 and a second that
    // *"seamlessly continues that animation but starts applying resistance"*,
    // with `DECELERATE` on scale (`RECENTS_SCALE_RESIST_INTERPOLATOR`) and
    // `FROM_APP(0.75f, 0.5f, 1f, false)` bounding how far it can go.
    //
    // Same shape. Past 1 the carry keeps extrapolating beyond the card rect —
    // the window shrinks further as the pull heads for home — but on a curve
    // that gives back less and less, so the last of the travel is felt as
    // weight rather than as a wall.
    //
    // A compositor older than the overshoot clamps this to 1 and the gesture is
    // exactly what it is today, which is what makes it safe to ship ahead of
    // the package (TASK-28).
    readonly property real maxOvershoot: 1.22
    readonly property real shift: root.clock <= 1
        ? Math.max(0, root.clock)
        : 1 + (root.maxOvershoot - 1) * root._decelerate(Math.min(1, root.clock - 1))

    // Android's DECELERATE, which is `1 - (1-t)²`.
    function _decelerate(t) {
        const inv = 1 - t;
        return 1 - inv * inv;
    }

    // Strategy two: how present the destination is. Rises to the detent, then
    // falls away over the same distance, so the two halves of the climb are
    // symmetric and a home-bound pull never arrives at a multitasking view the
    // release would not commit.
    readonly property real presence: root.clock <= 1
        ? Math.max(0, root.clock)
        : Math.max(0, 1 - (root.clock - 1))

    // Advance the gesture. One call per motion event.
    function pullTo(travel, detent) {
        root.detent = Math.max(1, detent);
        root.travel = Math.max(0, travel);
    }

    // Back to rest. The chrome settles through its own Behavior; the carry is
    // released by whatever destination the gesture arrived at.
    function rest() {
        root.travel = 0;
    }

    // The compositor is written from the derived value, not from the callers.
    //
    // A level, not an edge: `travel` moves under a thumb *and* under the settle
    // animation below, and both must reach the glass. Pushing from each caller
    // instead would mean the settle silently did nothing — which is precisely
    // the class of bug where a transform is applied by one path and undone by
    // another that forgot.
    onShiftChanged: root.progress(root.shift)

    // --- The settle ----------------------------------------------------------
    //
    // **Releasing continues the motion.** This is the piece that was missing,
    // and it is why the swipe into multitasking read as awkward no matter how
    // the fades were tuned: a lift at 60% of the climb committed *at* 60%, so
    // the window jumped from wherever the thumb left it to its final state with
    // no travel in between. Two motions with a cut between them.
    //
    // quickstep does not do that. `AbsSwipeUpHandler.handleNormalGestureEnd`:
    //
    //     float endShift = endTarget.isLauncher ? 1 : 0;
    //     long expectedDuration = Math.abs(Math.round((endShift - currentShift)
    //             * MAX_SWIPE_DURATION * SWIPE_DURATION_MULTIPLIER));
    //     duration = Math.min(MAX_SWIPE_DURATION, expectedDuration);
    //     startShift = currentShift;
    //
    // The shift animates from where the thumb left it to where the destination
    // is, over a duration proportional to the distance still to cover, and the
    // gesture lands only when it gets there. Same numbers here: 350 ms cap, and
    // the multiplier is `min(1/0.7, 1/0.3)` from `MIN_PROGRESS_FOR_OVERVIEW`.
    //
    // `last_zone` is the one target whose end shift is 0 — going back to the app
    // means the window travels *down* to full size rather than the view coming
    // up. It animates like every other destination instead of being the branch
    // that snaps.
    // What was still missing after the travel landed: a duration and an
    // easing curve cannot know how fast the thumb was going when it let go, so
    // a flick and a drift settled identically. Android's home path is
    // `RectFSpringAnim` with `DefaultSpringConfig` (`SwipeUpAnimationLogic:364`)
    // — a spring takes the release velocity as an *initial condition*, which is
    // the whole difference between a window that travels and one that was
    // thrown.
    //
    // So: a critically damped spring, integrated per frame, seeded with the
    // velocity the rail measures over the last 100 ms of the gesture.
    //
    // Per *frame*. `FrameAnimation` ticks on the render clock, so the carry
    // advances once per painted frame instead of once per whatever the socket
    // managed — which is what the old `NumberAnimation` on `travel` amounted
    // to, since every step of it went out through `progress()`.
    //
    // Critically damped, never underdamped: a navigation surface that rings
    // reads as a toy. It still overshoots once on a hard throw, and that part
    // is the point.

    // ω in 1/ms. Critical damping settles in about 6.6/ω, so 0.026 is ~250 ms.
    readonly property real settleOmega: 0.026
    // Nothing may outlive this. The commit is what releases the carry, so a
    // settle that never converged has to arrive anyway or the window strands —
    // which is the one failure TASK-60 exists to make impossible.
    readonly property int maxSettleMs: 600
    // px/ms. A thrown release helps; a wild one does not get to launch the
    // window off the glass.
    readonly property real maxSeedVelocity: 8

    property bool settling: false
    property var _settleTarget: null

    FrameAnimation {
        id: settleTick
        running: false
        property real velocity: 0
        property real to: 0
        property real elapsed: 0

        onTriggered: {
            // Clamped: a dropped frame must not integrate a large step and
            // fling the window across the panel.
            const dt = Math.min(48, settleTick.frameTime * 1000);
            settleTick.elapsed += dt;

            const w = root.settleOmega;
            const offset = root.travel - settleTick.to;
            settleTick.velocity += (-2 * w * settleTick.velocity - w * w * offset) * dt;
            root.travel = Math.max(0, root.travel + settleTick.velocity * dt);

            if ((Math.abs(root.travel - settleTick.to) < 0.5
                    && Math.abs(settleTick.velocity) < 0.02)
                || settleTick.elapsed >= root.maxSettleMs) {
                settleTick.running = false;
                root.travel = settleTick.to;
                root._finishSettle();
            }
        }
    }

    function _finishSettle() {
        root.settling = false;
        const t = root._settleTarget;
        root._settleTarget = null;
        // Commit *before* zeroing, and the order is load-bearing. Zeroing
        // first would drive `shift` to 0 with the carry still in flight —
        // the window snapping back to full size for a frame, which is the
        // very pop this whole task exists to remove. After `commit` the
        // carry is released, so `progress` refuses and the zero is inert
        // bookkeeping for the next gesture.
        root.commit(t);
        root.travel = 0;
    }

    // Release the gesture at a named destination, travelling there first.
    // `velocity` is px/ms along the rail, upward positive, as the rail measured
    // it over the tail of the gesture. Omitted means a release with no throw in
    // it, which is a legitimate way to let go.
    function settleTo(target, velocity) {
        // Where on the *clock* each destination lives — not on `shift`, which
        // saturates at the card and cannot express home at all.
        //
        // `presence` above is written to rise to the detent and then fall away
        // "over the same distance", which puts home at clock 2. This settled it
        // at 1 for both home and overview, so a long swipe ran the short
        // swipe's motion, held the multitasking view fully present, and then
        // teleported. Measured on the glass 2026-08-15: *"the long full swipe up
        // is completely fucked, only the short swipe partially works."*
        const endClock = target === "last_zone" ? 0 : target === "home" ? 2 : 1;
        settleTick.running = false;
        root._settleTarget = target;
        root.settling = true;
        settleTick.to = endClock * root.detent;
        settleTick.elapsed = 0;
        // Projected onto the way out, not applied raw. The gesture has already
        // resolved to one end target, so a hard release means "get there", not
        // "carry on past it and be pulled back" — which is what a raw upward
        // seed would do to a `last_zone` return and would read as a bounce.
        const toward = settleTick.to >= root.travel ? 1 : -1;
        settleTick.velocity = toward * Math.min(root.maxSeedVelocity,
            Math.max(0, velocity ?? 0));
        settleTick.running = true;
    }

    // --- The carry -----------------------------------------------------------

    // True between `begin` and the `commit`/`cancel` that releases it. Held so
    // a progress update with nothing in flight is silence rather than a refusal
    // per motion event — the compositor is right to refuse it, and a drag emits
    // one of those per frame.
    property bool inFlight: false

    // Last shift actually written, and how little a change has to be before it
    // is not worth sending.
    //
    // This was 0.01 — a hundred steps for the whole carry, inherited from
    // `poseActiveZone` on the reasoning that a round-trip per motion event was
    // expensive. **Measured on blueline 2026-08-16, it is not:** 200 samples of
    // connect → write → event-loop hop → reply → close, `{"op":"workspaces"}`,
    // came back at 0.47 ms median and 1.04 ms worst — under 3% of a 16.7 ms
    // frame. A hundred steps across 275 px of travel is a step every 2 logical
    // pixels, and on a scaling window that staircase is visible.
    //
    // 500 steps costs the same 0.5 ms per frame, because it is still one send
    // per motion event — the quantiser was never what bounded the rate.
    readonly property real shiftEpsilon: 0.002
    property real _lastShift: -1

    // Take the windows on the zone in front and name where each is going.
    //
    // The active zone only, which is quickstep's model too: `TaskViewSimulator`
    // transforms the *live* window of the running task onto its card, and every
    // other task in the strip is a snapshot. Carrying all of them would also be
    // wrong here for a concrete reason — the rects are named once at `begin`,
    // and the strip scrolls, so a window carried onto a card that then slides
    // sideways would come adrift from it.
    //
    // An empty zone carries nothing and says so: home has no windows, and the
    // compositor refuses an empty `begin` on purpose ("a carry with no windows
    // would release nothing on commit"). The gesture still works — the overview
    // is drawn, there is simply no window to bring with it.
    function begin() {
        const targets = [];
        for (const w of ViewtopControl.windows) {
            if (w.workspace === ViewtopControl.activeZone && w.at && w.size)
                targets.push(root.targetFor(w));
        }
        if (targets.length === 0)
            return false;
        root.inFlight = true;
        root._lastShift = -1;
        ViewtopControl.overviewBegin(targets);
        return true;
    }

    // 0 is where the windows live, 1 is each on its card, and past 1 is the
    // resisted overshoot toward home. A compositor that predates the overshoot
    // clamps it back to 1 on arrival, which is the old behaviour exactly.
    function progress(shift) {
        if (!root.inFlight)
            return;
        const s = Math.max(0, Math.min(root.maxOvershoot, shift));
        if (Math.abs(s - root._lastShift) < root.shiftEpsilon)
            return;
        root._lastShift = s;
        ViewtopControl.overviewProgress(s);
    }

    // Finish at a named destination: release the carry, then go there.
    //
    // Both halves, always, and in that order. The compositor releases the
    // transforms and reports back where it was told to arrive, but it does not
    // navigate — performing the arrival is the caller's, which is this. Doing
    // it here rather than at each call site is what makes a card tap and a pill
    // release the same machinery, which is precisely what the strand repro
    // broke.
    //
    // Safe with nothing in flight: a card tapped from an already-open overview
    // has no carry to release, and still has somewhere to go.
    function commit(target) {
        if (root.inFlight) {
            root.inFlight = false;
            root._lastShift = -1;
            ViewtopControl.overviewCommit(target);
        }
        root.arrive(target);
    }

    // Give up. Named apart from `commit("last_zone")` so the caller says which
    // one it meant, matching the compositor's own `cancel`.
    function cancel() {
        if (root.inFlight) {
            root.inFlight = false;
            root._lastShift = -1;
            ViewtopControl.overviewCancel();
        }
        root.arrive("last_zone");
    }

    // The end-target table: one branch per destination, and every gesture
    // resolves to exactly one of them.
    //
    // `target` is the wire's own shape — `"home"`, `"overview"`, `"last_zone"`,
    // or `{zone: {zone: N}}` — so the thing sent to the compositor and the
    // thing branched on here cannot drift apart into two vocabularies.
    function arrive(target) {
        if (target === "home") {
            // Square one, with the last screen's furniture cleared before the
            // strip moves rather than arrived at still wearing it.
            GlobalStates.missionControlOpen = false;
            GlobalStates.overviewOpen = false;
            GlobalStates.dockRevealed = false;
            GlobalStates.oskOpen = false;
            // `homeZone`, not a config lookup and not the literal 1: home is
            // zone 0 (`workspace::HOME_ZONE`), and pointing this at 1 put Home
            // on the first *app* zone — invisible while only home existed,
            // because the compositor clamps `to` against `count - 1`.
            ViewtopControl.zone(ViewtopControl.homeZone);
            return;
        }
        if (target === "overview") {
            GlobalStates.missionControlOpen = true;
            return;
        }
        if (target === "last_zone") {
            // Already there. The windows went back where they live when the
            // compositor released them, and that is the whole of it — this
            // branch exists so that "the swipe was given up" is a destination
            // with a name rather than the one path nobody wrote.
            return;
        }
        if (target && target.zone !== undefined) {
            ViewtopControl.zone(target.zone.zone);
            GlobalStates.missionControlOpen = false;
            GlobalStates.overviewOpen = false;
            return;
        }
        console.log("[ZoneTransition] arrival with no destination:", JSON.stringify(target));
    }

    // A zone, in the wire's shape. Spelled once so no call site hand-builds the
    // nesting and gets it subtly wrong.
    function zoneTarget(zone) {
        return { zone: { zone: zone } };
    }
}
