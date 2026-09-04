import QtQuick
import Quickshell

import qs.modules.common
import qs.services
// AppInventory.qml root is Item (not QtObject) so it can own its scanner
// Process child — the original QtObject root failed with "no default property."
import qs.modules.ii.appInventory
import qs.modules.ii.background
import qs.modules.ii.bar
import qs.modules.ii.cheatsheet
import qs.modules.ii.dock
import qs.modules.ii.lock
import qs.modules.ii.mediaControls
import qs.modules.ii.notificationPopup
import qs.modules.ii.onScreenDisplay
import qs.modules.ii.onScreenKeyboard
import qs.modules.ii.overview
import qs.modules.ii.polkit
import qs.modules.ii.regionSelector
import qs.modules.ii.screenCorners
import qs.modules.ii.screenTranslator
import qs.modules.ii.sessionScreen
import qs.modules.ii.sidebarLeft
import qs.modules.ii.sidebarRight
import qs.modules.ii.overlay
import qs.modules.ii.verticalBar
import qs.modules.ii.wallpaperSelector
import qs.modules.souveraine.dial
import qs.modules.souveraine.navigation
import qs.modules.souveraine.selection
import qs.modules.souveraine.windowSheet
import qs.modules.souveraine.airpods

// The Souveraine panel family — one family for both modes.
// Starts at exact panel parity with IllogicalImpulseFamily (our overridden
// components resolve in naturally); per-panel form-factor gates
// (Config.options.souveraine.phone) get added only where the modes genuinely
// differ, and removed as the modes homogenize. This file is the scoreboard
// of that convergence.

Scope {
    // Non-UI service scope: hosts the app-inventory IPC surface (apps.*).
    // Plain child, not a PanelLoader entry — it has no panel/visibility
    // concerns, just a long-lived Scope holding the inventory + IpcHandler.
    AppInventoryScope {}

    PanelLoader { extraCondition: !Config.options.bar.vertical; component: Bar {} }
    PanelLoader { component: Background {} }
    PanelLoader { component: Cheatsheet {} }
    PanelLoader { extraCondition: Config.options.dock.enable; component: Dock {} }
    // Lock is NOT here. It is a direct child of ShellRoot in shell.qml.
    //
    // It lived behind this LazyLoader until 2026-07-31, and that is the whole
    // of TASK-48's shell half. `PanelLoader` is `active: Config.ready`, and
    // this family sits behind a second `Config.ready` LazyLoader — so on a
    // scene reload both gates are shut at the moment quickshell propagates
    // reloads. `LazyLoader::onReload` finds `mItem == nullptr`, skips
    // propagation entirely, and incubates a fresh Lock milliseconds later with
    // no predecessor. The new `WlSessionLock` therefore never adopts the
    // outgoing one's compositor lock, and the reload either crashes on the
    // denied re-acquire or sends `unlock_and_destroy` and drops the session.
    //
    // No amount of `reloadableId` fixes that: the object was not there to be
    // matched. `session-authority-boot-order.md` Phase A.1 already called for
    // this hoist, for the adjacent reason — nothing may render before the lock
    // decision. `BootBloom` is hoisted in shell.qml on exactly this argument.
    PanelLoader { component: MediaControls {} }
    // TASK-74. Its sole event source is sessiond's admitted accessory
    // directive; it cannot wake the display or bypass lock content policy.
    PanelLoader { component: AirPodsSurface {} }
    // Desktop notifications belong in the right-hand notification panel.
    // Keeping a transient popup surface loaded on desktop let some shell
    // restarts expose it to Hyprland as a normal app window, which was both
    // focus-stealing and extremely noisy for high-volume users. Phone mode
    // retains the compact popup/lockscreen path; desktop still owns the
    // notification server and history through SidebarRight.
    PanelLoader {
        extraCondition: Config.options.souveraine.phone
        component: NotificationPopup {}
    }
    PanelLoader { component: OnScreenDisplay {} }
    PanelLoader { component: OnScreenKeyboard {} }
    PanelLoader { component: Overlay {} }
    PanelLoader { component: Overview {} }
    PanelLoader { component: Polkit {} }
    PanelLoader { component: DialHost {} }
    // The three-finger tap's surface (TASK-55). Ungated: it is the only way
    // the hand can close a window under viewtop, and the dial's own "Kill
    // window" is a dead `hyprctl dispatch`.
    PanelLoader { component: WindowSheet {} }
    // The device's verbs, on a held power button. Ungated and not lock-gated:
    // powering off is something you do to a locked phone.
    PanelLoader { component: PowerMenu {} }
    // Text-selection action menu (TASK-18). Gated on the opt-in config: the
    // service behind it watches every selection on the device, so it must not
    // load by default. See Config.options.selection.
    PanelLoader {
        extraCondition: Config.options.selection.enable
        component: SelectionHost {}
    }

    // Gestures is a singleton and QML instantiates singletons lazily, so its
    // IpcHandler never registered — `gesture` answered "Target not found"
    // while the file was plainly deployed. Touching a property is what brings
    // it into existence. Same reason AppInventoryScope is a plain child.
    Component.onCompleted: Gestures.squeezeAction
    PanelLoader { component: RegionSelector {} }
    PanelLoader { component: ScreenCorners {} }
    PanelLoader { component: ScreenTranslator {} }
    PanelLoader { component: SessionScreen {} }
    PanelLoader { component: SidebarLeft {} }
    PanelLoader { component: SidebarRight {} }
    PanelLoader {
        extraCondition: Config.options.souveraine.phone
        component: SystemGestureRail {}
    }
    PanelLoader { extraCondition: Config.options.bar.vertical; component: VerticalBar {} }
    PanelLoader { component: WallpaperSelector {} }
}
