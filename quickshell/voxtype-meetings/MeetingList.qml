import QtQuick
import QtQuick.Layouts
import Quickshell.Io

// Past meetings, newest first, for when no meeting is on screen: title, date
// and time, length, speakers and a Summary badge. Clicking one opens it like
// an ended meeting. `voxtype meeting list --json` gives the list, with
// `has_summary` and `speaker_count` on each entry. A voxtype from before those
// fields leaves them out; then each row reads its own folder for them
// (transcript.json, metadata.json, summary.md), only for rows actually built.
Item {
  id: root

  property var theme
  property var meetings: []
  property bool loading: false
  property string error: ""
  property bool hasMore: false

  signal openRequested(string meetingId)
  signal moreRequested()

  // Where the list was when older meetings were asked for: a longer list
  // replaces the model, which would otherwise jump back to the top.
  property real keepY: -1
  onMeetingsChanged: {
    if (root.keepY < 0) return
    var y = root.keepY
    root.keepY = -1
    Qt.callLater(function() { list.contentY = y })
  }

  // chrono writes nanoseconds; JavaScript dates take milliseconds at most.
  function parseTime(iso) {
    if (!iso) return NaN
    return Date.parse(String(iso).replace(/(\.\d{3})\d+/, "$1"))
  }

  function when(iso) {
    var t = root.parseTime(iso)
    return isNaN(t) ? "" : Qt.formatDateTime(new Date(t), "ddd d MMM yyyy, hh:mm")
  }

  function duration(m) {
    if (m.duration_secs === undefined || m.duration_secs === null)
      return String(m.status) === "active" || String(m.status) === "paused" ? "in progress" : ""
    var mins = Math.round(Number(m.duration_secs) / 60)
    if (mins < 1) return "under a minute"
    if (mins < 60) return mins + " min"
    return Math.floor(mins / 60) + " h " + (mins % 60 < 10 ? "0" : "") + (mins % 60) + " min"
  }

  function speakerCount(text) {
    var data = null
    try { data = JSON.parse(text) } catch (e) { return -1 }
    var segs = data && data.segments ? data.segments : []
    var seen = ({})
    var n = 0
    for (var i = 0; i < segs.length; i++) {
      var s = segs[i]
      var key = s.speaker_label || s.speaker_id || (s.source === "microphone" ? "You" : s.source === "loopback" ? "Remote" : "Unknown")
      var fold = String(key).toLowerCase()
      if (seen[fold]) continue
      seen[fold] = true
      n++
    }
    return n
  }

  ColumnLayout {
    anchors.fill: parent
    spacing: root.theme.md

    Text {
      Layout.fillWidth: true
      text: "Recent meetings"
      color: root.theme.fg
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.body
      font.bold: true
    }

    Text {
      objectName: "meetingListError"
      Layout.fillWidth: true
      visible: root.error !== ""
      text: root.error
      color: root.theme.urgent
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      objectName: "meetingListEmpty"
      Layout.fillWidth: true
      visible: !root.loading && root.error === "" && root.meetings.length === 0
      text: "No meetings yet. Meetings you record appear here, newest first."
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    ListView {
      id: list
      objectName: "meetingList"
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      spacing: root.theme.xs
      model: root.meetings
      boundsBehavior: Flickable.StopAtBounds

      delegate: Rectangle {
        id: row
        required property var modelData
        required property int index
        objectName: "meetingRow_" + index
        readonly property string meetingId: String(modelData.id)
        readonly property string title: modelData.title ? String(modelData.title) : "Untitled meeting"
        // From the list when it says; -1 / null until the folder is read.
        readonly property bool listed: modelData.speaker_count !== undefined && modelData.has_summary !== undefined
        property int speakers: modelData.speaker_count !== undefined ? Number(modelData.speaker_count) : -1
        property bool summary: modelData.has_summary !== undefined ? modelData.has_summary === true
          : (modelData.summary !== undefined && modelData.summary !== null)
        // Only an older voxtype's list needs the folder read.
        readonly property string folder: !row.listed && modelData.storage_path ? String(modelData.storage_path) : ""
        readonly property string details: {
          var parts = [root.when(modelData.started_at)]
          var d = root.duration(modelData)
          if (d !== "") parts.push(d)
          if (row.speakers > 0) parts.push(row.speakers + (row.speakers === 1 ? " speaker" : " speakers"))
          return parts.join(" · ")
        }

        width: ListView.view.width
        implicitHeight: rowColumn.implicitHeight + root.theme.md * 2
        radius: Math.min(root.theme.radius, 6)
        color: rowMouse.containsMouse ? root.theme.alpha(root.theme.fg, 0.08) : "transparent"

        FileView {
          path: row.folder !== "" ? row.folder + "/transcript.json" : ""
          printErrors: false
          onLoaded: row.speakers = root.speakerCount(text())
        }
        FileView {
          path: row.folder !== "" && !row.summary ? row.folder + "/metadata.json" : ""
          printErrors: false
          onLoaded: {
            try {
              var m = JSON.parse(text())
              if (m && m.summary) row.summary = true
            } catch (e) {}
          }
        }
        FileView {
          path: row.folder !== "" && !row.summary ? row.folder + "/summary.md" : ""
          printErrors: false
          onLoaded: row.summary = true
        }

        ColumnLayout {
          id: rowColumn
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          anchors.leftMargin: root.theme.padX
          anchors.rightMargin: root.theme.padX
          spacing: root.theme.xxs

          RowLayout {
            Layout.fillWidth: true
            spacing: root.theme.md
            Text {
              objectName: "meetingRowTitle"
              Layout.fillWidth: true
              text: row.title
              color: root.theme.fg
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.body
              elide: Text.ElideRight
            }
            Rectangle {
              objectName: "meetingRowSummary"
              visible: row.summary
              implicitWidth: badge.implicitWidth + root.theme.md * 2
              implicitHeight: badge.implicitHeight + root.theme.xxs * 2
              radius: height / 2
              color: root.theme.alpha(root.theme.accent, 0.2)
              border.width: 1
              border.color: root.theme.alpha(root.theme.accent, 0.6)
              Text {
                id: badge
                anchors.centerIn: parent
                text: "Summary"
                color: root.theme.fg
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.caption
              }
            }
          }

          Text {
            objectName: "meetingRowDetails"
            Layout.fillWidth: true
            text: row.details
            color: root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
            elide: Text.ElideRight
          }
        }

        MouseArea {
          id: rowMouse
          anchors.fill: parent
          hoverEnabled: true
          cursorShape: Qt.PointingHandCursor
          onClicked: root.openRequested(row.meetingId)
        }
      }

      footer: Item {
        width: ListView.view.width
        implicitHeight: more.visible ? more.implicitHeight + root.theme.lg * 2 : 0
        MeetingButton {
          id: more
          objectName: "meetingListMore"
          theme: root.theme
          anchors.centerIn: parent
          visible: root.hasMore
          text: root.loading ? "Loading…" : "Show older meetings"
          bordered: true
          enabled: !root.loading
          onClicked: {
            root.keepY = list.contentY
            root.moreRequested()
          }
        }
      }
    }
  }
}
