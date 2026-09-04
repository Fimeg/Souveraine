# Souveraine working agreement

The personal agreement is `~/.codex/AGENTS.md`. The cross-repo map is
`../AGENTS.md`; read it when work crosses repository boundaries.

## This repository

- This is the Rust agent substrate and Souveraine shell code. Use
  **substrate**, not harness, unless quoting an external source.
- The map is `../SouveraineOS/saf/INDEX.md` — the living architecture lives
  at the umbrella, not beside the code. Read it before design work; the
  substrate's own old `saf/` here is a pointer, not a second spine.
- Work/status documentation for the wider OS belongs in
  `../SouveraineOS/saf/` and `../SouveraineOS/saf/state.md`, not a new local
  handoff. Update the one owner when behavior moves.
- The normal branch is `primary`. Verify status and recent history before
  editing, preserve unrelated changes, and stage explicit paths.

## Build and delivery

- Gitea Actions is the reproducible build/test/package path. Push the scoped
  change and let CI build it.
- Keep local checks small and targeted. Do not replace the pipeline with a
  direct build-host build, an offline build, or a hand-copied artifact.
- Runner access is diagnostic. A green result matters only if the relevant job
  ran and, when required, published the expected package.
- Phone delivery is through package ownership and pacman. A symlink or copied
  file on glass may prove behavior, but it is not landed.

## Shape of changes

- Preserve the substrate's one-owner instincts: one event path, one state
  writer, one canonical memory/history operation, one shipping path.
- Commit subjects are terse and honest: change and reason. Terse does not mean
  sterile in our first-party forge; a body may have teeth if it still serves
  the change. No AI attribution or `Co-Authored-By` line.
- Separate verified behavior from reasoned design and from work never exercised
  on hardware.
- Before calling cross-repo work complete, reconcile the owning SouveraineOS
  task/state document and archive the task if its acceptance is actually met.
