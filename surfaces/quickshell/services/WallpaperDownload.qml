pragma Singleton

// Souveraine wallpaper download service. Owns fetching a random wallhaven
// image and handing it to the shell's wallpaper apply path. Repo-owned end to
// end: the script lives in this surface (scripts/wallpaper/), and applying
// calls the shell-owned Wallpapers singleton directly.
//
// Deliberately does NOT call ii's switchwall.sh directly: when the ii base is
// vendored away, only the `wallpapers` IPC target has to move with it; this
// service and its script are already ours.

import qs.modules.common
import qs.modules.common.functions
import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    // busy while a download is in flight — bind a button's enabled/spinner to it.
    property bool downloading: false
    // last error string for the UI (empty = ok).
    property string lastError: ""

    // Purity flags — which wallhaven categories the random pick may include.
    // Sourced from ~/.config/illogical-impulse/config.json (background.wallhaven
    // .purity, a CSV of sfw/sketchy/nsfw), the same place the download script
    // reads. Not in the Config.options schema upstream ships, so we read/write
    // the JSON directly rather than binding a schema key that doesn't exist.
    property bool puritySfw: true
    property bool puritySketchy: false
    property bool purityNsfw: false

    readonly property string configPath:
        FileUtils.trimFileProtocol(Quickshell.env("HOME") + "/.config/illogical-impulse/config.json")

    // The CSV the script's [purity] arg expects, or "sfw" if nothing is on
    // (never fetch an empty purity — that would 400 the API).
    readonly property string purityCsv: {
        const parts = [];
        if (puritySfw) parts.push("sfw");
        if (puritySketchy) parts.push("sketchy");
        if (purityNsfw) parts.push("nsfw");
        return parts.length > 0 ? parts.join(",") : "sfw";
    }

    signal downloaded(string path)
    signal failed(string message)

    readonly property string scriptPath:
        FileUtils.trimFileProtocol(Quickshell.shellPath("scripts/wallpaper/download_wallhaven.sh"))

    Component.onCompleted: readPurity.running = true

    // Load current purity from config.json into the three flags.
    Process {
        id: readPurity
        command: ["jq", "-r", ".background.wallhaven.purity // \"sfw,sketchy\"", root.configPath]
        stdout: StdioCollector {
            onStreamFinished: {
                const csv = text.trim();
                root.puritySfw = csv.indexOf("sfw") >= 0;
                root.puritySketchy = csv.indexOf("sketchy") >= 0;
                root.purityNsfw = csv.indexOf("nsfw") >= 0;
            }
        }
    }

    // Persist the current flags back to config.json (jq in-place via temp).
    function savePurity() {
        savePurityProc.exec(["bash", "-c",
            "f=" + Quickshell.env("HOME") + "/.config/illogical-impulse/config.json; " +
            "tmp=$(mktemp); jq --arg p " + JSON.stringify(root.purityCsv) +
            " '.background.wallhaven.purity = $p' \"$f\" > \"$tmp\" && mv \"$tmp\" \"$f\""]);
    }
    Process { id: savePurityProc }

    // Fetch one random wallpaper honoring the current purity flags.
    function download() {
        if (root.downloading) return;
        root.downloading = true;
        root.lastError = "";
        proc.stdoutText = "";
        proc.stderrText = "";
        proc.exec(["bash", root.scriptPath, root.purityCsv]);
    }

    Process {
        id: proc
        property string stdoutText: ""
        property string stderrText: ""
        stdout: StdioCollector { onStreamFinished: proc.stdoutText = text }
        stderr: StdioCollector { onStreamFinished: proc.stderrText = text }
        onExited: (exitCode, exitStatus) => {
            root.downloading = false;
            const path = proc.stdoutText.trim();
            if (exitCode === 0 && path.length > 0) {
                // Same in-process path as the picker: no nested qs instance.
                Wallpapers.apply(path);
                Config.options.background.wallpaperPath = path;
                root.downloaded(path);
            } else {
                const msg = proc.stderrText.trim() || Translation.tr("Download failed");
                root.lastError = msg;
                root.failed(msg);
            }
        }
    }
}
