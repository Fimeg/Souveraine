import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Quickshell
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Wallpaper — pick from ~/Pictures/Wallpapers inside the settings app.
// Settings is a window in the main shell process, so applying calls the
// shell-owned Wallpapers singleton directly. Never spawn a nested `qs ipc`
// process from this page.

ContentPage {
    forceWidth: true

    readonly property string homeDir: Quickshell.env("HOME")
    property string currentDir: homeDir + "/Pictures/Wallpapers"
    property var quickDirs: [
        { name: "Wallpapers", path: homeDir + "/Pictures/Wallpapers", icon: "wallpaper" },
        { name: "Pictures", path: homeDir + "/Pictures", icon: "image" },
        { name: "Downloads", path: homeDir + "/Downloads", icon: "download" },
        { name: "Home", path: homeDir, icon: "home" }
    ]

    ContentSection {
        icon: "wallpaper"
        title: Translation.tr("Wallpaper")

        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("Current: %1").arg(
                (Config.options.background.wallpaperPath || Translation.tr("none")).split("/").pop())
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            elide: Text.ElideMiddle
        }

        // Content filter — which wallhaven categories the random pick may
        // include. Persisted to config.json where the download script reads it.
        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("Allowed content")
            color: Appearance.colors.colOnLayer1
            font.pixelSize: Appearance.font.pixelSize.small
        }

        Flow {
            Layout.fillWidth: true
            spacing: 6

            Repeater {
                model: [
                    { key: "sfw",     label: Translation.tr("SFW") },
                    { key: "sketchy", label: Translation.tr("Sketchy") },
                    { key: "nsfw",    label: Translation.tr("NSFW") }
                ]

                RippleButton {
                    id: purityChip
                    required property var modelData
                    property bool on: modelData.key === "sfw" ? WallpaperDownload.puritySfw
                        : modelData.key === "sketchy" ? WallpaperDownload.puritySketchy
                        : WallpaperDownload.purityNsfw
                    padding: 8
                    buttonRadius: Appearance.rounding.small
                    toggled: on
                    colBackgroundToggled: Appearance.colors.colSecondaryContainer
                    colBackgroundToggledHover: Appearance.colors.colSecondaryContainerHover
                    colRippleToggled: Appearance.colors.colSecondaryContainerActive

                    onClicked: {
                        if (modelData.key === "sfw")
                            WallpaperDownload.puritySfw = !WallpaperDownload.puritySfw;
                        else if (modelData.key === "sketchy")
                            WallpaperDownload.puritySketchy = !WallpaperDownload.puritySketchy;
                        else
                            WallpaperDownload.purityNsfw = !WallpaperDownload.purityNsfw;
                        WallpaperDownload.savePurity();
                    }

                    contentItem: RowLayout {
                        spacing: 4
                        MaterialSymbol {
                            iconSize: 16
                            text: purityChip.on ? "check" : "add"
                            color: purityChip.on ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                        }
                        StyledText {
                            font.pixelSize: Appearance.font.pixelSize.small
                            text: modelData.label
                            color: purityChip.on ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                        }
                    }
                }
            }
        }

        // Download a fresh random wallpaper from wallhaven. Downloads land in
        // ~/Pictures/Wallpapers (each a distinct wallhaven_<id> file), then
        // apply through the shell like any picked image.
        RippleButton {
            Layout.fillWidth: true
            padding: 10
            buttonRadius: Appearance.rounding.small
            enabled: !WallpaperDownload.downloading
            onClicked: WallpaperDownload.download()

            contentItem: RowLayout {
                spacing: 8
                MaterialSymbol {
                    iconSize: 20
                    text: WallpaperDownload.downloading ? "hourglass_top" : "cloud_download"
                    color: Appearance.colors.colOnLayer1
                }
                StyledText {
                    Layout.fillWidth: true
                    font.pixelSize: Appearance.font.pixelSize.normal
                    text: WallpaperDownload.downloading
                        ? Translation.tr("Downloading…")
                        : Translation.tr("Download random wallpaper")
                    color: Appearance.colors.colOnLayer1
                }
            }
        }

        StyledText {
            visible: WallpaperDownload.lastError.length > 0
            Layout.fillWidth: true
            text: WallpaperDownload.lastError
            color: Appearance.colors.colError
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        // Quick directory buttons
        Flow {
            Layout.fillWidth: true
            spacing: 6

            Repeater {
                model: quickDirs

                RippleButton {
                    required property var modelData
                    property bool isCurrent: currentDir === modelData.path
                    padding: 8
                    buttonRadius: Appearance.rounding.small
                    toggled: isCurrent
                    colBackgroundToggled: Appearance.colors.colSecondaryContainer
                    colBackgroundToggledHover: Appearance.colors.colSecondaryContainerHover
                    colRippleToggled: Appearance.colors.colSecondaryContainerActive

                    onClicked: {
                        currentDir = modelData.path;
                    }

                    contentItem: RowLayout {
                        spacing: 4
                        MaterialSymbol {
                            iconSize: 16
                            text: modelData.icon
                            color: isCurrent ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                        }
                        StyledText {
                            font.pixelSize: Appearance.font.pixelSize.small
                            text: modelData.name
                            color: isCurrent ? Appearance.colors.colOnSecondaryContainer : Appearance.colors.colOnLayer1
                        }
                    }
                }
            }
        }

        // Current path display
        StyledText {
            Layout.fillWidth: true
            text: currentDir.replace(homeDir, "~")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            elide: Text.ElideMiddle
        }

        GridView {
            id: grid
            Layout.fillWidth: true
            // Rows of ~3 across the phone width; height fits the model.
            readonly property int cols: 3
            cellWidth: Math.floor(width / cols)
            cellHeight: Math.floor(cellWidth * 2)  // portrait-ish tiles
            implicitHeight: Math.ceil(folderModel.count / cols) * cellHeight
            interactive: false  // the page scrolls, not the grid
            clip: true

            model: FolderListModel {
                id: folderModel
                folder: "file://" + currentDir
                nameFilters: ["*.jpg", "*.jpeg", "*.png", "*.webp", "*.avif"]
                showDirs: true
                showDotAndDotDot: false
                showOnlyReadable: true
                sortField: FolderListModel.Name
            }

            delegate: Item {
                required property string filePath
                required property string fileName
                required property bool fileIsDir
                width: grid.cellWidth
                height: grid.cellHeight

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 5
                    radius: Appearance.rounding.small
                    color: Appearance.colors.colLayer1
                    border.width: (!fileIsDir && Config.options.background.wallpaperPath === filePath) ? 3 : 0
                    border.color: Appearance.colors.colPrimary
                    clip: true

                    Image {
                        anchors.fill: parent
                        anchors.margins: 3
                        source: fileIsDir ? "" : "file://" + filePath
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        sourceSize.width: 240
                        visible: !fileIsDir
                    }

                    // Directory indicator
                    Column {
                        anchors.centerIn: parent
                        visible: fileIsDir
                        spacing: 4

                        MaterialSymbol {
                            anchors.horizontalCenter: parent.horizontalCenter
                            iconSize: 32
                            text: "folder"
                            color: Appearance.colors.colPrimary
                        }
                        StyledText {
                            anchors.horizontalCenter: parent.horizontalCenter
                            font.pixelSize: Appearance.font.pixelSize.smaller
                            text: fileName
                            color: Appearance.colors.colOnLayer1
                            elide: Text.ElideRight
                            width: grid.cellWidth - 16
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            if (fileIsDir) {
                                currentDir = filePath;
                            } else {
                                // Shell process owns selection + theming.
                                Wallpapers.apply(filePath);
                                Config.options.background.wallpaperPath = filePath;
                            }
                        }
                    }
                }
            }
        }

        StyledText {
            visible: folderModel.count === 0
            Layout.fillWidth: true
            text: Translation.tr("No images in %1").arg(currentDir.replace(homeDir, "~"))
            color: Appearance.colors.colSubtext
            wrapMode: Text.WordWrap
        }
    }
}
