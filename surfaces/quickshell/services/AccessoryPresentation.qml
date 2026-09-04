// The shell's projection of an accessory event already admitted by sessiond.
//
// This is deliberately a view-state holder, not a Bluetooth service. LibrePods
// cannot import it, and there is no Quickshell IPC target here: an accessory
// reaches this state only when SessiondBridge receives sessiond's directive.
pragma Singleton

import QtQuick

QtObject {
    id: root

    // Incremented for every admitted presentation. A surface keys its local
    // animation and timeout to this edge, so reconnect updates do not grow a
    // pile of cards.
    property int generation: 0
    property string accessoryId: ""
    property string label: "AirPods"
    property int leftCharge: -1
    property int rightCharge: -1
    property int caseCharge: -1
    property bool charging: false

    function present(event) {
        // The daemon has already checked producer admission and payload class.
        // Keep this copy bounded: QML needs display fields, never pairing keys,
        // advertisement bytes, or arbitrary producer metadata.
        root.accessoryId = String(event.accessory_id ?? "");
        root.label = String(event.label ?? "AirPods");
        root.leftCharge = Number(event.left_charge ?? -1);
        root.rightCharge = Number(event.right_charge ?? -1);
        root.caseCharge = Number(event.case_charge ?? -1);
        root.charging = event.charging === true;
        root.generation += 1;
    }
}
