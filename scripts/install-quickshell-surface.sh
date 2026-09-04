#!/usr/bin/env bash
# Safely adopt the Souveraine QuickShell surface into an existing ii setup.
#
# This is deliberately separate from pacman installation: packages place
# assets on disk, while a person chooses whether their live shell may change.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: install-quickshell-surface.sh [--dry-run]

  --dry-run  Show every file in the current development overlay (default).

The present development overlay mixes phone and laptop assumptions. It is not
safe to adopt wholesale on either device. Profile-specific manifests must land
before an apply/adopt mode is enabled. Pacman should install assets only.
EOF
}

mode=dry-run
while (($#)); do
    case "$1" in
        --dry-run) mode=dry-run ;;
        --apply|--adopt)
            echo "Surface adoption is disabled until laptop and phone manifests are split." >&2
            exit 2
            ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY="$ROOT/surfaces/quickshell/deploy.sh"
QS="${XDG_CONFIG_HOME:-$HOME/.config}/quickshell"
II="$QS/ii"

[[ -x "$DEPLOY" ]] || { echo "surface deployer not found: $DEPLOY" >&2; exit 1; }
[[ -d "$II" ]] || {
    echo "No ii QuickShell configuration at $II." >&2
    echo "Install and initialise QuickShell/illogical-impulse first; no files changed." >&2
    exit 1
}

conflicts=()
new_count=0
managed_count=0

while read -r rel target; do
    src="$ROOT/surfaces/quickshell/$rel"
    dst="$QS/$target"
    [[ -f "$src" ]] || { echo "surface source missing: $src" >&2; exit 1; }

    if [[ -L "$dst" && "$(readlink -f "$dst")" == "$src" ]]; then
        printf 'managed  %s\n' "$target"
        ((managed_count += 1))
    elif [[ -e "$dst" || -L "$dst" ]]; then
        printf 'replace  %s\n' "$target"
        conflicts+=("$target")
    else
        printf 'new      %s\n' "$target"
        ((new_count += 1))
    fi
done < <("$DEPLOY" --manifest)

printf '\n%d managed, %d new, %d replacement(s).\n' \
    "$managed_count" "$new_count" "${#conflicts[@]}"

echo "Dry run only. Profile-specific laptop/phone manifests are required before adoption can be enabled."
