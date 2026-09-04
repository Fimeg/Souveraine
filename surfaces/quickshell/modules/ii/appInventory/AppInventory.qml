// App inventory — installed-application knowledge projected for external
// consumers (the agent, the settings app, anything that needs to answer
// "what's installed" without parsing .desktop files ad hoc). See
// docs/tasks/app-inventory-manifest.md and docs/tasks/souveraine-shell-ecosystem.md.
//
// This is a QtObject instantiated inside AppInventoryScope.qml (as
// AppInvLocal.AppInventory), NOT a qs.services singleton. Same reason
// DockManifest is a local type: GlobalStates.qml imports qs.services, so a
// qs.services singleton that itself imports qs would form a circular import
// QML can't resolve. As a local type in its own dir it sidesteps that cycle.
// It carries its own imports explicitly — local types do NOT inherit the
// importing file's imports.
//
// Source of truth: ~/.local/share/applications/*.desktop (user) +
// /usr/share/applications/*.desktop (system). Standard Desktop Entry spec.
// We do NOT shell out per call: a single Process run scans both dirs,
// extracts each entry's key fields, and emits one record per app in a flat
// delimiter-separated format. JSON is built in JS from that, never in shell
// — the shell never quotes, so field values can't break the parse. The
// parsed list is cached on the object and refreshed on demand.
//
// User-dir entries override system-dir entries of the same appId (standard
// XDG behavior): the user dir is scanned first, and _ingest keeps the first
// occurrence of each appId.
//
// Every public method returns a result shape. Not-found is a result
// ({ok:false, reason:"not-found"}), never a throw. Inputs are validated.

import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root

    // Root is Item, not QtObject: AppInventory owns a Process child (the
    // desktop-file scanner), and QtObject has no default property to hold
    // children. Item gives us the default `data` property for free. This
    // object is non-visual (width/height 0, never painted) — Item is just
    // the lightest type that accepts children.

    // Unit separator (field) and record separator (categories within a field)
    // for the scanner's flat output. Chosen because they cannot appear in
    // .desktop file values in practice.
    readonly property string _US: "\x1f"
    readonly property string _RS: "\x1e"

    // The parsed inventory. Each entry:
    //   { appId, name, genericName, comment, icon, categories:[], noDisplay, path }
    // noDisplay entries are excluded from list/find/categories results, but
    // get() still resolves them if asked by explicit appId.
    property var _entries: []         // noDisplay filtered out, sorted
    property var _byId: ({})           // appId -> entry (all, incl noDisplay)
    property var _categoryIndex: ({})  // category -> [appId,...]
    property bool _loaded: false

    // --- Public API -----------------------------------------------------

    // Full inventory (noDisplay excluded). Optional filter:
    //   { category?: string }  -> only entries in that category.
    function list(filter) {
        root._ensureLoaded();
        const cat = filter && filter.category ? String(filter.category) : "";
        if (cat) {
            const ids = root._categoryIndex[cat] || [];
            const ents = ids.map(id => root._byId[id]).filter(Boolean);
            return { ok: true, count: ents.length, entries: ents };
        }
        return { ok: true, count: root._entries.length, entries: root._entries };
    }

    // IPC-friendly list: filter by a single category string (empty/blank =
    // no filter). IpcHandler can't marshal an object argument, so the IPC
    // surface exposes this instead of list(filter).
    function listFromCategory(category) {
        const cat = String(category ?? "").trim();
        if (!cat) {
            root._ensureLoaded();
            return { ok: true, count: root._entries.length, entries: root._entries };
        }
        root._ensureLoaded();
        const ids = root._categoryIndex[cat] || [];
        const ents = ids.map(id => root._byId[id]).filter(Boolean);
        return { ok: true, count: ents.length, entries: ents, category: cat };
    }

    // One entry by appId. not-found is a result, not an error.
    function get(appId) {
        root._ensureLoaded();
        const id = String(appId ?? "").trim();
        if (!id) return { ok: false, reason: "empty-app-id" };
        const entry = root._byId[id];
        if (!entry) return { ok: false, reason: "not-found", appId: id };
        return { ok: true, entry: entry };
    }

    // Fuzzy match across name/appId/comment. query: string. Returns ranked
    // results (best first), capped at limit (default 50, max 500).
    function find(query, limit) {
        root._ensureLoaded();
        const q = String(query ?? "").trim();
        if (!q) return { ok: false, reason: "empty-query" };
        const cap = Math.max(1, Math.min(500, parseInt(limit, 10) || 50));

        // fuzzysort targets. Each carries the entry plus prepared search keys.
        const targets = root._entries.map(e => ({
            obj: e,
            name: Fuzzy.prepare(e.name || e.appId),
            appId: Fuzzy.prepare(e.appId),
            comment: Fuzzy.prepare(e.comment || e.genericName || "")
        }));

        const opts = { all: false, limit: cap };
        const byName = Fuzzy.go(q, targets, Object.assign({ key: "name" }, opts));
        const byId = Fuzzy.go(q, targets, Object.assign({ key: "appId" }, opts));
        const byComment = Fuzzy.go(q, targets, Object.assign({ key: "comment" }, opts));

        // Merge and dedupe by appId; best weighted score wins. Name is the
        // strongest signal, appId next (often matches typed queries),
        // comment/genericName weakest.
        const merged = ({});
        const consider = (results, weight) => {
            for (const r of (results || [])) {
                const id = r.obj.obj.appId;
                const score = (r._score ?? 0) * weight;
                if (!merged[id] || merged[id].score < score) {
                    merged[id] = { entry: r.obj.obj, score: score };
                }
            }
        };
        consider(byName, 1.0);
        consider(byId, 0.9);
        consider(byComment, 0.6);

        const ranked = Object.values(merged)
            .sort((a, b) => b.score - a.score)
            .slice(0, cap)
            .map(m => m.entry);

        return { ok: true, count: ranked.length, query: q, entries: ranked };
    }

    // Distinct categories with member counts, sorted by count desc then name.
    function categories() {
        root._ensureLoaded();
        const cats = Object.keys(root._categoryIndex)
            .map(c => ({ category: c, count: (root._categoryIndex[c] || []).length }))
            .sort((a, b) => b.count - a.count || a.category.localeCompare(b.category));
        return { ok: true, count: cats.length, categories: cats };
    }

    // Force a rescan (e.g. after installing an app). Async: returns the
    // pre-refresh count; the new count lands when scanProc finishes and is
    // logged. Callers that need the fresh value should re-query after the
    // log line appears.
    function refresh() {
        root._loaded = false;
        root._entries = [];
        root._byId = ({});
        root._categoryIndex = ({});
        scanProc.running = true;
        return { ok: true, reason: "refreshing" };
    }

    // --- Internals ------------------------------------------------------

    function _ensureLoaded() {
        if (!root._loaded && !scanProc.running) scanProc.running = true;
    }

    // Parse the scanner's flat output into the inventory. One line per app,
    // fields joined by unit separator (\x1f) in order:
    //   appId, name, genericName, comment, icon, categories, path
    // categories is itself record-separated (\x1e). noDisplay is encoded as
    // a "1:" prefix on the appId field so it survives the flat format
    // without an extra column.
    function _ingest(text) {
        const US = root._US;
        const RS = root._RS;
        const lines = (text || "").split("\n");
        const all = [];
        for (const raw of lines) {
            const line = raw.replace(/\r$/, "");
            if (!line) continue;
            const f = line.split(US);
            if (f.length < 6) continue;
            const rawAppId = f[0];
            if (!rawAppId) continue;
            let noDisplay = false;
            let appId = rawAppId;
            if (appId.startsWith("1:")) { noDisplay = true; appId = appId.slice(2); }
            all.push({
                appId: appId,
                name: f[1] || appId,
                genericName: f[2] || "",
                comment: f[3] || "",
                icon: f[4] || "",
                categories: (f[5] || "").split(RS).map(s => s.trim()).filter(Boolean),
                noDisplay: noDisplay,
                path: f[6] || ""
            });
        }

        // Dedupe by appId, first occurrence wins (user dir scanned first).
        const byId = ({});
        const entries = [];
        const categoryIndex = ({});
        for (const e of all) {
            if (byId[e.appId]) continue;
            byId[e.appId] = e;
            if (!e.noDisplay) {
                entries.push(e);
                for (const c of e.categories) {
                    if (!categoryIndex[c]) categoryIndex[c] = [];
                    categoryIndex[c].push(e.appId);
                }
            }
        }

        // Deterministic order: name, then appId.
        entries.sort((a, b) =>
            (a.name || "").localeCompare(b.name || "") || a.appId.localeCompare(b.appId));

        root._byId = byId;
        root._entries = entries;
        root._categoryIndex = categoryIndex;
        root._loaded = true;
    }

    // The scanner. One bash invocation walks both dirs and runs awk per
    // .desktop file. awk extracts the fields from the [Desktop Entry] group
    // and joins them with \x1f (and categories with \x1e). The shell does
    // NO JSON and NO quoting — field values pass through verbatim into the
    // delimiter-separated stream, so values can't break the parse.
    Process {
        id: scanProc
        command: ["bash", "-c", root._scanScript()]
        stdout: StdioCollector {
            onStreamFinished: {
                root._ingest(this.text);
                console.log("[appInventory] loaded", root._entries.length, "apps,",
                            Object.keys(root._categoryIndex).length, "categories");
            }
        }
    }

    // User dir first so user entries win the first-occurrence dedupe in
    // _ingest (standard XDG override semantics).
    //
    // One awk process per .desktop file is fine (170-ish files, trivial). The
    // earlier bug was calling awk WITHOUT passing the file, so it read stdin
    // and hung — `"$f"` MUST be the trailing arg so awk reads the file.
    function _scanScript() {
        return `
US=$(printf '\\037')
RS=$(printf '\\036')
export US RS
for d in "$HOME/.local/share/applications" /usr/share/applications; do
  [ -d "$d" ] || continue
  for f in "$d"/*.desktop; do
    [ -f "$f" ] || continue
    appId=$(basename "$f" .desktop)
    awk -v appId="$appId" -v USC="$US" -v RSC="$RS" -v fpath="$f" '
      BEGIN { inE=0; name=gn=comment=icon=cats=nd="" }
      /^\\[Desktop Entry\\]/ { inE=1; next }
      /^\\[/ { inE=0 }
      inE && /^Name=/        { name=substr($0, 6) }
      inE && /^GenericName=/ { gn=substr($0, 13) }
      inE && /^Comment=/     { comment=substr($0, 9) }
      inE && /^Icon=/        { icon=substr($0, 6) }
      inE && /^Categories=/  { cats=substr($0, 12) }
      inE && /^NoDisplay=/   { nd=substr($0, 11) }
      END {
        n = split(cats, a, ";"); catOut=""
        for (i=1;i<=n;i++) { if(a[i]=="")continue; if(catOut!="")catOut=catOut RSC; catOut=catOut a[i] }
        if (nd=="true") appId="1:" appId
        print appId USC name USC gn USC comment USC icon USC catOut USC fpath
      }
    ' "$f"
  done
done
`;
    }
}
