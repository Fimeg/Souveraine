// Souveraine's phone navigation rail.
//
// This is deliberately part of the primary Souveraine shell, rather than a
// second `qs -c …` configuration. It owns the small bottom input region that
// stays available over fullscreen applications.
//
// **Two swipes, and nothing else.** Casey, 2026-08-05: *"there is only two
// swipe modes on it."* A short climb reaches multitasking; a long one — past
// 18% of the panel — reaches home. Swipe down peels the nearest surface back.
// There is no tap: Home used to be one, which made a third gesture compete for
// the strip a thumb rests on, and it could only be told apart from a double tap
// by making it wait 350 ms first. Both are gone, along with the double tap's
// `hyprctl` fullscreen (START-HERE §3).
//
// With an OSK up, upward navigation is gated off. The keyboard owns the edge
// it rides on; a swipe that flips an app into multitasking while the user was
// mid-sentence was the complaint as it presented. Layout selection now lives
// on Squeekboard's own mode key, so the rail has no keyboard-swap gesture.
//
// The upward gesture is progressive, not a single threshold, and the travel is
// *reported* to `ZoneTransition` rather than interpreted here: the compositor
// carries the real windows onto their cards, and the destination fades in over
// the same climb, so the transition and the destination are one motion driven
// by one clock. This file measures a thumb and derives nothing from it.
// The handle tracks it too — it grows, brightens and lifts toward
// whichever stage the travel has reached, so the pill reads as "on top" of the
// motion rather than a passive strip. Release commits the stage the drag last
// held; a flick past the mission line commits multitasking even from a
// short-but-fast throw.
//
// Discovery: releasing short of REVEAL three times in a row (without ever
// finding Mission Control) nudges the handle with a brief pulse the first
// session only — Persistent.states.navigation.missionControlDiscovered gates
// it off for good once the gesture is found. This is the Souveraine analogue
// of Launcher3's AllAppsEduView triple-swipe hint, minus the full overlay.
//
// When the on-screen keyboard is summoned the rail rides ATOP it — the
// compositor anchors it above the keyboard's exclusive zone, so no height is
// measured here at all — and swipe down dismisses the
// keyboard first; with no keyboard up, swipe down dismisses a visible dock
// in any state (GlobalStates.dockSuppressed — pinned included).
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs
import qs.modules.common
import qs.services

PanelWindow {
    id: rail

    anchors.bottom: true
    implicitWidth: 200
    implicitHeight: Config.options?.dock.gestureRailHeight ?? 32
    // The compositor places this above the keyboard. Nothing here measures it.
    //
    // This used to be `ExclusionMode.Ignore` — layer-shell's exclusive zone
    // -1, "ignore what everyone else reserved and anchor to the whole screen" —
    // and then hand-computed a bottom margin from the keyboard's height, probed
    // by shelling out to `hyprctl -j layers`. Under viewtop there is no
    // hyprctl, so the probe threw on every attempt (a JSON.parse of an empty
    // string, logged in a loop every second or two forever) and the margin
    // stayed at a hardcoded fallback measured against a keyboard that is no
    // longer the one running. A fallback that is 200 while the keyboard is some
    // other height is exactly "the pill spawns in the middle of the keyboard".
    //
    // Zone 0 with Normal is the shell's own idiom for this (SidebarRight,
    // SidebarLeft unpinned, ReloadPopup): reserve nothing, but respect what
    // others reserved. The keyboard already declares an exclusive zone, so the
    // compositor anchors this to its top — correct at any height or rotation.
    //
    // START-HERE §3: a control that worked under Hyprland and does nothing now
    // is almost always a hyprctl or a hyprland.lua binding, and the fix is
    // never to re-add the binding.
    // Normal only while the keyboard is up, where riding above it is the whole
    // point — the pill is the dismiss handle and must stay reachable.
    //
    // Above the *dock* it has no such job. Casey, 2026-08-05: *"on the home
    // screen, does it really ever make sense for the PIL to be 'above' the
    // dock? I don't think so… I can see it above the keyboard, but above the
    // dock it has no function."* Respecting the dock's exclusive zone lifted it
    // there anyway, so off-keyboard it ignores exclusions and stays on the
    // bottom edge, behind the dock, still taking the gestures.
    exclusionMode: GlobalStates.oskOpen ? ExclusionMode.Normal : ExclusionMode.Ignore
    exclusiveZone: 0
    color: "transparent"
    // Top, not Overlay: Squeekboard's layout chooser is a child surface of the
    // OSK. On Overlay the pill could sit above that chooser and steal its taps;
    // on Top the popup draws over the pill. Dock and mission control are on
    // Overlay, so the pill now sits behind them when they're open — fine,
    // since those are summoned *from* the pill and occupy the screen anyway.
    // The pill still clears fullscreen apps (below Top), which is its job.
    WlrLayershell.layer: WlrLayer.Top
    WlrLayershell.namespace: "souveraine:navigation-rail"

    // The height probe that used to live here is gone, along with its retry
    // timer and `probeOskHeight()`. It existed to answer "how tall is the
    // keyboard right now"; getting that answer wrong put the pill inside the
    // keyboard's top rows, where it read as simply gone.
    //
    // The compositor answers it now, for free and always correctly, because the
    // keyboard's exclusive zone is the same fact the probe was reconstructing.
    // Deleted rather than ported to a viewtop equivalent: a second measurement
    // of a number the protocol already carries is the drift this class of bug
    // is made of.

    // --- Gesture stages -----------------------------------------------------
    // Two destinations, not two degrees of one. Casey, 2026-08-05: *"Home page
    // has dock visible, swipe up short once to multitasking, big swipe, like
    // beyond the bottom 15-20% is the to the home area."*
    //
    // A short climb arms **multitasking**; a big one — past a fifth of the
    // panel — arms **home**. The dock is no longer a stage at all: it lives on
    // zone one permanently, so there is nothing left to reveal.
    readonly property int revealAt: 48
    // Where the multitasking detent sits: the distance to the destination, asked
    // of the destination.
    //
    // This was `screen.height * 0.18` — a share of the panel with no
    // relationship to where the card actually is, which TASK-60 named as a
    // defect and left standing. Measured on the phone 2026-08-16 the card's
    // bottom edge sits at 805 logical px, so the window's real travel is 275 px
    // and the gesture was committing at 194: the release fired while the window
    // was still 40% short of being its card, and the rest of the distance was
    // covered by the hand-off. That is the seam Casey has reported six times.
    //
    // `ZoneTransition` owns it because it owns the card rect, and it is measured
    // rather than assumed — so it re-derives itself the day the dock reserves.
    // Everything downstream is still logical pixels, which is what `mouse.y` is.
    readonly property int missionAt: Math.max(revealAt + 60,
        Math.round(ZoneTransition.dragLength))
    // Speed and length become one number: how far the thumb would have carried
    // had it kept going. 130 ms is where the old rule sat — "1.1 px/ms promotes
    // one stage" over a 146 px stage gap — so the feel is the same minus the
    // cliff between promoted and not.
    readonly property int projectMs: 130

    // Live upward travel of the in-flight drag (0 while idle, grows as the
    // finger climbs). Drives the handle's appearance so the pill tracks the
    // motion. Only ever set by the MouseArea below.
    property real dragTravel: 0
    property bool dragging: false

    // The detent this gesture is running against, taken once at the press.
    //
    // quickstep names the destination and the distance to it together and once
    // — `initTransitionEndpoints` sets `mTransitionDragLength` at the start of
    // the gesture, not per motion event. Here it also closes a real seam: the
    // overview surface reports its geometry from inside a gated Loader, so on
    // the first climb of a session `missionAt` changes underneath the drag the
    // moment the peek realises the surface. Latched, the clock a gesture starts
    // on is the clock it finishes on.
    property int gestureDetent: 0
    readonly property int detentNow: rail.dragging && rail.gestureDetent > 0
        ? rail.gestureDetent : rail.missionAt

    // One edge, and everything rides on it. A drag ends by setting `dragging`
    // false and by nothing else. DUMP-bugs 8: a partial slide left the app
    // scaled small and only another pull brought it back, and the cause was
    // never found because every path that was *looked at* did call `endPull` —
    // release, cancel and the swap all did. Something ended a drag without
    // going through any of them, and searching for which one is the wrong shape
    // of fix: as long as restoring is a step, there is a path that skips it.
    //
    // Restoring is no longer a step (TASK-60). The compositor holds the
    // transform and releases it on `commit`/`cancel`, so what this edge cleans
    // up is the shell's own chrome plus a carry that no destination claimed —
    // an abandoned drag, or one the compositor took away mid-flight. The
    // release handler still sets `dragging` false *last*, after it has
    // committed, so `ZoneTransition.inFlight` is already clear by then and the
    // cancel here is a no-op rather than a second decision.
    onDraggingChanged: if (!rail.dragging) gestureArea.endPull()

    // The thumb has stopped climbing without letting go. iOS/Android both read
    // that as "show me where I'd land" and slide the switcher in before the
    // release; distance still decides what commits, this only draws it.
    //
    // 140 ms: past a 60 Hz stutter, under the 260 ms settle, so the preview
    // leads the commit instead of racing it. Latching — once dwelled it stays
    // for the rest of the drag, or a thumb that trembles would strobe the blur.
    property bool dwelled: false
    Timer {
        id: dwellTimer
        interval: 140
        onTriggered: rail.dwelled = true
    }

    // The destination is drawn for the whole climb, not only after a dwell.
    //
    // Casey has said this six times and it was the acceptance criterion left
    // unmet: *"the visual style from swiping up into this mode… we lost the
    // consistency… the swipe to this swap is awkward."* The cause was here. The
    // surface used to require `dwelled` — 140 ms of holding still — so an
    // ordinary pull shrank the real window over the live app with no backdrop
    // and no cards behind it, and then the entire destination appeared at once
    // on commit. Two visual events for one motion.
    //
    // Now it draws from the first climbing pixel and fades in against
    // `zonePullProgress` — the same number the compositor is carrying the window
    // on. At the commit frame the backdrop is already opaque and the card's
    // picture is already exactly under the real window, so releasing the carry
    // changes nothing on the glass. That is TASK-60's *"no frame where a window
    // is scaled by one and laid out by the other"*, and its *"the scale-on-drag
    // either continues into the view or is gone. Not both."*
    //
    // The dwell has not been deleted, it has been demoted: it still fires the
    // haptic that says "release here and you land in multitasking", which is
    // what it was actually good for. What it must not do is gate whether the
    // destination exists.
    //
    // The old worry — *"a fast flick to Home never flashes the blur on its way
    // past"* — is answered by the fade being continuous rather than by hiding
    // the surface. A flick spends a few frames at low progress, so it shows a
    // faint wash instead of a hard flash; and past the multitasking detent the
    // presence recedes (see `overviewPresence`), so a home-bound pull never
    // arrives at a destination the release would not commit.
    // The rail reports the clock — how far the thumb has climbed — and reads
    // none of the curves derived from it. `ZoneTransition` turns one travel into
    // the carry's shift and the destination's presence, because they are two
    // strategies over one clock and putting them in two files is how this task's
    // original bug was built (TASK-52: one attention model, one clock, effects
    // as strategies over it).
    //
    // The destination draws once the motion has *ever* paused, or once the climb
    // is deep enough to be heading there. Straight from quickstep, which decides
    // the same thing the same way (`AbsSwipeUpHandler`):
    //
    //     recentsAttachedToAppWindow = mHasMotionEverBeenPaused
    //             || mIsLikelyToStartNewTask;
    //
    // "Ever", not "currently" — `dwelled` latches for the rest of the drag, so a
    // thumb that trembles after pausing does not strobe the blur. This was
    // briefly removed here on the theory that the dwell gate caused the awkward
    // hand-off; it did not. The cut was the release committing *at* wherever the
    // thumb left it, which `ZoneTransition.settleTo` now travels through. A fast
    // unpaused flick still shows no destination on its way past, which is both
    // what Android does and what the original comment here wanted.
    readonly property bool peekWanted: rail.dragging && !GlobalStates.oskOpen
        && (rail.dwelled || rail.dragStage >= 1)

    // The haptic the dwell is still for: a tick when the thumb settles in
    // multitasking range, so the detent can be found without looking.
    onDwelledChanged: if (rail.dwelled && rail.dragStage === 1) Haptics.tick()

    onPeekWantedChanged: {
        if (rail.peekWanted) {
            peekFade.stop()
            GlobalStates.missionPeek = true
        } else if (GlobalStates.missionPeek) {
            // `endPull` zeroes the progress the cards are laid out against, and
            // that settle is animated. Dropping the surface on the release frame
            // would pop them off mid-animation, so it outlives the gesture just
            // long enough to shrink away. On a commit `missionControlOpen` has
            // already taken over and this expires unnoticed.
            peekFade.restart()
        }
    }

    Timer {
        id: peekFade
        interval: 300
        onTriggered: GlobalStates.missionPeek = false
    }

    // Stage the current travel has reached: 0 none, 1 reveal, 2 mission.
    // With a keyboard up the navigation stages do not exist; an app
    // mid-sentence has no business shrinking under the thumb.
    readonly property int dragStage: GlobalStates.oskOpen ? 0
        : dragTravel >= detentNow ? 2
        : dragTravel >= revealAt ? 1 : 0

    Rectangle {
        id: handle
        anchors.horizontalCenter: parent.horizontalCenter
        // Ride upward with the drag so the pill leads the motion, clamped so
        // it stays inside the rail's own height.
        y: (parent.height - height) / 2
            - Math.min(rail.dragTravel * 0.12, (parent.height - height) / 2)

        // Grow with travel; jump wider once a stage arms so the escalation is
        // felt, not just seen.
        width: (rail.dragStage >= 2 ? 190
            : rail.dragStage >= 1 ? 170 : 150)
            + Math.min(rail.dragTravel * 0.08, 24)
        height: 7 + (rail.dragStage >= 2 ? 3 : rail.dragStage >= 1 ? 1 : 0)
        radius: height / 2

        // Brighten and tint toward the accent as the drag escalates. Stage 2
        // pulls the handle to the theme accent — the "you've reached Mission
        // Control" tell.
        color: rail.dragStage >= 2
                ? (Appearance?.colors?.colPrimary ?? "#a0c8ff")
                : "#e6ffffff"
        // Invisible while the dock is up and nothing is being dragged — the dock
        // is the furniture on home, and a pill over it is a second hint for a
        // gesture the dock's presence already implies. Input is unaffected: the
        // rail still takes both swipes.
        opacity: rail.dragging ? 1
            : (ViewtopControl.activeZone === ViewtopControl.homeZone ? 0 : 0.9)

        // Idle nudge for the discovery hint: a brief scale pulse.
        scale: 1
        SequentialAnimation {
            id: discoveryPulse
            running: false
            loops: 2
            NumberAnimation { target: handle; property: "scale"; to: 1.18; duration: 160; easing.type: Easing.OutQuad }
            NumberAnimation { target: handle; property: "scale"; to: 1.0; duration: 220; easing.type: Easing.InOutQuad }
        }

        // All three off the house bank rather than hand-typed linear ramps.
        // Still gated on `!dragging` — while the thumb is on the pill the
        // handle tracks it directly and a Behavior would put a curve between
        // the finger and the thing it is holding.
        Behavior on width {
            enabled: !rail.dragging
            animation: Appearance.animation.elementMoveSmall.numberAnimation.createObject(this)
        }
        Behavior on color {
            animation: Appearance.animation.elementMoveFast.colorAnimation.createObject(this)
        }
        Behavior on y {
            enabled: !rail.dragging
            animation: Appearance.animation.elementMoveSmall.numberAnimation.createObject(this)
        }
    }

    MouseArea {
        id: gestureArea
        anchors.fill: parent
        property real startY: 0
        property real startAt: -1
        // Consecutive upward attempts that fell short of REVEAL without ever
        // reaching Mission Control — feeds the discovery nudge.
        property int shortSwipes: 0
        readonly property int tapSlop: 24

        // --- Release velocity ------------------------------------------------
        //
        // The commit used to read `travel / (now - startAt)` — the *mean* speed
        // across the whole gesture. A slow climb that ends in a flick therefore
        // read as slow, which is the one gesture a projection exists to catch,
        // and a long slow drag could never be thrown at all.
        //
        // quickstep is handed a measured end velocity instead:
        // `AbsSwipeUpHandler.onGestureEnded(float endVelocityPxPerMs, PointF)`,
        // filled by a VelocityTracker, and its fling test is
        // `isFling = mGestureStarted && !mIsMotionPaused
        //         && |endVelocityPxPerMs| > flingThreshold`.
        //
        // A ring over the tail of the motion is the same measurement. 100 ms is
        // ~6 frames at 60 Hz: long enough to fit a line through, short enough
        // that only the end of the gesture is in it.
        readonly property int velocityWindowMs: 100
        property var samples: []

        function sample(y: real): void {
            const now = Date.now()
            samples.push({ t: now, y: y })
            // Always keep two, or a flick that lands entirely inside one frame
            // has nothing to measure between.
            while (samples.length > 2 && now - samples[0].t > velocityWindowMs)
                samples.shift()
        }

        // px/ms, upward positive.
        function endVelocity(): real {
            if (samples.length < 2)
                return 0
            const first = samples[0]
            const last = samples[samples.length - 1]
            const dt = last.t - first.t
            if (dt <= 0)
                return 0
            return (first.y - last.y) / dt
        }

        // Double-tap used to toggle fullscreen through `hyprctl dispatch`, and
        // has therefore done nothing since the viewtop move — START-HERE §3's
        // exact shape, and the rule there is that the fix is never to re-add
        // the binding. viewtop tiles a zone to the whole display already and
        // has no fullscreen intent in `wire`; a window that wants the glass to
        // itself asks for a zone (`workspace.rs`: "a window that wants the
        // whole display is asking not to share"). So this is deleted rather
        // than ported, and the second tap is inert until there is a verb to
        // point it at — a gesture that fires nothing is better than one that
        // fires something nobody chose.
        //
        // What it leaves behind: the single-tap Home path no longer needs to be
        // delayed by a double-tap window. It still is, because the delay is
        // also what lets a fast double-tap be distinguished at all, and TASK-55
        // Q5 wants Home to stay predictable more than it wants it instant.

        // A single tap is Home. Delay it by the double-tap window so the
        // existing fullscreen gesture remains unambiguous; the second tap
        // cancels this timer before toggling fullscreen.
        // Square one has to be reachable: it is the state you get back to when
        // everything else is confusing, and a phone whose Home does nothing is
        // one wrong gesture from being stuck.
        //
        // The body moved to `ZoneTransition.arrive("home")` (TASK-60), which is
        // the one table where an end target becomes an action. It clears the
        // furniture and calls the compositor's own `workspace` verb — not
        // `hyprctl dispatch`, which is not running under viewtop, so it went
        // nowhere and Home silently stopped existing — with `homeZone` rather
        // than a config lookup that resolved to 1 and put Home on the first
        // *app* zone.
        function goHome(velocity: real): void {
            ZoneTransition.settleTo("home", velocity ?? 0)
        }

        // Commit an upward gesture. `stage` is the stage the drag settled on
        // (0/1/2).
        //
        // Casey, 2026-08-05: "a proper toggle for the multitasking sections,
        // that was the short swipe up deal, and long swipe up was to zone one
        // which was to be clear and have widget space."
        //
        // So the two swipes are two different destinations rather than degrees
        // of the same one: **short = multitasking, long = square one**. This
        // used to be short = dock, long = multitasking, which left Home
        // reachable only by tap — and the tap was `hyprctl`, so under viewtop
        // there was no way up and out at all.
        //
        // Short *toggles*. Reaching multitasking with a gesture that cannot
        // also leave it means the way out is a different gesture than the way
        // in, which is the thing that makes a phone feel stuck.
        // Every branch here is now a named end target, and each one releases the
        // carry by *arriving* — which is what makes a stranded window
        // impossible rather than merely unlikely (TASK-60).
        function commitUp(stage: int, velocity: real): void {
            if (stage >= 2) {
                // Square one. The arrival clears multitasking, the overview,
                // the dock and the OSK before moving, so the zone is not
                // arrived at with the last screen's furniture still up.
                goHome(velocity)
                shortSwipes = 0
                return
            }
            if (stage >= 1) {
                // Multitasking only. The dock is not touched here any more: it
                // belongs to zone one and is shown by being *there*, not by a
                // gesture that reveals it over whatever else is on screen.
                //
                // Short *toggles*: a way in that cannot also be the way out is
                // what makes a phone feel stuck. Leaving is `last_zone` — a
                // destination with a name, so the release runs the same
                // machinery as every other exit rather than being the one path
                // that only unsets a flag.
                if (GlobalStates.missionControlOpen) {
                    ZoneTransition.settleTo("last_zone", velocity)
                    GlobalStates.missionControlOpen = false
                } else {
                    ZoneTransition.settleTo("overview", velocity)
                    if (!Persistent.states.navigation.missionControlDiscovered)
                        Persistent.states.navigation.missionControlDiscovered = true
                }
                shortSwipes = 0
            }
        }

        onPressed: mouse => {
            startY = mouse.y
            startAt = Date.now()
            samples = [{ t: startAt, y: mouse.y }]
            rail.gestureDetent = rail.missionAt
            rail.dragging = true
            rail.dragTravel = 0
            rail.dwelled = false
            dwellTimer.stop()
        }


        onPositionChanged: mouse => {
            if (!rail.dragging)
                return
            // Upward travel is positive; downward drags leave travel at 0 so
            // the handle doesn't chase a dismiss gesture.
            rail.dragTravel = Math.max(0, startY - mouse.y)
            sample(mouse.y)

            // Restart while still moving: the timer only reaches its interval
            // once the thumb holds still.
            if (!rail.dwelled)
                dwellTimer.restart()

            // The app shrinks under the thumb, and it is the *real* app.
            // Casey, 2026-08-05: *"swipe up just a bit makes the app sorta
            // scale and you fall into a multi tasking area"*, and on the
            // question of pictures-vs-windows: *"real windows is nicer — that's
            // what we had on one of the previews and it worked great."* Phosh's
            // `home.c` is the same shape, a drag surface travelling between two
            // states rather than a button that teleports.
            //
            // This was deleted once, and deleting it was the wrong correction.
            // What was wrong was that the scale had been mistaken for the
            // *destination* — there was nothing behind it but the zone you were
            // already on, smaller. It is the transition, `ZoneOverview` is the
            // destination, and TASK-60's acceptance wants them to share one
            // progress rather than animate the same motion from two clocks. So
            // the number is published, not kept.
            //
            // Scaled against `missionAt`, the multitasking detent, so the app
            // has visibly become a card by the moment it would commit. It keeps
            // shrinking past that toward home, which makes the long pull read
            // as a further degree of the same motion rather than a second,
            // unrelated gesture.
            //
            // Nothing is scaled below 0.6: past that the window is a thumbnail
            // and its own inverse-mapped touch targets stop being findable if
            // the drag is abandoned there.
            //
            // The pose is a navigation gesture's transition. With a keyboard
            // up the climb belongs to the keyboard instead. A scale under that
            // thumb reads as the app entering multitasking, so the keyboard's
            // climb deliberately touches nothing.
            if (!GlobalStates.oskOpen) {
                // Name the destination once, at the moment this becomes an
                // upward pull — quickstep's `getSwipeUpDestinationAndLength`
                // returns the rect and the length together for the same reason.
                // From here the compositor owns these windows' geometry until
                // something commits.
                //
                // On the first climbing pixels rather than on `onPressed`,
                // which is where this started. `focus_border` yields while a
                // window is carried, so beginning on every press flashed the
                // focus frame off and back on for a tap or a swipe *down* —
                // gestures that are not this one and must cost nothing. Eight
                // pixels is well under the first detent at 48, so the carry is
                // running long before anything could commit and the motion is
                // still continuous from the start of the climb.
                //
                // Not with a keyboard up: the climb touches nothing. Layout
                // choice is an explicit key on Squeekboard itself.
                if (!ZoneTransition.inFlight && rail.dragTravel >= 8)
                    ZoneTransition.begin()
                // The clock, and nothing derived from it. `pullTo` turns this
                // into the carry's shift and the destination's presence.
                ZoneTransition.pullTo(rail.dragTravel, rail.detentNow)
            }
        }

        // End an upward pull. Called from one place — `rail.onDraggingChanged`
        // — because every destination wants the same thing and the difference
        // was never in the action.
        //
        // It used to take a `committed` flag it did not read, and it used to
        // restore the windows. Neither is true now: the destinations differ,
        // but the *cleanup* never did, and the windows are the compositor's to
        // put back.
        //
        // What is left is the shell's own chrome, plus `cancel` as the safety
        // net for a drag that ended without reaching any destination — a
        // sideways smudge, or the compositor taking the sequence away after a
        // wake. Every real commit has already cleared `inFlight`, so this is a
        // no-op on those paths rather than a second writer racing the first.
        //
        // Idempotent on purpose: it is the safety net, and a net that cannot be
        // touched twice is not one.
        function endPull(): void {
            dwellTimer.stop()
            rail.dwelled = false
            // A settle is a release that is still travelling. Zeroing the clock
            // or cancelling under it would be the second writer again — the
            // animation is already carrying the window to a named destination
            // and will commit when it arrives.
            if (ZoneTransition.settling)
                return
            ZoneTransition.rest()
            if (ZoneTransition.inFlight)
                ZoneTransition.cancel()
        }

        onReleased: mouse => {
            sample(mouse.y)
            const travel = Math.max(0, startY - mouse.y)
            const deltaY = mouse.y - startY
            const speed = Math.max(0, endVelocity()) // px/ms, upward only
            // The handle lights by raw travel — where the thumb is — and the
            // commit reads the projection — where it was going.
            const projected = travel + speed * rail.projectMs
            rail.dragTravel = 0

            // Upward: pick the committed stage from travel, then let a fast
            // flick promote it one step (a quick short throw still reaches
            // Mission Control).
            //
            // Navigation is OFF while the keyboard is up: the climb is an
            // abandoned pull, not a mission-to-home gesture. Layout selection
            // stays on Squeekboard's mode key.
            // `travel` floors it: projection alone would turn a 10 px smudge
            // flicked in 5 ms into a 270 px throw, i.e. Home. Bare `tapSlop` —
            // it is the MouseArea's. The old flick rule read `rail.tapSlop`,
            // which is undefined, so `travel >= undefined` was always false and
            // that whole promotion path had never once fired.
            if (!GlobalStates.oskOpen && travel >= tapSlop
                && projected >= rail.revealAt) {
                commitUp(projected >= rail.detentNow ? 2 : 1, speed)
                rail.dragging = false
                return
            }

            // Short of the first detent the destination is the app you were
            // already in — a commit like the others, not a pending state. And
            // it *travels* there: an abandoned half-swipe used to snap the
            // window back from wherever the thumb left it, which is the same cut
            // the commit paths had and the one felt most often, because a pull
            // that changes its mind is the commonest gesture of all. `last_zone`
            // settles to shift 0, so the window walks back down to full size.
            //
            // Only when there was something to carry. A tap, a sideways smudge
            // or a swipe down never began one, and `settleTo` on an idle
            // transition would animate a clock nothing is reading.
            if (!GlobalStates.oskOpen && ZoneTransition.inFlight)
                ZoneTransition.settleTo("last_zone", speed)
            rail.dragging = false

            // Downward: peel the nearest surface (keyboard, then dock).
            if (deltaY >= rail.revealAt) {
                if (GlobalStates.oskOpen) {
                    GlobalStates.oskOpen = false
                    return
                }
                if (GlobalStates.missionControlOpen) {
                    GlobalStates.missionControlOpen = false
                    return
                }
                GlobalStates.dockRevealed = false
                GlobalStates.dockSuppressed = true
                return
            }

            // Neither committed: an upward attempt that fell short of REVEAL
            // counts toward the discovery nudge. Anything else — a tap, a
            // sideways smudge — does nothing at all, on purpose. Casey,
            // 2026-08-05: *"there is only two swipe modes on it."* The rail is
            // two swipes and no taps: short up is multitasking, long up is
            // home. A tap used to be a third way to reach Home and a double tap
            // a fourth thing on the same 32 px strip, which is three gestures
            // competing for the one region a thumb rests in — and one of them
            // could only resolve by making the other wait 350 ms first.
            //
            // Not while the keyboard is up: navigation discovery should not
            // train against a gesture that is intentionally gated off.
            if (travel > tapSlop && !GlobalStates.oskOpen)
                maybeNudgeDiscovery()
        }

        // Count a short upward attempt; after three in a row, and only until
        // the gesture has been discovered, pulse the handle once.
        function maybeNudgeDiscovery(): void {
            if (Persistent.states.navigation.missionControlDiscovered)
                return
            shortSwipes += 1
            if (shortSwipes >= 3) {
                shortSwipes = 0
                discoveryPulse.restart()
            }
        }

        // The compositor taking the sequence away mid-drag — the FTS controller
        // does this after a wake. Same edge, same restore.
        onCanceled: {
            rail.dragTravel = 0
            rail.dragging = false
        }
    }
}
