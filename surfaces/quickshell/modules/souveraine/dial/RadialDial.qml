// The radial dial — a thumb-reachable ring of verbs.
//
// Component before contents (TASK-31). The ring, the hit-testing, the detents
// and the dismissal are the hard part and they are here; the entries are a
// list this file consumes, not a list it owns. When the verb tables land
// (TASK-30) `entries` gets fed from `describe` instead of from the default
// below, and nothing else in this file changes. That is the whole reason the
// entry shape is {icon, label, verb, enabled, reason} — `reason` exists so a
// refused verb can show the refusal's own words rather than going quietly grey.
//
// Invocation is deliberately not owned here either. Anything that can set
// `open` can raise it: the pill's long-hold, an edge gesture, a squeeze once
// grip produces events, or `dial open` over IPC.
import QtQuick
import QtQuick.Shapes
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets

Item {
    id: root
    anchors.fill: parent
    visible: opacity > 0.01
    opacity: root.open ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    property bool open: false

    // The point the opener asked for — a long-press hands over its touch point
    // so the ring appears under the thumb rather than in the middle of a phone
    // you are holding by one edge. NaN means no point was given.
    //
    // Held as a request, not written into originX/originY, because openAt()
    // runs on the same signal that maps the layer surface: width and height
    // are still 0 at that instant, so an assigned origin clamps to 0,0 and
    // stays there once the window sizes. As a binding it corrects itself.
    property real requestedX: NaN
    property real requestedY: NaN

    // Where the ring is centred.
    readonly property real originX: isNaN(root.requestedX)
        ? root.width / 2
        : Math.max(root.ringRadius * 1.3,
          Math.min(root.width - root.ringRadius * 1.3, root.requestedX))
    readonly property real originY: isNaN(root.requestedY)
        ? root.height * 0.68
        : Math.max(root.ringRadius * 1.3,
          Math.min(root.height - root.ringRadius * 1.3, root.requestedY))

    readonly property real ringRadius: Math.min(width, height) * 0.30
    readonly property real deadZone: ringRadius * 0.42

    // One entry per verb. `verb` is a callable; nothing here knows what it
    // does, which is what lets the same component serve system actions, agent
    // actions and app actions without growing a switch statement.
    property var entries: []

    // -1 = nothing selected (thumb inside the dead zone or dial closed).
    property int selected: -1
    property int lastDetent: -1

    signal invoked(var entry)

    // Call with no arguments when there is no touch point to hand over: the
    // ring then centres itself on the surface it was given.
    function openAt(x, y) {
        if (root.entries.length === 0)
            return;
        root.requestedX = x === undefined ? NaN : x;
        root.requestedY = y === undefined ? NaN : y;
        root.selected = -1;
        root.lastDetent = -1;
        root.open = true;
    }

    function close() {
        root.open = false;
        root.selected = -1;
        root.requestedX = NaN;
        root.requestedY = NaN;
    }

    // Angle of entry i, measured so the first entry sits at the top and the
    // ring fills clockwise.
    function angleFor(i) {
        return (i / Math.max(1, root.entries.length)) * 2 * Math.PI - Math.PI / 2;
    }

    // Which entry a point selects. Inside the dead zone selects nothing, which
    // is what makes "open it and let go without choosing" a first-class
    // outcome instead of an accident.
    function entryAt(x, y) {
        const dx = x - root.originX;
        const dy = y - root.originY;
        if (Math.sqrt(dx * dx + dy * dy) < root.deadZone)
            return -1;
        const n = root.entries.length;
        if (n === 0)
            return -1;
        let a = Math.atan2(dy, dx) + Math.PI / 2;
        while (a < 0) a += 2 * Math.PI;
        while (a >= 2 * Math.PI) a -= 2 * Math.PI;
        return Math.round(a / (2 * Math.PI) * n) % n;
    }

    // A detent every time the selection changes under the thumb. This is the
    // dial's whole tactile argument: you can pick an entry without looking,
    // because the ring answers your thumb. Requires the haptic device
    // (pmi8998_haptics, enabled 2026-07-26) and feedbackd's `quiet` profile,
    // where button-pressed maps to VibraPattern.
    onSelectedChanged: {
        if (!root.open || root.selected === root.lastDetent)
            return;
        root.lastDetent = root.selected;
        if (root.selected >= 0)
            Haptics.tick();
    }

    // Scrim. Tapping it dismisses without choosing.
    Rectangle {
        anchors.fill: parent
        color: "#99000000"
    }

    Repeater {
        model: root.entries

        delegate: Item {
            id: seat
            required property int index
            required property var modelData

            readonly property bool isSelected: root.selected === seat.index
            readonly property real a: root.angleFor(seat.index)
            readonly property bool usable: seat.modelData.enabled !== false

            x: root.originX + Math.cos(seat.a) * root.ringRadius - width / 2
            y: root.originY + Math.sin(seat.a) * root.ringRadius - height / 2
            width: 64
            height: 64

            // Grows toward the thumb rather than jumping — the selection
            // should feel like the ring leaning, not like a menu re-rendering.
            scale: seat.isSelected ? 1.22 : 1.0
            Behavior on scale { NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

            // The charge. Drawn under the seat and growing past it, so a hold
            // reads as the entry swelling toward committing rather than as a
            // separate widget appearing next to it.
            Rectangle {
                anchors.centerIn: parent
                visible: seat.isSelected && root.holdProgress > 0
                    && typeof seat.modelData.hold === "function"
                width: parent.width * (1 + 0.45 * root.holdProgress)
                height: width
                radius: width / 2
                color: "transparent"
                border.width: 3
                border.color: Appearance.colors.colPrimary
                opacity: 0.35 + 0.65 * root.holdProgress
            }

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: seat.isSelected
                    ? Appearance.colors.colPrimary
                    : Appearance.colors.colLayer2
                opacity: seat.usable ? 1.0 : 0.4

                MaterialSymbol {
                    anchors.centerIn: parent
                    text: seat.modelData.icon ?? "radio_button_unchecked"
                    iconSize: 28
                    color: seat.isSelected
                        ? Appearance.colors.colOnPrimary
                        : Appearance.colors.colOnLayer2
                }
            }

            StyledText {
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: parent.bottom
                anchors.topMargin: 6
                text: seat.modelData.label ?? ""
                color: "white"
                font.pixelSize: Appearance.font.pixelSize.smaller
                opacity: seat.isSelected ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: 90 } }
            }
        }
    }

    // The refusal, shown in the middle where the thumb is not. A disabled
    // entry that just greys out teaches nothing; this says why.
    StyledText {
        x: root.originX - width / 2
        y: root.originY - height / 2
        width: root.deadZone * 1.8
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        color: "#d9ffffff"
        font.pixelSize: Appearance.font.pixelSize.smaller
        visible: root.selected >= 0
            && root.entries[root.selected]?.enabled === false
        text: root.entries[root.selected]?.reason ?? ""
    }

    // ## Holding an entry, for the second verb it carries
    //
    // An entry may declare `hold` beside `verb`: tap does one thing, holding
    // does the heavier one. Screenshot/record is the case that asked for it —
    // Casey, 2026-08-06: "if I hold on the screenshot button... it'll start
    // recording a video until I hit the stop button on the radial dial."
    //
    // The hold has to *show*, or it is indistinguishable from a tap that has
    // not been let go of yet, and the user releases before it fires. The ring
    // below fills as it charges, so the commitment is visible while it is
    // still cancellable.
    //
    // Sliding to another entry cancels: the thumb is already the dial's
    // selection mechanism, so a hold that survived moving off its entry would
    // fire the wrong verb — and this ring holds Lock and Kill window.
    property real holdProgress: 0
    property bool holdFired: false
    readonly property int holdMs: 550

    Timer {
        id: holdCharge
        interval: 16
        repeat: true
        onTriggered: {
            root.holdProgress += interval / root.holdMs;
            if (root.holdProgress < 1)
                return;
            stop();
            root.holdProgress = 0;
            root.holdFired = true;
            const entry = root.entries[root.selected];
            if (!entry || entry.enabled === false || typeof entry.hold !== "function")
                return;
            // Closes on firing, like a release would: the verb has been
            // committed, and leaving the ring up over a recording that has
            // already started reads as "it did not take".
            root.close();
            root.invoked(entry);
            entry.hold();
        }
    }

    function beginHold() {
        root.holdProgress = 0;
        root.holdFired = false;
        const entry = root.entries[root.selected];
        if (entry && entry.enabled !== false && typeof entry.hold === "function")
            holdCharge.restart();
        else
            holdCharge.stop();
    }

    MouseArea {
        anchors.fill: parent
        enabled: root.open
        hoverEnabled: false
        preventStealing: true

        onPositionChanged: mouse => {
            const before = root.selected;
            root.selected = root.entryAt(mouse.x, mouse.y);
            if (root.selected !== before)
                root.beginHold();
        }
        onPressed: mouse => {
            root.selected = root.entryAt(mouse.x, mouse.y);
            root.beginHold();
        }
        onReleased: mouse => {
            holdCharge.stop();
            root.holdProgress = 0;
            // The hold already ran and already closed the ring. Firing the tap
            // verb now would take a screenshot every time a recording starts.
            if (root.holdFired) {
                root.holdFired = false;
                return;
            }
            const i = root.entryAt(mouse.x, mouse.y);
            root.close();
            if (i < 0 || i >= root.entries.length)
                return;
            const entry = root.entries[i];
            if (entry.enabled === false)
                return;
            root.invoked(entry);
            if (typeof entry.verb === "function")
                entry.verb();
        }
    }

    Keys.onEscapePressed: root.close()
}
