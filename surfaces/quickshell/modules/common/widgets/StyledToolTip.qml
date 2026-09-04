import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Souveraine: tooltip-trigger fix. Upstream treated a parent with no
// `hovered` property (parent.hovered === undefined) as "always show" — so any
// StyledToolTip attached to a non-hoverable parent (e.g. ConfigSpinBox, a
// RowLayout) was permanently visible. That's latent on desktop and broken on
// touch. Now: a tooltip only shows on a real positive hover (hoverable parent
// actually hovered) or the explicit alternativeVisibleCondition. Non-hoverable
// parents default off.
ToolTip {
    id: root
    property bool extraVisibleCondition: true
    property bool alternativeVisibleCondition: false

    readonly property bool internalVisibleCondition: (extraVisibleCondition && (parent?.hovered ?? false)) || alternativeVisibleCondition
    verticalPadding: 5
    horizontalPadding: 10
    background: null
    font {
        family: Appearance.font.family.main
        variableAxes: Appearance.font.variableAxes.main
        pixelSize: Appearance?.font.pixelSize.smaller ?? 14
        hintingPreference: Font.PreferNoHinting // Prevent shaky text
    }

    delay: 0
    visible: internalVisibleCondition

    contentItem: StyledToolTipContent {
        id: contentItem
        font: root.font
        text: root.text
        shown: root.internalVisibleCondition
        horizontalPadding: root.horizontalPadding
        verticalPadding: root.verticalPadding
    }
}
