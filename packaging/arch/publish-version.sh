#!/usr/bin/env bash
# Build and publish one immutable, signed pacman repository for a version tag.
#
# The tag is the release decision. It must be an annotated signed vMAJOR.MINOR.PATCH
# tag whose commit is already reachable from internal Gitea's public branch. This
# job publishes only to internal Gitea. Publication Rail copies the exact tag and
# these exact bytes to Forge after its independent checks pass.

set -euo pipefail

TAG="${1:?usage: publish-version.sh <tag> <repo-dir> [extra-asset ...]}"
REPO_DIR="${2:?usage: publish-version.sh <tag> <repo-dir> [extra-asset ...]}"
shift 2
EXTRA_ASSETS=("$@")

: "${RELEASE_TOKEN:?RELEASE_TOKEN must be set}"
: "${ARCHIVE_KEY:?ARCHIVE_KEY must be set}"

[[ "$TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
  || { echo "FATAL: stable release tag must be vMAJOR.MINOR.PATCH" >&2; exit 1; }

SERVER="${GITHUB_SERVER_URL:?GITHUB_SERVER_URL must be set}"
ARCHIVE_REPO="${ARCHIVE_REPO:-${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}}"
API="$SERVER/api/v1"
AUTH="Authorization: token $RELEASE_TOKEN"

log() { echo "[publish-version] $*"; }
api() { curl -sf -H "$AUTH" "$@"; }

TAG_OBJECT=$(git rev-parse --verify "$TAG^{tag}" 2>/dev/null) \
  || { echo "FATAL: $TAG is not an annotated tag" >&2; exit 1; }
COMMIT=$(git rev-parse --verify "$TAG^{commit}")
if [ "$COMMIT" != "${GITHUB_SHA:?GITHUB_SHA must be set}" ]; then
  echo "FATAL: $TAG peels to $COMMIT, workflow is building $GITHUB_SHA" >&2
  exit 1
fi
git merge-base --is-ancestor "$COMMIT" refs/remotes/origin/public \
  || { echo "FATAL: $TAG is not reachable from internal public" >&2; exit 1; }

# Trust in the signer is checked by the root-owned boundary service. The product
# job only refuses unsigned/lightweight tags so it cannot accidentally mint a
# release shape the boundary will never accept.
git cat-file tag "$TAG_OBJECT" \
  | grep -Eq '^-----BEGIN (PGP|SSH) SIGNATURE-----$' \
  || { echo "FATAL: $TAG has no Git tag signature" >&2; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ASSETS="$STAGE/assets"
mkdir -p "$ASSETS"

for ARCH_DIR in "$REPO_DIR"/*/; do
  [ -d "$ARCH_DIR" ] || continue
  ARCH=$(basename "$ARCH_DIR")
  shopt -s nullglob
  PKGS=("$ARCH_DIR"*.pkg.tar.zst)
  shopt -u nullglob
  [ ${#PKGS[@]} -gt 0 ] || continue

  DB="souveraine-$ARCH"
  WORK="$STAGE/$ARCH"
  mkdir -p "$WORK"
  for package in "${PKGS[@]}"; do
    [ -f "$package.sig" ] \
      || { echo "FATAL: unsigned package $(basename "$package")" >&2; exit 1; }
    gpg --batch --verify "$package.sig" "$package"
    cp "$package" "$package.sig" "$ASSETS/"
  done

  repo-add --include-sigs --sign --key "$ARCHIVE_KEY" \
    "$WORK/$DB.db.tar.zst" "${PKGS[@]}"
  gpg --batch --verify "$WORK/$DB.db.tar.zst.sig" "$WORK/$DB.db.tar.zst"

  rm -f "$WORK/$DB.db" "$WORK/$DB.db.sig"
  cp "$WORK/$DB.db.tar.zst" "$ASSETS/$DB.db.tar.zst"
  cp "$WORK/$DB.db.tar.zst.sig" "$ASSETS/$DB.db.tar.zst.sig"
  cp "$WORK/$DB.db.tar.zst" "$ASSETS/$DB.db"
  cp "$WORK/$DB.db.tar.zst.sig" "$ASSETS/$DB.db.sig"
  if [ -f "$WORK/$DB.files.tar.zst" ]; then
    cp "$WORK/$DB.files.tar.zst" "$ASSETS/$DB.files.tar.zst"
    cp "$WORK/$DB.files.tar.zst" "$ASSETS/$DB.files"
    if [ -f "$WORK/$DB.files.tar.zst.sig" ]; then
      cp "$WORK/$DB.files.tar.zst.sig" "$ASSETS/$DB.files.tar.zst.sig"
      cp "$WORK/$DB.files.tar.zst.sig" "$ASSETS/$DB.files.sig"
    fi
  fi
done

shopt -s nullglob
BUILT_PACKAGES=("$ASSETS"/*.pkg.tar.zst)
shopt -u nullglob
[ ${#BUILT_PACKAGES[@]} -gt 0 ] \
  || { echo "FATAL: no packages were assembled" >&2; exit 1; }

cp packaging/arch/souveraine-archive-key.asc "$ASSETS/"
cp packaging/arch/souveraine-stable.conf "$ASSETS/"
for asset in "${EXTRA_ASSETS[@]+"${EXTRA_ASSETS[@]}"}"; do
  [ -f "$asset" ] || { echo "FATAL: release asset is absent: $asset" >&2; exit 1; }
  cp "$asset" "$ASSETS/"
done

python3 - "$ASSETS" "$ARCHIVE_REPO" "$TAG" "$TAG_OBJECT" "$COMMIT" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

assets = Path(sys.argv[1])
entries = []
for path in sorted(assets.iterdir(), key=lambda item: item.name):
    if not path.is_file():
        continue
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    entries.append({"name": path.name, "sha256": digest.hexdigest(), "size": path.stat().st_size})

manifest = {
    "schema_version": 1,
    "repository": sys.argv[2],
    "tag": sys.argv[3],
    "tag_object": sys.argv[4],
    "commit": sys.argv[5],
    "public_ref": "refs/heads/public",
    "archive_key_fingerprint": "3CD9E99E222C2A174986FC9AFF4949AA20C8E911",
    "assets": entries,
}
(assets / "publication-manifest.json").write_text(
    json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
PY
gpg --batch --yes --local-user "$ARCHIVE_KEY" \
  --detach-sign "$ASSETS/publication-manifest.json"

REL_JSON=$(curl -s -H "$AUTH" "$API/repos/$ARCHIVE_REPO/releases/tags/$TAG" || true)
REL_ID=$(printf '%s' "$REL_JSON" | python3 -c '
import json, sys
try:
    print(json.load(sys.stdin).get("id", ""))
except Exception:
    print("")
')

if [ -z "$REL_ID" ]; then
  REL_ID=$(api -X POST -H 'Content-Type: application/json' \
    "$API/repos/$ARCHIVE_REPO/releases" \
    -d "$(python3 - "$TAG" "$COMMIT" <<'PY'
import json, sys
print(json.dumps({
    "tag_name": sys.argv[1],
    "target_commitish": sys.argv[2],
    "name": sys.argv[1],
    "body": "Immutable signed package release built once by internal Gitea.",
    "draft": True,
    "prerelease": False,
}))
PY
)" | python3 -c 'import json, sys; print(json.load(sys.stdin)["id"])')
  log "created draft release $REL_ID for $TAG"
else
  LIVE_COMMIT=$(printf '%s' "$REL_JSON" | python3 -c '
import json, sys
body = json.load(sys.stdin)
print(body.get("target_commitish") or body.get("target") or "")
')
  [ -z "$LIVE_COMMIT" ] || [ "$LIVE_COMMIT" = "$COMMIT" ] \
    || { echo "FATAL: existing $TAG release targets $LIVE_COMMIT, expected $COMMIT" >&2; exit 1; }
  log "resuming release $REL_ID for $TAG"
fi

asset_ids() {
  api "$API/repos/$ARCHIVE_REPO/releases/$REL_ID/assets" | python3 -c '
import json, sys
name = sys.argv[1]
for asset in json.load(sys.stdin):
    if asset.get("name") == name:
        print(asset.get("id", ""))
' "$1"
}

upload_immutable() {
  local path="$1" name ids count live
  name=$(basename "$path")
  ids=$(asset_ids "$name")
  count=$(printf '%s\n' "$ids" | sed '/^$/d' | wc -l)
  if [ "$count" -gt 1 ]; then
    echo "FATAL: existing $TAG release has duplicate asset $name" >&2
    exit 1
  fi
  if [ "$count" -eq 1 ]; then
    live="$STAGE/live-$name"
    curl -sfL -H "$AUTH" "$SERVER/$ARCHIVE_REPO/releases/download/$TAG/$name" -o "$live"
    cmp -s "$path" "$live" \
      || { echo "FATAL: immutable asset $name already exists with different bytes" >&2; exit 1; }
    log "kept identical $name"
    return
  fi
  curl -sf -X POST -H "$AUTH" \
    "$API/repos/$ARCHIVE_REPO/releases/$REL_ID/assets?name=$name" \
    -F "attachment=@$path" -o /dev/null
  log "uploaded $name"
}

# Packages and their signatures land before databases. The signed manifest is
# last, so its presence means every byte it names was already accepted.
for path in "$ASSETS"/*.pkg.tar.zst "$ASSETS"/*.pkg.tar.zst.sig; do
  [ -f "$path" ] && upload_immutable "$path"
done
for path in "$ASSETS"/*; do
  [ -f "$path" ] || continue
  case "$(basename "$path")" in
    *.pkg.tar.zst|*.pkg.tar.zst.sig|*.db|*.db.sig|*.db.tar.zst|*.db.tar.zst.sig|publication-manifest.json|publication-manifest.json.sig)
      continue
      ;;
  esac
  upload_immutable "$path"
done
for path in "$ASSETS"/*.db.tar.zst "$ASSETS"/*.db.tar.zst.sig "$ASSETS"/*.db "$ASSETS"/*.db.sig; do
  [ -f "$path" ] && upload_immutable "$path"
done
upload_immutable "$ASSETS/publication-manifest.json"
upload_immutable "$ASSETS/publication-manifest.json.sig"

api -X PATCH -H 'Content-Type: application/json' \
  "$API/repos/$ARCHIVE_REPO/releases/$REL_ID" \
  -d "$(python3 - "$TAG" <<'PY'
import json, sys
print(json.dumps({
    "name": sys.argv[1],
    "body": "Immutable signed package release built once by internal Gitea.",
    "draft": False,
    "prerelease": False,
}))
PY
)" -o /dev/null

for path in "$ASSETS"/*; do
  [ -f "$path" ] || continue
  live="$STAGE/verify-$(basename "$path")"
  curl -sfL -H "$AUTH" \
    "$SERVER/$ARCHIVE_REPO/releases/download/$TAG/$(basename "$path")" -o "$live"
  cmp -s "$path" "$live" \
    || { echo "FATAL: read-back differs for $(basename "$path")" >&2; exit 1; }
done

log "$TAG published internally at $COMMIT with ${#BUILT_PACKAGES[@]} packages"
