#!/bin/sh
# Souveraine Settings is a normal app window owned by the already-running
# authoritative shell. This launcher is only an IPC client: it must never
# start another Quickshell configuration or session heartbeat.
set -eu

for name in WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_RUNTIME_DIR XDG_CURRENT_DESKTOP; do
    value="$(systemctl --user show-environment 2>/dev/null | sed -n "s/^${name}=//p" | head -n 1)"
    [ -n "$value" ] && export "${name}=${value}"
done

export QT_QPA_PLATFORM=wayland
exec qs -c souveraine ipc --any-display call settings open
