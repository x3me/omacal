import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Model.js" as Model

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

  // Whether the OmaCal process itself is alive. The popup keeps showing the
  // last feed either way (a stale agenda beats a blank one), but the hero
  // says so, the bar icon dims, and the power button becomes a start button
  // — a Quit that silently no-ops reads as broken, and was reported as such.
  property bool appRunning: true
  readonly property var taskRows: Model.taskRows(feed, nowMs)
  readonly property var panelSections: Model.sections(events, nowMs, root.setting("maxEvents", 12))
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
  readonly property string dateText: todayFeed && todayFeed.day > 0 ? String(todayFeed.day) : ""

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
    if (runningEvent)
      return Model.title(runningEvent) + " · " + Model.endsText(runningEvent, nowMs)
    if (nextEvent)
      return Model.title(nextEvent) + " · " + Model.leadText(nextEvent.start_ms, nowMs)
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

  // `ymd` optional: with it, the app lands on that day (a row's own date);
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
    if (ev.conference) {
      Qt.openUrlExternally(ev.conference)
      root.close()
    } else {
      // Not just up — *there*: the app opens anchored on the row's own day,
      // so a glance at Friday's meeting continues as Friday on the grid.
      openApp(ymdOf(ev.start_ms))
    }
  }

  // `j`: join the call most worth joining — the running meeting first,
  // failing that the nearest upcoming one that has a link at all. The same
  // URL a row's own camera button would open; this is just the keyboard
  // reaching it without arrowing down.
  function joinCall() {
    let pick = null
    for (let i = 0; i < flatRows.length; i++) {
      const r = flatRows[i]
      if (!r.event || !r.event.conference) continue
      if (r.inOngoing) { pick = r.event; break }
      if (!pick) pick = r.event
    }
    if (pick) {
      Qt.openUrlExternally(pick.conference)
      root.close()
    }
  }

  function moveCursor(dy) {
    cursorActive = true
    if (flatRows.length === 0) return
    rowCursor = Math.max(0, Math.min(flatRows.length - 1, rowCursor + dy))
  }

  implicitWidth: button.implicitWidth
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

  FileView {
    id: feedFile
    path: root.feedPath
    watchChanges: true
    printErrors: false
    onLoaded: root.feed = Model.parseFeed(text())
    onFileChanged: reload()
    onLoadFailed: root.feed = null
  }

  Timer {
    interval: root.opened ? 15000 : 60000
    running: true
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
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
    // is omacal's one presence in the bar. Monochrome like its neighbours;
    // urgent-tinted when a meeting is imminent, as the glyph was — and the
    // date, when shown, is tinted with it: one voice, not two.
    iconComponent: Component {
      Item {
        Row {
          anchors.centerIn: parent
          spacing: root.showDate ? Style.space(4) : 0
          OmacalMark {
            anchors.verticalCenter: parent.verticalCenter
            iconSize: Style.space(12)
            color: root.imminent ? root.urgent : button.foreground
          }
          Text {
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
      if (buttonCode === Qt.MiddleButton) root.openApp()
      else root.toggle()
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
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (!root.cursorActive) { root.cursorActive = true; return }
        root.moveCursor(dy)
      }
      onActivateRequested: if (root.cursorActive) root.activateRow(root.flatRows[root.rowCursor])
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(t) {
        if (t === "o" || t === "O") root.openApp()
        else if (t === "r" || t === "R") feedFile.reload()
        else if (t === "s" || t === "S") { if (root.appRunning) root.syncNow() }
        else if (t === "q" || t === "Q") root.appRunning ? root.quitApp() : root.openApp()
        else if (t === "j" || t === "J") root.joinCall()
      }

      Flickable {
        id: panelFlick
        anchors.fill: parent
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
                // The hero can afford the brand's own orange; the bar cannot.
                dotColor: "#F97316"
              }
            }
            trailingControl: Component {
              PanelActionButton {
                iconText: ""
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.openApp()

                PanelToolTip {
                  visible: parent.containsMouse
                  text: "Open OmaCal"
                  fontFamily: root.fontFamily
                }
              }
            }
          }

          // Feed missing entirely: OmaCal has never run (or never on this
          // version). Say what to do, not just that there is nothing.
          Text {
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
            visible: root.feed !== null && root.empty && root.taskRows.length === 0
            width: parent.width
            text: "Nothing scheduled in the next two weeks."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
          }

          Repeater {
            model: root.panelSections

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

              PanelSeparator {
                visible: sectionColumn.index > 0
                foreground: root.foreground
              }

              PanelSectionHeader {
                text: sectionColumn.modelData.title
                foreground: sectionColumn.modelData.title === "ONGOING" ? root.urgent : root.foreground
                fontFamily: root.fontFamily
              }

              Repeater {
                model: sectionColumn.modelData.rows

                EventRow {
                  required property var modelData
                  required property int index
                  width: sectionColumn.width
                  event: modelData
                  flatIndex: sectionColumn.rowBase + index
                  sectionTitle: sectionColumn.modelData.title
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
                    Layout.fillWidth: true
                    text: taskRow.modelData.title
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    elide: Text.ElideRight
                  }

                  Text {
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

          // The tray menu's remaining vocabulary, so the tray icon is
          // dispensable: sync (s) and quit (q). Open lives on the hero.
          Column {
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

                PanelToolTip {
                  visible: syncButton.containsMouse
                  text: "Sync now"
                  fontFamily: root.fontFamily
                }
              }

              PanelActionButton {
                id: quitButton
                iconText: root.appRunning ? "\uf011" : "\uf04b"
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.appRunning ? root.quitApp() : root.openApp()

                PanelToolTip {
                  visible: quitButton.containsMouse
                  text: root.appRunning ? "Quit OmaCal" : "Start OmaCal"
                  fontFamily: root.fontFamily
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
    readonly property bool inOngoing: sectionTitle === "ONGOING"
    readonly property bool inAllDay: sectionTitle === "ALL DAY"

    hasCursor: root.cursorActive && root.rowCursor === flatIndex
    foreground: root.foreground

    implicitHeight: rowContent.implicitHeight + Style.spacing.rowPaddingX

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
          Layout.fillWidth: true
          text: Model.title(row.event)
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
          elide: Text.ElideRight
        }

        Text {
          Layout.fillWidth: true
          visible: text !== ""
          text: {
            var meta = Model.metaText(row.event)
            var lead = ""
            if (row.inOngoing) lead = Model.endsText(row.event, root.nowMs)
            // A single-day event under ALL DAY needs no caption — the
            // section header already says everything its dates could.
            else if (row.inAllDay && Model.isMultiDay(row.event)) lead = Model.untilText(row.event)
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
        // In the ALL DAY section the time column would only repeat the
        // header, so the whole column goes.
        visible: !row.inAllDay
        text: row.inAllDay ? "" : Model.timeText(row.event)
        color: row.inOngoing ? root.urgent : root.foreground
        opacity: row.inOngoing ? 1.0 : 0.75
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
        Layout.alignment: Qt.AlignVCenter
      }

      PanelActionButton {
        visible: row.event !== null && !!row.event.conference
        iconText: ""
        // The running meeting's Join wears the urgent colour — it is the one
        // button in the popup you are probably here to press.
        foreground: row.inOngoing ? root.urgent : root.foreground
        fontFamily: root.fontFamily
        Layout.alignment: Qt.AlignVCenter
        onClicked: root.activateRow(row.event)

        PanelToolTip {
          visible: parent.containsMouse
          text: "Join the call"
          fontFamily: root.fontFamily
        }
      }
    }
  }
}
