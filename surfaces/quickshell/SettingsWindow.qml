//@ pragma UseQApplication
//@ pragma Env QS_NO_RELOAD_POPUP=1
//@ pragma Env QT_QUICK_CONTROLS_STYLE=Basic
//@ pragma Env QT_QUICK_FLICKABLE_WHEEL_DECELERATION=10000

// Souveraine settings — one ordinary app window, owned by the authoritative
// shell process. Launchers call the `settings` IPC target; they never start a
// second Quickshell configuration or a second session-authority scope.
//
// Two form factors are gated on
// Config.options.souveraine.phone (the Device tab's "Phone mode" switch):
//
//   phone === true  → full-screen portrait stack navigation
//                     (list → push page → back), finger-sized rows.
//   phone === false → a normal desktop window: a side rail of pages beside
//                     a content pane, both selecting from the SAME pages[].
//
// Both modes load identical page components (modules/settings/*Config.qml) via
// one Loader, so a page added once appears in both. Only the chrome differs.
// This replaces the old split of settings.qml (upstream desktop) +
// settings-phone.qml (our mobile) — one file is the source of truth now.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Quickshell
import Quickshell.Io
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import qs.modules.common.functions as CF

ApplicationWindow {
    id: root

    readonly property bool phoneMode: Config.options.souveraine.phone
    // -1 = list view (phone idle state). In desktop mode a page is always
    // selected, so it starts at 0.
    property int currentPage: Config.options.souveraine.phone ? -1 : 0
    readonly property real rowHeight: 68      // finger-sized tap target
    readonly property real edgeMargin: 16

    // The page set — shared by both chromes. {name, icon, component}. One page
    // per domain we own; About reuses ii's stock page verbatim.
    property var pages: [
        {
            name: Translation.tr("Device"),
            icon: "smartphone",
            component: "modules/settings/DeviceConfig.qml"
        },
        {
            name: Translation.tr("Network & internet"),
            icon: "wifi",
            component: "modules/settings/NetworkConfig.qml"
        },
        {
            name: Translation.tr("Wi-Fi"),
            icon: "network_wifi",
            component: "modules/settings/system/WifiConfig.qml"
        },
        {
            name: Translation.tr("Saved networks"),
            icon: "wifi_password",
            component: "modules/settings/system/WifiKnownConfig.qml"
        },
        {
            name: Translation.tr("Wi-Fi advanced"),
            icon: "settings_ethernet",
            component: "modules/settings/system/WifiAdvancedConfig.qml"
        },
        {
            name: Translation.tr("VPN"),
            icon: "vpn_key",
            component: "modules/settings/system/VpnConfig.qml"
        },
        {
            name: Translation.tr("Bluetooth"),
            icon: "bluetooth",
            component: "modules/settings/system/BluetoothConfig.qml"
        },
        {
            name: Translation.tr("Display"),
            icon: "display_settings",
            component: "modules/settings/DisplayConfig.qml"
        },
        {
            name: Translation.tr("Sound & microphone"),
            icon: "volume_up",
            component: "modules/settings/SoundConfig.qml"
        },
        {
            name: Translation.tr("Lock screen"),
            icon: "lock",
            component: "modules/settings/LockConfig.qml"
        },
        {
            name: Translation.tr("Wallpaper"),
            icon: "wallpaper",
            component: "modules/settings/WallpaperConfig.qml"
        },
        {
            name: Translation.tr("Home screen"),
            icon: "apps",
            component: "modules/settings/OverviewConfig.qml"
        },
        {
            name: Translation.tr("Dock"),
            icon: "dock_to_bottom",
            component: "modules/settings/DockConfig.qml"
        },
        {
            name: Translation.tr("Navigation"),
            icon: "gesture",
            component: "modules/settings/NavigationConfig.qml"
        },
        {
            name: Translation.tr("Idle & sleep"),
            icon: "bedtime",
            component: "modules/settings/IdleConfig.qml"
        },
        {
            name: Translation.tr("Keyboard"),
            icon: "keyboard",
            component: "modules/settings/KeyboardConfig.qml"
        },
        {
            name: Translation.tr("Speech"),
            icon: "mic",
            component: "modules/settings/SpeechConfig.qml"
        },
        {
            name: Translation.tr("General"),
            icon: "tune",
            component: "modules/settings/GeneralConfig.qml"
        },
        {
            name: Translation.tr("Services"),
            icon: "cloud_sync",
            component: "modules/settings/ServicesConfig.qml"
        },
        {
            name: Translation.tr("Interface"),
            icon: "palette",
            component: "modules/settings/InterfaceConfig.qml"
        },
        {
            name: Translation.tr("Quick toggles"),
            icon: "toggle_on",
            component: "modules/settings/QuickConfig.qml"
        },
        {
            name: Translation.tr("Status bar"),
            icon: "web_asset",
            component: "modules/settings/BarConfig.qml"
        },
        {
            name: Translation.tr("Background & colours"),
            icon: "format_paint",
            component: "modules/settings/BackgroundConfig.qml"
        },
        {
            name: Translation.tr("Advanced"),
            icon: "code",
            component: "modules/settings/AdvancedConfig.qml"
        },
        {
            name: Translation.tr("About"),
            icon: "info",
            component: "modules/settings/About.qml"
        }
        // Twelve of the fourteen unregistered pages, added 2026-08-16. They
        // were finished, deployed to the phone, and reachable by nothing —
        // `pages[]` is the only registry and none of them were in it. Same
        // shape as the subconscious surface stranded for a week
        // (`SHELL-SURFACES.md`) and `pose` sitting eleven days with no caller:
        // a surface that never instantiates and one that renders nothing are
        // byte-identical from outside.
        //
        // Deliberately still out:
        //   system/MonitorConfig.qml — `Quickshell.Hyprland` + `hyprctl`, which
        //     do not exist under viewtop. Needs output verbs through
        //     `ViewtopControl` first.
        //   system/KdeConfig.qml — upstream's Plasma page.
        //
        // Next: app permissions/health, storage, accounts, updates — and the
        // schema pass in SETTINGS-AUTHORITY.md, which is what stops this list
        // being hand-maintained at all.
    ]

    visible: false
    onClosing: close => {
        // Closing Settings closes this app window, not the shell that owns it.
        close.accepted = false
        root.hideSettings()
    }
    title: "Settings"
    color: Appearance.m3colors.m3background

    // Phone windows deliberately start windowed and then take the exact same
    // Hyprland fullscreen path as the navigation pill. Qt's Window.FullScreen
    // sends a client fullscreen request instead; Hyprland treats that state
    // differently from the pill's internal fullscreen dispatcher and the
    // navigation rail can disappear. Keeping one compositor-owned path also
    // means double-tapping the pill reliably returns Settings to windowed.
    width: root.phoneMode ? Screen.width : 900
    height: root.phoneMode ? Screen.height : 640
    minimumWidth: root.phoneMode ? 0 : 640
    minimumHeight: root.phoneMode ? 0 : 480

    Component.onCompleted: {
        MaterialThemeLoader.reapplyTheme()
        Config.readWriteDelay = 0
    }

    function showSettings() {
        root.visible = true
        root.raise()
        root.requestActivate()
        if (root.phoneMode) {
            phoneFullscreenRetry.attempts = 0
            phoneFullscreenRetry.start()
        }
    }

    function hideSettings() {
        root.visible = false
        if (root.phoneMode) root.currentPage = -1
        phoneFullscreenRetry.stop()
    }

    function toggleSettings() {
        if (root.visible) root.hideSettings()
        else root.showSettings()
    }

    IpcHandler {
        target: "settings"

        function open() { root.showSettings() }
        function close() { root.hideSettings() }
        function toggle() { root.toggleSettings() }
        function state() {
            return JSON.stringify({
                visible: root.visible,
                phone: root.phoneMode,
                page: root.currentPage
            })
        }
    }

    Process {
        id: phoneFullscreen
        command: [
            "hyprctl", "dispatch",
            `hl.dsp.window.fullscreen({ window = "pid:${Quickshell.processId}", mode = "fullscreen", action = "set" })`
        ]
    }

    // The QWindow maps just after Component.onCompleted. Retry briefly so a
    // cold-started settings process cannot race its own Wayland toplevel.
    Timer {
        id: phoneFullscreenRetry
        property int attempts: 0
        interval: 180
        repeat: true
        onTriggered: {
            attempts += 1
            if (root.visible && !phoneFullscreen.running)
                phoneFullscreen.running = true
            if (attempts >= 3) stop()
        }
    }

    // ====================================================================
    // PHONE MODE — full-screen stack navigation
    // ====================================================================

    // --- List view (idle state) ------------------------------------------
    ColumnLayout {
        id: listView
        anchors.fill: parent
        spacing: 0
        visible: root.phoneMode && root.currentPage < 0

        Item {
            Layout.fillWidth: true
            Layout.leftMargin: root.edgeMargin
            Layout.rightMargin: root.edgeMargin
            Layout.topMargin: root.edgeMargin + 16
            Layout.bottomMargin: 8
            implicitHeight: titleText.implicitHeight
            StyledText {
                id: titleText
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                color: Appearance.colors.colOnLayer0
                text: Translation.tr("Settings")
                font {
                    family: Appearance.font.family.title
                    pixelSize: Appearance.font.pixelSize.title
                    variableAxes: Appearance.font.variableAxes.title
                }
            }
        }

        StyledListView {
            id: pageList
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: root.pages
            delegate: RippleButton {
                required property var index
                required property var modelData
                width: pageList.width
                height: root.rowHeight
                buttonRadius: 0
                onClicked: root.currentPage = index
                contentItem: RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: root.edgeMargin
                    anchors.rightMargin: root.edgeMargin
                    spacing: 16
                    MaterialSymbol {
                        text: modelData.icon
                        iconSize: Appearance.font.pixelSize.larger
                        color: Appearance.colors.colOnLayer0
                    }
                    StyledText {
                        Layout.fillWidth: true
                        text: modelData.name
                        color: Appearance.colors.colOnLayer0
                        font.pixelSize: Appearance.font.pixelSize.larger
                    }
                    MaterialSymbol {
                        text: "chevron_right"
                        iconSize: Appearance.font.pixelSize.larger
                        color: Appearance.colors.colSubtext
                    }
                }
            }
        }
    }

    // --- Page view (drilled-in state) ------------------------------------
    Item {
        id: pageView
        anchors.fill: parent
        visible: root.phoneMode && root.currentPage >= 0

        Item {
            id: pageHeader
            anchors { top: parent.top; left: parent.left; right: parent.right }
            height: 56
            RippleButton {
                id: backButton
                anchors {
                    left: parent.left
                    leftMargin: 4
                    verticalCenter: parent.verticalCenter
                }
                buttonRadius: Appearance.rounding.full
                implicitWidth: 48
                implicitHeight: 48
                onClicked: root.currentPage = -1
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    horizontalAlignment: Text.AlignHCenter
                    text: "arrow_back"
                    iconSize: 24
                }
            }
            StyledText {
                anchors {
                    left: backButton.right
                    leftMargin: 8
                    right: parent.right
                    rightMargin: root.edgeMargin
                    verticalCenter: parent.verticalCenter
                }
                color: Appearance.colors.colOnLayer0
                text: (root.currentPage >= 0 && root.currentPage < root.pages.length)
                    ? root.pages[root.currentPage].name : ""
                elide: Text.ElideRight
                font {
                    family: Appearance.font.family.title
                    pixelSize: Appearance.font.pixelSize.larger
                }
            }
        }

        Loader {
            id: phonePageLoader
            anchors {
                top: pageHeader.bottom
                left: parent.left
                right: parent.right
                bottom: parent.bottom
            }
            active: root.phoneMode && root.currentPage >= 0 && Config.ready
            source: (root.currentPage >= 0 && root.currentPage < root.pages.length)
                ? root.pages[root.currentPage].component : ""
        }
    }

    // ====================================================================
    // DESKTOP MODE — side rail + content pane
    // ====================================================================
    RowLayout {
        id: desktopView
        anchors.fill: parent
        spacing: 0
        visible: !root.phoneMode

        // Left rail
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 220
            color: Appearance.colors.colLayer1

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                Item {
                    Layout.fillWidth: true
                    Layout.topMargin: root.edgeMargin + 8
                    Layout.leftMargin: root.edgeMargin
                    Layout.bottomMargin: 8
                    implicitHeight: railTitle.implicitHeight
                    StyledText {
                        id: railTitle
                        anchors.left: parent.left
                        text: Translation.tr("Settings")
                        color: Appearance.colors.colOnLayer1
                        font {
                            family: Appearance.font.family.title
                            pixelSize: Appearance.font.pixelSize.larger
                            variableAxes: Appearance.font.variableAxes.title
                        }
                    }
                }

                StyledListView {
                    id: railList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: root.pages
                    delegate: RippleButton {
                        required property var index
                        required property var modelData
                        width: railList.width
                        height: 44
                        buttonRadius: 0
                        toggled: root.currentPage === index
                        colBackgroundToggled: Appearance.colors.colSecondaryContainer
                        colBackgroundToggledHover: Appearance.colors.colSecondaryContainerHover
                        colRippleToggled: Appearance.colors.colSecondaryContainerActive
                        onClicked: root.currentPage = index
                        contentItem: RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: root.edgeMargin
                            anchors.rightMargin: 8
                            spacing: 12
                            MaterialSymbol {
                                text: modelData.icon
                                iconSize: Appearance.font.pixelSize.large
                                color: root.currentPage === index
                                    ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                            }
                            StyledText {
                                Layout.fillWidth: true
                                text: modelData.name
                                elide: Text.ElideRight
                                color: root.currentPage === index
                                    ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                                font.pixelSize: Appearance.font.pixelSize.normal
                            }
                        }
                    }
                }
            }
        }

        // Content pane
        Loader {
            id: desktopPageLoader
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: !root.phoneMode && root.currentPage >= 0 && Config.ready
            source: (root.currentPage >= 0 && root.currentPage < root.pages.length)
                ? root.pages[root.currentPage].component : ""
        }
    }
}
