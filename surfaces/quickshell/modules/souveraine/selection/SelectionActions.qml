// The expanded state: the action list.
//
// Grouping follows TASK-18's sections, and its stated default for progressive
// disclosure: Read Aloud and the agent actions at this level, Reference behind
// "More…" because the full set is long for a phone screen.
//
// Tiers (SESSION-AUTHORITY §2) are enforced here rather than assumed. The whole
// surface is already unreachable on the lock screen — Selection.qml kills its
// watcher unless the session is genuinely unlocked — but each action still
// declares its own tier so the rule is legible where the action lives, and so
// this file stays correct if the surface is ever reached another way.
//
// An action with no backend says so. TASK-18: "A menu whose actions are stubs is
// acceptable; a janky menu is not" — but a stub that looks live and does nothing
// is worse than either, so `reason` is shown rather than a dead tap.
import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import qs
import qs.services
import qs.modules.common

ColumnLayout {
    id: root

    signal acted()

    property bool showReference: false

    readonly property string tierAmbient: LockContentPolicy.ambient
    readonly property string tierPersonal: LockContentPolicy.personal

    implicitWidth: 264
    spacing: 0

    // ── Read Aloud — the one live backend ────────────────────────────────
    // The literal selected text, no agent ingestion. Routes through
    // Speech.speak(), which reads the who->voice mapping from souveraine's
    // /v1/config, so the shell still picks no voice of its own.
    SelectionAction {
        icon: "volume_up"
        label: Translation.tr("Read aloud")
        tier: root.tierAmbient
        enabled: Config.options.selection.readAloud && Speech.enabled
        reason: !Config.options.selection.readAloud
            ? Translation.tr("Turned off in Settings")
            : Translation.tr("Enable TTS in Settings → Speech")
        onTriggered: {
            Speech.speak(Selection.text);
            root.acted();
        }
    }

    SelectionSeparator { visible: Config.options.selection.agentActions }

    // ── Agent actions ────────────────────────────────────────────────────
    // Both reach conversation history, which is `personal`. TASK-18 §2 is the
    // spec for the distinction and it is the same one the OCR pipeline wants:
    // one seeds a side-shoot, the other appends to the primary conversation.
    SelectionAction {
        visible: Config.options.selection.agentActions
        icon: "forum"
        label: Translation.tr("Talk about this")
        sublabel: Translation.tr("Starts a side conversation")
        tier: root.tierPersonal
        enabled: false
        reason: Translation.tr("Needs the side-shoot seam in Ai.qml")
        onTriggered: root.acted()
    }

    SelectionAction {
        visible: Config.options.selection.agentActions
        icon: "add_comment"
        label: Translation.tr("Add to our conversation")
        sublabel: Translation.tr("Appends to the current thread")
        tier: root.tierPersonal
        enabled: false
        reason: Translation.tr("Needs the primary-conversation seam in Ai.qml")
        onTriggered: root.acted()
    }

    SelectionSeparator {}

    // ── Utility ──────────────────────────────────────────────────────────
    SelectionAction {
        icon: "content_copy"
        label: Translation.tr("Copy")
        tier: root.tierAmbient
        // The selection is already in the primary buffer; this promotes it to
        // the clipboard, which is what a phone user means by "copy".
        onTriggered: {
            copyProc.command = ["sh", "-c",
                "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1} " +
                "wl-paste --primary --no-newline | wl-copy"];
            copyProc.running = true;
            root.acted();
        }
    }

    SelectionSeparator { visible: Config.options.selection.referenceActions }

    // ── Reference, behind one more tap ───────────────────────────────────
    SelectionAction {
        visible: Config.options.selection.referenceActions && !root.showReference
        icon: "more_horiz"
        label: Translation.tr("More…")
        tier: root.tierAmbient
        onTriggered: root.showReference = true
    }

    // TASK-18's default stance, unchanged: strictly local, never the agent.
    // None of these have a local backend on the device yet, so all three say so.
    Repeater {
        model: root.showReference && Config.options.selection.referenceActions
            ? [
                { icon: "book_2", label: Translation.tr("Define") },
                { icon: "translate", label: Translation.tr("Translate") },
                { icon: "search", label: Translation.tr("Search the web") },
            ]
            : []

        SelectionAction {
            required property var modelData
            icon: modelData.icon
            label: modelData.label
            tier: root.tierAmbient
            enabled: false
            reason: Translation.tr("No local backend yet")
            onTriggered: root.acted()
        }
    }

    Process { id: copyProc }
}
