// Lock-surface information policy. Every card asks this singleton instead of
// growing an accidental privacy rule of its own.
pragma Singleton

import QtQuick
import Quickshell
import qs.modules.common

Singleton {
    id: root

    readonly property string ambient: "ambient"
    readonly property string personal: "personal"
    readonly property string stepUp: "step-up"

    function allowsOnLock(tier, promotedAmbient = false) {
        if (tier === root.ambient) return true;
        // Promotion is intentionally field-specific (for example media title)
        // and never applies to credentials, memories, agent output, or actions.
        return tier === root.personal && promotedAmbient;
    }

    readonly property bool mediaControlsVisible: Config.options.lock.content.showMediaControls
    readonly property bool mediaMetadataVisible: root.allowsOnLock(
        root.personal, Config.options.lock.content.mediaMetadataAmbient)
    readonly property bool batteryVisible: Config.options.lock.content.showBattery

    readonly property bool notificationsVisible: Config.options.lock.content.showNotifications
    readonly property bool notificationContentVisible: root.allowsOnLock(
        root.personal, Config.options.lock.content.notificationContentAmbient)
}
