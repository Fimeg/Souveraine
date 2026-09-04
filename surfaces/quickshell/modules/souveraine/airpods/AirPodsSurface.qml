// The admitted AirPods opening card.
//
// It is a projection, not an event source. sessiond supplies the admitted
// presentation through AccessoryPresentation; this surface never wakes the
// display, asks for input, or renders Personal-class fields while locked.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import qs
import qs.services
import qs.modules.common

Scope {
    id: scope

    property int dismissedGeneration: 0
    readonly property bool canShow: AccessoryPresentation.generation > dismissedGeneration
                                 && GlobalStates.displayActive
                                 && !GlobalStates.screenLockSecure
    // Preserve the focused desktop output under Hyprland. Membrane does not
    // publish a selected-output fact yet; on its one-panel device, choosing
    // Quickshell's first announced screen is the truthful fallback. Returning
    // no screen here creates no PanelWindow at all.
    readonly property var presentationScreen: Quickshell.screens.find(
        screen => screen.name === Hyprland.focusedMonitor?.name)
        ?? (Quickshell.screens.length > 0 ? Quickshell.screens[0] : null)

    Connections {
        target: AccessoryPresentation
        function onGenerationChanged() {
            // A new admitted edge supersedes the old card. It is not a request
            // to wake glass: an inactive display leaves it waiting for nothing.
            if (GlobalStates.displayActive && !GlobalStates.screenLockSecure)
                closeTimer.restart();
        }
    }

    Timer {
        id: closeTimer
        interval: 6200
        repeat: false
        onTriggered: scope.dismissedGeneration = AccessoryPresentation.generation
    }

    Variants {
        model: scope.presentationScreen ? [scope.presentationScreen] : []

        PanelWindow {
            id: win
            required property var modelData
            screen: modelData

            anchors { top: true; left: true; right: true; bottom: true }
            color: "transparent"
            visible: scope.canShow || card.opacity > 0
            WlrLayershell.namespace: "souveraine:airpods"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            exclusionMode: ExclusionMode.Ignore

            Item {
                id: card
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(parent.width - 36, 356)
                height: 292
                opacity: scope.canShow ? 1 : 0
                scale: scope.canShow ? 1 : 0.94

                Behavior on opacity { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }
                Behavior on scale { NumberAnimation { duration: 300; easing.type: Easing.OutBack } }

                Rectangle {
                    anchors.fill: parent
                    radius: 32
                    color: "#eb15171c"
                    border.width: 1
                    border.color: "#3ffffffF"
                }

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 1
                    radius: 31
                    gradient: Gradient {
                        GradientStop { position: 0; color: "#f42b3039" }
                        GradientStop { position: 1; color: "#f10f1014" }
                    }
                }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 23
                    spacing: 0

                    RowLayout {
                        Layout.fillWidth: true
                        Text {
                            text: AccessoryPresentation.label
                            color: "#f7f7f8"
                            font.pixelSize: 20
                            font.weight: Font.DemiBold
                        }
                        Item { Layout.fillWidth: true }
                        Text {
                            text: AccessoryPresentation.charging ? "charging" : "nearby"
                            color: "#aeb5bf"
                            font.pixelSize: 13
                        }
                    }

                    Item {
                        id: stage
                        Layout.fillWidth: true
                        Layout.preferredHeight: 157
                        Layout.topMargin: 3

                        property real open: 0
                        property int seenGeneration: -1
                        onVisibleChanged: if (visible) reveal.restart()
                        onSeenGenerationChanged: reveal.restart()

                        Connections {
                            target: AccessoryPresentation
                            function onGenerationChanged() {
                                stage.seenGeneration = AccessoryPresentation.generation;
                                stage.open = 0;
                                reveal.restart();
                            }
                        }

                        SequentialAnimation {
                            id: reveal
                            NumberAnimation { target: stage; property: "open"; to: 1; duration: 620; easing.type: Easing.OutQuart }
                        }

                        // Keep the object light, not illustrated. The pool
                        // gives the white case a place to sit without turning
                        // the card into a product render.
                        Rectangle {
                            width: 218; height: 36; radius: height / 2
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: 115
                            color: "#2c70e8"
                            opacity: 0.20 * stage.open
                            scale: 0.76 + 0.24 * stage.open
                            Behavior on opacity { NumberAnimation { duration: 420 } }
                        }

                        Item {
                            id: caseArt
                            width: 190; height: 146
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.bottom: parent.bottom

                            // The old 3-D lid was a flat white slab in a real
                            // layer surface. This is deliberately a clear 2-D
                            // silhouette: lid lifts on its hinge, buds clear
                            // the cavity, then the front of the case settles.
                            Item {
                                id: leftBud
                                z: 2
                                width: 31; height: 89
                                x: 34; y: 39 - 16 * stage.open
                                opacity: Math.max(0, (stage.open - 0.12) / 0.88)
                                scale: 0.84 + 0.16 * stage.open
                                rotation: -5 * stage.open
                                transformOrigin: Item.Bottom

                                Rectangle {
                                    width: 31; height: 43; radius: 16
                                    color: "#f7f9fb"
                                    border.width: 1; border.color: "#ffffff"
                                }
                                Rectangle {
                                    width: 11; height: 47; radius: 6
                                    x: 10; y: 31
                                    color: "#f7f9fb"
                                }
                                Rectangle {
                                    width: 7; height: 3; radius: 2
                                    x: 12; y: 55
                                    color: "#bfc8d1"
                                }
                                Rectangle {
                                    width: 6; height: 6; radius: 3
                                    x: 6; y: 17
                                    color: "#d1d8e0"
                                }
                            }
                            Item {
                                id: rightBud
                                z: 2
                                width: 31; height: 89
                                x: 125; y: 39 - 16 * stage.open
                                opacity: Math.max(0, (stage.open - 0.12) / 0.88)
                                scale: 0.84 + 0.16 * stage.open
                                rotation: 5 * stage.open
                                transformOrigin: Item.Bottom

                                Rectangle {
                                    width: 31; height: 43; radius: 16
                                    color: "#f7f9fb"
                                    border.width: 1; border.color: "#ffffff"
                                }
                                Rectangle {
                                    width: 11; height: 47; radius: 6
                                    x: 10; y: 31
                                    color: "#f7f9fb"
                                }
                                Rectangle {
                                    width: 7; height: 3; radius: 2
                                    x: 12; y: 55
                                    color: "#bfc8d1"
                                }
                                Rectangle {
                                    width: 6; height: 6; radius: 3
                                    x: 19; y: 17
                                    color: "#d1d8e0"
                                }
                            }

                            Rectangle {
                                id: caseBody
                                z: 4
                                width: 174; height: 68; radius: 34
                                x: 8; y: 78
                                gradient: Gradient {
                                    GradientStop { position: 0; color: "#ffffff" }
                                    GradientStop { position: 1; color: "#e9edf1" }
                                }
                                border.width: 1
                                border.color: "#ffffff"

                                // A small, shadowed mouth makes the buds read
                                // as nested in a case rather than pasted on it.
                                Rectangle {
                                    width: 150; height: 29; radius: 15
                                    x: 12; y: 5
                                    color: "#ccd3db"
                                    opacity: 0.14 + 0.52 * stage.open
                                }
                                Rectangle {
                                    width: 146; height: 1; radius: 1
                                    x: 14; y: 28
                                    color: "#c8d0d8"
                                    opacity: 0.55
                                }
                                Rectangle {
                                    width: 7; height: 7; radius: 4
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    y: 42
                                    color: "#58d979"
                                    opacity: 0.45 + 0.55 * stage.open
                                }
                            }
                            Rectangle {
                                id: lid
                                // It is the rear half of an open case: behind
                                // the buds, not a white bar painted across
                                // their faces.
                                z: 1
                                width: 166; height: 58 - 17 * stage.open; radius: height / 2
                                // Closed, the lid meets the front shell. As it
                                // opens, the hinge rises just enough to leave
                                // the buds a visible throat of air.
                                x: 12; y: 78 - 8 * stage.open - height
                                rotation: -4 * stage.open
                                transformOrigin: Item.Bottom
                                gradient: Gradient {
                                    GradientStop { position: 0; color: "#ffffff" }
                                    GradientStop { position: 1; color: "#edf1f5" }
                                }
                                border.width: 1
                                border.color: "#ffffff"
                                Rectangle {
                                    width: 126; height: 1; radius: 1
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: 2
                                    color: "#d8dee5"
                                    opacity: 0.62
                                }
                                Rectangle {
                                    width: 136; height: 13; radius: 7
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: 6
                                    color: "#dbe1e7"
                                    opacity: 0.50
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.topMargin: 7
                        spacing: 8
                        ChargePill { Layout.fillWidth: true; label: "Left"; level: AccessoryPresentation.leftCharge }
                        ChargePill { Layout.fillWidth: true; label: "Case"; level: AccessoryPresentation.caseCharge }
                        ChargePill { Layout.fillWidth: true; label: "Right"; level: AccessoryPresentation.rightCharge }
                    }
                }
            }
        }
    }

    component ChargePill: Rectangle {
        required property string label
        required property int level
        implicitHeight: 42
        radius: 14
        color: "#1bffffff"
        border.width: 1
        border.color: "#24ffffff"

        Column {
            anchors.centerIn: parent
            spacing: 1
            Text { anchors.horizontalCenter: parent.horizontalCenter; text: label; color: "#aeb5bf"; font.pixelSize: 11 }
            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: level >= 0 ? level + "%" : "—"
                color: level >= 0 ? "#f6f7f8" : "#7e8793"
                font.pixelSize: 15
                font.weight: Font.DemiBold
            }
        }
    }
}
