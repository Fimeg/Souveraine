import QtQuick
import QtQuick.Layouts
import qs.modules.common
import qs.modules.common.widgets

// Souveraine: responsive variant of ii's ContentPage. Upstream clamps the
// content column to baseWidth (600) and centers it — fine on a desktop
// window, but on a narrow/touch surface it clips both edges and stretches a
// huge gap into every row. Here, when the page is narrower than 650px the
// column is anchored left+right with fixed margins, so it tracks the live
// page width (reacts to fullscreen/resize) and nothing clips. Desktop
// windows (>=650px) keep upstream's clamp-and-center behavior.
StyledFlickable {
    id: root
    property real baseWidth: 600
    property bool forceWidth: false
    property real bottomContentPadding: 100
    readonly property bool narrow: root.width < 650
    readonly property real sideMargin: 20

    default property alias contentData: contentColumn.data

    clip: true
    contentHeight: contentColumn.implicitHeight + root.bottomContentPadding
    implicitWidth: contentColumn.implicitWidth

    ColumnLayout {
        id: contentColumn
        spacing: 30
        // Narrow: anchor to both edges so the column IS the page width minus
        // symmetric margins — tracks resize, no clipping, no centered-overflow.
        // Wide: fixed width, centered (upstream behavior).
        anchors {
            top: parent.top
            margins: root.sideMargin
            horizontalCenter: root.narrow ? undefined : parent.horizontalCenter
            left: root.narrow ? parent.left : undefined
            right: root.narrow ? parent.right : undefined
        }
        width: root.narrow ? (root.width - root.sideMargin * 2)
            : (root.forceWidth ? root.baseWidth : Math.max(root.baseWidth, implicitWidth))
    }
}
