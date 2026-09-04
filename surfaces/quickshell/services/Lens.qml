// The garden on the limb — the vault's micro-view service.
//
// One more process owned the Face way: this holds a `souveraine-lens`
// process with `--ipc` at a socket only the shell ever touches. The lens
// process has no connection to the server, and this service has no
// connection to the vault — the micro-state the widget renders arrives
// over the same line protocol the lens already speaks, one JSON object
// per line on stdout. The seam stays: nobody reads the vault but the lens
// and the user.
//
// `joined` is the whole state, as with the face. While it is false the
// garden window is not on glass and no webview holds memory; the widget
// shows a cold card until the user opens it.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import qs
import qs.modules.common

Singleton {
    id: root

    property bool joined: false
    property string socketPath: Quickshell.env("XDG_RUNTIME_DIR") + "/souveraine/lens.sock"
    property string vaultDir: Quickshell.env("HOME") + "/vault"
    readonly property string vaultRel: vaultDir.replace(Quickshell.env("HOME"), "~")

    // The micro-state, exactly as the lens reports it.
    property var notes: []
    property int noteCount: 0
    property int dirty: 0
    property int ahead: 0
    property int behind: 0
    property string head: ""
    property string branch: ""
    property bool hasRemote: true
    property string lastError: ""

    // cold until the first state answers; then one of clean / ahead /
    // behind / diverged / dirty — computed, so there is one place the
    // widget reads.
    property string syncState: "cold"
    readonly property string stateText: {
        if (!root.joined) return "offline";
        switch (root.syncState) {
        case "clean": return root.noteCount + " notes · clean";
        case "ahead": return root.noteCount + " notes · push " + root.ahead;
        case "behind": return root.noteCount + " notes · pull " + root.behind;
        case "diverged": return root.noteCount + " notes · push " + root.ahead + " pull " + root.behind;
        case "dirty": return root.noteCount + " notes · " + root.dirty + " uncommitted";
        default: return root.noteCount + " notes";
        }
    }

    onJoinedChanged: {
        if (root.joined)
            root.requestState();
    }

    function join() {
        if (root.joined)
            return;
        host.running = true;
        root.joined = true;
    }

    function leave() {
        if (!root.joined)
            return;
        root._send({ op: "quit" });
        host.running = false;
        root.joined = false;
        root.syncState = "cold";
        root.notes = [];
        root.noteCount = 0;
    }

    function toggle() {
        if (root.joined)
            root.leave();
        else
            root.join();
    }

    function requestState() {
        root._send({ op: "state" });
    }

    function openNote(rel) {
        root._send({ op: "open", rel: rel });
    }

    function createNote(title) {
        root._send({ op: "create", title: title });
    }

    // The page owns its sync flow; the shell only pulls the trigger.
    function syncNow() {
        root._eval("window.__lens.sync()");
    }

    function _eval(script) {
        root._send({ op: "eval", script: script });
    }

    function _send(msg) {
        if (sock.connected)
            sock.write(JSON.stringify(msg) + "\n");
    }

    Process {
        id: host
        command: ["souveraine-lens",
            "--vault", root.vaultDir,
            "--ipc", root.socketPath,
            "--app-id", "org.souveraine.lens",
            "--title", "Garden"]
        stdout: SplitParser {
            splitMarker: "\n"
            onRead: line => {
                let msg;
                try {
                    msg = JSON.parse(line);
                } catch (e) {
                    return;
                }
                if (msg.event === "note_created") {
                    const rel = msg.rel ?? "";
                    const title = rel.replace(/\.md$/, "").split("/").pop();
                    Quickshell.execDetached(["notify-send", "-a", "Garden", "Note planted", title]);
                }
                else if (msg.event === "synced") {
                    Quickshell.execDetached(["notify-send", "-a", "Garden", "Garden synced"]);
                }

                if (msg.event === "state" && msg.state) {
                    const st = msg.state.status ?? {};
                    root.notes = msg.state.notes ?? [];
                    root.noteCount = st.note_count ?? root.notes.length;
                    root.dirty = st.dirty ?? 0;
                    root.ahead = st.ahead ?? 0;
                    root.behind = st.behind ?? 0;
                    root.head = st.head ?? "";
                    root.branch = st.branch ?? "";
                    root.hasRemote = !!st.remote;
                    root.syncState = root.dirty > 0
                        ? "dirty"
                        : root.ahead > 0 && root.behind > 0
                            ? "diverged"
                            : root.ahead > 0 ? "ahead"
                            : root.behind > 0 ? "behind" : "clean";
                }
                else if (msg.event === "synced")
                    refreshTimer.restart();
                else if (msg.event === "note_opened" || msg.event === "note_created" || msg.event === "note_saved")
                    refreshTimer.restart();
                else if (msg.event === "console")
                    console.log("[lens]", msg.level ?? "", msg.text ?? "");
                else
                    console.log("[lens]", line);
            }
        }
        onExited: {
            root.joined = false;
            root.syncState = "cold";
        }
    }

    // One refresh after a burst of events, not one per line.
    Timer {
        id: refreshTimer
        interval: 300
        repeat: false
        onTriggered: root.requestState()
    }

    // The garden's heartbeat while it is joined; cheap — one scan.
    Timer {
        id: pollTimer
        running: root.joined
        interval: 60000
        repeat: true
        triggeredOnStart: true
        onTriggered: root.requestState()
    }

    Socket {
        id: sock
        path: root.socketPath
        connected: root.joined

        onConnectionStateChanged: {
            if (sock.connected)
                root.requestState();
        }
    }

    // Reachable by name: the agent summons the garden the same way it
    // summons the face, without guessing.
    IpcHandler {
        target: "lens"

        function open(rel: string): void {
            if (!root.joined)
                root.join();
            root.openNote(rel);
        }

        function create(title: string): void {
            if (!root.joined)
                root.join();
            root.createNote(title);
        }

        function sync(): void {
            root.syncNow();
        }

        function join(): void {
            root.join();
        }

        function leave(): void {
            root.leave();
        }

        function status(): string {
            return JSON.stringify({
                joined: root.joined,
                syncState: root.syncState,
                noteCount: root.noteCount,
                head: root.head
            });
        }
    }
}
