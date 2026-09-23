# Contributing to OmaCal

Thanks for even considering it. Here is everything you need to be productive
in the first hour.

## You do not need a Google Cloud project

This is the part that surprises people: **demo mode is the development
environment.**

    npm --prefix ui install               # once
    OMACAL_SEED_DEMO=1 cargo tauri dev

That seeds a year of synthetic events into a separate database, blocks every
network call, and gives you the full app — every view, create/edit/delete,
drag, search, reminders scheduling — with nothing real at stake and no
credentials of any kind. Most UI and logic work never needs more.

If you do need real Google data, see "Bring your own Google credentials" in
the README and the run guides in `docs/`.

## Toolchain

Rust stable, Node 22, and the webview stack:

- Arch: `webkit2gtk-4.1 gtk3 libayatana-appindicator`
- Debian/Ubuntu: `libwebkit2gtk-4.1-dev build-essential libxdo-dev
  libssl-dev libayatana-appindicator3-dev librsvg2-dev`

## The suites

    cargo test --workspace --no-fail-fast   # Rust: every crate
    npm --prefix ui run check               # svelte-check + tsc
    npm --prefix ui run test:ui             # Playwright, WebKit + Chromium

CI runs all three on every pull request, plus `cargo clippy --workspace
--all-targets -- -D warnings`. Two Playwright caveats: screenshot goldens are
rendered on specific machines, so run with `--ignore-snapshots` on other
distros (CI does); and the CI workflow's header names the handful of specs it
skips and why.

## The testing standard

[`docs/testing-standard.md`](docs/testing-standard.md) is short and it is the
house rule that matters most: **a test is not trusted until it has been shown
to fail against deliberately broken code.** Delete the rule you are pinning,
watch the test go red, restore from a copy, watch it go green — and say so in
the commit body ("proven red against X"). Tests that have not earned their
green this way tend to get rewritten in review.

## Two rules that fail quietly

Neither of these reddens anything when you get it wrong, which is why they
keep arriving as review comments instead of as test failures.

**A new message shown to the user has to be allow-listed.**
`src-tauri/src/errors.rs` holds `SAFE_EXACT` and `SAFE_PREFIXES`; anything
that does not match one is replaced by "Sync failed. See the application log
for details." That default is deliberate — an error can carry a URL, an
address or a token, and a deny-list only ever covers the secrets it was told
to name. So if you add a refusal the user is meant to act on:

- make it a `pub(crate) const`, interpolating nothing (or only a benign,
  genuinely variable tail — that is what `SAFE_PREFIXES` is for);
- add it to the right list with a comment citing the call site that raises it
  and confirming nothing wraps it in `.context(..)` on the way;
- name it in `every_message_the_app_relies_on_showing_is_still_allowlisted`
  (or its prefix twin), which fails on a count mismatch if you forget;
- and test that it is not `OPAQUE`.

Skipping this does not fail the build. It just means a user who mistyped
something reads a sync-fault report, which is how "that is not a calendar
address" reached people as "Sync failed. See the application log."

**A change to the CLI updates `skills/omacal/SKILL.md` in the same commit.**
The skill is embedded in the binary with `include_str!` and refreshed on
users' machines by `omacal skill install`, so a stale one is an agent being
told about a flag that no longer exists. Same commit, not a follow-up.

## Where to start

The codebase pushes logic out of the integration layers and into pure,
test-reachable modules — those are the friendly entry points:

- `crates/omacal-core` — lane packing, layout, recurrence. Pure functions,
  no IO, dense test suites to copy the style from.
- `ui/src/lib/*.ts` — `drag.ts`, `status.ts`, `position.ts`, `filmstrip.ts`:
  the decisions behind the Svelte components, tested directly.
- The Svelte components and `src-tauri/src/lib.rs` are the integration skin;
  they mostly delegate to the above.

## Style

- Do not run `rustfmt` — the tree deliberately does not follow it, and a
  reformat drowns the diff. Match the code around you.
- Comments explain *why* and record the incident that made a rule necessary;
  the git history is full of examples to imitate.
- Commit subjects are lowercase and conventional-commit flavoured
  (`fix(sync): …`), and the body tells the story.

## Releases

Maintainers cut releases; the pipeline embeds credentials contributors do not
have, which is also why a source build needs your own (see the README). You
never need any of that to develop or to land changes.
