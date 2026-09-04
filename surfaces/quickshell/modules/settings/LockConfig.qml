import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Quickshell
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Lock screen — behavior + appearance. Pure Config bindings: every control
// writes an option that already exists in Config.qml's adapter, so
// persistence + hot-apply come for free.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "lock"
        title: Translation.tr("Behavior")

        ConfigSwitch {
            buttonIcon: "pin"
            text: Translation.tr("Touch keypad (phone)")
            checked: Config.options.lock.touchKeypad
            onCheckedChanged: {
                Config.options.lock.touchKeypad = checked;
            }
            StyledToolTip {
                text: Translation.tr("Show the on-lock PIN keypad. The on-screen keyboard can't rise above a session lock, so the lock surface carries its own input.")
            }
        }

        ConfigSwitch {
            buttonIcon: "rocket_launch"
            text: Translation.tr("Launch lock on startup")
            checked: Config.options.lock.launchOnStartup
            onCheckedChanged: {
                Config.options.lock.launchOnStartup = checked;
            }
            StyledToolTip {
                text: Translation.tr("Start the session locked so a PIN is required before the shell is exposed.")
            }
        }

        ConfigSwitch {
            buttonIcon: "key_off"
            text: Translation.tr("Require password to power off")
            checked: Config.options.lock.security.requirePasswordToPower
            onCheckedChanged: {
                Config.options.lock.security.requirePasswordToPower = checked;
            }
            StyledToolTip {
                text: Translation.tr("Guard the power menu behind the lock so the device can't be silenced without a PIN.")
            }
        }

        ConfigSwitch {
            buttonIcon: "power_settings_new"
            text: Translation.tr("Allow power off / reboot from lock screen")
            checked: Config.options.lock.security.allowPowerFromLock
            onCheckedChanged: {
                Config.options.lock.security.allowPowerFromLock = checked;
            }
            StyledToolTip {
                text: Translation.tr("Show the power and reboot buttons on the lock screen. Off by default — a destructive action from the locked surface is opt-in. The buttons still respect “require password to power off” above.")
            }
        }

        ConfigSwitch {
            buttonIcon: "vpn_key"
            text: Translation.tr("Unlock keyring on PIN unlock")
            checked: Config.options.lock.security.unlockKeyring
            onCheckedChanged: {
                Config.options.lock.security.unlockKeyring = checked;
            }
            StyledToolTip {
                text: Translation.tr("Feed the PIN to the keyring so stored secrets unlock together with the session.")
            }
        }
    }

    ContentSection {
        icon: "palette"
        title: Translation.tr("Appearance")

        ConfigSwitch {
            buttonIcon: "format_align_center"
            text: Translation.tr("Center the clock")
            checked: Config.options.lock.centerClock
            onCheckedChanged: {
                Config.options.lock.centerClock = checked;
            }
        }

        ConfigSwitch {
            buttonIcon: "schedule"
            text: Translation.tr("12-hour clock (am/pm)")
            checked: Config.options.lock.twelveHourClock
            onCheckedChanged: {
                Config.options.lock.twelveHourClock = checked;
            }
        }

        ConfigSwitch {
            buttonIcon: "text_fields"
            text: Translation.tr("Show locked text")
            checked: Config.options.lock.showLockedText
            onCheckedChanged: {
                Config.options.lock.showLockedText = checked;
            }
        }

        ConfigSwitch {
            buttonIcon: "blur_on"
            text: Translation.tr("Blur background")
            checked: Config.options.lock.blur.enable
            onCheckedChanged: {
                Config.options.lock.blur.enable = checked;
            }
        }

        ConfigSwitch {
            buttonIcon: "category"
            text: Translation.tr("Material shapes for PIN dots")
            checked: Config.options.lock.materialShapeChars
            onCheckedChanged: {
                Config.options.lock.materialShapeChars = checked;
            }
        }
    }

    ContentSection {
        id: lockWallSection
        icon: "wallpaper"
        title: Translation.tr("Lock screen wallpaper")

        readonly property string wallpaperDir: Quickshell.env("HOME") + "/Pictures/Wallpapers"

        ConfigSwitch {
            buttonIcon: "sync"
            text: Translation.tr("Follow system wallpaper")
            checked: !Config.options.lock.wallpaperPath
            onCheckedChanged: {
                if (checked) Config.options.lock.wallpaperPath = "";
            }
            StyledToolTip {
                text: Translation.tr("The lock screen shows the same wallpaper as the shell. Pick an image below to pin the lock screen's own.")
            }
        }

        GridView {
            id: lockWallGrid
            Layout.fillWidth: true
            readonly property int cols: 3
            cellWidth: Math.floor(width / cols)
            cellHeight: Math.floor(cellWidth * 2)
            implicitHeight: Math.ceil(lockWallModel.count / cols) * cellHeight
            interactive: false
            clip: true

            model: FolderListModel {
                id: lockWallModel
                folder: "file://" + lockWallSection.wallpaperDir
                nameFilters: ["*.jpg", "*.jpeg", "*.png", "*.webp"]
                showDirs: false
            }

            delegate: Item {
                required property string filePath
                width: lockWallGrid.cellWidth
                height: lockWallGrid.cellHeight

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 5
                    radius: Appearance.rounding.small
                    color: Appearance.colors.colLayer1
                    border.width: Config.options.lock.wallpaperPath === filePath ? 3 : 0
                    border.color: Appearance.colors.colPrimary
                    clip: true

                    Image {
                        anchors.fill: parent
                        anchors.margins: 3
                        source: "file://" + filePath
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        sourceSize.width: 240
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: Config.options.lock.wallpaperPath = filePath
                    }
                }
            }
        }
    }

    ContentSection {
        icon: "visibility"
        title: Translation.tr("Lock screen content")

        ConfigSwitch {
            buttonIcon: "music_note"
            text: Translation.tr("Show media controls")
            checked: Config.options.lock.content.showMediaControls
            onCheckedChanged: Config.options.lock.content.showMediaControls = checked
            StyledToolTip {
                text: Translation.tr("Shows previous, play/pause, and next. Track details remain private unless enabled below.")
            }
        }

        ConfigSwitch {
            buttonIcon: "visibility"
            text: Translation.tr("Show media title and artist")
            checked: Config.options.lock.content.mediaMetadataAmbient
            enabled: Config.options.lock.content.showMediaControls
            onCheckedChanged: Config.options.lock.content.mediaMetadataAmbient = checked
            StyledToolTip {
                text: Translation.tr("Treats current media metadata as ambient. Leave off to keep it hidden until unlock.")
            }
        }

        ConfigSwitch {
            buttonIcon: "battery_android_full"
            text: Translation.tr("Show battery on lock screen")
            checked: Config.options.lock.content.showBattery
            onCheckedChanged: Config.options.lock.content.showBattery = checked
        }

        ConfigSwitch {
            buttonIcon: "notifications"
            text: Translation.tr("Show notifications while locked")
            checked: Config.options.lock.content.showNotifications
            onCheckedChanged: Config.options.lock.content.showNotifications = checked
            StyledToolTip {
                text: Translation.tr("Shows which apps had notifications arrive and how many. Content stays private unless enabled below.")
            }
        }

        ConfigSwitch {
            buttonIcon: "visibility"
            text: Translation.tr("Show notification summaries")
            checked: Config.options.lock.content.notificationContentAmbient
            enabled: Config.options.lock.content.showNotifications
            onCheckedChanged: Config.options.lock.content.notificationContentAmbient = checked
            StyledToolTip {
                text: Translation.tr("Treats the notification title line as ambient. Bodies never show on the lock screen.")
            }
        }
    }

    ContentSection {
        icon: "key"
        title: Translation.tr("Step-up authentication")

        ConfigSwitch {
            buttonIcon: "shield_lock"
            text: Translation.tr("Enable step-up authentication")
            checked: Config.options.lock.stepUp.enabled
            onCheckedChanged: Config.options.lock.stepUp.enabled = checked
            StyledToolTip {
                text: Translation.tr("Require re-authentication for sensitive operations (send, delete, payment). Requires the souveraine-stepup PAM service to be installed on the system.")
            }
        }

        ConfigSpinBox {
            icon: "timer"
            text: Translation.tr("Grant validity (seconds)")
            value: Config.options.lock.stepUp.grantTtlMs / 1000
            from: 30
            to: 3600
            stepSize: 30
            enabled: Config.options.lock.stepUp.enabled
            onValueChanged: Config.options.lock.stepUp.grantTtlMs = value * 1000
            StyledToolTip {
                text: Translation.tr("How long a step-up grant remains valid after authentication. The user can perform sensitive operations within this window without re-authenticating.")
            }
        }
    }

    ContentSection {
        icon: "fingerprint"
        title: Translation.tr("Fingerprint integration")

        ConfigSwitch {
            buttonIcon: "fingerprint"
            text: Translation.tr("Show fingerprint wiring preview")
            checked: Config.options.lock.fingerprintPreview.enabled
            onCheckedChanged: Config.options.lock.fingerprintPreview.enabled = checked
            StyledToolTip {
                text: Translation.tr("Shows a three-second hold test on the lock screen. It never unlocks the device; post-login Polkit is configured separately below.")
            }
        }

        ConfigSpinBox {
            icon: "timer"
            text: Translation.tr("Preview hold (seconds)")
            value: Config.options.lock.fingerprintPreview.holdMs / 1000
            from: 1
            to: 10
            stepSize: 1
            enabled: Config.options.lock.fingerprintPreview.enabled
            onValueChanged: Config.options.lock.fingerprintPreview.holdMs = value * 1000
            StyledToolTip {
                text: Translation.tr("This controls the visible lock-screen exercise only. It does not change the temporary post-login Polkit factor.")
            }
        }

        ConfigSwitch {
            buttonIcon: "admin_panel_settings"
            text: Translation.tr("Allow temporary FPC confirmation for Polkit")
            checked: Config.options.lock.fingerprintPolkit.enabled
            onCheckedChanged: Config.options.lock.fingerprintPolkit.enabled = checked
            StyledToolTip {
                text: Translation.tr("After PIN login, user-facing Polkit prompts may accept a fresh reader assertion after the visible confirmation interval. It never unlocks the session or replaces first-login PIN.")
            }
        }
    }
}
