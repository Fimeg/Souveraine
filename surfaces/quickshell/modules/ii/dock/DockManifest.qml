// Dock manifest — the dock's state projected for external consumers
// (the agent, the settings app, anything that needs to read or act on the
// dock without parsing QML). See docs/tasks/souveraine-shell-ecosystem.md.
//
// This is a QtObject instantiated inside Dock.qml's Scope (as
// DockLocal.DockManifest), NOT a qs.services singleton. Reason: GlobalStates.qml
// imports qs.services, so a DockManifest singleton would form a circular import
// (GlobalStates → qs.services → DockManifest) that QML can't resolve. As a local
// type in the dock dir it sidesteps that cycle. It carries its own `import qs`
// (for GlobalStates) and `import qs.modules.common` (for Config) — local types
// do NOT inherit the importing file's imports.
//
// Two halves:
//   1. manifest()      — read-only snapshot of pinned apps, stacks,
//                        visibility state, and mode.
//   2. guarded methods — pin/unpin/restack/rename. Every method validates
//                        its inputs and refuses mutation when the dock is
//                        in a state that forbids it. Returns a result object.
//
// The agent does NOT get a new toolcall integration here — Souveraine's
// existing harness integration calls these methods. The teaching of when
// to use them lives in the Souveraine School, not in this file.

import qs
import qs.services
import qs.modules.common
import QtQuick

QtObject {
    id: root

    // --- Read-only projection -------------------------------------------

    // One shape the agent (and the dock-stacks settings editor) can rely on.
    function manifest() {
        const ta = TaskbarApps;
        const stacks = (ta ? ta.stacksList() : []).map(s => ({
            id: s.id,
            name: s.name,
            members: s.members
        }));
        const stackedMembers = new Set();
        for (const s of stacks)
            for (const m of s.members) stackedMembers.add(String(m).toLowerCase());

        const pinned = (Config.options?.dock?.pinnedApps ?? [])
            .filter(id => !stackedMembers.has(String(id).toLowerCase()));

        return {
            mode: Config.options?.souveraine?.phone ? "phone" : "desktop",
            hidden: root._isHidden(),
            pinned: root.dockState(),
            pinnedApps: pinned,
            stacks: stacks,
            canMutate: root._canMutate(),
            blockReason: root._blockReason()
        };
    }

    // The dock's computed visibility, bound by Dock.qml. Read, never derived:
    // the ladder this used to keep could not see the home-zone rule and so
    // answered "hidden" on the one zone where the dock is always furniture.
    property string visibility: "hidden"

    // Visible state as a stable string the agent can reason about. The osk and
    // lock strings are the *reason* a hidden dock is hidden, not a second
    // opinion about whether it is.
    function dockState() {
        if (GlobalStates.screenLocked) return "locked";
        if (root.visibility === "hidden" && GlobalStates.oskOpen) return "suppressed-by-osk";
        return root.visibility;
    }

    // --- Guarded mutation ------------------------------------------------

    // Every mutation returns { ok, reason? }. No throw, no silent failure.

    function pin(appId) {
        const guard = root._checkMutatable(appId);
        if (!guard.ok) return guard;
        if (TaskbarApps.isPinned(appId)) return { ok: true, reason: "already-pinned" };
        TaskbarApps.togglePin(appId);
        return { ok: true };
    }

    function unpin(appId) {
        const guard = root._checkMutatable(appId);
        if (!guard.ok) return guard;
        if (TaskbarApps.stackContaining(appId)) {
            return { ok: false, reason: "app is in a stack; use unstackMember" };
        }
        if (!TaskbarApps.isPinned(appId)) return { ok: true, reason: "not-pinned" };
        TaskbarApps.togglePin(appId);
        return { ok: true };
    }

    function addToStack(stackId, appId) {
        const guard = root._checkMutatable(appId);
        if (!guard.ok) return guard;
        if (!root._stackExists(stackId)) return { ok: false, reason: "no-such-stack" };
        TaskbarApps.addToStack(stackId, appId);
        return { ok: true };
    }

    function removeFromStack(stackId, appId) {
        const guard = root._checkMutatable(appId);
        if (!guard.ok) return guard;
        if (!root._stackExists(stackId)) return { ok: false, reason: "no-such-stack" };
        TaskbarApps.removeFromStack(stackId, appId);
        return { ok: true };
    }

    function renameStack(stackId, newName) {
        const guard = root._checkMutatable();
        if (!guard.ok) return guard;
        if (!root._stackExists(stackId)) return { ok: false, reason: "no-such-stack" };
        const name = String(newName ?? "").trim();
        if (!name) return { ok: false, reason: "empty-name" };
        TaskbarApps.renameStack(stackId, name);
        return { ok: true };
    }

    // --- State checks (the guardrails) ----------------------------------

    function _canMutate() { return root._blockReason() === ""; }

    function _blockReason() {
        if (GlobalStates.screenLocked) return "screen-locked";
        if (GlobalStates.oskOpen) return "osk-open";
        if (GlobalStates.dockDragInProgress) return "drag-in-progress";
        return "";
    }

    function _checkMutatable(appId) {
        const reason = root._blockReason();
        if (reason) return { ok: false, reason: reason };
        if (appId !== undefined && !String(appId).trim()) {
            return { ok: false, reason: "empty-app-id" };
        }
        return { ok: true };
    }

    function _stackExists(stackId) {
        return TaskbarApps.stacksList().some(s => s.id === stackId);
    }

    function _isHidden() {
        return GlobalStates.screenLocked || root.visibility === "hidden";
    }
}
