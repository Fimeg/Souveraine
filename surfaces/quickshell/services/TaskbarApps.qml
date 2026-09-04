pragma Singleton

import qs.modules.common
import QtQuick
import Quickshell
import Quickshell.Wayland

Singleton {
    id: root

    function isPinned(appId) {
        return Config.options.dock.pinnedApps.indexOf(appId) !== -1;
    }

    function togglePin(appId) {
        if (root.isPinned(appId)) {
            Config.options.dock.pinnedApps = Config.options.dock.pinnedApps.filter(id => id !== appId)
        } else {
            Config.options.dock.pinnedApps = Config.options.dock.pinnedApps.concat([appId])
        }
    }

    // --- Fan-out stacks ------------------------------------------------
    // Backing store: Config.options.dock.stacks, a list<string> — string
    // list because nested-object arrays don't survive the JsonAdapter.
    // Each entry is a JSON object string:
    //   {"id":"stack-1","name":"Stack 1","members":["appId","appId"]}
    // JSON-per-entry is escaping-safe (the first iteration's "name|app,app"
    // format corrupted silently on a | or , in a name). id is the stable
    // lookup key; name is the display label (rename never breaks lookups).
    // Legacy pipe entries still parse (id doubles as name) and get
    // rewritten as JSON on the next stacks write.

    function parseStack(entry) {
        if (entry.startsWith("{")) {
            try {
                const o = JSON.parse(entry);
                return { id: o.id ?? "", name: o.name ?? o.id ?? "", members: o.members ?? [] };
            } catch (e) {
                return { id: entry, name: entry, members: [] };
            }
        }
        // Legacy "stackId|appId,appId" format.
        const bar = entry.indexOf("|");
        if (bar === -1) return { id: entry, name: entry, members: [] };
        const id = entry.slice(0, bar);
        const rest = entry.slice(bar + 1).trim();
        const members = rest.length ? rest.split(",").map(s => s.trim()).filter(s => s.length) : [];
        return { id: id, name: id, members: members };
    }

    function stacksList() {
        return (Config.options?.dock.stacks ?? []).map(root.parseStack);
    }

    // appId -> the stackId that contains it, or "" if none.
    function stackContaining(appId) {
        const low = appId.toLowerCase();
        for (const s of root.stacksList()) {
            if (s.members.some(m => m.toLowerCase() === low)) return s.id;
        }
        return "";
    }

    function encodeStack(s) {
        return JSON.stringify({ id: s.id, name: s.name, members: s.members });
    }

    // Rewrite the whole stacks list from a parsed [{id, name, members}]
    // array, dropping any that end up empty.
    function writeStacks(parsed) {
        Config.options.dock.stacks = parsed
            .filter(s => s.members.length > 0)
            .map(root.encodeStack);
    }

    function addToStack(stackId, appId) {
        const parsed = root.stacksList();
        const existing = parsed.find(s => s.id === stackId);
        if (existing) {
            if (!existing.members.some(m => m.toLowerCase() === appId.toLowerCase()))
                existing.members = existing.members.concat([appId]);
        } else {
            parsed.push({ id: stackId, members: [appId] });
        }
        root.writeStacks(parsed);
    }

    function removeFromStack(stackId, appId) {
        const parsed = root.stacksList();
        const existing = parsed.find(s => s.id === stackId);
        if (!existing) return;
        existing.members = existing.members.filter(m => m.toLowerCase() !== appId.toLowerCase());
        root.writeStacks(parsed);
    }

    // --- Drag interactions (Tier 1) ------------------------------------
    // Mint a stable id + display name for a fresh stack. Ids scan for the
    // max existing numeric suffix, never reuse after a delete (the old
    // count-based scheme collided: delete "Stack 1" of two and the next
    // combine minted a second "Stack 2" — and id is the lookup key, so two
    // stacks silently shared members).
    function mintStack() {
        let n = 0;
        for (const s of root.stacksList()) {
            const mi = /^stack-(\d+)$/.exec(s.id);
            if (mi) n = Math.max(n, parseInt(mi[1]));
            const mn = /^Stack (\d+)$/.exec(s.name);
            if (mn) n = Math.max(n, parseInt(mn[1]));
        }
        return { id: "stack-" + (n + 1), name: "Stack " + (n + 1) };
    }

    // Rename a stack's display label. The id (lookup key) never changes.
    function renameStack(stackId, newName) {
        if (!newName) return;
        const parsed = root.stacksList();
        const s = parsed.find(x => x.id === stackId);
        if (!s) return;
        s.name = newName;
        root.writeStacks(parsed);
    }

    // Drag `draggedAppId` onto `targetAppId` -> combine. If target is
    // already a stack (stackId non-empty), add into it; else make a new
    // stack containing target then dragged (target stays on top = first).
    function combineIntoStack(targetAppId, draggedAppId, targetStackId) {
        if (!draggedAppId || draggedAppId.toLowerCase() === targetAppId.toLowerCase()) return;
        // If the dragged app is currently in some stack, pull it out first.
        const from = root.stackContaining(draggedAppId);
        if (from) root.removeFromStack(from, draggedAppId);

        if (targetStackId) {
            root.addToStack(targetStackId, draggedAppId);
        } else {
            const fresh = root.mintStack();
            const parsed = root.stacksList();
            parsed.push({ id: fresh.id, name: fresh.name, members: [targetAppId, draggedAppId] });
            root.writeStacks(parsed);
        }
        // An app lives in one place: its stack. The standalone pin actually
        // leaves config here (it used to be only render-suppressed, leaving
        // a phantom pin string behind); unstackMember re-pins on the way out.
        const dropPins = [draggedAppId.toLowerCase()];
        if (!targetStackId) dropPins.push(targetAppId.toLowerCase());
        Config.options.dock.pinnedApps = Config.options.dock.pinnedApps.filter(
            id => !dropPins.includes(id.toLowerCase()));
    }

    // Replace a stack's member order wholesale (arc-reorder commit).
    function setStackOrder(stackId, members) {
        const parsed = root.stacksList();
        const s = parsed.find(x => x.id === stackId);
        if (!s) return;
        s.members = members;
        root.writeStacks(parsed);
    }

    // Reorder a pinned app within the pinnedApps array. targetPinnedIndex
    // is the desired final index (0-based) in Config.options.dock.pinnedApps.
    // The app is removed from its current position and spliced in at the
    // target; other pins shift to fill / make room.
    function reorderPinned(appId, targetPinnedIndex) {
        const pinned = Config.options.dock.pinnedApps.slice();
        const srcIdx = pinned.findIndex(id => id.toLowerCase() === appId.toLowerCase());
        if (srcIdx < 0 || srcIdx === targetPinnedIndex) return;
        pinned.splice(srcIdx, 1);
        pinned.splice(targetPinnedIndex, 0, appId);
        Config.options.dock.pinnedApps = pinned;
    }

    // Pull a member out of its stack back to a standalone pinned app.
    function unstackMember(stackId, appId) {
        root.removeFromStack(stackId, appId);
        if (!root.isPinned(appId)) root.togglePin(appId);
    }

    property list<var> apps: {
        var map = new Map();

        // Fan-out stacks come first, in their configured order. Their
        // member appIds are suppressed as standalone pinned entries below
        // so an app lives in one place: its stack.
        const stacks = root.stacksList();
        const stackedMembers = new Set();
        const memberToStackKey = new Map();
        for (const s of stacks) {
            for (const m of s.members) {
                stackedMembers.add(m.toLowerCase());
                memberToStackKey.set(m.toLowerCase(), "STACK:" + s.id);
            }
            map.set("STACK:" + s.id, {
                pinned: true, toplevels: [], isStack: true, members: s.members,
                name: s.name
            });
        }

        // Pinned apps (skip any already living in a stack)
        const pinnedApps = Config.options?.dock.pinnedApps ?? [];
        for (const appId of pinnedApps) {
            if (stackedMembers.has(appId.toLowerCase())) continue;
            if (!map.has(appId.toLowerCase())) map.set(appId.toLowerCase(), ({
                pinned: true,
                toplevels: []
            }));
        }

        // Separator
        if (map.size > 0) {
            map.set("SEPARATOR", { pinned: false, toplevels: [] });
        }

        // Ignored apps
        const ignoredRegexStrings = Config.options?.dock.ignoredAppRegexes ?? [];
        const ignoredRegexes = ignoredRegexStrings.map(pattern => new RegExp(pattern, "i"));
        // Open windows
        for (const toplevel of ToplevelManager.toplevels.values) {
            if (ignoredRegexes.some(re => re.test(toplevel.appId))) continue;
            // A running app that belongs to a stack contributes its windows
            // to the STACK entry (so tapping the member focuses the open
            // window) instead of appearing as a separate icon.
            const stackKey = memberToStackKey.get(toplevel.appId.toLowerCase());
            if (stackKey) {
                map.get(stackKey).toplevels.push(toplevel);
                continue;
            }
            if (!map.has(toplevel.appId.toLowerCase())) map.set(toplevel.appId.toLowerCase(), ({
                pinned: false,
                toplevels: []
            }));
            map.get(toplevel.appId.toLowerCase()).toplevels.push(toplevel);
        }

        var values = [];

        for (const [key, value] of map) {
            values.push(appEntryComp.createObject(null, {
                appId: value.isStack ? key.slice(6) : key,
                toplevels: value.toplevels,
                pinned: value.pinned,
                isStack: value.isStack ?? false,
                members: value.members ?? [],
                stackName: value.name ?? ""
            }));
        }

        return values;
    }

    component TaskbarAppEntry: QtObject {
        id: wrapper
        required property string appId
        required property list<var> toplevels
        required property bool pinned
        property bool isStack: false
        property list<var> members: []
        property string stackName: ""
    }
    Component {
        id: appEntryComp
        TaskbarAppEntry {}
    }
}
