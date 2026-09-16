import QtQuick
import QtQuick.Controls
import qs.Commons
import "Timeline.mjs" as Timeline

// Scrollable single-day calendar; layout is shared with the macOS popup.
Item {
  id: root
  property var day: null
  property double nowMs: Date.now()
  property color foreground: "white"
  property color urgent: "#e2564a"
  property string fontFamily: "sans-serif"
  signal activate(var ev)
  property int cursor: -1
  function moveCursor(dy) {
    if (!rows.length) return
    cursor = Math.max(0, Math.min(rows.length - 1, cursor + dy))
    var y = rows[cursor].top * grid.height
    flick.contentY = Math.max(0, Math.min(flick.contentHeight - flick.height, y - Style.space(40)))
  }
  function activateSelected() { if (cursor >= 0 && cursor < rows.length) activate(rows[cursor].event) }
  readonly property var range: day ? Timeline.visibleRange(day) : ({start: 0, end: 86400000})
  readonly property var rows: day ? Timeline.layout(day.events, range.start, range.end) : []
  readonly property var allDay: day ? day.events.filter(function(e) { return e.all_day }) : []
  readonly property real fraction: day ? (nowMs - range.start) / (range.end - range.start) : 0
  function clock(ms) { return day ? day.clocks[String(ms)] || "" : "" }
  function scrollToNow() { flick.contentY = Math.max(0, Math.min(flick.contentHeight - flick.height, fraction * grid.height - Style.space(80))) }
  onVisibleChanged: if (visible) { cursor = -1; Qt.callLater(scrollToNow) }
  property string scrolledDate: ""
  onDayChanged: if (visible && day && (day.date + range.start + range.end) !== scrolledDate) { scrolledDate = day.date + range.start + range.end; Qt.callLater(scrollToNow) }

  Flickable {
    id: flick
    anchors.fill: parent
    contentWidth: width
    contentHeight: content.implicitHeight
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
    Column {
      id: content
      width: flick.width
      spacing: Style.space(12)
      Column {
        width: parent.width
        visible: root.allDay.length > 0
        Text { text: "ALL DAY"; color: root.foreground; opacity: 0.6; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
        Repeater {
          model: root.allDay
          delegate: Text {
            required property var modelData
            width: parent.width
            bottomPadding: modelData.colors && modelData.colors.length > 1 ? 5 : 0
            CalendarColors { colors: parent.modelData.colors || [] }
            text: modelData.title || "(no title)"
            textFormat: Text.PlainText
            elide: Text.ElideRight
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }
        }
      }
      Item {
        id: grid
        width: parent.width
        height: Style.space((root.range.end - root.range.start) / 60000)
        clip: true
        Repeater {
          model: root.day ? root.day.hours.filter(function(h) { return h >= root.range.start && h < root.range.end }) : []
          delegate: Item {
            required property double modelData
            width: grid.width
            y: (modelData - root.range.start) / (root.range.end - root.range.start) * grid.height
            Rectangle { width: parent.width; height: 1; color: root.foreground; opacity: 0.12 }
            Text { text: root.clock(modelData); textFormat: Text.PlainText; color: root.foreground; opacity: 0.6; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          }
        }
        Repeater {
          model: root.rows
          delegate: Rectangle {
            id: eventBlock
            required property var modelData
            required property int index
            x: Style.space(64) + (grid.width - Style.space(68)) * modelData.lane / modelData.lanes
            y: modelData.top * grid.height
            width: (grid.width - Style.space(68)) / modelData.lanes - Style.space(3)
            height: Math.max(eventTitle.implicitHeight + Style.space(6), Style.space(22), modelData.height * grid.height)
            readonly property color eventColor: modelData.event.color || root.foreground
            color: Qt.rgba(eventColor.r, eventColor.g, eventColor.b, 0.18)
            opacity: modelData.event.end_ms <= root.nowMs ? 0.5 : 1
            radius: Style.space(4)
            clip: true
            CalendarColors { colors: eventBlock.modelData.event.colors || []; anchors.margins: 1 }
            border.color: eventColor
            activeFocusOnTab: true
            Keys.onReturnPressed: root.activate(modelData.event)
            border.width: activeFocus || root.cursor === index ? 2 : 1
            Column {
              anchors.fill: parent
              anchors.margins: Style.space(3)
              Text {
                id: eventTitle
                width: parent.width
                text: eventBlock.modelData.event.title || "(no title)"
                textFormat: Text.PlainText
                elide: Text.ElideRight
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }
              Text {
                width: parent.width
                visible: parent.height >= eventTitle.implicitHeight + implicitHeight
                text: root.clock(eventBlock.modelData.event.start_ms) + " – " + root.clock(eventBlock.modelData.event.end_ms)
                textFormat: Text.PlainText
                elide: Text.ElideRight
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
            MouseArea { id: eventMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.activate(eventBlock.modelData.event) }
            ToolTip {
              id: eventTip
              visible: eventMouse.containsMouse || eventBlock.activeFocus
              delay: 400
              background: Rectangle { color: Color.tooltip.background; border.color: Color.tooltip.border; radius: Style.space(4) }
              text: (eventBlock.modelData.event.title || "(no title)") + "\n" + root.clock(eventBlock.modelData.event.start_ms) + " – " + root.clock(eventBlock.modelData.event.end_ms)
              contentItem: Text { text: eventTip.text; textFormat: Text.PlainText; color: Color.tooltip.text; font.family: root.fontFamily; wrapMode: Text.Wrap; width: Math.min(implicitWidth, Style.space(320)) }
            }
          }
        }
        Rectangle {
          visible: root.fraction >= 0 && root.fraction < 1
          x: Style.space(60)
          y: root.fraction * grid.height
          width: grid.width - x
          height: Style.space(2)
          color: root.urgent
          Text { anchors.right: parent.left; anchors.verticalCenter: parent.verticalCenter; anchors.rightMargin: Style.space(4); text: "NOW"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
        }
      }
    }
  }
}
