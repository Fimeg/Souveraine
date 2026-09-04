import qs.modules.ii.bar.weather
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Services.UPower
import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions

// BarContent — ONE bar for every device. Souveraine-owned override of the ii
// original, replacing BOTH ii-base/modules/ii/bar/BarContent.qml and the
// ii-phone fork of the same file.
//
// WHY THIS FILE EXISTS
// ii-base and ii-phone each carried a 357-line BarContent to express exactly
// eight differences, and every one of the eight was either "is this widget
// shown" or "which slot is it in". No behaviour differed. A 357-line fork
// maintained against a pin, forever, to reorder three widgets — so every
// future bar edit had to be made twice or silently diverge.
//
// The eight, for the record (ii-base -> ii-phone):
//   1. cellular carrier readout added, far left
//   2. leftCenterGroup (resources/media/claudeUsage) removed entirely
//   3. clock moved to the middle group
//   4. workspaces moved to the right-of-centre group
//   5. battery moved out of the centre group to the right section, ungated
//   6. resources added to the right section
//   7. xkb + bluetooth indicators dropped from the pill
//   8. pomodoro dropped from the right section
//
// HOW IT REPLACES THEM
// Widgets are declared once as Components in the registry below. Each slot is
// a Repeater over a list of widget NAMES, so placement and order are data.
// Config wins if it names a slot; otherwise the slot comes from a device
// profile. An unknown name loads nothing, so ["none"] is how a slot is
// deliberately emptied, and a typo degrades to a gap rather than an error.
//
// Loaders are `active` only when their name is listed, so an unplaced widget
// is never constructed — placement is also the lazy-loading boundary.
//
// DEVICE TYPES ARE STILL REAL
// "auto" derives the profile from the same cramped-ness test the bar already
// uses for useShortenedForm, so the phone keeps its current arrangement with
// no config file at all, and a narrow bar behaves like a narrow bar wherever
// it appears. Setting bar.layout.profile or naming slots overrides it — which
// is what makes this reachable from souveraine-settings instead of from a
// second copy of the file.
Item { // Bar content region
    id: root

    property var screen: root.QsWindow.window?.screen
    property var brightnessMonitor: Brightness.getMonitorForScreen(screen)
    property real useShortenedForm: (Appearance.sizes.barHellaShortenScreenWidthThreshold >= screen?.width) ? 2 : (Appearance.sizes.barShortenScreenWidthThreshold >= screen?.width) ? 1 : 0
    readonly property int centerSideModuleWidth: (useShortenedForm == 2) ? Appearance.sizes.barCenterSideModuleWidthHellaShortened : (useShortenedForm == 1) ? Appearance.sizes.barCenterSideModuleWidthShortened : Appearance.sizes.barCenterSideModuleWidth

    // ── Layout resolution ────────────────────────────────────────────────
    // One authority for "is this bar cramped": the same threshold that drives
    // useShortenedForm. The island asks the same question independently and
    // gets the same answer, so it narrows when its neighbours do.
    readonly property string profile: {
        const p = Config.options.bar.layout?.profile ?? "auto";
        if (p !== "auto")
            return p;
        return root.useShortenedForm >= 1 ? "compact" : "desktop";
    }

    // Profile defaults. `desktop` is the ii-base arrangement verbatim;
    // `compact` is the ii-phone arrangement verbatim. Changing a bar layout
    // is now editing a list here (or in config), not forking a file.
    readonly property var layoutDefaults: ({
            "desktop": {
                "left": ["activeWindow"],
                "centerLeft": ["resources", "media", "claudeUsage"],
                "centerMiddle": ["workspaces"],
                "centerRight": ["clock", "utilButtons", "battery"],
                "right": ["pomodoro", "systray"]
            },
            "compact": {
                "left": ["cellular", "activeWindow"],
                "centerLeft": ["none"],
                "centerMiddle": ["clock"],
                "centerRight": ["workspaces", "utilButtons"],
                "right": ["battery", "resources", "systray"]
            }
        })

    // Config names a slot -> config wins. Otherwise the profile default.
    // An empty config list means "unset", not "empty"; use ["none"] to empty.
    function slot(name) {
        const cfg = Config.options.bar.layout?.[name] ?? null;
        if (cfg && cfg.length > 0)
            return cfg;
        const prof = root.layoutDefaults[root.profile] ?? root.layoutDefaults["desktop"];
        return prof[name] ?? [];
    }

    function slotHas(name, widget) {
        return root.slot(name).indexOf(widget) !== -1;
    }

    // Layout hints belong to the Loader, not the loaded item: an item inside a
    // Loader inside a layout has its own Layout.* ignored. So the few widgets
    // that stretch declare it here, by name.
    function fillWidthFor(name) {
        if (name === "resources")
            return root.useShortenedForm === 2;
        if (name === "media" || name === "clock" || name === "activeWindow")
            return true;
        return false;
    }
    function fillHeightFor(name) {
        return name === "workspaces" || name === "systray" || name === "activeWindow";
    }

    // The registry. Every bar widget, declared exactly once.
    readonly property var widgets: ({
            "activeWindow": activeWindowComp,
            "cellular": cellularComp,
            "resources": resourcesComp,
            "media": mediaComp,
            "claudeUsage": claudeUsageComp,
            "workspaces": workspacesComp,
            "clock": clockComp,
            "utilButtons": utilButtonsComp,
            "battery": batteryComp,
            "pomodoro": pomodoroComp,
            "systray": systrayComp
        })

    component VerticalBarSeparator: Rectangle {
        Layout.topMargin: Appearance.sizes.baseBarHeight / 3
        Layout.bottomMargin: Appearance.sizes.baseBarHeight / 3
        Layout.fillHeight: true
        implicitWidth: 1
        color: Appearance.colors.colOutlineVariant
    }

    // A slot: order and membership from config, construction gated on
    // placement so an unplaced widget costs nothing.
    component WidgetSlot: Repeater {
        required property string slotName
        model: root.slot(slotName)
        delegate: Loader {
            required property var modelData
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: root.fillWidthFor(modelData)
            Layout.fillHeight: root.fillHeightFor(modelData)
            active: !!root.widgets[modelData]
            visible: active
            sourceComponent: root.widgets[modelData] ?? null
        }
    }

    // ── Widget registry ──────────────────────────────────────────────────
    Component {
        id: activeWindowComp
        ActiveWindow {
            Layout.leftMargin: 10 + (leftSidebarButton.visible ? 0 : Appearance.rounding.screenRounding)
            Layout.rightMargin: Appearance.rounding.screenRounding
            visible: root.useShortenedForm === 0
        }
    }

    // Phone-only in practice, but not phone-*gated*: Cellular.available is
    // false where there is no modem, so the desktop needs no special case.
    Component {
        id: cellularComp
        RowLayout {
            spacing: 4
            visible: Cellular.available
            MaterialSymbol {
                Layout.alignment: Qt.AlignVCenter
                text: Cellular.materialSymbol
                iconSize: Appearance.font.pixelSize.larger
                color: Appearance.colors.colOnLayer0
            }
            StyledText {
                Layout.alignment: Qt.AlignVCenter
                text: (Cellular.operatorName + " " + Cellular.accessTech).trim()
                color: Appearance.colors.colOnLayer0
                font.pixelSize: Appearance.font.pixelSize.normal
            }
        }
    }

    Component {
        id: resourcesComp
        Resources {
            autoRotate: root.profile === "compact"
            alwaysShowAllResources: root.useShortenedForm === 2
        }
    }

    Component {
        id: mediaComp
        Media {
            visible: root.useShortenedForm < 2
        }
    }

    Component {
        id: claudeUsageComp
        Loader {
            active: Config.options.bar.claudeUsage.enable
            visible: root.useShortenedForm < 2 && active
            sourceComponent: ClaudeUsageBar {}
        }
    }

    Component {
        id: workspacesComp
        Workspaces {
            id: workspacesWidget
            MouseArea {
                // Right-click to toggle overview
                anchors.fill: parent
                acceptedButtons: Qt.RightButton

                onPressed: event => {
                    if (event.button === Qt.RightButton) {
                        GlobalStates.overviewOpen = !GlobalStates.overviewOpen;
                    }
                }
            }
        }
    }

    // The compact profile wants a dense one-line clock; the desktop follows
    // bar.verbose as before. Format is config-overridable for either.
    Component {
        id: clockComp
        ClockWidget {
            showDate: root.profile === "compact" ? false : (Config.options.bar.verbose && root.useShortenedForm < 2)
            customFormat: Config.options.bar.clock?.format || (root.profile === "compact" ? "ddd. dd/MM h:mmAP" : "")
        }
    }

    Component {
        id: utilButtonsComp
        UtilButtons {
            visible: root.profile === "compact" ? true : (Config.options.bar.verbose && root.useShortenedForm === 0)
        }
    }

    Component {
        id: batteryComp
        // Ungated under `compact`: ii-phone showed the battery at every width
        // (note 5 in the header), and the unification applied the desktop gate
        // to every device, so a phone at useShortenedForm 2 lost its icon
        // silently while the critical-battery alert kept firing. 2026-08-13.
        BatteryIndicator {
            visible: Battery.available && (root.profile === "compact" || root.useShortenedForm < 2)
        }
    }

    Component {
        id: pomodoroComp
        // The child is deliberately NOT anchored to the Revealer's centre.
        // Revealer takes implicitHeight from childrenRect, so a child anchored
        // to its parent closes a real cycle once the Revealer sits in a Loader
        // (implicitHeight -> height -> child.y -> childrenRect -> ...). ii-base
        // hid it by letting the layout drive the Revealer's height directly.
        // The indicator has a fixed implicitHeight and the Loader carries
        // Layout.alignment, so it centres without the anchor.
        Revealer {
            reveal: TimerService.pomodoroRunning
            PomodoroBarIndicator {}
        }
    }

    Component {
        id: systrayComp
        SysTray {
            visible: root.useShortenedForm === 0
            invertSide: Config?.options.bar.bottom
        }
    }

    // ── Structure ────────────────────────────────────────────────────────
    // Background shadow
    Loader {
        active: Config.options.bar.showBackground && Config.options.bar.cornerStyle === 1 && Config.options.bar.floatStyleShadow
        anchors.fill: barBackground
        sourceComponent: StyledRectangularShadow {
            anchors.fill: undefined // The loader's anchors act on this, and this should not have any anchor
            target: barBackground
        }
    }
    // Background
    Rectangle {
        id: barBackground
        anchors {
            fill: parent
            margins: Config.options.bar.cornerStyle === 1 ? (Appearance.sizes.hyprlandGapsOut) : 0 // idk why but +1 is needed
        }
        color: Config.options.bar.showBackground ? Appearance.colors.colLayer0 : "transparent"
        radius: Config.options.bar.cornerStyle === 1 ? Appearance.rounding.windowRounding : 0
        border.width: Config.options.bar.cornerStyle === 1 ? 1 : 0
        border.color: Appearance.colors.colLayer0Border
    }

    FocusedScrollMouseArea { // Left side | scroll to change brightness
        id: barLeftSideMouseArea

        anchors {
            top: parent.top
            bottom: parent.bottom
            left: parent.left
            right: middleSection.left
        }
        implicitWidth: leftSectionRowLayout.implicitWidth
        implicitHeight: Appearance.sizes.baseBarHeight

        onScrollDown: Brightness.decreaseBrightness()
        onScrollUp: Brightness.increaseBrightness()
        onMovedAway: GlobalStates.osdBrightnessOpen = false
        onPressed: event => {
            if (event.button === Qt.LeftButton)
                GlobalStates.sidebarLeftOpen = !GlobalStates.sidebarLeftOpen;
        }

        // Visual content
        ScrollHint {
            reveal: barLeftSideMouseArea.hovered
            icon: Hyprsunset.gamma === 100 ? "light_mode" : "wb_twilight"
            tooltipText: Translation.tr("Scroll to change brightness")
            side: "left"
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
        }

        RowLayout {
            id: leftSectionRowLayout
            anchors.fill: parent
            spacing: 0

            LeftSidebarButton { // Left sidebar button
                id: leftSidebarButton
                Layout.alignment: Qt.AlignVCenter
                Layout.leftMargin: Appearance.rounding.screenRounding
                colBackground: barLeftSideMouseArea.hovered ? Appearance.colors.colLayer1Hover : ColorUtils.transparentize(Appearance.colors.colLayer1Hover, 1)
            }

            WidgetSlot {
                slotName: "left"
            }
        }
    }

    Row { // Middle section
        id: middleSection
        anchors {
            top: parent.top
            bottom: parent.bottom
            horizontalCenter: parent.horizontalCenter
        }
        spacing: 4

        BarGroup {
            id: leftCenterGroup
            anchors.verticalCenter: parent.verticalCenter
            visible: root.slot("centerLeft").length > 0 && implicitWidth > padding * 2

            WidgetSlot {
                slotName: "centerLeft"
            }
        }

        VerticalBarSeparator {
            visible: (Config.options?.bar.borderless ?? false) && leftCenterGroup.visible
        }

        BarGroup {
            id: middleCenterGroup
            anchors.verticalCenter: parent.verticalCenter
            // Workspaces sets its own padding wherever it lands.
            padding: root.slotHas("centerMiddle", "workspaces") ? 2 : 5
            visible: root.slot("centerMiddle").length > 0 && implicitWidth > padding * 2

            WidgetSlot {
                slotName: "centerMiddle"
            }
        }

        VerticalBarSeparator {
            visible: (Config.options?.bar.borderless ?? false) && middleCenterGroup.visible
        }

        MouseArea {
            id: rightCenterGroup
            anchors.verticalCenter: parent.verticalCenter
            implicitWidth: rightCenterGroupContent.implicitWidth
            implicitHeight: rightCenterGroupContent.implicitHeight

            onPressed: {
                GlobalStates.sidebarRightOpen = !GlobalStates.sidebarRightOpen;
            }

            BarGroup {
                id: rightCenterGroupContent
                anchors.fill: parent
                padding: root.slotHas("centerRight", "workspaces") ? 2 : 5

                WidgetSlot {
                    slotName: "centerRight"
                }
            }
        }
    }

    FocusedScrollMouseArea { // Right side | scroll to change volume
        id: barRightSideMouseArea

        anchors {
            top: parent.top
            bottom: parent.bottom
            left: middleSection.right
            right: parent.right
        }
        implicitWidth: rightSectionRowLayout.implicitWidth
        implicitHeight: Appearance.sizes.baseBarHeight

        onScrollDown: Audio.decrementVolume()
        onScrollUp: Audio.incrementVolume()
        onMovedAway: GlobalStates.osdVolumeOpen = false
        onPressed: event => {
            if (event.button === Qt.LeftButton) {
                GlobalStates.sidebarRightOpen = !GlobalStates.sidebarRightOpen;
            }
        }

        // Visual content
        ScrollHint {
            reveal: barRightSideMouseArea.hovered
            icon: "volume_up"
            tooltipText: Translation.tr("Scroll to change volume")
            side: "right"
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
        }

        RowLayout {
            id: rightSectionRowLayout
            anchors.fill: parent
            spacing: 5
            layoutDirection: Qt.RightToLeft

            RippleButton { // Right sidebar button
                id: rightSidebarButton

                Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                Layout.rightMargin: Appearance.rounding.screenRounding
                Layout.fillWidth: false

                implicitWidth: indicatorsRowLayout.implicitWidth + 10 * 2
                implicitHeight: indicatorsRowLayout.implicitHeight + 5 * 2

                buttonRadius: Appearance.rounding.full
                colBackground: barRightSideMouseArea.hovered ? Appearance.colors.colLayer1Hover : ColorUtils.transparentize(Appearance.colors.colLayer1Hover, 1)
                colBackgroundHover: Appearance.colors.colLayer1Hover
                colRipple: Appearance.colors.colLayer1Active
                colBackgroundToggled: Appearance.colors.colSecondaryContainer
                colBackgroundToggledHover: Appearance.colors.colSecondaryContainerHover
                colRippleToggled: Appearance.colors.colSecondaryContainerActive
                toggled: GlobalStates.sidebarRightOpen
                property color colText: toggled ? Appearance.m3colors.m3onSecondaryContainer : Appearance.colors.colOnLayer0

                Behavior on colText {
                    animation: Appearance.animation.elementMoveFast.colorAnimation.createObject(this)
                }

                onPressed: {
                    GlobalStates.sidebarRightOpen = !GlobalStates.sidebarRightOpen;
                }

                // The indicator pill stays hand-ordered rather than
                // slot-driven: these Revealers interlock through
                // realSpacing margins that depend on their neighbours'
                // reveal state, and a Repeater would have to reproduce that
                // coupling to gain an ordering nobody has asked to change.
                // Visibility is config, which is the whole delta that
                // existed between the two forks.
                RowLayout {
                    id: indicatorsRowLayout
                    anchors.centerIn: parent
                    property real realSpacing: 15
                    spacing: 0

                    Revealer {
                        reveal: Audio.sink?.audio?.muted ?? false
                        Layout.fillHeight: true
                        Layout.rightMargin: reveal ? indicatorsRowLayout.realSpacing : 0
                        Behavior on Layout.rightMargin {
                            animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
                        }
                        MaterialSymbol {
                            text: "volume_off"
                            iconSize: Appearance.font.pixelSize.larger
                            color: rightSidebarButton.colText
                        }
                    }
                    Revealer {
                        reveal: Audio.source?.audio?.muted ?? false
                        Layout.fillHeight: true
                        Layout.rightMargin: reveal ? indicatorsRowLayout.realSpacing : 0
                        Behavior on Layout.rightMargin {
                            animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
                        }
                        MaterialSymbol {
                            text: "mic_off"
                            iconSize: Appearance.font.pixelSize.larger
                            color: rightSidebarButton.colText
                        }
                    }
                    Loader {
                        active: Config.tristate(Config.options.bar.indicators?.showXkb, root.profile !== "compact")
                        visible: active
                        Layout.alignment: Qt.AlignVCenter
                        Layout.rightMargin: indicatorsRowLayout.realSpacing
                        sourceComponent: HyprlandXkbIndicator {
                            color: rightSidebarButton.colText
                        }
                    }
                    Revealer {
                        reveal: Notifications.silent || Notifications.unread > 0
                        Layout.fillHeight: true
                        Layout.rightMargin: reveal ? indicatorsRowLayout.realSpacing : 0
                        implicitHeight: reveal ? notificationUnreadCount.implicitHeight : 0
                        implicitWidth: reveal ? notificationUnreadCount.implicitWidth : 0
                        Behavior on Layout.rightMargin {
                            animation: Appearance.animation.elementMoveFast.numberAnimation.createObject(this)
                        }
                        NotificationUnreadCount {
                            id: notificationUnreadCount
                        }
                    }
                    // On a compact bar this is the pill's always-visible face
                    // (the carrier readout holds the far left), so it fills
                    // height there; on the desktop it sits inline as before.
                    MaterialSymbol {
                        Layout.fillHeight: root.profile === "compact"
                        Layout.alignment: Qt.AlignVCenter
                        text: Network.materialSymbol
                        iconSize: Appearance.font.pixelSize.larger
                        color: rightSidebarButton.colText
                    }
                    Loader {
                        active: Config.tristate(Config.options.bar.indicators?.showBluetooth, root.profile !== "compact") && BluetoothStatus.available
                        visible: active
                        Layout.leftMargin: indicatorsRowLayout.realSpacing
                        sourceComponent: MaterialSymbol {
                            text: BluetoothStatus.connected ? "bluetooth_connected" : BluetoothStatus.enabled ? "bluetooth" : "bluetooth_disabled"
                            iconSize: Appearance.font.pixelSize.larger
                            color: rightSidebarButton.colText
                        }
                    }
                }
            }

            WidgetSlot {
                slotName: "right"
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            // Weather
            Loader {
                Layout.leftMargin: 4
                active: Config.options.bar.weather.enable

                sourceComponent: BarGroup {
                    WeatherBar {}
                }
            }
        }
    }
}
