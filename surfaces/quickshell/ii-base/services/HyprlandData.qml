pragma Singleton
pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland

/**
 * Provides access to some Hyprland data not available in Quickshell.Hyprland.
 */
Singleton {
    id: root
    property var windowList: []
    property var addresses: []
    property var windowByAddress: ({})
    property var workspaces: []
    property var workspaceIds: []
    property var workspaceById: ({})
    property var activeWorkspace: null
    property var activeWindow: null
    property var monitors: []

    // Parse hyprctl's JSON, or keep what we had.
    //
    // Every collector below did a bare `JSON.parse()` on the process output.
    // That is fine while Hyprland is the compositor and a hard error the
    // moment it is not: under viewtop there is no `hyprctl`, the output is
    // empty, and each collector threw a SyntaxError on every refresh — six
    // exceptions per pass, forever, drowning the log the shell is diagnosed
    // from.
    //
    // Absence is a state, not a failure. `HYPRLAND_INSTANCE_SIGNATURE` unset
    // means "another compositor", and the honest answer is to keep the last
    // known value and say so once — the same distinction
    // DEVICE-STATE-MACHINE.md §10 draws between "no evidence" and "evidence
    // says nothing".
    property bool available: true
    function parseOrKeep(text, fallback, what) {
        if (!text || text.trim().length === 0) {
            if (root.available) {
                root.available = false;
                console.log("[HyprlandData] no hyprctl output for", what,
                            "— assuming another compositor; monitor geometry comes from `screen`");
            }
            return fallback;
        }
        try {
            const parsed = JSON.parse(text);
            if (!root.available) {
                root.available = true;
                console.log("[HyprlandData] hyprctl is answering again");
            }
            return parsed;
        } catch (e) {
            // Latched like the empty case: `hyprctl` missing does not always
            // mean *empty* output — a shell that prints an error to stdout
            // lands here instead, and it lands here on every refresh. One line
            // per edge, not four per pass. Same rule §10 applies to a sensor
            // that has gone quiet: say it when it changes, not when it repeats.
            if (root.available) {
                root.available = false;
                console.log("[HyprlandData]", what, "is unparseable —",
                            "assuming another compositor; monitor geometry comes from `screen`:", e);
            }
            return fallback;
        }
    }

    property var layers: ({})

    // Convenient stuff

    function toplevelsForWorkspace(workspace) {
        return ToplevelManager.toplevels.values.filter(toplevel => {
            const address = `0x${toplevel.HyprlandToplevel?.address}`;
            var win = HyprlandData.windowByAddress[address];
            return win?.workspace?.id === workspace;
        })
    }

    function hyprlandClientsForWorkspace(workspace) {
        return root.windowList.filter(win => win.workspace.id === workspace);
    }

    function clientForToplevel(toplevel) {
        if (!toplevel || !toplevel.HyprlandToplevel) {
            return null;
        }
        const address = `0x${toplevel?.HyprlandToplevel?.address}`;
        return root.windowByAddress[address];
    }

    // Internals

    function updateWindows() {
        getClients.running = true;
        getActiveWindow.running = true;
    }

    function updateLayers() {
        getLayers.running = true;
    }

    function updateMonitors() {
        getMonitors.running = true;
    }

    function updateWorkspaces() {
        getWorkspaces.running = true;
        getActiveWorkspace.running = true;
    }

    function updateAll() {
        updateWindows();
        updateMonitors();
        updateLayers();
        updateWorkspaces();
    }

    function biggestWindowForWorkspace(workspaceId) {
        const windowsInThisWorkspace = HyprlandData.windowList.filter(w => w.workspace.id == workspaceId);
        return windowsInThisWorkspace.reduce((maxWin, win) => {
            const maxArea = (maxWin?.size?.[0] ?? 0) * (maxWin?.size?.[1] ?? 0);
            const winArea = (win?.size?.[0] ?? 0) * (win?.size?.[1] ?? 0);
            return winArea > maxArea ? win : maxWin;
        }, null);
    }

    Component.onCompleted: {
        updateAll();
    }

    Connections {
        target: Hyprland

        function onRawEvent(event) {
            // console.log("Hyprland raw event:", event.name);
            if (["openlayer", "closelayer", "screencast"].includes(event.name)) return;
            updateAll()
        }
    }

    Process {
        id: getClients
        command: ["hyprctl", "clients", "-j"]
        stdout: StdioCollector {
            id: clientsCollector
            onStreamFinished: {
                root.windowList = root.parseOrKeep(clientsCollector.text, [], "data")
                let tempWinByAddress = {};
                for (var i = 0; i < root.windowList.length; ++i) {
                    var win = root.windowList[i];
                    tempWinByAddress[win.address] = win;
                }
                root.windowByAddress = tempWinByAddress;
                root.addresses = root.windowList.map(win => win.address);
            }
        }
    }

    Process {
        id: getActiveWindow
        command: ["hyprctl", "activewindow", "-j"]
        stdout: StdioCollector {
            id: activeWindowCollector
            onStreamFinished: {
                root.activeWindow = root.parseOrKeep(activeWindowCollector.text, root.activeWindow, "activewindow")
            }
        }
    }

    Process {
        id: getMonitors
        command: ["hyprctl", "monitors", "-j"]
        stdout: StdioCollector {
            id: monitorsCollector
            onStreamFinished: {
                root.monitors = root.parseOrKeep(monitorsCollector.text, root.monitors, "monitors");
            }
        }
    }

    Process {
        id: getLayers
        command: ["hyprctl", "layers", "-j"]
        stdout: StdioCollector {
            id: layersCollector
            onStreamFinished: {
                root.layers = root.parseOrKeep(layersCollector.text, root.layers, "layers");
            }
        }
    }

    Process {
        id: getWorkspaces
        command: ["hyprctl", "workspaces", "-j"]
        stdout: StdioCollector {
            id: workspacesCollector
            onStreamFinished: {
                var rawWorkspaces = root.parseOrKeep(workspacesCollector.text, root.workspaces, "workspaces");
                // Filter out invalid workspace ids (e.g. lock-screen temp workspace 2147483647 - N)
                root.workspaces = rawWorkspaces.filter(ws => ws.id >= 1 && ws.id <= 100);
                let tempWorkspaceById = {};
                for (var i = 0; i < root.workspaces.length; ++i) {
                    var ws = root.workspaces[i];
                    tempWorkspaceById[ws.id] = ws;
                }
                root.workspaceById = tempWorkspaceById;
                root.workspaceIds = root.workspaces.map(ws => ws.id);
            }
        }
    }

    Process {
        id: getActiveWorkspace
        command: ["hyprctl", "activeworkspace", "-j"]
        stdout: StdioCollector {
            id: activeWorkspaceCollector
            onStreamFinished: {
                root.activeWorkspace = root.parseOrKeep(activeWorkspaceCollector.text, root.activeWorkspace, "activeworkspace");
            }
        }
    }
}
