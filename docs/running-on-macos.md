# Running omacal on macOS

> **Just want to use it?** The [releases
> page](https://github.com/x3me/omacal/releases) carries a `.dmg` (Apple
> Silicon, unsigned for now: right-click → Open to get past Gatekeeper) with
> credentials included — no Google Cloud project, no config file. Everything
> below is for building and running from source.

## What the macOS build does and doesn't do (as of v0.4.4)

The UI is one shared codebase, so everything recent is in the `.dmg`: the
list-mode rows (invitee count, repeat glyph, the Join camera), the now
marker, weather in the day headers, the clock-following time fields, the
non-selectable chrome. Two platform notes:

- **Weather location** comes from your IP (there is no Omarchy widget to
  read here); the Settings → General knob turns the feature off.
- **Updates are one click from v0.5.0 on**: the `.dmg` is signed with a
  Developer ID certificate and notarized by Apple, so the header's update
  notice carries the same **Update** button the AppImage has — download,
  signature check, swap, restart. Builds before 0.5.0 are unsigned and
  keep the notice-only banner: update those once by hand (right-click →
  Open the new `.dmg`) and the button takes over from there.


For Omarchy Linux, see [`running-on-omarchy.md`](running-on-omarchy.md) — the
setup is the same except for token storage, which needs a Secret Service
provider there.

## Prerequisites

- **Rust** (stable), via [rustup](https://rustup.rs).
- **Node.js and npm** (any current LTS).
- **The Tauri CLI**: `cargo install tauri-cli --version "^2"` (gives you the
  `cargo tauri` subcommand used throughout this guide).
- **Xcode Command Line Tools**: `xcode-select --install`.

`sqlite3` is not needed at all. It used to be, for the sync interval; that is a
control in Settings now.

## Look at it first, without any credentials

    npm --prefix ui install
    OMACAL_SEED_DEMO=1 cargo tauri dev

Demo mode writes to a **separate database** (`omacal-demo.db`) and never calls
Google, so it cannot touch or invent real calendar data. The header shows a
`DEMO DATA` badge while it is active.

## Connecting your real calendar

> **Given a client ID and secret by a colleague?** Then steps 1–3 are already
> done — skip straight to [step 4](#4-write-the-config-file) and paste what you
> were given. Their client identifies the *app*, not their account: you sign in
> as yourself, and your token lands in your own Keychain. On first sign-in
> Google shows a **"Google hasn't verified this app"** screen — expected for an
> unverified single-team app; click **Advanced → Go to … (unsafe) →
> Continue**.

### 1. Create a Google Cloud project

1. <https://console.cloud.google.com/projectcreate> — create a project.
2. **APIs & Services → Library** → enable **Google Calendar API**.

### 2. Configure the OAuth consent screen

1. **APIs & Services → OAuth consent screen** → External.
2. Fill in app name and your email.
3. Add the scope `https://www.googleapis.com/auth/calendar`.
4. Add yourself as a test user.
5. **Publish the app to Production.**

> **Do not skip step 5.** An app left in *Testing* issues refresh tokens that
> **expire after 7 days**. When that happens omacal has no way to renew the
> token in the background — it will simply stop syncing, with no error
> visible anywhere except that the calendar quietly goes stale, and you will
> need to sign in again to notice and fix it. Publishing to Production
> removes the 7-day expiry entirely. You will see an "unverified app"
> warning on first sign-in — that is expected for a single-user app; click
> through it. Google's verification review is only required to distribute
> the app to other people, not to use it yourself.

### 3. Create a Desktop client ID

**APIs & Services → Credentials → Create credentials → OAuth client ID →
Desktop app.** Copy the client ID and secret.

### 4. Write the config file

    mkdir -p ~/.config/omacal
    cat > ~/.config/omacal/config.toml <<'EOF'
    client_id = "PASTE_CLIENT_ID.apps.googleusercontent.com"
    client_secret = "PASTE_CLIENT_SECRET"
    EOF
    chmod 600 ~/.config/omacal/config.toml

The refresh token is never written here or to the database — it goes to the
macOS Keychain, under the service name `omacal`, keyed by your account email.

### 5. Run and sign in

    cargo tauri dev

Click **Connect Google Calendar**. A browser opens; grant access; the tab
confirms and closes. The first sync runs automatically, then every 5 minutes
and whenever the window regains focus.

## Commands

| Command | What it does |
| --- | --- |
| `cargo tauri dev` | Run the app against your real calendar |
| `OMACAL_SEED_DEMO=1 cargo tauri dev` | Run against synthetic demo data |
| `cargo test --workspace` | Rust suite |
| `npm --prefix ui run test:ui` | UI component + visual-regression suite (WebKit + Chromium) |
| `npm --prefix ui run check` | TypeScript and Svelte type checking |
| `cargo tauri build` | Build a release `.app` |

## Settings

Behind the **hamburger** in the header, in five tabs.

**General** — the sync interval. Default 5 minutes, and it will not go below one
minute: Google's quota is finite and a desktop app has no business polling
faster. A shorter value is **refused with a reason** rather than accepted and
quietly clamped, which is the difference between knowing what your app is doing
and believing something untrue about it. (Setting this used to mean running
`sqlite3` against the database by hand. It no longer does.)

Also on General: **When you log in** — don't start omacal, start it, or start
it **in the background**. Nothing is registered until you answer: a fresh
install asks once, with a small card in the corner of the window, and that
answer and this setting are the same choice. (Until then starting was simply
assumed, and before v0.14.0 the app even rewrote its own launch entry on
every start. An install that never chose was moved to the background when
the question arrived, so its reminders kept firing without a window
appearing at login.) Choosing not to start unregisters the entry there and
then, and leaves it off.

**In the background** is the third answer: omacal starts, syncs, fires
reminders and keeps the bar widget's feed current, and no window appears.
Open it whenever you want from the widget, the tray, or your app launcher —
launching omacal yourself always shows the window, whatever this is set to.
The setting applies to the login launch alone, which is why the entry it
writes carries an `--autostart` flag: that flag is how a launch says where it
came from.

**Appearance** — how the calendar looks rather than what it does. **Theme**:
follow the desktop (the Omarchy theme where there is one, dark elsewhere), or
Light or Dark outright, replacing the whole palette without a restart.
**Window frame**: on Linux only — a Hyprland desktop tiles the window and
closes or moves it from the keyboard, so omacal draws no title bar there and
one on any other desktop, where the frame is what you grab; pick Shown or
Hidden to overrule that. On macOS the row is absent, because the overlay
title bar with the traffic lights *is* the frame. **Week view**: a whole week
from a chosen day, or a rolling 3, 5 or 7 days from today.

**Calendars** — the same rows as the header's picker: **show** and **sync** as
two separate switches, plus a colour. **Accounts** — what is connected, and
adding another. **Notifications** — whether reminders fire at all; what fires is
still each event's own Google reminders rather than a schedule omacal invents.

## Per-calendar colour

Each calendar takes Google's colour until you pick another, from a curated set
chosen to stay legible on both a light and a dark theme.

**The choice is local to omacal and is never sent to Google.** Your phone, the
web UI and anyone else subscribed to that calendar see exactly what they saw
before. *Use Google's* clears your choice, which is a different thing from
picking the colour Google currently uses — a cleared calendar follows Google's
colour from then on, including when Google changes it.

## Search

`/`, or the magnifier in the header. Titles only — not location, not
description, not guests — so every result is explicable from what you typed.

Results are ordered by distance from today in **either** direction: a trip last
month and a trip next month both beat one from four years ago. A recurring event
is **one** result rather than one per occurrence, resolved to the occurrence
nearest today.

It searches only the calendars you display. Searching everything synced sounds
more useful and produces a dead end: you click a result on a hidden calendar,
the view jumps to its date, and nothing is drawn — because hiding it is what you
asked for.

Choosing a result moves the calendar to that date in the view you are already in
and opens the event. Escape closes search and leaves you where you were.

## Where the database lives

    ~/Library/Application Support/com.omacal.app/omacal.db

`com.omacal.app` is the app's bundle identifier; macOS's per-app data directory
is derived from it automatically. It is SQLite in WAL mode, so it is **three**
files — plus `-wal` and `-shm`. Copy or delete all three together. The demo
database (`omacal-demo.db`) lives alongside it and is never synced.

## Dragging

Move an event by dragging it, resize it by an edge, or sweep empty grid to start
a new one.

**A drag never emails anybody by itself.** Dropping an event that has guests
asks first, and *Move without notifying* is the default answer — a gesture can
happen by accident, and an accident that mails a meeting's whole guest list is
the outcome the question exists to prevent. Dropping an occurrence of a series
asks which occurrences you meant, the same three scopes the form offers. A drop
that lands where it started does nothing at all: no request, no dialog.

Sweeping empty grid opens the event form pre-filled with the span rather than
creating something untitled — a new event needs a name, and the form is where
that lives.

## Guests, and who gets an email

You can add somebody by address, remove them, and mark them optional, in the
event form beside everything else about an event — on a brand-new event
exactly as on one you already have. omacal used to refuse a create carrying
guests; it no longer does.

**Save asks whether to tell them.** It used to always mail the guest list, on
the reasoning that a time you typed on purpose is exactly what people need to
hear about. That stopped being right once the same Save could fix a typo in an
address: *Save without notifying* is the default action, and *Save and notify
guests* is the only thing on that panel that sends mail. On a create the same
panel reads *Create without notifying* and *Create and notify guests* instead,
naming the action the button actually takes. A drag never mails anybody
without asking either.

**An event you were invited to is yours to move — on your calendar.** Google
keeps a guest's copy of an invitation apart from the organizer's, so dragging
or editing a meeting somebody else organized changes the time on your calendar
alone: the organizer and the other guests keep theirs, and nobody is told.
omacal says so on the move and save panels and offers one button, *Move my
copy* or *Save my copy*, rather than a notify choice that would reach nobody.
The exception is an event whose organizer ticked "guests can modify" — then
the change does reach everyone, the panel says that instead, and the notify
choice is back. To change somebody else's meeting for everyone otherwise, ask
them, or answer with a proposed time. Deleting is different again: only the
organizer can delete an event for everyone, so *Remove from my calendar* is
what a guest gets, and Google tells the organizer you declined. The CLI reads
the same facts — `events show --json` reports `reach` — and its `update` no
longer asks for `--notify` on your own copy of somebody else's event.

Two rules the form enforces rather than letting Google refuse them: the
organizer cannot be removed, and an address that is not an address is refused
before Save rather than coming back as an error afterwards. Removing **yourself**
is offered and is not the same as declining — it takes you off the event, where
declining keeps you on it and tells the organizer. Use the RSVP buttons on the
event for that.

## Troubleshooting

**Sign-in fails immediately with an error containing `no config at
…/.config/omacal/config.toml`** — step 4 above was skipped or the file is in
the wrong place. The full message names the exact path it looked for and
ends with `Create it with client_id and client_secret.`

**Sign-in stops working after about a week** — the OAuth app is still in
*Testing*. Publish it to Production (step 2.5) and sign in again.

**"state mismatch — possible CSRF, sign-in aborted"** — a stale browser tab hit
the loopback listener. Close it and retry.

**Blank window** — check `npm --prefix ui run build` succeeds, then rerun.

## What is not built yet

Offline writes: a save needs the network and says so rather than queueing.
Signing an account out — that means revoking a token, clearing the Keychain
entry and deleting that account's calendars and their events, and a button
that did half of it would leave rows nothing can reach.

**Notifications from a dev run.** The installed, signed app talks to
`UNUserNotificationCenter` directly (`notify_mac.rs`, since the arc after
v0.5.1): the standard permission prompt on first launch, Join and Snooze as
real buttons, and a click that lands the calendar on the occurrence — the
Omarchy feature set, minus only the sticky invitation toast, whose
banner-versus-alert persistence macOS reserves for the user's own System
Settings. What stays unreliable is the **unbundled** run this document is
about: `cargo tauri dev` has no bundle identity, the centre refuses on
principle, and the old best-effort plugin path fails quietly by design — no
error banner, no retry loop, and the reminder is still recorded as fired so a
refusing transport cannot turn into an unbounded retry. To see a real macOS
notification, test against a bundled build.

All three residuals in §7 of
`docs/superpowers/specs/2026-08-08-omacal-form-time-boundary-design.md` are now
closed: all-day placement by the calendar's own date (§7.1), the **All day**
toggle no longer producing a span Save refuses or quietly writing times invented
from the calendar's UTC offset (§7.2), and a time typed into a spring-forward
gap now being named and explained rather than leaving Save dead with no field
looking wrong (§7.3).
