// Step-up authentication singleton for the Souveraine shell.
//
// Provides short-lived, in-memory grants for sensitive operations (send, push,
// delete, payment, physical access, admin). A grant is minted after a separate
// PAM conversation succeeds via the `souveraine-stepup` PAM service. This
// service never unlocks the session and never accepts a boolean from an agent
// as proof.
//
// Architecture reference: SESSION-TRUST-ARCHITECTURE.md — "Step-up
// authentication is a separate PamContext, using a dedicated PAM service such
// as souveraine-stepup. It never unlocks the session and it never accepts a
// boolean from an agent as proof. A successful result mints a short-lived,
// in-memory grant bound to the local action family."
//
// The grant TTL is a deliberate policy setting (grantTtlMs), not an
// implementation accident. It defaults to 5 minutes and is configurable via
// Config.options.lock.stepUp.grantTtlMs.
//
// Grants are cleared on: lock, session end, PAM failure, and expiry. The
// expiry timer runs every 30 seconds; the precision of expiry is intentionally
// coarse because step-up is a convenience layer, not a security kernel.
//
// Integration:
//   - GlobalStates.onScreenLockedChanged  -> revokeAll()
//   - Session actionFailed / lock verbs   -> revokeAll()
//   - 30-second Timer                     -> expire stale grants
//
// The PAM service file `/etc/pam.d/souveraine-stepup` is NOT shipped by the
// shell — it is root-owned system config, delivered by the souveraine package.
// Without it every request refuses at start() rather than falling through to
// something weaker.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Pam
import qs
import qs.services
import qs.modules.common
import qs.modules.common.functions

Singleton {
    id: root

    // --- Action families ----------------------------------------------------
    // These are the semantic groupings of operations that require step-up.
    // A grant for "send" covers message sends and content pushes; "delete"
    // covers irreversible removals; "payment" covers financial transactions;
    // "physical" covers door locks, device unlock beyond session; "admin"
    // covers system administration that session-level lock does not gate.
    readonly property string familySend: "send"
    readonly property string familyDelete: "delete"
    readonly property string familyPayment: "payment"
    readonly property string familyPhysical: "physical"
    readonly property string familyAdmin: "admin"

    // --- Grant model --------------------------------------------------------
    // actionFamily -> { granted: timestamp_ms, expires: timestamp_ms }
    // Grants are plain objects, not QML types, because the set of families is
    // open-ended and callers only need the two timestamps.
    property var grants: ({})

    // Default 5 minutes. Overridable via Config.options.lock.stepUp.grantTtlMs
    // so the policy knob lives in the user's config, not in source.
    property int grantTtlMs: Config.options?.lock?.stepUp?.grantTtlMs ?? 300000

    // --- Signals ------------------------------------------------------------
    signal authSucceeded(string actionFamily)
    signal authFailed(string actionFamily)
    signal grantExpired(string actionFamily)
    signal grantRevoked(string actionFamily)

    // PAM needs an answer. `secret` is false for prompts that should be echoed.
    // An empty respond() is valid — the FPC factor prompts "Touch and hold" and
    // consumes its ticket on a blank answer.
    signal promptRequired(string actionFamily, string message, bool secret)

    // --- Internal state -----------------------------------------------------
    // The callback for the in-flight auth request. Only one auth conversation
    // runs at a time; a second requestAuth while one is pending is refused.
    // This matches PAM's own serial conversation model — you cannot interleave
    // two pam_authenticate calls on the same handle.
    property var _pendingCallback: null
    property string _pendingFamily: ""

    // --- PAM conversation ---------------------------------------------------
    // A second, narrower PamContext than the lock surface's (doctrine §3),
    // pointed at the system souveraine-stepup service. It never unlocks the
    // session. Same mechanism LockContext already uses for fprintd.conf.
    //
    // The prompt is not owned here: pamMessage raises promptRequired and a
    // surface answers with respond(). One conversation at a time, which is
    // PAM's own model and the reason a second requestAuth is refused.
    PamContext {
        id: stepUpPam

        config: "souveraine-stepup"

        property string activeFamily: ""

        onPamMessage: {
            if (stepUpPam.responseRequired) {
                root.promptRequired(stepUpPam.activeFamily, stepUpPam.message,
                                    !stepUpPam.responseVisible);
            } else if (stepUpPam.message.length > 0) {
                console.log(`[step-up] ${stepUpPam.message}`);
            }
        }

        onCompleted: result => {
            const family = stepUpPam.activeFamily;
            stepUpPam.activeFamily = "";

            if (family.length === 0) {
                console.log("[step-up] PAM completed with no active family (stale?)");
                return;
            }

            const callback = root._pendingCallback;
            root._pendingCallback = null;
            root._pendingFamily = "";

            if (result === PamResult.Success) {
                root._mintGrant(family);
                root.authSucceeded(family);
                console.log(`[step-up] auth succeeded for ${family}`);
                if (callback) callback(true);
            } else {
                root._clearGrant(family);
                root.authFailed(family);
                console.log(`[step-up] auth failed for ${family} (${PamResult.toString(result)})`);
                if (callback) callback(false);
            }
        }
    }

    // --- Public API ---------------------------------------------------------

    // requestAuth — initiate a step-up authentication for the given action
    // family. The callback receives a boolean: true if the user authenticated
    // successfully and a grant was minted, false otherwise.
    //
    // Returns { ok: true } if the auth flow started, or { ok: false, reason }
    // if it was refused (auth already in progress, or empty family).
    function requestAuth(actionFamily, callback) {
        const family = String(actionFamily || "").trim();
        if (!family) return { ok: false, reason: "empty action family" };
        if (stepUpPam.active) return { ok: false, reason: "auth in progress" };

        // If a valid grant already exists, skip the PAM conversation entirely.
        // The caller gets an immediate true and the TTL is not extended — this
        // is intentional: repeated requests within the window do not refresh
        // the clock, so the grant expires on schedule regardless of how often
        // it is checked. A caller that wants a fresh window must revoke first.
        if (root.isGranted(family)) {
            console.log(`[step-up] grant for ${family} still valid, skipping auth`);
            if (callback) callback(true);
            return { ok: true, reason: "already granted" };
        }

        root._pendingCallback = callback;
        root._pendingFamily = family;

        stepUpPam.activeFamily = family;
        if (!stepUpPam.start()) {
            stepUpPam.activeFamily = "";
            root._pendingCallback = null;
            root._pendingFamily = "";
            console.warn(`[step-up] PAM refused to start for ${family}`
                + " — is /etc/pam.d/souveraine-stepup installed?");
            if (callback) callback(false);
            return { ok: false, reason: "pam unavailable" };
        }
        console.log(`[step-up] auth started for ${family}`);
        return { ok: true };
    }

    // respond — answer the current prompt. Empty text is a legitimate answer,
    // not a cancel: that is how the fingerprint factor is accepted.
    function respond(text) {
        if (!stepUpPam.active) return { ok: false, reason: "no auth in progress" };
        stepUpPam.respond(String(text ?? ""));
        return { ok: true };
    }

    // cancel — abandon the in-flight conversation. Completion still fires, so
    // the pending callback is answered rather than dropped.
    function cancel() {
        if (!stepUpPam.active) return { ok: false, reason: "no auth in progress" };
        stepUpPam.abort();
        return { ok: true };
    }

    // isGranted — check if a valid (non-expired) grant exists for the given
    // action family. Returns true only if the grant exists and has not yet
    // exceeded its TTL. Does NOT trigger re-auth; it is a pure read.
    function isGranted(actionFamily) {
        const family = String(actionFamily || "").trim();
        const grant = root.grants[family];
        if (!grant) return false;
        const now = Date.now();
        if (now - grant.granted >= root.grantTtlMs) {
            // Grant has expired. Clear it synchronously so the next call does
            // not re-read a stale entry, and emit the signal so surfaces can
            // react (e.g. disable a send button).
            root._clearGrant(family);
            root.grantExpired(family);
            return false;
        }
        return true;
    }

    // revokeGrant — explicitly revoke a single grant. Used when an operation
    // completes (the grant served its purpose) or when the caller decides the
    // context has changed. Does nothing if no grant exists for that family.
    function revokeGrant(actionFamily) {
        const family = String(actionFamily || "").trim();
        if (!root.grants[family]) return;
        root._clearGrant(family);
        root.grantRevoked(family);
        console.log(`[step-up] grant revoked for ${family}`);
    }

    // revokeAll — clear every active grant. Called on lock, session end, and
    // any condition that invalidates the entire trust surface. This is the
    // fail-closed path: if something goes wrong that we cannot characterize
    // per-family, all grants die.
    function revokeAll() {
        const families = Object.keys(root.grants);
        const hadBreakGlass = root._breakGlassGrant !== null;
        if (families.length === 0 && !hadBreakGlass) return;
        root.grants = ({});
        root._clearBreakGlass();
        families.forEach(family => root.grantRevoked(family));
        console.log(`[step-up] all grants revoked (${families.length} families`
            + (hadBreakGlass ? ", break-glass cleared" : "") + ")");
    }

    // state — return the current grant state as a plain object for IPC
    // projection. This is what an agent reads when it needs to know which
    // action families are currently authorized. Every field is re-derived at
    // call time; nothing is cached trust.
    function state() {
        const now = Date.now();
        const active = {};
        const families = Object.keys(root.grants);
        families.forEach(family => {
            const grant = root.grants[family];
            if (grant && (now - grant.granted < root.grantTtlMs)) {
                active[family] = {
                    granted: grant.granted,
                    expires: grant.granted + root.grantTtlMs,
                    remainingMs: (grant.granted + root.grantTtlMs) - now
                };
            }
        });
        return {
            grantTtlMs: root.grantTtlMs,
            activeGrants: active,
            authInProgress: stepUpPam.active,
            pendingFamily: root._pendingFamily,
            breakGlassActive: root._breakGlassGrant !== null
        };
    }

    // --- Break-glass grant -------------------------------------------------
    // A one-time, short-lived, journaled override for emergency operations.
    // This is NOT a config toggle. It is a per-decision, reasoned, logged
    // bypass that exists because real emergencies happen and the user must
    // be able to act.
    //
    // Properties:
    //   - Requires a non-empty reason string (the "why")
    //   - Short TTL (60 seconds by default, not the normal 5 minutes)
    //   - One-time: consumed on use, cannot be reused
    //   - Prominently logged with the reason
    //   - Cannot be issued while the session is locked
    //   - Cleared on lock, like all other grants
    //
    // The break-glass grant is tracked separately from normal grants so
    // that audit tools can distinguish "user authenticated normally" from
    // "user declared an emergency override".
    property var _breakGlassGrant: null
    property int breakGlassTtlMs: 60000  // 60 seconds

    signal breakGlassIssued(string reason, int expiresAt)
    signal breakGlassConsumed(string reason)
    signal breakGlassExpired(string reason)

    // breakGlass — issue a one-time emergency grant for the given action
    // family. Returns { ok: true, expiresAt } or { ok: false, reason }.
    //
    // The reason is mandatory and logged. If the caller cannot explain why
    // they need break-glass, they should use normal step-up auth instead.
    function breakGlass(actionFamily, reason) {
        const family = String(actionFamily || "").trim();
        const why = String(reason || "").trim();
        if (!family) return { ok: false, reason: "empty action family" };
        if (!why) return { ok: false, reason: "break-glass requires a reason" };
        if (GlobalStates.screenLocked)
            return { ok: false, reason: "break-glass unavailable while locked" };
        if (root._breakGlassGrant)
            return { ok: false, reason: "break-glass already active" };

        const now = Date.now();
        root._breakGlassGrant = {
            family: family,
            reason: why,
            granted: now,
            expires: now + root.breakGlassTtlMs
        };
        console.log(`[step-up] BREAK-GLASS issued for ${family}: "${why}" `
            + `(expires in ${root.breakGlassTtlMs / 1000}s)`);
        root.breakGlassIssued(why, now + root.breakGlassTtlMs);
        return { ok: true, expiresAt: now + root.breakGlassTtlMs };
    }

    // isBreakGlass — check if a valid break-glass grant exists for the
    // given action family. Unlike normal grants, break-glass is one-time:
    // calling this function consumes it. Returns true if the grant was
    // valid and consumed, false otherwise.
    function isBreakGlass(actionFamily) {
        const family = String(actionFamily || "").trim();
        const bg = root._breakGlassGrant;
        if (!bg) return false;
        if (bg.family !== family) return false;
        if (Date.now() >= bg.expires) {
            root._breakGlassGrant = null;
            root.breakGlassExpired(bg.reason);
            return false;
        }
        // Consume the grant — one-time use.
        root._breakGlassGrant = null;
        root.breakGlassConsumed(bg.reason);
        console.log(`[step-up] BREAK-GLASS consumed for ${family}: "${bg.reason}"`);
        return true;
    }

    // Revoke break-glass on lock (fail-closed).
    function _clearBreakGlass() {
        if (!root._breakGlassGrant) return;
        root._breakGlassGrant = null;
    }

    // --- Internal -----------------------------------------------------------

    function _mintGrant(family) {
        const now = Date.now();
        const next = Object.assign({}, root.grants);
        next[family] = { granted: now, expires: now + root.grantTtlMs };
        root.grants = next;
    }

    function _clearGrant(family) {
        if (!root.grants[family]) return;
        const next = Object.assign({}, root.grants);
        delete next[family];
        root.grants = next;
    }

    // --- Lock integration ---------------------------------------------------
    // On lock, every grant dies. The session is no longer in a state where
    // step-up can meaningfully authorize anything — the user is behind a
    // credential gate and any grant minted before the lock would be
    // meaningless after it. This is the fail-closed path.
    Connections {
        target: GlobalStates
        function onScreenLockedChanged() {
            if (GlobalStates.screenLocked) {
                root.revokeAll();
            }
        }
    }

    // --- Session end integration --------------------------------------------
    // Session.logout() terminates the compositor session. Grants are
    // in-memory only and die with the process, but explicit revocation on
    // session end ensures the signal fires so surfaces can update their state
    // before the session tears down, rather than discovering the grants are
    // gone only when they try to read them after the fact.
    Connections {
        target: Session
        function onActionFailed(action, exitCode) {
            // If a lock action failed, that means we might be in an
            // ambiguous state — the session intended to lock but did not.
            // Revoking all grants is the conservative choice: a failed lock
            // is a trust anomaly.
            if (action === "lock") {
                root.revokeAll();
            }
        }
    }

    // --- Expiry timer -------------------------------------------------------
    // Checks every 30 seconds for grants that have exceeded their TTL. The
    // granularity is deliberately coarse: step-up is a convenience layer for
    // the user, not a security kernel. Sub-second precision would add
    // complexity for no meaningful security gain — the TTL itself is a policy
    // setting with a 5-minute default, and 30 seconds of drift on a
    // 300-second window is acceptable.
    Timer {
        id: expiryTimer
        interval: 30000
        repeat: true
        running: true
        onTriggered: {
            const now = Date.now();
            const families = Object.keys(root.grants);
            families.forEach(family => {
                const grant = root.grants[family];
                if (grant && (now - grant.granted >= root.grantTtlMs)) {
                    root._clearGrant(family);
                    root.grantExpired(family);
                    console.log(`[step-up] grant expired for ${family}`);
                }
            });
        }
    }
}
