# omacal — wheel and touch paging in the Month and Year views

**Status:** approved 2026-10-06
**Refines:** `docs/superpowers/specs/2026-08-06-omacal-views-design.md` §5 (switching and navigation); supersedes the "no pan in Month/Year" note in `docs/superpowers/specs/2026-08-28-omacal-form-polish-and-week-pan-design.md` §4 for the paging case
**Base:** `main` @ `b7db7ee` — Week pans by day, Month/Year move only by button or key

---

## 1. What this delivers

The Month and Year views gain the gesture Week already has: a **vertical wheel**
or a **touch swipe** moves **exactly one period** — one calendar month in Month
view, one calendar year in Year view — preserving the anchor, through the same
`step(±1)` the header's `‹`/`›` and the `h`/`l` keys use. The move is **paged,
not panned**: one gesture, one period; a period is discrete, not a pixel track.

**Vocabulary** (fixed here so the code and this document agree):

- **period** — what a view shows: one Month (Month view) or one Year (Year view).
- **page** (verb *paging*) — move exactly one period; what a wheel gesture or a
  swipe does.
- **step** — the pre-existing `step(±1)` in `App.svelte` that the buttons, the
  keys, and now the pager all call. A page is a wheel-driven step.
- **pan** — the Week grid's continuous pixel slide (`WeekGrid.wheelPan`). Paging
  is **not** panning; the two are kept apart on purpose.
- **anchor** — the anchor date already defined in the views spec.
- **scrollport** — the overflowing grid element: `MonthGrid`'s `.grid`,
  `YearGrid`'s `.ygrid`.
- **edge-chain** — when a period is clipped, the wheel scrolls the scrollport
  first and pages only at its edge.

**Not in scope:** panning (continuous motion) in Month/Year; scrolling inside a
day cell to reveal a clipped `+N more`; any change to Big Year; any change to how
`‹`/`›`, `h`/`l`, `j`/`k`/`b`/`w`, or the anchor already behave.

## 2. The decisions this refines

Each was resolved on the wayfinder map `.scratch/month-year-scroll/` and is
restated here as the spec's contract:

- **Gestures and devices** — the **vertical wheel** pages. A horizontal or
  Shift+wheel is ignored: Month/Year carry only `overflow-y`, so there is no
  horizontal overflow to reserve the axis for. Trackpad two-finger scrolling
  emits the same `wheel` events; only the vertical component counts.
- **The overlay guard** — nothing pages while an overlay owns the screen. The
  predicate is the keyboard guard's own (`form`, `pendingDelete`, `searchOpen`,
  `quickAdd`, `helpOpen`), widened with the header's overlays (`settingsOpen`,
  `pickerOpen`, `gridDetail` the popover, `pendingEventMove`, `importPath`).
- **The keyboard path is untouched** — the wheel is an additional, pointer-only
  route. `h`/`l` and `j`/`k`/`b`/`w` still do exactly what they did, and remain
  the accessible route.

## 3. Axis, direction, and the overflow boundary

- **Axis: vertical, in both views.** Month is a stack of weeks in time. Year is a
  4×3 block of months with no time axis, but a vertical wheel was chosen there
  too, so the gesture is the same in both.
- **Direction: down / right = forward.** A wheel-down pages to the next period;
  a wheel-up pages back. This is the mainstream convention (Thunderbird,
  KOrganizer, GNOME).
- **Edge-chain.** When a short window has clipped the period, the scrollport can
  scroll *and* the gesture could page. The wheel scrolls the clipped grid first
  and pages only once the scrollport is at the edge in the gesture's direction.
  This mirrors Thunderbird's month view, which stops propagation so a clipped
  list scrolls before the month changes. A period that fits (the common case)
  pages immediately.
- **Year boundary.** Paging in Month view wraps cleanly (January back to
  December of the previous year) and the header reads month and year together;
  `step`'s existing clamping is unchanged.

## 4. The feel

- **One period per gesture, and no runaway.** Vertical wheel travel accumulates
  in `periodscroll.ts`; crossing **~50 px** fires **one** page. A **~350 ms
  lock** after the page drops the trackpad's momentum tail — without it a single
  flick pages across many periods. An **idle reset** (~180 ms) drops
  sub-threshold travel, so a slow, deliberate drag is not a gesture and cannot
  bank travel across separate ones. A single mouse notch exceeds the threshold,
  so one notch is one page.
- **The snap: animated.** A page animates the new period in over **~260 ms**,
  eased (`cubic-bezier(.22,.61,.36,1)`), from the direction of travel. This is
  what "smooth" means here; it is **not** continuous motion.
- **Reduced motion.** Under `prefers-reduced-motion: reduce`, the snap becomes an
  **instant swap** — still exactly one period, no travel.
- **The header title swaps in sync.** It lives in `Header.svelte`, outside the
  grid, and does not animate separately; the grid's slide carries the motion.

## 5. The mechanism

- **One chokepoint.** A page calls `step(±1)`. The buttons, the keys, and the
  pager therefore cannot drift.
- **`ui/src/lib/periodscroll.ts`** — the pure decisions: the accumulator, the
  lock, the idle reset, the strong-swipe rule, and `prefersReducedMotion`. No
  DOM, so each rule is tested directly (the shape of `zoom.ts`/`weekwindow.ts`).
- **`ui/src/lib/periodpaging.ts`** — the listeners, attached by a Svelte action
  on `App`'s `.view` container (`.view` holds the grid, so edge-chain can read
  its scrollport). `WeekGrid.wheelPan` is **not** generalised: it is bespoke to
  the week's pixel-column track (samples, RAF flush, spring settle, touch).
- **Prefetch.** The snap slides *to* the next period, so `App` keeps the **next**
  period fetched on the page that settles (`prefetchNext`), and a cached period
  draws with no IPC. The cache is cleared on every view switch. This narrows the
  earlier "neither prefetch nor cache" finding, which was measured against an
  instant swap; a failed prefetch is not a failure, since the page fetches as it
  always did.

## 6. Touch

- **A strong vertical swipe pages; a gentle one does nothing.** The rule reuses
  `#118`'s measured strong-swipe thresholds (`STRONG_SWIPE_PX_PER_MS = 2`,
  `STRONG_SWIPE_MIN_PX = 40`) from `weekwindow.ts`, but none of its track or
  spring — Month/Year have no track to slide, so a gentle swipe deliberately does
  nothing rather than following the finger.
- **Direction is content-drag:** the sheet follows the finger, so finger-up pages
  forward. This is the opposite of the wheel's down-as-forward only because a
  drag moves content while a wheel moves the viewport.
- **Tap is preserved.** A swipe suppresses the click the browser synthesises
  after a touch release, so a swipe across a day cell never also picks that day;
  a tap still picks the day exactly as before.
- **Edge-chain for touch** is the simple reading: a clipped period scrolls under
  the finger natively, so touch pages only a period that fits. (The wheel's
  edge-chain is exact; touch's is deliberately coarser.)

## 7. Alternatives considered

**The Week grid's continuous pan.** Week moves by dragging a track of days under
the finger, handing whole days up as they are crossed and springing to a boundary
(`WeekGrid.wheelPan`; the primitives — `velocityOf`, `settleTarget`,
`springPlan`/`springAt`, `padFor`, `windowHeld` — live in `weekwindow.ts`). A
continuous Month/Year version was considered and **set aside**: the discrete page
is simpler, and it preserves the "one period per gesture" promise exactly, where a
track that follows momentum does not. The pan was re-raised 2026-10-07 after the
snap shipped and consciously deferred, not forgotten — the choice is put to the
upstream author on the pull request. Because the pan primitives are already
shared, it is a tractable follow-up if preferred; only the Month/Year rendering
and the discrete machinery here would change.

## 8. Testing

- `ui/tests/periodscroll.spec.ts` — the pure rules directly: one notch is one
  page; small events gather to the threshold; the lock swallows the tail and a
  fresh flick after it pages; the idle reset stops banking; a strong swipe pages
  and a gentle one does not.
- `ui/tests/app.spec.ts` — the wiring through the real `App` with the stubbed
  IPC: a wheel notch pages Month forward and its tail does not; a notch pages
  Year by a year; no page while the search overlay is open; none in Week view;
  reduced motion still pages exactly one period.
- Per the house standard, each rule was shown **red against deliberately broken
  code** (the lock, the threshold, the `step` call, and the guard) before its
  green.

## 9. Definition of done

- A vertical wheel over the Month grid pages one month; over the Year grid, one
  year; the anchor survives.
- A trackpad flick pages once, not many times; a slow drag does not page.
- A clipped period scrolls before it pages.
- Nothing pages behind an overlay, and the keyboard path is unchanged.
- A touch swipe pages one period; a tap still picks the day.
- Reduced motion pages without travel.
- `npm --prefix ui run check` and `npm run test:ui` pass.
