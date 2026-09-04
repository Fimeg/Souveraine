// Shell side of the souveraine-sessiond handoff protocol.
//
// sessiond takes ext-session-lock before the shell exists; this bridge is
// how the shell (a) announces itself and takes the lock over, (b) keeps the
// heartbeat connection open so sessiond can retake the lock the moment the
// shell dies, and (c) confirms the compositor-acked lock (locked_ack).
//
// The session is never unlocked during the handoff: sessiond abandons its
// lock (connection drop) and misc:allow_session_lock_restore lets our
// WlSessionLock inherit the locked session. See
// souveraine/src/sessiond/protocol.rs — that file is the contract.
//
// No sessiond on the socket (laptop, or bring-up) = everything no-ops and
// the legacy launchOnStartup path decides alone.
pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.modules.common.functions

Singleton {
    id: root

    // Only shell.qml calls claimAuthority(). Importing this singleton from a
    // utility window must never create a session-authority connection.
    property bool authorityScope: false

    // Registered = shell_ready was answered ok on the CURRENT connection.
    property bool registered: false
    // Latched copy of `registered` taken when the connection drops. The
    // disconnect branch clears `registered` before the reconnect branch runs,
    // so reconnect cannot read it directly to decide whether to re-register.
    property bool wasRegistered: false
    // We owe the authority a registration that has not landed yet — set when a
    // handshake times out or is refused because the lease is still held,
    // cleared once one is answered. Drives registerRetry.
    property bool needsRegistration: false
    // Consecutive "already registered" refusals. Only for log cadence — the
    // retry itself is unconditional, because the common cause is our own
    // outgoing connection not having hit EOF yet.
    property int refusalStreak: 0
    // sessiond held the session lock when we registered; we owe it a lock
    // and a locked_ack.
    property bool oweLock: false
    property bool ackSent: false
    property var pendingReady: null // callback awaiting the shell_ready response

    function claimAuthority() {
        root.authorityScope = true
    }

    function load() {
        if (!root.authorityScope)
            console.log("[sessiond-bridge] inactive outside authority scope")
    }

    // One short-lived request connection for power authority. Never put this
    // on `sock`: suspend can keep the daemon's synchronous request handler
    // occupied until resume, while the heartbeat connection must remain free
    // to carry locked_ack, directives and the EOF that means shell death.
    //
    // This is deliberately only the transport seam for now. Session.qml keeps
    // its legacy executor until the packaged daemon and its polkit subject have
    // been proven on each target. A locally accepted request returns pending;
    // the callback and powerFinished carry the daemon's eventual verdict.
    property var pendingPower: null
    property int powerRequestSequence: 0
    signal powerFinished(string requestId, var reply)

    function requestPower(verb, callback) {
        const powerVerb = String(verb ?? "");
        if (!["poweroff", "reboot", "suspend", "hibernate"].includes(powerVerb)) {
            return {
                ok: false,
                code: "unsupported",
                reason: "unsupported power verb: " + powerVerb
            };
        }
        if (root.pendingPower !== null) {
            return {
                ok: false,
                code: "refused_by_state",
                reason: "another power request is already in flight"
            };
        }

        root.powerRequestSequence += 1;
        const requestId = "power-" + Date.now() + "-" + root.powerRequestSequence;
        root.pendingPower = {
            requestId: requestId,
            verb: powerVerb,
            callback: typeof callback === "function" ? callback : null,
            sent: false
        };
        powerConnectTimeout.restart();
        powerSock.connected = true;
        return { ok: true, status: "pending", request_id: requestId };
    }

    function _finishPower(reply) {
        if (root.pendingPower === null)
            return;
        const pending = root.pendingPower;
        root.pendingPower = null;
        powerConnectTimeout.stop();

        const result = {};
        for (const key in reply)
            result[key] = reply[key];
        result.request_id = pending.requestId;

        // Clear our request before closing. The disconnect edge must not turn
        // a parsed refusal/acceptance into a second outcome_unknown callback.
        powerSock.connected = false;
        root.powerFinished(pending.requestId, result);
        if (pending.callback !== null) {
            try {
                pending.callback(result);
            } catch (error) {
                console.error("[sessiond-bridge] power callback failed: " + error);
            }
        }
    }

    Timer {
        id: powerConnectTimeout
        interval: 1500
        repeat: false
        onTriggered: {
            if (root.pendingPower !== null && !root.pendingPower.sent) {
                root._finishPower({
                    ok: false,
                    code: "unavailable",
                    reason: "sessiond power socket unavailable"
                });
            }
        }
    }

    Socket {
        id: powerSock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/sessiond.sock"

        onConnectionStateChanged: {
            if (powerSock.connected && root.pendingPower !== null
                    && !root.pendingPower.sent) {
                const pending = root.pendingPower;
                root.pendingPower = {
                    requestId: pending.requestId,
                    verb: pending.verb,
                    callback: pending.callback,
                    sent: true
                };
                powerConnectTimeout.stop();
                powerSock.write(JSON.stringify({
                    op: "power",
                    verb: pending.verb
                }) + "\n");
                powerSock.flush();
            } else if (!powerSock.connected && root.pendingPower !== null) {
                const sent = root.pendingPower.sent;
                root._finishPower(sent ? {
                    ok: false,
                    code: "outcome_unknown",
                    status: "outcome_unknown",
                    reason: "sessiond disconnected after the power request was sent"
                } : {
                    ok: false,
                    code: "unavailable",
                    reason: "sessiond power socket unavailable"
                });
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                if (root.pendingPower === null)
                    return;
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (error) {
                    root._finishPower({
                        ok: false,
                        code: "outcome_unknown",
                        status: "outcome_unknown",
                        reason: "sessiond returned an unparseable power reply"
                    });
                    return;
                }
                root._finishPower(reply);
            }
        }
    }

    // Announce the shell. cb(mustLock) fires exactly once: mustLock true
    // means sessiond was holding and the session IS locked — the shell must
    // raise its own lock surface immediately.
    //
    // EVERY failure path here answers TRUE. Not knowing whether the session is
    // locked is not the same as knowing it is not, and the two must never be
    // collapsed: sessiond takes ext-session-lock before any shell surface can
    // exist (protocol.rs §1), so a shell that cannot get an answer is a shell
    // that arrived after something already locked the session. The daemon side
    // of this contract is explicit — heartbeat EOF retakes the lock "whether or
    // not the session was locked at the time. Fail closed." — and this side has
    // to match it or the pair fails open at exactly the moment the authority is
    // unreachable.
    //
    // Answering true costs an unnecessary lock surface the user dismisses with
    // PAM. Answering false costs an unlocked phone.
    function shellReady(cb) {
        if (!root.authorityScope) {
            // Not a fail-open: a utility QML process is not the session
            // authority and has no lock to owe. Only shell.qml claims scope.
            console.log("[sessiond-bridge] shell_ready refused outside authority scope")
            cb(false)
            return
        }
        if (!sock.connected) {
            console.warn("[sessiond-bridge] shell_ready with no socket — assuming locked");
            cb(true);
            return;
        }
        if (root.pendingReady) {
            // A handshake is already in flight; it will answer authoritatively.
            // This duplicate assumes locked rather than racing it to "unlocked".
            console.warn("[sessiond-bridge] duplicate shell_ready — assuming locked");
            cb(true);
            return;
        }
        root.pendingReady = cb;
        readyTimeout.restart();
        sock.write(JSON.stringify({ op: "shell_ready" }) + "\n");
        sock.flush();
    }

    function sendLockedAck() {
        if (!sock.connected || !root.registered || root.ackSent) return;
        root.ackSent = true;
        sock.write(JSON.stringify({ op: "locked_ack" }) + "\n");
        sock.flush();
        console.log("[sessiond-bridge] locked_ack sent");
    }

    Timer {
        id: readyTimeout
        // Longer than sessiond's own deadline, and that ordering is the whole
        // point. `shell_ready` blocks in the daemon for up to 5 s waiting for
        // its lock-session thread to drop its Wayland connection, because the
        // compositor refuses a second locker while the first is alive
        // (server.rs, `wait_timeout_while`). At 3 s this timer fired *first*,
        // so the shell gave up on a handshake the daemon was still answering,
        // assumed locked, and asked for a lock sessiond had not released yet —
        // straight into TASK-48's `Tried to show lockscreen surfaces without
        // active lock`.
        //
        // Under Hyprland that race is usually won: the release lands in
        // milliseconds. Measured against viewtop on blueline 2026-08-02 it
        // loses every time — the shell crash-looped every 11 s and never came
        // up. Same latent bug, a compositor that exposes it.
        //
        // The daemon has an answer for this case and it is a refusal
        // (`lock session did not release in time`). Waiting for a real refusal
        // beats inventing a verdict: `assuming locked` is the shell holding
        // state the authority owns, which is the failure doctrine §4 is about.
        interval: 7000
        repeat: false
        onTriggered: {
            if (root.pendingReady) {
                // Was "proceeding without sessiond" with cb(false): a silent
                // fail-open that left the session unlocked precisely when the
                // authority was not answering. Assume locked, and keep trying —
                // an unanswered handshake is a transient (sessiond restarting),
                // not a verdict.
                console.warn("[sessiond-bridge] shell_ready timed out — assuming locked, will retry");
                const cb = root.pendingReady;
                root.pendingReady = null;
                root.needsRegistration = true;
                cb(true);
            }
        }
    }

    // Register, and apply whatever the authority says we owe. One path, used by
    // the reconnect handler and the retry timer alike, so the two can never
    // drift into handling the answer differently.
    function registerWithAuthority() {
        root.shellReady(function(mustLock) {
            if (!mustLock)
                return;
            // sessiond locked while we were disconnected (it treats our EOF as
            // shell death), or we could not confirm and are failing closed.
            GlobalStates.screenLocked = true;
            // Our surface may ALREADY be secure from before the daemon
            // restarted. onScreenLockSecureChanged is an edge, and that edge is
            // in the past, so nothing would ever send the ack this new handoff
            // owes — sessiond waits out its timer and retakes the lock
            // ("shell never confirmed its lock after handoff").
            if (GlobalStates.screenLockSecure)
                root.sendLockedAck();
        });
    }

    // A handshake that timed out is retried until it lands. Without this a
    // shell that merely started while sessiond was restarting stays
    // unregistered for its whole life: sessiond sees no heartbeat, believes the
    // shell is dead, and raises its own fallback surface over ours forever.
    Timer {
        id: registerRetry
        interval: 5000
        repeat: true
        running: root.authorityScope && root.needsRegistration
                 && !root.registered && sock.connected
        onTriggered: root.registerWithAuthority()
    }

    // Reconnect: sessiond may restart (upgrade) or start late. While
    // connected this timer is idle; the Socket does not retry by itself.
    Timer {
        id: reconnect
        interval: 5000
        repeat: true
        running: root.authorityScope && !sock.connected
        onTriggered: sock.connected = true
    }

    Socket {
        id: sock
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/sessiond.sock"
        connected: root.authorityScope

        // `connectionStateChanged` is the real Signal on Quickshell.Io.Socket;
        // the `onSocketConnected`/`onSocketDisconnected` handler-slots aren't
        // reliably attachable across quickshell builds, so branch on
        // `connected` here. Re-register on reconnect if we were registered
        // before (sessiond restarted underneath us).
        onConnectionStateChanged: {
            if (sock.connected) {
                console.log("[sessiond-bridge] connected");
                const wasRegistered = root.wasRegistered;
                root.registered = false;
                root.wasRegistered = false;
                root.ackSent = false;
                if (wasRegistered || root.needsRegistration) {
                    root.registerWithAuthority();
                }
            } else {
                console.log("[sessiond-bridge] disconnected");
                root.wasRegistered = root.registered;
                root.registered = false;
                root.pendingReady = null;
            }
        }

        parser: SplitParser {
            splitMarker: "\n"
            onRead: message => {
                let reply;
                try {
                    reply = JSON.parse(message);
                } catch (e) {
                    console.log("[sessiond-bridge] unparseable reply: " + message);
                    return;
                }
                // Not every line is a reply. The authority pushes directives
                // down this connection: it owns the decision, the shell owns
                // the surface. A blank that needs a lock first arrives here
                // (LOCK-DPMS-LESSONS §1 — lock, then off), and the daemon is
                // holding the panel dark until this is answered.
                if (reply.directive !== undefined) {
                    root.handleDirective(reply);
                    return;
                }
                if (root.pendingReady) {
                    // The only request we await a response for.
                    const cb = root.pendingReady;
                    root.pendingReady = null;
                    readyTimeout.stop();
                    if (reply.ok) {
                        root.registered = true;
                        root.needsRegistration = false;
                        root.refusalStreak = 0;
                        root.oweLock = reply.must_lock === true;
                        console.log("[sessiond-bridge] registered, must_lock=" + root.oweLock);
                        cb(root.oweLock);
                    } else {
                        // "already registered" means the lease is held right
                        // now. It does NOT mean it is held by someone else,
                        // and treating it as a verdict was a real bug: a scene
                        // RELOAD re-runs this file while the outgoing
                        // connection is still open, so the reload's shell_ready
                        // races its own predecessor's EOF and is refused. The
                        // old code then stopped retrying — and when that EOF
                        // landed a second later it cleared shell_alive, leaving
                        // sessiond believing there was no shell at all, for the
                        // life of the session. Observed 2026-07-29 09:14:35:
                        // refused, old socket closed 09:16:36, and the daemon
                        // reported shell_alive=false with a live shell on the
                        // other end of a connected socket.
                        //
                        // So it is a TRANSIENT. Keep registering. If the lease
                        // really is another live shell's, every retry is
                        // refused again and costs nothing — and the moment that
                        // shell dies we are the one that should hold it.
                        const leaseHeld =
                            String(reply.reason || "").indexOf("already registered") !== -1;
                        if (leaseHeld) {
                            root.needsRegistration = true;
                            root.refusalStreak += 1;
                            // Loud once, then once a minute: a lease that never
                            // frees is a real problem, but 12 lines a minute is
                            // how a real problem gets scrolled past.
                            if (root.refusalStreak === 1 || root.refusalStreak % 12 === 0)
                                console.warn("[sessiond-bridge] shell_ready refused: "
                                    + reply.reason + " — lease still held, retrying ("
                                    + root.refusalStreak + ")");
                        } else {
                            console.warn("[sessiond-bridge] shell_ready refused: "
                                + reply.reason + " — assuming locked");
                        }
                        cb(!leaseHeld);
                    }
                }
            }
        }
    }

    // Carry out an authority directive. The shell is the executor here, not a
    // peer deciding whether it agrees: sessiond is the session authority and
    // it has already withheld the panel waiting for this.
    //
    // Unknown directives are LOUD. A newer daemon asking for something this
    // shell cannot do is a real divergence, and silently dropping it would
    // leave the daemon waiting out its ack budget and then blanking unlocked.
    function handleDirective(msg) {
        if (msg.directive === "lock") {
            console.log("[sessiond-bridge] authority directive: lock ("
                + (msg.why || "no reason given") + ")");
            Session.lock();
            return;
        }
        if (msg.directive === "accessory_presentation") {
            // sessiond sends this only after producer admission and content
            // classification. The shell projects it; it does not reinterpret
            // a Bluetooth observation into an authority decision.
            AccessoryPresentation.present(msg.presentation ?? {});
            return;
        }
        console.error("[sessiond-bridge] UNKNOWN authority directive: "
            + JSON.stringify(msg)
            + " — this shell is older than the daemon driving it");
    }

    // The compositor acknowledged OUR lock surface — tell sessiond the
    // handoff is complete. Gated on secure, not the request, per doctrine.
    Connections {
        target: GlobalStates
        function onScreenLockSecureChanged() {
            if (GlobalStates.screenLockSecure && root.oweLock && !root.ackSent) {
                root.sendLockedAck();
            }
        }
    }
}
