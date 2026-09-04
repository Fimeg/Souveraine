// The collapsed state: a small chip that says a selection is live and invites a
// tap. Deliberately does NOT preview the selected text.
//
// A preview would be the obvious design and it is the wrong one here. The chip
// floats over whatever app owns the selection, so a preview duplicates content
// already on screen while adding a surface that can outlive the context it came
// from — and the content may be a password. SESSION-AUTHORITY §2 puts that in
// `personal`; showing a count instead keeps the chip `ambient` and means the
// surface itself never discloses anything.
import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

RowLayout {
    id: root

    required property int charCount

    signal tapped()
    signal dismissed()

    implicitHeight: 44
    spacing: 0

    // Tap target: the whole chip except the dismiss affordance.
    Item {
        Layout.fillHeight: true
        implicitWidth: label.implicitWidth + icon.implicitWidth + 26

        MouseArea {
            anchors.fill: parent
            onClicked: root.tapped()
        }

        RowLayout {
            anchors.centerIn: parent
            spacing: 6

            MaterialSymbol {
                id: icon
                text: "text_select_start"
                iconSize: 20
                color: Appearance.colors.colOnLayer1
            }

            StyledText {
                id: label
                text: root.charCount === 1
                    ? Translation.tr("1 character selected")
                    : Translation.tr("%1 characters selected").arg(root.charCount)
                color: Appearance.colors.colOnLayer1
                font.pixelSize: Appearance.font.pixelSize.smaller
            }
        }
    }

    // Dismiss. Present because the mask means we never see a tap landing
    // elsewhere — without this the chip's only exit is changing the selection.
    Item {
        Layout.fillHeight: true
        implicitWidth: 40

        MouseArea {
            anchors.fill: parent
            onClicked: root.dismissed()
        }

        MaterialSymbol {
            anchors.centerIn: parent
            text: "close"
            iconSize: 18
            color: Appearance.colors.colSubtext
        }
    }
}
