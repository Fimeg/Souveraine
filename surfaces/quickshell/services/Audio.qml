pragma Singleton
pragma ComponentBehavior: Bound

import qs.modules.common
import QtQuick
import Quickshell
import Quickshell.Io

/*
 * PulseAudio-backed replacement for ii's PipeWire Audio singleton.
 *
 * The Pixel 3 uses native PulseAudio because its Q6 PCM needs the patched
 * module-alsa-sink fallback.  Quickshell has a PipeWire service but no native
 * PulseAudio service, so keep ii's public Audio API and mirror pactl's default
 * sink into the small node-shaped object the existing controls consume.
 */
Singleton {
    id: root

    property bool ready: false
    property bool sourceReady: false
    property bool syncing: false
    property bool autoMuted: false
    property bool micActive: false
    readonly property real hardMaxValue: 1.00
    property string audioTheme: Config.options.sounds.theme
    readonly property real value: sink.audio.volume

    property QtObject sink: QtObject {
        id: sinkNode
        property string id: name
        property string name: ""
        property string description: Translation.tr("Internal speakers")
        property string nickname: description
        property bool isSink: true
        property var properties: ({ "node.name": name })
        property QtObject audio: QtObject {
            property real volume: 0
            property bool muted: false

            onVolumeChanged: {
                if (root.ready && !root.syncing)
                    root.setVolume(volume)
            }
            onMutedChanged: {
                if (root.ready && !root.syncing)
                    root.setMuted(muted)
            }
        }
    }

    // Mirror PulseAudio's default source into the same node-shaped API used
    // for output.  This is the handset UCM capture endpoint (hw:0,1), not a
    // placeholder: QS must be able to control and report the real microphone
    // even while the lower-level zero-sample fault is being diagnosed.
    property QtObject source: QtObject {
        id: sourceNode
        property string id: name
        property string name: ""
        property string description: Translation.tr("Internal microphone")
        property string nickname: description
        property bool isSink: false
        property var properties: ({ "node.name": name })
        property QtObject audio: QtObject {
            property real volume: 0
            property bool muted: false

            onVolumeChanged: {
                if (root.sourceReady && !root.syncing)
                    root.setSourceVolume(volume)
            }
            onMutedChanged: {
                if (root.sourceReady && !root.syncing)
                    root.setSourceMuted(muted)
            }
        }
    }
    readonly property list<var> outputDevices: sink.name.length > 0 ? [sink] : []
    readonly property list<var> inputDevices: source.name.length > 0 ? [source] : []
    readonly property list<var> outputAppNodes: []
    readonly property list<var> inputAppNodes: []

    signal sinkProtectionTriggered(string reason)

    function friendlyDeviceName(node) {
        return node?.nickname || node?.description || Translation.tr("Unknown")
    }

    function appNodeDisplayName(node) {
        return node?.properties?.["application.name"] || node?.description || node?.name || Translation.tr("Unknown")
    }

    function refresh() {
        if (!statusProcess.running)
            statusProcess.running = true
    }

    function toggleMute() {
        if (!ready)
            return
        autoMuted = false
        setMuted(!sink.audio.muted)
    }

    function toggleMicMute() {
        if (!sourceReady)
            return
        setSourceMuted(!source.audio.muted)
    }

    function incrementVolume() {
        setVolume(Math.min(hardMaxValue, value + (value < 0.1 ? 0.01 : 0.02)))
    }

    function decrementVolume() {
        setVolume(Math.max(0, value - (value < 0.1 ? 0.01 : 0.02)))
    }

    function setVolume(nextVolume) {
        if (!ready || !isFinite(nextVolume))
            return
        const bounded = Math.max(0, Math.min(hardMaxValue, Number(nextVolume)))
        volumeProcess.exec(["pactl", "set-sink-volume", "@DEFAULT_SINK@", `${Math.round(bounded * 100)}%`])
        refreshSoon.restart()
    }

    function setMuted(muted) {
        if (!ready)
            return
        muteProcess.exec(["pactl", "set-sink-mute", "@DEFAULT_SINK@", muted ? "1" : "0"])
        refreshSoon.restart()
    }

    function setSourceVolume(nextVolume) {
        if (!sourceReady || !isFinite(nextVolume))
            return
        const bounded = Math.max(0, Math.min(hardMaxValue, Number(nextVolume)))
        sourceVolumeProcess.exec(["pactl", "set-source-volume", "@DEFAULT_SOURCE@", `${Math.round(bounded * 100)}%`])
        refreshSoon.restart()
    }

    function setSourceMuted(muted) {
        if (!sourceReady)
            return
        sourceMuteProcess.exec(["pactl", "set-source-mute", "@DEFAULT_SOURCE@", muted ? "1" : "0"])
        refreshSoon.restart()
    }

    function setDefaultSink(node) {
        if (!node?.name)
            return
        defaultSinkProcess.exec(["pactl", "set-default-sink", node.name])
        refreshSoon.restart()
    }

    function setDefaultSource(node) {
        if (!node?.name)
            return
        defaultSourceProcess.exec(["pactl", "set-default-source", node.name])
        refreshSoon.restart()
    }

    // Read the default sink and source in one transaction so the UI never
    // mixes state from different PulseAudio generations.
    // The tagged output avoids relying on locale-sensitive pactl labels.
    Process {
        id: statusProcess
        command: ["sh", "-c", "printf 'sink_name='; pactl get-default-sink; printf 'sink_volume='; pactl get-sink-volume @DEFAULT_SINK@; printf 'sink_mute='; pactl get-sink-mute @DEFAULT_SINK@; printf 'source_name='; pactl get-default-source; printf 'source_volume='; pactl get-source-volume @DEFAULT_SOURCE@; printf 'source_mute='; pactl get-source-mute @DEFAULT_SOURCE@; printf 'source_outputs='; pactl list short source-outputs | wc -l"]
        environment: ({ LANG: "C", LC_ALL: "C" })
        stdout: StdioCollector {
            onStreamFinished: {
                const text = this.text
                const sinkName = /^sink_name=(.+)$/m.exec(text)?.[1]?.trim()
                const sinkVolume = /^sink_volume=.*?(\d+)%/m.exec(text)?.[1]
                const sinkMuted = /^sink_mute=Mute:\s*(yes|no)$/m.exec(text)?.[1]
                const sourceName = /^source_name=(.+)$/m.exec(text)?.[1]?.trim()
                const sourceVolume = /^source_volume=.*?(\d+)%/m.exec(text)?.[1]
                const sourceMuted = /^source_mute=Mute:\s*(yes|no)$/m.exec(text)?.[1]
                const sourceOutputs = /^source_outputs=(\d+)$/m.exec(text)?.[1]
                if (!sinkName || sinkVolume === undefined || sinkMuted === undefined) {
                    root.ready = false
                } else {
                    root.syncing = true
                    sinkNode.name = sinkName
                    sinkNode.description = (sinkName.includes("hw_0_0") || sinkName.includes("Speaker__sink"))
                        ? Translation.tr("Internal speakers") : sinkName
                    sinkNode.audio.volume = Math.max(0, Math.min(root.hardMaxValue, Number(sinkVolume) / 100))
                    sinkNode.audio.muted = sinkMuted === "yes"
                    root.syncing = false
                    root.ready = true
                }

                if (!sourceName || sourceVolume === undefined || sourceMuted === undefined) {
                    root.sourceReady = false
                    root.micActive = false
                } else {
                    root.syncing = true
                    sourceNode.name = sourceName
                    sourceNode.description = (sourceName === "blueline_mic" || sourceName.includes("hw_0_1") || sourceName.includes("Mic__source"))
                        ? Translation.tr("Internal microphone") : sourceName
                    sourceNode.audio.volume = Math.max(0, Math.min(root.hardMaxValue, Number(sourceVolume) / 100))
                    sourceNode.audio.muted = sourceMuted === "yes"
                    root.syncing = false
                    root.sourceReady = true
                    root.micActive = Number(sourceOutputs ?? 0) > 0
                }
            }
        }
        onExited: exitCode => {
            if (exitCode !== 0) {
                root.ready = false
                root.sourceReady = false
                root.micActive = false
            }
        }
    }

    Process { id: volumeProcess }
    Process { id: muteProcess }
    Process { id: sourceVolumeProcess }
    Process { id: sourceMuteProcess }
    Process { id: defaultSinkProcess }
    Process { id: defaultSourceProcess }

    Timer {
        id: refreshSoon
        interval: 120
        repeat: false
        onTriggered: root.refresh()
    }

    Timer {
        interval: 1500
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    function playSystemSound(soundName) {
        const base = `/usr/share/sounds/${audioTheme}/stereo/${soundName}`
        Quickshell.execDetached(["sh", "-c", `ffplay -nodisp -autoexit \"${base}.oga\" 2>/dev/null || ffplay -nodisp -autoexit \"${base}.ogg\" 2>/dev/null`])
    }
}
