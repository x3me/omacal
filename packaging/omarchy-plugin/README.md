# OmaCal bar widget for the Omarchy shell

An Omarchy 4 `bar-widget` plugin. The bar shows the current or next event,
its start time, and a countdown (`Design sync @ 13:30  39m left`). A camera
button beside it joins the eligible call without opening the popup.

The agenda keeps completed events under EARLIER TODAY, dims them, and draws
elapsed progress beside NOW for an ongoing event. Its header and footer stay
visible while events scroll, with the popup capped at 80% of the screen height.
The number of agenda days follows the Week view preference.

In **OmaCal → Settings → Menu bar**, show/hide the meeting label and choose
when Join appears.
The same preferences control the macOS popup: show/hide the bar label and
choose when Join appears (at start time, or 1–60 minutes before). The default
is five minutes; Join remains available until the meeting ends. Calendar
selection and conference links continue to come from OmaCal.

Settings changes refresh the widget immediately through IPC. The widget also
checks the local feed every 15 seconds and when opened. All data comes from
the feed OmaCal itself writes to
`$XDG_STATE_HOME/omacal/upcoming.json` (default `~/.local/state/...`; see
`src-tauri/src/upcoming.rs` for the contract). The widget never touches the
app's database or the network, so without OmaCal it simply shows a quiet
empty state. OmaCal v0.1.9 or newer writes the feed on startup, after every
sync, and after every local edit.

## Install

**You don't.** OmaCal ≥ 0.1.11 carries this plugin inside the app binary
and installs it itself on the first start on an Omarchy machine — files
into `~/.config/omarchy/plugins/omacal.upcoming/`, enabled once in the
bar's right section, and kept up to date by later app releases (see
`src-tauri/src/omarchy_plugin.rs` for the exact rules). Remove the widget
(`omarchy plugin remove omacal.upcoming`) and the app respects that
forever; move it around your bar and updates never touch its placement.

On an older OmaCal, or to hack on the widget, install by hand:

```bash
cp -r packaging/omarchy-plugin ~/.config/omarchy/plugins/omacal.upcoming
omarchy-shell shell rescanPlugins
omarchy plugin enable omacal.upcoming --section right
```

(A hand-installed copy is *adopted* by the app afterwards, not fought —
but note the app rewrites the files whenever its embedded copy is newer,
so hack in the repo checkout, not in `~/.config`.)

## Drive it

- Click the bar icon (or `omarchy-shell omacal.upcoming toggle`) to open.
- Right-click opens OmaCal’s existing natural-language Quick Add form.
- Middle-click the icon opens the OmaCal app directly.
- In the popup: arrows move, Enter joins the call (or opens the app),
  `J` joins the eligible call,
  `o` opens the app, `s` syncs now, `q` quits OmaCal, `r` reloads the
  feed, Esc closes. Sync and Quit also sit at the popup's foot.
- A row with a joinable link shows a camera button; on the running
  meeting it wears the urgent colour. Zoom, Meet, Teams, Webex and
  Jitsi links are recognised even when the invitation only carries
  them in its location field.
- Clicking a row joins its call inside the configured Join window, and opens
  OmaCal on the event's date otherwise. Day-view arrows select timed events;
  Enter activates the selected event.

## One icon, not two

The bar button is OmaCal's own mark, and the popup carries everything the
app's tray menu does — open, sync now, quit (the latter two need OmaCal ≥
0.1.10, which accepts `--sync-now` and `--quit` on a second invocation).
So with this widget installed the tray icon is redundant: turn it off in
the shell's tray settings. **OmaCal → Settings → Menu bar → Show the tray icon**
controls the OmaCal widget as well as its native tray icon.

The `maxEvents` setting (default 12) caps each of the agenda's past and
remaining slices; edit it from the bar's widget settings or in `shell.json`.
The agenda snapshot includes completed events, capped at 200 with a visible
notice if there are more. Times use OmaCal's display clock. The original
upcoming-only feed remains compatible with older readers.

`Timeline.mjs` supplies the same elapsed-progress and Join-window
calculations to this widget and the macOS webview. The app embeds it and
the shared agenda helpers alongside the existing plugin files.

The shell reads snapshots through the bundled `read-feed.py`, using the
system `/usr/bin/python3` (no Python packages or provider CLIs). It opens the
state file once without following links or blocking on special files, checks
ownership and type, reads at most 2 MiB plus one sentinel byte, and has a
two-second deadline. Malformed input is refused, text is capped and cleaned,
and stdout is limited to 1 MiB, checked again before QML parses it. Failures
leave the widget in its existing unavailable-data state.

### Meeting window indicator

The menu-bar camera has a red outline and a pulsing dot only while its selected
meeting is scheduled to run **and a matching meeting window is open**. This is
window presence, not confirmation that audio/video connected or that your camera
is enabled. The tooltip says “Meeting window open.”

On Omarchy/Wayland, OmaCal observes window app IDs and titles locally. Meet codes
and meeting titles identify Google Meet and Teams windows; Zoom meeting IDs or
meeting titles identify named Zoom windows. A generic “Zoom Meeting” window is
associated only with an unambiguous eligible calendar meeting, or with a recent
OmaCal Join request when a new meeting window opens. Clicking Join alone never
activates the indicator. Window associations do not move to another occurrence
when the clock advances. Closing the window or changing tabs clears the match.

Unsupported/localized titles, generic Teams windows, and browser meetings hidden
behind another tab can remain neutral. A provider's lobby and connected call may
share a title, so an open-window match is deliberately not labeled “connected.”
No browser extension, browser history access, microphone access, or network
request is involved. Observation is bounded to 256 windows/events and 1,024
characters per input field; oversized or ambiguous input stays neutral.

Combined calendar copies use one title and a bottom color segment for each calendar.
These segments identify calendar membership; they do not indicate RSVP status.
