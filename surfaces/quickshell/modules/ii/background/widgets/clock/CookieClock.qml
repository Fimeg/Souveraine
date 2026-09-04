pragma ComponentBehavior: Bound

// Souveraine fork of ii's CookieClock. One change: the category-preset gate
// reads cookie.aiPreset instead of cookie.aiStyling (see the comment at
// setClockPreset). Diff against ii-base before re-applying if ii updates.

import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions
import QtQuick
import QtQuick.Layouts
import Qt5Compat.GraphicalEffects
import Quickshell.Io

import qs.modules.ii.background.widgets.clock.dateIndicator
import qs.modules.ii.background.widgets.clock.minuteMarks

Item {
    id: root

    readonly property string clockStyle: Config.options.background.widgets.clock.style

    property real implicitSize: 230

    property color colShadow: Appearance.colors.colShadow
    property color colBackground: Appearance.colors.colPrimaryContainer
    property color colOnBackground: ColorUtils.mix(Appearance.colors.colSecondary, Appearance.colors.colPrimaryContainer, 0.15)
    property color colBackgroundInfo: ColorUtils.mix(Appearance.colors.colPrimary, Appearance.colors.colPrimaryContainer, 0.55)
    property color colHourHand: Appearance.colors.colPrimary
    property color colMinuteHand: Appearance.colors.colTertiary
    property color colSecondHand: Appearance.colors.colPrimary

    readonly property list<string> clockNumbers: DateTime.time.split(/[: ]/)
    readonly property int clockHour: parseInt(clockNumbers[0]) % 12
    readonly property int clockMinute: DateTime.clock.minutes
    readonly property int clockSecond: DateTime.clock.seconds

    implicitWidth: implicitSize
    implicitHeight: implicitSize

    function applyStyle(sides, dialStyle, hourHandStyle, minuteHandStyle, secondHandStyle, dateStyle) {
        Config.options.background.widgets.clock.cookie.sides = sides
        Config.options.background.widgets.clock.cookie.dialNumberStyle = dialStyle
        Config.options.background.widgets.clock.cookie.hourHandStyle = hourHandStyle
        Config.options.background.widgets.clock.cookie.minuteHandStyle = minuteHandStyle
        Config.options.background.widgets.clock.cookie.secondHandStyle = secondHandStyle
        Config.options.background.widgets.clock.cookie.dateStyle = dateStyle
    }

    function setClockPreset(category) {
        // Souveraine fork (2026-08-05): gate is cookie.aiPreset, not
        // cookie.aiStyling. Upstream runs both the wallpaper-categorizer
        // (switchwall.sh) and this preset override off the one flag, so
        // turning categorization on rewrote users' configured clock styles
        // via applyStyle. aiStyling stays true for the pipeline; this
        // surface only moves when aiPreset is explicitly on. Verbatim
        // upstream otherwise.
        if (!Config.options.background.widgets.clock.cookie.aiPreset) return;
        if (category === "") return;
        print("[Cookie clock] Setting clock preset for category: " + category)
        // "abstract", "anime", "city", "minimalist", "landscape", "plants", "person", "space"
        if (category == "abstract") {
            applyStyle(9, "none", "fill", "medium", "dot", "bubble")
        } else if (category == "anime") {
            applyStyle(7, "none", "fill", "bold", "dot", "bubble")
        } else if (category == "city" || category == "space") {
            applyStyle(23, "full", "hollow", "thin", "classic", "bubble")
        } else if (category == "minimalist") {
            applyStyle(6, "none", "fill", "bold", "dot", "hide")
        } else if (category == "landscape") {
            applyStyle(14, "full", "hollow", "medium", "classic", "bubble")
        } else if (category == "plants") {
            applyStyle(9, "dots", "fill", "bold", "dot", "border")
        } else if (category == "person") {
            applyStyle(14, "full", "classic", "classic", "classic", "rect")
        }
    }

    FileView {
        id: categoryFileView
        path: Config.ready ? Directories.generatedWallpaperCategoryPath : ""
        watchChanges: true
        onFileChanged: reload()
        onLoaded: {
            root.setClockPreset(categoryFileView.text().trim())
        }
    }

    property bool useSineCookie: Config.options.background.widgets.clock.cookie.useSineCookie
    StyledDropShadow {
        target: root.useSineCookie ? sineCookieLoader : roundedPolygonCookieLoader

        RotationAnimation on rotation {
            running: Config.options.background.widgets.clock.cookie.constantlyRotate
            duration: 30000
            easing.type: Easing.Linear
            loops: Animation.Infinite
            from: 360
            to: 0
        }
    }
    Loader {
        id: sineCookieLoader
        z: 0
        visible: false // The DropShadow already draws it
        active: root.useSineCookie
        sourceComponent: SineCookie {
            implicitSize: root.implicitSize
            sides: Config.options.background.widgets.clock.cookie.sides
            color: root.colBackground
        }
    }
    Loader {
        id: roundedPolygonCookieLoader
        z: 0
        visible: false // The DropShadow already draws it
        active: !root.useSineCookie
        sourceComponent: MaterialCookie {
            implicitSize: root.implicitSize
            sides: Config.options.background.widgets.clock.cookie.sides
            color: root.colBackground
        }
    }

    // Hour/minutes numbers/dots/lines
    MinuteMarks {
        anchors.fill: parent
        color: root.colOnBackground
    }

    // Stupid extra hour marks in the middle
    FadeLoader {
        id: hourMarksLoader
        anchors.centerIn: parent
        shown: Config.options.background.widgets.clock.cookie.hourMarks
        sourceComponent: HourMarks {
            implicitSize: 135 * (1.75 - 0.75 * hourMarksLoader.opacity)
            color: root.colOnBackground
            colOnBackground: ColorUtils.mix(root.colBackgroundInfo, root.colOnBackground, 0.5)
        }
    }

    // Number column in the middle
    FadeLoader {
        id: timeColumnLoader
        anchors.centerIn: parent
        shown: Config.options.background.widgets.clock.cookie.timeIndicators
        scale: 1.4 - 0.4 * timeColumnLoader.shown
        Behavior on scale {
            animation: Appearance.animation.elementResize.numberAnimation.createObject(this)
        }

        sourceComponent: TimeColumn {
            color: root.colBackgroundInfo
        }
    }

    // Minute hand
    FadeLoader {
        anchors.fill: parent
        z: 1
        shown: Config.options.background.widgets.clock.cookie.minuteHandStyle !== "hide"
        sourceComponent: MinuteHand {
            anchors.fill: parent
            clockMinute: root.clockMinute
            style: Config.options.background.widgets.clock.cookie.minuteHandStyle
            color: root.colMinuteHand
        }
    }

    // Hour hand
    FadeLoader {
        anchors.fill: parent
        z: item?.style === "hollow" ? 0 : 2
        shown: Config.options.background.widgets.clock.cookie.hourHandStyle !== "hide"
        sourceComponent: HourHand {
            clockHour: root.clockHour
            clockMinute: root.clockMinute
            style: Config.options.background.widgets.clock.cookie.hourHandStyle
            color: root.colHourHand
        }
    }

    // Second hand
    FadeLoader {
        id: secondHandLoader
        z: (Config.options.background.widgets.clock.cookie.secondHandStyle === "line") ? 2 : 3
        shown: Config.options.time.secondPrecision && Config.options.background.widgets.clock.cookie.secondHandStyle !== "hide"
        anchors.fill: parent
        sourceComponent: SecondHand {
            id: secondHand
            clockSecond: root.clockSecond
            style: Config.options.background.widgets.clock.cookie.secondHandStyle
            color: root.colSecondHand
        }
    }

    // Center dot
    FadeLoader {
        z: 4
        anchors.centerIn: parent
        shown: Config.options.background.widgets.clock.cookie.minuteHandStyle !== "bold"
        sourceComponent: Rectangle {
            color: Config.options.background.widgets.clock.cookie.minuteHandStyle === "medium" ? root.colBackground : root.colMinuteHand
            implicitWidth: 6
            implicitHeight: implicitWidth
            radius: width / 2
        }
    }

    // Date
    FadeLoader {
        anchors.fill: parent
        shown: Config.options.background.widgets.clock.cookie.dateStyle !== "hide"

        sourceComponent: DateIndicator {
            color: root.colBackgroundInfo
            style: Config.options.background.widgets.clock.cookie.dateStyle
        }
    }
    // Double tap summons Annie. Casey, 2026-08-05: *"I double tap on the clock
    // widget and I get the avatar."* The desktop clock, not the bar's — this is
    // the one on the wallpaper, on home, where she has room to stand.
    //
    // A toggle, because the same gesture has to be the way out: a presence you
    // cannot dismiss is the user's column being ignored (doctrine §13). The
    // flip/fade/puff the clock should do on the way is deliberately not here
    // yet — the trigger first, the theatre after.
    // Touching the singleton here is what constructs it. Quickshell builds
    // singletons lazily, so `Face`'s IpcHandler did not register until
    // something reached for it — `qs ipc call face status` answered "Target not
    // found" while the double tap below worked fine, because the tap was the
    // first reference. The dial and the agent both need it addressable before
    // anyone has tapped anything.
    readonly property bool faceUp: Face.joined

    // She takes this spot, so the clock gets out of it.
    //
    // `Face.rigPresent` used to be read here for the same construct-the-
    // singleton reason and **no such property exists** — the pre-flight rig
    // check was deleted from `Face.qml` (a guard that answered false for a
    // directory that was plainly there) and this reference was left behind,
    // binding to `undefined` ever since. `joined` is real, and it is also the
    // thing worth watching, so the touch and the meaning are the same read.
    opacity: root.faceUp ? 0 : 1
    scale: root.faceUp ? 0.86 : 1

    // Out faster than in. Arriving is her entrance and wants to be seen;
    // leaving is just the clock getting out of the way, and a slow fade there
    // reads as lag rather than as motion. The 0.86 shrink is the same value
    // the overview cards use for their intro, so the two pieces of theatre on
    // this device move alike rather than each inventing a number.
    Behavior on opacity {
        NumberAnimation {
            duration: root.faceUp ? 180 : 260
            easing.type: Easing.OutCubic
        }
    }
    Behavior on scale {
        NumberAnimation {
            duration: root.faceUp ? 180 : 260
            easing.type: Easing.OutCubic
        }
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        // A faded clock is not a target. Without this the way back is a double
        // tap on an invisible object, which is not a way back — it is a thing
        // you have to already know. Dismissing her is a double tap on *her*
        // instead, which is the same gesture in the same place as summoning.
        enabled: !root.faceUp
        // Above the clock's children, not below. `z: -1` was the first guess
        // and it is why the first version did nothing: the clock's own visual
        // items sit at the default z, so the handler was underneath every one
        // of them. Nothing here competes for the tap — the clock has no other
        // input at all, and never has.
        z: 1
        property real lastTapAt: -1
        onClicked: {
            const now = Date.now();
            console.log("[face] clock tap at " + now
                + " (delta " + (lastTapAt > 0 ? now - lastTapAt : -1) + ")");
            if (lastTapAt > 0 && now - lastTapAt <= 350) {
                lastTapAt = -1;
                Face.toggle();
            } else {
                lastTapAt = now;
            }
        }
    }
}
