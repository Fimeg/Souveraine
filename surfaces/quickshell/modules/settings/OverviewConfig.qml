import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Widgets
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Home screen (app drawer) — the TASK-14 split's launcher half. The drawer
// reads its grid straight from Config (see AppGrid.qml), so these knobs are
// the single source; the grid re-lays-out on the next drawer open. Rows and
// columns at the phone's 540x1080: 4 columns keeps a thumb travel across a
// page, 5 rows fills the panel without scrolling. The old desktop overview's
// rows/columns are untouched — this page owns Config.options.overview.appGrid.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "grid_view"
        title: Translation.tr("App drawer")

        ConfigSwitch {
            buttonIcon: "apps"
            text: Translation.tr("Enable drawer")
            checked: Config.options.overview.enable
            onCheckedChanged: {
                Config.options.overview.enable = checked;
            }
            StyledToolTip {
                text: Translation.tr("The Home surface: search on top, the app grid below. Off, the pill's swipe-home does nothing.")
            }
        }

        ConfigSpinBox {
            icon: "view_column"
            text: Translation.tr("Columns")
            value: Config.options.overview.appGrid.columns
            from: 1
            to: 10
            stepSize: 1
            onValueChanged: {
                Config.options.overview.appGrid.columns = value;
            }
            StyledToolTip {
                text: Translation.tr("Icons per row. More columns = denser grid and smaller tiles.")
            }
        }

        ConfigSpinBox {
            icon: "view_agenda"
            text: Translation.tr("Rows")
            value: Config.options.overview.appGrid.rows
            from: 1
            to: 10
            stepSize: 1
            onValueChanged: {
                Config.options.overview.appGrid.rows = value;
            }
            StyledToolTip {
                text: Translation.tr("Icon rows per page. More rows fills the screen; fewer leaves room for the page dots.")
            }
        }

        ConfigSpinBox {
            icon: "photo_size_select_large"
            text: Translation.tr("Icon size (px)")
            value: Config.options.overview.appGrid.iconSize
            from: 24
            to: 96
            stepSize: 4
            onValueChanged: {
                Config.options.overview.appGrid.iconSize = value;
            }
            StyledToolTip {
                text: Translation.tr("The icon glyph itself; labels always scale to the tile.")
            }
        }
    }
}
