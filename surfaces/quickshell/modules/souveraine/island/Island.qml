import qs
import qs.services
import qs.modules.common
import qs.modules.common.widgets
import QtQuick
import QtQuick.Layouts

// Island — the agent surface for the top bar. TASK-70, fed by TASK-69.
//
// A morph host: one element that changes shape with what the agents are doing,
// rather than a fixed widget that is mostly empty. States, in order of weight:
//
//   hidden    no sessions at all — occupy nothing, not a placeholder
//   dot       something is active; a pulse and nothing else
//   pill      the primary session: provider glyph, label, state
//   expanded  tapped open — every session, grouped
//
// DEVICE TYPES ARE A REAL DIFFERENCE, NOT A SETTING
// The Pixel 3 bar is ~1080px with a carrier readout already competing for the
// left side, and ii-phone/BarContent drops the entire leftCenterGroup for that
// reason. So the island's *resting* state is device-dependent: `dot` on a
// narrow bar, `pill` where there is room. That is derived from the same
// threshold the rest of the bar uses (Appearance.sizes.barShorten...), so the
// island narrows exactly when its neighbours do. One authority for "is this
// bar cramped"; the island is a rendering of it, not a second opinion.
//
// HOST-AGNOSTIC, like SubconsciousTicker
// This owns no overlay state and reaches into no manager. Expansion is a local
// state change; anything larger is a signal for whoever mounted it to handle.
// That is what lets the same file serve the desktop bar, the phone bar, and
// later a viewtop node without a fork.
Item {
    id: root

    // The host may pin a form; otherwise it is derived from the bar's own
    // cramped-ness. Values: "auto" | "dot" | "pill".
    property string restingForm: "auto"

    // Optional all the way down: inside a Loader the attached QsWindow is not
    // yet resolved at construction, and reading through it threw a TypeError
    // before the bar had a window.
    property var screen: root.QsWindow?.window?.screen ?? null

    // Same test BarContent uses, so island and neighbours narrow together.
    readonly property bool narrowBar: (Appearance.sizes.barShortenScreenWidthThreshold >= (screen?.width ?? 99999))

    readonly property string form: {
        if (root.restingForm !== "auto")
            return root.restingForm;
        return root.narrowBar ? "dot" : "pill";
    }

    // Nothing to say → occupy nothing. An always-present empty chip trains the
    // eye to ignore the spot, which costs us the one thing the island is for.
    readonly property bool hasContent: AgentSessions.sessions.length > 0

    property bool expanded: false

    // For a host that wants to put the full session list somewhere better than
    // an inline popout (a sidebar, a sheet, a notch overlay). If nobody
    // connects it, the inline expansion below is the fallback — the component
    // is useful alone and better when hosted.
    signal requestOpenPanel

    visible: hasContent
    implicitWidth: visible ? content.implicitWidth : 0
    implicitHeight: Appearance.sizes.baseBarHeight

    readonly property var primary: AgentSessions.primarySession

    // Colour carries the state, so the island reads pre-attentively — you know
    // something is running before you read a word of it.
    readonly property color stateColor: {
        if (AgentSessions.stale)
            return Appearance.colors.colOutlineVariant;
        if (AgentSessions.anyActive)
            return Appearance.colors.colPrimary;
        if (root.primary?.state === "recent")
            return Appearance.colors.colOnLayer0;
        return Appearance.colors.colOutlineVariant;
    }

    Rectangle {
        id: content
        anchors.centerIn: parent
        implicitWidth: row.implicitWidth + 16
        implicitHeight: Math.max(20, Appearance.sizes.baseBarHeight - 10)
        radius: height / 2
        color: root.expanded ? Appearance.colors.colLayer2
             : mouse.containsMouse ? Appearance.colors.colLayer1
             : "transparent"

        Behavior on implicitWidth {
            NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
        }
        Behavior on color {
            ColorAnimation { duration: 120 }
        }

        RowLayout {
            id: row
            anchors.centerIn: parent
            spacing: 6

            // The pulse. Present in every form — in `dot` it IS the island.
            Rectangle {
                id: pulse
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: 8
                implicitHeight: 8
                radius: 4
                color: root.stateColor

                // Only animate while something is genuinely active. A dot that
                // always breathes is decoration; a dot that breathes only when
                // an agent is working is information.
                SequentialAnimation on opacity {
                    running: AgentSessions.anyActive && !AgentSessions.stale
                    loops: Animation.Infinite
                    NumberAnimation { to: 0.35; duration: 900; easing.type: Easing.InOutSine }
                    NumberAnimation { to: 1.0;  duration: 900; easing.type: Easing.InOutSine }
                }
                // Leaving the loop mid-fade would strand it dim.
                onOpacityChanged: if (!AgentSessions.anyActive && opacity !== 1) opacity = 1
            }

            // Multiple agents at once is the case the old per-provider widgets
            // could not show at all. A count is the cheapest honest summary.
            StyledText {
                Layout.alignment: Qt.AlignVCenter
                visible: AgentSessions.activeCount > 1
                text: AgentSessions.activeCount
                color: root.stateColor
                font.pixelSize: Appearance.font.pixelSize.smaller
                font.weight: Font.DemiBold
            }

            MaterialSymbol {
                Layout.alignment: Qt.AlignVCenter
                visible: root.form === "pill" && root.primary !== null
                text: AgentSessions.providerIcon(root.primary?.provider ?? "")
                iconSize: Appearance.font.pixelSize.normal
                color: root.stateColor
            }

            StyledText {
                Layout.alignment: Qt.AlignVCenter
                visible: root.form === "pill" && root.primary !== null
                text: AgentSessions.sessionLabel(root.primary)
                color: Appearance.colors.colOnLayer0
                font.pixelSize: Appearance.font.pixelSize.smaller
                elide: Text.ElideRight
                Layout.maximumWidth: 110
            }
        }

        MouseArea {
            id: mouse
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: (ev) => {
                if (ev.button === Qt.RightButton) {
                    root.requestOpenPanel();
                    return;
                }
                root.expanded = !root.expanded;
            }
        }

        StyledToolTip {
            // Stale is worth saying out loud rather than only dimming: a dim
            // island and a quiet one look identical at a glance.
            text: AgentSessions.stale
                ? qsTr("Agent sessions — stale (%1)").arg(AgentSessions.lastError)
                : AgentSessions.available
                    ? qsTr("%1 agent session(s), %2 active").arg(AgentSessions.sessions.length).arg(AgentSessions.activeCount)
                    : qsTr("Agent sessions — collecting…")
            extraVisibleCondition: mouse.containsMouse && !root.expanded
        }
    }

    IslandExpansion {
        id: expansion
        anchors.top: content.bottom
        anchors.topMargin: 6
        anchors.horizontalCenter: content.horizontalCenter
        visible: root.expanded
        onDismissed: root.expanded = false
    }
}
