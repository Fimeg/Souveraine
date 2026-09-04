pragma Singleton

import Quickshell

/*
 * The Panel's vocabulary for Souveraine's sensors.
 *
 * Transcribed from src/ui/chat/tool_renderers.rs — the TUI solved this once
 * already, for the same event stream. Where a summary here differs from the
 * Rust, the Rust is right and this is a bug.
 *
 * The vendor snapshot's ToolCallBlock knew 8 tools: bash, read, write, edit,
 * grep, glob, list_dir, memory. Those are the tools any coding agent has.
 * The 11 it could not name — outfit, nickname, subagent, atmosphere, reach,
 * consult, itinerary, todo, schedule, halt, intrusive — are precisely the
 * ones that make her someone rather than something. They rendered as
 * anonymous grey blocks.
 *
 * Kinds group by what the act *is*, not by which subsystem implements it:
 *   shell     — she acts on the machine
 *   sensor    — she looks
 *   file      — she changes something durable outside herself
 *   memory    — she changes herself
 *   presence  — how she appears, who she is with
 *   intent    — what she has committed to, where she is in it
 *   reach     — she addresses someone else
 *   control   — she interrupts her own flow
 */
Singleton {
    id: root

    readonly property var kinds: ({
        "bash": "shell",
        "read": "sensor",
        "glob": "sensor",
        "grep": "sensor",
        "list_dir": "sensor",
        "write": "file",
        "edit": "file",
        "memory": "memory",
        "outfit": "presence",
        "atmosphere": "presence",
        "nickname": "presence",
        "todo": "intent",
        "itinerary": "intent",
        "schedule": "intent",
        "reach": "reach",
        "consult": "reach",
        "subagent": "reach",
        "halt": "control",
        "intrusive": "control"
    })

    readonly property var icons: ({
        "bash": "terminal",
        "read": "article",
        "glob": "folder_open",
        "grep": "manage_search",
        "list_dir": "folder",
        "write": "edit_note",
        "edit": "edit_note",
        "memory": "psychology",
        "outfit": "checkroom",
        "atmosphere": "palette",
        "nickname": "badge",
        "todo": "checklist",
        "itinerary": "route",
        "schedule": "schedule",
        "reach": "hub",
        "consult": "forum",
        "subagent": "account_tree",
        "halt": "pan_tool",
        "intrusive": "bolt"
    })

    function kindOf(name) {
        return root.kinds[String(name ?? "")] ?? "unknown";
    }

    function iconOf(name) {
        return root.icons[String(name ?? "")] ?? "sensors";
    }

    // An unknown tool is a real event, not a defect to hide. It renders as
    // itself with a generic icon — and it is legible as unknown, which is how
    // a new sensor announces that this file needs a line.
    function isKnown(name) {
        return root.kinds[String(name ?? "")] !== undefined;
    }

    function clip(text, max) {
        const value = String(text ?? "");
        return value.length > max ? value.slice(0, Math.max(0, max - 1)) + "…" : value;
    }

    function parseArguments(raw) {
        try {
            const parsed = JSON.parse(String(raw ?? "{}"));
            return parsed && typeof parsed === "object" ? parsed : {};
        } catch (error) {
            return {};
        }
    }

    function str(args, key) {
        const value = args[key];
        return (typeof value === "string" && value.length > 0) ? value : null;
    }

    // Every key: value pair, joined. The fallback, matching summarize_generic.
    function generic(args) {
        const parts = [];
        for (const key in args) {
            const value = args[key];
            const text = (typeof value === "string") ? root.clip(value, 60) : root.clip(JSON.stringify(value), 60);
            parts.push(key + ": " + text);
        }
        return parts.join("  ·  ");
    }

    /*
     * One line describing what this call actually did.
     * Mirrors summarize_tool_args(). Falls through to generic() exactly where
     * the Rust does — including for a tool whose expected field is missing,
     * which is a real case and must not render as empty.
     */
    function summarize(name, rawArguments) {
        const args = root.parseArguments(rawArguments);
        const tool = String(name ?? "");

        switch (tool) {
        case "bash": {
            const command = root.str(args, "command");
            if (command === null) return root.generic(args);
            return "$ " + root.clip(command, 80) + (args.run_in_background === true ? " [bg]" : "");
        }
        case "read": {
            const path = root.str(args, "path");
            if (path === null) return root.generic(args);
            return root.clip(path, 60) + (args.force === true ? " [force]" : "");
        }
        case "write": {
            const path = root.str(args, "path");
            if (path === null) return root.generic(args);
            const mode = root.str(args, "mode");
            return (mode === "append" ? "append → " : "write → ") + root.clip(path, 60);
        }
        case "edit": {
            const path = root.str(args, "path");
            if (path === null) return root.generic(args);
            return "edit → " + root.clip(path, 60) + (args.replace_all === true ? " [all]" : "");
        }
        case "grep": {
            const pattern = root.str(args, "pattern");
            if (pattern === null) return root.generic(args);
            const where = root.str(args, "path");
            return "\"" + root.clip(pattern, 48) + "\"" + (where !== null ? " in " + root.clip(where, 30) : "");
        }
        case "glob": {
            const pattern = root.str(args, "pattern");
            return pattern === null ? root.generic(args) : root.clip(pattern, 80);
        }
        case "list_dir":
            return root.clip(root.str(args, "path") ?? ".", 80);
        case "memory": {
            const command = root.str(args, "command") ?? "list";
            const path = root.str(args, "path");
            return path === null ? command : command + " " + root.clip(path, 40);
        }
        case "todo": {
            const action = root.str(args, "action") ?? "list";
            if (action === "list") return "list";
            const what = root.str(args, "text") ?? root.str(args, "id");
            return what === null ? action : action + ": " + root.clip(what, 50);
        }
        case "itinerary": {
            const action = root.str(args, "action") ?? "describe";
            let title = root.str(args, "title");
            if (title === null && Array.isArray(args.stops) && args.stops.length > 0) {
                const first = args.stops[0];
                // A stop is an object with a name; older callers passed a bare string.
                title = (first && typeof first === "object") ? (first.name ?? null) : (typeof first === "string" ? first : null);
            }
            return title === null ? action : action + ": " + root.clip(title, 40);
        }
        case "schedule": {
            const action = root.str(args, "action") ?? "list";
            const named = root.str(args, "name");
            return named === null ? action : action + ": " + root.clip(named, 30);
        }
        case "nickname": {
            const action = root.str(args, "action") ?? "get";
            const named = root.str(args, "name");
            return named === null ? action : action + ": " + root.clip(named, 30);
        }
        case "atmosphere":
        case "outfit": {
            const named = root.str(args, "name");
            // An empty name is meaningful for both: it is a return to default.
            if (named === null) return (args.name === "") ? "default" : root.generic(args);
            return named;
        }
        case "subagent": {
            const prompt = root.str(args, "prompt");
            const subType = root.str(args, "subagent_type");
            let out = "";
            if (args.run_in_background === true) out += "[bg] ";
            if (subType !== null && subType !== "general-purpose") out += "[" + subType + "] ";
            if (prompt !== null) out += root.clip(prompt, 50);
            return out.length > 0 ? out : root.generic(args);
        }
        case "reach":
        case "consult": {
            const target = root.str(args, "target");
            return target === null ? root.generic(args) : root.clip(target, 40);
        }
        default:
            return root.generic(args);
        }
    }
}
