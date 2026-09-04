// App inventory surface — holds the AppInventory projection and the IPC
// method surface the agent (via Souveraine's harness) calls. No UI; this is
// a service-only Scope, sibling in shape to how Dock.qml hosts DockManifest
// + its IpcHandler. See docs/tasks/app-inventory-manifest.md.
//
// Instantiated from panelFamilies/SouveraineFamily.qml. The AppInventory
// type lives here as a local type (not as a qs.services singleton) to avoid
// the GlobalStates circular import — see AppInventory.qml's header comment.

import qs
import qs.modules.common
import "." as AppInvLocal
import QtQuick
import Quickshell
import Quickshell.Io

Scope {
    // The inventory projection + parse/cache machinery. Carries its own
    // imports (qs, qs.services, qs.modules.common.functions) because local
    // types do NOT inherit this file's imports.
    AppInvLocal.AppInventory { id: appInventory }

    // Read-only method surface. Every method delegates to appInventory and
    // returns its result shape ({ok, ...}). Refusals/not-founds are results,
    // not errors, so a caller learns from them instead of guessing.
    //
    // IPC type constraint: quickshell's IpcHandler can only marshal declared
    // primitive types across the wire — untyped args become QVariant and are
    // rejected ("cannot be used across IPC"). So every parameter has an
    // explicit type. The inventory's list(filter) takes an object in-process;
    // over IPC we expose list(category: string) and build the filter here.
    // The same five-type limit applies to RETURNS, not just arguments: a `var`
    // return is mapped to VOID and the payload is dropped without an error
    // (src/io/ipc.cpp ipcType(); "void and var get mixed by qml engine"). These
    // were declared `: var` and so registered as `(): void` — every call
    // returned nothing. Returning JSON as a string is what actually crosses.
    IpcHandler {
        target: "apps"

        // apps.list()                 -> all (noDisplay excluded)
        // apps.list("Network")        -> only entries in the "Network" category
        function list(category: string): string {
            return JSON.stringify(appInventory.listFromCategory(category));
        }

        // apps.get("firefox") -> {ok, entry?} or {ok:false, reason:"not-found"}
        function get(appId: string): string {
            return JSON.stringify(appInventory.get(appId));
        }

        // apps.find("fire")           -> fuzzy-ranked entries (default limit 50)
        // apps.find("fire", 10)       -> capped at 10
        function find(query: string, limit: int): string {
            return JSON.stringify(appInventory.find(query, limit));
        }

        // apps.categories() -> {ok, categories:[{category,count}]}
        function categories(): string {
            return JSON.stringify(appInventory.categories());
        }

        // apps.refresh() -> trigger a rescan (async; re-query after the log line)
        function refresh(): string {
            return JSON.stringify(appInventory.refresh());
        }
    }
}
