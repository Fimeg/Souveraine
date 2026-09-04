import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Souveraine's integrated phone navigation surface.
ContentPage {
    forceWidth: true

    ContentSection {
        icon: "gesture"
        title: Translation.tr("Layout")

        ConfigSpinBox {
            icon: "swap_vert"
            text: Translation.tr("Navigation rail height (px)")
            value: Config.options.dock.gestureRailHeight
            from: 0
            to: 96
            stepSize: 2
            onValueChanged: Config.options.dock.gestureRailHeight = value

            StyledToolTip {
                text: Translation.tr("Bottom strip reserved for Souveraine navigation — visually and in the dock's input mask. The navigation rail must always win touch here.")
            }
        }
    }

    ContentSection {
        icon: "swipe"
        title: Translation.tr("Gestures")

        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("Double-tap toggles app fullscreen. Swipe up reveals the dock. Swipe down dismisses the nearest surface: the keyboard when it's open (the rail rides on top of it), otherwise a visible dock — pinned included. Timing and threshold controls will appear here once their defaults prove out.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }
    }
}
