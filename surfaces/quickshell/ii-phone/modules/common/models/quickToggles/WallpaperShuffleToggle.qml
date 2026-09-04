// Pixel3Arch addition: one-tap wallpaper shuffle (the settings page barely
// scales on the phone). Deploy to modules/common/models/quickToggles/.
import QtQuick
import qs
import qs.services
import qs.modules.common

QuickToggleModel {
    name: Translation.tr("Shuffle wallpaper")
    toggled: false
    icon: "wallpaper"
    mainAction: () => {
        Wallpapers.randomFromCurrentFolder();
    }
    tooltipText: Translation.tr("Random wallpaper from the current folder")
}
