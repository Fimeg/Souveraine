#!/usr/bin/env bash
# One spine for agent session state — TASK-69 / TASK-70.
#
# Emits a SINGLE JSON envelope describing every agent session on this machine,
# across providers: Souveraine, Claude Code, Codex. The shell reads this and
# renders projections of it; no surface fetches per-provider any more.
#
# WHY A SCRIPT AND NOT THE DAEMON YET
# TASK-69 calls for a Rust collector on a unix socket. This is the same
# envelope, produced by polling, so the surface can be built and proven against
# the real shape now. When the daemon lands, AgentSessions.qml swaps its source
# from this process to the socket and NOTHING downstream changes. The envelope
# is the contract; the transport is an implementation detail. Keep it that way.
#
# CONTRACT
#   stdout: exactly one line of JSON, always. Never empty, never partial.
#   exit:   always 0. A provider that fails reports available:false and the
#           reason; it never takes the envelope down with it. A blank bar is a
#           worse failure than a stale one.
#
# Usage: agent-sessions.sh [--window-min N]   (default 1440 = 24h)

set -uo pipefail   # deliberately NOT -e: a failing provider must not abort

WINDOW_MIN=1440
[ "${1:-}" = "--window-min" ] && WINDOW_MIN="${2:-1440}"

NOW=$(date +%s)

# Emit a minimal valid envelope and leave, for the cases where we cannot even
# start (no jq). Downstream must never see malformed JSON.
if ! command -v jq >/dev/null 2>&1; then
    printf '{"ts":%s,"sessions":[],"providers":{"claude":{"available":false,"error":"jq missing"},"codex":{"available":false,"error":"jq missing"},"souveraine":{"available":false,"error":"jq missing"}}}\n' "$NOW"
    exit 0
fi

# ---------------------------------------------------------------- claude ----
# ~/.claude/projects/<slug>/<uuid>.jsonl — one file per session. Directory slug
# is the cwd with '/' -> '-'. Per-assistant `message.usage` carries
# input/output/cache_creation/cache_read. We read only files touched inside the
# window: a months-old session is not "a session", it is history.
claude_json() {
    local dir="$HOME/.claude/projects"
    [ -d "$dir" ] || { echo '{"available":false,"error":"no ~/.claude/projects"}'; return; }

    local files
    files=$(find "$dir" -name '*.jsonl' -mmin "-$WINDOW_MIN" 2>/dev/null | head -40)
    [ -z "$files" ] && { echo '{"available":true,"sessions":[]}'; return; }

    # tail -400: token totals are cumulative in intent but recorded per-message;
    # reading whole multi-MB transcripts on a UI timer is not acceptable. We
    # report recent-window tokens and say so, rather than pretending to a
    # lifetime total we did not pay to compute.
    local out="[]"
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        local slug session
        slug=$(basename "$(dirname "$f")")
        session=$(basename "$f" .jsonl)
        local s
        s=$(tail -n 400 "$f" 2>/dev/null | jq -c -s \
            --arg id "$session" --arg slug "$slug" '
            (map(select(.message.usage != null))) as $u
            | (map(select(.timestamp != null) | .timestamp) | max) as $last
            | {
                provider: "claude",
                id: $id,
                cwd: ($slug | gsub("^-";"/") | gsub("-";"/")),
                model: ([$u[].message.model] | map(select(. != "<synthetic>")) | last // ""),
                lastActivity: ($last // ""),
                tokensIn:    ([$u[].message.usage.input_tokens]              | add // 0),
                tokensOut:   ([$u[].message.usage.output_tokens]             | add // 0),
                cacheRead:   ([$u[].message.usage.cache_read_input_tokens]   | add // 0),
                cacheCreate: ([$u[].message.usage.cache_creation_input_tokens] | add // 0),
                turns: ($u | length),
                windowed: true
              }' 2>/dev/null)
        [ -n "$s" ] && out=$(jq -c --argjson s "$s" '. + [$s]' <<<"$out" 2>/dev/null || echo "$out")
    done <<<"$files"

    jq -c '{available:true, sessions:.}' <<<"$out" 2>/dev/null \
        || echo '{"available":false,"error":"claude parse failed"}'
}

# ----------------------------------------------------------------- codex ----
# ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl. The LAST token_count event holds
# cumulative `total_token_usage` for the session plus the live `rate_limits`
# block (primary = 5h window, secondary = weekly). Cumulative here, unlike
# Claude — so no windowing caveat.
codex_json() {
    local dir="$HOME/.codex/sessions"
    [ -d "$dir" ] || { echo '{"available":false,"error":"no ~/.codex/sessions"}'; return; }

    local files
    files=$(find "$dir" -name '*.jsonl' -mmin "-$WINDOW_MIN" 2>/dev/null | head -40)
    [ -z "$files" ] && { echo '{"available":true,"sessions":[],"limits":null}'; return; }

    local out="[]" limits="null"
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        local id s
        # rollout-2026-06-25T08-47-08-<uuid>.jsonl — take the uuid, not the
        # date prefix, so the id is stable and actually identifies the session.
        id=$(basename "$f" .jsonl | grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
        [ -n "$id" ] || id=$(basename "$f" .jsonl | sed 's/^rollout-//')
        s=$(jq -c -s --arg id "$id" '
            (map(select(.payload.type == "token_count")) | last) as $tc
            | (map(select(.payload.type == "session_meta")) | last) as $meta
            | if $tc == null then empty else {
                provider: "codex",
                id: $id,
                cwd: ($meta.payload.cwd // ""),
                model: ($meta.payload.model // ""),
                lastActivity: ($tc.timestamp // ""),
                tokensIn:    ($tc.payload.info.total_token_usage.input_tokens // 0),
                tokensOut:   ($tc.payload.info.total_token_usage.output_tokens // 0),
                cacheRead:   ($tc.payload.info.total_token_usage.cached_input_tokens // 0),
                cacheCreate: 0,
                reasoning:   ($tc.payload.info.total_token_usage.reasoning_output_tokens // 0),
                contextWindow: ($tc.payload.info.model_context_window // 0),
                rateLimits:  ($tc.payload.rate_limits // null),
                turns: 0,
                windowed: false
              } end' "$f" 2>/dev/null)
        if [ -n "$s" ]; then
            out=$(jq -c --argjson s "$s" '. + [$s]' <<<"$out" 2>/dev/null || echo "$out")
            local rl
            rl=$(jq -c '.rateLimits // empty' <<<"$s" 2>/dev/null)
            [ -n "$rl" ] && limits="$rl"   # newest file wins; loop is time-ordered by find
        fi
    done <<<"$files"

    jq -c --argjson lim "$limits" '{available:true, sessions:., limits:$lim}' <<<"$out" 2>/dev/null \
        || echo '{"available":false,"error":"codex parse failed"}'
}

# ------------------------------------------------------------ souveraine ----
# ~/.souveraine/server/agents/<agent>/conversations/<conv>/conversation.json
# carries updated_at + message_count. It does NOT carry aggregate token usage:
# the substrate persists per-turn `TokenUsage` in its message record, while
# this bounded presence scan intentionally reads only conversation metadata.
# We report -1 rather than 0, because 0 is a measurement and -1 is an
# admission. A direct substrate projection can fill the fields later without
# changing the surface contract.
souveraine_json() {
    local dir="$HOME/.souveraine/server/agents"
    [ -d "$dir" ] || { echo '{"available":false,"error":"no ~/.souveraine"}'; return; }

    local files
    files=$(find "$dir" -name 'conversation.json' -mmin "-$WINDOW_MIN" 2>/dev/null | head -40)
    [ -z "$files" ] && { echo '{"available":true,"sessions":[]}'; return; }

    local out="[]"
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        local s agent_id parent_id agent_name
        agent_id=$(jq -r '.agent_id // empty' "$f" 2>/dev/null)
        parent_id=${agent_id%-sub}
        agent_name=$(jq -r '.name // empty' "$dir/$parent_id/agent.json" 2>/dev/null)
        [ -n "$agent_name" ] || agent_name="Souveraine"
        s=$(jq -c --arg agentName "$agent_name" '
            # A zero-message record is a newly-minted conversation slot, not
            # an agent session. Rendering it made every panel restart add a
            # plausible-looking ghost row.
            if .archived == true or (.message_count // 0) == 0 then empty else {
                provider: "souveraine",
                id: (.id // ""),
                agentId: (.agent_id // ""),
                agentName: $agentName,
                cwd: "",
                model: "",
                lastActivity: (.last_message_at // .updated_at // ""),
                tokensIn: -1, tokensOut: -1, cacheRead: -1, cacheCreate: -1,
                turns: (.message_count // 0),
                subconscious: ((.agent_id // "") | endswith("-sub")),
                windowed: false
            } end' "$f" 2>/dev/null)
        [ -n "$s" ] && out=$(jq -c --argjson s "$s" '. + [$s]' <<<"$out" 2>/dev/null || echo "$out")
    done <<<"$files"

    jq -c '{available:true, sessions:.}' <<<"$out" 2>/dev/null \
        || echo '{"available":false,"error":"souveraine parse failed"}'
}

CLAUDE=$(claude_json);      [ -n "$CLAUDE" ]     || CLAUDE='{"available":false,"error":"collector crashed"}'
CODEX=$(codex_json);        [ -n "$CODEX" ]      || CODEX='{"available":false,"error":"collector crashed"}'
SOUV=$(souveraine_json);    [ -n "$SOUV" ]       || SOUV='{"available":false,"error":"collector crashed"}'

# Merge. `state` is derived here so every surface agrees on what "active" means
# — one authority, everything else a rendering.
#   active  : touched in the last 2 minutes
#   recent  : within the hour
#   idle    : older
# There is deliberately no "waiting" state. Knowing an agent awaits a permission
# decision requires the hook bridge (TASK-70 stage 2); inventing it from
# timestamps would be a guess wearing the costume of a measurement.
jq -c -n \
    --argjson now "$NOW" \
    --argjson window "$((WINDOW_MIN * 60))" \
    --argjson claude "$CLAUDE" \
    --argjson codex "$CODEX" \
    --argjson souv "$SOUV" '
    # jq'"'"'s fromdateiso8601 accepts ONLY %Y-%m-%dT%H:%M:%SZ. Every source here
    # emits fractional seconds (Claude .179Z, Codex .238Z, Souveraine .126950960Z),
    # so the naive parse fails on all of them — and a bare try/catch turns that
    # into a silent 0, which renders as "idle" for a session that is live right
    # now. Strip the fraction before parsing, and surface a parse miss as -1 so
    # a future breakage is visible instead of quietly plausible.
    def epoch:
        if . == "" or . == null then 0
        else (sub("\\.[0-9]+(?=Z$)"; "") | try fromdateiso8601 catch -1)
        end;
    def state($now): (.lastActivity | epoch) as $t
        | if $t == 0 then "idle"
          elif ($now - $t) < 120  then "active"
          elif ($now - $t) < 3600 then "recent"
          else "idle" end;
    ( ($claude.sessions // []) + ($codex.sessions // []) + ($souv.sessions // []) )
    | map(. + {state: state($now), age: ($now - (.lastActivity | epoch))})
    # `find -mmin` is only the cheap filesystem prefilter. Restores and copies
    # can touch a May record today; enforce the promised window against the
    # the record timestamp before anything calls it a current session.
    | map(select(.age >= 0 and .age <= $window))
    | sort_by(.age)
    as $all
    | {
        ts: $now,
        sessions: $all,
        active: ($all | map(select(.state == "active")) | length),
        providers: {
            claude:     ($claude | del(.sessions)) + {sessions: ($all | map(select(.provider == "claude"))     | length)},
            codex:      ($codex  | del(.sessions)) + {sessions: ($all | map(select(.provider == "codex"))      | length)},
            souveraine: ($souv   | del(.sessions)) + {sessions: ($all | map(select(.provider == "souveraine")) | length)}
        }
      }' 2>/dev/null \
  || printf '{"ts":%s,"sessions":[],"active":0,"providers":{"claude":{"available":false,"error":"merge failed"},"codex":{"available":false,"error":"merge failed"},"souveraine":{"available":false,"error":"merge failed"}}}\n' "$NOW"
