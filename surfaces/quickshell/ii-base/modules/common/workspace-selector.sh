#!/usr/bin/env bash
# Workspace Selector — quick jump to a numbered workspace
# Uses wofi for touch-friendly selection

set -euo pipefail

# Generate list of workspaces 1-10 (pipe to wofi dmenu mode)
CHOICE=$(printf '%s\n' {1..10} | wofi --dmenu --prompt="Go to workspace" --width=200 --height=400 --columns=2)

# If user cancelled, exit
[[ -z "$CHOICE" ]] && exit 0

# Strip any non-numeric prefix wofi might add (e.g. "1 - ")
CHOICE="${CHOICE##* }"
CHOICE="${CHOICE//[^0-9]/}"

# Validate and dispatch
if [[ "$CHOICE" =~ ^[0-9]+$ ]] && (( CHOICE >= 1 && CHOICE <= 10 )); then
    hyprctl dispatch "workspace $CHOICE"
fi
