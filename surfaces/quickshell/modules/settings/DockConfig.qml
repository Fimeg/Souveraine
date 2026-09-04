import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Widgets
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Dock — every configurable the dock components read. Feel knobs are the
// single source (components read Config, never hardcode); stacks and pins
// are managed by drag on the dock itself — an editor lands here later.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "dock_to_bottom"
        title: Translation.tr("Behavior")

        ConfigSwitch {
            buttonIcon: "dock_to_bottom"
            text: Translation.tr("Enable dock")
            checked: Config.options.dock.enable
            onCheckedChanged: {
                Config.options.dock.enable = checked;
            }
            StyledToolTip {
                text: Translation.tr("The dock surface itself. This settings app runs standalone, so it can always re-enable it.")
            }
        }

        ConfigSwitch {
            buttonIcon: "bottom_panel_open"
            text: Translation.tr("Auto-hide on Home")
            checked: Config.options.dock.autoHide
            onCheckedChanged: {
                Config.options.dock.autoHide = checked;
            }
            StyledToolTip {
                text: Translation.tr("Hide the Home dock until the pointer reaches the bottom edge.")
            }
        }

        ConfigSwitch {
            buttonIcon: "push_pin"
            text: Translation.tr("Reserve screen space")
            checked: Config.options.dock.pinnedOnStartup
            enabled: !Config.options.dock.autoHide
            onCheckedChanged: {
                Config.options.dock.pinnedOnStartup = checked;
            }
            StyledToolTip {
                text: Translation.tr("Keep normal windows above the visible dock instead of allowing them behind it.")
            }
        }

        ConfigSwitch {
            buttonIcon: "swipe_up"
            text: Translation.tr("Reveal from other zones")
            checked: Config.options.dock.hoverToReveal
            onCheckedChanged: {
                Config.options.dock.hoverToReveal = checked;
            }
            StyledToolTip {
                text: Translation.tr("Keep a bottom-edge reveal strip available when the dock is not Home furniture.")
            }
        }

        ConfigSpinBox {
            icon: "hourglass_top"
            text: Translation.tr("Reveal delay (ms)")
            value: Config.options.dock.revealDelayMs
            from: 0
            to: 1000
            stepSize: 25
            onValueChanged: {
                Config.options.dock.revealDelayMs = value;
            }
        }

        ConfigSpinBox {
            icon: "hourglass_bottom"
            text: Translation.tr("Hide delay (ms)")
            value: Config.options.dock.hideDelayMs
            from: 0
            to: 2000
            stepSize: 25
            onValueChanged: {
                Config.options.dock.hideDelayMs = value;
            }
        }

        ConfigSpinBox {
            icon: "timer"
            text: Translation.tr("Drag dwell (ms)")
            value: Config.options.dock.dragDwellMs
            from: 100
            to: 2000
            stepSize: 50
            onValueChanged: {
                Config.options.dock.dragDwellMs = value;
            }
            StyledToolTip {
                text: Translation.tr("How long a drag hovers an icon or stack before it reads as combine-intent.")
            }
        }
    }

    ContentSection {
        icon: "palette"
        title: Translation.tr("Appearance")

        ConfigSwitch {
            buttonIcon: "filter_b_and_w"
            text: Translation.tr("Monochrome icons")
            checked: Config.options.dock.monochromeIcons
            onCheckedChanged: {
                Config.options.dock.monochromeIcons = checked;
            }
        }

        // The dock's own scale. Two numbers, because the button is the row's
        // height and the icon is what you actually see — sizing one from the
        // other would mean either cramped icons in a tall row or icons
        // overflowing a short one, depending on which way the ratio was fixed.
        ConfigSpinBox {
            icon: "aspect_ratio"
            text: Translation.tr("Button size (px)")
            value: Config.options.dock.buttonSize
            from: 36
            to: 88
            stepSize: 2
            onValueChanged: {
                Config.options.dock.buttonSize = value;
            }
        }

        ConfigSpinBox {
            icon: "apps"
            text: Translation.tr("Icon size (px)")
            value: Config.options.dock.iconSize
            from: 24
            to: 72
            stepSize: 2
            onValueChanged: {
                Config.options.dock.iconSize = value;
            }
        }

        ConfigSpinBox {
            icon: "height"
            text: Translation.tr("Dock height (px)")
            value: Config.options.dock.height
            from: 40
            to: 120
            stepSize: 2
            onValueChanged: {
                Config.options.dock.height = value;
            }
        }

        ConfigSpinBox {
            icon: "expand"
            text: Translation.tr("Reveal region height (px)")
            value: Config.options.dock.hoverRegionHeight
            from: 1
            to: 20
            stepSize: 1
            onValueChanged: {
                Config.options.dock.hoverRegionHeight = value;
            }
            StyledToolTip {
                text: Translation.tr("Height of the invisible bottom strip that triggers hover-reveal.")
            }
        }
    }

    ContentSection {
        icon: "push_pin"
        title: Translation.tr("Pinned apps")

        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("Drag on the dock is the primary way to manage these (drag onto an icon to stack, drag a member off the arc to split). This list is the fallback editor.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        // One row per pinned app. Model binds the raw config list so writes
        // (from here, the dock, or the agent) re-render immediately.
        Repeater {
            model: Config.options.dock.pinnedApps
            delegate: RowLayout {
                required property string modelData
                Layout.fillWidth: true
                spacing: 8

                IconImage {
                    implicitSize: 24
                    source: Quickshell.iconPath(AppSearch.guessIcon(modelData), "image-missing")
                }
                StyledText {
                    Layout.fillWidth: true
                    text: modelData
                    elide: Text.ElideMiddle
                    color: Appearance.m3colors.m3onSurface
                    font.pixelSize: Appearance.font.pixelSize.small
                }
                RippleButton {
                    implicitWidth: 32
                    implicitHeight: 32
                    onClicked: TaskbarApps.togglePin(modelData)
                    contentItem: MaterialSymbol {
                        anchors.centerIn: parent
                        horizontalAlignment: Text.AlignHCenter
                        text: "close"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.m3colors.m3onSurface
                    }
                    StyledToolTip { text: Translation.tr("Unpin") }
                }
            }
        }

        StyledText {
            visible: (Config.options.dock.pinnedApps?.length ?? 0) === 0
            text: Translation.tr("Nothing pinned. Long-press a running app on the dock to pin it.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
        }
    }

    ContentSection {
        icon: "stacks"
        title: Translation.tr("Stacks")

        // One block per stack: editable name, then member rows. All edits go
        // through TaskbarApps so the rules (id never changes, empty stacks
        // dissolve, unstacked members re-pin) live in one place.
        Repeater {
            model: Config.options.dock.stacks
            delegate: ColumnLayout {
                id: stackBlock
                required property string modelData
                readonly property var stack: TaskbarApps.parseStack(modelData)
                Layout.fillWidth: true
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    MaterialSymbol {
                        text: "stacks"
                        iconSize: Appearance.font.pixelSize.large
                        color: Appearance.m3colors.m3onSurface
                    }
                    MaterialTextField {
                        Layout.fillWidth: true
                        text: stackBlock.stack.name
                        placeholderText: Translation.tr("Stack name")
                        onEditingFinished: {
                            if (text.length > 0 && text !== stackBlock.stack.name)
                                TaskbarApps.renameStack(stackBlock.stack.id, text);
                        }
                    }
                    RippleButton {
                        implicitWidth: 32
                        implicitHeight: 32
                        onClicked: {
                            // Dissolve: every member goes back to a pin.
                            for (const m of stackBlock.stack.members.slice())
                                TaskbarApps.unstackMember(stackBlock.stack.id, m);
                        }
                        contentItem: MaterialSymbol {
                            anchors.centerIn: parent
                            horizontalAlignment: Text.AlignHCenter
                            text: "delete"
                            iconSize: Appearance.font.pixelSize.normal
                            color: Appearance.m3colors.m3onSurface
                        }
                        StyledToolTip { text: Translation.tr("Dissolve stack (members become pins)") }
                    }
                }

                Repeater {
                    model: stackBlock.stack.members
                    delegate: RowLayout {
                        required property string modelData
                        Layout.fillWidth: true
                        Layout.leftMargin: 28
                        spacing: 8

                        IconImage {
                            implicitSize: 22
                            source: Quickshell.iconPath(AppSearch.guessIcon(modelData), "image-missing")
                        }
                        StyledText {
                            Layout.fillWidth: true
                            text: modelData
                            elide: Text.ElideMiddle
                            color: Appearance.m3colors.m3onSurface
                            font.pixelSize: Appearance.font.pixelSize.small
                        }
                        RippleButton {
                            implicitWidth: 32
                            implicitHeight: 32
                            onClicked: TaskbarApps.unstackMember(stackBlock.stack.id, modelData)
                            contentItem: MaterialSymbol {
                                anchors.centerIn: parent
                                horizontalAlignment: Text.AlignHCenter
                                text: "remove"
                                iconSize: Appearance.font.pixelSize.normal
                                color: Appearance.m3colors.m3onSurface
                            }
                            StyledToolTip { text: Translation.tr("Unstack (back to a pin)") }
                        }
                    }
                }
            }
        }

        StyledText {
            visible: (Config.options.dock.stacks?.length ?? 0) === 0
            text: Translation.tr("No stacks yet. Drag one dock icon onto another and hold until it highlights.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
        }
    }
}
