import QtQuick
import QtQuick.Layouts
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// Device — Souveraine form-factor and (later) per-device overrides.
// Principle: pages are views over Config.options / the owning daemon;
// nothing app-private. This page holds the knobs that describe WHAT this
// device is, so behaviors elsewhere gate on config, not hardcoded checks.

ContentPage {
    forceWidth: true

    ContentSection {
        icon: "smartphone"
        title: Translation.tr("Form factor")

        ConfigSwitch {
            buttonIcon: "smartphone"
            text: Translation.tr("Phone mode")
            checked: Config.options.souveraine.phone
            onCheckedChanged: {
                Config.options.souveraine.phone = checked;
            }
            StyledToolTip {
                text: Translation.tr("Gates phone behaviors: OSK rises for polkit prompts, phone-only pages, single-column layouts. Laptop deploys leave this off.")
            }
        }
    }

    // ── Proprioception ───────────────────────────────────────────────────
    // TASK-08(f) / TASK-19: the state machine computes its state, its
    // evidence, its confidence and its per-source health, and until now none
    // of it reached a screen. `forensic.jsonl` knew; the device could not tell
    // you. A body that cannot feel itself is the thing this OS is not
    // supposed to be.
    //
    // READOUT ONLY, deliberately. TASK-19: the confidence gates are computed,
    // logged and never branched on, so a control over them "would be lying" —
    // showing a threshold slider nothing consults breaks this page's own rule
    // against success-shaped switches. Observations can be shown honestly
    // today; controls wait on TASK-08(g).
    property bool _watchingEvidence: false

    function _startWatching() {
        if (_watchingEvidence) return;
        _watchingEvidence = true;
        DeviceEvidence.watch();
    }
    function _stopWatching() {
        if (!_watchingEvidence) return;
        _watchingEvidence = false;
        DeviceEvidence.unwatch();
    }

    Component.onCompleted: _startWatching()
    Component.onDestruction: _stopWatching()

    ContentSection {
        icon: "monitor_heart"
        title: Translation.tr("Device state")

        // The laptop has no sessiond. Say so, rather than rendering zeroes
        // that look like a healthy reading (§10: "no evidence" and "evidence
        // says nothing is happening" must not be the same state).
        StyledText {
            Layout.fillWidth: true
            visible: !DeviceEvidence.available
            text: Translation.tr("sessiond is not answering on this device — no state to report.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: DeviceEvidence.available ? [
                { k: Translation.tr("State"),        v: DeviceEvidence.state.device_state ?? "—" },
                { k: Translation.tr("Lock phase"),   v: DeviceEvidence.state.phase ?? "—" },
                { k: Translation.tr("Locked"),       v: (DeviceEvidence.state.locked ?? false) ? Translation.tr("yes") : Translation.tr("no") },
                { k: Translation.tr("Panel"),        v: (DeviceEvidence.state.panel_on ?? false) ? Translation.tr("on") : Translation.tr("off") },
                { k: Translation.tr("Dimmed"),       v: (DeviceEvidence.state.dimmed ?? false) ? Translation.tr("yes") : Translation.tr("no") },
                { k: Translation.tr("Display active"), v: (DeviceEvidence.state.display_active ?? false) ? Translation.tr("yes") : Translation.tr("no") },
                { k: Translation.tr("Idle"),         v: (DeviceEvidence.state.idle_secs ?? 0) + "s" },
                { k: Translation.tr("Observed"),     v: (DeviceEvidence.state.observed ?? false) ? Translation.tr("yes") : Translation.tr("no") },
                { k: Translation.tr("Confidence"),   v: Number(DeviceEvidence.state.observed_confidence ?? 0).toFixed(2) },
                { k: Translation.tr("Wake suppressed"), v: (DeviceEvidence.state.suppress_dpms_wake ?? false) ? Translation.tr("yes") : Translation.tr("no") },
                { k: Translation.tr("Shell alive"),  v: (DeviceEvidence.state.shell_alive ?? false) ? Translation.tr("yes") : Translation.tr("no") }
            ] : []

            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 8

                StyledText {
                    Layout.fillWidth: true
                    text: modelData.k
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
                StyledText {
                    text: String(modelData.v)
                    color: Appearance.colors.colOnLayer1
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
            }
        }
    }

    ContentSection {
        icon: "sensors"
        title: Translation.tr("Evidence sources")

        // The flag §10 was built for. It rides every forensic snapshot and had
        // nowhere to appear: the SLPI outage on 2026-07-25 killed every sensor
        // for four hours and exited status 0, so the crash reporter
        // structurally could not help.
        StyledText {
            Layout.fillWidth: true
            visible: DeviceEvidence.available && (DeviceEvidence.state.sensors_degraded ?? false)
            text: Translation.tr("A source reported and then went silent. Readings below are not trustworthy.")
            color: Appearance.colors.colError
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        StyledText {
            Layout.fillWidth: true
            visible: DeviceEvidence.available
            text: Translation.tr("live = reporting · unknown = never heard from (no reporter wired) · down = spoke, then stopped")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: {
                if (!DeviceEvidence.available) return [];
                const health = DeviceEvidence.state.sensor_health ?? {};
                const fresh = DeviceEvidence.state.evidence_fresh ?? {};
                return Object.keys(health).map(name => ({
                    name: name,
                    health: health[name],
                    fresh: fresh[name] === true
                }));
            }

            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 8

                MaterialSymbol {
                    iconSize: Appearance.font.pixelSize.normal
                    text: modelData.health === "live" ? "sensors"
                        : modelData.health === "down" ? "sensors_off"
                        : "help"
                    color: modelData.health === "down" ? Appearance.colors.colError
                        : modelData.health === "live" ? Appearance.colors.colOnLayer1
                        : Appearance.colors.colSubtext
                }
                StyledText {
                    Layout.fillWidth: true
                    text: modelData.name
                    color: Appearance.colors.colOnLayer1
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
                StyledText {
                    // "unknown" is not a failure — accel, light and touch have
                    // no reporter on this device and correctly sit there
                    // forever. Only a source that spoke and then stopped failed.
                    text: modelData.health + (modelData.fresh ? Translation.tr(" · fresh") : "")
                    color: modelData.health === "down" ? Appearance.colors.colError
                                                       : Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                }
            }
        }
    }

    ContentSection {
        icon: "history"
        title: Translation.tr("Recent decisions")

        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("What the machine last decided, and what it decided it from. The same entries the forensic trail hash-chains.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }

        Repeater {
            // Newest first, by seq — independent of the order the buffer
            // happens to return.
            model: (DeviceEvidence.recentDecisions ?? []).slice()
                .sort((a, b) => (b.seq ?? 0) - (a.seq ?? 0))
                .slice(0, 12)

            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 1

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    StyledText {
                        text: {
                            // `event` is a tagged union (decision,
                            // state-transition, sensor-input, error-*). Take
                            // whichever key it carries rather than assuming.
                            const ev = modelData.event ?? {};
                            const kind = Object.keys(ev)[0] ?? "event";
                            const body = ev[kind] ?? {};
                            return body.decision ?? body.to ?? kind;
                        }
                        color: Appearance.colors.colOnLayer1
                        font.pixelSize: Appearance.font.pixelSize.smaller
                        font.bold: true
                    }
                    Item { Layout.fillWidth: true }
                    StyledText {
                        text: "#" + (modelData.seq ?? "?")
                        color: Appearance.colors.colSubtext
                        font.pixelSize: Appearance.font.pixelSize.smaller
                    }
                }
                StyledText {
                    Layout.fillWidth: true
                    visible: (modelData.reason ?? "").length > 0
                    text: modelData.reason ?? ""
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    ContentSection {
        icon: "tune"
        title: Translation.tr("Device overrides")

        StyledText {
            Layout.fillWidth: true
            text: Translation.tr("Per-device profile overrides (panel size, sensor set, feel presets) land here as the framework grows. One config tree, many devices.")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }
    }
}
