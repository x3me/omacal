# The Flatpak build

> **Status: unblocked, submission parked (2026-08-24).** The manifest builds,
> installs and runs on this machine, pinned to **v0.4.0 — the release that
> carries the `FLATPAK_ID`-aware `single_instance_plugin`**, so the
> never-granted `--own-name` is gone from `finish-args` and the linter is
> down to the one error that is granted on explanation
> (`finish-args-login1-system-talk-name`). Nothing has been submitted to
> Flathub yet, by choice.
>
> **To pick it up, start at "Before you submit" below** — step 1 is done;
> what remains is the logind exception PR and the submission itself.


Every other install path omacal has is distro-shaped: the AppImage, the
`.deb`, the `.rpm`, the AUR recipe. This one is not, and it is also the only
channel that answers *how many people run this* — Flathub publishes install
counts per app, publicly, with no telemetry in the binary:

    curl -s https://flathub.org/api/v2/stats/io.extremelabs.omacal

That returns `installs_total` and a full `installs_per_day` series. GitHub's
asset counter cannot tell a returning user from a new one; this can.

## What it builds

`io.extremelabs.omacal.yml` **repackages the official release `.deb`** rather
than building from source, for exactly the reason `packaging/aur/PKGBUILD`
does: release artifacts carry the Google OAuth client pair, embedded at
release-build time (`docs/distribution.md` §1). A source build produces a
binary that cannot sign in without the user creating their own Cloud project.

Four things ride along with it:

| File | Why it exists |
| --- | --- |
| `io.extremelabs.omacal.desktop` | The `.deb`'s own entry is named `omacal`, and a Flatpak's must carry the app id |
| `io.extremelabs.omacal.metainfo.xml` | AppStream metadata — required by Flathub, validated in its CI |
| `bump.sh` | Points the manifest at a new release: URL, checksum, `<release>` entry |
| `shared-modules/` | Flathub's submodule, used for one thing: intltool |

The tray comes from the Ayatana stack — libdbusmenu, ayatana-ido,
libayatana-indicator, libayatana-appindicator — built inline rather than from
shared-modules' `libappindicator` recipe. That recipe would be one line, but
it pulls dbus-glib from `dbus.freedesktop.org`, which it lists no mirror for
and which was unreachable when this was written. Ayatana needs neither
dbus-glib nor libindicator, and it provides
`libayatana-appindicator3.so.1` — the name omacal's dlopen tries *first*,
ahead of the `libappindicator3.so.1` fallback.

The app id is deliberately **not** the Tauri identifier. `com.omacal.app`
claims a domain nobody here owns, and Flathub requires an id under a domain
the publisher controls. `extremelabs.io` is already the verified domain on
the Google consent screen, so it is the natural one. The Tauri identifier
stays put — changing it would move every existing user's data directory.

## Build it

Arch/Omarchy needs the tooling first; neither is installed by default:

    sudo pacman -S flatpak flatpak-builder
    flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    flatpak install -y flathub org.gnome.Platform//50 org.gnome.Sdk//50

Then, from the repo root:

    git submodule update --init packaging/flatpak/shared-modules
    flatpak-builder --force-clean --user --install \
        build packaging/flatpak/io.extremelabs.omacal.yml
    flatpak run io.extremelabs.omacal

The first build compiles the appindicator stack and takes a few minutes;
after that it is cached.

To produce the single-file bundle that can hang off the releases page next to
the AppImage:

    flatpak-builder --force-clean --repo=repo \
        build packaging/flatpak/io.extremelabs.omacal.yml
    flatpak build-bundle repo omacal-0.3.14.flatpak io.extremelabs.omacal

That `.flatpak` installs with `flatpak install ./omacal-0.3.14.flatpak` and
needs no repository at all — worth attaching to releases whether or not
Flathub ever happens.

## Bump it

    packaging/flatpak/bump.sh            # follow whatever GitHub calls latest
    packaging/flatpak/bump.sh 0.3.15     # or name the version

It downloads the `.deb`, computes the checksum, and rewrites the manifest and
the `<release>` entry. Once the app is on Flathub,
flatpak-external-data-checker reads the `x-checker-data` block in the
manifest and opens that same bump as a pull request on its own.

## Publishing to Flathub

Verified against docs.flathub.org on 2026-08-24, and the linter was run
locally against this manifest.

### Before you submit

1. **Cut a release containing the single-instance fix.** `--own-name` for a
   name outside the app id is a linter error Flathub documents as *"never
   granted"* — no exception exists for it. `src-tauri/src/lib.rs`
   (`single_instance_plugin`) now derives the bus name from `FLATPAK_ID`
   when it is set, so inside the sandbox the name becomes
   `io.extremelabs.omacal.SingleInstance`, needs no permission, and every
   other build keeps the name it always had. Because this manifest
   repackages a *released* `.deb`, the fix only reaches the Flatpak once a
   release carries it.
2. **Then run `./bump.sh <that version>` and delete the
   `--own-name=com.omacal.app.SingleInstance` line from `finish-args`.** It
   is still there today because the pinned 0.3.14 predates the fix and needs
   it; shipping without it on an old build gives you two full instances,
   two trays and doubled reminders.
3. **Request the logind exception.** `--system-talk-name=org.freedesktop.login1`
   is a linter error too, but this one is *"granted on sufficient
   explanation"*. It needs its own pull request to Flathub's
   [exceptions file](https://github.com/flathub/flatpak-builder-lint/blob/master/flatpak_builder_lint/staticfiles/exceptions.json),
   one entry for this app. The explanation is `src-tauri/src/resume.rs`:
   logind's `PrepareForSleep` is the only signal that a laptop woke up, and
   without it every lid-close leaves the calendar stale.

`flathub.json` pinning `only-arches: ["x86_64"]` is already here and is
required — Flathub builds aarch64 by default and the pinned `.deb` is
amd64-only.

### Build and lint the way Flathub does

    flatpak install -y flathub org.flatpak.Builder
    flatpak run --command=flathub-build org.flatpak.Builder --install \
        packaging/flatpak/io.extremelabs.omacal.yml
    flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
        manifest packaging/flatpak/io.extremelabs.omacal.yml
    flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo

The manifest check must come back with an empty `errors` list, bar the
exception granted in step 3.

### The submission pull request

    gh repo fork --clone flathub/flathub && cd flathub
    git checkout --track origin/new-pr
    git checkout -b omacal-submission new-pr

Copy the four files — manifest, metainfo, desktop entry, `flathub.json` —
into the repository root, commit, push, and open a pull request **against
the `new-pr` branch** (never `master`), titled
`Add io.extremelabs.omacal`. Do not delete the PR template; submissions that
strip it get closed unreviewed, as do ones that read as bulk AI output.

A reviewer comments; `bot, build` on the PR starts a test build. Never close
and reopen the PR to address feedback — app id changes included, it all
happens in place. On approval it is merged into a fresh
`flathub/io.extremelabs.omacal` repository and you get a write invitation.

### After the merge

**Verification** — the "verified" badge, distinct from being published. Log
in to Flathub, open the Developer Portal, pick the app, click Verification,
and it hands you a token to place at
`https://extremelabs.io/.well-known/org.flathub.VerifiedApps.txt`. That file
is `public/.well-known/…` in the extremelabs.io repo, so it ships with the
next deploy of the site.

**Updates never repeat this process.** Push the bumped manifest to the
`flathub/io.extremelabs.omacal` repo, or let
flatpak-external-data-checker open that pull request for you off the
`x-checker-data` block already in the manifest.

### Two things a reviewer may still raise

- **"Build from source."** The credentials argument above is the reason, and
  it is the same one the AUR recipe documents. The alternative is committing
  a Google client secret into a public manifest repo, which invites automated
  secret-scanning rotation — and a rotated secret strands every binary
  already shipped. If Flathub insists, the fallback is `extra-data`, which
  fetches the `.deb` on the user's own machine at install time.
- **`--talk-name=org.freedesktop.Notifications`.** Flathub prefers the
  notification portal. omacal's invitation notifications carry *actions*
  (clicking accepts the series), which is the whole point of them, and
  `notify-rust` talks to the session bus directly. Moving to
  `org.freedesktop.portal.Notification` is a code change, not a manifest one.
  It is not currently a linter error.

## What the sandbox changes

Nothing breaks, but three behaviours quietly go away, and all three are
already guarded in the code rather than needing a patch:

- **Omarchy theme following** reads `$HOME/.local/state/omarchy/current/theme`.
  Inside the sandbox that path does not exist, so `omarchy_theme_dir()`
  returns `None` and the built-in palette is used. Granting
  `--filesystem=~/.local/state/omarchy:ro` would restore it, at the cost of a
  permission Flathub would question for a non-Omarchy audience — which is
  the entire audience for this build.
- **The bar widget** installs only where `~/.config/omarchy` already exists.
  It does not in the sandbox, so `plugin_dir()` returns `None` and the module
  leaves no trace, exactly as designed for non-Omarchy machines.
- **Autostart** does not work. `tauri-plugin-autostart` writes
  `$XDG_CONFIG_HOME/autostart/*.desktop`, which the sandbox redirects into
  `~/.var/app/…` where the host session never looks. The fix is the Background
  portal's `RequestBackground(autostart: true)`, which is a code change; until
  then the Settings toggle is a no-op in this build and should be hidden when
  `/.flatpak-info` exists.

Sync on resume *does* work, but only because `--system-talk-name=org.freedesktop.login1`
is granted explicitly: the watcher listens for logind's `PrepareForSleep` on
the system bus, which a sandbox cannot see by default. Without that line the
app logs `suspend watcher unavailable` at startup and every lid-close leaves
the calendar stale until something else nudges it. Expect a Flathub reviewer
to ask about this one; the answer is `src-tauri/src/resume.rs`.

The database lands in `~/.var/app/io.extremelabs.omacal/data/com.omacal.app/`
and persists normally. `config.toml`, read from `$HOME` directly rather than
through XDG, gets `--persist=.config/omacal` so the power-user credential
override has a durable home at
`~/.var/app/io.extremelabs.omacal/.config/omacal/config.toml`.
