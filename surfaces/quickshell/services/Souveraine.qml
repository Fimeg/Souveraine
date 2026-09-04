pragma Singleton
pragma ComponentBehavior: Bound

import qs.modules.common.functions as CF
import qs.modules.common
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick

/**
 * Souveraine — the substrate singleton for every shell module.
 *
 * This is the one connection to the Souveraine server. The chat sidebar,
 * presence widget, cockpit pane, agent manager and settings module all hang
 * off this service; none of them open their own transport. Ai.qml is the
 * ii-compat adapter over this for the existing sidebar UI.
 *
 * Responsibilities:
 *  - agent inventory (GET /v1/agents)
 *  - conversation lifecycle (create and server-derived resume)
 *  - the SSE turn stream — raw events re-emitted via streamEvent(var)
 *  - the backchannel: cancelTurn() and interject(text)
 *  - the desktop sensorium: every send carries ambient context (active
 *    window, open apps, cursor position) so she perceives the room she is
 *    being spoken to in. Extension point for device sensors (SouveraineOS).
 */
Singleton {
    id: root

    property string serverBase: Config.options?.ai?.souveraineUrl ?? "http://127.0.0.1:8484"
    property bool serverUp: false
    // Ambient perception on by default; ai.ambient=false in ii config disables.
    property bool ambientEnabled: Config.options?.ai?.ambient ?? true
    // Start `souveraine server` ourselves when it isn't running. The surface
    // is the OS frontend — opening it means summoning her, not staring at a
    // connection error. ai.souveraineAutostart=false disables; a manual
    // start affordance can call startServer() directly.
    property bool autostartEnabled: Config.options?.ai?.souveraineAutostart ?? true
    property string serverBin: Config.options?.ai?.souveraineBin ?? "souveraine"
    property bool _autostartTried: false

    // id -> { name, description }
    property var agents: ({})
    property var agentList: Object.keys(agents)
    property string currentAgentId: ""
    // Structured read-only projection of the current agent's canonical
    // itinerary. The memfs file remains the owner; this survives a pane/shell
    // reload by asking the substrate instead of keeping a ribbon-local copy.
    property string itineraryAgentId: ""
    property var itinerary: ({
        "exists": false,
        "active": false,
        "title": "",
        "current": 0,
        "route": "",
        "stops": []
    })
    property bool itineraryStale: false
    // On agent (re)establishment — shell start, reboot, agent switch — we look
    // up her latest server-persisted conversation. What we do with it depends
    // on autoResume.
    //
    //   autoResume false (default): we *offer* it. `resumeOffered` fires with
    //     the thread's id and metadata; nothing is attached. Doing nothing
    //     starts fresh, which is what a reload should do. Continuing is one
    //     deliberate act (acceptOfferedResume), not a default.
    //   autoResume true: legacy behaviour — attach it silently.
    //
    // The distinction matters because the shell reloads often (a deploy, a
    // lock, a crash) and silent re-attachment makes every one of those look
    // like a continuation of a conversation the human may have finished with.
    // Explicit resume paths (/resume, the Face control) are unaffected: they
    // call resumeLatestConversation() with no argument and still load.
    //
    // Note this is NOT the amnesia fix from e4e6594 — that one stopped
    // selectAgent clearing conversationId on an unchanged agent, and stays.
    // Gated so it never clobbers a live turn or an already-attached thread.
    property bool autoResume: Config.options?.ai?.autoResume ?? false

    // Emitted when a resumable thread exists and we chose not to attach it.
    // agentId/conversationId identify it; the rest is for drawing the offer.
    signal resumeOffered(string agentId, string conversationId, string title, string updatedAt)
    property string offeredConversationId: ""
    property string offeredConversationTitle: ""
    property string offeredConversationUpdatedAt: ""
    property var conversations: []
    property bool conversationsLoading: false
    property bool conversationsStale: false

    onCurrentAgentIdChanged: {
        if (root.currentAgentId.length > 0 && root.serverUp
            && root.conversationId.length === 0 && !root.turnActive) {
            root.resumeLatestConversation(!root.autoResume);
        }
        if (root.currentAgentId.length > 0 && root.serverUp)
            Qt.callLater(root.refreshItinerary);
        else
            root._clearItinerary();
    }
    onServerUpChanged: if (root.serverUp && root.currentAgentId.length > 0)
        Qt.callLater(root.refreshItinerary)
    property string conversationId: ""
    property bool turnActive: false

    // ── Turn clock ───────────────────────────────────────────────────────
    // Wall-clock for the request in flight. Lives here because turnActive
    // does; the chat surface and the pill both read it. `turnStartedAt` is
    // when we handed the request to curl (ms epoch, 0 = nothing sent yet),
    // `turnElapsedMs` ticks while the turn runs and freezes at the total.
    property double turnStartedAt: 0
    property int turnElapsedMs: 0

    Timer {
        running: root.turnActive
        interval: 100
        repeat: true
        onTriggered: root.turnElapsedMs = Date.now() - root.turnStartedAt
    }

    /* Raw wire events (message_type-tagged objects from the SSE stream). */
    signal streamEvent(var event)
    /* Stream closed (process exit). exitCode 0 = clean. */
    signal streamClosed(int exitCode)
    signal agentsRefreshed()
    signal serverUnreachable()
    // Emitted after a server-owned conversation has been selected and its
    // persisted transcript loaded for the active surface.
    signal conversationResumed(string agentId, string conversationId, var messages)
    // Emitted when a send is blocked by step-up auth. The UI should call
    // StepUpAuth.requestAuth("send", callback) and retry on success.
    signal stepUpRequired(string actionFamily, string queuedText)

    // ── Agent inventory ──────────────────────────────────────────────────
    Process {
        id: getAgents
        running: true
        command: ["curl", "-sf", "--max-time", "3", `${root.serverBase}/v1/agents`]
        stdout: StdioCollector {
            onStreamFinished: {
                if (text.length === 0) return;
                try {
                    const list = JSON.parse(text);
                    const map = {};
                    // voice_id comes from the agent's `_souveraine` block via
                    // the public list endpoint. None/absent = use the system
                    // voice. Speech reads this per active agent.
                    list.forEach(a => { map[a.id] = { "name": a.name, "description": a.description ?? "", "voice_id": a.voice_id ?? "" }; });
                    root.agents = map;
                    root.agentList = Object.keys(map);
                    root.serverUp = true;
                    if (!root.agents[root.currentAgentId] && root.agentList.length > 0) {
                        root.currentAgentId = root.agentList[0];
                    }
                    root.agentsRefreshed();
                } catch (e) {
                    console.log("[Souveraine] Could not parse agent list:", e);
                }
            }
        }
        onExited: (exitCode) => {
            if (exitCode !== 0) {
                root.serverUp = false;
                if (root.autostartEnabled && !root._autostartTried) {
                    root.startServer();
                } else {
                    root.serverUnreachable();
                }
            }
        }
    }

    function refreshAgents() {
        getAgents.running = true;
    }

    // ── Persistent itinerary projection ────────────────────────────────
    Process {
        id: getItinerary
        property string agentId: ""

        stdout: StdioCollector {
            onStreamFinished: {
                if (getItinerary.agentId !== root.currentAgentId) return;
                if (text.length === 0) {
                    root.itineraryStale = true;
                    return;
                }
                try {
                    root.itinerary = JSON.parse(text);
                    root.itineraryAgentId = getItinerary.agentId;
                    root.itineraryStale = false;
                } catch (e) {
                    root.itineraryStale = true;
                    console.log("[Souveraine] Could not parse itinerary:", e);
                }
            }
        }

        onExited: exitCode => {
            if (exitCode !== 0 && getItinerary.agentId === root.currentAgentId)
                root.itineraryStale = true;
        }
    }

    function _clearItinerary() {
        root.itineraryAgentId = root.currentAgentId;
        root.itinerary = ({
            "exists": false,
            "active": false,
            "title": "",
            "current": 0,
            "route": "",
            "stops": []
        });
        root.itineraryStale = false;
    }

    function refreshItinerary() {
        const agentId = root.currentAgentId;
        if (!root.serverUp || agentId.length === 0) {
            root._clearItinerary();
            return false;
        }
        if (root.itineraryAgentId !== agentId)
            root._clearItinerary();
        getItinerary.running = false;
        getItinerary.agentId = agentId;
        getItinerary.command = ["bash", "-c",
            root._tokenReadLine(agentId)
            + `curl -sf --max-time 5 "${root.serverBase}/v1/agents/${agentId}/itinerary"`
            + ` -H "Authorization: Bearer $TOKEN"`
        ];
        getItinerary.running = true;
        return true;
    }

    // The inventory used to be fetched exactly once, at shell start, so an
    // agent created afterwards stayed invisible until the whole shell was
    // reloaded. One curl a minute is cheaper than that surprise. Skipped
    // while a turn is in flight so a slow local model isn't competing with
    // polling for the server's attention.
    Timer {
        interval: 60000
        repeat: true
        running: true
        onTriggered: if (!root.turnActive) root.refreshAgents()
    }

    // ── Server autostart ─────────────────────────────────────────────────
    // systemd user unit first (survives shell restarts, journald logging);
    // bare nohup fallback for systems without it. One attempt per shell
    // session — a broken install shouldn't spawn-loop.
    Process {
        id: serverStarter
        command: ["bash", "-c",
            `if command -v systemctl >/dev/null && systemctl --user list-unit-files souveraine.service &>/dev/null; then
                systemctl --user start souveraine.service
            else
                nohup ${root.serverBin} server >/dev/null 2>&1 &
            fi`]
        onExited: {
            serverRetryTimer.start();
        }
    }

    Timer {
        id: serverRetryTimer
        interval: 2500
        repeat: false
        onTriggered: root.refreshAgents()
    }

    function startServer() {
        if (root._autostartTried) return;
        root._autostartTried = true;
        console.log("[Souveraine] server not reachable — starting it");
        serverStarter.running = true;
    }

    function selectAgent(agentId) {
        if (!root.agents[agentId]) return false;
        // Re-affirming the agent that is already active is a no-op, and it has
        // to be. Ai.qml re-selects the persisted agent on every agentsRefreshed,
        // and the inventory poll above fires that once a minute, forever. While
        // this function cleared conversationId unconditionally, every message
        // sent more than a minute after the previous one opened a NEW
        // conversation and therefore arrived with no history at all.
        //
        // Measured on the phone 2026-07-31: 27 conversations for one agent in a
        // day, all but two exactly four messages long — system prompt, ambient,
        // user, reply. One exchange each. That is the whole of the "she doesn't
        // remember what I just said" report, and it is not the turn loop: the
        // server assembles history from session.messages correctly, and there
        // was simply never more than one exchange in a session to assemble.
        //
        // Switching agents is a decision. Polling is not.
        if (agentId === root.currentAgentId) return true;
        // A live turn belongs to the current conversation. Switching beneath
        // it would render one agent's response in another agent's surface.
        if (root.turnActive) return false;
        // Clear before the id changes, so onCurrentAgentIdChanged observes an
        // empty conversation and re-attaches the incoming agent's own latest
        // thread rather than leaving her on a blank one.
        listConversations.running = false;
        root.conversationId = "";
        root.dismissOfferedResume();
        root.conversations = [];
        root.conversationsStale = false;
        root.currentAgentId = agentId;
        return true;
    }

    function newConversation() {
        root.conversationId = "";
        root.dismissOfferedResume();
    }

    // ── Server-derived resume ───────────────────────────────────────────
    // The GUI keeps no per-agent conversation map. The server is the source
    // of truth: it persists conversations under each agent and this query
    // hydrates them after a server restart before returning the latest one.
    Process {
        id: listConversations
        property string agentId: ""
        // Set by resumeLatestConversation(true): announce the thread rather
        // than loading it.
        property bool offerOnly: false
        // Set by refreshConversations(): update the footer picker without
        // attaching to or offering any thread.
        property bool listOnly: false
        stdout: StdioCollector {
            onStreamFinished: {
                if (listConversations.agentId !== root.currentAgentId) return;
                // Empty stdout is a FAILED request, not an empty agent: curl -sf
                // writes nothing on 4xx/5xx. Conflating the two clears
                // conversationId and hands the surface an empty transcript to
                // render, so one transient hiccup wipes the visible thread and
                // orphans the live one. Only a parsed response is authoritative.
                if (text.length === 0) {
                    root.conversationsStale = true;
                    console.log("[Souveraine] empty conversation list response — leaving current conversation in place");
                    return;
                }
                let conversations = [];
                try {
                    conversations = JSON.parse(text);
                } catch (e) {
                    root.conversationsStale = true;
                    console.log("[Souveraine] Could not parse conversation list:", e);
                    return;
                }
                root.conversations = conversations;
                root.conversationsStale = false;
                if (listConversations.listOnly) return;
                if (conversations.length === 0) {
                    // The server answered, and the answer is "none yet".
                    root.conversationId = "";
                    root.conversationResumed(root.currentAgentId, "", []);
                    return;
                }
                // Server sorts by updated_at descending — [0] is her latest.
                const latest = conversations[0];
                if (listConversations.offerOnly) {
                    // Announce, don't attach. conversationId stays empty, so a
                    // send without accepting mints a fresh thread on purpose.
                    root.offeredConversationId = latest.id;
                    root.offeredConversationTitle = latest.title ?? "";
                    root.offeredConversationUpdatedAt = latest.updated_at ?? "";
                    root.resumeOffered(listConversations.agentId, latest.id,
                                       latest.title ?? "", latest.updated_at ?? "");
                    return;
                }
                root._loadConversation(listConversations.agentId, latest.id);
            }
        }
        onExited: exitCode => {
            root.conversationsLoading = false;
            // A failed list fetch is transient (server busy, network blip).
            // Do NOT clear conversationId — that orphans the live thread and
            // forces the next send to mint a fresh conversation. Log and leave
            // state alone; the next refresh or /resume retries.
            if (exitCode !== 0 && listConversations.agentId === root.currentAgentId) {
                root.conversationsStale = true;
                console.log("[Souveraine] conversation list fetch failed (exit " + exitCode + ") — leaving current conversation in place");
            }
        }
    }

    Process {
        id: loadConversation
        property string agentId: ""
        property string requestedConversationId: ""
        stdout: StdioCollector {
            onStreamFinished: {
                if (loadConversation.agentId !== root.currentAgentId) return;
                // Same rule as the list above: no body means the fetch failed.
                // Attaching to the id anyway and announcing an empty transcript
                // would blank the surface while claiming the thread is loaded.
                if (text.length === 0) {
                    console.log("[Souveraine] empty transcript response — leaving current conversation in place");
                    return;
                }
                try {
                    const messages = JSON.parse(text);
                    root.conversationId = loadConversation.requestedConversationId;
                    root.conversationResumed(root.currentAgentId, root.conversationId, messages);
                    // If the shell reloaded in the middle of a turn, the POST
                    // socket that started it is gone but the turn belongs to
                    // the server and is still running. Replay its journal and
                    // follow the live tail without posting another message.
                    Qt.callLater(root._reattachActiveTurn);
                } catch (e) {
                    console.log("[Souveraine] Could not parse conversation transcript:", e);
                }
            }
        }
        onExited: exitCode => {
            // Transient failure (e.g. a 5xx on GET /messages). Don't clear —
            // see listConversations.onExited. Leaving conversationId alone keeps
            // an already-attached thread reachable instead of forcing a new one.
            if (exitCode !== 0 && loadConversation.agentId === root.currentAgentId) {
                console.log("[Souveraine] conversation transcript fetch failed (exit " + exitCode + ") — leaving current conversation in place");
            }
        }
    }

    // offerOnly: fetch the latest thread but announce it instead of attaching.
    // Defaults to false so every existing caller keeps its old behaviour.
    function resumeLatestConversation(offerOnly) {
        if (!root.serverUp || root.currentAgentId.length === 0 || root.turnActive
            || listConversations.running) return false;
        listConversations.offerOnly = (offerOnly === true);
        listConversations.listOnly = false;
        listConversations.agentId = root.currentAgentId;
        listConversations.command = [
            "curl", "-sf", "--max-time", "5",
            `${root.serverBase}/v1/conversations?agent_id=${encodeURIComponent(root.currentAgentId)}`
        ];
        root.conversationsLoading = true;
        listConversations.running = true;
        return true;
    }

    function refreshConversations() {
        if (!root.serverUp || root.currentAgentId.length === 0
            || listConversations.running) return false;
        listConversations.offerOnly = false;
        listConversations.listOnly = true;
        listConversations.agentId = root.currentAgentId;
        listConversations.command = [
            "curl", "-sf", "--max-time", "5",
            `${root.serverBase}/v1/conversations?agent_id=${encodeURIComponent(root.currentAgentId)}`
        ];
        root.conversationsLoading = true;
        listConversations.running = true;
        return true;
    }

    // Take up an offer made by resumeOffered. No-op if the offer has gone
    // stale (a turn started, or something else attached in the meantime).
    function acceptOfferedResume() {
        if (root.offeredConversationId.length === 0 || root.turnActive) return false;
        if (root.conversationId.length > 0) return false;
        root._loadConversation(root.currentAgentId, root.offeredConversationId);
        root.dismissOfferedResume();
        return true;
    }

    // Decline. The thread stays on the server; we simply start fresh.
    function dismissOfferedResume() {
        root.offeredConversationId = "";
        root.offeredConversationTitle = "";
        root.offeredConversationUpdatedAt = "";
    }

    function loadConversationById(conversationId) {
        if (!root.serverUp || root.currentAgentId.length === 0 || root.turnActive
            || conversationId.length === 0) return false;
        root.dismissOfferedResume();
        root._loadConversation(root.currentAgentId, conversationId);
        return true;
    }

    function _loadConversation(agentId, conversationId) {
        loadConversation.agentId = agentId;
        loadConversation.requestedConversationId = conversationId;
        loadConversation.command = ["bash", "-c",
            root._tokenReadLine(agentId)
            + `curl -sf --max-time 10 "${root.serverBase}/v1/conversations/${conversationId}/messages"`
            + ` -H "Authorization: Bearer $TOKEN"`
        ];
        loadConversation.running = true;
    }

    Process {
        id: reattachRequester
        property bool receivedEvent: false
        stdout: SplitParser {
            onRead: data => {
                if (data.length === 0 || !data.startsWith("data:")) return;
                let event;
                try {
                    event = JSON.parse(data.slice(5).trim());
                } catch (e) {
                    console.log("[Souveraine] Unparseable replay SSE line:", data);
                    return;
                }
                reattachRequester.receivedEvent = true;
                root.streamEvent(event);
            }
        }
        onExited: exitCode => {
            if (root.turnStartedAt > 0)
                root.turnElapsedMs = Date.now() - root.turnStartedAt;
            root.turnActive = false;
            // A 409 means there was no turn to recover. It is an ordinary
            // resume, not a failed response and must not finish a blank card.
            if (reattachRequester.receivedEvent)
                root.streamClosed(exitCode);
        }
    }

    function _reattachActiveTurn() {
        if (root.conversationId.length === 0 || requester.running || reattachRequester.running)
            return;
        reattachRequester.receivedEvent = false;
        reattachRequester.command = ["bash", "-c",
            root._tokenReadLine(root.currentAgentId)
            + `curl --no-buffer -sf "${root.serverBase}/v1/conversations/${root.conversationId}/events"`
            + ` -H "Authorization: Bearer $TOKEN"`
        ];
        root.turnStartedAt = Date.now();
        root.turnElapsedMs = 0;
        root.turnActive = true;
        reattachRequester.running = true;
    }

    // ── Ambient sensorium ────────────────────────────────────────────────
    // What the desktop feels like at the moment of speaking. Cheap,
    // synchronous reads here; the cursor needs a hyprctl round-trip and is
    // collected in the send chain. Device sensors (SouveraineOS positional
    // data) extend collectAmbient().
    property string _cursorPos: ""

    function collectAmbient() {
        if (!root.ambientEnabled) return "";
        const lines = [];
        const active = ToplevelManager.activeToplevel;
        if (active) {
            lines.push(`active window: ${active.appId ?? "?"} — "${active.title ?? ""}"`);
        }
        const tops = ToplevelManager.toplevels?.values ?? [];
        if (tops.length > 0) {
            const apps = tops.map(t => t.appId).filter(Boolean);
            const counts = {};
            apps.forEach(a => counts[a] = (counts[a] ?? 0) + 1);
            const summary = Object.entries(counts)
                .map(([app, n]) => n > 1 ? `${app} (${n})` : app)
                .join(", ");
            lines.push(`open: ${summary}`);
        }
        if (root._cursorPos.length > 0) {
            lines.push(`cursor: ${root._cursorPos}`);
        }
        // Joining the face loads its skill; leaving unloads it. Attached here
        // because ambient is already the per-send block the surface owns, so
        // the cost of not being joined is exactly zero tokens rather than a
        // flag the server has to remember to check.
        if (typeof Face !== "undefined" && Face.joined) {
            lines.push(Face.skill);
        }
        if (typeof HidController !== "undefined" && HidController.active) {
            lines.push(HidController.skill);
        }
        return lines.join("\n");
    }

    // Compositor-specific cursor read. hyprctl on Hyprland, kdotool on KDE;
    // anything else just skips the cursor line — ambient degrades gracefully,
    // it never blocks the send.
    //
    // 2026-08-12: `command -v hyprctl` tests whether the tool is *installed*,
    // not whether it *answered*. On the phone hyprctl is present (a Lua-eval
    // shim) but there is no Hyprland under viewtop, so it printed
    // "HYPRLAND_INSTANCE_SIGNATURE not set!" and StdioCollector stored that
    // sentence as the cursor position — shipped in every ambient block, every
    // turn. Validate the shape of the reply; an answer that isn't a
    // coordinate pair is not an answer.
    Process {
        id: cursorProc
        command: ["bash", "-c",
            `if command -v hyprctl >/dev/null; then hyprctl cursorpos 2>/dev/null;
             elif command -v kdotool >/dev/null; then kdotool getmouselocation 2>/dev/null;
             fi`]
        stdout: StdioCollector {
            onStreamFinished: {
                const reply = text.trim();
                // "1234, 567" from hyprctl; kdotool's x:N y:N is normalised
                // by the caller below. Anything else is discarded.
                const pair = reply.match(/^(-?\d+)\s*,\s*(-?\d+)$/);
                const kde = reply.match(/x:\s*(-?\d+)\s+y:\s*(-?\d+)/);
                if (pair) {
                    root._cursorPos = `${pair[1]}, ${pair[2]}`;
                } else if (kde) {
                    root._cursorPos = `${kde[1]}, ${kde[2]}`;
                } else {
                    root._cursorPos = "";
                }
            }
        }
        onExited: {
            root._ensureConversationThenRequest();
        }
    }

    // ── Send chain: cursor → conversation → stream ───────────────────────
    property string _queuedText: ""

    /* Send a user message with ambient context. Returns false if the
       server is down or no agent is selected. Returns "step-up" if
       step-up auth is required but no valid grant exists — the caller
       should trigger StepUpAuth.requestAuth("send") and retry. */
    function send(text) {
        if (!root.serverUp || root.currentAgentId.length === 0) return false;
        if (text.length === 0) return false;
        if (root.turnActive) return false;
        // Step-up gate: if enabled and no valid send grant exists, block
        // the send and emit a signal so the UI can trigger auth + retry.
        // Break-glass grants bypass normal step-up — they are one-time,
        // short-lived, and journaled.
        if (Config.options?.lock?.stepUp?.enabled
            && typeof StepUpAuth !== "undefined"
            && !StepUpAuth.isGranted("send")
            && !StepUpAuth.isBreakGlass("send")) {
            root.stepUpRequired("send", text);
            return "step-up";
        }
        root._queuedText = text;
        if (root.ambientEnabled) {
            cursorProc.running = true; // chain continues in onExited
        } else {
            root._ensureConversationThenRequest();
        }
        return true;
    }

    Process {
        id: createConversation
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const conv = JSON.parse(text);
                    root.conversationId = conv.id;
                    root._makeRequest();
                } catch (e) {
                    console.log("[Souveraine] conversation create failed:", text);
                    root.streamClosed(1);
                }
            }
        }
    }

    function _ensureConversationThenRequest() {
        if (root.conversationId.length > 0) {
            root._makeRequest();
            return;
        }
        // Speaking instead of accepting the offered thread is an explicit
        // fresh start. Retire the offer before minting the new conversation so
        // the footer cannot keep advertising "Continue" over an active one.
        root.dismissOfferedResume();
        createConversation.command = [
            "curl", "-sf", "-X", "POST",
            `${root.serverBase}/v1/conversations`,
            "-H", "Content-Type: application/json",
            "--data", JSON.stringify({ "agent_id": root.currentAgentId })
        ];
        createConversation.running = true;
    }

    property string requestScriptFilePath: "/tmp/quickshell/ai/souveraine-request.sh"

    FileView {
        id: requesterScriptFile
    }

    function _tokenReadLine(agentId) {
        // Bearer token read at request time so rotation works.
        return `TOKEN=$(cat "$HOME/.souveraine/server/agents/${agentId}/api_token" 2>/dev/null)\n`;
    }

    function _makeRequest() {
        const data = {
            "messages": [{ "role": "user", "content": root._queuedText }],
            "stream": true
        };
        const ambient = root.collectAmbient();
        if (ambient.length > 0) data["ambient"] = ambient;
        root._queuedText = "";

        const scriptContent = "#!/usr/bin/env bash\n"
            + root._tokenReadLine(root.currentAgentId)
            + `curl --no-buffer -sS -X POST "${root.serverBase}/v1/conversations/${root.conversationId}/messages"`
            + ` -H 'Content-Type: application/json'`
            + ` -H "Authorization: Bearer $TOKEN"`
            + ` --data '${CF.StringUtils.shellSingleQuoteEscape(JSON.stringify(data))}'`
            + "\n";

        const shellScriptPath = CF.FileUtils.trimFileProtocol(root.requestScriptFilePath);
        requesterScriptFile.path = Qt.resolvedUrl(shellScriptPath);
        requesterScriptFile.setText(scriptContent);
        requester.command = ["bash", shellScriptPath];
        root.turnStartedAt = Date.now();
        root.turnElapsedMs = 0;
        root.turnActive = true;
        requester.running = true;
    }

    Process {
        id: requester
        stdout: SplitParser {
            onRead: data => {
                if (data.length === 0 || !data.startsWith("data:")) return;
                let event;
                try {
                    event = JSON.parse(data.slice(5).trim());
                } catch (e) {
                    console.log("[Souveraine] Unparseable SSE line:", data);
                    return;
                }
                root.streamEvent(event);
            }
        }
        onExited: (exitCode, exitStatus) => {
            // Freeze the clock on the real total — the 100ms tick can be up to
            // one interval behind when the process exits.
            if (root.turnStartedAt > 0)
                root.turnElapsedMs = Date.now() - root.turnStartedAt;
            root.turnActive = false;
            root.streamClosed(exitCode);
        }
    }

    // ── Backchannel ──────────────────────────────────────────────────────
    Process {
        id: backchannelProc
        property string script: ""
        command: ["bash", "-c", script]
    }

    function cancelTurn() {
        if (root.conversationId.length === 0) return;
        backchannelProc.script = root._tokenReadLine(root.currentAgentId)
            + `curl -sf -X POST "${root.serverBase}/v1/conversations/${root.conversationId}/cancel"`
            + ` -H "Authorization: Bearer $TOKEN"`;
        backchannelProc.running = true;
    }

    function interject(text) {
        if (root.conversationId.length === 0 || text.length === 0) return;
        const body = JSON.stringify({ "text": text });
        backchannelProc.script = root._tokenReadLine(root.currentAgentId)
            + `curl -sf -X POST "${root.serverBase}/v1/conversations/${root.conversationId}/interject"`
            + ` -H 'Content-Type: application/json'`
            + ` -H "Authorization: Bearer $TOKEN"`
            + ` --data '${CF.StringUtils.shellSingleQuoteEscape(body)}'`;
        backchannelProc.running = true;
    }
}
