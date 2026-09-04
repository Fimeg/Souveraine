// Notification event fan-out. The freedesktop server itself lives in ii's
// Notifications.qml (it owns org.freedesktop.Notifications on the bus); this
// singleton is the Souveraine-side seam everything else attaches to when a
// notification lands: the lock glance card today, crash-report surfacing
// (TASK-05) and agent message events (queue 5a/b) later.
//
// Deliberately NOT here: any decision about what the agent may see. That is
// capability-gate territory (queue 5b) — this file is only the wire. Events
// are plain objects, not Notif wrappers, so consumers can't reach actions or
// dismissal through this path.
pragma Singleton

import QtQuick
import Quickshell

Singleton {
    id: root

    // Newest-first ring of recent events: { app, summary, body, urgency,
    // isTransient, time }. Session-scoped; ii already persists the full
    // notification list to disk, this ring is for reactive consumers.
    property var recent: []
    readonly property int capacity: 32

    // Fired once per incoming notification, after `recent` is updated.
    signal landed(var evt)

    Connections {
        target: Notifications
        function onNotify(notif) {
            const evt = {
                app: notif.appName,
                summary: notif.summary,
                body: notif.body,
                urgency: notif.urgency,
                isTransient: notif.isTransient,
                time: notif.time,
            };
            root.recent = [evt, ...root.recent].slice(0, root.capacity);
            root.landed(evt);
        }
    }
}
