# Running omacal on Omarchy

> **Just want to use it?**
> `curl -fsSL https://omacal.app/install.sh | sh` installs the
> latest release, credentials included: no Google Cloud project, no config
> file. Everything below is for building and running from source.

Omarchy is Arch-based and uses WebKitGTK rather than macOS's WKWebView. The app
has been built and run there; the week grid, theming and live theme reload are
all verified on real hardware.

## Prerequisites

    sudo pacman -S --needed base-devel curl wget file openssl \
      webkit2gtk-4.1 librsvg libappindicator-gtk3 nodejs npm

Plus **Rust** (stable, via [rustup](https://rustup.rs)) and the Tauri CLI:

    cargo install tauri-cli --version "^2"

## Look at it first, without any credentials

    npm --prefix ui install
    OMACAL_SEED_DEMO=1 cargo tauri dev

Demo mode writes to a **separate database** (`omacal-demo.db`) and never calls
Google, so it cannot touch or invent real calendar data. The header shows a
`DEMO DATA` badge while it is active, and every write command refuses.

This is also the right way to check a build: it exercises WebKitGTK rendering and
all five views without needing an account.

## The bar widget (Omarchy 4)

`packaging/omarchy-plugin/` is a shell plugin for Omarchy 4's bar: a
calendar icon whose popup lists the event you are in right now and the rest
of today's agenda — or, when today is done, the nearest day that has
something — with times, locations, headcounts, and a join button for events
with a conferencing link. **The app installs it automatically** on the
first start on an Omarchy machine (omacal ≥ 0.1.11), keeps it updated, and
never brings it back once you remove it. Usage and the hand-install path:
[`packaging/omarchy-plugin/README.md`](../packaging/omarchy-plugin/README.md).

The widget's bar button is omacal's own mark, and its popup carries the
tray menu's whole vocabulary — open, sync now, quit — over the app's
single-instance flags (`omacal --sync-now`, `omacal --quit`). With the
widget installed you can therefore turn the tray icon off (Settings →
General) and keep a single omacal icon in the bar.

With an iCloud or CalDAV account connected, the popup also shows a TASKS
section: open tasks that are overdue or due within two days, colored by
their list. Display only — completing them happens in the app's Tasks
panel (menu → Tasks…).

It reads the feed omacal writes to `$XDG_STATE_HOME/omacal/upcoming.json`
(default `~/.local/state/...`) — rewritten atomically on startup, after
every successful sync, and after every local edit. The feed follows the
same rules as the app's grid and the reminder scheduler: selected calendars
only, cancelled occurrences suppressed, declined invitations skipped, and
recurring series expanded by the app itself. The file is plain JSON, so
anything else (a script, another bar) can read the same answer; the
`version` field only bumps when a field changes meaning.

## Theming

Colour is read from `~/.local/state/omarchy/current/theme` (Omarchy 4) or
`~/.config/omarchy/current/theme` (earlier Omarchy) and follows
`omarchy-theme-set` **live** — the UI repaints within about a second, no restart.

The palette comes from the theme's `colors.toml` when it carries the full
Omarchy 4 palette (`mode`, `background`, `foreground`, …); older themes fall
back to `alacritty.toml`, with an `accent` from `colors.toml` still honoured.

The watcher observes the *parent* directory rather than the theme path itself,
because `omarchy-theme-set` replaces the theme wholesale — Omarchy 4 swaps in
a freshly staged directory, pre-4 relinked a symlink — rather than editing
files beneath it; watching the theme path would never fire.

### Font

Settings → Appearance → **Use the Omarchy font** draws the interface in the
monospace family `omarchy font set` chose. It is off by default: monospace is
wider, so titles in narrow event blocks cut off sooner. The family is read the
way `omarchy font current` reads it, `fc-match monospace`, and only while the
switch is on; the usual stack stays behind it for any glyph it lacks.
`omarchy font set` writes `~/.config/fontconfig` and nothing under the theme,
so omacal watches that directory beside the theme's, and every theme switch
reads the font again too. The directory has to exist when omacal starts; if
the first `omarchy font set` is what creates it, that change shows at the next
theme switch or restart.

### Opacity

Omarchy blends every window a little on its own — 98.5% focused, 96%
unfocused, the `default-opacity` tag in
`/usr/share/omarchy/default/hypr/windows.lua` — and always did to this one.
Settings → Appearance starts the calendar background at 4% here to match
(anywhere else it starts at 0), and the slider runs on top of the
compositor's share, so 0% is not physically opaque on Omarchy. If you want it
to be, take the window out of the tag in `~/.config/hypr/hyprland.lua`:

    o.window("^omacal$", { tag = "-default-opacity", opacity = "1 override 1 override" })

Hyprland reloads on save; `hyprctl configerrors` should print nothing.

## Connecting your real calendar

Identical to the macOS guide's steps 1–4 — see
[`running-on-macos.md`](running-on-macos.md) for the Google Cloud project, the
consent screen (**publish to Production**, or refresh tokens expire after seven
days), and the Desktop client ID.

If a colleague already gave you a `client_id` and `client_secret`, all of that
is done — write the config file below, sign in as yourself, and click through
the **"Google hasn't verified this app"** screen on first sign-in (**Advanced →
Go to … (unsafe) → Continue**). Your token is your own, stored on your machine.

The config file is at the same path:

    mkdir -p ~/.config/omacal
    cat > ~/.config/omacal/config.toml <<'EOF'
    client_id = "PASTE_CLIENT_ID.apps.googleusercontent.com"
    client_secret = "PASTE_CLIENT_SECRET"
    EOF
    chmod 600 ~/.config/omacal/config.toml

### The one thing that differs from macOS: token storage

On macOS the refresh token goes to the Keychain. On Linux the `keyring` crate
resolves to the **Secret Service** backend, which needs a provider actually
running — gnome-keyring, KeePassXC or kwallet. A minimal Hyprland session often
has none, and sign-in then fails at the token write.

Check before you bother:

    busctl --user list | grep -i secret

Nothing there means you need a Secret Service provider running in your session.
That is a gap in omacal's Linux support, not a mistake in your setup.

## Notifications

This is the platform they were built for. Reminder times come from each event's
own Google `reminders`, falling back to the calendar's `defaultReminders`, so
what fires here matches what your phone does. Only `popup` reminders fire —
Google sends the `email` ones from its own servers, and firing those locally
would double every one.

They go out over D-Bus to `org.freedesktop.Notifications`, which on Omarchy
means **mako**. As with the Secret Service above, that needs something actually
running to receive them:

    busctl --user list | grep -i Notifications

Nothing there means no daemon is listening and reminders will go nowhere. omacal
treats a refused post as expected rather than as an error — it logs it, records
the reminder as fired, and carries on, because the alternative is retrying the
same failure on every pass forever.

Only calendars you have **selected** fire, not everything you sync. Deselecting
a calendar cancels its pending reminders, which is the intended reading of that
switch: if it is not worth drawing, it is not worth interrupting you. Search
follows the same switch, for the same reason.

Reminders can be turned off entirely in **Settings → Notifications**. That
switch controls whether omacal fires anything at all; *what* fires is still each
event's own Google reminders — with one addition omacal owns. Google reminders
are **per user**: on a shared calendar you only receive, this account often has
no reminders at all, and every meeting on it is silent. For exactly that case —
a timed event following its calendar's defaults, on a calendar that has none —
omacal's own **fallback reminders** apply: 60 and 10 minutes out of the box,
editable as rows in the same tab, and clearing the list turns them off. They
never override an event's or a calendar's real reminders, and they never touch
all-day events.

A reminder that came due while omacal was not running fires at the next launch,
provided the event has not already ended. Closing the window **hides** it rather
than quitting — the scheduler is the point, and a closed window that stopped
firing reminders would be a bug. Quit from the tray when you mean it.

## Settings, colour, search and dragging

These behave identically on both platforms, and the macOS guide describes them
in full — [Settings](running-on-macos.md#settings), [per-calendar
colour](running-on-macos.md#per-calendar-colour),
[search](running-on-macos.md#search),
[dragging](running-on-macos.md#dragging) and [guests](running-on-macos.md#guests-and-who-gets-an-email).
Written once rather than twice: nothing about any of them is
platform-specific, and a second copy is one that drifts.

The four worth knowing without following a link:

- **Settings** live behind the hamburger. The sync interval is a control there
  now rather than an `sqlite3` incantation, and a value under a minute is
  refused with a reason instead of being quietly clamped.
- **A calendar's colour is local to omacal** and never reaches Google — your
  phone, the web UI and anyone else subscribed see what they always saw.
- **Search** is `/`, titles only, nearest to today first, and only across the
  calendars you display.
- **A drag never emails anybody by itself**; dropping an event with guests asks
  first, and not notifying is the default answer.

## Where things live

| | macOS | Omarchy |
| --- | --- | --- |
| Database | `~/Library/Application Support/com.omacal.app/omacal.db` | `~/.local/share/com.omacal.app/omacal.db` |
| Config | `~/.config/omacal/config.toml` | same |
| Theme | (none — falls back to built-in colours) | `~/.local/state/omarchy/current/theme`, else `~/.config/omarchy/current/theme` |

The database is SQLite in WAL mode, so it is **three** files — `omacal.db`, plus
`-wal` and `-shm`. Copy or delete all three together.

Everything omacal remembers is in that database: your calendars and their
events, the sync cursors, which reminders have fired, your settings, and your
per-calendar colours. The one thing that is **not** there is the refresh token,
which lives in the Secret Service. Deleting the database costs a resync, not a
sign-in.

## Commands

| Command | What it does |
| --- | --- |
| `cargo tauri dev` | Run against your real calendar |
| `OMACAL_SEED_DEMO=1 cargo tauri dev` | Run against synthetic demo data |
| `cargo test --workspace` | Rust suite |
| `npm --prefix ui run test:ui` | UI suite (WebKit + Chromium) |
| `npm --prefix ui run check` | TypeScript and Svelte type checking |
| `cargo tauri build` | Build a release binary |

## The UI suite on Arch

`npm --prefix ui run test:ui` needs Playwright's own browsers, and on Arch
two things stand between you and them. Without the browsers **the suite does
not fail — it reports the ~260 node-side tests as passed and lists the ~880
browser tests as failures above the summary**, which a piped `tail` happily
truncates into what looks like a green run. Check the exit code, not the last
line.

First, the download refuses because Arch is not a supported host:

    PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true npx --prefix ui playwright install

Chromium then runs as-is. WebKit is an Ubuntu 24.04 build and wants five
libraries under Ubuntu's sonames — ICU 74, `libxml2.so.2` and the flite
voices — which Arch does not ship. No `sudo` required: the bundle already
carries a `sys/lib` for exactly this, so extract the three noble debs
(`libicu74`, `libxml2`, `libflite1`) and copy their `*.so.*` into **both**

    ~/.cache/ms-playwright/webkit-*/minibrowser-gtk/sys/lib/
    ~/.cache/ms-playwright/webkit-*/minibrowser-wpe/sys/lib/

— headed runs use the GTK bundle, headless the WPE one, and a reinstall of
the browsers wipes both (the suite then fails loudly on launch, which is the
signal to redo this).

One known failure remains on Linux: `a standalone view gets the same box the
app gives it` is off by 1px on both engines, because `APP_CHROME_PX` was
measured against macOS font metrics. It is an environment constant going
stale, not a layout regression.

## Troubleshooting

**Sign-in fails with `No matching credential found`** — no Secret Service is
running. See above.

**Sign-in fails with `no config at …/.config/omacal/config.toml`** — the config
step was skipped. The message names the exact path it looked for.

**Sign-in stops working after about a week** — the OAuth app is still in
*Testing*. Publish it to Production and sign in again.

**Blank window** — check `npm --prefix ui run build` succeeds, then rerun.

**Dropdown menus and checkboxes are light on a dark theme** (AppImage,
before v0.20.0) — the AppImage's launch hook exported `GTK_THEME=Adwaita:light`,
which makes GTK ignore the dark hint omacal sets. omacal now drops that
variable at startup and keeps the widget theme on Adwaita in the variant the
palette asks for. If you export `APPIMAGE_GTK_THEME` yourself, that still
wins, as the hook intends.

**The theme does not follow `omarchy-theme-set`** — check
`~/.local/state/omarchy/current/theme` (Omarchy 4) or
`~/.config/omarchy/current/theme` (earlier) exists. omacal watches its parent
directory; if both paths are missing, live reload is silently disabled and
the app keeps the palette it started with.
