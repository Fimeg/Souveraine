import QtQuick
import QtQuick.Layouts
import Quickshell.Bluetooth
import qs.services
import qs.modules.common
import qs.modules.common.widgets

// First-party connectivity page. The controls are direct views over the
// resident Network/Cellular/Bluetooth services; no settings-only state is
// allowed to pretend a radio changed when its owning service did not.
ContentPage {
    id: page
    forceWidth: true

    function wifiIcon(strength) {
        return strength > 80 ? "signal_wifi_4_bar"
            : strength > 60 ? "network_wifi_3_bar"
            : strength > 40 ? "network_wifi_2_bar"
            : strength > 20 ? "network_wifi_1_bar"
            : "signal_wifi_0_bar";
    }

    ContentSection {
        icon: "wifi"
        title: Translation.tr("Wi-Fi")

        ConfigSwitch {
            buttonIcon: Network.materialSymbol
            text: Network.wifiEnabled
                ? Translation.tr("Wi-Fi on") : Translation.tr("Wi-Fi off")
            checked: Network.wifiEnabled
            onCheckedChanged: {
                if (checked !== Network.wifiEnabled) Network.enableWifi(checked);
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 10

            StyledText {
                Layout.fillWidth: true
                text: Network.active
                    ? Translation.tr("Connected to %1").arg(Network.active.ssid)
                    : Translation.tr("Not connected")
                color: Appearance.colors.colSubtext
                elide: Text.ElideRight
            }

            RippleButton {
                implicitWidth: 44
                implicitHeight: 44
                enabled: Network.wifiEnabled && !Network.wifiScanning
                buttonRadius: Appearance.rounding.full
                onClicked: Network.rescanWifi()
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    text: Network.wifiScanning ? "progress_activity" : "refresh"
                    iconSize: 21
                }
                StyledToolTip { text: Translation.tr("Scan for networks") }
            }
        }

        Repeater {
            model: Network.wifiEnabled ? Network.friendlyWifiNetworks : []

            delegate: DialogListItem {
                id: networkRow
                required property var modelData
                Layout.fillWidth: true
                active: modelData.active
                buttonRadius: Appearance.rounding.normal
                onClicked: {
                    if (modelData.active) Network.disconnectWifiNetwork();
                    else Network.connectToWifiNetwork(modelData);
                }

                contentItem: ColumnLayout {
                    anchors {
                        fill: parent
                        leftMargin: networkRow.horizontalPadding
                        rightMargin: networkRow.horizontalPadding
                        topMargin: networkRow.verticalPadding
                        bottomMargin: networkRow.verticalPadding
                    }
                    spacing: 8

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        MaterialSymbol {
                            text: page.wifiIcon(networkRow.modelData.strength)
                            iconSize: 22
                        }
                        StyledText {
                            Layout.fillWidth: true
                            text: networkRow.modelData.ssid
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                        }
                        MaterialSymbol {
                            text: networkRow.modelData.active ? "check"
                                : networkRow.modelData.isSecure ? "lock" : ""
                            iconSize: 20
                        }
                    }

                    MaterialTextField {
                        Layout.fillWidth: true
                        visible: networkRow.modelData.askingPassword
                        placeholderText: Translation.tr("Network password")
                        echoMode: TextInput.Password
                        inputMethodHints: Qt.ImhSensitiveData
                        onAccepted: {
                            Network.changePassword(networkRow.modelData, text);
                            text = "";
                        }
                    }
                }
            }
        }
    }

    ContentSection {
        icon: Cellular.materialSymbol
        title: Translation.tr("Mobile network")

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                StyledText {
                    Layout.fillWidth: true
                    text: Cellular.available
                        ? (Cellular.operatorName || Translation.tr("Mobile network"))
                        : Translation.tr("No modem detected")
                    elide: Text.ElideRight
                }
                StyledText {
                    Layout.fillWidth: true
                    text: Cellular.available
                        ? Translation.tr("%1 · %2% signal%3")
                            .arg(Cellular.accessTech || Cellular.state)
                            .arg(Cellular.signalQuality)
                            .arg(Cellular.roaming ? Translation.tr(" · roaming") : "")
                        : Translation.tr("ModemManager has not exposed a modem")
                    color: Appearance.colors.colSubtext
                    font.pixelSize: Appearance.font.pixelSize.smaller
                    wrapMode: Text.WordWrap
                }
            }
            RippleButton {
                implicitWidth: 44
                implicitHeight: 44
                buttonRadius: Appearance.rounding.full
                onClicked: Cellular.update()
                contentItem: MaterialSymbol {
                    anchors.centerIn: parent
                    text: "refresh"
                    iconSize: 21
                }
            }
        }
    }

    ContentSection {
        icon: "bluetooth"
        title: Translation.tr("Bluetooth")

        ConfigSwitch {
            buttonIcon: BluetoothStatus.connected ? "bluetooth_connected" : "bluetooth"
            text: BluetoothStatus.available
                ? Translation.tr("Bluetooth") : Translation.tr("Bluetooth unavailable")
            enabled: BluetoothStatus.available
            checked: BluetoothStatus.enabled
            onCheckedChanged: {
                if (Bluetooth.defaultAdapter && checked !== Bluetooth.defaultAdapter.enabled)
                    Bluetooth.defaultAdapter.enabled = checked;
            }
        }

        StyledText {
            Layout.fillWidth: true
            text: !BluetoothStatus.available
                ? Translation.tr("BlueZ has not exposed an adapter; no success-shaped toggle is shown.")
                : BluetoothStatus.activeDeviceCount > 0
                    ? Translation.tr("%1 connected device(s)").arg(BluetoothStatus.activeDeviceCount)
                    : Translation.tr("No connected devices")
            color: Appearance.colors.colSubtext
            font.pixelSize: Appearance.font.pixelSize.smaller
            wrapMode: Text.WordWrap
        }
    }
}
