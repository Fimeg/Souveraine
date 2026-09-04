pragma Singleton
pragma ComponentBehavior: Bound

import qs.modules.common
import Quickshell
import Quickshell.Io
import QtQuick

/**
 * Speech — TTS egress for the shell (the Read Aloud seam, TASK-18).
 *
 * Souveraine owns the who→voice mapping. The TTS endpoint (tts_url) is a
 * substrate concern, read from VoiceConfig at /v1/config. The VOICE is a
 * per-agent concern: each agent's [_souveraine].voice_id rides on the public
 * agent list, and on every speak()/prefetch() this service resolves it — the
 * active agent's own voice wins, the system voice_id (/v1/config) is the
 * fallback for an agent who hasn't set one. The shell picks no voice of its
 * own; souveraine decides how each agent sounds, and they need not match.
 *
 * Config.options.speech.tts.enable is the user's kill switch (settings →
 * Speech); Config.options.speech.tts.endpoint, when set, overrides the
 * server-mapped tts_url.
 *
 * Prefetch: the latest assistant reply is synthesized into the single tmpfs
 * file in the background the moment it finishes streaming, so tapping Speak
 * is near-instant when the audio is already ready. Prefetch NEVER auto-plays
 * — the user always initiates playback. A prefetch that fails is not retried
 * (the manual Speak path retries on its own when tapped). Only the latest
 * reply is kept; a new/edited reply invalidates the cached file because
 * readiness is keyed on the exact text.
 */
Singleton {
    id: root

    readonly property bool enabled: Config.options?.speech?.tts?.enable ?? false

    // `speaking` is the OR that surfaces have always read — kept as-is so
    // nothing downstream changes meaning under them.
    property bool speaking: synthProc.running || playProc.running

    // But one boolean cannot distinguish "waiting on the synthesizer" from
    // "audio is coming out of the speaker", and those look nothing alike to a
    // person: synthesis of a short line measured ~11s against the service,
    // and during all of it the shell showed the same state it shows while
    // actually talking. That is the whole reason a press feels unacknowledged
    // and gets pressed again.
    //
    // Split, so a surface can show a spinner for one and a level for the
    // other, and so a stop button can say which thing it is about to stop.
    readonly property bool synthesizing: synthProc.running
    readonly property bool playing: playProc.running
    property string lastError: ""

    property string _pendingText: ""
    // Resolved endpoint + voice from the last /v1/config lookup, kept so a
    // transient synth failure can be retried without re-resolving.
    property string _synthUrl: ""
    // The substrate's system voice, from /v1/config. This is the fallback —
    // the voice of "no agent picked" or an agent who hasn't set her own.
    property string _systemVoice: ""
    // The voice actually used this turn: the active agent's own voice if she
    // has one ([_souveraine].voice_id via the agent list), else the system
    // voice. Recomputed at every speak()/prefetch() so switching agents
    // switches voice without a re-fetch — the agent list is already local.
    property string _synthVoice: ""
    // Retry cap: the VibeVoice server can intermittently return 500 (concurrency
    // on its shared model — now server-locked, but a collision still costs one
    // failed request) or time out on a slow synth. One retry recovers the common
    // case; a second failure surfaces a human message.
    property int _synthAttempts: 0
    readonly property int _synthMaxAttempts: 2
    readonly property string _outFile: (Quickshell.env("XDG_RUNTIME_DIR") || "/tmp") + "/souveraine-speech.mp3"

    // What the file on disk currently holds (the exact text it was synthesized
    // from), or "" if nothing valid is cached. Keyed on text so a new or edited
    // reply naturally misses → Speak synthesizes fresh. Set only on a successful
    // synth; cleared by stop()/speak()/prefetch() of different text.
    property string _readyText: ""

    // speak — synthesize and play `text` in the agent's voice. A new call
    // replaces any in-flight synthesis or playback. If `text` is already
    // cached on disk (from a prefetch), skip synthesis and play instantly.
    // Check the cache BEFORE stopping — a prefetch that just completed should
    // be honored, not killed and re-requested.
    function speak(text) {
        const t = String(text ?? "").trim();
        if (t.length === 0 || !root.enabled) return;
        root.lastError = "";
        // Prefetch hit: audio for exactly this text is already on disk — play
        // it straight away, no synth round-trip. Don't call stop() here;
        // if a prefetch is still running for a *different* message, stop()
        // would kill it — but our text matches, so we know the file is ours.
        if (root._readyText === t) {
            stop();
            root._pendingText = t;
            root._play();
            return;
        }
        // No cache hit — stop anything in-flight and synthesize fresh.
        stop();
        root._pendingText = t;
        root._synthAttempts = 0;
        root._readyText = "";
        // If the endpoint is already resolved, skip the /v1/config round-trip
        // and go straight to synthesis. Voice is recomputed locally — the
        // agent may have switched since the last turn — so it's never the gate.
        if (root._synthUrl.length > 0) {
            root._applyVoice();
            root._fireSynth(false /*interactive*/);
        } else {
            voiceLookup.running = true;
        }
    }

    // prefetch — synthesize `text` into the cache file in the background so a
    // later speak() of the same text plays instantly. Does NOT play. A prefetch
    // failure is silent and not retried; the eventual manual speak() will
    // synthesize (with its own retry) if the text still isn't ready.
    function prefetch(text) {
        const t = String(text ?? "").trim();
        if (t.length === 0 || !root.enabled) return;
        // Already have exactly this text ready — don't re-request it.
        if (root._readyText === t) return;
        // Don't let a prefetch clobber a speak() the user just initiated.
        if (root.speaking && root._pendingText !== t) return;
        root._pendingText = t;
        root._synthAttempts = 0;
        // Mark this as a background prefetch so voiceLookup chains into
        // _fireSynth(prefetch=true) instead of treating it as interactive.
        synthProc._prefetch = true;
        // If the endpoint is already resolved, go straight to synth; voice is
        // recomputed locally for the current agent. Otherwise resolve via
        // /v1/config first.
        if (root._synthUrl.length > 0) {
            root._applyVoice();
            root._fireSynth(true /*prefetch*/);
        } else {
            voiceLookup.running = true;
        }
    }

    function stop() {
        voiceLookup.running = false;
        synthProc.running = false;
        playProc.running = false;
        retryTimer.running = false;
    }

    // resynthesize — request fresh audio for text we may already have cached.
    //
    // speak() opens with a cache check keyed on the text itself, and
    // re-synthesis is by definition the *same text* — so calling speak() to
    // "try again" is guaranteed to hit the cache and replay the identical
    // broken audio. The one control that exists for "that came out wrong"
    // could not do the only thing it is for.
    //
    // Invalidating _readyText before delegating is the whole fix: it forces
    // speak() down the synthesis path rather than the playback path.
    function resynthesize(text) {
        const t = String(text ?? "").trim();
        if (t.length === 0 || !root.enabled) return;
        stop();
        root._readyText = "";
        root.speak(t);
    }

    // The active agent's own voice, if she has one. The shell picks no voice
    // of its own: souveraine owns the who→voice mapping, and that mapping is
    // per-agent now — [_souveraine].voice_id rides on the public agent list.
    // "" means "this agent sounds like the substrate" → system-voice fallback.
    function _agentVoice() {
        const a = Souveraine.agents[Souveraine.currentAgentId];
        return (a && a.voice_id && a.voice_id.length > 0) ? a.voice_id : "";
    }

    // Recompute the turn's voice from the active agent + cached system voice.
    // Called on every speak()/prefetch() (agent may have switched since last
    // turn) and after a /v1/config fetch. No fetch here — both inputs are local.
    function _applyVoice() {
        root._synthVoice = root._agentVoice() || root._systemVoice;
    }

    // Play whatever is in the cache file.
    function _play() {
        playProc.running = true;
    }

    // Fire (or re-fire) the synth request with the already-resolved url+voice.
    // `prefetch` marks the result as a background fill (no playback, no retry,
    // populate _readyText on success) vs. an interactive speak().
    function _fireSynth(prefetch) {
        synthProc._prefetch = prefetch;
        synthProc.command = [
            "curl", "-sf", "--max-time", "180",
            "-X", "POST", "-H", "Content-Type: application/json",
            "-d", JSON.stringify({
                input: root._pendingText,
                voice: root._synthVoice,
                model: "vibevoice-v1"
            }),
            "-o", root._outFile,
            `${root._synthUrl.replace(/\/$/, "")}/audio/speech`
        ];
        synthProc.running = true;
    }

    // Delay before a retry — short, just long enough for the server to clear.
    Timer {
        id: retryTimer
        interval: 750
        repeat: false
        onTriggered: root._fireSynth(false /*interactive speak retries only*/)
    }

    // ── who → voice, from souveraine ─────────────────────────────────────
    Process {
        id: voiceLookup
        command: ["curl", "-sf", "--max-time", "3", `${Souveraine.serverBase}/v1/config`]
        stdout: StdioCollector {
            onStreamFinished: {
                let ttsUrl = Config.options?.speech?.tts?.endpoint ?? "";
                let voice = "";
                try {
                    const cfg = JSON.parse(text);
                    voice = cfg?.voice?.voice_id ?? "";
                    if (ttsUrl.length === 0)
                        ttsUrl = cfg?.voice?.tts_url ?? "";
                } catch (e) {
                    console.log("[Speech] could not parse /v1/config:", e);
                }
                if (ttsUrl.length === 0) {
                    root.lastError = "no TTS endpoint (server unmapped, shell override empty)";
                    console.log("[Speech]", root.lastError);
                    return;
                }
                root._synthUrl = ttsUrl;
                // `voice` here is the SYSTEM voice — the substrate speaking as
                // itself. Stash it as the fallback, then let the active agent
                // override it: she sounds like herself when she has a voice.
                root._systemVoice = voice;
                root._applyVoice();
                // If speak() was the caller, synth+play; if prefetch() was, the
                // flag is already on synthProc from _fireSynth — but voiceLookup
                // can be the first hop of a prefetch, so default to interactive.
                root._fireSynth(synthProc._prefetch ?? false);
            }
        }
        onExited: (exitCode) => {
            // Exit 15 = SIGTERM from stop(); not a real failure — suppress.
            if (exitCode !== 0 && exitCode !== 15) {
                root.lastError = "souveraine server unreachable for voice mapping";
                console.log("[Speech]", root.lastError);
            }
        }
    }

    // ── synthesis ────────────────────────────────────────────────────────
    Process {
        id: synthProc
        // True when this synth is a background prefetch (no playback, no retry).
        property bool _prefetch: false
        onExited: (exitCode) => {
            if (exitCode === 0) {
                // Success: the file now holds _pendingText.
                root._readyText = root._pendingText;
                if (synthProc._prefetch) {
                    // Prefetch only fills the cache; never auto-plays.
                    synthProc._prefetch = false;
                    return;
                }
                root._play();
                return;
            }
            // Prefetch failures are silent and not retried — per design, we
            // don't burn a second request on background fill. The manual
            // speak() path will synthesize (and retry) when actually tapped.
            if (synthProc._prefetch) {
                synthProc._prefetch = false;
                console.log("[Speech] prefetch failed (curl exit " + exitCode + ") — will synth on demand when spoken");
                return;
            }
            // Exit 15 = SIGTERM — stop() killed this synth (user tapped stop,
            // or a new speak() replaced it). This is always intentional, never
            // an error. Suppress silently.
            if (exitCode === 15) {
                return;
            }
            // Transient failures: exit 22 = HTTP ≥400 (the server's intermittent
            // 500 under model concurrency), exit 28 = timeout (slow synth).
            // Retry once; a collision or a slow first attempt usually clears.
            const transient = (exitCode === 22 || exitCode === 28);
            root._synthAttempts += 1;
            if (transient && root._synthAttempts < root._synthMaxAttempts) {
                console.log(`[Speech] synth exit ${exitCode}, retry ${root._synthAttempts}/${root._synthMaxAttempts - 1}`);
                retryTimer.running = true;
                return;
            }
            if (exitCode === 28) {
                root.lastError = root._synthAttempts > 1
                    ? "voice synthesis timed out twice — the server may be busy or the reply too long"
                    : "voice synthesis timed out";
            } else if (exitCode === 22) {
                root.lastError = root._synthAttempts > 1
                    ? "voice synthesis failed twice — the TTS server returned an error (may be busy)"
                    : "voice synthesis failed (server error)";
            } else {
                root.lastError = `voice synthesis failed (curl exit ${exitCode})`;
            }
            console.log("[Speech]", root.lastError);
        }
    }

    // ── playback ─────────────────────────────────────────────────────────
    //
    // No `sh -c` wrapper, deliberately. The previous form was
    //
    //     ["sh", "-c", "mpv ... || ffplay ..."]
    //
    // and a compound command means sh does NOT exec-replace itself: it forks
    // the player as a child and waits. So `playProc.running = false` sends
    // SIGTERM to *sh*, sh dies, and the player keeps making noise as an
    // orphan. Reproduced directly: killing the wrapper left the child alive.
    //
    // That is why stop() never stopped anything, and why two speak() calls in
    // a row played over each other instead of replacing one another.
    //
    // One player, invoked directly, so the pid quickshell holds is the pid
    // making sound. The ffplay fallback is dropped rather than fixed: its
    // invocation was already wrong (raw input needs -i) and a fallback is
    // exactly what forced the shell wrapper that broke the kill. mpv is
    // present on both the laptop and the phone; if it is ever missing, the
    // honest outcome is a named error, not silent audio nobody can stop.
    //
    // --keep-open=no --idle=no is not cosmetic. mpv can reach the end of a
    // stream, print (Paused), and never exit — which, since `speaking` is
    // derived from playProc.running, renders as speaking forever.
    Process {
        id: playProc
        command: ["mpv", "--no-video", "--really-quiet",
                  "--keep-open=no", "--idle=no", root._outFile]
        onExited: (exitCode) => {
            // Exit 15 = SIGTERM from stop(); not a real failure — suppress.
            // This now actually reaches mpv rather than a shell wrapper.
            if (exitCode !== 0 && exitCode !== 15) {
                root.lastError = `audio playback failed (exit ${exitCode}) — is mpv installed?`;
                console.log("[Speech]", root.lastError);
            }
        }
    }
}
