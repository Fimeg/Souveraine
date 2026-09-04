import qs.modules.common
import qs.modules.common.widgets
import qs.services
import QtQuick
import QtQuick.Layouts

Item {
    id: root
    property bool borderless: Config.options.bar.borderless
    property bool showDate: Config.options.bar.verbose
    // Per-instance Qt date format; empty = the global DateTime.time
    // (time.format in config.json, which the desktop clock also uses).
    //
    // Souveraine-owned override of the ii ClockWidget. This property was the
    // ENTIRE content of ii-phone's 8-line fork of this file, and BarContent
    // now sets it on every device — so without unifying here, the desktop
    // clock would be handed a property that does not exist on it. Additive
    // and defaulted to "", so the ii behaviour is unchanged when unset.
    property string customFormat: ""
    implicitWidth: rowLayout.implicitWidth
    implicitHeight: Appearance.sizes.barHeight

    RowLayout {
        id: rowLayout
        anchors.centerIn: parent
        spacing: 4

        StyledText {
            font.pixelSize: Appearance.font.pixelSize.large
            color: Appearance.colors.colOnLayer1
            text: root.customFormat ? Qt.locale().toString(DateTime.clock.date, root.customFormat) : DateTime.time
        }

        StyledText {
            visible: root.showDate
            font.pixelSize: Appearance.font.pixelSize.small
            color: Appearance.colors.colOnLayer1
            text: "•"
        }

        StyledText {
            visible: root.showDate
            font.pixelSize: Appearance.font.pixelSize.small
            color: Appearance.colors.colOnLayer1
            text: DateTime.longDate
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: !Config.options.bar.tooltips.clickToShow

        ClockWidgetPopup {
            hoverTarget: mouseArea
        }
    }
}
