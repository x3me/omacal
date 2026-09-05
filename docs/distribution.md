# Shipping omacal: testers, production, open source

What it takes to put omacal in other people's hands — a handful of testers
first, the public eventually — recorded while the reasoning is fresh
(2026-08-11, publisher plan added 2026-08-13). Nothing here is built yet
unless it says so; this is the map, not the territory.

The one fact everything below hangs off: **the OAuth client credentials are
the real barrier, not packaging.** Today every user hand-creates a Google
Cloud project and writes `~/.config/omacal/config.toml`. No normal person
will do that, and no step below makes sense until that stops being required.

The second fact, decided 2026-08-13: **Extreme Labs publishes this, not a
person.** That is mostly a question of which accounts own which assets, and
those assets are the ones painful to migrate later — so they are born under
the company (§6).

## 1. Embed the OAuth client in the build

The `client_id`/`client_secret` pair identifies **the application**, not any
person. When a user signs in, omacal presents its client id, Google shows the
consent screen, the user approves on their own account, and the refresh token
— theirs, minted for them — lands in their keychain. Embedding the pair just
lets that dance start without a per-user Cloud project.

**The secret is not secret, by Google's own position.** For the installed-app
flow their docs state the client secret is not treated as confidential —
anything shipped in a binary is extractable by definition. Sign-in security
rests on the consent screen, the loopback redirect (the token returns only to
a process on the user's machine), and PKCE. Thunderbird, GNOME Online
Accounts and rclone all ship their pairs in public source. The realistic
worst case of extraction is brand impersonation — someone's consent screen
says "omacal" — which still cannot touch anyone's data without that person
clicking Approve, and the secret can be rotated in the Cloud Console at any
time (a new build picks it up).

**Design (implemented 2026-08-14 — `load_config_from` in `src-tauri/src/lib.rs`,
precedence pinned by four tests proven against two mutations):**

- Compile-time `option_env!("OMACAL_CLIENT_ID")` / `OMACAL_CLIENT_SECRET`,
  baked only when the release build sets them:

      OMACAL_CLIENT_ID=… OMACAL_CLIENT_SECRET=… cargo tauri build

  Zoom meeting creation is independent and uses the native/public client id
  `option_env!("OMACAL_ZOOM_PUBLIC_CLIENT_ID")`; there is deliberately no
  Zoom client secret in a desktop PKCE flow:

      OMACAL_ZOOM_PUBLIC_CLIENT_ID=… cargo tauri build

  The exact Extreme Labs Marketplace fields, native loopback redirect,
  technology-stack response, and production test checklist live in
  [`zoom-marketplace.md`](zoom-marketplace.md).

- **Precedence: `config.toml` wins when present**; the embedded pair is the
  fallback; only when neither exists does today's "no config at …" error
  appear. Developers and distro packagers keep using their own projects, and
  dev builds on this machine (no env vars set) behave exactly as today.
- The pair is **never committed**. In CI it arrives as a GitHub Actions
  secret at release-build time. It is extractable from binaries regardless —
  keeping it out of source dodges scrapers and keeps rotation meaningful.

## 2. The Google consent screen, per audience

All users of the official binaries share one Cloud project, so its state is
the product's state:

| Consent screen state | What users experience |
| --- | --- |
| **Testing** | Only listed test users (max 100) may sign in, and their refresh tokens **expire every 7 days** — the trap `running-on-macos.md` already documents. Never ship testers this. |
| **Production, unverified** | Anyone may sign in through a "Google hasn't verified this app" interstitial (Advanced → continue), but a **100-user cap** applies to new grants for sensitive scopes. Fine for a handful of testers; a wall for open source. |
| **Production, verified** | No warning, no cap. Required for real distribution. |

**Verification** is the long pole — start it early. Calendar scopes are
*sensitive* but not *restricted*, so it is the free review (homepage, privacy
policy, demonstrating scope use), not the paid security assessment
Gmail/Drive apps need. Days to weeks, once. Before submitting, **minimize
scopes**: request `calendar.events` plus a read-only calendar list rather
than the full `calendar` scope if that covers what sync and the write paths
actually do — narrower requests review faster and read less scary on the
consent screen.

Verification checks **domain ownership, not domain exclusivity**: the
homepage and privacy policy must live on a domain the Cloud project owner
has verified in Search Console. The company's existing domain qualifies —
see §7; no separate domain is required.

**Quota** is shared across all users of the embedded client. Calendar's
default (~1M queries/day) supports thousands of installs syncing every 5
minutes; the `config.toml` escape hatch is also the documented courtesy exit
(rclone's own pattern) for anyone who wants their own quota.

## 3. Artifacts, per platform

### Linux

- `cargo tauri build` produces `.deb`, `.rpm` and an **AppImage** (the
  bundler tooling downloads on first use). The AppImage is the
  hand-someone-one-file answer: download, `chmod +x`, run — Arch, Ubuntu,
  Fedora alike. `--no-bundle` (what this machine uses day-to-day) skips all
  of this and emits only the bare binary.
- The AppImage bundles the build host's ICU, whose time zone data is frozen
  at the host's (Ubuntu 22.04: ICU 70, tzdata 2021a3), and the webview's
  JavaScript takes local-time offsets from it — so a zone that changed its
  rules since, `Asia/Tehran` after 2022, drew an hour off (issue #41). The
  bundle therefore ships ICU's own time zone update files as a resource
  (`src-tauri/icu-tz/`, refreshed with its `update.sh`) and the app points
  ICU at them at startup (`src-tauri/src/icu_tz.rs`). Refresh that directory
  when a tzdata release matters to a user; the deb and rpm read the system's
  ICU and only fall back to the bundled set through the same variable.
- The icon's source is `src-tauri/icons/icon.svg`; every raster beside it,
  the favicon and the CLI logo are generated from it (the file's header
  says how). Linux bundles install the SVG as the scalable hicolor icon and
  a 512px raster besides the usual sizes (issue #39), the Flatpak manifest
  copies both, and the macOS `.icns` carries every size to 1024px.
- The released AppImage carries **AppImage update information** and ships a
  `.zsync` beside it, added by the repack step in `release.yml` (issue #27):
  `appimagetool -u "gh-releases-zsync|x3me|omacal|latest|omacal_*_amd64.AppImage.zsync"`.
  That is what AppImageUpdate, appimaged and the third-party managers use to
  find and fetch a new version, and it is separate from the app's own
  in-app updater (which reads `latest.json` and verifies a minisign `.sig`).
  The same step writes the `X-AppImage-*` desktop keys — name, version, arch,
  homepage — which is what a manager reads to recognise the app at all. The
  glob in the update information is matched against the release's assets, so
  it has to keep describing whatever the bundler names the image.
- **AUR package** for Arch/Omarchy users — the native path on this app's home
  platform: `omacal-bin` repackaging the GitHub release, optionally `omacal`
  building from source. AUR maintainership is a personal account, not an
  org's — Plamen maintains, the org owns the upstream; that split is normal.
  Eventually **Flathub** for the widest reach (with its own debugging pass —
  tray + keyring under Flatpak's sandbox is not free); package managers also
  solve updates.
- The one honest caveat for the README: sign-in stores the refresh token in
  the Secret Service, so a keyring daemon (gnome-keyring / KeePassXC) must be
  running. Desktop GNOME/KDE users have one by default; minimal-WM setups are
  the exception, and `running-on-omarchy.md` documents the fix.

### macOS

- Local builds need a Mac — no realistic cross-compile for Tauri — but
  **GitHub Actions macOS runners lift that constraint for releases**: the
  release workflow builds, signs and notarizes the `.dmg` in CI with
  secrets. A local Mac session is still the moment to regenerate the stale
  darwin snapshot baselines (`npx playwright test components.spec.ts
  --update-snapshots`).
- **Unsigned**: Gatekeeper blocks on first open; right-click → Open works and
  is acceptable for a handful of testers, and nobody else.
- **Real distribution needs an Apple Developer account** ($99/yr): Developer
  ID signing plus notarization removes the friction entirely. Distribute via
  GitHub Releases and a **Homebrew cask**.

## 4. Handover to a few testers (the short version)

1. Implement §1 (embedded credentials); confirm the consent screen is in
   Production.
2. Build: AppImage here, `.dmg` on the Mac (unsigned is fine at this scale).
3. A GitHub Release with both artifacts and a `TESTING.md`: install per OS,
   the one-time unverified-app click-through, the Gatekeeper right-click, the
   Linux keyring caveat, where to report.
4. If the repo is private: grant read access, or just send the files.

## 5. Open source, the additional furniture

- **LICENSE** — MIT/Apache-2.0 dual, the Rust convention: matches every
  dependency's expectations, imposes nothing on packagers, and the Apache
  half carries a patent grant. Copyright line: **© 2026 Extreme Labs**. All
  commits so far are personal — a one-line internal note assigning the work
  to the company closes that loop, and it must happen *before* outside
  contributions arrive, because theirs cannot be claimed retroactively.
- **DCO, not a CLA** — sign-off on commits, the kernel/GitLab standard. A
  CLA is friction this audience will resent; for MIT/Apache code the DCO is
  enough.
- **README** with a normal-person quickstart (install → launch → connect);
  the existing `docs/` already carry the deep guides.
- **CI on every PR**: `cargo test --workspace` + the Playwright suite on a
  Linux/macOS matrix — both suites already exist, this is transcription. A
  tag-triggered release workflow (`tauri-action` does most of it) builds all
  artifacts with the credentials injected from repository secrets.
- **SECURITY.md** with a company contact (`security@` the company domain),
  CONTRIBUTING.md stating the DCO and how to run both suites, issue
  templates, `cargo audit`/Dependabot.
- **Semver tags + hand-curated release notes.** The commit style here is
  expressive rather than conventional-commits — keep it; changelogs are
  written, not generated.
- **Before flipping public: scan the full history for secrets** (gitleaks or
  trufflehog). Real client ids have been in `config.toml` on dev machines
  throughout — verify none ever landed in a commit. Also decide consciously
  that `docs/superpowers/` (internal plans and specs) ships — the current
  answer is yes, it is good archaeology, but it should be a decision, not an
  accident.

## 6. Extreme Labs as publisher: which accounts own what

Everything user-facing chains back to one of these, and each is painful to
migrate later — so each is born under the company, not a person:

1. **GitHub org** — the repo goes public at
   `github.com/<extreme-labs-org>/omacal`, its final home. Transfers
   redirect clones but not reputation. Plamen stays maintainer; the org
   owns.
2. **Google Cloud project** — the OAuth client embedded in release builds
   lives in a project owned by an Extreme Labs Google account (Workspace,
   not a personal Gmail). The consent screen shows the app name and the
   verified homepage domain — that pair *is* the publisher identity users
   see. Search Console verification of the homepage domain must be done from
   this same account, so §7's domain answer precedes the verification
   submission.
3. **Apple Developer** — enroll as an **organization** (needs a D-U-N-S
   number; slower than individual enrollment, so start early). The signing
   certificate then reads "Extreme Labs", which is what belongs on a
   company-published `.dmg`.
4. **The domain** (§7) — company-owned, in the company's Search Console.

## 7. The web page

One page does triple duty: landing page, install instructions, and the
homepage + privacy policy that Google verification requires.

- **Domain: a subdomain of the existing company domain** —
  `omacal.extremelabs.<tld>` — not a separate purchase. Verification needs
  ownership, not exclusivity (§2), and a domain-level Search Console
  verification covers subdomains. The subdomain points at GitHub Pages with
  one CNAME record, touching nothing on the main site. A dedicated domain
  (`omacal.app`) buys branding only, at the cost of another verification and
  renewal; if ever wanted, it is a redirect later, not a migration.
- **Landing page**: screenshots — the app is visual, and omarchy
  theme-following is the demo — then install per platform in audience order
  (AUR one-liner, AppImage, Homebrew), then the repo link.
- **Privacy policy**, honest and short because the truth is short: no
  telemetry, no server, tokens live in the OS keychain, event data flows
  only between the user's machine and Google. For this audience that page
  is also the marketing.

## 8. Order of work

1. **Company accounts first** — GitHub org, Google Cloud project under the
   company account, subdomain + Search Console, Apple org enrollment
   started (the longest bureaucratic lead time).
2. Credentials embedding (§1) — **done** (2026-08-14).
3. LICENSE + DCO + README + CI (§5), secret-scan the history, flip public.
4. Web page with privacy policy (§7), then the verification submission (§2)
   — the long pole, so early; the 100-user cap is what actually gates
   "anyone can install this".
5. Packaging: AUR first, GitHub Release with AppImage. Then an omarchy
   community mention (Discord, ecosystem lists) — the highest-leverage
   marketing available once install is one line.
6. macOS signing + notarization + Homebrew cask; Flathub last.

Only three pieces cannot be built from this chair: the Google verification
submission, the Apple enrollment, and the Search Console verification — all
belong to the account owner, which now means Extreme Labs.
