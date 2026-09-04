# Pinned ii shell base

`ii-base/` is a content-exact vendor snapshot of the upstream tree identified
by `ii-base.pin`. The nested rounded-polygon submodule is expanded so a deploy
has no network or Git dependency.

Souveraine changes do not land in the vendor snapshot:

- device-independent changes belong in this directory's manifest-backed
  `modules/`, `services/`, or `scripts/` trees;
- phone-only changes belong in `ii-phone/`;
- `deploy.sh` first replaces `~/.config/quickshell/ii` from the pin, then
  applies `ii-phone/` on aarch64, and finally composes `qs -c souveraine`.

To update the pin, snapshot both devices first, replace `ii-base/` from a clean
upstream checkout (including the expanded shapes submodule), update
`ii-base.pin`, and require a checksum-only rsync dry run before deployment.
