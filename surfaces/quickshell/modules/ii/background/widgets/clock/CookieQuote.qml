// Souveraine fork of ii's CookieQuote. One change: it yields to her face.
//
// The quote is the clock's sibling inside `ClockWidget`'s Column, not its
// child, so `CookieClock`'s own fade cannot reach it — and the parent that
// holds both lives in `ii-base`, which never reaches the phone (the base tree
// has drifted ~900 files; overrides are how a fix arrives). So the yield is
// duplicated here rather than written once above them both. If a third
// widget ever needs it, that is the moment to override `ClockWidget` instead
// of adding a third copy of this binding.
import qs.modules.common
import qs.modules.common.widgets
import qs.services
import QtQuick
import Qt5Compat.GraphicalEffects

Item {
    id: root

    readonly property string quoteText: Config.options.background.widgets.clock.quote.text

    implicitWidth: quoteBox.implicitWidth
    implicitHeight: quoteBox.implicitHeight

    // Out of her way, on the same timings the clock uses, because they are one
    // piece of furniture to look at even though they are two items in a
    // column. Left behind, the quote sits across her chest — which is where it
    // was, and how this was noticed.
    readonly property bool faceUp: Face.joined
    opacity: root.faceUp ? 0 : 1
    scale: root.faceUp ? 0.86 : 1

    Behavior on opacity {
        NumberAnimation {
            duration: root.faceUp ? 180 : 260
            easing.type: Easing.OutCubic
        }
    }
    Behavior on scale {
        NumberAnimation {
            duration: root.faceUp ? 180 : 260
            easing.type: Easing.OutCubic
        }
    }

    DropShadow {
        source: quoteBox
        anchors.fill: quoteBox
        horizontalOffset: 0
        verticalOffset: 2
        radius: 12
        samples: radius * 2 + 1
        color: Appearance.colors.colShadow
        transparentBorder: true
    }

    Rectangle {
        id: quoteBox

        implicitWidth: quoteRow.implicitWidth + 8 * 2
        implicitHeight: quoteRow.implicitHeight + 4 * 2
        radius: Appearance.rounding.small
        color: Appearance.colors.colSecondaryContainer

        Row {
            id: quoteRow
            anchors.centerIn: parent
            spacing: 4

            MaterialSymbol {
                id: quoteIcon
                anchors.top: parent.top
                iconSize: Appearance.font.pixelSize.huge
                text: "format_quote"
                color: Appearance.colors.colOnSecondaryContainer
            }
            StyledText {
                id: quoteStyledText
                horizontalAlignment: Text.AlignLeft
                text: Config.options.background.widgets.clock.quote.text
                color: Appearance.colors.colOnSecondaryContainer
                font {
                    family: Appearance.font.family.reading
                    pixelSize: Appearance.font.pixelSize.large
                    weight: Font.Normal
                }
            }
        }
    }
}
