// Aspect-aware wallpaper selection for unlike display shapes. This retains the
// original as canonical and selects only explicit local derivatives.
pragma Singleton

import QtQuick
import Quickshell
import qs.modules.common

Singleton {
    id: root

    function aspectFor(screen) {
        if (!screen || !screen.height) return 1;
        return screen.width / screen.height;
    }

    function pathFor(screen) {
        const aspect = root.aspectFor(screen);
        const portrait = Config.options.background.portraitVariantPath;
        const landscape = Config.options.background.landscapeVariantPath;
        if (aspect < 0.9 && portrait) return portrait;
        if (aspect > 1.1 && landscape) return landscape;
        return Config.options.background.wallpaperPath;
    }

    function focalPoint() {
        return {
            x: Math.max(0, Math.min(1, Config.options.background.wallpaperFocalX)),
            y: Math.max(0, Math.min(1, Config.options.background.wallpaperFocalY))
        };
    }
}
