// The selection action menu's surface (TASK-18).
//
// Two states on one layer surface: a compact chip that appears when a selection
// settles, and the action list it expands into on tap. Apple's shape, and the
// one the protocol allows — see Selection.qml for why we get the selected TEXT
// but never its bounding rectangle, and therefore why this anchors to the
// pointer rather than to the highlight.
//
// TASK-18's priority is explicit: the surface must be solid before any backend
// is wired, because "a menu whose actions are stubs is acceptable; a janky menu
// is not." So Read Aloud (the one action with a live backend) is real and the
// rest declare themselves unimplemented rather than failing silently.
//
// ── Z-order, settled here ──────────────────────────────────────────────────
// WlrLayer.Overlay, following DialHost: it puts the surface above the OSK's
// layer entirely instead of competing inside Top, which is the collision that
// made the stevia pill/menu bug (TASK-17). keyboardFocus stays None throughout
// — taking focus would disturb the source app that owns the selection, and an
// Exclusive grab is the documented way to stop touch reaching the layer below
// (tried on the polkit surface 2026-07-26). Every action here is a tap.
//
// The input mask is load-bearing. This window covers the screen so the chip can
// be placed anywhere, but `mask: Region { item: ... }` narrows the input region
// to the visible card so taps elsewhere reach the app underneath. Note the trap
// documented in OnScreenKeyboard.qml: `Region { item: null }` does NOT mean
// "no input" — it behaves like a full-window region and silently eats taps.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import qs
import qs.services
import qs.modules.common

Scope {
    id: scope

    // Expanded = the action list is showing. Reset whenever the selection goes.
    property bool expanded: false

    Connections {
        target: Selection
        function onSelectionSettled(text, x, y) {
            // A new selection always re-collapses: the previous expansion was
            // about the previous text.
            scope.expanded = false;
        }
        function onSelectionCleared() {
            scope.expanded = false;
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: win
            required property var modelData
            screen: win.modelData

            // Sized to the card and positioned with margins, NOT a fullscreen
            // shield with an input mask. The masked-fullscreen version shipped
            // first and the chip was completely untappable on device; rather
            // than keep guessing at the mask, this removes the question — every
            // pixel of the window IS the chip, so there is nothing to mask and
            // no invisible surface over the rest of the screen to fight the
            // app's own selection UI (Firefox's popup, notably).
            anchors { top: true; left: true }
            margins { left: Math.round(win.targetX); top: Math.round(win.targetY) }
            implicitWidth: card.implicitWidth
            implicitHeight: card.implicitHeight

            color: "transparent"
            visible: Selection.hasSelection
            WlrLayershell.namespace: "souveraine:selection"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            exclusionMode: ExclusionMode.Ignore

            // Selection gives a pointer hint — where the finger lifted. -1 means
            // it could not be resolved, so fall back to thumb-reachable centre.
            // Clamped against the screen, not the window: the window is now the
            // card's size, so its own width/height cannot bound the placement.
            readonly property int pad: 12
            readonly property int hintX: Selection.anchorX
            readonly property int hintY: Selection.anchorY
            readonly property int scrW: win.screen ? win.screen.width : 1080
            readonly property int scrH: win.screen ? win.screen.height : 2160

            readonly property real targetX: {
                if (win.hintX < 0) return Math.max(win.pad, (win.scrW - card.implicitWidth) / 2);
                return Math.max(win.pad,
                    Math.min(win.hintX - card.implicitWidth / 2,
                             win.scrW - card.implicitWidth - win.pad));
            }
            readonly property real targetY: {
                if (win.hintY < 0) return win.scrH * 0.72;
                // Prefer above the touch point so the finger does not cover it;
                // flip below when there is no room up there.
                const above = win.hintY - card.implicitHeight - 16;
                if (above >= win.pad) return above;
                return Math.min(win.hintY + 16,
                                win.scrH - card.implicitHeight - win.pad);
            }

            // `card` takes its size from the layout's implicit size, and the
            // layout takes its size from its own — no path where a child's size
            // depends on the parent's. The earlier version centered the layout
            // in the card while the card sized from the layout, which resolved
            // to zero and silently made `mask` a zero-size input region: the
            // chip painted but nothing could be tapped. The background stays a
            // plain Item child, because anchors on a layout-managed item are
            // undefined behavior.
            // The card fills the window now; the window's margins do the moving,
            // so there is no x/y to animate here.
            Item {
                id: card
                anchors.fill: parent
                implicitWidth: content.implicitWidth
                implicitHeight: content.implicitHeight

                Rectangle {
                    anchors.fill: parent
                    z: -1
                    radius: Appearance.rounding.normal
                    color: Appearance.colors.colLayer1
                    border.width: 1
                    border.color: Appearance.colors.colLayer2
                }

                ColumnLayout {
                    id: content
                    anchors.left: parent.left
                    anchors.top: parent.top
                    width: implicitWidth
                    height: implicitHeight
                    spacing: 0

                // ── collapsed: the chip ──────────────────────────────────
                SelectionChip {
                    visible: !scope.expanded
                    charCount: Selection.text.length
                    onTapped: {
                        DeviceEvidence.touched("selection-menu");
                        Haptics.trigger("button-pressed");
                        scope.expanded = true;
                    }
                    onDismissed: {
                        DeviceEvidence.touched("selection-menu");
                        Selection.dismiss();
                    }
                }

                // ── expanded: the actions ────────────────────────────────
                SelectionActions {
                    visible: scope.expanded
                    onActed: {
                        DeviceEvidence.touched("selection-menu");
                        Selection.dismiss();
                    }
                }
                }
            }
        }
    }
}
