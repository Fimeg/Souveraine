import QtQuick
import QtQuick.Layouts
import Quickshell
import qs.services
import qs.modules.common
import qs.modules.common.widgets

ContentPage {
    id: page
    forceWidth: true

    readonly property var monitor: Brightness.monitors.length > 0
        ? Brightness.monitors[0] : null

    ContentSection {
        icon: "brightness_6"
        title: Translation.tr("Brightness")

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            MaterialSymbol {
                text: "brightness_low"
                iconSize: 22
            }
            StyledSlider {
                Layout.fillWidth: true
                enabled: page.monitor ? page.monitor.ready : false
                value: page.monitor ? page.monitor.brightness : 0
                from: 0.01
                to: 1
                onMoved: {
                    if (page.monitor) page.monitor.setBrightness(value);
                }
            }
            StyledText {
                text: page.monitor && page.monitor.ready
                    ? `${Math.round(page.monitor.brightness * 100)}%` : "—"
                font.family: Appearance.font.family.numbers
            }
        }

        ConfigSwitch {
            buttonIcon: "flash_off"
            text: Translation.tr("Anti-flashbang dimming")
            checked: Config.options.light.antiFlashbang.enable
            onCheckedChanged: Config.options.light.antiFlashbang.enable = checked
            StyledToolTip {
                text: Translation.tr("Temporarily softens large brightness jumps when content changes.")
            }
        }
    }

    ContentSection {
        icon: "monitor"
        title: Translation.tr("Built-in display")

        StyledText {
            Layout.fillWidth: true
            text: Quickshell.screens.length > 0
                ? Translation.tr("%1 · %2 × %3 logical pixels")
                    .arg(Quickshell.screens[0].name)
                    .arg(Quickshell.screens[0].width)
                    .arg(Quickshell.screens[0].height)
                : Translation.tr("No display reported")
            color: Appearance.colors.colSubtext
            wrapMode: Text.WordWrap
        }
    }
}
