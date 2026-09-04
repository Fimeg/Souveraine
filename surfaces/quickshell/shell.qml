//@ pragma UseQApplication
//@ pragma Env QS_NO_RELOAD_POPUP=1
//@ pragma Env QT_QUICK_CONTROLS_STYLE=Basic
//@ pragma Env QT_QUICK_FLICKABLE_WHEEL_DECELERATION=10000

// Souveraine shell entry — `qs -c souveraine`.
// One family, two modes (desktop and phone), converging over time. Panels
// that differ by form factor are gated on Config.options.souveraine.phone
// inside SouveraineFamily; a panel is "homogenized" when its gate is gone.
// No panelFamily switching: this config loads SouveraineFamily, period.
// ii's tree is borrowed underneath (composed by deploy.sh) until each piece
// is owned outright.

import "modules/common"
import "modules/ii/lock"
import "modules/souveraine/boot"
import "services"
import "panelFamilies"

import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Io
import Quickshell.Hyprland

ShellRoot {
    id: root

    // NOTE: no DeviceConnectNotification here — the phone's ii snapshot
    // predates it, and version-dependent types don't belong in the entry file.
    ReloadPopup {}

    // Boot bloom overlay. Top-level (NOT inside the Config.ready LazyLoader) so
    // it maps as Hyprland's first surface, before the rest of the shell loads —
    // covering all of Hyprland's boot render and continuing the C splash's
    // bloom until the lockscreen is secure. Signals the splash to release DRM
    // master on its first frame; LockScreen clears GlobalStates.bootBloomActive.
    BootBloom {}

    // The lock, top-level and ungated, for both of the reasons the docs give.
    //
    // Boot (`session-authority-boot-order.md` Phase A.1): no surface may
    // precede the lock decision. Behind `Config.ready` it could, and once did —
    // an NM password dialog rendered before the lockscreen.
    //
    // Reload (TASK-48): a `LazyLoader` whose `active` is false when quickshell
    // propagates reloads has no item to hand its successor, so the new
    // `WlSessionLock` never adopts the live compositor lock. As a direct child
    // of `ShellRoot` — itself a `ReloadPropagator` — this matches its
    // predecessor positionally, with no `reloadableId` needed, and the adopt
    // path in `LockScreen.qml` finally has something to adopt.
    //
    // It must stay a direct child. Wrapping it in any conditional loader
    // reintroduces both failures at once.
    Lock {}

    // A normal XDG app window in this process. Its desktop/sidebar launchers
    // call the `settings` IPC target; no second Quickshell/session scope is
    // created.
    SettingsWindow {}

    Component.onCompleted: {
        MaterialThemeLoader.reapplyTheme()
        SessiondBridge.claimAuthority()
        SessiondBridge.load()
        Hyprsunset.load()
        FirstRunExperience.load()
        ConflictKiller.load()
        CrashReporter.start()
        Cliphist.refresh()
        Wallpapers.load()
        Updates.load()
        // Brightness is a singleton nothing references, so its IPC target
        // never registers and the lever's `||` fallback never fires.
        Brightness.initializeMonitor(0)
    }

    LazyLoader {
        active: Config.ready
        component: SouveraineFamily {}
    }
}
