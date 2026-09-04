// Shell layer/state registry — the declarative surface model.
//
// One place that says, for each meaningful surface: which layer-shell layer
// it lives on, which GlobalStates bit(s) gate its visibility, and whether it
// is active right now (derived from the live state). This is the registry
// half of the layer-registry + ShellState work
// (docs/tasks/souveraine-shell-ecosystem.md, section 1).
//
// This is a QtObject instantiated inside Dock.qml's Scope (as
// ShellModelLocal.ShellModel), NOT a qs.services singleton — same reason as
// DockManifest: GlobalStates.qml imports qs.services, so a qs.services
// singleton that imports qs for GlobalStates forms a circular import QML
// cannot resolve. As a local type under modules/common it sidesteps that
// cycle. It carries its own imports (qs, qs.services, qs.modules.common,
// QtQuick) because a local type does NOT inherit the importing file's
// imports.
//
// IMPORTANT — what this is NOT:
//   - It does NOT change how panels render or claim their layer. Panels keep
//     their ad-hoc WlrLayershell.layer / exclusiveZone bindings. Making
//     panels READ from this registry is a later, riskier step.
//   - It is a read-only projection that mirrors the truth, so the agent and
//     external callers can ask "what surfaces exist, what layer, what state
//     gates each, what is active" without parsing QML.
//
// Layer vocabulary mirrors the wlr-layer-shell protocol quickshell exposes
// (Quickshell.Wayland.WlrLayer): Background < Bottom < Top < Overlay. The
// lock surface is separate — it uses ext-session-lock-v1
// (WlSessionLockSurface), not the layer shell — recorded here as layer
// "session-lock" so consumers can tell the mechanisms apart.
//
// Quickshell's PanelWindow default layer is Top when WlrLayershell.layer is
// unset, which is how several surfaces (dock, bar, sidebarLeft, sidebarRight)
// end up on Top implicitly. Those defaults are reflected here as "top" so the
// registry matches what actually renders, not just what is declared.

import qs
import qs.services
import qs.modules.common
import QtQuick

QtObject {
    id: root

    // --- The declarative registry --------------------------------------
    //
    // Each entry is a plain object the projection returns verbatim. Fields:
    //   name        stable id, matches the family/panel name
    //   layer       "background" | "bottom" | "top" | "overlay" | "session-lock"
    //   namespace   the wlr layer namespace the panel claims, when known
    //   layerRule   "declared" (set explicitly in QML) | "default-top"
    //               (PanelWindow default — no explicit WlrLayershell.layer)
    //               | "session-lock" (ext-session-lock-v1, not layer shell)
    //   gateStates  array of GlobalStates property names whose truth gates
    //               visibility (the state that drives the surface being
    //               shown). Empty for always-resident structural surfaces.
    //   gateConfig  Config.options.* path that enables the surface at all
    //               (the family extraCondition), "" when none.
    //   activeExpr  short human-readable description of when active.

    readonly property var _surfaceDefs: [
        {
            name: "background",
            layer: "background",
            namespace: "quickshell:background",
            layerRule: "declared",
            gateStates: [],
            gateConfig: "",
            activeExpr: "always (resident); raises to overlay when screenLocked"
        },
        {
            name: "bar",
            layer: "top",
            namespace: "quickshell:bar",
            layerRule: "default-top",
            gateStates: ["barOpen"],
            gateConfig: "options.bar.vertical (excludes verticalBar)",
            activeExpr: "barOpen && !options.bar.vertical"
        },
        {
            name: "verticalBar",
            layer: "top",
            namespace: "quickshell:verticalBar",
            layerRule: "default-top",
            gateStates: ["barOpen"],
            gateConfig: "options.bar.vertical",
            activeExpr: "barOpen && options.bar.vertical"
        },
        {
            name: "dock",
            layer: "top",
            namespace: "quickshell:dock",
            layerRule: "default-top",
            gateStates: ["dockRevealed"],
            gateConfig: "options.dock.enable",
            activeExpr: "options.dock.enable && Dock.computeDockState() !== Hidden \u2014 pinned on home, hidden on every other zone; suppressed by oskOpen + screenLocked"
        },
        {
            name: "sidebarLeft",
            layer: "top",
            namespace: "quickshell:sidebarLeft",
            layerRule: "default-top",
            gateStates: ["sidebarLeftOpen"],
            gateConfig: "",
            activeExpr: "sidebarLeftOpen"
        },
        {
            name: "sidebarRight",
            layer: "top",
            namespace: "quickshell:sidebarRight",
            layerRule: "default-top",
            gateStates: ["sidebarRightOpen"],
            gateConfig: "",
            activeExpr: "sidebarRightOpen"
        },
        {
            name: "overview",
            layer: "top",
            namespace: "quickshell:overview",
            layerRule: "declared",
            gateStates: ["overviewOpen"],
            gateConfig: "",
            activeExpr: "overviewOpen"
        },
        {
            name: "onScreenKeyboard",
            layer: "overlay",
            namespace: "quickshell:onScreenKeyboard",
            layerRule: "declared",
            gateStates: ["oskOpen"],
            gateConfig: "",
            activeExpr: "oskOpen"
        },
        {
            name: "lock",
            layer: "session-lock",
            namespace: "",
            layerRule: "session-lock",
            gateStates: ["screenLocked"],
            gateConfig: "",
            activeExpr: "screenLocked (ext-session-lock-v1, not layer shell)"
        }
    ]

    // --- Read projections ----------------------------------------------

    // surfaces() — the registry list with a live `active` flag per entry.
    // Shape rhymes with DockManifest.manifest(): a plain JS object array
    // safe to serialize and hand to the agent / settings app.
    function surfaces() {
        return root._surfaceDefs.map(s => Object.assign({}, s, {
            active: root._isActive(s.name)
        }));
    }

    // state() — the current GlobalStates bits that matter for layer gating,
    // plus the high-level shell mode. Read-only snapshot.
    function state() {
        return {
            mode: Config.options?.souveraine?.phone ? "phone" : "desktop",
            barOpen: GlobalStates.barOpen,
            oskOpen: GlobalStates.oskOpen,
            screenLocked: GlobalStates.screenLocked,
            overviewOpen: GlobalStates.overviewOpen,
            sidebarLeftOpen: GlobalStates.sidebarLeftOpen,
            sidebarRightOpen: GlobalStates.sidebarRightOpen,
            dockRevealed: GlobalStates.dockRevealed,
            dockSuppressed: GlobalStates.dockSuppressed,
            dockDragInProgress: GlobalStates.dockDragInProgress,
            overlayOpen: GlobalStates.overlayOpen
        };
    }

    // --- Internal: derive active from the live state -------------------
    //
    // The single source of truth for "is this surface showing right now" is
    // the GlobalStates bit that gates it. The dock is special-cased because
    // its visibility is computed (pinned / revealed / shown-on-empty-desktop)
    // rather than a bare boolean; we approximate "active" as the dock's own
    // manifest hidden flag so we never disagree with the dock about itself.

    function _isActive(name) {
        switch (name) {
            case "background":
                return true; // always resident
            case "bar":
                return GlobalStates.barOpen && !Config.options?.bar?.vertical;
            case "verticalBar":
                return GlobalStates.barOpen && !!Config.options?.bar?.vertical;
            case "sidebarLeft":
                return GlobalStates.sidebarLeftOpen;
            case "sidebarRight":
                return GlobalStates.sidebarRightOpen;
            case "overview":
                return GlobalStates.overviewOpen;
            case "onScreenKeyboard":
                return GlobalStates.oskOpen;
            case "lock":
                return GlobalStates.screenLocked;
            case "dock":
                return root._dockActive();
        }
        return false;
    }

    // Bound by Dock.qml, which instantiates this. The fallback ladder that
    // stood here read GlobalStates only, so it could not see the home-zone
    // rule and called the dock inactive on home.
    property string dockVisibility: "hidden"

    function _dockActive() {
        if (GlobalStates.screenLocked) return false;
        return root.dockVisibility !== "hidden";
    }
}
