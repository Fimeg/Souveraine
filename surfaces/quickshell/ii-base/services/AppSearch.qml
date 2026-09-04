pragma Singleton

import qs.modules.common
import qs.modules.common.functions
import Quickshell

/**
 * - Eases fuzzy searching for applications by name
 * - Guesses icon name for window class name
 */
Singleton {
    id: root
    property bool sloppySearch: Config.options?.search.sloppy ?? false
    property real scoreThreshold: 0.2
    property var substitutions: ({
        "code-url-handler": "visual-studio-code",
        "Code": "visual-studio-code",
        "gnome-tweaks": "org.gnome.tweaks",
        "pavucontrol-qt": "pavucontrol",
        "wps": "wps-office2019-kprometheus",
        "wpsoffice": "wps-office2019-kprometheus",
        "footclient": "foot",
    })
    property var regexSubstitutions: [
        {
            "regex": /^steam_app_(\d+)$/,
            "replace": "steam_icon_$1"
        },
        {
            "regex": /Minecraft.*/,
            "replace": "minecraft"
        },
        {
            "regex": /.*polkit.*/,
            "replace": "system-lock-screen"
        },
        {
            "regex": /gcr.prompter/,
            "replace": "system-lock-screen"
        }
    ]

    // Deduped list to fix double icons
    readonly property list<DesktopEntry> list: Array.from(DesktopEntries.applications.values)
        .filter((app, index, self) => 
            index === self.findIndex((t) => (
                t.id === app.id
            ))
    )
    
    readonly property var preppedNames: list.map(a => ({
        name: Fuzzy.prepare(`${a.name} `),
        entry: a
    }))

    readonly property var preppedIcons: list.map(a => ({
        name: Fuzzy.prepare(`${a.icon} `),
        entry: a
    }))

    function fuzzyQuery(search: string): var { // Idk why list<DesktopEntry> doesn't work
        if (root.sloppySearch) {
            return root.levenshteinQuery(search);
        }

        return Fuzzy.go(search, preppedNames, {
            all: true,
            key: "name"
        }).map(r => {
            return r.obj.entry
        });
    }

    function levenshteinQuery(search: string): var {
        const prepared = BitwiseFuzzy.prepare(search);
        return BitwiseFuzzy.search(prepared, list, { key: "name", threshold: root.scoreThreshold });
    }

    function iconExists(iconName) {
        if (!iconName || iconName.length == 0) return false;
        return (Quickshell.iconPath(iconName, true).length > 0) 
            && !iconName.includes("image-missing");
    }

    function getReverseDomainNameAppName(str) {
        return str.split('.').slice(-1)[0]
    }

    function getKebabNormalizedAppName(str) {
        return str.toLowerCase().replace(/\s+/g, "-");
    }

    function getUndescoreToKebabAppName(str) {
        return str.toLowerCase().replace(/_/g, "-");
    }

    /**
     * Resolve a window app_id / pinned appId to its DesktopEntry.
     *
     * DesktopEntries.heuristicLookup() matches the entry id and
     * StartupWMClass, which covers most apps but NOT the ones that append an
     * instance suffix to their Wayland app_id. Firefox is the reference case:
     * it derives its remoting name from the profile, so the window reports
     * "firefox-default" while the entry is "firefox" with
     * StartupWMClass=firefox. heuristicLookup returns null, and a dock tap on
     * a pinned-but-not-running app then calls execute() on null — a silent
     * dead tap that reports no error anywhere.
     *
     * So: fall back to trimming trailing "-segment" pieces, longest match
     * first, then to a StartupWMClass prefix scan. Returns null only when
     * nothing in the entry set plausibly owns the id.
     */
    function resolveEntry(appId) {
        if (!appId || appId.length == 0) return null;

        const direct = DesktopEntries.heuristicLookup(appId);
        if (direct) return direct;

        // "firefox-default" -> "firefox"; "signal-desktop-beta" -> "signal-desktop"
        let candidate = appId;
        while (candidate.includes("-")) {
            candidate = candidate.slice(0, candidate.lastIndexOf("-"));
            const trimmed = DesktopEntries.heuristicLookup(candidate);
            if (trimmed) return trimmed;
        }

        // Last resort: an entry whose StartupWMClass prefixes the app_id.
        const lowered = appId.toLowerCase();
        for (const entry of root.list) {
            const wmClass = entry.startupClass;
            if (!wmClass || wmClass.length == 0) continue;
            if (lowered.startsWith(wmClass.toLowerCase())) return entry;
        }

        return null;
    }

    function guessIcon(str) {
        if (!str || str.length == 0) return "image-missing";

        // Quickshell's desktop entry lookup
        const entry = DesktopEntries.byId(str);
        if (entry) return entry.icon;

        // Normal substitutions
        if (substitutions[str]) return substitutions[str];
        if (substitutions[str.toLowerCase()]) return substitutions[str.toLowerCase()];

        // Regex substitutions
        for (let i = 0; i < regexSubstitutions.length; i++) {
            const substitution = regexSubstitutions[i];
            const replacedName = str.replace(
                substitution.regex,
                substitution.replace,
            );
            if (replacedName != str) return replacedName;
        }

        // Icon exists -> return as is
        if (iconExists(str)) return str;


        // Simple guesses
        const lowercased = str.toLowerCase();
        if (iconExists(lowercased)) return lowercased;

        const reverseDomainNameAppName = getReverseDomainNameAppName(str);
        if (iconExists(reverseDomainNameAppName)) return reverseDomainNameAppName;

        const lowercasedDomainNameAppName = reverseDomainNameAppName.toLowerCase();
        if (iconExists(lowercasedDomainNameAppName)) return lowercasedDomainNameAppName;

        const kebabNormalizedGuess = getKebabNormalizedAppName(str);
        if (iconExists(kebabNormalizedGuess)) return kebabNormalizedGuess;

        const undescoreToKebabGuess = getUndescoreToKebabAppName(str);
        if (iconExists(undescoreToKebabGuess)) return undescoreToKebabGuess;

        // Search in desktop entries
        const iconSearchResults = Fuzzy.go(str, preppedIcons, {
            all: true,
            key: "name"
        }).map(r => {
            return r.obj.entry
        });
        if (iconSearchResults.length > 0) {
            const guess = iconSearchResults[0].icon
            if (iconExists(guess)) return guess;
        }

        const nameSearchResults = root.fuzzyQuery(str);
        if (nameSearchResults.length > 0) {
            const guess = nameSearchResults[0].icon
            if (iconExists(guess)) return guess;
        }

        // Desktop entry lookup, suffix-tolerant so "firefox-default" and
        // friends land on their real entry's icon instead of falling through.
        const heuristicEntry = root.resolveEntry(str);
        if (heuristicEntry) return heuristicEntry.icon;

        // Give up
        return "application-x-executable";
    }
}
