import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

RippleButton {
    Layout.fillHeight: true
    // No top-only Layout.topMargin: it shoved every button DOWN with nothing
    // balancing it, so icons drooped below the bar. The dockRow already
    // centers the row in the visible bar; let fillHeight do the centering.
    implicitWidth: implicitHeight - topInset - bottomInset
    buttonRadius: Appearance.rounding.normal

    // 56, not 44. The dock read small on a 540px-wide panel — Casey,
    // 2026-08-07: "the whole dock is a bit small, it should scale a bit."
    // This is the one number the row's height follows, so the icons and the
    // stack box scale with it rather than each being tuned apart.
    background.implicitHeight: Config.options?.dock.buttonSize ?? 56
}
