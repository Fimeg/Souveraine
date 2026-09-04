#!/usr/bin/env bash
# Souveraine wallpaper download — fetch one random wallhaven image into the
# user's Wallpapers folder and print its path. Owned by this repo (not ii's
# scripts/colors/random tree, which is going away): self-contained, no
# QUICKSHELL_CONFIG_NAME assumption, no direct switchwall.sh call. The caller
# (WallpaperDownload service) applies the printed path through the shell's
# `wallpapers apply` IPC so config persistence + matugen theming stay in the
# shell process that owns them.
#
# Usage: download_wallhaven.sh [purity]     purity: sfw|sketchy|nsfw|all|csv
# stdout: the downloaded file's absolute path (only on success)
# stderr: human-readable errors; exit non-zero on failure.
set -euo pipefail

pictures_dir() {
    if command -v xdg-user-dir >/dev/null 2>&1; then
        xdg-user-dir PICTURES
        return
    fi
    local cfg="${XDG_CONFIG_HOME:-$HOME/.config}/user-dirs.dirs"
    if [ -f "$cfg" ]; then
        # shellcheck disable=SC1090
        ( . "$cfg" >/dev/null 2>&1; echo "${XDG_PICTURES_DIR/#\$HOME/$HOME}" )
        return
    fi
    echo "$HOME/Pictures"
}

WALL_DIR="$(pictures_dir)/Wallpapers"
mkdir -p "$WALL_DIR"

# API key + purity default: read from illogical-impulse config if present, but
# don't require it. When we vendor our own config store this path is the only
# ii coupling left and moves with it.
ii_config="$HOME/.config/illogical-impulse/config.json"
api_key=""
default_purity="sfw,sketchy"
if [ -f "$ii_config" ] && command -v jq >/dev/null 2>&1; then
    api_key="$(jq -r '.background.wallhaven.apiKey // empty' "$ii_config" 2>/dev/null || true)"
    p="$(jq -r '.background.wallhaven.purity // empty' "$ii_config" 2>/dev/null || true)"
    [ -n "$p" ] && default_purity="$p"
fi

purity="${1:-$default_purity}"
case "$purity" in
    sfw)                        bits="100" ;;
    sketchy)                    bits="010" ;;
    nsfw)                       bits="001" ;;
    sfw,sketchy|sketchy,sfw)    bits="110" ;;
    sfw,nsfw|nsfw,sfw)          bits="101" ;;
    sketchy,nsfw|nsfw,sketchy)  bits="011" ;;
    all)                        bits="111" ;;
    *)                          bits="100" ;;
esac

# 9x16 portrait, at least 1080x2160, random order, one result.
url="https://wallhaven.cc/api/v1/search?ratios=9x16&purity=${bits}&sorting=random&atleast=1080x2160&limit=1"
[ -n "$api_key" ] && url="${url}&apikey=${api_key}"

response="$(curl -fsS "$url")" || { echo "wallhaven request failed" >&2; exit 1; }

if echo "$response" | jq -e '.error' >/dev/null 2>&1; then
    echo "wallhaven error: $(echo "$response" | jq -r '.error')" >&2
    exit 1
fi

link="$(echo "$response" | jq -r '.data[0].path // empty')"
[ -n "$link" ] || { echo "no wallpapers matched" >&2; exit 1; }

ext="${link##*.}"
# Name by wallhaven ID so every download is a distinct, traceable file and the
# library grows instead of overwriting a single wallhaven_wallpaper.<ext>.
id="$(echo "$response" | jq -r '.data[0].id // empty')"
[ -n "$id" ] || id="$(date +%s)"
dest="$WALL_DIR/wallhaven_${id}.${ext}"

# Download beside the destination, validate the complete image, then publish it
# atomically. A failed/interrupted curl must never leave a truncated wallpaper
# that later runs mistake for a valid cached download.
tmp="$(mktemp --tmpdir="$WALL_DIR" ".wallhaven_${id}.XXXXXX")"
cleanup() {
    [ -z "${tmp:-}" ] || rm -f -- "$tmp"
}
trap cleanup EXIT INT TERM

curl -fsSL --retry 2 --retry-all-errors "$link" -o "$tmp"
mime="$(file -b --mime-type -- "$tmp" 2>/dev/null || true)"
case "$mime" in
    image/jpeg|image/png|image/webp) ;;
    *) echo "wallhaven returned invalid image data ($mime)" >&2; exit 1 ;;
esac

if command -v magick >/dev/null 2>&1; then
    magick identify "$tmp" >/dev/null 2>&1 \
        || { echo "downloaded wallpaper failed image validation" >&2; exit 1; }
elif command -v identify >/dev/null 2>&1; then
    identify "$tmp" >/dev/null 2>&1 \
        || { echo "downloaded wallpaper failed image validation" >&2; exit 1; }
fi

mv -f -- "$tmp" "$dest"
tmp=""

echo "$dest"
