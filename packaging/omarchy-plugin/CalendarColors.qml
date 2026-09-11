pragma ComponentBehavior: Bound
import QtQuick

// The feed reader bounds this to 200 validated hex colors before QML sees it.
Item {
  id: root
  property var colors: []
  anchors.left: parent.left
  anchors.right: parent.right
  anchors.bottom: parent.bottom
  height: 3
  visible: colors.length > 1
  Repeater {
    model: root.colors.length > 1 ? root.colors : []
    delegate: Rectangle {
      required property color modelData
      required property int index
      x: index * root.width / root.colors.length + 1
      width: Math.max(0, root.width / root.colors.length - 2)
      height: root.height
      radius: 1
      color: modelData
    }
  }
}
