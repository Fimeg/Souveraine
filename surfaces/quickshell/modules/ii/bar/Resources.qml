import qs.modules.common
import qs.modules.common.widgets
import qs.services
import QtQuick
import QtQuick.Layouts

// Resources — Souveraine-owned override of the ii original, replacing the
// ii-phone fork of the same file.
//
// This was the one bar fork carrying a genuine behavioural difference rather
// than pure layout: a 540px bar cannot show three stat circles plus a network
// readout, so the phone showed one stat at a time and rotated it. That is a
// real mode, so it stays a real mode — it just stops being a second copy of
// the file. `rotate` selects it; everything else is shared.
//
// The heavy half (network traffic, with its TextMetrics and two RowLayouts)
// is behind a Loader that is inactive while rotating, so the compact mode
// does not construct what it will never show.
MouseArea {
    id: root
    property bool borderless: Config.options.bar.borderless
    property bool alwaysShowAllResources: false

    // The host (BarContent) offers a default from the device profile; an
    // explicit config value outranks it. Same precedence as the bar layout:
    // config wins if it says anything, profile decides otherwise.
    property bool autoRotate: false
    readonly property bool rotate: {
        const v = Config.options.bar.resources?.rotate;
        if (v === "on" || v === true) return true;
        if (v === "off" || v === false) return false;
        return root.autoRotate;
    }

    implicitWidth: rowLayout.implicitWidth + rowLayout.anchors.leftMargin + rowLayout.anchors.rightMargin
    implicitHeight: Appearance.sizes.barHeight
    hoverEnabled: !Config.options.bar.tooltips.clickToShow

    // Rotating mode: memory -> cpu -> swap. Tap still opens the full popup,
    // so nothing is unreachable, only unshown.
    property int shownResource: 0
    Timer {
        interval: (Config.options.bar.resources?.rotateInterval ?? 4) * 1000
        running: root.rotate
        repeat: true
        onTriggered: root.shownResource = (root.shownResource + 1) % 3
    }

    RowLayout {
        id: rowLayout

        spacing: 0
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 4

        Loader {
            active: !root.rotate && NetworkTraffic.available
            visible: active
            Layout.rightMargin: active ? 16 : 0

            sourceComponent: Item {
                implicitWidth: speedMeasure.implicitWidth
                implicitHeight: Appearance.sizes.barHeight
                clip: true

                TextMetrics {
                    id: speedTextMetrics
                    text: "8888G/s"
                    font.pixelSize: Appearance.font.pixelSize.small
                    font.family: Appearance.font.family.main
                    font.variableAxes: Appearance.font.variableAxes.main
                }

                RowLayout {
                    id: speedMeasure
                    visible: false

                    MaterialSymbol {
                        text: "south"
                        iconSize: Appearance.font.pixelSize.normal
                    }

                    Item {
                        implicitWidth: speedTextMetrics.width
                        implicitHeight: 1
                    }

                    Item {
                        implicitWidth: 2
                        implicitHeight: 1
                    }

                    MaterialSymbol {
                        text: "north"
                        iconSize: Appearance.font.pixelSize.normal
                    }

                    Item {
                        implicitWidth: speedTextMetrics.width
                        implicitHeight: 1
                    }
                }

                RowLayout {
                    id: speedRow
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2

                    MaterialSymbol {
                        text: "south"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colOnLayer1
                    }

                    StyledText {
                        width: speedTextMetrics.width
                        text: NetworkTraffic.downloadSpeedCompactText
                        font.pixelSize: Appearance.font.pixelSize.small
                        color: Appearance.colors.colOnLayer1
                        horizontalAlignment: Text.AlignRight
                        elide: Text.ElideLeft
                    }

                    MaterialSymbol {
                        text: "north"
                        iconSize: Appearance.font.pixelSize.normal
                        color: Appearance.colors.colOnLayer1
                        Layout.leftMargin: 2
                    }

                    StyledText {
                        width: speedTextMetrics.width
                        text: NetworkTraffic.uploadSpeedCompactText
                        font.pixelSize: Appearance.font.pixelSize.small
                        color: Appearance.colors.colOnLayer1
                        horizontalAlignment: Text.AlignRight
                        elide: Text.ElideLeft
                    }
                }
            }
        }

        // Compact: a single circle, cycling.
        Resource {
            visible: root.rotate
            iconName: root.shownResource === 0 ? "memory" : root.shownResource === 1 ? "planner_review" : "swap_horiz"
            percentage: root.shownResource === 0 ? ResourceUsage.memoryUsedPercentage : root.shownResource === 1 ? ResourceUsage.cpuUsage : ResourceUsage.swapUsedPercentage
            warningThreshold: root.shownResource === 0 ? Config.options.bar.resources.memoryWarningThreshold : root.shownResource === 1 ? Config.options.bar.resources.cpuWarningThreshold : Config.options.bar.resources.swapWarningThreshold
        }

        // Full: all three, each with its own reveal rule.
        Resource {
            visible: !root.rotate
            iconName: "memory"
            percentage: ResourceUsage.memoryUsedPercentage
            warningThreshold: Config.options.bar.resources.memoryWarningThreshold
        }

        Resource {
            iconName: "swap_horiz"
            percentage: ResourceUsage.swapUsedPercentage
            shown: !root.rotate && ((Config.options.bar.resources.alwaysShowSwap && percentage > 0) || (MprisController.activePlayer?.trackTitle == null) || root.alwaysShowAllResources)
            Layout.leftMargin: shown ? 6 : 0
            warningThreshold: Config.options.bar.resources.swapWarningThreshold
        }

        Resource {
            iconName: "planner_review"
            percentage: ResourceUsage.cpuUsage
            shown: !root.rotate && (Config.options.bar.resources.alwaysShowCpu || !(MprisController.activePlayer?.trackTitle?.length > 0) || root.alwaysShowAllResources)
            Layout.leftMargin: shown ? 6 : 0
            warningThreshold: Config.options.bar.resources.cpuWarningThreshold
        }
    }

    ResourcesPopup {
        hoverTarget: root
    }
}
