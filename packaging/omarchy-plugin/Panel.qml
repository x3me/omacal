import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "Model.js" as Model
import "Timeline.mjs" as Timeline
import "MeetingPresence.mjs" as MeetingPresence

// OmaCal's bar widget: a calendar glyph in the bar, and a popup listing what
// is happening now and what is coming up, in the same visual grammar as the
// stock network/bluetooth panels. All data comes from the feed OmaCal itself
// writes (`~/.local/state/omacal/upcoming.json`, see `src-tauri/src/upcoming.rs`
// in the OmaCal repo) — this widget never touches the app's database or the
// network, so it degrades to a quiet empty state when OmaCal is not running.
Panel {
  id: root
  moduleName: "omacal.upcoming"
  ipcTarget: "omacal.upcoming"
  // Own handler rather than the base's (same pattern as omarchy.dropbox):
  // the extra methods make every popup action scriptable —
  // `omarchy-shell omacal.upcoming syncNow` from a keybinding, a script,
  // or a test.
  manageIpc: false

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function openApp(): void { root.openApp() }
    function preferences(): void { root.openApp("--preferences") }
    function quickAdd(): void { root.openApp("--quick-add") }
    function refresh(): void { feedFile.reload() }
    function syncNow(): string { root.syncNow(); return "ok" }
    function quitApp(): string { root.quitApp(); return "ok" }
  }

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // The machine clock the popup buckets and counts against. Ticks while the
  // popup is open so "ends in 12 min" and the ONGOING section stay honest; a
  // slower tick keeps the closed bar icon's state fresh without cost.
  property double nowMs: Date.now()

  readonly property string feedPath: {
    var state = Quickshell.env("XDG_STATE_HOME")
    var home = Quickshell.env("HOME")
    return (state && String(state).length > 0 ? state : home + "/.local/state")
      + "/omacal/upcoming.json"
  }

  property var feed: null
  readonly property var events: feed ? feed.events : null
  readonly property var day: feed && feed.panel ? feed.panel : null
  readonly property bool dayView: day && day.day_view
  readonly property var callEvent: Timeline.joinable(events || [], nowMs, day ? day.join_minutes : 5)
  property var meetingWindows: []
  property var meetingPresence: []
  property int nextMeetingWindowKey: 0
  property var joinIntent: null
  readonly property bool meetingWindowOpen: MeetingPresence.isPresent(meetingPresence, callEvent, nowMs)
  function observeMeetingWindows() {
    if (!feed) return
    var windows = ToplevelManager.toplevels.values
    if (windows.length > 256) { meetingPresence = []; meetingWindows = []; return }
    var current = [], snapshots = []
    for (var i = 0; i < windows.length; i++) {
      var window = windows[i]
      var known = meetingWindows.find(function(row) { return row.window === window })
      var key = known ? known.key : String(++nextMeetingWindowKey)
      current.push({ window: window, key: key })
      snapshots.push({ key: key, appId: window.appId, title: window.title })
    }
    meetingPresence = MeetingPresence.observe(meetingPresence, snapshots, events || [], nowMs,
      day ? day.join_minutes : 5, joinIntent)
    meetingWindows = current
    if (joinIntent && nowMs - joinIntent.at > 90000) joinIntent = null
  }
  readonly property var barEvent: callEvent || runningEvent || nextEvent
  readonly property bool showLabel: !barVertical && (!day || day.label) && !!barEvent
  readonly property string barLabel: {
    if (!barEvent) return ""
    var ongoing = barEvent.start_ms <= nowMs
    var minutes = Math.max(1, Math.ceil(((ongoing ? barEvent.end_ms : barEvent.start_ms) - nowMs) / 60000))
    var duration = Timeline.countdownDuration(minutes)
    var title = Model.title(barEvent)
    if (title.length > 18) title = title.slice(0, 17) + "…"
    return Timeline.meetingLabel(day ? day.label_format : "", {
      title: title, time: displayClock(barEvent.start_ms), end_time: displayClock(barEvent.end_ms),
      countdown: ongoing ? duration + " left" : "in " + duration, calendar: barEvent.calendar || ""
    })
  }
  function displayClock(ms) { return day && day.clocks[String(ms)] ? day.clocks[String(ms)] : Model.clock(ms) }
  function displayTime(ev) { return ev.all_day ? "ALL DAY" : displayClock(ev.start_ms) + " – " + displayClock(ev.end_ms) }


  // Whether the OmaCal process itself is alive. The popup keeps showing the
  // last feed either way (a stale agenda beats a blank one), but the hero
  // says so, the bar icon dims, and the power button becomes a start button
  // — a Quit that silently no-ops reads as broken, and was reported as such.
  property bool appRunning: true
  readonly property var taskRows: Model.taskRows(feed, nowMs)
  readonly property var panelSections: Model.agendaSections(feed, nowMs, root.setting("maxEvents", 12))
  readonly property var runningEvent: Model.current(events, nowMs)
  readonly property var nextEvent: Model.nextAhead(events, nowMs)
  readonly property bool empty: panelSections.length === 0

  // Today's date beside the mark, when OmaCal's own Appearance switch asks
  // for it (2026-09-04). The *number* comes from the feed rather than from
  // this widget's clock: the zone is the app's setting, and a widget reading
  // the desktop's would disagree with the grid beside it for hours at a
  // time. `setting("showDate")` is the local opt-out, for a bar that wants
  // the app's tray to carry the date and not the bar.
  readonly property var todayFeed: feed && feed.today ? feed.today : null
  readonly property bool barVertical: root.bar ? root.bar.vertical === true : false
  readonly property bool showDate: todayFeed !== null
    && todayFeed.show === true
    && root.setting("showDate", true) === true
    && !barVertical
  readonly property string dateText: todayFeed ? (todayFeed.label !== undefined ? todayFeed.label : todayFeed.day > 0 ? String(todayFeed.day) : "") : ""

  // The bar glyph turns urgent-coloured when a meeting is less than ten
  // minutes out — the glanceable version of "wrap this conversation up".
  readonly property bool imminent: nextEvent !== null
    && nextEvent.start_ms - nowMs < 10 * 60000

  property bool cursorActive: false
  property int rowCursor: 0

  // Sections flattened to one keyboard-navigable list of rows.
  readonly property var flatRows: {
    var rows = []
    for (var s = 0; s < panelSections.length; s++)
      for (var r = 0; r < panelSections[s].rows.length; r++)
        rows.push(panelSections[s].rows[r])
    return rows
  }

  // The bar tooltip's line: with the popup closed, the running meeting and
  // when it ends is exactly the glance the bar is for.
  function heroMeta() {
    if (!appRunning) return "OmaCal is not running"
    if (barEvent)
      return Model.title(barEvent) + " · " + (barEvent.start_ms <= nowMs
        ? Model.endsText(barEvent, nowMs) : Model.leadText(barEvent.start_ms, nowMs))
    if (!feed) return "Waiting for OmaCal"
    return "Nothing scheduled"
  }

  // The popup's header line. NOT `heroMeta`: with the sections on screen,
  // anything the header says about a meeting is some row said again — first
  // the running one, then, once that moved into ONGOING, the next one, which
  // is just the first UPCOMING row (both caught live, 2026-08-17). So the
  // line only speaks when there are no rows to speak for it: the app being
  // down, the feed missing, an empty day. Empty string collapses — PanelHero
  // hides an empty meta.
  function popupHeroMeta() {
    if (!appRunning) return "OmaCal is not running"
    if (!feed) return "Waiting for OmaCal"
    if (empty) return "Nothing scheduled"
    return ""
  }

  // `ymd` is a generated date or the fixed --quick-add action;
  // without, this is the plain "bring up the calendar" it always was.
  function openApp(ymd) {
    if (root.appRunning) {
      // A running app answers the messenger with show()+set_focus(), and on
      // Hyprland an app-side focus request cannot pull the user to a window
      // on ANOTHER workspace — the button read as dead exactly there
      // (2026-08-17, reported live). Only the compositor can cross
      // workspaces, so: deliver the messenger (un-hides a tray-hidden
      // window), give it a beat to map, then have hyprctl jump to it. The
      // messenger runs backgrounded inside the shell so a stale appRunning
      // (app actually gone) turns it into a plain launch that this line
      // never waits on; the dispatches then find nothing, which is fine —
      // a fresh window opens focused on the current workspace anyway.
      //
      // Hyprland ≥ 0.56 (Omarchy 4) speaks Lua through hyprctl — the old
      // keyword dispatchers are gone, `focuswindow class:…` is a parse
      // error. And focus alone does not restack: without bring_to_top a
      // floating omacal covered by another float takes keyboard focus while
      // staying hidden, which reads as a dead button with keystrokes going
      // to the wrong window.
      // The optional date rides the same invocation (OmaCal ≥ 0.3.14 parses
      // a positional YYYY-MM-DD): against a running app it crosses the
      // single-instance channel and moves the calendar; on a fresh launch
      // the app collects it at startup. Interpolated into the shell line
      // only as the fixed shape ymdOf produces — never free text.
      Quickshell.execDetached(["sh", "-c",
        "(omacal " + (ymd || "") + " >/dev/null 2>&1 &); sleep 0.4; " +
        "hyprctl dispatch 'hl.dsp.focus({ window = \"class:omacal\" })'; " +
        "exec hyprctl dispatch 'hl.dsp.window.bring_to_top({ window = \"class:omacal\" })'"])
    } else {
      Quickshell.execDetached(ymd ? ["omacal", ymd] : ["omacal"])
    }
    root.appRunning = true // optimistic; the next check corrects if not
    root.close()
  }

  // The local calendar date of an instant, spelled the one way the app's
  // argv parser admits. Local deliberately: the bar and the calendar sit in
  // the same session, so the row's day and the day the app lands on agree.
  function ymdOf(ms) {
    var d = new Date(ms)
    var mm = ("0" + (d.getMonth() + 1)).slice(-2)
    var dd = ("0" + d.getDate()).slice(-2)
    return d.getFullYear() + "-" + mm + "-" + dd
  }

  // The tray menu's other two actions, carried over the app's
  // single-instance channel (OmaCal ≥ 0.1.10) — this widget plus these two
  // is what lets the tray icon be turned off without losing anything.
  function syncNow() {
    Quickshell.execDetached(["omacal", "--sync-now"])
  }

  function quitApp() {
    Quickshell.execDetached(["omacal", "--quit"])
    root.close()
  }

  // A row's primary action: join the call when there is one, otherwise bring
  // up the app — the two things a calendar row in a status bar is for.
  function activateRow(ev) {
    if (!ev) return
    if (Timeline.joinable([ev], nowMs, day ? day.join_minutes : 5)) {
      joinIntent = { eventKey: MeetingPresence.eventKey(ev), at: Date.now() }
      Qt.openUrlExternally(ev.conference)
      root.close()
    } else {
      // Not just up — *there*: the app opens anchored on the row's own day,
      // so a glance at Friday's meeting continues as Friday on the grid.
      openApp(ymdOf(ev.start_ms))
    }
  }

  // `j`: join the call most worth joining — the next meeting inside its lead window,
  // otherwise the ongoing call. The same
  // URL a row's own camera button would open; this is just the keyboard
  // reaching it without arrowing down.
  function joinCall() {
    if (callEvent) {
      joinIntent = { eventKey: MeetingPresence.eventKey(callEvent), at: Date.now() }
      Qt.openUrlExternally(callEvent.conference)
      root.close()
    }
  }

  function moveCursor(dy) {
    cursorActive = true
    if (flatRows.length === 0) return
    rowCursor = Math.max(0, Math.min(flatRows.length - 1, rowCursor + dy))
  }

  readonly property bool showTray: !feed || feed.tray_icon !== false
  visible: showTray
  onShowTrayChanged: if (!showTray) root.close()
  implicitWidth: showTray ? button.implicitWidth + (barJoin.visible ? barJoin.implicitWidth : 0) + (barTitle.visible ? barTitle.implicitWidth : 0) : 0
  implicitHeight: button.implicitHeight

  onOpenedChanged: if (opened) {
    cursorActive = false
    rowCursor = 0
    nowMs = Date.now()
    feedFile.reload()
    appCheck.running = true
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  Process {
    id: appCheck
    command: ["pgrep", "-x", "omacal"]
    onExited: function(exitCode) { root.appRunning = exitCode === 0 }
  }

  Timer {
    interval: root.opened ? 5000 : 60000
    running: true
    repeat: true
    onTriggered: if (!appCheck.running) appCheck.running = true
  }

  Process {
    id: feedFile
    command: ["/usr/bin/python3", decodeURIComponent(Qt.resolvedUrl("read-feed.py").toString().replace(/^file:\/\//, "")), root.feedPath]
    running: true
    property bool reloadPending: false
    function reload() { if (running) reloadPending = true; else running = true }
    stdout: StdioCollector {
      onStreamFinished: {
        var parsed = text.length <= 1048576 ? Model.parseFeed(text) : null
        if (parsed) {
          parsed.events = Timeline.uniqueAllDay(parsed.events)
          if (parsed.panel) {
            parsed.panel.events = Timeline.uniqueAllDay(parsed.panel.events)
            if (Array.isArray(parsed.panel.agenda_days))
              parsed.panel.agenda_days.forEach(function(day) { day.events = Timeline.uniqueAllDay(day.events) })
          }
        }
        root.feed = parsed
      }
    }
    onExited: function(code) {
      if (code !== 0) root.feed = null
      if (reloadPending) { reloadPending = false; Qt.callLater(reload) }
    }
  }

  Timer {
    interval: 15000
    running: true
    repeat: true
    onTriggered: feedFile.reload()
  }

  Timer {
    interval: root.opened || !!root.callEvent ? 1000 : 60000
    running: true
    repeat: true
    onTriggered: { root.nowMs = Date.now(); root.observeMeetingWindows() }
  }

  BarIconButton {
    id: button
    anchors.left: parent.left
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    width: implicitWidth
    bar: root.bar
    active: root.imminent
    dimmed: !root.appRunning || root.events === null || root.events.length === 0
    tooltipText: root.heroMeta()
    // Wider only when the date rides along: the slot is the icon's own, and
    // a button that reserved room for a number nobody asked for would take
    // panel width from its neighbours. `dateMetrics` measures the glyphs
    // rather than guessing at them, so "9" and "31" each get what they need.
    slotSize: Style.bar.iconSlot
      + (root.showDate ? dateMetrics.width + Style.space(4) : 0)
    // The app's own mark, not a generic glyph: with the tray icon off this
    // is omacal's one presence in the bar. Preserve its orange brand dot.
    iconComponent: Component {
      Item {
        Row {
          anchors.centerIn: parent
          spacing: root.showDate || root.showLabel ? Style.space(4) : 0
          OmacalMark {
            anchors.verticalCenter: parent.verticalCenter
            iconSize: Style.space(12)
            dotColor: "#F97316"
            color: root.imminent ? root.urgent : button.foreground
          }
          Text {
            textFormat: Text.PlainText
            anchors.verticalCenter: parent.verticalCenter
            visible: root.showDate
            text: root.dateText
            color: root.imminent ? root.urgent : button.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            font.weight: Font.DemiBold
          }

        }
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.LeftButton) root.toggle()
      else if (buttonCode === Qt.RightButton) root.openApp("--quick-add")
      else if (buttonCode === Qt.MiddleButton) root.openApp()
    }
  }

  TextMetrics { id: labelMetrics; text: root.barLabel; font.family: root.fontFamily; font.pixelSize: Style.font.body }
  BarIconButton {
    id: barJoin
    anchors.left: button.right
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    bar: root.bar
    visible: !!root.callEvent && !root.barVertical
    readonly property bool live: root.meetingWindowOpen
    iconComponent: Component {
      Item {
        Text {
          anchors.centerIn: parent
          visible: !barJoin.live
          text: ""
          textFormat: Text.PlainText
          font.family: barJoin.fontFamily
          font.pixelSize: barJoin.fontSize
          color: barJoin.foreground
        }
        Item {
          anchors.centerIn: parent
          width: Style.space(16)
          height: Style.space(12)
          visible: barJoin.live
          readonly property color liveColor: "#ff7b86"
          Rectangle {
            id: cameraBody
            x: 0
            anchors.verticalCenter: parent.verticalCenter
            width: Style.space(11)
            height: Style.space(9)
            radius: Style.space(2)
            color: "transparent"
            border.width: Style.space(1)
            border.color: parent.liveColor
            Rectangle {
              anchors.centerIn: parent
              width: Style.space(3)
              height: width
              radius: width / 2
              color: cameraBody.border.color
              SequentialAnimation on opacity {
                running: barJoin.live && barJoin.visible
                loops: Animation.Infinite
                NumberAnimation { from: 0.4; to: 1; duration: 850; easing.type: Easing.InOutSine }
                NumberAnimation { from: 1; to: 0.4; duration: 850; easing.type: Easing.InOutSine }
              }
            }
          }
          Canvas {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            width: Style.space(5)
            height: Style.space(8)
            onPaint: {
              var ctx = getContext("2d")
              ctx.clearRect(0, 0, width, height)
              ctx.strokeStyle = "#ff7b86"
              ctx.lineWidth = Style.space(1)
              ctx.beginPath()
              ctx.moveTo(0.5, height * 0.3)
              ctx.lineTo(width - 0.5, 0.5)
              ctx.lineTo(width - 0.5, height - 0.5)
              ctx.lineTo(0.5, height * 0.7)
              ctx.closePath()
              ctx.stroke()
            }
          }
        }
      }
    }
    tooltipText: root.callEvent ? (barJoin.live ? "Meeting window open · " : "Join ") + Model.title(root.callEvent) : ""
    onPressed: root.joinCall()
  }

  BarIconButton {
    id: barTitle
    anchors.left: barJoin.visible ? barJoin.right : button.right
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    bar: root.bar
    visible: root.showLabel
    active: root.imminent
    dimmed: !root.appRunning
    slotSize: labelMetrics.advanceWidth + Style.space(8)
    tooltipText: root.heroMeta()
    iconComponent: Component {
      Item {
      Text {
        anchors.centerIn: parent
        textFormat: Text.PlainText
        width: labelMetrics.advanceWidth
        text: root.barLabel
        elide: Text.ElideRight
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
        color: root.imminent ? root.urgent : barTitle.foreground
      }
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.LeftButton) root.toggle()
      else if (buttonCode === Qt.RightButton) root.openApp("--quick-add")
      else if (buttonCode === Qt.MiddleButton) root.openApp()
    }
  }

  // Off-screen, and only to measure: the button's width has to be known
  // before the label is laid out inside it.
  TextMetrics {
    id: dateMetrics
    text: root.dateText
    font.family: root.fontFamily
    font.pixelSize: Style.font.body
    font.weight: Font.DemiBold
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(header.implicitHeight + (root.dayView ? Style.space(900) : column.implicitHeight) + footer.implicitHeight + Style.space(24), panel.screenH > 0 ? panel.screenH * 0.8 : Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (root.dayView) { dayTimeline.moveCursor(dy); return }
        if (!root.cursorActive) { root.cursorActive = true; return }
        root.moveCursor(dy)
      }
      onActivateRequested: {
        if (root.dayView) dayTimeline.activateSelected()
        else if (root.cursorActive) root.activateRow(root.flatRows[root.rowCursor])
      }
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(t) {
        if (t === "o" || t === "O") root.openApp()
        else if (t === "r" || t === "R") feedFile.reload()
        else if (t === "s" || t === "S") { if (root.appRunning) root.syncNow() }
        else if (t === "q" || t === "Q") root.appRunning ? root.quitApp() : root.openApp()
        else if (t === "j" || t === "J") root.joinCall()
      }

      Column {
        id: header
        anchors.top: parent.top
        width: parent.width
        spacing: Style.space(12)
          PanelHero {
            width: parent.width
            title: "OmaCal"
            meta: root.popupHeroMeta()
            foreground: root.foreground
            fontFamily: root.fontFamily
            iconComponent: Component {
              OmacalMark {
                iconSize: Style.font.display
                color: root.foreground
                // Preserve the brand accent.
                dotColor: "#F97316"
              }
            }
            trailingControl: Component {
              Row {
                spacing: Style.space(8)
                PanelActionButton {
                  iconText: "+"
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  tooltipText: "Add event with natural language"
                  onClicked: root.openApp("--quick-add")
                }
                PanelActionButton {
                  iconText: ""
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  tooltipText: "OmaCal preferences"
                  onClicked: root.openApp("--preferences")
                }
              PanelActionButton {
                iconText: ""
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.openApp()

                tooltipText: "Open OmaCal"
              }
              }
            }
          }

          Text {
            textFormat: Text.PlainText
            visible: !!root.day
            width: parent.width
            text: root.day ? (root.day.date_label || root.day.date) + " · " + Timeline.currentClock(root.nowMs, root.day.time_format, root.day.utc_offset_seconds === undefined ? -new Date(root.nowMs).getTimezoneOffset() * 60 : root.day.utc_offset_seconds) : ""
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

      }

      Column {
            id: footer
            anchors.bottom: parent.bottom
            width: parent.width
            spacing: Style.space(6)

            PanelSeparator {
              foreground: root.foreground
            }

            Row {
              anchors.right: parent.right
              spacing: Style.space(6)

              PanelActionButton {
                id: syncButton
                iconText: ""
                enabled: root.appRunning
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.syncNow()

                tooltipText: "Sync now"
              }

              PanelActionButton {
                id: quitButton
                iconText: root.appRunning ? "\uf011" : "\uf04b"
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.appRunning ? root.quitApp() : root.openApp()

                tooltipText: root.appRunning ? "Quit OmaCal" : "Start OmaCal"
              }
            }
          }

      Flickable {
        id: panelFlick
        anchors.top: header.bottom
        anchors.bottom: footer.top
        anchors.topMargin: Style.space(12)
        anchors.bottomMargin: Style.space(12)
        width: parent.width
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(12)

          // Feed missing entirely: OmaCal has never run (or never on this
          // version). Say what to do, not just that there is nothing.
          Text {
            textFormat: Text.PlainText
            visible: root.feed === null
            width: parent.width
            text: "No calendar data yet.\nStart OmaCal to populate this panel."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
          }

          Text {
            textFormat: Text.PlainText
            visible: root.feed !== null && root.empty && root.taskRows.length === 0
            width: parent.width
            text: "Nothing scheduled in the next two weeks."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
          }

          Text {
            visible: root.day && root.day.truncated === true
            width: parent.width
            text: "Showing the first 200 events. Open OmaCal for the complete calendar."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }

          DayView {
            id: dayTimeline
            visible: root.dayView
            width: parent.width
            height: visible ? Math.max(Style.space(200), panelFlick.height) : 0
            day: root.day
            nowMs: root.nowMs
            foreground: root.foreground
            urgent: root.urgent
            fontFamily: root.fontFamily
            onActivate: function(ev) { root.activateRow(ev) }
          }

          Repeater {
            model: root.dayView ? [] : root.panelSections

            Column {
              id: sectionColumn
              required property var modelData
              required property int index
              width: column.width
              spacing: Style.space(6)

              // Index of this section's first row in the flattened cursor
              // list, so each row can tell whether it holds the cursor.
              readonly property int rowBase: {
                var base = 0
                for (var s = 0; s < index; s++)
                  base += root.panelSections[s].rows.length
                return base
              }

              PanelSectionHeader {
                visible: sectionColumn.modelData.title !== "ONGOING"
                text: sectionColumn.modelData.title
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              RowLayout {
                visible: sectionColumn.modelData.title === "ONGOING"
                width: parent.width
                spacing: Style.space(10)

                Text {
                  text: "NOW"
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }
                Item {
                  Layout.fillWidth: true
                  implicitHeight: Style.space(3)
                  Accessible.role: root.runningEvent ? Accessible.ProgressBar : Accessible.StaticText
                  Accessible.name: root.runningEvent ? "Elapsed time: " + Model.title(root.runningEvent) : "Current time"
                  Accessible.description: root.runningEvent ? Math.round(Timeline.progress(root.runningEvent, root.nowMs) * 100) + "% elapsed" : "No event in progress"
                  Rectangle { anchors.fill: parent; radius: height / 2; color: root.foreground; opacity: 0.15 }
                  Rectangle {
                    width: root.runningEvent ? parent.width * Timeline.progress(root.runningEvent, root.nowMs) : 0
                    height: parent.height
                    radius: height / 2
                    color: root.urgent
                  }
                }
              }

              Column {
                width: parent.width
                spacing: 0

                Repeater {
                  model: sectionColumn.modelData.rows

                  EventRow {
                    required property var modelData
                    required property int index
                    width: sectionColumn.width
                    event: modelData
                    flatIndex: sectionColumn.rowBase + index
                    lastInSection: index === sectionColumn.modelData.rows.length - 1
                    sectionTitle: sectionColumn.modelData.title
                  }
                }
              }
            }
          }

          // Overdue-or-imminent tasks (OmaCal ≥ 0.2.0 writes them into the
          // feed). Display only — a click opens the app, where the checkbox
          // lives; the bar is for noticing, not managing.
          Column {
            visible: root.taskRows.length > 0
            width: parent.width
            spacing: Style.space(6)

            PanelSeparator {
              visible: !root.empty
              foreground: root.foreground
            }

            PanelSectionHeader {
              text: "TASKS"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Repeater {
              model: root.taskRows

              CursorSurface {
                id: taskRow
                required property var modelData
                width: column.width
                hasCursor: false
                foreground: root.foreground
                implicitHeight: taskContent.implicitHeight + Style.spacing.rowPaddingX

                MouseArea {
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: Qt.PointingHandCursor
                  onClicked: root.openApp()
                }

                RowLayout {
                  id: taskContent
                  anchors.left: parent.left
                  anchors.right: parent.right
                  anchors.verticalCenter: parent.verticalCenter
                  anchors.leftMargin: Style.space(10)
                  anchors.rightMargin: Style.space(10)
                  spacing: Style.space(10)

                  Rectangle {
                    width: 3
                    height: Style.space(20)
                    radius: 1.5
                    color: taskRow.modelData.color ? taskRow.modelData.color : root.dim
                    Layout.alignment: Qt.AlignVCenter
                  }

                  Text {
            textFormat: Text.PlainText
                    Layout.fillWidth: true
                    text: taskRow.modelData.title
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    elide: Text.ElideRight
                  }

                  Text {
            textFormat: Text.PlainText
                    text: taskRow.modelData.label
                    color: taskRow.modelData.overdue ? root.urgent : root.dim
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    Layout.alignment: Qt.AlignVCenter
                  }
                }
              }
            }
          }

        }
      }
    }
  }

  component EventRow: CursorSurface {
    id: row
    property var event: null
    property int flatIndex: 0
    property string sectionTitle: ""
    property bool lastInSection: true
    readonly property bool inOngoing: sectionTitle === "ONGOING"
    opacity: event && !event.all_day && event.end_ms <= root.nowMs ? 0.5 : 1.0
    readonly property bool inAllDay: sectionTitle === "ALL DAY"

    hasCursor: root.cursorActive && root.rowCursor === flatIndex
    foreground: root.foreground

    implicitHeight: rowContent.implicitHeight + Style.spacing.rowPaddingX

    Rectangle {
      anchors.left: parent.left
      anchors.bottom: parent.bottom
      width: row.lastInSection ? 0 : parent.width
      height: 1
      color: "#808080"
      // Past text is dimmed by the row, but separators stay equally subtle.
      opacity: 0.2 / row.opacity
    }

    MouseArea {
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onEntered: { root.cursorActive = true; root.rowCursor = row.flatIndex }
      onClicked: root.activateRow(row.event)
    }

    RowLayout {
      id: rowContent
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      anchors.leftMargin: Style.space(10)
      anchors.rightMargin: Style.space(10)
      spacing: Style.space(10)

      // The calendar's colour as a slim leading tick, the same signal the
      // app's own grid uses for "which calendar is this".
      Rectangle {
        width: 3
        height: Style.space(28)
        radius: 1.5
        color: row.event && row.event.color ? row.event.color : root.dim
        Layout.alignment: Qt.AlignVCenter
      }

      ColumnLayout {
        Layout.fillWidth: true
        spacing: Style.space(1)

        Text {
          textFormat: Text.PlainText
          Layout.fillWidth: true
          text: Model.title(row.event)
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
          elide: Text.ElideRight
        }

        Text {
          textFormat: Text.PlainText
          Layout.fillWidth: true
          visible: text !== ""
          text: {
            var meta = Model.metaText(row.event)
            var lead = ""
            if (row.inOngoing) lead = Model.endsText(row.event, root.nowMs)
            // A single-day event under ALL DAY needs no caption — the
            // section header already says everything its dates could.
            else if (row.inAllDay && Model.isMultiDay(row.event)) lead = Model.untilText(row.event, root.day ? root.day.date_format : null)
            if (lead === "") return meta
            return meta === "" ? lead : lead + "  ·  " + meta
          }
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }
      }

      Text {
          textFormat: Text.PlainText
        // In the ALL DAY section the time column would only repeat the
        // header, so the whole column goes.
        visible: !row.inAllDay
        text: row.inAllDay ? "" : root.displayTime(row.event)
        color: root.foreground
        opacity: row.inOngoing ? 1.0 : 0.75
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
        Layout.alignment: Qt.AlignVCenter
      }

      PanelActionButton {
        visible: !!Timeline.joinable(row.event ? [row.event] : [], root.nowMs, root.day ? root.day.join_minutes : 5)
        iconText: ""
        // Actionable text/icons use the readable foreground; red marks time.
        foreground: root.foreground
        fontFamily: root.fontFamily
        Layout.alignment: Qt.AlignVCenter
        onClicked: root.activateRow(row.event)

        tooltipText: "Join the call"
      }
    }
  }
}
