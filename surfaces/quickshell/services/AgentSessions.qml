pragma Singleton
pragma ComponentBehavior: Bound

import qs
import qs.modules.common
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Io

/**
 * AgentSessions — one spine for agent activity. TASK-69.
 *
 * Every agent session on this machine, across providers (Souveraine, Claude
 * Code, Codex), from ONE collector on ONE cadence. Surfaces render projections
 * of this; nothing fetches per-provider any more.
 *
 * WHY THIS EXISTS
 * The bar used to own a fetch per provider — ClaudeUsage polls OAuth, a Codex
 * panel would have polled its own JSONL, and the substrate's own sessions were
 * read by nothing at all. Three owners of one question is the failure mode this
 * project keeps rediscovering (see: hypridle vs the idle rule, compositor vs
 * sessiond over the panel). One authority; everything else a rendering.
 *
 * TRANSPORT IS NOT THE CONTRACT
 * Today the source is scripts/agent/agent-sessions.sh, polled. TASK-69's Rust
 * daemon will serve the identical envelope over
 * $XDG_RUNTIME_DIR/souveraine-sessions.sock. When it lands, only `collector`
 * below changes — every property, every consumer, stays put. Do not let surface
 * code reach past these properties into the JSON shape.
 *
 * DEGRADATION
 * A failed collection NEVER blanks the model. Last-good data is retained and
 * `stale` goes true. A bar that empties on a transient failure reads to the
 * human as "nothing is running", which is a lie; a bar that dims and says
 * "stale" tells the truth. Same reasoning as the collector's always-exit-0.
 */
Singleton {
    id: root

    // Gate: absent config (older ii-base pin) must not disable the service
    // silently — default on, because the cost is one 0.5s subprocess a minute.
    readonly property bool enabled: Config.options?.bar?.agentSessions?.enable ?? true
    readonly property int refreshInterval: (Config.options?.bar?.agentSessions?.refreshInterval ?? 60) * 1000

    // ---- the envelope, projected -------------------------------------------
    property var sessions: []          // sorted by age ascending (newest first)
    property int activeCount: 0        // sessions touched in the last 2 minutes
    property var providers: ({})       // per-provider availability + counts
    property double lastUpdate: 0

    property bool available: false     // have we EVER collected successfully
    property bool stale: false         // last attempt failed, showing old data
    property string lastError: ""

    // ---- derived, for compact surfaces --------------------------------------

    // The one session a narrow surface should show. Not simply "newest":
    // an active session outranks a merely recent one regardless of age, because
    // "something is happening right now" is the thing a glance needs to answer.
    readonly property var primarySession: {
        if (root.sessions.length === 0)
            return null;
        const act = root.sessions.filter(s => s.state === "active");
        return act.length > 0 ? act[0] : root.sessions[0];
    }

    readonly property bool anyActive: root.activeCount > 0

    // Codex reports live rate limits inside its session records; Claude's
    // subscription window still comes from ClaudeUsage (its own OAuth poll).
    // Stage 3 of TASK-69 folds that poll in here too — until then this property
    // is honest about covering only what the collector actually sees.
    readonly property var codexLimits: root.providers?.codex?.limits ?? null

    function providerLabel(p) {
        switch (p) {
        case "souveraine": return "Souveraine";
        case "claude":     return "Claude Code";
        case "codex":      return "Codex";
        default:           return p;
        }
    }

    function providerIcon(p) {
        switch (p) {
        case "souveraine": return "psychology";
        case "claude":     return "auto_awesome";
        case "codex":      return "terminal";
        default:           return "smart_toy";
        }
    }

    // Short human label for a session — the project directory if we have one,
    // else the configured agent name plus a conversation discriminator, else
    // a truncated id. Never an empty string: a blank chip is indistinguishable
    // from a broken one, and ten rows all called "agent" are only technically
    // non-blank.
    function sessionLabel(s) {
        if (!s)
            return "";
        if (s.cwd && s.cwd.length > 0)
            return s.cwd.split("/").filter(x => x.length > 0).pop() ?? s.cwd;
        if (s.provider === "souveraine") {
            const name = s.agentName && s.agentName.length > 0
                ? s.agentName : "Souveraine";
            const id = (s.id ?? "").slice(0, 4);
            return s.subconscious
                ? `${name} · subconscious · ${id}`
                : `${name} · ${id}`;
        }
        return (s.id ?? "").slice(0, 8);
    }

    // Total tokens for a session, or -1 when the provider genuinely does not
    // report them. -1 is deliberate: Souveraine persists per-turn TokenUsage,
    // but the conversation.json metadata inspected by the presence collector
    // does not aggregate it. A 0 here would be a measurement claim we cannot
    // back. Surfaces must render -1 as "—", never as zero.
    function sessionTokens(s) {
        if (!s)
            return -1;
        if (s.tokensIn < 0 || s.tokensOut < 0)
            return -1;
        return s.tokensIn + s.tokensOut;
    }

    function refresh() {
        if (!root.enabled)
            return;
        collector.running = false;
        collector.running = true;
    }

    Process {
        id: collector
        // Path convention matches WallpaperDownload/ConflictKiller: shellPath
        // returns a file:// URL, which Process will not exec. Window minutes is
        // passed through so the config owns "how far back counts as a session"
        // rather than the script hardcoding it.
        command: [
            FileUtils.trimFileProtocol(Quickshell.shellPath("scripts/agent/agent-sessions.sh")),
            "--window-min",
            String(Config.options?.bar?.agentSessions?.windowMinutes ?? 1440)
        ]

        stdout: StdioCollector {
            onStreamFinished: {
                const raw = text.trim();
                if (raw.length === 0) {
                    // The collector contracts to always emit one line. Empty
                    // means it did not run at all (missing, not executable) —
                    // which is a different failure from "ran and found nothing",
                    // so do not let it look like an empty session list.
                    root.stale = true;
                    root.lastError = "collector produced no output";
                    retryTimer.restart();
                    return;
                }
                try {
                    const d = JSON.parse(raw);
                    root.sessions = d.sessions ?? [];
                    root.activeCount = d.active ?? 0;
                    root.providers = d.providers ?? ({});
                    root.lastUpdate = (d.ts ?? 0) * 1000;
                    root.available = true;
                    root.stale = false;
                    root.lastError = "";
                } catch (e) {
                    root.stale = true;
                    root.lastError = e.message;
                    retryTimer.restart();
                    console.error(`[AgentSessions] parse failed: ${e.message}`);
                }
            }
        }
    }

    Timer {
        running: root.enabled
        repeat: true
        interval: root.refreshInterval
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    // Cold start / transient: retry quickly rather than showing "unavailable"
    // until the next full interval. Mirrors ClaudeUsage's ladder deliberately —
    // same problem, same answer, so the two behave alike when both are stale.
    Timer {
        id: retryTimer
        interval: 15000
        repeat: false
        onTriggered: if (root.enabled && root.stale) root.refresh()
    }
}
