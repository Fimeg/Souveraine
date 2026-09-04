// Crash surfacing (TASK-05) — the other half of the supervision work in
// souveraine-shell.service. Supervision journals and restarts; this makes a
// crash *reported*: a notification through the org.freedesktop.Notifications
// pipe, which fans to banner unlocked / lock card locked / NotifyEvents.
//
// This runs inside the shell on purpose. When qs dies, systemd resurrects it
// and the fresh instance finds the new crashes.log line and announces its own
// recovery. The 8-retry give-up case has no shell to banner from — sessiond's
// fail-closed lock is that surface, not us.
//
// Emission is a real notify-send, not a shortcut into the Notifications
// singleton: the crash report exercises the same D-Bus path every other
// client uses, so a broken pipe is itself detected.
//
// Sources watched:
//   1. ~/.local/state/souveraine/crashes.log — inotify via FileView, no poll.
//   2. systemctl --user --failed            — 30s timer (doc: no tight loops).
//   3. coredumpctl list tail                — same 30s tick.
// Dedupe state persists in crash-reporter.state so a crash is reported once
// across shell restarts, not once per resurrection.
pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    property string stateDir: Quickshell.env("HOME") + "/.local/state/souveraine"
    property bool started: false

    // Persisted dedupe cursors.
    property int crashLinesSeen: -1        // -1 = state not loaded yet
    property var knownFailedUnits: []
    property string lastCoredumpLine: ""

    function start() {
        if (root.started) return;
        root.started = true;
        stateFile.reload();
    }

    function notify(summary, body, urgency) {
        Quickshell.execDetached(["notify-send", "-a", "souveraine",
            "-u", urgency ?? "critical", summary, body ?? ""]);
    }

    function saveState() {
        stateFile.setText(JSON.stringify({
            crashLinesSeen: root.crashLinesSeen,
            knownFailedUnits: root.knownFailedUnits,
            lastCoredumpLine: root.lastCoredumpLine,
        }));
    }

    FileView {
        id: stateFile
        path: root.stateDir + "/crash-reporter.state"
        onLoaded: {
            try {
                const s = JSON.parse(text());
                root.crashLinesSeen = s.crashLinesSeen ?? 0;
                root.knownFailedUnits = s.knownFailedUnits ?? [];
                root.lastCoredumpLine = s.lastCoredumpLine ?? "";
            } catch (e) {
                root.crashLinesSeen = 0;
            }
            crashLog.reload();
        }
        onLoadFailed: {
            // First run: baseline everything as seen so a fresh deploy does
            // not storm about history; only new events report.
            root.crashLinesSeen = -2;
            crashLog.reload();
        }
    }

    // Set once we have successfully read the log, so a later failure is
    // distinguishable from "it has never existed".
    property bool _crashLogSeen: false
    property bool _crashLogFailureReported: false

    FileView {
        id: crashLog
        path: root.stateDir + "/crashes.log"
        watchChanges: true
        // No crashes.log yet — nothing has ever crashed. The watcher can't
        // watch a nonexistent file, so the 30s tick retries the reload, and
        // each retry printed a "Read of ... failed" warning. On a healthy
        // 16h session that was 1293 lines — 78% of everything in the shell
        // log, drowning the file we read to verify our own changes.
        //
        // Absence is the *expected* state here, so it is silenced. But it is
        // not silenced blind: onLoadFailed still fires, and a failure after
        // we have once read the file successfully is a real fault and says
        // so — once, not every 30 seconds.
        printErrors: false
        onFileChanged: reload()
        onLoaded: {
            root._crashLogSeen = true;
            root._crashLogFailureReported = false;
            root.consumeCrashLog();
        }
        onLoadFailed: {
            if (root._crashLogSeen && !root._crashLogFailureReported) {
                root._crashLogFailureReported = true;
                console.log("[CrashReporter] crashes.log became unreadable at",
                            crashLog.path, "— crash reporting is blind until it returns");
            }
        }
    }

    function consumeCrashLog() {
        if (root.crashLinesSeen === -1) return; // state not loaded yet
        const lines = crashLog.text().split("\n").filter(l => l.trim().length > 0);
        if (root.crashLinesSeen === -2) {       // first-run baseline
            root.crashLinesSeen = lines.length;
            root.saveState();
            return;
        }
        if (lines.length < root.crashLinesSeen) root.crashLinesSeen = 0; // rotated
        if (lines.length === root.crashLinesSeen) return;
        const fresh = lines.slice(root.crashLinesSeen);
        root.crashLinesSeen = lines.length;
        root.saveState();
        // "$(date -Is) souveraine-shell result=exit-code exit=255"
        const last = fresh[fresh.length - 1];
        const detail = last.replace(/^\S+\s+/, "");
        root.notify(
            fresh.length > 1
                ? qsTr("Shell crashed ×%1 — recovered").arg(fresh.length)
                : qsTr("Shell crashed — recovered"),
            detail);
    }

    Timer {
        interval: 30000
        running: root.started
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (crashLog.path && !crashLog.loaded) crashLog.reload();
            failedUnits.running = true;
            coredumps.running = true;
        }
    }

    Process {
        id: failedUnits
        command: ["systemctl", "--user", "--failed", "--plain", "--no-legend"]
        stdout: StdioCollector {
            onStreamFinished: {
                const units = text.split("\n")
                    .map(l => l.trim().split(/\s+/)[0])
                    .filter(u => u.length > 0)
                    // Our own unit's crashes come from crashes.log with detail;
                    // it also can't be in --failed while we are running.
                    .filter(u => u !== "souveraine-shell.service");
                const fresh = units.filter(u => !root.knownFailedUnits.includes(u));
                root.knownFailedUnits = units;
                if (root.crashLinesSeen === -1) return; // still booting state
                root.saveState();
                if (fresh.length > 0) {
                    root.notify(
                        qsTr("Service failed: %1").arg(fresh.join(", ")),
                        qsTr("systemctl --user status for details"));
                }
            }
        }
    }

    Process {
        id: coredumps
        command: ["sh", "-c", "coredumpctl list --no-legend 2>/dev/null | tail -1"]
        stdout: StdioCollector {
            onStreamFinished: {
                const line = text.trim();
                if (line.length === 0) return;
                if (root.crashLinesSeen === -1) return;
                if (line === root.lastCoredumpLine) return;
                const first = root.lastCoredumpLine.length === 0;
                root.lastCoredumpLine = line;
                root.saveState();
                if (first) return; // baseline, don't report history
                // "... TIME PID UID GID SIG COREFILE EXE SIZE"
                const cols = line.split(/\s+/);
                const exe = cols.length >= 2 ? cols[cols.length - 2] : "?";
                root.notify(qsTr("Process dumped core: %1").arg(exe), line, "normal");
            }
        }
    }
}
