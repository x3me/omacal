# OmaCal

**Site, downloads and the CLI guide: [omacal.app](https://omacal.app)**

A native desktop calendar for **Google Calendar, iCloud and any CalDAV
server**. Born on Omarchy Linux; runs on any Linux and on macOS. Tauri v2,
Rust, Svelte 5. **No servers**: your events live in a local database, your
tokens in your keyring, and nothing of yours passes through us.

![OmaCal's Week view](docs/images/omacal-week.webp)

## Your terminal and your agent read the same calendar

The app binary is also a CLI. Reads come straight off the local database,
offline:

    omacal agenda --json
    omacal events list --from 2026-09-01 --to 2026-09-05 --json
    omacal search quarterly review

Writes (`events create / update / delete / respond`) are carried out by the
running app through the same guards its own form has. The CLI **refuses to
guess** which occurrences of a repeating event you mean, or whether guests
get emailed. Stable JSON envelope, stable exit codes, never prompts.

Wiring an agent is one command, and every update refreshes the installed
skill:

    omacal skill install

The full guide, with real output: [omacal.app/agents](https://omacal.app/agents).

## Built into Omarchy, not just running on it

Colours follow your Omarchy theme live. Installing also installs the
`omacal.upcoming` bar widget: what is running now, today's remaining
meetings, due tasks.

## Invitations cannot slip past you

A new invitation notifies you and **stays on screen until you deal with
it**; one click accepts. The header tray keeps Yes / Maybe / No for anything
unanswered, plus who declined your meetings and what was rescheduled or
cancelled. Only the invitation itself notifies.

## It never emails people on your behalf

Moving an event with guests asks first, and *without notifying* is the
default. Saving an edit **asks who to tell**. CalDAV edits are etag-guarded:
a change that raced another device tells you instead of overwriting.

## Sign-in with nothing to configure

No API key, no cloud project. Installed builds carry OmaCal's own
Google-verified client. iCloud connects with an app-specific password; any
other CalDAV server with its URL. Task lists (VTODO) come along.

## Install

    curl -fsSL https://omacal.app/install.sh | sh

Linux x86_64 and macOS on Apple Silicon. On Linux the AppImage lands in
`~/.local/bin` with a desktop entry; on a Mac, OmaCal.app in Applications
with the `omacal` command on your PATH, signed and notarized. A `.deb`,
`.rpm` and `.dmg` are on the [releases page](https://github.com/x3me/omacal/releases).
A minimal Hyprland session needs a keyring running: gnome-keyring,
KeePassXC or kwallet.

## Updating

When a newer release exists the header grows an **Update** button; one
click fetches it, verifies the signature and restarts. Re-running the
install line does the same. A `.deb` or `.rpm` is replaced with the newer
package; AppImage managers update in place through the published `.zsync`.
`omacal doctor` prints the version you are on.

## The rest, briefly

Five views, Day to a whole-year ribbon; a week that can start today and
show the next three, five or seven days; keyboard-first, with `?` showing
every key; a list mode that leaves empty days out; search that resolves a
repeating event to one result; several accounts, with colours that stay
local; reminders that mirror what your phone fires.

**Video calls:** Add Video Call creates Google Meet through the target
calendar or a unique scheduled Zoom meeting after a one-time Zoom
connection in Settings → Accounts. The attendee link is attached before
invitations go out; pasting an existing Zoom link remains a no-login
fallback.

Building from source: [`docs/running-on-omarchy.md`](docs/running-on-omarchy.md) ·
[`docs/running-on-macos.md`](docs/running-on-macos.md). The design record
lives under [`docs/superpowers/`](docs/superpowers/).

## License

MIT — see [`LICENSE`](LICENSE).
