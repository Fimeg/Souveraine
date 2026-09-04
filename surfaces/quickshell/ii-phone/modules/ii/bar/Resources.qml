import qs.modules.common
import qs.services
import QtQuick
import QtQuick.Layouts

MouseArea {
    id: root
    property bool borderless: Config.options.bar.borderless
    property bool alwaysShowAllResources: false
    implicitWidth: rowLayout.implicitWidth + rowLayout.anchors.leftMargin + rowLayout.anchors.rightMargin
    implicitHeight: Appearance.sizes.barHeight
    hoverEnabled: !Config.options.bar.tooltips.clickToShow

    // Pixel 3: one stat at a time, rotating memory -> cpu -> swap every few
    // seconds — three circles don't fit a 540px bar. Tap still opens the
    // full ResourcesPopup.
    property int shownResource: 0
    Timer {
        interval: 4000
        running: true
        repeat: true
        onTriggered: root.shownResource = (root.shownResource + 1) % 3
    }

    RowLayout {
        id: rowLayout

        spacing: 0
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 4

        Resource {
            iconName: root.shownResource === 0 ? "memory" : root.shownResource === 1 ? "planner_review" : "swap_horiz"
            percentage: root.shownResource === 0 ? ResourceUsage.memoryUsedPercentage : root.shownResource === 1 ? ResourceUsage.cpuUsage : ResourceUsage.swapUsedPercentage
            warningThreshold: root.shownResource === 0 ? Config.options.bar.resources.memoryWarningThreshold : root.shownResource === 1 ? Config.options.bar.resources.cpuWarningThreshold : Config.options.bar.resources.swapWarningThreshold
        }
    }

    ResourcesPopup {
        hoverTarget: root
    }
}
