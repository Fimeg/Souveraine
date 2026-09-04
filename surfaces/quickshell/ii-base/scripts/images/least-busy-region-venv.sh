#!/usr/bin/env bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

source "$HOME/.local/state/quickshell/.venv/bin/activate"
"$SCRIPT_DIR/least_busy_region.py" "$@"
deactivate
