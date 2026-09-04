# Souveraine quickshell surface

The desktop shell as a Souveraine surface — the primary visual frontend for
SouveraineOS, with the TUI remaining the dive-in instrument.

## Architecture

- `services/Souveraine.qml` — the substrate singleton. The ONE server
  connection every shell module hangs off: agent inventory, conversation
  lifecycle, the SSE turn stream (raw events re-emitted via
  `streamEvent(var)`), the backchannel (`cancelTurn()` / `interject(text)`),
  and the desktop sensorium — every send carries ambient context (active
  window, open apps, cursor position) so she perceives the room she is being
  spoken to in. Device sensors (SouveraineOS positional data from the Pixel
  3 kernel path) extend `collectAmbient()`.
- `services/Ai.qml` — ii-compat adapter. Keeps the API the illogical-impulse
  sidebar expects; owns no transport. Shapes wire events into the message
  objects the existing chat UI renders.
- `modules/` — `souveraine/` (ours: lock, navigation, dial, subconscious),
  `settings/`, `common/`, and `ii/` (the files where we override upstream).
  Each subscribes to the Souveraine singleton. Still pending here: presence
  (portrait PNGs from memfs, posture state machine), the cockpit pane and the
  agent manager.

## What changes

- "Models" in the sidebar are **Souveraine agents** (`GET /v1/agents`).
  Picking one starts a conversation with that agent — memory, sensors,
  subconscious and all.
- Messages stream over the server's SSE endpoint
  (`POST /v1/conversations/:id/messages`), authenticated with the agent's
  bearer token from `~/.souveraine/server/agents/<id>/api_token`.
- Subconscious **surfacings**, **reflection**, and **archivist** pressure
  render in the chat as interface notes (dedicated widgets later).
- Reasoning and sensor activity render inside collapsible `<think>` blocks.
- Keys/providers/temperature are owned by `souveraine.toml` — the sidebar's
  `/key` and `/temp` commands now just point there. The keyring path is dead.
- Token pressure is fetched after each turn from
  `GET /v1/conversations/:id/tokens`.

## Portability (KDE / non-Hyprland)

`Souveraine.qml` itself is compositor-agnostic: quickshell runs on any
wlroots-ish Wayland compositor and KWin; window sensing uses the
foreign-toplevel protocol (KWin implements it); the cursor read tries
`hyprctl`, then `kdotool`, then degrades to nothing — ambient never blocks
a send. Server autostart is desktop-neutral (systemd user unit, nohup
fallback), so opening any surface summons her.

What is NOT portable yet is the chrome: the chat UI is illogical-impulse's
sidebar. The path for "I run KDE, can I use this?" is a standalone
quickshell config (own ShellRoot + a window hosting the chat/presence
modules) that ships `Souveraine.qml` unchanged — planned once the modules
stop being ii-embedded. Same service, same mappings, different shell.

## Deploy

```bash
./deploy.sh        # compose ~/.config/quickshell/souveraine
./deploy.sh -u     # remove the composed config (ii untouched)
./deploy.sh --phone  # rsync this surface to the phone and deploy there
```

`deploy.sh` no longer just swaps `Ai.qml` — it BUILDS the whole config: our
files symlinked from the repo, untouched upstream directories borrowed as
whole-dir symlinks into `~/.config/quickshell/ii`, and that ii tree itself
rsynced from this repo's `ii-base/` pin on every run (plus `ii-phone/` on
aarch64). Never hand-edit `~/.config/quickshell/ii` — the next deploy
overwrites it. Read the header of `deploy.sh`; it is the authority.

Requires the server: `souveraine server` (default http://127.0.0.1:8484,
override with `ai.souveraineUrl` in the ii config).

## Wire contract

The server's SSE layer is a full mirror of `BackendEvent` (see
`src/api/models.rs::StreamEvent` — exhaustive `From` impls both ways, so a
new engine event is a compile error at the seam, not a silent skip). The
surface consumes the personification channel: subconscious tokens buffer and
flush as one bubble when the N+1 pass ends (`subconscious_pass`), halts land
as body signals, interstitials render by register (cenno = quiet aside,
her_voice = gutter passage), `primary_complete` releases the input while the
stream stays open for the subconscious, and `context_pressure` drives the
live token counter. `atmosphere`/`outfit`/`itinerary` are logged, awaiting
their shell-chrome layer.

Server-side, the backchannel and verbs exist for every surface:
`POST /v1/conversations/:id/cancel` (interrupt, `*[raised hand]*`
semantics), `.../interject` (mid-turn notes, queued between turns),
`GET .../messages` (transcript backfill), `POST .../fork` (`/btw`
side-quests). `SendMessageRequest.ambient` injects the sensorium note.
RemoteBackend rides all of it, so TUI remote mode gained cancel/interject/
fork/resume in the same stroke.

## Not yet wired

- Sidebar UI hooks for cancel (Esc) and interject (type-while-busy) — the
  service functions exist, the ii chat input doesn't call them yet
- Conversation resume in the sidebar (server verb exists; surface always
  starts fresh)
- Atmosphere/outfit/itinerary driving actual shell chrome (events arrive;
  modules pending)
- File/image attachments (server has an image path; surface doesn't use it yet)
- Regenerate (Souveraine conversations are forward-only by doctrine)
- "Blank LLM mode" — a memoryless passthrough agent for throwaway questions;
  needs a server-side agent flavor first
- Dedicated widgets for surfacing/subconscious bubbles instead of interface
  notes
