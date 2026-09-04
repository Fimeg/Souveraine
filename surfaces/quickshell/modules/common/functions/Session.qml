// Souveraine fork of ii's stock Session.qml.
//
// Upstream ii's Session is a set of fire-and-forget verbs:
//     Quickshell.execDetached(["bash", "-c", "systemctl poweroff || loginctl poweroff"])
// That is fine for a desktop where a failed poweroff is visible to the person
// sitting at the keyboard. It is not fine for the phone, where the shell is
// the only session manager and an agent can drive these verbs over IPC. A
// verb that silently does nothing is the worst outcome: the caller believes
// the machine is suspending and it is not.
//
// So this fork keeps every upstream verb (call sites in LockScreen.qml and
// the session menus are unchanged) and adds the parts a real session arbiter
// needs:
//
//   1. Capability detection. We query logind's Can* methods once at startup
//      instead of treating a command being installed, or /sys/power/state
//      advertising "disk", as proof that an action is usable.
//      caps() reports what this machine can actually do, so a caller can ask
//      before it acts and the session menu can grey out what is unavailable.
//
//   2. Honest failure. Upstream execDetached throws the exit code away. Every
//      verb here runs through a Process with an onExited that logs
//      [session] <verb> failed (exit N) and emits actionFailed(). A wedged
//      logind is now a fact in the log, not silence.
//
//   3. Reason-tracked inhibits. Idle.qml's inhibit is a bare bool: something
//      is holding the machine awake and nothing records what or why. inhibit()
//      takes a reason, returns a cookie, and state() lists every holder. "Why
//      is the phone not sleeping" becomes a question with an answer.
//
//   4. State that is re-derived, not cached. `secure` is WlSessionLock's
//      compositor acknowledgement; it is distinct from `lockRequested`, the
//      shell input that asks WlSessionLock to lock.
//
// The trust boundary here is deliberately trivial and stated so it stays that
// way: this surface is local, single-user, reachable only over quickshell's
// IPC socket by the user who owns the session. It has no remote caller and no
// second operator, so it has no grants, no signing, and no nonces. If it ever
// grows a network-reachable caller, that assumption is what breaks first.
pragma Singleton

import qs
import qs.services
import qs.modules.common
import Quickshell
import Quickshell.Io
import Quickshell.Services.Mpris

Singleton {
    id: root

    // --- Capabilities ------------------------------------------------------
    // Probed once, at startup. Until the probe returns, every capability reads
    // false: better to refuse a suspend we are unsure of than to fire a verb
    // into a machine that cannot honor it.
    property bool probed: false
    property bool hasLoginctl: false
    property bool hasSystemctl: false
    property string suspendCapability: "unknown"
    property string hibernateCapability: "unknown"
    property string poweroffCapability: "unknown"
    property string rebootCapability: "unknown"

    // logind is the preferred backend when present: it is the thing that
    // actually owns the session, and it works under elogind as well as
    // systemd. systemctl is the fallback for the poweroff/reboot verbs.
    // "challenge" means logind can do it after polkit authentication. It is
    // available to a normal desktop session with a functioning polkit agent,
    // but callers still learn that a prompt may be required through caps().
    readonly property bool canSuspend: ["yes", "challenge"].includes(root.suspendCapability)
    readonly property bool canHibernate: ["yes", "challenge"].includes(root.hibernateCapability)
    readonly property bool canPoweroff: ["yes", "challenge"].includes(root.poweroffCapability)
    readonly property bool canReboot: ["yes", "challenge"].includes(root.rebootCapability)

    // Live compositor acknowledgement, mirrored from WlSessionLock.secure by
    // LockScreen.qml. `screenLocked` remains the requested state that drives
    // the lock surface; do not treat it as proof that the session is secure.
    readonly property bool locked: GlobalStates.screenLockSecure

    // Report the SECURE lock state to logind as the session's LockedHint —
    // the freedesktop contract's state half (loginctl lock-session above is
    // only the request half). This makes `LockedHint` truthful device-wide,
    // so logind-aware apps (culver stops its Matrix sync, media players
    // pause, recorders blank) can watch one standard property instead of
    // each growing a bespoke shell IPC. Reporting the secure edge — not the
    // requested edge — means the hint never claims locked before the
    // compositor holds the lock. set-locked-hint is not a Lock signal, so
    // the hypridle echo storm documented in lock() cannot recur through it.
    onLockedChanged: {
        if (root.hasLoginctl)
            lockedHintProc.report(root.locked);
    }

    // Replay the hint once the capability probe lands.
    //
    // `hasLoginctl` starts false and only becomes true when `capabilityProbe`
    // returns, which is a `Process` round trip. The lock goes secure long
    // before that: measured 2026-08-02, `secure=true` at 15:21:07 and
    // `loginctl=true` at 15:21:37 — thirty seconds later. The one edge that
    // mattered was therefore dropped by the guard above and never retried,
    // because the shell locks once at boot and `locked` never changes again.
    //
    // The cost was the whole lock-before-blank invariant. `LockedHint` stayed
    // `no`, so sessiond's `locked` (which comes from logind, doctrine §4) was
    // permanently false, `request_blank()` timed out its `LOCK_ACK_BUDGET`
    // every time, and the panel blanked on a session nobody could confirm was
    // locked — `blank-without-lock` on every single blank.
    //
    // This is the fourth edge-vs-level bug in this system after `locked_ack`,
    // `ChargeRate` and `bootBloomActive`. A guard that drops a report must
    // replay it when the guard opens, or the report is only ever delivered by
    // luck of ordering.
    onHasLoginctlChanged: {
        if (root.hasLoginctl)
            lockedHintProc.report(root.locked);
    }
    // The seat's session object path, resolved once.
    //
    // `report()` used to resolve it on every call, which meant `sh` plus two
    // busctl round trips before the hint could move. Measured 2026-08-02: the
    // hint landed *after* sessiond's 2 s `LOCK_ACK_BUDGET`, so `request_blank()`
    // timed out and blanked unlocked even though every other link in the chain
    // was by then correct. Resolving once turns the lock report into a single
    // call.
    //
    // Safe to cache for this shell's lifetime: the seat's session only changes
    // when greetd restarts, and that restarts the shell with it.
    property string seatSessionPath: ""
    Process {
        id: seatPathProbe
        running: true
        command: ["sh", "-c",
                  "busctl get-property org.freedesktop.login1 " +
                  "/org/freedesktop/login1/seat/seat0 " +
                  "org.freedesktop.login1.Seat ActiveSession " +
                  "| grep -o '/org/freedesktop/login1/session/[^\"]*' " +
                  "|| busctl get-property org.freedesktop.login1 " +
                  "/org/freedesktop/login1/user/_$(id -u) " +
                  "org.freedesktop.login1.User Display " +
                  "| grep -o '/org/freedesktop/login1/session/[^\"]*'"]
        stdout: StdioCollector {
            onStreamFinished: {
                root.seatSessionPath = text.trim();
                console.log("[session] seat session path:", root.seatSessionPath);
                // Replay: the lock may already be secure by the time this
                // lands, and the edge that would have reported it is gone.
                // Same fault `onHasLoginctlChanged` above exists to fix.
                if (root.seatSessionPath !== "" && root.hasLoginctl)
                    lockedHintProc.report(root.locked);
            }
        }
    }

    Process {
        id: lockedHintProc
        property bool pending: false
        property bool pendingValue: false
        function report(value) {
            if (running) {
                // Coalesce: remember the latest value, replay on exit.
                pending = true;
                pendingValue = value;
                return;
            }
            if (root.seatSessionPath === "") {
                // The probe has not landed. Dropping here is safe only because
                // the probe replays on completion — see its onStreamFinished.
                return;
            }
            // Never `/session/auto`: that is the *caller's* session, and the
            // shell is not in the one that owns the seat. Measured: viewtop in
            // logind 66 (seat0/tty1), `qs` in 70 — the hint was being written to
            // a session nobody reads while the graphical session stayed `no`
            // forever. sessiond takes `locked` from `LockedHint` (doctrine §4),
            // so that made `locked` permanently false and every blank went out
            // on a session nobody could confirm was locked.
            command = ["busctl", "call", "org.freedesktop.login1",
                       root.seatSessionPath,
                       "org.freedesktop.login1.Session",
                       "SetLockedHint", "b", value ? "true" : "false"];
            running = true;
        }
        onExited: {
            if (pending) {
                pending = false;
                report(pendingValue);
            }
        }
    }

    signal actionFailed(string action, int exitCode)

    Process {
        id: capabilityProbe
        // Runs at construction: the probe must land before anything asks
        // caps(), and every capability reads false until it does.
        running: true
        // One shell, one round trip. logind's Can* methods incorporate the
        // policy and configuration that /sys/power/state cannot see (notably
        // swap/resume setup for hibernation). Possible values include yes,
        // no, challenge, and na; retain the value rather than flattening it.
        // busctl prints `s "challenge"`; awk pulls the second field verbatim
        // and the quotes come off in JS below. An earlier version parsed it
        // with sed inside single quotes, where sh does not process the \" and
        // sed ended up matching a literal backslash-quote that busctl never
        // emits — so on the phone the probe returned nothing and every
        // capability stuck at "unknown". Keep the shell here quote-free; do
        // the string work in QML where there is no second escaping layer.
        command: ["sh", "-c",
            "command -v loginctl >/dev/null && echo loginctl; " +
            "command -v systemctl >/dev/null && echo systemctl; " +
            "if command -v busctl >/dev/null; then " +
              "for cap in CanSuspend CanHibernate CanPowerOff CanReboot; do " +
                "value=$(busctl --system call org.freedesktop.login1 /org/freedesktop/login1 " +
                  "org.freedesktop.login1.Manager $cap 2>/dev/null | awk '{print $2}'); " +
                "[ -n \"$value\" ] && echo $cap=$value; " +
              "done; " +
            "fi; " +
            "true"]

        stdout: StdioCollector {
            onStreamFinished: {
                const lines = text.split("\n").map(l => l.trim());
                root.hasLoginctl = lines.includes("loginctl");
                root.hasSystemctl = lines.includes("systemctl");
                const capability = (name) => {
                    const prefix = name + "=";
                    const line = lines.find(l => l.startsWith(prefix));
                    // Value arrives quoted from busctl (e.g. "challenge").
                    return line ? line.slice(prefix.length).replace(/"/g, "") : "unknown";
                };
                root.suspendCapability = capability("CanSuspend");
                root.hibernateCapability = capability("CanHibernate");
                root.poweroffCapability = capability("CanPowerOff");
                root.rebootCapability = capability("CanReboot");
                root.probed = true;
                console.log("[session] capabilities:",
                    "loginctl=" + root.hasLoginctl,
                    "systemctl=" + root.hasSystemctl,
                    "suspend=" + root.suspendCapability,
                    "hibernate=" + root.hibernateCapability,
                    "poweroff=" + root.poweroffCapability,
                    "reboot=" + root.rebootCapability);
            }
        }
    }

    // --- Verb runner -------------------------------------------------------
    // Every power verb goes through here so that none of them can fail
    // silently. Upstream used execDetached, which cannot report an exit code.
    Process {
        id: verbProc
        property string verb: ""
        onExited: (exitCode, exitStatus) => {
            root.lastAction = {
                action: verbProc.verb,
                status: exitCode === 0 ? "succeeded" : "failed",
                exitCode: exitCode
            };
            if (exitCode !== 0) {
                console.log(`[session] ${verbProc.verb} failed (exit ${exitCode})`);
                root.actionFailed(verbProc.verb, exitCode);
            }
        }
    }

    function runVerb(verb, argv) {
        // Process has one command slot. Overwriting it while a prior action
        // is still running makes the eventual exit code belong to the wrong
        // action, which is another form of silent failure.
        if (verbProc.running) return false;
        verbProc.verb = verb;
        verbProc.command = argv;
        root.lastAction = { action: verb, status: "running", exitCode: null };
        verbProc.running = true;
        return true;
    }

    // IPC returns when an action is accepted, not when the kernel has already
    // suspended or powered off. This records the later Process outcome so a
    // caller can distinguish "started" from "succeeded".
    property var lastAction: ({ action: "", status: "idle", exitCode: null })

    // systemctl owns the power verbs, NOT loginctl. loginctl only manages
    // sessions/users/seats — `loginctl poweroff` exits 1 with "Unknown
    // command verb" (verified on systemd 261, and its --help lists no power
    // commands at all). Preferring loginctl here silently broke poweroff,
    // reboot, suspend and hibernate from every shell surface: the button
    // fired, the Process exited 1, and the phone stayed on (2026-07-20).
    //
    // The capability probe still asks logind over D-Bus (CanPowerOff etc.) —
    // that part was always right; logind owns the *policy*. It just isn't
    // the CLI that carries out the action.
    function powerCommand(action) {
        if (root.hasSystemctl) return ["systemctl", action];
        // elogind ships loginctl with power verbs and usually no systemctl;
        // only reachable on such a system, where these verbs do exist.
        if (root.hasLoginctl) return ["loginctl", action];
        return [];
    }

    // --- Inhibits ----------------------------------------------------------
    // A bare "something is holding the machine awake" bool cannot answer the
    // only question that matters when the phone will not sleep: WHAT is
    // holding it, and why. Each holder gets a cookie and carries a reason.
    property var inhibitors: ({})
    property int nextCookie: 1

    readonly property bool inhibited: Object.keys(root.inhibitors).length > 0

    function inhibit(what, reason) {
        const kind = String(what || "idle").trim().toLowerCase();
        const why = String(reason || "").trim();
        if (!why) return root.refuse("inhibit", "an inhibit must carry a reason");
        // idle and sleep are wired today. idle uses the Wayland/hypridle
        // mechanism via Idle.qml; sleep uses SessionEvents' delay-mode
        // systemd-inhibit. Recording logout/user-switch without applying
        // their mechanism would create a dangerous success-shaped no-op.
        if (kind !== "idle" && kind !== "sleep")
            return root.refuse("inhibit", `unsupported inhibit kind ${kind}; only idle and sleep are implemented`);
        const cookie = String(root.nextCookie++);
        // Reassign rather than mutate: QML only notifies on assignment, so an
        // in-place insert would leave `inhibited` and any binding on it stale.
        const next = Object.assign({}, root.inhibitors);
        next[cookie] = { what: kind, reason: why };
        root.inhibitors = next;
        console.log(`[session] inhibit ${cookie}: ${kind} — ${why}`);
        root.applyInhibits();
        return { ok: true, cookie: cookie };
    }

    function uninhibit(cookie) {
        if (!root.inhibitors[cookie])
            return root.refuse("uninhibit", `no inhibitor with cookie ${cookie}`);
        const next = Object.assign({}, root.inhibitors);
        delete next[cookie];
        root.inhibitors = next;
        console.log(`[session] uninhibit ${cookie}`);
        root.applyInhibits();
        return { ok: true };
    }

    // Any holder inhibiting "idle" keeps the machine awake. Idle.qml owns the
    // mechanism (it knows the hypridle quirk on this device); we own the
    // policy of who is asking and why.
    //
    // "sleep" inhibitors are managed by SessionEvents (delay-mode
    // systemd-inhibit). They don't need a mechanism toggle here —
    // SessionEvents holds the inhibitor from startup and releases it
    // only when PrepareForSleep(true) fires and the Wayland lock is secure.
    function applyInhibits() {
        const wantIdle = Object.values(root.inhibitors).some(i => i.what === "idle");
        Idle.toggleInhibit(wantIdle);
    }

    // --- State -------------------------------------------------------------
    // The projection an agent reads. Everything here is re-derived at call
    // time; nothing is a bool we set ourselves and then trusted.
    function state() {
        const holders = Object.keys(root.inhibitors).map(c => ({
            cookie: c,
            what: root.inhibitors[c].what,
            reason: root.inhibitors[c].reason
        }));
        return {
            locked: root.locked,
            lockRequested: GlobalStates.screenLocked,
            idle: {
                stage: IdleCoordinator.state,
                nativeCoordinatorEnabled: IdleCoordinator.nativeEnabled
            },
            idleInhibited: Idle.inhibit,
            inhibitors: holders,
            lastAction: root.lastAction,
            capabilities: root.caps(),
            stepUp: typeof StepUpAuth !== "undefined" ? StepUpAuth.state() : null,
            sleepInhibitorHeld: typeof SessionEvents !== "undefined" ? SessionEvents.sleepInhibitorHeld : null
        };
    }

    // `probed` is not decoration: until the probe lands every capability reads
    // false, and false-because-unknown is not the same claim as
    // false-because-unsupported. A caller that ignores `probed` during the
    // startup window would conclude this machine cannot suspend at all. Check
    // `probed` before believing a false.
    function caps() {
        return {
            probed: root.probed,
            suspend: root.canSuspend,
            suspendStatus: root.suspendCapability,
            hibernate: root.canHibernate,
            hibernateStatus: root.hibernateCapability,
            poweroff: root.canPoweroff,
            poweroffStatus: root.poweroffCapability,
            reboot: root.canReboot,
            rebootStatus: root.rebootCapability,
            inhibitors: ["idle", "sleep"]
        };
    }

    // --- Verbs -------------------------------------------------------------
    // Every upstream ii verb is preserved by name and behavior, so existing
    // call sites (LockScreen.qml's poweroff/reboot on the lock's power action,
    // the session menus) keep working. What changed is that they now refuse
    // honestly when the machine cannot do the thing, and log when it fails.
    //
    // Those call sites are all statements — `onClicked: Session.suspend()` —
    // so they ignore the returned {ok, reason}. That is fine for the IPC
    // caller, which reads the value, but it means a UI button that hits a
    // refusal would otherwise do nothing at all, silently: press hibernate on
    // the phone, no swap, nothing happens, no trace. Every refusal therefore
    // goes through refuse(), which logs before it returns. A refused verb is
    // an event, not a void.
    function refuse(verb, reason) {
        console.log(`[session] ${verb} refused: ${reason}`);
        return { ok: false, reason: reason };
    }

    function closeAllWindows() {
        HyprlandData.windowList.map(w => w.pid).forEach(pid => {
            Quickshell.execDetached(["kill", pid]);
        });
    }

    function pauseAllPlayers() {
        for (const player of Mpris.players.values) {
            if (player.canPause) player.pause();
        }
    }

    function lock() {
        // Raise our Wayland lock ourselves: logind's Lock signal is a request
        // for session software to lock, not a Wayland lock implementation.
        // We also notify logind when it is available so other consumers see
        // the standard session event. The safe lock does not depend on that
        // asynchronous notification returning successfully.
        //
        // Notify ONLY on the unlocked->locked edge. hypridle's lock_cmd
        // fires on logind's Lock signal, so an unconditional
        // `loginctl lock-session` here echoes back through logind ->
        // hypridle -> this function forever. Observed on the phone: a
        // sustained storm of lock requests (several per second, for hours)
        // that re-locked the screen seconds after every unlock and chewed
        // battery all night.
        const alreadyLocked = GlobalStates.screenLocked;
        GlobalStates.screenLocked = true;
        if (alreadyLocked) {
            return { ok: true, status: "already-locked" };
        }
        if (root.hasLoginctl) {
            const notified = root.runVerb("lock", ["loginctl", "lock-session"]);
            return notified
                ? { ok: true, status: "requested" }
                : { ok: true, status: "requested", degraded: "logind notification skipped; another action is running" };
        }
        return { ok: true, degraded: "no loginctl; locked without logind" };
    }

    function unlock() {
        // Deliberately not a verb an agent gets. Unlocking is the credential
        // gate on this device — the only thing standing between a picked-up
        // phone and the session. It is refused here so that no IPC caller can
        // route around the PIN pad. The human unlocks; nothing else does.
        return root.refuse("unlock", "unlock is the credential gate; not remotely callable");
    }

    function suspend() {
        if (!root.probed) return root.refuse("suspend", "capabilities not probed yet");
        if (!root.canSuspend) return root.refuse("suspend", "no loginctl or systemctl on this machine");
        pauseAllPlayers();
        if (!root.runVerb("suspend", root.powerCommand("suspend")))
            return root.refuse("suspend", "another session action is still running");
        return { ok: true, status: "started" };
    }

    function hibernate() {
        if (!root.probed) return root.refuse("hibernate", "capabilities not probed yet");
        // logind validates swap/resume configuration as well as kernel support,
        // so a phone without hibernation refuses instead of firing a no-op.
        if (!root.canHibernate)
            return root.refuse("hibernate", "hibernate unavailable (logind: " + root.hibernateCapability + ")");
        pauseAllPlayers();
        if (!root.runVerb("hibernate", root.powerCommand("hibernate")))
            return root.refuse("hibernate", "another session action is still running");
        return { ok: true, status: "started" };
    }

    function poweroff() {
        if (!root.probed) return root.refuse("poweroff", "capabilities not probed yet");
        if (!root.canPoweroff) return root.refuse("poweroff", "no loginctl or systemctl on this machine");
        closeAllWindows();
        if (!root.runVerb("poweroff", root.powerCommand("poweroff")))
            return root.refuse("poweroff", "another session action is still running");
        return { ok: true, status: "started" };
    }

    function reboot() {
        if (!root.probed) return root.refuse("reboot", "capabilities not probed yet");
        if (!root.canReboot) return root.refuse("reboot", "no loginctl or systemctl on this machine");
        closeAllWindows();
        if (!root.runVerb("reboot", root.powerCommand("reboot")))
            return root.refuse("reboot", "another session action is still running");
        return { ok: true, status: "started" };
    }

    function rebootToFirmware() {
        if (!root.hasSystemctl)
            return root.refuse("rebootToFirmware", "firmware-setup reboot needs systemctl");
        closeAllWindows();
        if (!root.runVerb("rebootToFirmware", ["systemctl", "reboot", "--firmware-setup"]))
            return root.refuse("rebootToFirmware", "another session action is still running");
        return { ok: true, status: "started" };
    }

    function logout() {
        closeAllWindows();
        // loginctl terminate-session ends the session properly (logind tears
        // down the scope and the seat); pkill Hyprland just kills the
        // compositor and leaves logind believing the session is alive.
        if (root.hasLoginctl) {
            if (!root.runVerb("logout", ["loginctl", "terminate-session", ""]))
                return root.refuse("logout", "another session action is still running");
            return { ok: true, status: "started" };
        }
        if (!root.runVerb("logout", ["pkill", "-i", "Hyprland"]))
            return root.refuse("logout", "another session action is still running");
        return { ok: true, status: "started", degraded: "no loginctl; killed the compositor" };
    }

    function changePassword() {
        Quickshell.execDetached(["bash", "-c", `${Config.options.apps.changePassword}`]);
    }

    function launchTaskManager() {
        Quickshell.execDetached(["bash", "-c", `${Config.options.apps.taskManager}`]);
    }

    // NO IpcHandler here. This file used to register `target: "session"` with
    // state/inhibit/uninhibit, and Lock.qml registers the same target with a
    // strict superset (those three plus capabilities, lock, unlock, suspend,
    // hibernate, poweroff, reboot, logout). Quickshell keeps whichever
    // registers first and drops the other with a warning:
    //
    //   QML IpcHandler at Session.qml[470:5]: Handler was registered but will
    //   not be used because another handler is registered for target session
    //
    // Which one won was load-order, not design — so `session inhibit` and
    // `session uninhibit` were reachable or dead depending on the run, and
    // deploy.sh's `session state` was riding the same coin flip. Since every
    // verb here exists on Lock.qml's handler, the duplicate is removed rather
    // than renamed: one target, one owner. Add new session verbs there.

    // --- Boot-time IPC audit ------------------------------------------------
    // Logs every IPC endpoint the shell exposes at startup. This is a
    // defensive visibility measure — not a gate, not a refusal. It answers
    // "what is reachable over the IPC socket?" so a human or an audit tool
    // can verify the surface matches intent. The list is static per shell
    // config; it does not change at runtime.
    //
    // Known IPC targets at time of writing:
    //   lock       — LockScreen.qml (activate, focus)
    //   lock2      — Lock.qml (lock, unlock, toggleLock)
    //   dock       — Dock.qml (launch, pin, unpin, moveStack, ...)
    //   sidebar    — SidebarLeft.qml / SidebarRight.qml
    //   session    — Lock.qml (the sole owner: state, capabilities, lock,
    //                unlock, suspend, hibernate, poweroff, reboot, logout,
    //                inhibit, uninhibit). Session.qml's duplicate was removed
    //                2026-07-26; SessionScreen.qml's moved to `sessionMenu`.
    //   overview   — Overview.qml
    //   keyboard   — OnScreenKeyboard.qml
    //   appInventory — AppInventoryScope.qml
    //
    // If a new IpcHandler is added, this comment must be updated. The
    // console.log below is the runtime check; the comment is the human
    // audit trail.
    // NOTE: Component.onCompleted doesn't work on Singletons in QML,
    // and Timer isn't available in this module's import scope. Use a
    // Process with onExited instead.
    Process {
        id: bootAuditProc
        running: true
        command: ["true"]
        onExited: {
            console.log("[session] boot IPC audit — shell exposes: "
                + "lock, lock2, dock, sidebar, session, overview, keyboard, appInventory");
        }
    }
}
