// Her face on the glass — TASK-59.
//
// The face is a *limb*, not a second client. `Souveraine.qml` is the one
// connection to the server and stays that way: this owns a `souveraine-web`
// process, feeds it what she is already saying on the sidebar's stream, and
// sends what the user says to it back through the same `Souveraine.send()`.
// Casey, 2026-08-05: *"I will want it to be in sync with the sidebar — meaning
// if we 'resume' it's resumed."* Two transports could not promise that; one
// does by construction.
//
// USB Hands joins the same limb rather than opening a controller app beside
// her. Its agent field, explicit microphone and trackpad occupy the room the
// compositor already left below her. The HID reports still belong to
// HidController; this service only routes page intent.
//
// ## Turning her on is joining
//
// `joined` is the whole state. While it is true she is on the glass **and** the
// expression vocabulary rides in the per-send ambient block, so she has a
// syntax for shifting expression. While it is false neither happens — and the
// second half is the point: that prompt is context nobody asked for when the
// face is closed. Casey, 2026-08-05: *"it'll be like a loadable/unloadable
// skill… we might have times where we just don't want that extra prompt added
// to context."*
//
// The sidebar ignores the tags it sees, which is why they are safe to leave in
// the stream rather than stripped on the way to one surface.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.modules.common

Singleton {
    id: root

    // On the glass, and in the prompt. One flag, both consequences.
    property bool joined: false
    property string socketPath: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/face.sock"
    property string rigDir: Quickshell.env("HOME") + "/.souveraine/face"

    // Accumulated text of the turn in flight, so the bubble shows the whole
    // line rather than the last delta.
    property string _line: ""

    // The fragment that rides in `ambient` while she is joined. Kept here
    // rather than in the server so that leaving costs exactly nothing — there
    // is no flag to unset and no prompt to remember to remove.
    readonly property string skill: "You have a face on this device right now. "
        + "You may shift expression by emitting a tag on its own line: "
        + "[[face:idle]], [[face:alert]], [[face:thinking]], [[face:processing]], "
        + "[[face:affectionate]], [[face:straining]], [[face:yawning]], "
        + "[[face:listening]], [[face:speaking]]. "
        + "These are the postures the presence system already uses. "
        + "Use them sparingly, where the shift is real."

    // No pre-flight check on the rig.
    //
    // There was one, reading `FileView.exists`, and it reported false for a
    // directory that was plainly there — so the guard meant to explain a
    // missing rig became the thing preventing a present one from loading. The
    // host already fails loudly and specifically when the directory is wrong,
    // and `onExited` puts `joined` back, so the honest answer is to let it try
    // and report what actually happened. A guard that can be wrong about the
    // world is worse than no guard.
    function join() {
        if (root.joined)
            return;
        host.running = true;
        root.joined = true;
    }

    function joinHands(): bool {
        if (!HidController.open())
            return false;
        if (!root.joined)
            root.join();
        // Continue the attached thread when one exists. Otherwise ask the
        // server for this agent's latest; an agent with no history naturally
        // mints a new conversation on the first utterance.
        if (Souveraine.conversationId.length === 0 && !Souveraine.turnActive)
            Souveraine.resumeLatestConversation();
        root._syncHands();
        return true;
    }

    function leaveHands() {
        if (HidController.active)
            HidController.close();
        root._syncHands();
    }

    function leave() {
        if (root.listening)
            root._talk("cancel");
        if (HidController.active)
            HidController.close();
        root.joined = false;
        root._send({ op: "quit" });
        host.running = false;
    }

    function toggle() {
        if (root.joined)
            root.leave();
        else
            root.join();
    }

    // Her body keeps the measured 540x760 canvas. The transparent 240px below
    // it is the room viewtop deliberately reserved for whatever she shares;
    // USB Hands fills that room when joined and otherwise publishes no input
    // region there, so the home screen continues to receive touch.
    //
    // The Cubism view fits the rig to the canvas, so shrinking the canvas
    // shrinks *her*; it does not trim the empty margin around her. Tried on
    // 2026-08-06: 640 to cut the ~100px of dead space under her feet, and it
    // came back "a tiny version that's scaled odd" because the whole figure
    // came down with it. The dead space is the rig's own layout (`center_y`
    // and `width` in model.json), and moving it is a rig change, not a window
    // one. 760 is the size that reads right.
    readonly property string faceSize: "540x1000"

    Process {
        id: host
        command: ["souveraine-web",
            "--rig", root.rigDir,
            "--ipc", root.socketPath,
            "--transparent",
            "--size", root.faceSize,
            "--app-id", "org.souveraine.face",
            "--title", "Ani"]
        stdout: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                let msg;
                try {
                    msg = JSON.parse(line);
                } catch (e) {
                    return;
                }
                // Anything the page wants to say goes through the one transport.
                if (msg.event === "said" && msg.text) {
                    const result = Souveraine.send(msg.text);
                    const accepted = result === true;
                    const reason = result === "step-up"
                        ? "Authentication is required before sending"
                        : accepted ? "" : "The agent is unavailable or already answering";
                    root._eval(`window.hands && window.hands.agentResult(${JSON.stringify({
                        accepted: accepted,
                        reason: reason
                    })})`);
                }
                else if (msg.event === "tapped")
                    root.tapped(msg.area ?? "body");
                else if (msg.event === "talk")
                    root._talk(msg.phase);
                else if (msg.event === "hid")
                    root._hid(msg);
                else if (msg.event === "ready")
                    root._syncHands();
                else if (msg.event === "dismiss")
                    root.leave();
                else if (msg.event === "console")
                    root._pageSaid(msg.level, msg.text);
            }
        }
        onExited: {
            root.joined = false;
            if (HidController.active)
                HidController.close();
        }
    }

    signal tapped(string area)

    function _hid(message) {
        if (!HidController.active)
            return;
        switch (message.op) {
        case "move":
            HidController.movePointer(message.x ?? 0, message.y ?? 0, message.wheel ?? 0);
            break;
        case "click":
            HidController.click(message.button ?? "left");
            break;
        case "key":
            HidController.key(message.key ?? "", message.modifiers ?? "");
            break;
        case "type": {
            const accepted = HidController.sendText(message.text ?? "");
            root._eval(`window.hands && window.hands.hostResult(${JSON.stringify({
                accepted: accepted,
                reason: accepted ? "" : HidController.lastError
            })})`);
            break;
        }
        }
    }

    function _syncHands() {
        if (!root.joined)
            return;
        const agent = Souveraine.agents[Souveraine.currentAgentId];
        const state = {
            active: HidController.active,
            ready: HidController.ready,
            mode: UsbState.mode,
            error: HidController.lastError,
            agent: agent?.name ?? Souveraine.currentAgentId ?? "agent",
            listening: root.listening,
            thinking: Souveraine.turnActive
        };
        root._eval(`window.hands && window.hands.state(${JSON.stringify(state)})`);
    }

    // Whether she is recording right now. One flag, so a second press cannot
    // start a second recorder over the first one's WAV.
    property bool listening: false

    // ## Speaking through her
    //
    // The explicit microphone beneath her is the primary affordance in USB
    // Hands; press-and-hold on the figure remains available when she is joined
    // without it. Both edges enter this one recorder and transcription path.
    // She is the *face* of the voice pipeline, not a second chat surface
    // (TASK-59 Q1a) — which is why nothing here holds a transcript or a
    // conversation, it only hands text to `Souveraine.send()`.
    //
    // The recorder is `pw-record` at 16k mono s16 and the transcription is
    // `souveraine-stt --file`, deliberately: that script already owns the
    // endpoint from Settings → Speech and the whole error vocabulary
    // (unreachable / 5xx / rejected), and a second copy of that here would be
    // the second answer to "where does dictation go". 16k mono s16 is not a
    // preference either — the comment in that script records that the server
    // 500s on anything else.
    //
    // Written as one `sh -c` rather than a helper on PATH because the shell
    // tree deploys as a unit and a new file on the device would need a package
    // to reach it (CLAUDE.md's rule, and the trap that left sessiond five days
    // stale). Two commands, one place.
    function _talk(phase) {
        if (phase === "start") {
            if (root.listening)
                return;
            root.listening = true;
            root._syncHands();
            root._eval(`window.face.posture("listening")`);
            recorder.command = ["sh", "-c",
                "rm -f \"$W\"; pw-record --rate 16000 --channels 1 --format s16 \"$W\" & echo $! > \"$P\"; wait"];
            recorder.running = true;
            return;
        }
        if (!root.listening)
            return;
        root.listening = false;
        root._syncHands();
        recorder.running = false;
        // Cancelled — a finger that slid off her, or the window losing focus.
        // The recording is dropped rather than transcribed: sending whatever
        // was captured before an abandoned gesture would put words she never
        // finished into the conversation.
        if (phase !== "end") {
            stopper.command = ["sh", "-c", `kill -INT $(cat "$P" 2>/dev/null) 2>/dev/null; rm -f "$P" "$W"`];
            stopper.running = true;
            root._eval(`window.face.posture("idle")`);
            return;
        }
        // The 0.3s is not padding: pw-record finalises the WAV header on the
        // way out, and reading it sooner gets a file the server rejects.
        // `souveraine-stt` learned this the same way and its comment says so.
        // The `tr`/`sed` is not tidying. Whisper wraps its output with
        // embedded newlines, and the transcript arrives here through a
        // `SplitParser` on "\n" — so four wrapped lines would be **four
        // separate messages** sent to her, one turn each, instead of one
        // utterance. `souveraine-stt` collapses them on its own typing leg
        // and says why in a comment; `--file` prints them raw, so the same
        // trap arrives by the other door and has to be closed on this side.
        transcriber.command = ["sh", "-c",
            `kill -INT $(cat "$P" 2>/dev/null) 2>/dev/null; rm -f "$P"; sleep 0.4; ` +
            `[ -s "$W" ] || exit 0; souveraine-stt --file "$W" ` +
            `| tr '\\n\\r' '  ' | sed 's/  */ /g; s/^ //; s/ $//'; echo; rm -f "$W"`];
        transcriber.running = true;
        root._eval(`window.face.posture("thinking")`);
    }

    readonly property string _wav: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine-face-talk.wav"
    readonly property string _pid: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine-face-talk.pid"

    Process {
        id: recorder
        environment: ({ W: root._wav, P: root._pid })
    }

    Process {
        id: stopper
        environment: ({ W: root._wav, P: root._pid })
    }

    Process {
        id: transcriber
        environment: ({ W: root._wav, P: root._pid })
        stdout: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                const said = line.trim();
                if (said.length === 0)
                    return;
                // Straight into the one transport, so the sidebar logs it and
                // the reply streams back to the bubble through the same
                // `onStreamEvent` her own speech already uses.
                Souveraine.send(said);
            }
        }
        onExited: root._eval(`window.face.posture("idle")`)
    }

    // What the page says, where someone can see it.
    //
    // The host forwards console and errors on the same line protocol. Dropped
    // here, a rig that fails to draw is silent in every direction — which it
    // was, and it cost 2026-08-06 an afternoon: a missing `#live_talk` element
    // threw on the runtime's first update, after the model and all four
    // textures had loaded, so every other signal read healthy.
    function _pageSaid(level, text) {
        if (level === "error")
            console.warn("[face] page error:", text);
        else
            console.log("[face]", text);
    }

    // Reachable by name, so the dial, a launcher and the agent all summon her
    // the same way rather than each growing a copy (TASK-30/31). `status`
    // answers rather than assumes — an agent that cannot ask whether she is up
    // has to guess, and guessing is what a verb table exists to stop.
    IpcHandler {
        target: "face"

        function toggle(): void {
            root.toggle();
        }

        function join(): void {
            root.join();
        }

        function leave(): void {
            root.leave();
        }

        function status(): string {
            return JSON.stringify({
                joined: root.joined,
                rig: root.rigDir,
                rigConnected: rigSocketLoader.item?.connected ?? false,
                gazeConnected: gaze.connected
            });
        }
    }

    // Quickshell's Socket retains its QLocalSocket object after
    // ConnectionRefused. Setting `connected` false then true cannot retry:
    // socket.cpp only calls connectToServer when that object is null, and the
    // error path never nulls it. Recreate the Socket object after a failed
    // startup race; the second body gets a fresh QLocalSocket and can connect.
    Loader {
        id: rigSocketLoader
        active: false
        sourceComponent: Component {
            Socket {
                path: root.socketPath
                connected: true

                onConnectionStateChanged: {
                    if (connected)
                        pageReadyTimer.restart();
                }
            }
        }
    }

    Connections {
        target: root

        function onJoinedChanged() {
            root._rigConnectAttempts = 0;
            if (!root.joined)
                rigSocketLoader.active = false;
        }
    }

    Timer {
        id: faceSocketRecreateTimer
        interval: 500
        repeat: true
        triggeredOnStart: true
        running: root.joined
        onTriggered: {
            if (rigSocketLoader.item?.connected)
                return;
            if (root._rigConnectAttempts++ < 4)
                console.log("[face] connecting to rig host, attempt " + root._rigConnectAttempts);
            rigSocketLoader.active = false;
            faceSocketCreateTimer.restart();
        }
    }

    Timer {
        id: faceSocketCreateTimer
        interval: 50
        repeat: false
        onTriggered: {
            if (root.joined)
                rigSocketLoader.active = true;
        }
    }

    property int _rigConnectAttempts: 0

    Timer {
        id: pageReadyTimer
        interval: 250
        repeat: false
        onTriggered: root._syncHands()
    }

    function _send(msg) {
        const rigSocket = rigSocketLoader.item;
        if (rigSocket?.connected)
            rigSocket.write(JSON.stringify(msg) + "\n");
    }

    // ## She is the user's, so she leaves when the user does
    //
    // Casey, 2026-08-06: "if I lock the screen, she should probably assume to
    // turn off... she's not for everyone, just the user." Explicitly *not* the
    // same as switching to an app — she persists across app use; only the lock
    // takes her away.
    //
    // Gated on `screenLockSecure` — the compositor's acknowledgement — and NOT
    // on `screenLocked`, which is only the request.
    //
    // The request drifts. Measured on the phone 2026-08-06: `session lock`
    // answered `already-locked` while logind reported `LockedHint=no` and the
    // phone was in use, so `screenLocked` had been stuck true for some time.
    // That is CLAUDE.md's rule 2 exactly — a shadow copy of state the protocol
    // owns — and a face gated on it would have been permanently dismissed with
    // nothing on screen to explain why. `screenLockSecure` is the one
    // GlobalStates itself calls "the real 'session is locked' signal".
    //
    // Nothing is lost by waiting for the ack: the compositor composites lock
    // surfaces and nothing else while locked, so she is already off the glass
    // before this runs. This is about not holding 283MB of webview through a
    // locked night, not about disclosure.
    // She does not come back on unlock, deliberately. Casey, 2026-08-06: "I
    // want it recognized it locked, and going back to clock, and being clock
    // until we retrigger it." Unlocking returns you to the clock, and summoning
    // her is a double tap away — so the state you find is the plain one, and
    // the face is something you choose each time rather than something that
    // was left on.
    Connections {
        target: GlobalStates

        function onScreenLockSecureChanged() {
            if (GlobalStates.screenLockSecure && root.joined)
                root.leave();
        }
    }

    // Where fingers are, straight from the compositor, so she can look at them.
    //
    // Only open while she is up, because the compositor throttles but does not
    // stop: a feed nobody is reading is a socket buffer filling behind a face
    // that is not on screen.
    //
    // Screen coordinates come in; her window's own coordinates go out. The
    // compositor reports in logical panel pixels and the page thinks in CSS
    // pixels inside her window, so the origin has to be subtracted or she
    // looks at a point offset by however far down the panel she is standing.
    Socket {
        id: gaze
        path: (Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/1000") + "/souveraine/viewtop.sock"
        connected: root.joined

        // `onConnectionStateChanged`, not `onConnectedChanged` — Quickshell's
        // Socket emits the former, so the latter is a handler for a signal
        // that does not exist and never runs. The subscribe was therefore
        // never sent, the compositor never pushed, and she never followed a
        // finger. `ViewtopControl`'s feed had the right idiom the whole time.
        onConnectionStateChanged: {
            console.log("[face] gaze socket connected=" + gaze.connected);
            if (gaze.connected)
                gaze.write('{"op":"gaze"}\n');
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                let m;
                try {
                    m = JSON.parse(line);
                } catch (e) {
                    return;
                }
                if (root._gazeSeen === undefined) root._gazeSeen = 0;
                if (root._gazeSeen++ < 4)
                    console.log("[face] gaze push: " + line);
                if (m.ok !== undefined && m.down === undefined)
                    return;
                if (!m.down) {
                    root._eval("window.face.lookAway()");
                    return;
                }
                root._eval(`window.face.lookAt(${m.x - root.originX}, ${m.y - root.originY})`);
            }
        }
    }

    // Where her window sits on the panel. Read from the compositor's own
    // furniture report rather than assumed, because the layout decides it and
    // it moves with the zone.
    property var _gazeSeen: undefined
    property real originX: 0
    property real originY: 0

    Socket {
        id: whereAmI
        path: (Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/1000") + "/souveraine/viewtop.sock"

        onConnectionStateChanged: {
            if (whereAmI.connected)
                whereAmI.write('{"op":"state"}\n');
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                try {
                    const s = JSON.parse(line);
                    const f = (s.furniture ?? [])[0];
                    if (f?.at) {
                        root.originX = f.at.x;
                        root.originY = f.at.y;
                    }
                } catch (e) {}
                whereAmI.connected = false;
            }
        }
    }

    // Asked once she is up, and again a moment later: the first answer can
    // land before the compositor has stood her up, and then her origin is
    // whatever the last window left there.
    Timer {
        running: root.joined
        interval: 2000
        repeat: true
        triggeredOnStart: true
        onTriggered: whereAmI.connected = true
    }

    function _eval(script) {
        root._send({ op: "eval", script: script });
    }

    // Everything she says on the sidebar's stream reaches the bubble. The face
    // is a second *view* of one turn, never a second turn.
    Connections {
        target: Souveraine
        enabled: root.joined

        function onStreamEvent(event) {
            if (event.message_type === "assistant_message" && event.content) {
                root._line += event.content;
                // Posture tags are hers to emit and the bubble's to not show.
                const tag = /\[\[face:([a-z]+)\]\]/g;
                let m;
                while ((m = tag.exec(root._line)) !== null)
                    root._eval(`window.face.posture(${JSON.stringify(m[1])})`);
                const shown = root._line.replace(tag, "").trim();
                root._eval(`window.face.say(${JSON.stringify(shown)})`);
            }
        }

        function onTurnActiveChanged() {
            if (Souveraine.turnActive)
                root._line = "";
            root._syncHands();
        }

        function onConversationResumed(agentId, conversationId, messages) {
            root._syncHands();
        }

        function onCurrentAgentIdChanged() {
            root._syncHands();
        }

        function onConversationIdChanged() {
            root._syncHands();
        }
    }

    Connections {
        target: HidController

        function onControllerChanged() {
            root._syncHands();
        }
    }
}
