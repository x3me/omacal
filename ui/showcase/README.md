# Showcase: the website's screenshots

The real interface, rendered in the test harness from payloads the real
backend produced. Nothing runs on your desktop.

```sh
ui/showcase/dump.sh                                   # demo seed → target/showcase/*.json
(cd ui && npx playwright test -c showcase/playwright.config.ts)   # → target/showcase/shots/*.png
ui/showcase/compose.sh                                # → target/showcase/site/*.webp
```

- **The data** is `fixtures::seed_demo` (the same seed `OMACAL_SEED_DEMO=1`
  shows), answered by the read commands through `showcase_payloads`, an
  ignored test in `src-tauri/src/lib.rs`. Change the demo there, not here.
- **The moment** is Wed 2026-10-14 11:40 in Europe/Berlin (`SHOWCASE_NOW`).
- **The palettes** are resolved from Omarchy's theme files by the app's own
  `theme::resolve`; `SHOWCASE_THEMES` picks which.
- **Not part of the suite.** Its own config, its own directory; it gates
  nothing and nothing gates on it.

The one image this cannot make is the Omarchy bar with the widget in it:
that is the real desktop, taken by hand.
