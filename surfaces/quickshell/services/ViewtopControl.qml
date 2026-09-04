// The shell's door into viewtop's control socket.
//
// The compositor serves one JSON object per line — `{"op":…}` in,
// `{"ok":true,…}` or `{"ok":false,"code":…,"reason":…}` back — the same house
// grammar sessiond speaks, so this is shaped like SessiondPolicy rather than
// inventing a second idea of what talking to a daemon looks like.
//
// **This is the only place the shell addresses the scene.** Every window verb
// the hand can reach — close, kill, place, pose, raise, focus — goes through
// `scene()`. The alternative is what the dial does today: `hyprctl dispatch`
// baked into a surface, which stopped existing under viewtop and took the
// dial's "Kill window" with it. A verb spelled out inside a widget is a verb
// that dies when the compositor changes.
//
// A short-lived connection per request, deliberately. viewtop's socket is
// request/response and holds no session; there is no heartbeat to protect here
// the way SessiondBridge's EOF is load-bearing, so nothing is gained by
// keeping it open and a dropped long-lived socket would need reconnect logic
// to be correct.
//
// Failures are LOUD. A window verb that silently did nothing is exactly the
// bug this exists to close: `overview toggle` shelled out to a handler that
// did not exist and the tap did nothing, silently, for weeks.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    // The last refusal, for a surface that wants to show one. `close` is a
    // request the client may refuse; the hand deserves to see that rather than
    // watch nothing happen.
    property string lastError: ""

    signal refused(string intent, string reason)
    signal succeeded(string intent)

    // What the compositor last said is on the canvas: `[{id, workspace}, …]`.
    // Queried, never cached across opens — a chooser showing a window that
    // closed a minute ago is worse than one that takes a moment to fill.
    property var windows: []
    signal windowsChanged_()

    // The serialised form of the last canvas we published, so an unchanged
    // answer is not republished.
    //
    // This is not a micro-optimisation, it is a correctness fix. `windows` is a
    // `var` holding a fresh array on every reply, so assigning it fires
    // `windowsChanged` whether or not anything changed. A surface that binds a
    // ListView model to it therefore had its entire delegate tree — and every
    // `ScreencopyView` inside it — destroyed and rebuilt on the poll interval,
    // which is a view that flickers and loses its place while you are reading
    // it. Republish on *difference*, the same level-not-edge discipline
    // `lockhint.rs` applies to `LockedHint`.
    property string _lastCanvas: ""

    // Which zone is in front, as the compositor last reported it. -1 is
    // "not asked yet" and is deliberately not 0: home is 0, so defaulting to it
    // would make every surface believe it was on home before the first reply.
    property int activeZone: -1
    property int zoneCount: 0
    // Home is zone 0, matching `workspace::HOME_ZONE` in the compositor. Named
    // here rather than written as a literal at each call site so the two ends
    // of the wire have one place to disagree if it ever moves.
    readonly property int homeZone: 0

    // Ask what is open, and where we are. Answers into `windows`/`activeZone`.
    function refreshWindows() {
        root._send({ op: "workspaces" });
    }

    // The colour ramp in force, as the compositor last reported it. `-1` is
    // "not asked yet" and is deliberately not 100: 100 is identity, a real
    // answer, so defaulting to it would have every surface believe the panel
    // was untinted before the first reply.
    //
    // Read back rather than remembered because the compositor is the writer of
    // record. A compositor restart resets its ramp to identity, and a shell
    // that cached its own idea of the ramp across that would draw a night-light
    // indicator over a panel that is no longer warm.
    property int gammaValue: -1
    property int gammaTemperature: 0
    // The output carrying ViewTop's seat attention. Empty means the state
    // reply has not arrived yet (or the compositor predates the field).
    property string activeOutputName: ""
    property var outputs: []

    // Panel power, outputs, borders and the ramp. Answers into `gammaValue` /
    // `gammaTemperature`.
    function refreshState() {
        root._send({ op: "state" });
    }

    // True once the compositor has accepted a `subscribe` and is pushing canvas
    // changes down the second socket below.
    property bool subscribed: false
    // A compositor that does not serve `subscribe` will not start serving it
    // while it is running. Latch the refusal, or the disconnect that follows it
    // re-arms the retry that the refusal just stopped — which is a reconnect
    // loop at timer speed against a socket that will keep saying no. Cleared by
    // the shell restarting, which is also when the compositor has changed.
    property bool _pushRefused: false

    // The poll, which now exists only as the fallback for a compositor too old
    // to push.
    //
    // TASK-60 Q4: *"Either the compositor pushes zone/window changes, or this
    // surface asks synchronously when it opens and stops guessing in between."*
    // Polling was the proximate cause of the multitasking view scaling the
    // wrong windows — a two-second answer is wrong for the whole of every
    // gesture, and a gesture is exactly when something asks. The push channel
    // is the answer; this stays because the phone can be running a compositor
    // that predates it, and a shell that hard-depends on an op the running
    // compositor does not serve is a shell that breaks on the deploy ordering
    // TASK-28 is made of.
    Timer {
        interval: 2000
        running: !root.subscribed
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refreshWindows()
    }

    // Take one reply's canvas facts, whether it arrived as an answer or as a
    // push. Both carry the same shape by construction — the compositor
    // serialises the `workspaces` payload once and uses it for both — so there
    // is one reader here rather than two that can drift.
    function _ingest(reply) {
        if (reply.windows !== undefined) {
            // Compare before publishing. See `_lastCanvas`.
            const encoded = JSON.stringify(reply.windows);
            if (encoded !== root._lastCanvas) {
                root._lastCanvas = encoded;
                root.windows = reply.windows;
                root.windowsChanged_();
            }
        }
        // `active` is an index and 0 is a real, meaningful value — it is home —
        // so this must test for presence, not truthiness. `if (reply.active)`
        // would silently ignore every report that we are on home, which is the
        // one zone anything here cares about.
        if (reply.active !== undefined)
            root.activeZone = reply.active;
        if (reply.count !== undefined)
            root.zoneCount = reply.count;
        // A `state` reply carries the ramp. `temperature` is null when the
        // channels are balanced, which is not the same as 0 K — flatten it to 0
        // here so a consumer can test one number.
        if (reply.gamma !== undefined) {
            root.gammaValue = reply.gamma.value;
            root.gammaTemperature = reply.gamma.temperature || 0;
        }
        if (reply.outputs !== undefined)
            root.outputs = reply.outputs;
        if (reply.active_output !== undefined && reply.active_output !== null)
            root.activeOutputName = reply.active_output;
    }

    // The push channel: a second, long-lived connection that carries canvas
    // changes as they happen.
    //
    // Separate from the request socket on purpose. viewtop answers one request
    // per connection and then closes; a subscription is the opposite shape — it
    // is written to, never read from, and outlives every request. Multiplexing
    // both onto one socket would mean interleaving a push into the middle of
    // somebody's reply, which is how a request/response client learns to
    // distrust its own parser.
    Socket {
        id: feed
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/viewtop.sock"
        connected: true

        onConnectionStateChanged: {
            if (connected) {
                feed.write(JSON.stringify({ op: "subscribe" }) + "\n");
            } else {
                // Either the compositor went away or it never served the op.
                // Both mean the poll is the truth again until we get back in.
                root.subscribed = false;
                if (!root._pushRefused)
                    resubscribe.restart();
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (e) {
                    console.error("[viewtop-control] unparseable push: " + message);
                    return;
                }
                if (reply.ok === false) {
                    // A compositor that does not serve `subscribe` says so with
                    // a code, which is the whole point of the codes. Stop
                    // asking, say it once, and let the poll carry it — this is
                    // the ordinary state of a phone between a shell update and
                    // the compositor package that follows it.
                    console.log("[viewtop-control] no push channel ("
                        + (reply.code || "refused")
                        + "); falling back to the 2 s poll");
                    root.subscribed = false;
                    root._pushRefused = true;
                    resubscribe.stop();
                    feed.connected = false;
                    return;
                }
                root.subscribed = true;
                root._ingest(reply);
            }
        }
    }

    // Reconnect the feed after the compositor restarts. A session restart is
    // the ordinary case: the shell outlives individual compositor runs, and a
    // subscription that never came back would leave every surface reading a
    // canvas frozen at the moment of the crash.
    Timer {
        id: resubscribe
        interval: 5000
        repeat: false
        onTriggered: if (!root.subscribed && !root._pushRefused) feed.connected = true
    }

    // Requests waiting on a connection. A queue rather than SessiondPolicy's
    // single slot: the sheet can fire two verbs in a row (kill after a close
    // that was refused), and dropping the second would be silent.
    property var _queue: []
    property var _inflight: null

    // --- Verbs that must arrive ---------------------------------------------
    //
    // A transport error is not the compositor refusing. It is the message not
    // being delivered, and for a handful of verbs those two have completely
    // different consequences: a lost `overview_commit` leaves the compositor
    // still carrying windows the shell believes it released, so the app stays
    // shrunk and **nothing in the system will ever put it back**. Casey,
    // 2026-08-16: *"once I put a window in multitasking sometimes it glitches
    // and then I have to drag it back down manually… once I reopen it"* —
    // reopening starts a fresh carry, and it is that carry's commit that
    // finally releases the stranded one.
    //
    // Measured the same morning on blueline, one boot, ~90 minutes of ordinary
    // use: **52** transport failures, each of which executed `_queue = []`, and
    // 113 `PeerClosedError`s behind them. The odds of one of those landing on a
    // commit are not small, which is exactly the "sometimes".
    //
    // All three are idempotent — committing a released carry is `Idle`, and
    // going to the zone you are already on is a no-op — so redelivering costs
    // nothing and losing one costs the gesture. Everything else still drops:
    // `overview_progress` is a level and the next one supersedes it.
    //
    // One retry, not a loop. A compositor that is genuinely gone must not turn
    // this into a spin.
    readonly property var _mustArrive: ["overview_commit", "overview_cancel", "workspace"]

    function _mustBeDelivered(msg) {
        if (!msg)
            return false;
        const name = msg.intent || msg.op;
        return root._mustArrive.indexOf(name) !== -1 && (msg._tries || 0) < 1;
    }
    // Whether the in-flight request was answered before the socket closed.
    // The compositor serves exactly one request per connection and then hangs
    // up, so a disconnect is the *normal* end of every exchange — and telling
    // that apart from a compositor that died mid-request is the difference
    // between a silent success and a spurious "socket unavailable".
    property bool _replied: false

    function _send(msg) {
        root._queue.push(msg);
        root._pump();
    }

    // One request per connection, because that is what the other end serves.
    //
    // This used to write the whole queue down a single socket, which worked for
    // exactly one request: `handle()` in `control.rs` reads one line, answers,
    // and drops the stream. The second verb of any pair — `kill` after a
    // refused `close`, the one case the queue exists for — was written into a
    // socket that had already been closed, and surfaced as a refusal of a verb
    // that was never delivered. So the connection is re-established per
    // request, and a close with nothing in flight is silence rather than an
    // error.
    function _pump() {
        if (root._inflight !== null || root._queue.length === 0)
            return;
        if (!sock.connected) {
            sock.connected = true;
            return;
        }
        root._inflight = root._queue.shift();
        root._replied = false;
        sock.write(JSON.stringify(root._inflight) + "\n");
    }

    // Address the scene. `intent` is one of the compositor's own — the table it
    // returns from `{"op":"describe"}` is authoritative, and this deliberately
    // does not keep a second copy of it to validate against.
    function scene(intent, args) {
        const msg = { op: "scene", intent: intent };
        for (const k in args)
            msg[k] = args[k];
        root._send(msg);
    }

    // Ask the client to close. It may refuse or prompt; that is the protocol,
    // not a bug, and `refused` carries it.
    function close(id) {
        root.scene("close", { id: id });
    }

    // End it regardless. The floor under close(), for a client that is hung or
    // says no — "two apps and no way to turn them off" is what this answers.
    // Unsaved work is lost, so a surface offering this should mean it.
    function kill(id) {
        root.scene("kill", { id: id });
    }

    // Position and size a window. The shell owns layout; the compositor owns
    // whether a placement is legal and will refuse one that is not.
    function place(id, x, y, width, height) {
        root.scene("place", {
            id: id,
            at: { x: x, y: y },
            size: { width: width, height: height }
        });
    }

    // Give a window back to the layout.
    function unplace(id) {
        root.scene("unplace", { id: id });
    }

    // Let a window out of its zone.
    //
    // TASK-60 Q6. A zone tiles what is on it, which is right for the thing you
    // are doing and wrong for the thing you are keeping — a video that should
    // survive going somewhere else, a call, anything picture-in-picture. Those
    // want to leave the zone's confinement rather than take a half of it.
    //
    // Distinct from `place`: a placed window is still the zone's, put somewhere
    // specific in it. A floated one has stopped being the zone's business, so
    // it is not counted when the strip decides whether a zone still has
    // anything on it. That is also why it must be visible on a card — a float
    // is a window you can no longer find by remembering which zone you left it
    // on.
    function float(id) {
        root.scene("float", { id: id });
    }

    function unfloat(id) {
        root.scene("unfloat", { id: id });
    }

    // Compose a window's visual geometry — the verb that makes a live app a
    // scaled, floating, still-touchable thing rather than a picture of one.
    // The compositor maps input back through the inverse, so a shrunken window
    // still receives touch where it is drawn.
    function pose(id, scale, rotation, anchorX, anchorY) {
        root.scene("pose", {
            id: id,
            scale: scale,
            rotation: rotation ?? 0.0,
            anchor: { x: anchorX ?? 0.5, y: anchorY ?? 0.5 }
        });
    }

    // Hand a window to the finger. While grabbed, one-finger drags move it;
    // `drop` ends the mode. This is Move as a *mode* rather than a computed
    // geometry — the three-finger carry that used to do it lost a race with
    // the three-finger tap every time, so it is entered on purpose now.
    function grab(id) {
        root.scene("grab", { id: id });
    }

    function drop() {
        root.scene("drop", {});
    }

    // The overview carry: the compositor takes the real windows and puts them
    // on their cards.
    //
    // These replace `poseActiveZone` / `clearPose` / `_posed`, which were the
    // shell's half of TASK-60's two-writer bug — the rail scaled the windows
    // here while `ZoneOverview` scaled its cards independently, and a transform
    // the shell applied is a transform some path out of the gesture has to
    // remember to undo. There was always a path that forgot, and tapping a card
    // was it.
    //
    // What makes this different is not that it is tidier: the transform is
    // released *by the compositor*, on `commit` or `cancel`, and there is no
    // third way for a carry to end. A window cannot outlive the gesture that
    // carried it.
    //
    // `ZoneTransition` is the one caller. It owns the rects, the in-flight
    // flag, and the end-target table; this is only the door.
    function overviewBegin(targets) {
        root._send({ op: "scene", intent: "overview_begin", to: targets });
    }

    function overviewProgress(shift) {
        root._send({ op: "scene", intent: "overview_progress", progress: shift });
    }

    // `target` is the wire's `EndTarget`: "home", "overview", "last_zone", or
    // {zone: {zone: N}}.
    function overviewCommit(target) {
        root._send({ op: "scene", intent: "overview_commit", target: target });
    }

    function overviewCancel() {
        root._send({ op: "scene", intent: "overview_cancel" });
    }

    function raise(id) {
        root.scene("raise", { id: id });
    }

    // Go to a zone. Not a `scene` intent — zones are the canvas, not a surface
    // on it, so the compositor serves this as its own op.
    //
    // "Zone", not "workspace", in everything we name: viewtop's canvas is a
    // large scalable space of states rather than Hyprland's numbered desks, and
    // the vocabulary is being moved off Hyprland's deliberately. The wire op is
    // still spelled `workspace` — renaming that is a separate sweep, and doing
    // it halfway would leave the shell calling an op the compositor does not
    // serve.
    function zone(to) {
        root._send({ op: "workspace", to: to });
    }

    // Move a window to a zone without going there. Same op, its other half —
    // `workspace` takes an optional surface and moves it before it looks.
    function moveToZone(id, to) {
        root._send({ op: "workspace", to: to, surface: id });
    }

    function focus(id) {
        root.scene("focus", { id: id });
    }

    // Set the panel's colour ramp. `value` is 0..=100 and scales the curve;
    // 100 with no temperature is identity. `temperature` is Kelvin.
    //
    // Gamma is a pixel claim on the glass, so the compositor owns it — not
    // sessiond, which owns device *states*. The slider used to reach `hyprctl
    // hyprsunset`, which stopped existing with Hyprland and took both the
    // dimming and the night-light with it, silently, since the viewtop move.
    //
    // One writer, one ramp: brightness scaling and the evening warmth are two
    // curve generators composed into a single LUT on the far side rather than
    // two callers racing for the same hardware slot. That is why this takes
    // both arguments at once instead of offering a `temperature()` of its own.
    //
    // Refused with `unavailable` when the panel is off — there is nothing to
    // ramp — so a caller must not read a refusal here as the verb missing.
    function gamma(value, temperature) {
        const msg = { value: Math.round(Math.max(0, Math.min(100, value))) };
        if (temperature)
            msg.temperature = Math.round(temperature);
        root.scene("gamma", msg);
    }

    Socket {
        id: sock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/viewtop.sock"

        onConnectionStateChanged: {
            if (connected) {
                root._pump();
                return;
            }
            // Answered, then hung up: the exchange completed. Carry on with
            // whatever is behind it.
            if (root._inflight === null && root._replied) {
                root._pump();
                return;
            }
            // Nothing was in flight and nothing is waiting — an idle socket
            // closing is not news.
            if (root._inflight === null && root._queue.length === 0)
                return;

            const failed = root._inflight;
            const intent = (failed && failed.intent) || (failed && failed.op) || "?";
            root.lastError = "viewtop socket unavailable";

            // Keep what must arrive, drop what was only a level. See
            // `_mustArrive` — this used to be `_queue = []` unconditionally,
            // which is how a commit went missing and a window stranded.
            const keep = [];
            for (const msg of [failed].concat(root._queue)) {
                if (!root._mustBeDelivered(msg))
                    continue;
                msg._tries = (msg._tries || 0) + 1;
                keep.push(msg);
            }
            const dropped = root._queue.length + (failed ? 1 : 0) - keep.length;

            // Name the verb and the counts. The old wording claimed the
            // compositor could not be reached, which was the one thing it never
            // established — on 2026-08-13 this fired once per shell start while
            // viewtop answered a hand-written request on that same socket in
            // the same second. A transport error is not a diagnosis, and one
            // that does not say what it ate cannot be traced to the symptom it
            // caused three days later.
            console.error("[viewtop-control] " + intent + " was not delivered:"
                + " the socket at " + sock.path + " closed with the request in"
                + " flight (" + keep.length + " redelivered, "
                + dropped + " dropped)");

            root._inflight = null;
            root._queue = keep;
            if (keep.length === 0)
                root.refused(intent, root.lastError);
            else
                root._pump();
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                const sent = root._inflight;
                const intent = (sent && sent.intent) || (sent && sent.op) || "?";
                root._inflight = null;
                root._replied = true;

                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (e) {
                    console.error("[viewtop-control] unparseable reply: " + message);
                    root.lastError = "unparseable reply";
                    root.refused(intent, root.lastError);
                    if (sock.connected)
                        sock.connected = false;
                    else
                        root._pump();
                    return;
                }

                if (reply.ok !== true) {
                    // `gone` is the ordinary one: the window closed between the
                    // sheet opening and the button being pressed. Still a
                    // refusal, still surfaced, because a sheet acting on a dead
                    // id should say so rather than appear to work.
                    const why = reply.reason || reply.code || "refused without a reason";
                    console.log("[viewtop-control] " + intent + " refused: " + why);
                    root.lastError = why;
                    root.refused(intent, why);
                } else {
                    root.lastError = "";
                    // A `workspaces` reply carries the canvas rather than a
                    // verb's outcome. Captured here so a chooser has something
                    // real to list instead of a guess at what is open.
                    root._ingest(reply);
                    root.succeeded(intent);
                }
                // Hang up rather than wait to discover we have been hung up on.
                //
                // `handle()` in control.rs answers one request and returns, so
                // the connection is spent the moment the reply is parsed. But
                // `connected` is this end's belief and it lags the peer's
                // close: measured 2026-08-13, the poll's second tick wrote to
                // the socket a full two seconds after the reply and the write
                // still went out, dying as PeerClosedError. That surfaced as
                // "could not reach the compositor" for a compositor that was
                // answering by hand at that same moment, and it cost the queued
                // verb. Closing here makes the next `_pump` start from a state
                // we set rather than one we inferred.
                if (sock.connected)
                    sock.connected = false; // the disconnect handler re-pumps
                else
                    root._pump();
            }
        }
    }
}
