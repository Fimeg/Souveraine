import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// On-screen keyboard.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "keyboard"
        title: Translation.tr("On-screen keyboard")

        ConfigSwitch {
            buttonIcon: "push_pin"
            text: Translation.tr("Pinned on startup")
            checked: Config.options.osk.pinnedOnStartup
            onCheckedChanged: {
                Config.options.osk.pinnedOnStartup = checked;
            }
            StyledToolTip {
                text: Translation.tr("Keep the on-screen keyboard visible from launch rather than on demand.")
            }
        }
        // osk.layout is intentionally not exposed here: its value space
        // (adapter default "qwerty_full" vs the layouts.js byName registry
        // keyed by display name like "English (US)") is inconsistent and
        // needs reconciling before a selector is honest about it.
    }
}
