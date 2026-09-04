#!/usr/bin/env bash
# Put the Live2D rig where the face host expects it.
#
# The rig lives in the repo (self-hosted; TASK-59's caution was about anything
# PUBLIC, and the Codeberg mirror is retired). This only copies it to where the
# host reads it at runtime.
set -euo pipefail
HERE_EARLY="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${1:-$HERE_EARLY/assets/face/live2d}"
DEST="${FACE_DIR:-$HOME/.souveraine/face}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"

[ -d "$SRC" ] || { echo "no rig at $SRC — pass its path as \$1" >&2; exit 1; }
mkdir -p "$DEST"
cp -r "$SRC" "$DEST/"
cp "$HERE/assets/face/index.html" "$DEST/index.html"
echo "staged $(du -sh "$DEST" | cut -f1) at $DEST"
