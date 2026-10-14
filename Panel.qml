import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "dnappier.netgear-poe"
  ipcTarget: "dnappier.netgear-poe"
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  property var ports: []
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  property string status: "Open panel to read switch ports"
  property string output: ""
  property bool resetting: false
  readonly property bool busy: request.running

  function deviceName(port) {
    var device = port.attachment ? port.attachment.device : null
    return device ? (device.name || device.vendor || "") : ""
  }

  function portColor(port) {
    if (port.poe_powered === true) return "#0088ff"
    if ((port.poe_capable && port.poe_powered === null) || port.link_up === null) return "#ffb840"
    return port.link_up === true ? "#00dd55" : "#777777"
  }

  function deviceText(device) {
    return (device.name || device.vendor || "Unknown device") + " | " + device.mac
      + (device.ips.length ? " | " + device.ips.join(", ") : "")
  }

  function attachmentText(attachment) {
    if (!attachment) return "Physical device unknown — refresh with updated CLI"
    if (attachment.device) {
      var label = attachment.status === "lldp_neighbor" ? "LLDP neighbor"
        : attachment.status === "switch_ui_neighbor" ? "Switch UI neighbor" : "MAC candidate (unverified)"
      return label + "\n" + deviceText(attachment.device)
    }
    return "Physical device unknown\n" + attachment.explanation
  }

  function refresh() {
    if (busy) return
    resetting = false
    output = ""
    status = "Reading switch..."
    request.command = ["netgear-poe", "--json", "ports"]
    request.running = true
  }

  function resetPort(port) {
    if (busy || port < 1) return
    resetting = true
    output = ""
    status = "Requesting PoE reset on port " + port + "..."
    request.command = ["netgear-poe", "--json", "cycle", String(port), "--yes"]
    request.running = true
  }

  onOpenedChanged: {
    if (opened) refresh()
  }

  Process {
    id: request
    stdout: StdioCollector { onStreamFinished: root.output = text }
    onExited: function(exitCode) {
      try {
        var response = JSON.parse(root.output)
        if (exitCode !== 0 || !response.ok) {
          root.status = response.error || "Command failed; do not automatically retry resets"
          root.ports = []
        } else if (root.resetting) {
          root.status = "Reset accepted. Recovery unverified; refresh to inspect."
          root.ports = []
        } else {
          root.ports = response.data
          root.status = "Blue: PoE · green: link only · gray: offline · amber: unknown. Inverted bolt = powered."
        }
      } catch (error) {
        root.status = "CLI unavailable or invalid output. Check netgear-poe and credentials."
        root.ports = []
      }
    }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "\uf0e7"
    tooltipText: "NETGEAR PoE ports"
    onPressed: root.toggle()
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(500))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(650))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        Controls.ScrollBar.vertical: Controls.ScrollBar { policy: Controls.ScrollBar.AsNeeded }

        ColumnLayout {
          id: column
          width: flick.width
          spacing: Style.space(6)

          Text {
            text: "NETGEAR PoE"
            color: Color.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.title
            font.bold: true
          }
          Text {
            Layout.fillWidth: true
            text: root.status
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            color: Color.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.bodySmall
          }
          Controls.Button {
            text: "Refresh"
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            enabled: !root.busy
            onClicked: root.refresh()
          }

          Repeater {
            model: root.ports
            delegate: ColumnLayout {
              required property var modelData
              required property int index
              Layout.fillWidth: true
              spacing: Style.space(3)
              Rectangle {
                visible: index > 0
                Layout.fillWidth: true
                Layout.topMargin: Style.space(3)
                Layout.bottomMargin: Style.space(5)
                implicitHeight: 1
                color: Color.foreground
                opacity: 0.22
              }
              RowLayout {
                Layout.fillWidth: true
                Text {
                  Layout.fillWidth: true
                  text: "Port " + modelData.port + (root.deviceName(modelData) ? " · " + root.deviceName(modelData) : "")
                    + " | " + (modelData.link_up === true ? "up" : modelData.link_up === false ? "down" : "unknown")
                    + " | " + modelData.speed + " | " + (modelData.power_watts === null ? "no PoE" : modelData.power_watts.toFixed(1) + " W")
                    + (modelData.protected ? " | protected" : "")
                  textFormat: Text.PlainText
                  wrapMode: Text.Wrap
                  color: root.portColor(modelData)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                }
                Controls.Button {
                  id: powerButton
                  readonly property bool delivering: modelData.poe_powered === true
                  text: "\uf0e7"
                  implicitWidth: Style.space(36)
                  implicitHeight: Style.space(36)
                  hoverEnabled: true
                  Accessible.name: "Power cycle port " + modelData.port
                  contentItem: Text {
                    text: powerButton.text
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.icon
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    color: powerButton.delivering ? Color.background : (modelData.poe_capable ? root.portColor(modelData) : "#777777")
                  }
                  background: Rectangle {
                    color: powerButton.delivering ? root.portColor(modelData) : Color.background
                    border.color: modelData.poe_capable ? root.portColor(modelData) : "#777777"
                    border.width: 1
                    radius: Style.space(3)
                  }
                  Controls.ToolTip.visible: hovered
                  Controls.ToolTip.text: !modelData.poe_capable ? "No PoE support"
                    : (modelData.protected ? "Protected port. " : "")
                      + (modelData.poe_powered === true ? "Delivering PoE power" : modelData.poe_powered === false ? "Not delivering PoE power" : "PoE delivery unknown")
                      + (enabled ? ". Click to power-cycle immediately." : ". Reset unavailable.")
                  enabled: !root.busy && modelData.unit === 1 && modelData.poe_capable && modelData.poe_enabled && !modelData.protected
                  onClicked: root.resetPort(modelData.port)
                }
              }
              Text {
                Layout.fillWidth: true
                text: root.attachmentText(modelData.attachment)
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                color: Color.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                lineHeight: 1.1
              }
              Controls.Button {
                id: evidenceButton
                property bool expanded: false
                text: (expanded ? "Hide" : "Show") + " evidence (" + (modelData.attachment ? modelData.attachment.learned_mac_count : "?") + " MACs)"
                flat: true
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                onClicked: expanded = !expanded
              }
              Text {
                visible: evidenceButton.expanded
                Layout.fillWidth: true
                text: (modelData.attachment ? modelData.attachment.explanation + "\n" : "")
                  + (modelData.attachment && modelData.attachment.device && modelData.attachment.device.description ? modelData.attachment.device.description + "\n" : "")
                  + "Learned through this port — not necessarily directly attached:\n"
                  + modelData.devices.map(function(d) {
                    return (d.source || "unknown") + ": " + (d.hostName || "unknown") + " " + (d.hostIpAddress || "") + " " + (d.hostMacAddress || "")
                  }).join("\n")
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                color: Color.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }
      }
    }
  }
}
