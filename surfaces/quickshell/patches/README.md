# patches/

> **0008 and 0009 landed `93fe30a` (2026-08-12).** Applied by Casey, reloaded,
> and — for the first time in this lane — *verified against a running shell*.
>
> The load immediately found two faults that every prior gate had called clean
> (`44acb11`): a missing `qs.modules.common.functions` import in `ThinkingCard`
> throwing `ReferenceError: ColorUtils` on every reasoning segment, and
> `sidebar.ai.fontSize` absent from our Config override, so the vendor
> `MessageTextBlock` assigned `undefined` to `font.pixelSize` on every render.
>
> Neither was reachable by lint as it was being run. The lesson is recorded in
> that commit and worth repeating here: **lint the composed tree, not a
> hand-built approximation of it** — a stand-in tests the stand-in. And a
> qmllint run that resolves nothing reports 0 findings in exactly the same
> voice as one that resolves everything.

<details>
<summary>Original 0008 header — kept for the two controls dropped on purpose</summary>

> `0008-agent-surface-owned-message-delegate.patch` swapped the message delegate
> to `modules/souveraine/agent/AgentMessage.qml`. When it was queued, that file
> carried **none** of the vendor's seven controls
> (`ii-base/.../aiChat/AiMessage.qml:196-308`), so applying it would have
> silently removed all seven — a parity regression wearing the costume of a
> swap.
>
> As of `df11bba` five are carried (Speak, Re-synthesize, Copy, Show-raw,
> Delete) and **two are dropped deliberately**, which is a decision and not an
> oversight:
>
> - **Regenerate** — the conversation is forward-only. `Ai.regenerate()` is
>   already a no-op returning advice, so the button's only behaviour was to
>   explain it did nothing.
> - **Edit** — there is no in-place edit. The vendor's wrote to a local array
>   the server never sees, so the message read back was not the message held.
>
> **Delete is armed, not immediate.** `removeMessage()` splices two local
> arrays and leaves the server transcript untouched — `/resume` brings the
> message straight back — so it is a view filter wearing a delete icon. The
> armed row says so in words.
>
> Apply **0009 first or together**: the re-synthesize control prefers
> `Speech.resynthesize()`, which 0009 adds. Without it the control degrades to
> the legacy cache-hitting path rather than breaking, but that path cannot
> actually re-synthesize.
>
> Lint: 0 syntax findings; every other finding traces to one directory import
> qmllint cannot resolve, proven environmental because `AiChat.qml` — live in
> the running shell — produces the identical failure. **Nothing has loaded it.**

</details>


Staged shell changes that are **not** applied to the tree.

## Why this directory existed — and why it is being wound down

> **Retired 2026-08-12.** The premise below was measured and found false. New
> shell work is edited directly; see *Direct edits* at the foot of this file.

The original rule was: *editing a symlinked file edits the running shell, so an
edit can take down the process the agent's own turn is running inside.*

**It cannot.** Measured twice on 2026-08-12:

    qs   232867  /user@1000.service/kitty-20961-2.scope
    me   442470  /user@1000.service/app.slice/souveraine.service

Separate cgroups. The shell is parented to the terminal it was launched from,
not to `souveraine.service`. On 2026-08-12 patches 0007, 0010 and 0011 were
applied *directly to the live symlinked files* while an agent turn was in
flight; the shell reloaded and kept running, PID unchanged, for five hours
afterwards.

What remains true is smaller: a shell that fails to compile does not come back
on its own (29ec9fe — one bad root type failed the whole `qs.services` module).
That costs **Casey his bar**, not the agent its turn, and it is one
`git checkout` from repaired. It is a reason to *verify*, not a reason to
*queue*.

## Direct edits — the discipline that replaces the queue

1. **Commit before the edit.** Rollback is `git checkout <file> && ./deploy.sh`.
2. **Lint the composed tree**, never a hand-built stand-in
   (`/usr/lib/qt6/bin/qmllint`, resolved against `~/.config/quickshell/`).
   A run that resolves nothing reports 0 findings in the same voice as one that
   resolves everything.
3. **Scan the shell log after the reload** —
   `/tmp/souveraine-shell.log`, for `Error|error:|Unable to assign|ReferenceError`.
   This is the step that was missing all along: on 2026-08-12 the shell had been
   throwing `Cannot assign [undefined] to int` on *every turn for two and a half
   hours* and nobody read the file.
4. **Confirm it is still alive ~10s after "Configuration Loaded."** A load is
   not a pass — the null-config crash loads clean and dies ten seconds later.

## Applying

    cd ~/Projects/souveraine
    git am surfaces/quickshell/patches/0001-....patch
    # then reload the shell yourself and watch it come up

Verify before trusting:

    cd surfaces/quickshell
    ./deploy.sh
    timeout 40 qs -c souveraine 2>&1 | grep -E "ERROR|Configuration Loaded"

If it does not come up, `git reset --hard HEAD~1` and the patch is just a file
again.

## Discipline

- One patch, one behaviour. Reviewable in a sitting.
- The patch's commit message says what it changes and what it deliberately
  leaves alone.
- A patch is **untested against a running shell** unless its notes say
  otherwise. Say which: verified / reasoned / untested.
- Applied patches get deleted from this directory in the same commit that
  applies them. This directory is a queue, not an archive — git keeps history.

## In the queue

**Empty as of 2026-08-13.** 0001 and 0006 landed (`ec87e6a`, `6edebae`); 0007
and 0010 landed earlier in `6f12fce` and their files were removed then, but
this list was not updated — it went on describing them as pending for a day.
A queue file is a claim with a shelf life, and so is a queue *README*.

Status below is from `git apply --check` in **both directions**, not from
memory: forward-applies means pending, reverse-applies means already landed.
Run it before trusting this list.

    for p in surfaces/quickshell/patches/000*.patch; do
      git apply --check "$p" 2>/dev/null && echo "$p PENDING"
      git apply --check --reverse "$p" 2>/dev/null && echo "$p LANDED"
    done

With the queue empty, `~/Documents/casey.sh` still has a job: it runs
verify-only, restarting the shell and proving HEAD loads. It used to name
0008/0009 explicitly and died on "missing 0008" once they landed; the queue is
its input now.


## Recently closed

- **0001 — resume offer. Landed `ec87e6a`, 2026-08-13.** Pending since 08-10.
  The agent-established hook now *offers* the latest thread instead of silently
  attaching it; default is start-fresh, `ai.autoResume: true` restores the old
  behaviour. Explicit resume paths (`/resume`, the Face control) call with no
  argument and are unchanged, and the e4e6594 amnesia fix is untouched — **do
  not revert that commit.** The second half is still undrawn: `resumeOffered`
  fires and nothing renders it. With no UI the behaviour is still correct
  (doing nothing starts fresh, which is the ask) but the affordance to continue
  is missing. That is the first job of the renderer lane.
- **0006 — step-up authenticates through PAM. Landed `6edebae`, 2026-08-13.**
  **Rook's work, not mine** — the patch file carried my identity in its From
  header and I did not write it. `StepUpAuth` called `souveraine-pam-auth` (a
  binary never written) and fell back to `pkcheck` against an action never
  shipped, so **every step-up grant was silently denied**. Now a real
  `PamContext` against `souveraine-stepup`, which `cc541d1` ships and which is
  present on this box (owned by `souveraine r467`, the running version).
  Verified against the type system rather than by hope: every API it uses is
  either already used by `LockContext.qml` (live in the running shell) or
  present in `quickshell-service-pam.qmltypes` — `responseVisible` and
  `PamResult.toString(value)` were the two LockContext does not exercise, and
  both exist. **Still password, not finger**: `fpc-polkit-pam.c:86` refuses
  every service except `polkit-1`. Widening that is one line and a security
  decision that is Casey's.
- **0007 — context occupancy + hyprctl cursor. Landed `6f12fce`, 2026-08-12.**
  Keep: **before building the click-through breakdown on this**, read
  `tasks/context-accounting-two-counters-2026-08-12.md`. The pill renders the
  *payload* (what is in the model's window); `/v1/conversations/:id/tokens`
  renders the *archive* (what the transcript weighs). They differ by ~3x. Ship
  them side by side unlabelled and the Panel shows two numbers that contradict
  each other. Group the breakdown by **block kind, not role** — 74% is tool
  traffic, which per-role accounting files under "assistant" and makes it read
  as talkativeness.
- **0010 — halt in her own register. Landed `6f12fce`, 2026-08-12.** Keep: the
  wording deliberately mirrors `migraine_text()` in `src/server/turn.rs`, which
  `29f8475` commits into her history — so what the human reads and what she
  carries into the next turn are the same sentence. **If one changes, the other
  must.**

- **0008 — owned message delegate. Landed `93fe30a`, 2026-08-12.** Applied by
  Casey and reloaded. The load found two faults no gate had (`44acb11`) — see
  the header. Worth keeping: the two controls dropped were *decisions*
  (regenerate is impossible on text; there is no in-place edit), so a later
  session should not helpfully restore them.
- **0009 — speech: stop actually stops. Landed `93fe30a`, 2026-08-12.**
  `sh -c "mpv … || ffplay …"` is a *compound* command, so sh does not
  exec-replace itself and SIGTERM killed the wrapper while the player kept
  sounding as an orphan — reproduced directly. That was the back-to-back TTS
  slam. Shell dropped entirely; the pid held is now the pid making noise.
  `resynthesize()` bypasses the cache the old path always hit.
  **Still unverified: no audio has been played through it.**

- **0002 — think-fence collision. Retired 2026-08-12, obsolete not abandoned.**
  It patched `inThinkBlock` in `services/Ai.qml`; `grep -c inThinkBlock` on
  that file now returns **0**. Typed chat segments (0003) made the collision
  impossible *by construction* — a sensor firing mid-reasoning lands in its own
  segment and cannot close a fence, because there is no fence. Escaping harder
  was the right fix for the string design; the design changed underneath it.
  A queued patch that can no longer apply reads as "not done yet" forever, so
  it goes.
- **0003 — typed chat segments + tool cards.** Landed as `8c434df`.
- **0004 — keep typed segments QML-compatible.** Landed as `23cebc5`.
- **0005 — config: never store a null option.** Applied and committed as
  `f7e0e51`. Null-valued options serialised into `config.json` and segfaulted
  `JsonAdapter::deserializeRec` on the *next* launch — the session that wrote
  the option ran fine, every launch after it died ~10 s in. Replaced with
  tristate strings (`auto`/`on`/`off`), booleans still honoured. Verified live:
  shell loads clean, no nulls written back.
- **0005 (earlier) — mount the agent island.** Retired by `013a06a`; the island
  left the bar to sit beside chat (`2179b62`). The number was reused.
