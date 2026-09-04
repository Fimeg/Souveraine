// Souveraine patch to ii's stock Persistent.qml.
//
// Adds states.lock.locked: written by LockScreen.qml whenever the session
// lock engages/releases, so a quickshell crash or restart while locked
// comes back locked instead of silently dropping the lock. Everything else
// is unchanged stock ii — diff against upstream before re-applying if ii
// updates.
pragma Singleton
pragma ComponentBehavior: Bound
import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root
    property alias states: persistentStatesJsonAdapter
    property string fileDir: Directories.state
    property string fileName: "states.json"
    property string filePath: `${root.fileDir}/${root.fileName}`

    property bool ready: false
    property string previousHyprlandInstanceSignature: ""

    // What identifies THIS compositor run.
    //
    // `HYPRLAND_INSTANCE_SIGNATURE` is unset under viewtop, so this compared
    // "" to "" and `isNewHyprlandInstance` was false every single time. It
    // gates `lock.launchOnStartup` (LockScreen.qml), which therefore never
    // fired once, and both Idle.qml copies read it too. viewtop publishes
    // `viewtop.instance` beside its control socket for exactly this: pid plus
    // startup nanos, different on every start.
    //
    // The env var stays as the fallback so this file still behaves on a
    // Hyprland session — the laptop is still one — and an empty answer from
    // both means "assume the session continues", which re-locks rather than
    // assuming a fresh boot.
    readonly property string instanceSignature: viewtopInstance.text().trim()
        || Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")
        || ""

    property bool isNewHyprlandInstance: previousHyprlandInstanceSignature !== states.hyprlandInstanceSignature

    FileView {
        id: viewtopInstance
        path: `${Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/1000"}/souveraine/viewtop.instance`
        // Absence is normal, not an error: a Hyprland session has no such file
        // and falls through to the env var below.
        printErrors: false
        // Synchronously, because `onReadyChanged` reads `text()` exactly once
        // to decide whether this is a new session. Loaded async it was still
        // empty at that moment, fell through to the unset env var, and the
        // signature persisted as "" — the same bug this replaced, arrived at
        // by a different route.
        blockLoading: true
    }

    onReadyChanged: {
        root.previousHyprlandInstanceSignature = root.states.hyprlandInstanceSignature
        root.states.hyprlandInstanceSignature = root.instanceSignature
    }

    Timer {
        id: fileReloadTimer
        interval: 100
        repeat: false
        onTriggered: {
            persistentStatesFileView.reload()
        }
    }

    Timer {
        id: fileWriteTimer
        interval: 100
        repeat: false
        onTriggered: {
            persistentStatesFileView.writeAdapter()
        }
    }

    FileView {
        id: persistentStatesFileView
        path: root.filePath

        watchChanges: true
        onFileChanged: fileReloadTimer.restart()
        onAdapterUpdated: fileWriteTimer.restart()
        onLoaded: root.ready = true
        onLoadFailed: error => {
            console.log("Failed to load persistent states file:", error);
            if (error == FileViewError.FileNotFound) {
                fileWriteTimer.restart();
            }
        }

        adapter: JsonAdapter {
            id: persistentStatesJsonAdapter

            property string hyprlandInstanceSignature: ""

            property JsonObject ai: JsonObject {
                property string model: "gemini-2.5-flash"
                property real temperature: 0.5
            }

            property JsonObject cheatsheet: JsonObject {
                property int tabIndex: 0
            }

            property JsonObject sidebar: JsonObject {
                property JsonObject bottomGroup: JsonObject {
                    property bool collapsed: false
                    property int tab: 0
                }
            }

            property JsonObject booru: JsonObject {
                property bool allowNsfw: false
                property string provider: "yandere"
            }

            property JsonObject idle: JsonObject {
                property bool inhibit: false
            }

            property JsonObject lock: JsonObject {
                property bool locked: false
            }

            // Navigation-rail onboarding. `missionControlDiscovered` flips
            // true the first time the triple-swipe raises Mission Control;
            // until then the rail may nudge the gesture after repeated
            // incomplete swipes (the Souveraine analogue of Launcher3's
            // AllAppsEduView, driven from SystemGestureRail.qml). Persisted so
            // the nudge doesn't return after the user has found the gesture.
            property JsonObject navigation: JsonObject {
                property bool missionControlDiscovered: false
            }

            property JsonObject overlay: JsonObject {
                property list<string> open: ["crosshair", "recorder", "volumeMixer", "resources"]
                property JsonObject crosshair: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: true
                    property real x: 827
                    property real y: 441
                    property real width: 250
                    property real height: 100
                }
                property JsonObject floatingImage: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: false
                    property real x: 1650
                    property real y: 390
                    property real width: 0
                    property real height: 0
                }
                property JsonObject fpsLimiter: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: false
                    property real x: 1570
                    property real y: 615
                    property real width: 280
                    property real height: 80
                }
                property JsonObject recorder: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: false
                    property real x: 80
                    property real y: 80
                    property real width: 350
                    property real height: 130
                }
                property JsonObject resources: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: true
                    property real x: 1500
                    property real y: 770
                    property real width: 350
                    property real height: 200
                    property int tabIndex: 0
                }
                property JsonObject volumeMixer: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: false
                    property real x: 80
                    property real y: 280
                    property real width: 350
                    property real height: 600
                    property int tabIndex: 0
                }
                property JsonObject notes: JsonObject {
                    property bool pinned: false
                    property bool clickthrough: true
                    property real x: 1400
                    property real y: 42
                    property real width: 460
                    property real height: 330
                }
            }

            property JsonObject timer: JsonObject {
                property JsonObject pomodoro: JsonObject {
                    property bool running: false
                    property int start: 0
                    property bool isBreak: false
                    property int cycle: 0
                }
                property JsonObject stopwatch: JsonObject {
                    property bool running: false
                    property int start: 0
                    property list<var> laps: []
                }
            }
        }
    }
}
