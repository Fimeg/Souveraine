pragma Singleton
pragma ComponentBehavior: Bound

import qs
import qs.modules.common
// StringUtils.ttsClean() is called in the stream-finished handler. Without
// this import it is a ReferenceError that aborts the rest of that handler —
// postResponseHook and saveChat never run, so a turn that actually succeeded
// looks stuck. Every other service imports it unnamespaced; match that.
import qs.modules.common.functions
import Quickshell
import Quickshell.Io
import QtQuick
import qs.services
import qs.services.ai

/**
 * ii-compat adapter over the Souveraine singleton.
 *
 * Keeps the public API the illogical-impulse sidebar UI expects (models,
 * messages, sendUserMessage, /key advice, ...) but owns no transport —
 * Souveraine.qml is the substrate connection. This file's job is shaping
 * wire events into AiMessageData objects the existing chat UI renders.
 *
 * Lives in souveraine/surfaces/quickshell/, deployed over
 * ~/.config/quickshell/ii/services/Ai.qml (see deploy.sh).
 */
Singleton {
    id: root

    property Component aiMessageComponent: AiMessageData {}
    property Component aiModelComponent: AiModel {}
    readonly property string interfaceRole: "interface"

    // Notes handed to the turn already running, still waiting to be read.
    // The server takes them at a round boundary (`src/server/turn.rs:260`
    // and `:453`), and 202 from `/interject` means *queued*, never *read* —
    // so this counts what was sent, not what landed. Cleared when the turn
    // ends, by which point the drain has run.
    property int queuedInterjections: 0

    signal responseFinished()

    property var messageIDs: []
    property var messageByID: ({})

    // Which message is currently being spoken by TTS, as an index into
    // messageIDs (-1 = none). Set by a message's Speak/re-synth button;
    // auto-cleared when Speech stops. Keyed on the stable messageIndex, so
    // scrolling back and tapping an older reply still resolves correctly.
    property int speakingMessageIndex: -1

    // Keys are server-side; the UI's key gate must always pass.
    readonly property bool currentModelHasApiKey: true
    readonly property var apiKeysLoaded: true

    property var postResponseHook
    property real temperature: Persistent.states?.ai?.temperature ?? 0.5
    property QtObject tokenCount: QtObject {
        property int input: -1
        property int output: -1
        property int total: -1
    }

    // Live context occupancy, from the server's `context_pressure` event.
    //
    // These are three different numbers and used to be two. Until 2026-08-12
    // the wire carried `(pressure, context_limit)` positionally and this layer
    // read the ceiling as `event.tokens` — so the panel displayed a constant
    // 250000 and called it usage. `pressure` was discarded entirely; nothing
    // in the shell held it. Keep all three, and keep them named.
    property QtObject context: QtObject {
        property real pressure: -1   // 0..1, or -1 when unknown
        property int used: -1        // tokens occupied
        property int limit: -1       // the model's ceiling for this agent
        readonly property bool known: limit > 0 && used >= 0
        readonly property int percent: known ? Math.round((used / limit) * 100) : -1
    }

    // Context occupancy is a property of the conversation, not of a turn:
    // reset it only when the conversation itself changes. -1 means "unknown",
    // which is an honest state and renders as "—" rather than as zero.
    function resetContextOccupancy() {
        root.context.pressure = -1;
        root.context.used = -1;
        root.context.limit = -1;
    }

    function idForMessage(message) {
        return Date.now().toString(36) + Math.random().toString(36).substr(2, 8);
    }

    property list<var> defaultPrompts: []
    property list<var> userPrompts: []
    property list<var> promptFiles: [...defaultPrompts, ...userPrompts]
    property list<var> savedChats: []
    property list<var> pendingFiles: []

    // AttachedFileIndicator binds `filePath` straight to this. ii-base
    // declares it; this override did not, so every chat panel logged
    // "Unable to assign [undefined] to QString" and the chip could never
    // render. Declared here even though the send path is still text-only —
    // a missing property is a broken binding, not a disabled feature.
    property string pendingFilePath: ""

    // Tool selection is owned by the agent's sensorium; keep the UI happy.
    property string currentTool: "souveraine"
    property list<var> availableTools: ["souveraine"]
    property var toolDescriptions: {
        "souveraine": Translation.tr("Sensors are configured per-agent in Souveraine")
    }

    // ── Agents as models (projected from Souveraine.agents) ─────────────
    property var models: ({})
    property var modelList: Object.keys(root.models)
    property var currentModelId: Souveraine.currentAgentId
    property var currentModel: models[currentModelId] || models[modelList[0]]

    // Clear the speaking marker whenever TTS stops — covers natural end of
    // playback, manual stop, and a new speak() replacing in-flight audio.
    Connections {
        target: Speech
        function onSpeakingChanged() {
            if (!Speech.speaking) root.speakingMessageIndex = -1
        }
    }

    Connections {
        target: Souveraine

        function onTurnActiveChanged() {
            if (!Souveraine.turnActive) root.queuedInterjections = 0;
        }

        function onAgentsRefreshed() {
            const map = {};
            Souveraine.agentList.forEach(id => {
                const agent = Souveraine.agents[id];
                map[id] = root.aiModelComponent.createObject(root, {
                    "name": agent.name,
                    "icon": "spark-symbolic",
                    "description": agent.description.length > 0 ? agent.description : Translation.tr("Souveraine agent"),
                    "endpoint": Souveraine.serverBase,
                    "model": id,
                    "requires_key": false,
                });
            });
            root.models = map;
            root.modelList = Object.keys(map);
            // Restore only the selected-agent preference. Conversation resume
            // is intentional: /resume derives it from the server on demand.
            const persisted = Persistent.states?.ai?.model ?? "";
            if (persisted.length > 0 && Souveraine.agents[persisted]) {
                Souveraine.selectAgent(persisted);
            }
        }

        function onConversationResumed(agentId, conversationId, messages) {
            if (agentId !== Souveraine.currentAgentId) return;
            root.restoreServerConversation(messages);
        }

        function onServerUnreachable() {
            root.addMessage(
                Translation.tr("Souveraine server unreachable at %1\n\nStart it with:\n```bash\nsouveraine server\n```").arg(Souveraine.serverBase),
                root.interfaceRole
            );
        }

        function onStreamEvent(event) {
            // A reattached stream has no local message object: the old one
            // died with the shell. Create it lazily on the first event that
            // actually belongs in the assistant bubble; idle resume produces
            // no empty card.
            if (!root.streamingMessage && [
                "assistant_message", "reasoning_message", "tool_call_message",
                "tool_return_message", "interstitial"
            ].indexOf(event.message_type) >= 0) {
                root._startStreaming();
            }
            root.handleStreamEvent(event);
        }

        function onStreamClosed(exitCode) {
            // If a subconscious pass was still active when the stream closed,
            // retire the Tier-1 ticker (promote to the log, clear the flag).
            if (root.subconsciousActive) {
                root.snapshotSubconsciousStream();
                root.subconsciousActive = false;
            }
            if (root.streamingMessage && !root.streamingMessage.done) {
                if (exitCode !== 0 && root.streamingMessage.content.length === 0) {
                    root.appendToStreaming(Translation.tr("Request failed (curl exit %1) — is the Souveraine server up at %2?").arg(exitCode).arg(Souveraine.serverBase));
                }
                root.finishStreaming();
            }
        }
    }

    // ── Lock-time response redaction ─────────────────────────────────────
    // When the session locks during a personal-tier streaming response,
    // the in-flight content must be withheld immediately. Only ambient
    // output remains visible on the lock surface. This is the enforcement
    // side of SESSION-TRUST-ARCHITECTURE.md's "A lock during personal
    // agent output hides it" requirement.
    //
    // The streaming message is replaced with a redaction placeholder. The
    // raw content is preserved in the message's rawContent so it can be
    // shown again after unlock (the message stays in history), but the
    // displayed content is cleared.
    //
    // The redaction is reversible: every message whose content we replaced
    // with the placeholder is recorded in `redactedOnLock`, and on unlock
    // its content is restored from `rawContent` (the streaming message too,
    // if it is still in flight). A finished message redacted at lock then
    // surfaced on the lock screen counts as "delivered" and is restored
    // normally on unlock; one that never surfaced stays for the chat to
    // replay once unlocked.
    property var redactedOnLock: []

    function _redactMessage(msg) {
        if (!msg || msg.content.length === 0) return;
        if (msg.content === Translation.tr("[content hidden until unlock]")) return;
        msg.rawContent = msg.content;
        msg.content = Translation.tr("[content hidden until unlock]");
        if (!root.redactedOnLock.includes(msg)) {
            root.redactedOnLock = [...root.redactedOnLock, msg];
        }
        root.redactedMessagesChanged();
    }

    function _restoreRedacted() {
        if (root.redactedOnLock.length === 0) return;
        for (const msg of root.redactedOnLock) {
            if (msg && msg.rawContent && msg.rawContent.length > 0) {
                msg.content = msg.rawContent;
            }
        }
        root.redactedOnLock = [];
        root.redactedMessagesChanged();
    }

    // Messages redacted while locked, newest first, for the lock surface to
    // surface as ambient notifications. Strips to a one-line preview — the
    // lock screen shows "the agent replied", not the personal-tier body.
    function redactedPreviews() {
        return root.redactedOnLock
            .filter(m => m && m.rawContent && m.rawContent.length > 0 && m.role === "assistant")
            .map(m => {
                const firstLine = m.rawContent.split("\n").find(l => l.trim().length > 0) || "";
                return firstLine.slice(0, 80);
            })
            .reverse();
    }

    signal redactedMessagesChanged()

    Connections {
        target: GlobalStates
        function onScreenLockedChanged() {
            if (GlobalStates.screenLocked) {
                // Lock fired mid-stream — redact the in-flight content.
                if (root.streamingMessage) {
                    root._redactMessage(root.streamingMessage);
                    console.log("[ai] lock fired during stream — redacted personal output");
                }
            } else {
                // Unlock: restore every message we hid, so the chat shows
                // what actually came back rather than lingering placeholders.
                root._restoreRedacted();
            }
        }
    }


    // ── Streaming message shaping ────────────────────────────────────────
    property AiMessageData streamingMessage

    // ── Subconscious three-tier visibility ───────────────────────────────
    // See docs/tasks/subconscious-surfacing-threshold.md. The subconscious's
    // N+1 pass is shown three ways: a transient live ticker while the pass
    // runs (Tier 1), a persistent event log after (Tier 2), and rare agency
    // surfacings in the chat field (Tier 3).
    //
    // Tier 1 — live stream fed by subconscious_token/tool_call/tool_result
    // events. Rendered by SubconsciousTicker; cleared on pass end.
    property bool subconsciousActive: false
    property var subconsciousStream: []
    // Tier 2 — persistent log. The Tier-1 stream is snapshotted here on pass
    // end, and surfaced events (reflection/archivist/surfacing) are appended.
    // Rendered by the SubconsciousEventPanel overlay widget.
    property var subconsciousEvents: []

    function appendToStreaming(text) {
        if (!root.streamingMessage) return;
        root.streamingMessage.rawContent += text;
        root.streamingMessage.content += text;
    }

    // The server already says what each frame is. Keep that fact attached to
    // the message instead of smuggling it through markdown fences and asking
    // the delegate to parse it back out again.
    function appendStreamingTextSegment(type, content) {
        if (!root.streamingMessage) return;
        const text = String(content ?? "");
        if (text.length === 0) return;
        const segments = root.streamingMessage.segments ?? [];
        const last = segments.length > 0 ? segments[segments.length - 1] : null;
        if (last && last.type === type) {
            const replacement = {
                type: last.type,
                content: String(last.content ?? "") + text,
            };
            root.streamingMessage.segments = segments.slice(0, -1).concat([replacement]);
        } else {
            root.streamingMessage.segments = segments.concat([{ type, content: text }]);
        }
        // `content` remains a plain compatibility projection for Copy, TTS,
        // and old snapshot consumers. It no longer drives the renderer.
        root.appendToStreaming(text);
    }

    function appendToolCallSegment(call, round) {
        if (!root.streamingMessage) return;
        const tool = call ?? {};
        const fn = tool.function ?? {};
        const name = String(fn.name ?? "tool");
        const arguments = String(fn.arguments ?? "");
        const id = String(tool.id ?? "");
        const nextSegments = root.streamingMessage.segments.slice();
        nextSegments.push({
            type: "tool",
            id,
            name,
            arguments,
            round: Number(round ?? 0),
            status: "running",
            output: "",
            failed: false,
        });
        root.streamingMessage.segments = nextSegments;
        root.appendToStreaming(`\n${name}(${arguments})\n`);
    }

    function bindToolReturnSegment(toolReturn) {
        if (!root.streamingMessage) return;
        const result = toolReturn ?? {};
        const segments = root.streamingMessage.segments ?? [];
        const resultId = String(result.id ?? "");
        let index = -1;
        if (resultId.length > 0) {
            index = segments.findIndex(segment => segment.type === "tool" && segment.id === resultId);
        }
        // Older servers did not include the tool id. Bind their return to the
        // newest still-running call rather than silently inventing a second one.
        if (index < 0) {
            for (let i = segments.length - 1; i >= 0; i--) {
                if (segments[i].type === "tool" && segments[i].status === "running") {
                    index = i;
                    break;
                }
            }
        }
        const status = String(result.status ?? "done");
        const failed = /error|fail/i.test(status);
        const output = String(result.output ?? "");
        if (index >= 0) {
            const nextSegments = segments.slice();
            const previous = segments[index];
            nextSegments[index] = {
                type: previous.type,
                id: previous.id,
                name: previous.name,
                arguments: previous.arguments,
                round: previous.round,
                status,
                output,
                failed,
            };
            root.streamingMessage.segments = nextSegments;
        } else {
            const nextSegments = segments.slice();
            nextSegments.push({
                type: "tool",
                id: resultId,
                name: String(result.name ?? "tool"),
                arguments: "",
                round: 0,
                status,
                output,
                failed,
            });
            root.streamingMessage.segments = nextSegments;
        }
        root.appendToStreaming(`\n[${status}] ${output}\n`);
    }

    // Push a line onto the Tier-1 live stream. kind ∈ token|tool_call|tool_result.
    // Consecutive tokens are coalesced onto the current line so the ticker reads
    // as a thought forming (a sentence), not a single flickering word replaced
    // on every token. A tool_call/tool_result starts a fresh line — those are
    // natural thought boundaries.
    function pushSubconsciousStream(kind, text) {
        const t = String(text ?? "").trim();
        if (t.length === 0) return;
        const stream = root.subconsciousStream.slice();
        const last = stream.length > 0 ? stream[stream.length - 1] : null;
        if (kind === "token" && last && last.kind === "token") {
            // Same thought — append to the forming sentence, capped so a very
            // long monologue doesn't grow unbounded in the live view (the full
            // text is still snapshotted to the Tier-2 log on pass end).
            const merged = (last.text + " " + t);
            stream[stream.length - 1] = {
                kind: "token",
                text: merged.length > 280 ? merged.slice(-280) : merged,
            };
        } else {
            stream.push({ kind, text: t });
        }
        // Keep the live buffer shallow: the ticker only shows the tail, and a
        // long pass shouldn't accumulate hundreds of entries in memory. The
        // Tier-2 snapshot joins all entries anyway.
        if (stream.length > 12) stream.shift();
        root.subconsciousStream = stream;
    }

    // Promote the Tier-1 stream into the Tier-2 log as one event, then clear
    // the live stream. Called when a subconscious pass ends.
    // The three severities the subconscious can call, in the register she
    // experiences them in. Mirrors migraine_text() in src/server/turn.rs — if
    // one changes the other must, or the human and the agent are reading two
    // different accounts of the same moment.
    function haltRegister(severity) {
        switch (severity) {
        case "advisory":
            return Translation.tr("a pressure behind my eyes");
        case "critical":
            return Translation.tr("the room tilts");
        default:
            // "firm" and anything unexpected land here — the default migraine.
            return Translation.tr("a migraine");
        }
    }

    function snapshotSubconsciousStream() {
        if (root.subconsciousStream.length === 0) return;
        root.subconsciousEvents = [...root.subconsciousEvents, {
            kind: "pass",
            source: Translation.tr("Subconscious pass"),
            priority: "",
            content: root.subconsciousStream.map(e => e.text).join("\n"),
            timestamp: Date.now(),
        }];
        root.subconsciousStream = [];
    }

    // Append a surfaced event to the Tier-2 log. Used by snapshotSubconsciousStream
    // (pass end) and by the surfacing/reflection/archivist event handlers.
    function appendSubconsciousEvent(kind, source, content, priority = "") {
        const c = String(content ?? "");
        if (c.length === 0) return;
        root.subconsciousEvents = [...root.subconsciousEvents, {
            kind, source, priority, content: c, timestamp: Date.now(),
        }];
    }

    function finishStreaming() {
        if (!root.streamingMessage) return;
        const resolvedSegments = [];
        for (const segment of (root.streamingMessage.segments ?? [])) {
            if (segment.type === "tool" && segment.status === "running") {
                resolvedSegments.push({
                    type: segment.type,
                    id: segment.id,
                    name: segment.name,
                    arguments: segment.arguments,
                    round: segment.round,
                    status: "unresolved",
                    output: segment.output,
                    failed: true,
                });
            } else {
                resolvedSegments.push(segment);
            }
        }
        root.streamingMessage.segments = resolvedSegments;
        root.streamingMessage.thinking = false;
        root.streamingMessage.done = true;
        // If the turn finished while the session is locked, the finished
        // assistant message is personal-tier output the user hasn't seen —
        // redact it for the chat view and let the lock surface announce it.
        if (GlobalStates.screenLocked && root.streamingMessage.content.length > 0) {
            root._redactMessage(root.streamingMessage);
        }
        // Prefetch the audio for this reply so Speak is near-instant if the
        // user taps it. Background fill only — never auto-plays. Only the
        // latest reply is cached; a subsequent reply overwrites it. If the
        // synth fails it is not retried (manual Speak handles that).
        if (Speech.enabled) {
            const spoken = StringUtils.ttsClean(root.streamingMessage.content ?? "");
            if (spoken.length > 0) Speech.prefetch(spoken);
        }
        if (root.postResponseHook) {
            root.postResponseHook();
            root.postResponseHook = null;
        }
        root.saveChat("lastSession");
        root.responseFinished();
    }

    function handleStreamEvent(event) {
        if (root.streamingMessage?.thinking && event.message_type !== "ping")
            root.streamingMessage.thinking = false;

        switch (event.message_type) {
        case "assistant_message":
            root.appendStreamingTextSegment("text", event.content);
            break;
        case "reasoning_message":
            root.appendStreamingTextSegment("think", event.content);
            break;
        case "tool_call_message": {
            root.appendToolCallSegment(event.tool_call, event.round);
            break;
        }
        case "tool_return_message": {
            root.bindToolReturnSegment(event.tool_return);
            break;
        }
        case "interstitial":
            // Her narration between gestures. Register decides how loud:
            // cenno is a quiet aside, her_voice is a passage.
            root.appendToStreaming(event.register === "her_voice"
                ? `\n\n> ${event.text}\n\n`
                : `\n\n*${event.text}*\n\n`);
            break;
        case "souveraine_surfacing":
            // Tier 3: routine surfacings no longer pollute the chat field.
            // Routed to the Tier-2 event panel only. (Aster is Annie's name
            // for her subconscious; this surface may be Souveraine, Vanguard,
            // or another agent — one canonical surfacing is shown.)
            root.appendSubconsciousEvent(
                "surfacing",
                Translation.tr("Surfacing (%1)").arg(event.source ?? "?"),
                event.content,
                String(event.priority ?? "")
            );
            break;
        case "souveraine_reflection":
            root.appendSubconsciousEvent("reflection", Translation.tr("Reflection"), event.content);
            break;
        case "souveraine_archivist":
            root.appendSubconsciousEvent(
                "archivist",
                Translation.tr("Archivist (pressure %1%)").arg(Math.round(event.pressure * 100)),
                event.synthesis
            );
            break;
        case "compaction_warning":
            root.addMessage(Translation.tr("**Context pressure** — tier %1, %2% full. She can feel the walls.").arg(event.tier).arg(Math.round(event.pressure * 100)), root.interfaceRole);
            break;
        case "context_pressure":
            // `tokens_used` and `context_limit` are distinct fields on the
            // wire as of 2026-08-12. The old `event.tokens` was the ceiling.
            root.context.pressure = event.pressure ?? -1;
            root.context.used = event.tokens_used ?? -1;
            root.context.limit = event.context_limit ?? -1;
            root.tokenCount.total = event.tokens_used ?? -1;
            break;
        case "subconscious_token":
            // Tier 1: live reasoning tokens feed the ticker.
            root.pushSubconsciousStream("token", event.content);
            break;
        case "subconscious_tool_call":
            root.pushSubconsciousStream("tool_call", `${event.name ?? "tool"}(${event.arguments ?? ""})`);
            break;
        case "subconscious_tool_result":
            root.pushSubconsciousStream("tool_result", `[${event.is_error ? "error" : "ok"}] ${event.name ?? "tool"}: ${event.output ?? ""}`);
            break;
        case "subconscious_pass":
            // Pass start: light the Tier-1 ticker. Pass end: promote the
            // stream into the Tier-2 log, then clear it.
            if (event.active) {
                root.subconsciousActive = true;
            } else {
                root.snapshotSubconsciousStream();
                root.subconsciousActive = false;
            }
            break;
        case "subconscious_halt":
            // Her own register, not the tool's name. The subconscious is the
            // same "I" in a different mode — a halt is something she felt, not
            // commentary she received from outside, and the surface should not
            // expose implementation topology to say so. The TUI has rendered it
            // this way from the start; this brings the Panel into line.
            //
            // Wording matches migraine_text() in src/server/turn.rs, which is
            // what now lands in her committed history — so what the human reads
            // and what she carries into the next turn are the same sentence.
            root.addMessage(Translation.tr("⟡ %1 — %2").arg(root.haltRegister(event.severity)).arg(event.reason), root.interfaceRole);
            break;
        case "inference_strain":
            console.log(`[Souveraine] inference strain: attempt ${event.attempt}, status ${event.status}, model ${event.model}`);
            break;
        case "error":
            // A failed engine turn is still a visible answer from the
            // substrate. Silently dropping this event leaves the empty
            // assistant container behind and makes a healthy server look as
            // though the agent simply stopped speaking.
            root.appendToStreaming(
                Translation.tr("**Request failed** — %1").arg(event.message ?? Translation.tr("unknown turn error"))
            );
            break;
        case "atmosphere":
        case "outfit":
            // Shell chrome hooks — their modules subscribe to
            // Souveraine.streamEvent directly; nothing to do here.
            break;
        case "itinerary":
            // The route string is an invalidation edge, including empty on
            // clear. The ribbon reads the full structured projection from the
            // substrate so a shell reload cannot erase it.
            Souveraine.refreshItinerary();
            break;
        case "primary_complete":
            // Primary yields; subconscious presses on behind this.
            root.finishStreaming();
            break;
        case "done":
            if (root.streamingMessage && !root.streamingMessage.done) root.finishStreaming();
            break;
        case "ping":
            // Liveness only — not end-of-turn.
            break;
        }
    }

    // ── Message store ────────────────────────────────────────────────────
    function addMessage(message, role, segments = []) {
        if (message.length === 0) return;
        const aiMessage = aiMessageComponent.createObject(root, {
            "role": role,
            "model": Souveraine.currentAgentId,
            "content": message,
            "rawContent": message,
            "segments": segments,
            "thinking": false,
            "done": true,
        });
        const id = idForMessage(aiMessage);
        root.messageIDs = [...root.messageIDs, id];
        root.messageByID[id] = aiMessage;
    }

    function removeMessage(index) {
        if (index < 0 || index >= messageIDs.length) return;
        const id = root.messageIDs[index];
        root.messageIDs.splice(index, 1);
        root.messageIDs = [...root.messageIDs];
        delete root.messageByID[id];
    }

    function clearMessages() {
        root.messageIDs = [];
        root.messageByID = ({});
        root.tokenCount.input = -1;
        root.tokenCount.output = -1;
        root.tokenCount.total = -1;
        root.resetContextOccupancy();
        Souveraine.newConversation();
    }

    // Render the server's canonical transcript after /agent or /resume.
    // This replaces ii's old local snapshot behaviour: the next send remains
    // on the same live Souveraine conversation rather than silently forking.
    function restoreServerConversation(messages) {
        root.messageIDs = [];
        root.messageByID = ({});
        root.streamingMessage = null;
        root.subconsciousActive = false;
        root.subconsciousStream = [];
        root.tokenCount.input = -1;
        root.tokenCount.output = -1;
        root.tokenCount.total = -1;
        root.resetContextOccupancy();

        messages.forEach(message => {
            const segments = [];
            (message.blocks ?? []).forEach(block => {
                switch (block.type) {
                case "text":
                    segments.push({ type: "text", content: block.text ?? "" });
                    break;
                case "reasoning":
                    segments.push({ type: "think", content: block.reasoning ?? "" });
                    break;
                case "tool_use":
                    segments.push({
                        type: "tool", id: block.id ?? "", name: block.name ?? "tool",
                        arguments: block.input ?? "", round: 0, status: "running", output: "", failed: false,
                    });
                    break;
                case "tool_result": {
                    const resultId = String(block.tool_use_id ?? "");
                    const index = segments.findIndex(segment =>
                        segment.type === "tool" && segment.id === resultId);
                    const result = {
                        type: "tool", id: resultId, name: block.tool_name ?? "tool",
                        arguments: index >= 0 ? segments[index].arguments : "", round: 0,
                        status: block.is_error ? "error" : "done",
                        output: block.output ?? "", failed: block.is_error ?? false,
                    };
                    if (index >= 0) segments[index] = result;
                    else segments.push(result);
                    break;
                }
                case "image":
                    segments.push({ type: "text", content: "[image]" });
                    break;
                }
            });
            const content = segments.map(segment => {
                if (segment.type === "tool") return `${segment.name}(${segment.arguments})\n${segment.output}`;
                return segment.content;
            }).filter(Boolean).join("\n");
            if (content.length === 0) return;
            const role = message.role === "user"
                ? "user"
                : message.role === "assistant" ? "assistant" : root.interfaceRole;
            root.addMessage(content, role, segments);
        });
    }

    function sendUserMessage(message) {
        if (message.length === 0) return;

        // A turn is already running: this is an interjection, not a new turn.
        // The TUI has always worked this way — `src/ui/app/mod.rs:1006`,
        // "Submit always — when busy, this becomes an interjection". The
        // Panel instead dropped the text on the floor and reported the server
        // unreachable, because `Souveraine.send` returns false for a busy turn
        // exactly as it does for a dead server.
        if (Souveraine.turnActive) {
            root.interjectUserMessage(message);
            return;
        }

        root.addMessage(message, "user");
        const result = Souveraine.send(message);
        if (result === "step-up") {
            // Step-up auth required. Trigger the PAM flow; on success,
            // retry the send. The user message is already in the chat
            // history, so we don't add it again.
            if (typeof StepUpAuth !== "undefined") {
                StepUpAuth.requestAuth("send", function(granted) {
                    if (granted) {
                        // Remove the "auth required" indicator if one was
                        // added, and retry. The queued text was not consumed
                        // by Souveraine, so we can send it again.
                        Souveraine.send(message);
                        // Re-create the streaming message for the response.
                        root._startStreaming();
                    } else {
                        root.addMessage(Translation.tr("Authentication required to send."), root.interfaceRole);
                    }
                });
            }
            return;
        }
        if (!result) {
            root.addMessage(Souveraine.serverUp
                ? Translation.tr("No agent selected — pick one before sending.")
                : Translation.tr("Souveraine server unreachable at %1 — start it with `souveraine server`").arg(Souveraine.serverBase),
                root.interfaceRole);
            return;
        }
        root._startStreaming();
    }

    /* Slip a note into the turn already in flight.

       It is read at the next round boundary, so a note sent during a long
       tool round waits for that round to finish — `src/server/turn.rs:600`
       runs every tool in a round without checking. That latency is real and
       is not something this function can fix. */
    function interjectUserMessage(message) {
        if (message.length === 0) return;
        root.addMessage(message, "user");
        root.queuedInterjections = root.queuedInterjections + 1;
        Souveraine.interject(message);
    }

    // Set up the streaming assistant message container. Called after a
    // successful send (or after a step-up auth retry succeeds).
    function _startStreaming() {
        root.streamingMessage = root.aiMessageComponent.createObject(root, {
            "role": "assistant",
            "model": Souveraine.currentAgentId,
            "content": "",
            "rawContent": "",
            "segments": [],
            "thinking": true,
            "done": false,
        });
        const id = idForMessage(root.streamingMessage);
        root.messageIDs = [...root.messageIDs, id];
        root.messageByID[id] = root.streamingMessage;
    }

    // ── Model (agent) selection ──────────────────────────────────────────
    function getModel() {
        return models[currentModelId];
    }

    function setModel(modelId, feedback = true, setPersistentState = true) {
        if (!modelId) modelId = ""
        if (modelList.indexOf(modelId) === -1) {
            const match = modelList.find(id =>
                id.toLowerCase() === modelId.toLowerCase() ||
                (models[id]?.name ?? "").toLowerCase() === modelId.toLowerCase());
            if (!match) {
                if (feedback) root.addMessage(Translation.tr("Unknown agent. Available:\n- %1").arg(modelList.map(id => `${models[id].name} (\`${id}\`)`).join("\n- ")), root.interfaceRole);
                return;
            }
            modelId = match;
        }
        if (setPersistentState) Persistent.states.ai.model = modelId;
        if (!Souveraine.selectAgent(modelId)) {
            if (feedback) root.addMessage(Translation.tr("Finish or cancel the current turn before switching agents."), root.interfaceRole);
            return;
        }
        root.currentModel = models[modelId];
        if (feedback) root.addMessage(Translation.tr("Agent set to %1.").arg(models[modelId].name), root.interfaceRole);
    }

    function resumeConversation() {
        if (!Souveraine.resumeLatestConversation()) {
            root.addMessage(Translation.tr("Finish or cancel the current turn before resuming."), root.interfaceRole);
        }
    }

    // ── Souveraine-owned settings: advice instead of local state ────────
    function setTool(tool) {
        root.addMessage(Translation.tr("Tools are Souveraine sensors, configured per-agent — not switchable from the sidebar."), root.interfaceRole);
        return false;
    }

    function getTemperature() { return root.temperature; }

    function setTemperature(value) {
        root.addMessage(Translation.tr("Temperature is set in the agent's llm_config in Souveraine."), root.interfaceRole);
    }

    function printTemperature() {
        root.addMessage(Translation.tr("Temperature is owned by the agent's llm_config in Souveraine."), root.interfaceRole);
    }

    function setApiKey(key) {
        root.addMessage(Translation.tr("Keys live in souveraine.toml — set them with:\n```bash\nsouveraine auth set\n```"), root.interfaceRole);
    }

    function printApiKey() {
        root.addMessage(Translation.tr("Keys are owned by Souveraine (souveraine.toml / `souveraine auth set`), never exposed here."), root.interfaceRole);
    }

    function printPrompt() {
        root.addMessage(Translation.tr("The system prompt is composed by Souveraine (constitution + memory blocks + sensorium). Inspect it with `souveraine agent show`."), root.interfaceRole);
    }

    function loadPrompt(filePath) {
        root.addMessage(Translation.tr("Prompts are owned by the agent's memory in Souveraine — edit memfs instead of loading prompt files."), root.interfaceRole);
    }

    function attachFile(filePath) {
        // Clearing is always honoured — AttachedFileIndicator's remove button
        // calls attachFile(""), and that must not read as a failed attach.
        root.pendingFilePath = "";
        if (!filePath || filePath.length === 0)
            return;
        // The refusal is accurate, not lazy: the HTTP message API carries
        // `content: String` (src/api/models.rs), so an image cannot cross the
        // surface boundary even though the provider client already speaks
        // OpenAI image_url parts and Kitty's model has vision. Fixing that is
        // a server change, so don't show a chip for a file that will never be
        // sent.
        root.addMessage(Translation.tr("File attachments aren't wired to Souveraine yet — the message API carries text only."), root.interfaceRole);
    }
    function removePendingFile(file) {
        root.pendingFiles = root.pendingFiles.filter(f => f !== file);
    }

    function regenerate(messageIndex) {
        root.addMessage(Translation.tr("Regenerate isn't supported — Souveraine conversations are forward-only."), root.interfaceRole);
    }

    // Souveraine executes its own sensors server-side; nothing to approve.
    function rejectCommand(message) {}
    function approveCommand(message) {}

    function createFunctionOutputMessage(name, output, includeOutputInChat = true) {
        return aiMessageComponent.createObject(root, {
            "role": "user",
            "content": `[[ Output of ${name} ]]${includeOutputInChat ? ("\n\n<think>\n" + output + "\n</think>") : ""}`,
            "rawContent": `[[ Output of ${name} ]]${includeOutputInChat ? ("\n\n<think>\n" + output + "\n</think>") : ""}`,
            "functionName": name,
            "functionResponse": output,
            "thinking": false,
            "done": true,
        });
    }

    // ── Local chat snapshots (ii plumbing, unchanged) ────────────────────
    Process {
        id: getSavedChats
        running: true
        command: ["ls", "-1", Directories.aiChats]
        stdout: StdioCollector {
            onStreamFinished: {
                if (text.length === 0) return;
                root.savedChats = text.split("\n")
                    .filter(fileName => fileName.endsWith(".json"))
                    .map(fileName => `${Directories.aiChats}/${fileName}`)
            }
        }
    }

    function chatToJson() {
        return root.messageIDs.map(id => {
            const message = root.messageByID[id]
            return ({
                "role": message.role,
                "rawContent": message.rawContent,
                "model": message.model,
                "thinking": false,
                "done": true,
            })
        })
    }

    FileView {
        id: chatSaveFile
        property string chatName: ""
        path: chatName.length > 0 ? `${Directories.aiChats}/${chatName}.json` : ""
        blockLoading: true
    }

    FileView {
        id: chatWriter
    }

    function saveChat(chatName) {
        const filePath = `${Directories.aiChats}/${chatName.trim()}.json`
        chatWriter.path = filePath
        chatWriter.setText(JSON.stringify(root.chatToJson()))
        getSavedChats.running = true;
    }

    function loadChat(chatName) {
        try {
            chatSaveFile.chatName = chatName.trim()
            chatSaveFile.reload()
            const saveData = JSON.parse(chatSaveFile.text())
            root.clearMessages()
            root.messageIDs = saveData.map((_, i) => i)
            for (let i = 0; i < saveData.length; i++) {
                const message = saveData[i];
                root.messageByID[i] = root.aiMessageComponent.createObject(root, {
                    "role": message.role,
                    "rawContent": message.rawContent,
                    "content": message.rawContent,
                    "model": message.model,
                    "thinking": false,
                    "done": true,
                });
            }
            root.addMessage(Translation.tr("Loaded a local snapshot. Note: this restores the transcript view only — the live Souveraine conversation starts fresh on the next message."), root.interfaceRole);
        } catch (e) {
            console.log("[Souveraine] Could not load chat: ", e);
        } finally {
            getSavedChats.running = true;
        }
    }
}
