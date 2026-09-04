// Pixel3Arch addition: quick toggle for blueline-autorotate (systemd user
// service). Deploy to modules/common/models/quickToggles/.
import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.services
import qs.modules.common

QuickToggleModel {
    id: root
    property bool running: false

    name: Translation.tr("Auto-rotate")
    toggled: root.running
    icon: "screen_rotation"
    mainAction: () => {
        root.running = !root.running;
        // iio-sensor-proxy runs at boot (blueline-sensors-enable.service)
        // for proximity gating + ambient light; this toggle only controls
        // the rotation daemon.
        Quickshell.execDetached(["sh", "-c", root.running
            ? "systemctl --user start blueline-autorotate"
            : "systemctl --user stop blueline-autorotate"]);
    }
    tooltipText: Translation.tr("Lock screen rotation to accelerometer")

    Process {
        running: true
        command: ["systemctl", "--user", "is-active", "--quiet", "blueline-autorotate"]
        onExited: exitCode => root.running = (exitCode === 0)
    }
}
