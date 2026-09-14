<!-- ui/src/lib/WeekGrid.svelte -->
<script lang="ts">
  import { visibleHours } from "./visiblehours.svelte";
  import { clockFormat } from './clock.svelte';
  import { gutterWidth, secondZone } from './secondzone.svelte';
  import { temperatureUnit } from './tempunit.svelte';
  import { formatTemp } from './temperature';
  import WeatherGlyph from './WeatherGlyph.svelte';
  import { dateKey, type DayWeather } from './weather';
  import type { TaskChip } from './taskchips';
  import { formatClock, gutterLabel, zoneAbbrev, zoneGutterLabel } from './timefmt';
  import { tick, untrack } from 'svelte';
  import { HOUR_PX_DEFAULT, hourPxAfterPinch, hourPxAfterWheel, scrollTopKeeping } from './zoom';
  import { onPinch, type Pinch } from './pinch';
  import {
    BAND_ROWS, FLING_TAU_MS, flingProgress, flingTravel, packBandLanes, panCommit, sliceWeek, snapPlan,
    velocityOf, visibleIndex, type PanSample,
  } from './weekwindow';
  import type { DayColumn, Lane, WeekPayload, UiEvent } from './api';
  import type { Calendar } from './calendars';
  import { containsPlacement } from './combined';
  import type { Rect } from './position';
  import EventBlock from './EventBlock.svelte';
  import AllDayBand from './AllDayBand.svelte';
  import EventPopover from './EventPopover.svelte';
  import { getEventDetail, refreshEvent, type EventDetail, type Occurrence } from './eventdetail';
  import {
    SNAP_MS, beganDrag, colsMoved, edgeAt, spanForMove, spanForResize, sweepAsk,
  } from './drag';
  import { cursorNamesEvent, type KeyboardCursor } from './keyboardnav';
  import { dateOf } from './eventform';

  let { week, calendars = [], weather = null, weatherStale = false, onweather = null, tasks = null, ontaskmove = null, ontasktoggle = null, formPreview = null, createColor = null, revealNowRequest = 0, keyboardCursor = null, onpan = null, hourPx = $bindable(HOUR_PX_DEFAULT), visibleStartMs = null, visibleDays = null, onerror = null, oncreate, oncreateallday, onedit, ondelete, oncopy, onduplicate, onmove, ondraftmove = null, onresponded }: {
    /** Padded since 2026-09-03: `visibleDays` from `visibleStartMs` are what
     *  is on screen, and the days either side are the track's to slide into
     *  under a finger (`weekwindow.ts`). Both null — a standalone mount, a
     *  stub — shows the whole payload, which is what every payload used to
     *  be. */
    week: WeekPayload;
    visibleStartMs?: number | null;
    visibleDays?: number | null;
    /** How tall an hour is (2026-09-03). Bound, because both ends write it:
     *  the grid, from a pinch or Ctrl+scroll over itself, and `App`, from the
     *  keys and the stored preference. `zoom.ts` owns the arithmetic and the
     *  range; this grid only ever asks it. */
    hourPx?: number;
    /** A horizontal wheel/trackpad gesture asking the window to slide by
     *  whole days — positive is forward. Optional: a grid without it (a
     *  future embedding) simply keeps the wheel native. */
    onpan?: ((days: number) => void) | null;
    /** The forecast by ISO date (`weather.ts`), or null for none — off, not
     *  yet fetched, or failed all look the same here: a header with no sky,
     *  which is what this header looked like for its whole life until now. */
    weather?: Map<string, DayWeather> | null;
    /** Whether the forecast behind `weather` is old enough to warn about
     *  (`weather.ts`'s `freshness`). The glyph fades and says so, so a
     *  stale sky is noticed without opening a card. */
    weatherStale?: boolean;
    /** Tasks due in this week, by the ISO date they fall on (`weather.ts`'s
     *  `dateKey`). Null or empty draws no row at all: a strip of chrome for
     *  a week with nothing due is the cost Option B was warned about, and
     *  it is avoidable. */
    tasks?: Map<string, TaskChip[]> | null;
    /** A task chip was dragged to another day: its id and that day's start.
     *  The grid decides which column, never what a due date becomes — the
     *  hour and the all-day flag are the caller's to keep. */
    ontaskmove?: ((id: number, dayStartMs: number) => void) | null;
    ontasktoggle?: ((id: number, done: boolean) => void) | null;
    /** The sky in a day header was clicked: that day's start and the
     *  glyph's own rect, for App to open the weather card over. Optional —
     *  a grid without it keeps the glyph as the label it always was. */
    onweather?: ((dayStartMs: number, anchor: import('./position').Rect) => void) | null;
    /** The span the open event form currently describes, drawn as a dashed
     *  ghost so the user watches the event land while typing its times —
     *  create and edit alike. Null draws nothing. */
    formPreview?: import('./eventform').FormGhost | null;
    /** The open form's draft was dragged or resized to this span. Fired
     *  continuously through the gesture, not once at the end: the ghost is
     *  drawn from `formPreview`, which comes back from the form, so the
     *  draft follows the pointer only because each move round-trips. Null
     *  leaves the ghost inert, which is what it was before it could be
     *  grabbed at all. */
    ondraftmove?: ((span: { startMs: number; endMs: number }) => void) | null;
    /** The colour a create started here would land in — the calendar the form
     *  will open pre-set to. The sweep wears it so the gesture and the draft
     *  it becomes are the same colour as well as the same shape: releasing
     *  the button must change the ribbon's dress, not its identity. Null
     *  falls back to `--accent`, which is also what an unwritable-to app
     *  (no calendar a create can land on) draws. */
    createColor?: string | null;
    /** Incremented by App for every explicit Today action, including when the
     *  anchor already names today and therefore no payload navigation occurs. */
    revealNowRequest?: number;
    keyboardCursor?: KeyboardCursor | null;
    /** A sweep that crossed day columns: an all-day span, first day to last
     *  inclusive. Separate from `oncreate` rather than a flag on it — the two
     *  ends here are *days*, and a caller reading a wall clock off them would
     *  be reading something that was never there. */
    oncreateallday: (firstDayMs: number, lastDayMs: number, rect: Rect) => void;
    /** A click on empty space in a day column, at the half hour it landed in,
     *  or a **sweep** across it, which names an `endMs` as well.
     *  `rect` is the anchor to put the form beside — the column at the height
     *  of the click, so the form appears next to where the user pointed.
     *
     *  One callback for both, deliberately: a sweep and a click ask for the
     *  same thing — the event form, opened on a time — and differ only in
     *  whether the grid knows how long. A second prop would be a second way to
     *  create an event, and this grid still creates none of them itself. */
    oncreate: (startMs: number, rect: Rect, endMs?: number) => void;
    /** Edit was clicked in this grid's own popover. The `Occurrence` carries
     *  the *clicked block's* own `start_ms`/`end_ms` alongside the detail —
     *  never `detail.start_ms`, which for a series is the master's DTSTART.
     *  See `eventdetail.ts`'s `updateEvent`. */
    onedit: (occurrence: Occurrence, rect: Rect) => void;
    /** Delete was clicked there, carrying the same `Occurrence` for the same
     *  reason. Nothing is deleted by this: the caller confirms first. */
    ondelete: (occurrence: Occurrence, rect: Rect) => void;
    /** Ctrl+C landed in that popover: `App` should hold this occurrence as
     *  what Ctrl+V pastes. Not through `relay` — a copy leaves the popover
     *  open, the way every selection survives being copied. */
    oncopy: (occurrence: Occurrence) => void;
    calendars?: Calendar[];
    onduplicate: ((occurrence: Occurrence, rect: Rect) => void) | null;
    /** A completed drag, handed up rather than written here: the grid decides
     *  *which occurrence* moved and *where to*, and `App` owns every write —
     *  the same split `oncreate`/`onedit`/`ondelete` already use. `WeekGrid`
     *  contains no `invoke`, and that is a property worth keeping. */
    onmove: (event: UiEvent, span: { startMs: number; endMs: number }) => void;
    /** A click that could not be answered — the event's detail would not
     *  load. Handed up because `App` owns the one error line the window
     *  has; the grid has nowhere of its own to say it. */
    onerror?: ((message: string) => void) | null;
    /** Told after a successful RSVP, so `App` reloads the payload. The
     *  `responseOverrides` restyle below is display only: the struck block
     *  still carries the master's row id, and reopening it fetched the
     *  master's own answer — eternally the old one — until a reload swapped
     *  the exception row in (seen live, 2026-08-11: decline, reopen, "Yes"). */
    onresponded?: () => void;
  } = $props();

  // The window: how many days are on screen, where they start in the
  // payload, and the payload as if only they had been fetched. Everything
  // about *what is on screen* — today, the ruler's reference day, the
  // opening scroll — reads `visibleWeek`; only the track below ever draws
  // the padding.
  const vis = $derived(visibleIndex(week.days, visibleStartMs ?? week.days[0]?.start_ms ?? 0));
  // A payload without the window — see `visibleIndex` — is shown whole.
  const visible = $derived(vis < 0 ? week.days.length : (visibleDays ?? week.days.length));
  const visStart = $derived(Math.max(vis, 0));
  /** Whether the band shows every row or folds the rest behind "+N more".
   *  Sticky across navigation on purpose: a week with nothing hidden draws
   *  the rows it needs either way, so leaving it open costs nothing and
   *  closing it on every step would fight the user who opened it. */
  let bandExpanded = $state(false);
  const bandRows = $derived(bandExpanded ? Infinity : BAND_ROWS);
  const visibleWeek = $derived(sliceWeek(week, visStart, visible, bandRows));

  // Every hour, not every second one: a rule at 10:00 with nothing at 11:00
  // makes a meeting's edge unplaceable by eye.
  const HOURS = $derived(Array.from({ length: visibleHours().end - visibleHours().start }, (_, i) => i + visibleHours().start));
  const visibleHeight = $derived(Math.round(hourPx) * (visibleHours().end - visibleHours().start));
  const DOW = ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT'];
  // Named from the day's own date, not its position in the week — the same
  // rule works for a 7-column week and a 1-column day.
  const dayName = (ms: number) => DOW[new Date(ms).getDay()];

  // Where wall-clock hour `h` falls in a column, as a fraction of that column's
  // *true* span. Both halves matter on a DST day: the span is 23 or 25 hours,
  // and the elapsed time to 09:00 is not 9 hours if the clocks moved overnight.
  // Reading the hour back off a Date gets both right, in the same zone the
  // events were laid out in. Rust computes the geometry against these same
  // boundaries, so blocks and rules cannot drift apart.
  const hourFrac = (day: { start_ms: number; end_ms: number }, h: number) => {
    const d = new Date(day.start_ms);
    d.setHours(h, 0, 0, 0);
    return (d.getTime() - day.start_ms) / (day.end_ms - day.start_ms);
  };

  function crop(day: { start_ms: number; end_ms: number }) {
    const start = hourFrac(day, visibleHours().start), end = hourFrac(day, visibleHours().end);
    return end > start ? { start, span: end - start } : { start: 0, span: 1 };
  }
  function columnStyle(day: { start_ms: number; end_ms: number }) {
    const range = crop(day), height = visibleHeight / range.span;
    return `height:${height}px;min-height:0;top:${-range.start * height}px`;
  }
  // The gutter labels are shared by all seven columns, so they use the first
  // ordinary-length day; a DST day's own rules still come from its own span.
  const gutterDay = $derived(
    visibleWeek.days.find((d) => d.end_ms - d.start_ms === 86_400_000) ?? visibleWeek.days[0]
  );

  // The instant a primary hour line marks — `hourFrac`'s own Date, kept as
  // milliseconds, so the second zone's label describes exactly the rule it
  // sits beside (and lands on `:30` where the zones are half an hour apart,
  // which is the honest reading, not a rounding error).
  const hourMs = (day: { start_ms: number }, h: number) => {
    const d = new Date(day.start_ms);
    d.setHours(h, 0, 0, 0);
    return d.getTime();
  };

  // The zone this grid is laid out in — the process's own, which is the
  // display zone when one is set. Named only when a second clock appears:
  // one clock needs no label, two clocks unlabelled are a guessing game.
  const primaryZone = Intl.DateTimeFormat().resolvedOptions().timeZone;

  // Current-time line, recomputed each minute. Held as an instant and divided by
  // the column it lands in, rather than assuming a 1440-minute day.
  //
  // The focus listener is the suspend story: a laptop that sleeps past
  // midnight wakes with the interval up to a minute out, and the first thing
  // the user does is focus the window — snapping the clock then is what
  // makes "today" already right when they look.
  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => { nowMs = Date.now(); }, 60_000);
    const snap = () => { nowMs = Date.now(); };
    window.addEventListener('focus', snap);
    return () => { clearInterval(id); window.removeEventListener('focus', snap); };
  });

  // **Derived from the ticking clock, never computed once.** This was a
  // plain `const` evaluated at mount, and an app left running overnight —
  // or through a suspend — kept yesterday ringed as today while the
  // current-time line, whose column is chosen by this value, ran off the
  // bottom of yesterday and vanished (seen live, 2026-08-19, after a night
  // of sleep with the app open).
  const todayStart = $derived.by(() => {
    const d = new Date(nowMs);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  });

  // Opening at midnight puts the working day off-screen. Preserve the existing
  // once-on-mount placement at one third of the viewport; weeks without today
  // open at 08:00. An explicit Today request instead puts now at 45%, and
  // repeats even when the anchor was already today—which is why it cannot be
  // inferred from a `week` prop change.
  //
  // Deliberately once, not on every week change: navigating away and back should
  // keep where you were looking, which is what every desktop calendar does.
  let bodyEl: HTMLDivElement | undefined = $state();
  let hasScrolled = false;

  // ---- Zoom: the one absolute number, and keeping the pointer's instant put.
  /** The height the columns currently draw at. Compared against `hourPx` in
   *  the effect below to tell a change apart from a re-run. */
  let appliedPx = hourPx;
  /** Where the next change should hold still, in pixels below the body's
   *  top edge: the pointer, for a wheel or a pinch. `null` — a key, or the
   *  stored preference arriving — holds the middle of the pane. */
  let zoomAnchorY: number | null = null;
  /** The height when the pinch began; its cumulative scale applies to this. */
  let pinchStartPx = 0;

  /** Rescale the scroll so the instant under the anchor stays under it.
   *  `$effect.pre`, because `scrollTop` has to be read *before* the columns
   *  change height — afterwards a zoom-out may already have clamped it. The
   *  write waits for the DOM, since until then the taller column does not
   *  exist to scroll into. */
  $effect.pre(() => {
    const target = hourPx;
    if (target === appliedPx) return;
    const el = bodyEl;
    if (!el) { appliedPx = target; return; }
    const anchorY = zoomAnchorY ?? el.clientHeight / 2;
    const next = scrollTopKeeping(el.scrollTop, anchorY, appliedPx, target);
    appliedPx = target;
    zoomAnchorY = null;
    tick().then(() => { el.scrollTop = next; });
  });

  /** A pinch over the grid. Anywhere else in the window — the Linux event
   *  is app-wide — is not this grid's to answer. Above the body counts: the
   *  day headers are the grid too, and a pinch that starts on a date and
   *  drifts is one gesture. */
  function handlePinch(p: Pinch) {
    if (!bodyEl) return;
    const r = bodyEl.getBoundingClientRect();
    if (p.clientX < r.left || p.clientX > r.right || p.clientY > r.bottom) return;
    if (p.phase === 'begin') { pinchStartPx = hourPx; return; }
    if (p.phase === 'end') { pinchStartPx = 0; return; }
    // An update with no begin — the gesture began off the grid and drifted
    // on — takes the current height as its start, undoing the scale it has
    // already accumulated, so it continues from here rather than jumping.
    if (!pinchStartPx) pinchStartPx = hourPx / p.scale;
    zoomAnchorY = Math.max(0, p.clientY - r.top);
    hourPx = hourPxAfterPinch(pinchStartPx, p.scale);
  }

  $effect(() => {
    if (!bodyEl) return;
    return onPinch(bodyEl, handlePinch);
  });

  // ---- Sideways: the track under the finger.
  /** The finger's travel in columns, positive when the content has moved
   *  right; the fraction of a column the track is currently offset by. Whole
   *  columns are handed up as a shift of the window the moment they are
   *  crossed (`panCommit`), and the offset compensates in the same flush, so
   *  the day under the finger never moves — the window's start moves
   *  through the padding instead, and the payload for the new window
   *  arrives underneath whenever it does. */
  let panDays = $state(0);
  /** While a swipe is in progress or settling, the track draws every day the
   *  payload holds, padding included, so there is something to slide into.
   *  At rest it draws the window alone, which is exactly the DOM it drew
   *  before the payload was padded. */
  let panActive = $state(false);
  let panLull: ReturnType<typeof setTimeout> | undefined;
  let snapRaf = 0;
  /** A column's width, measured once when a gesture begins. Measuring on
   *  every wheel event forced a layout per event on top of the one the
   *  move itself cost — the lag (2026-09-03). */
  let panColWidth = 0;
  /** The last wheel events, for the speed at lift. */
  let panSamples: PanSample[] = [];
  /** Finger travel to column travel. One to one made a week the whole
   *  width of a touchpad, since a wheel's pixels are the finger's; at two,
   *  a week is a comfortable swipe and a day still lands where you meant.
   *  Tuned by feel on the omarchy box, 2026-09-03. */
  const PAN_GAIN = 2;
  /** A wheel this long unheard is the fingers lifting: WebKit reports no
   *  gesture phases on a DOM wheel event, so the pause is the only signal.
   *  Momentum on macOS keeps the events coming and so keeps the track
   *  moving, which is what a scroll view would do. */
  const PAN_LULL_MS = 120;
  const PAN_SNAP_MS = 180;

  /** Any column's width — they are all one width, and the track is sized
   *  from the visible count, so this is the pane's width over `visible`. */
  function colWidth(): number {
    const col = bodyEl?.querySelector('.col');
    return col ? col.getBoundingClientRect().width : 0;
  }

  /** The fingers lifted. A flick keeps going with momentum first — the
   *  speed at lift decaying away, whole days handed up as their columns
   *  pass — and then, or at once for a stop, the track settles on the
   *  nearest whole column (one more day past half) and eases the
   *  remainder out. */
  function settlePan() {
    const travel = flingTravel(velocityOf(panSamples));
    panSamples = [];
    if (travel === 0) {
      snapToColumn();
      return;
    }
    const from = panDays;
    const started = performance.now();
    let handed = 0;
    const glide = (t: number) => {
      const target = from + travel * flingProgress(t - started);
      // Commit the whole columns the glide has passed, the way the wheel
      // does, so the window walks through the padding under the motion.
      const { shift, rest } = panCommit(target - handed);
      if (shift !== 0) {
        handed -= shift;
        onpan?.(shift);
      }
      panDays = rest;
      if (t - started < FLING_TAU_MS * 4) {
        snapRaf = requestAnimationFrame(glide);
      } else {
        snapToColumn();
      }
    };
    snapRaf = requestAnimationFrame(glide);
  }

  function snapToColumn() {
    const { shift, from } = snapPlan(panDays);
    if (shift !== 0) onpan?.(shift);
    panDays = from;
    const started = performance.now();
    const step = (t: number) => {
      const k = Math.min(1, (t - started) / PAN_SNAP_MS);
      const eased = 1 - (1 - k) * (1 - k);
      panDays = from * (1 - eased);
      if (k < 1) {
        snapRaf = requestAnimationFrame(step);
      } else {
        panDays = 0;
        panActive = false;
      }
    };
    snapRaf = requestAnimationFrame(step);
  }
  let handledRevealNowRequest: number | null = null;
  const INITIAL_VIEWPORT_FRACTION = 1 / 3;
  const NOW_VIEWPORT_FRACTION = 0.45;

  $effect(() => {
    if (!bodyEl || visibleWeek.days.length === 0) return;
    const el = bodyEl;
    const now = Date.now();
    // Among the days on screen: today in the padding is not a reason to
    // open next week at the current hour.
    const today = visibleWeek.days.find((d) => now >= d.start_ms && now < d.end_ms);
    if (handledRevealNowRequest === null) handledRevealNowRequest = revealNowRequest;
    const revealRequested = revealNowRequest !== handledRevealNowRequest;

    // When Today also navigated from another period, its request reaches this
    // component before the new payload can. Leave it pending until the payload
    // containing now arrives; handling it against the old week would scroll an
    // unrelated 08:00 into view and consume the user's request.
    if (revealRequested && !today) return;
    if (!revealRequested && hasScrolled) return;

    const fullFrac = today
      ? (now - today.start_ms) / (today.end_ms - today.start_ms)
      : hourFrac(gutterDay, 8);
    const range = crop(today ?? gutterDay);
    const frac = (fullFrac - range.start) / range.span;
    hasScrolled = true;
    if (revealRequested) handledRevealNowRequest = revealNowRequest;
    // After layout: scrollHeight is meaningless until the columns have height.
    requestAnimationFrame(() => {
      const viewportFraction = revealRequested
        ? NOW_VIEWPORT_FRACTION
        : INITIAL_VIEWPORT_FRACTION;
      el.scrollTop = Math.max(
        0,
        frac * el.scrollHeight - el.clientHeight * viewportFraction,
      );
    });
  });

  // The open popover. `selectedId`/`selectedStartMs` name the *UiEvent block
  // that was clicked* — every expanded occurrence of a recurring master
  // shares that master's store row id, so only the block's own `start_ms`
  // says which occurrence was actually clicked, and `respondToEvent` needs
  // exactly that value (see eventdetail.ts).
  //
  // A pair of primitives, not the `UiEvent` object itself: reassigning an
  // object into a `$state` variable proxies it, and a later `===`/`!==`
  // against the original (unproxied, or differently-proxied) reference can
  // then read as unequal even for "the same" event — Svelte's own
  // `state_proxy_equality_mismatch` warning exists exactly for this
  // mistake. `id` + `start_ms` are plain numbers; equality between two
  // reads of a number never depends on which proxy either passed through.
  let selectedId = $state<number | null>(null);
  let selectedStartMs = $state<number | null>(null);
  // Carried for the same reason `selectedStartMs` is, one step further: the
  // event form needs the clicked occurrence's whole span, and deriving its end
  // from the master's duration is wrong for any occurrence whose own length
  // crosses a daylight-saving transition the master's does not. Deliberately
  // *not* part of `isSelected` — `id` + `start_ms` already name an occurrence
  // uniquely, and a third term could only ever make two reads of the same
  // block disagree.
  let selectedEndMs = $state<number | null>(null);
  let anchor = $state<Rect | null>(null);
  let detail = $state<EventDetail | null>(null);
  let selectedEvent = $state<UiEvent | null>(null);

  function isSelected(event: UiEvent): boolean {
    return selectedId === event.id && selectedStartMs === event.start_ms;
  }

  // Optimistic RSVP overrides, keyed by "id:startMs" so one occurrence's
  // answer never bleeds onto another sharing the same recurring master's
  // row id. Reassigned wholesale on every change (`handleResponded`, the
  // eviction effect below) — same reasoning as `CalendarPopover`'s own
  // `busy` Set: `$state` does not make a plain `Map`'s own mutations
  // reactive, only the variable binding does.
  //
  // Deliberately *not* mutating `week.days[...].events[...].response`
  // directly: `week` is this component's own prop, not something it was
  // ever given via `$state()` here, and whether mutating a nested field of
  // it is even observable depends on machinery this component does not
  // control (whether the caller's own `week` happens to be a deep-reactive
  // `$state` proxy). An override this component declares and owns with its
  // own `$state` is guaranteed reactive regardless of what `week` is.
  //
  // `baseline` is the payload's *own* response at the moment the override
  // was recorded — not just the overridden value. Without it, an override
  // would win over every future payload for the rest of the session:
  // decline locally, then accept from another device, and the next sync's
  // payload would arrive saying "accepted" while the grid kept showing
  // "declined" until the app relaunched. Comparing the *current* payload
  // against `baseline` (see the eviction effect below) is what lets a
  // fresher sync win once it actually disagrees with what this override
  // was recorded against — the in-place mutation this replaced self-healed
  // on the next payload for free; an owned override has to do it on purpose.
  type Override = { response: UiEvent['response']; baseline: UiEvent['response'] };
  let responseOverrides = $state<Map<string, Override>>(new Map());

  function overrideKey(id: number, startMs: number): string {
    return `${id}:${startMs}`;
  }

  /** The payload's own (un-overridden) response for one occurrence, or
   *  `undefined` if `week` no longer carries it at all (the week navigated
   *  away, or the occurrence fell out of the window). */
  function payloadResponse(id: number, startMs: number): UiEvent['response'] | undefined {
    for (const d of week.days) {
      const found = d.events.find((e) => e.id === id && e.start_ms === startMs);
      if (found) return found.response;
    }
    return undefined;
  }

  // Evicts any override whose recorded `baseline` no longer matches what
  // `week` itself says — a fresher payload landed and disagrees, so it
  // wins. Runs whenever `week` changes (that's what `payloadResponse`
  // reads); reassigning `responseOverrides` here also re-triggers this
  // effect, but only entries actually evicted are ever removed, so the
  // second pass finds nothing left to do and settles immediately.
  $effect(() => {
    let next: Map<string, Override> | null = null;
    for (const [key, ov] of responseOverrides) {
      const [idStr, startMsStr] = key.split(':');
      const current = payloadResponse(Number(idStr), Number(startMsStr));
      if (current !== undefined && current !== ov.baseline) {
        (next ??= new Map(responseOverrides)).delete(key);
      }
    }
    if (next) responseOverrides = next;
  });

  // What actually renders: `week.days`, but with any occurrence that still
  // has a live (not yet evicted) override showing its overridden `response`
  // instead of the payload's own.
  //
  // All-day events are outside this, and do not need to be in it. They live
  // in `week.all_day_events`, never in a day column, and an `AllDayBand`
  // chip renders no RSVP state at all — so there is nothing on a chip for an
  // override to restyle. `payloadResponse` walks only `week.days` and
  // therefore returns `undefined` for one, which makes `handleResponded`
  // record nothing for a chip; the answer still reaches Google either way.
  // Give chips a response style and both this and `payloadResponse` have to
  // grow an all-day arm together.
  const effectiveDays = $derived(
    week.days.map((d) => ({
      ...d,
      events: d.events.map((e) => {
        const override = responseOverrides.get(overrideKey(e.id, e.start_ms));
        return override ? { ...e, response: override.response } : e;
      }),
    })),
  );

  /** What the track draws: the window at rest, every day while sliding —
   *  and the offset that keeps the window's first day at the track's left
   *  edge either way. `renderVis` is the columns of padding to the left of
   *  the window, zero at rest because none are drawn. */
  const renderedDays = $derived(panActive ? effectiveDays : effectiveDays.slice(visStart, visStart + visible));
  const renderVis = $derived(panActive ? visStart : 0);
  /** The band's rows while sliding: packed for the window the gesture began
   *  on and held for its whole length — re-packing per day crossed would
   *  reshuffle rows under the swipe — and only re-packed when the payload
   *  itself is replaced. At rest, the window's own rows (`sliceWeek`). */
  let panLanes = $state<Lane[]>([]);
  /** And its "+N more", frozen with the rows for the same reason: a count
   *  that changed per day crossed would take the row it sits on with it,
   *  which is a height change under the gesture. */
  let panHidden = $state<number[]>([]);
  $effect.pre(() => {
    const active = panActive;
    const lanes = week.all_day;
    const rows = bandRows;
    if (!active) return;
    const packed = packBandLanes(
      lanes, untrack(() => visStart), untrack(() => visible), true, rows,
    );
    panLanes = packed.lanes;
    panHidden = packed.hidden;
  });
  const renderedLanes = $derived(panActive ? panLanes : visibleWeek.all_day);
  const renderedHidden = $derived(panActive ? panHidden : visibleWeek.overflow);

  /**
   * An in-flight drag, or `null`.
   *
   * **Task 3 writes nothing.** This moves a block and puts it back; the write,
   * the notify dialog and the recurring-scope question are Task 4's. Keeping
   * the gesture on its own first is what lets it be got right while being
   * wrong is free.
   *
   * `offsetPct` is what the block is rendered with, and is a percentage of the
   * column so it lands wherever the column happens to be sized — the same unit
   * the block's own `top` is in.
   */
  type Drag = {
    /** The occurrence being dragged, kept whole so the drop can hand it up
     *  without the grid having to find it again in a `week` that may have been
     *  replaced by a background reload while the pointer was down. */
    event: UiEvent;
    id: number;
    startMs: number;
    originX: number;
    originY: number;
    colHeight: number;
    colWidth: number;
    dayMs: number;
    origin: { startMs: number; endMs: number };
    /** Which end was grabbed, or `null` for the body of the block — decided
     *  once, at the press, by `edgeAt`. Deciding it again on each move would
     *  let a gesture change from a resize into a move halfway through, because
     *  the pointer leaves the band it started in almost immediately. */
    edge: 'start' | 'end' | null;
    /** Past the threshold. Below it this is still a click. */
    moving: boolean;
    /** The span this drag would write. `null` until it has moved at all. */
    landed: { startMs: number; endMs: number } | null;
    /** Where the block is drawn while dragging, relative to where `Placed`
     *  put it. Presentational only — the write reads `landed`, so the two can
     *  never disagree by being derived from one another. */
    preview: { topDeltaPct: number; heightDeltaPct: number; dx: number } | null;
  };
  let drag = $state<Drag | null>(null);

  /**
   * Whether the press that just ended was a *drag*, so the `click` the browser
   * dispatches after `pointerup` does not also open the popover.
   *
   * **Assigned on every release, never merely set**, which is what stops it
   * outliving the gesture it describes. A drag cancelled by Escape leaves it
   * true with no click to consume it; the next press's own `pointerup` then
   * assigns it `false` before that press's `click` is dispatched, so a stale
   * `true` can never reach a click that deserved to open something.
   *
   * An earlier version also cleared it on `pointerdown`. That read as prudent
   * and was dead: the release above already assigns it on every gesture, and a
   * mutation deleting the clear reddened nothing at all.
   */
  let draggedNotClicked = false;

  /** The preview for `event`, or `null` when it is not the one being dragged. */
  const previewFor = (event: UiEvent) =>
    drag && drag.moving && drag.id === event.id && drag.startMs === event.start_ms
      ? drag.preview
      : null;

  /** The span the drag would write, for the dragged block's own card to say
   *  — `landed` is the drop's value, so the clock on the card and the write
   *  cannot disagree. Same identity test as `previewFor`. */
  const liveSpanFor = (event: UiEvent) =>
    drag && drag.moving && drag.id === event.id && drag.startMs === event.start_ms
      ? drag.landed
      : null;

  /** Whether any visible day has a task due. The row exists only then. */
  const anyTasks = $derived(
    renderedDays.some((d) => (tasks?.get(dateKey(d.start_ms))?.length ?? 0) > 0),
  );

  /** A task chip being dragged across the row: which one, from where, and
   *  how many columns the pointer has travelled. */
  let taskDrag = $state<
    { id: number; fromMs: number; originX: number; colWidth: number; cols: number; moving: boolean } | null
  >(null);

  /** The day a drag currently points at, or null when nothing is moving.
   *  One derived answer rather than a function called per cell, so the
   *  landing chip and the lit column cannot disagree about where it is. */
  const taskDropMs = $derived.by(() => {
    if (!taskDrag || !taskDrag.moving) return null;
    const days = renderedDays.map((d) => d.start_ms);
    const i = days.indexOf(taskDrag.fromMs);
    if (i < 0) return null;
    return days[Math.min(days.length - 1, Math.max(0, i + taskDrag.cols))];
  });
  /** Whether a chip is the one in flight, and so drawn under the pointer
   *  rather than in the column it came from. */
  const inFlight = (chip: TaskChip) => taskDrag?.moving === true && taskDrag.id === chip.id;

  function startTaskDrag(chip: TaskChip, dayMs: number, e: PointerEvent) {
    if (e.button !== 0 || !chip.canWrite || !ontaskmove) return;
    const col = (e.currentTarget as HTMLElement).closest('.tcell');
    if (!col) return;
    taskDrag = {
      id: chip.id,
      fromMs: dayMs,
      originX: e.clientX,
      colWidth: col.getBoundingClientRect().width,
      cols: 0,
      moving: false,
    };
    window.addEventListener('pointermove', onTaskDragMove);
    window.addEventListener('pointerup', onTaskDragEnd);
    window.addEventListener('keydown', onTaskDragKey);
  }

  function onTaskDragMove(e: PointerEvent) {
    if (!taskDrag) return;
    const dx = e.clientX - taskDrag.originX;
    if (!taskDrag.moving && !beganDrag(dx, 0)) return;
    taskDrag.moving = true;
    taskDrag.cols = colsMoved(dx, taskDrag.colWidth);
  }

  function endTaskDrag(commit: boolean) {
    const d = taskDrag;
    taskDrag = null;
    window.removeEventListener('pointermove', onTaskDragMove);
    window.removeEventListener('pointerup', onTaskDragEnd);
    window.removeEventListener('keydown', onTaskDragKey);
    if (!d || !d.moving || !commit || d.cols === 0) return;
    const days = renderedDays.map((x) => x.start_ms);
    const i = days.indexOf(d.fromMs);
    if (i < 0) return;
    const j = Math.min(days.length - 1, Math.max(0, i + d.cols));
    if (days[j] !== d.fromMs) ontaskmove?.(d.id, days[j]);
  }

  const onTaskDragEnd = () => endTaskDrag(true);
  /** Escape puts it back, the same escape the event drag honours. */
  const onTaskDragKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') endTaskDrag(false);
  };

  function startDrag(event: UiEvent, day: { start_ms: number; end_ms: number }, e: PointerEvent) {
    // Primary button only: a right-click opens a context menu and must not
    // leave a half-armed drag behind it.
    if (e.button !== 0) return;
    const target = e.currentTarget as HTMLElement;
    const col = target.closest('.col');
    if (!col) return;
    const colBox = col.getBoundingClientRect();
    // Decide once at pointer-down. Releasing Alt during the sweep must not
    // turn a new event into a move/resize of the event underneath it.
    if (e.altKey) {
      e.preventDefault();
      startSweep(day, renderedDays.findIndex((d) => d.start_ms === day.start_ms), e, colBox);
      return;
    }
    if (event.copies && event.copies.length > 1) {
      // A combined block has no single event to move. Choose a copy in its
      // details first.
      draggedNotClicked = false;
      return;
    }
    const box = target.getBoundingClientRect();

    drag = {
      event,
      id: event.id,
      startMs: event.start_ms,
      originX: e.clientX,
      originY: e.clientY,
      colHeight: colBox.height,
      colWidth: colBox.width,
      // Decided from where the press landed *within the block*, which is what
      // `edgeAt` answers and what a band drawn as an element could disagree
      // with.
      edge: edgeAt(e.clientY - box.top, box.height),
      dayMs: day.end_ms - day.start_ms,
      origin: { startMs: event.start_ms, endMs: event.end_ms },
      moving: false,
      landed: null,
      preview: null,
    };

    window.addEventListener('pointermove', onDragMove);
    window.addEventListener('pointerup', onDragEnd);
    window.addEventListener('keydown', onDragKey);
  }

  function onDragMove(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.originX;
    const dy = e.clientY - drag.originY;

    // Below the threshold this is still a click, and nothing has moved yet.
    if (!drag.moving && !beganDrag(dx, dy)) return;
    drag.moving = true;

    // The geometry is `drag.ts`'s, never recomputed here — the snap, the
    // duration rule, the civil day, the inversion clamp and how many columns
    // a sideways travel crosses all live there with a table each.
    const dyFrac = drag.colHeight === 0 ? 0 : dy / drag.colHeight;
    // A resize is one edge of one block and never crosses a day.
    const cols = drag.edge ? 0 : colsMoved(dx, drag.colWidth);

    // What would be written. Stored rather than reconstructed on drop, so the
    // instants that go to Google are the ones the geometry produced and not a
    // second derivation of them.
    drag.landed = drag.edge
      ? spanForResize(drag.origin, drag.edge, dyFrac, drag.dayMs, SNAP_MS)
      : spanForMove(drag.origin, dyFrac, drag.dayMs, renderedDays.length, cols, SNAP_MS);

    // What is drawn, and **the two axes are drawn separately**: a day is a
    // sideways translation, not a hundred percent of a column's height. Asking
    // the same span for both puts a block dragged one column right a whole
    // column *down* as well — `top: calc(105%)`, off the bottom of the grid,
    // which is exactly what it did.
    const vertical = drag.edge
      ? drag.landed
      : spanForMove(drag.origin, dyFrac, drag.dayMs, renderedDays.length, 0, SNAP_MS);

    // Deltas on `Placed`, not absolutes: a block's position comes from the
    // backend's own layout and is not recoverable from its instants.
    //
    // `d` rather than `drag`: the closure below outlives the null check above
    // as far as the compiler is concerned, and narrowing a module-level `let`
    // is not something it will carry into one.
    const d = drag;
    const pct = (ms: number) => (ms / d.dayMs) * 100;
    const wasMs = d.origin.endMs - d.origin.startMs;
    drag.preview = {
      topDeltaPct: pct(vertical.startMs - d.origin.startMs),
      heightDeltaPct: pct(vertical.endMs - vertical.startMs - wasMs),
      // Whole columns in pixels: a block is a fraction of a column's width, so
      // translating it by its own width would land it somewhere arbitrary.
      dx: cols * d.colWidth,
    };
  }

  /**
   * Dragging the form's own draft, which is the same gesture as dragging a
   * saved event and deliberately shares its geometry: `edgeAt` decides move
   * from resize, `colsMoved` counts days crossed, and `spanForMove` /
   * `spanForResize` do the snapping, the duration rule and the inversion
   * clamps. A second implementation of any of that would be a second set of
   * answers to questions `drag.ts` has already answered once, with a table
   * each.
   *
   * What differs is where the result goes. A saved event's drop calls
   * `onmove` once, at the end, because the write is a request. A draft has
   * nothing to write: it reports **every** move to the form, which owns the
   * times, and the ghost redraws because `formPreview` comes back changed.
   * The gesture is therefore its own preview, and this function keeps no
   * preview state at all.
   */
  let draftDrag: {
    originX: number; originY: number; colHeight: number; colWidth: number;
    dayMs: number; edge: ReturnType<typeof edgeAt>;
    origin: { startMs: number; endMs: number }; moving: boolean;
  } | null = null;

  function startDraftDrag(e: PointerEvent, day: { start_ms: number; end_ms: number }) {
    // Primary button only, and only a timed draft: an all-day ghost is a
    // ribbon over whole days, placed by the sideways sweep, and `applySpan`
    // on the form refuses it for the same reason.
    if (e.button !== 0 || !ondraftmove || formPreview?.kind !== 'timed') return;
    const target = e.currentTarget as HTMLElement;
    const col = target.closest('.col');
    if (!col) return;
    const colBox = col.getBoundingClientRect();
    const box = target.getBoundingClientRect();

    // The whole draft, not the slice this column draws: a span crossing
    // midnight is one event in two columns, and moving it by the piece under
    // the pointer would stretch it instead.
    draftDrag = {
      originX: e.clientX, originY: e.clientY,
      colHeight: colBox.height, colWidth: colBox.width,
      dayMs: day.end_ms - day.start_ms,
      edge: edgeAt(e.clientY - box.top, box.height),
      origin: { startMs: formPreview.startMs, endMs: formPreview.endMs },
      moving: false,
    };
    e.stopPropagation(); // never also start a sweep underneath
    window.addEventListener('pointermove', onDraftMove);
    window.addEventListener('pointerup', endDraftDrag);
    window.addEventListener('keydown', onDraftKey);
  }

  function onDraftMove(e: PointerEvent) {
    if (!draftDrag) return;
    const dx = e.clientX - draftDrag.originX;
    const dy = e.clientY - draftDrag.originY;
    if (!draftDrag.moving && !beganDrag(dx, dy)) return;
    draftDrag.moving = true;

    const dyFrac = draftDrag.colHeight === 0 ? 0 : dy / draftDrag.colHeight;
    const cols = draftDrag.edge ? 0 : colsMoved(dx, draftDrag.colWidth);
    const landed = draftDrag.edge
      ? spanForResize(draftDrag.origin, draftDrag.edge, dyFrac, draftDrag.dayMs, SNAP_MS)
      : spanForMove(
          draftDrag.origin, dyFrac, draftDrag.dayMs, renderedDays.length, cols, SNAP_MS,
        );
    ondraftmove?.(landed);
  }

  /** Escape puts the draft back where the gesture found it, matching what
   *  Escape does to a real drag — and leaving the form open, because the
   *  draft is what the form is about. */
  function onDraftKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !draftDrag) return;
    e.stopPropagation();
    e.preventDefault();
    const origin = draftDrag.origin;
    endDraftDrag();
    ondraftmove?.(origin);
  }

  function endDraftDrag() {
    draftDrag = null;
    window.removeEventListener('pointermove', onDraftMove);
    window.removeEventListener('pointerup', endDraftDrag);
    window.removeEventListener('keydown', onDraftKey);
  }

  function onDragEnd() {
    if (!drag) return;
    draggedNotClicked = drag.moving;

    // **§4: a drop that lands where it started takes no action at all.** Not
    // "writes the same values" — no request, no dialog, nothing. Grabbing an
    // event and putting it back must be free, and the only way to guarantee
    // that is to decide it here, before anything downstream can be asked to
    // notice that a write would be a no-op.
    //
    // Compared as instants rather than as pixels: two pointer positions inside
    // one 15-minute slot are the same drop, and the geometry has already said
    // so by returning the span it did.
    // One question, asked of the span rather than of the gesture: a press that
    // never passed the threshold has no preview and so lands on its own
    // origin, which is the same answer for the same reason.
    //
    // **Both ends**, now that a resize is possible: a resize leaves the start
    // exactly where it was and moves only the end, so a comparison of starts
    // alone would call every resize a no-op and write nothing.
    const landed = drag.landed ?? drag.origin;
    const changed =
      landed.startMs !== drag.origin.startMs || landed.endMs !== drag.origin.endMs;
    const event = drag.event;

    endDrag();
    if (changed) onmove(event, landed);
  }

  function onDragKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !drag) return;
    // Cancelled: the block returns to its origin and the pointer release that
    // follows must not be read as a drop.
    e.stopPropagation();
    draggedNotClicked = drag.moving;
    endDrag();
  }

  function endDrag() {
    drag = null;
    window.removeEventListener('pointermove', onDragMove);
    window.removeEventListener('pointerup', onDragEnd);
    window.removeEventListener('keydown', onDragKey);
  }

  /**
   * Opens the details card on one occurrence — or, with `thenEdit`, goes
   * straight past it into the editor (#109).
   *
   * **One detail path, not two.** A right-click could have fetched the event
   * itself and skipped this function entirely, and that is exactly the second
   * set of guards this comment exists to refuse: `draggedNotClicked`, the
   * superseded-while-loading check and the "say so on failure" rule are all
   * here, and a shortcut around them would have to keep its own copy in step.
   * The editor is reached at the end, after those have had their say.
   */
  async function openPopover(event: UiEvent, rect: Rect, thenEdit = false) {
    // The `click` that follows a drag's `pointerup` is not a click on this
    // block; swallow it. See `draggedNotClicked` for why the flag is cleared
    // on press rather than here.
    if (draggedNotClicked) return;

    hoveredOccurrence = null;
    selectedEvent = event;
    selectedId = event.id;
    selectedStartMs = event.start_ms;
    selectedEndMs = event.end_ms;
    anchor = rect;
    detail = null;

    let d: EventDetail;
    try {
      d = await getEventDetail(event.id);
    } catch (e) {
      // Nothing to show. Close rather than leave an empty shell open, but
      // only if the user hasn't already clicked something else while this
      // was in flight — **and say so**. Closing silently made a failed
      // fetch indistinguishable from a dead control: reported 2026-09-04
      // as "I press an all-day event and nothing happens", which is
      // exactly what a click looks like when the detail behind it cannot
      // be read and the app keeps that to itself.
      if (isSelected(event)) {
        closePopover();
        onerror?.(`Could not open "${event.title}" · ${String(e)}`);
      }
      return;
    }
    if (!isSelected(event)) return; // superseded while loading

    // Right-click's whole point: the card never appears. Taken before
    // `detail` is assigned so it cannot paint for a frame on the way past.
    //
    // **`can_edit` gates this exactly as it gates the card's own Edit
    // button**, and for the reason written there: an editor opened on a
    // subscribed holiday calendar can only produce a Save the server
    // refuses, after the user has decided to go through with it. A
    // right-click that cannot edit falls through to the card rather than
    // doing nothing — a control that answers nothing reads as broken.
    if (thenEdit && d.can_edit) {
      const occurrence = { detail: d, startMs: event.start_ms, endMs: event.end_ms };
      closePopover();
      onedit(occurrence, rect);
      return;
    }

    detail = d;

    // Fires only once the popover has painted the local detail — a
    // freshness optimisation, not a load, so a rejection (offline, a
    // revoked token) is silently ignored and the last-synced detail already
    // on screen stands unchanged.
    await tick();
    if (!isSelected(event)) return;
    refreshEvent(event.id)
      .then((fresh) => {
        if (isSelected(event) && JSON.stringify(fresh) !== JSON.stringify(detail)) {
          detail = fresh;
        }
      })
      .catch(() => {});
  }

  function closePopover() {
    selectedEvent = null;
    selectedId = null;
    selectedStartMs = null;
    selectedEndMs = null;
    anchor = null;
    detail = null;
  }

  /**
   * The half hour a click landed in, in the day it landed on.
   *
   * Read as a fraction of the column's own height and applied to that column's
   * own span, both halves for the reason `hourFrac` above documents: a DST day
   * is 23 or 25 hours long, and dividing by a fixed 24 puts every click after
   * the transition an hour out.
   *
   * Snapped in *local* wall-clock minutes rather than by flooring the instant
   * to a multiple of thirty minutes: a zone offset at :45 (Kathmandu, Chatham)
   * has no half hour on a whole-half-hour UTC boundary at all, and the arrival
   * of one would be a form offering 09:15 for a click on the 09:30 line.
   */
  function slotAt(day: { start_ms: number; end_ms: number }, e: MouseEvent): number {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const frac = Math.min(Math.max((e.clientY - r.top) / r.height, 0), 1);
    const at = new Date(day.start_ms + frac * (day.end_ms - day.start_ms));
    at.setMinutes(at.getMinutes() < 30 ? 0 : 30, 0, 0);
    return at.getTime();
  }

  /** Where a right button went down, or `null`. The release decides whether
   *  it was a click (create here) or a drag (nothing) — see the `.newhere`
   *  handlers. */
  let rightPress: { x: number; y: number } | null = null;

  function startCreate(day: { start_ms: number; end_ms: number }, e: MouseEvent) {
    // The `click` the browser dispatches after a sweep's `pointerup` is not a
    // click on empty grid — it is the tail of a gesture that has already asked
    // for a form. Without this the user gets two: one on the span they swept,
    // and then one on the half hour the release happened to land in, which is
    // the one that wins. Same flag, and the same reasoning, as `openPopover`.
    if (draggedNotClicked) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Anchored at the click's own height rather than the column's top: the
    // column is 1680px tall inside a scrolling body, and a form placed against
    // its top edge would open somewhere off-screen above the click.
    oncreate(slotAt(day, e), { top: e.clientY, left: r.left, width: r.width, height: 0 });
  }

  /**
   * A sweep across empty grid, or `null`.
   *
   * **This creates nothing.** It ends by handing a span up through `oncreate`,
   * the same callback a click already uses, and the form does the creating
   * through the path it has always used — a new event needs a title, and the
   * form is where that lives. A create command reached from here would be a
   * second way to make an event, and the less-used one rots.
   *
   * Separate state from `drag` rather than a fourth `edge` value on it: the two
   * gestures start on different elements, one has an occurrence and the other
   * has a column, and only one of them can be in flight at a time anyway. A
   * union would have made every field of both optional.
   *
   * **Sideways is a different event.** Stay in one column and this is a timed
   * event, exactly as it always was. Cross a boundary and it becomes an
   * all-day span over the days crossed (requested 2026-08-31). The rule this
   * replaces refused sideways travel precisely to avoid a sweep "silently"
   * producing a three-day event — so the ghost below stops it being silent:
   * every covered column is lit for its whole height while the button is
   * down, and the form then opens with All day already ticked.
   */
  type Sweep = {
    /** The day swept in — both the arithmetic and which column draws it. */
    dayStartMs: number;
    dayMs: number;
    /** Its index in `effectiveDays`, so sideways travel can name the days it
     *  crosses without re-measuring anything. */
    dayIndex: number;
    /** The column's box at the press. `colHeight` is what the pointer's travel
     *  is divided by; the other two only place the form. */
    colHeight: number;
    colLeft: number;
    colWidth: number;
    originX: number;
    originY: number;
    /** Where the press landed, as a fraction of the column's height. The one
     *  absolute reading this gesture takes — it names a *time*, once. */
    fromFrac: number;
    /** The last pointer position, for the rect the form opens beside — the
     *  same "next to where the user pointed" rule `startCreate` follows. */
    clientX: number;
    clientY: number;
    /** Past the threshold. Below it this is still a click. */
    sweeping: boolean;
    /** What the form would open on. `null` until the hand has moved at all. */
    ask: import('./drag').SweepAsk | null;
  };
  let sweep = $state<Sweep | null>(null);
  let altHeld = $state(false);
  let hoveredOccurrence = $state<{ day: number; id: number; startMs: number } | null>(null);
  // Indices belong to one payload. A drag or sync can change them without a
  // pointer move, so resolve the hovered occurrence against the current data.
  const hoveredPlacement = $derived.by(() => {
    const hover = hoveredOccurrence;
    if (!hover) return null;
    const day = renderedDays.find(d => d.start_ms === hover.day);
    const idx = day?.events.findIndex(e => e.id === hover.id && e.start_ms === hover.startMs) ?? -1;
    return idx < 0 ? null : { day: hover.day, idx };
  });
  function trackEventHover(day: DayColumn, e: PointerEvent) {
    if (e.buttons) return; // A move/resize keeps the event picked up at the press.
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width;
    const y = (e.clientY - r.top) / r.height;
    const underTime = day.placed.filter(p => y >= p.top + 1 / r.height && y < p.top + p.height - 1 / r.height);
    // Use the packed columns, not the expanded DOM boxes: otherwise the first
    // event to expand owns every subsequent hover over its neighbours.
    const current = underTime.find(p => hoveredPlacement?.day === day.start_ms && p.idx === hoveredPlacement.idx);
    const candidate = underTime.find(p => x >= p.column / p.columns && x < (p.column + 1) / p.columns);
    // Shared copy targets may fill lanes belonging to staggered meetings.
    // Keep those targets until the pointer leaves this event's time span;
    // only completely covered meetings need an internal handoff.
    const keepShared = current && (day.events[current.idx].copies?.length ?? 0) > 1
      && candidate && !containsPlacement(current, candidate);
    const p = keepShared ? current : candidate ?? current;
    const event = p && day.events[p.idx];
    hoveredOccurrence = event ? { day: day.start_ms, id: event.id, startMs: event.start_ms } : null;
  }
  // The gesture is chosen at pointer-down; pressing Alt during a move must
  // not advertise creation, and releasing it during a sweep must not end it.
  const createMode = $derived(sweep !== null || (altHeld && drag === null));
  const hoverContext = $derived.by(() => {
    const hover = hoveredPlacement;
    if (!hover || createMode || drag) return null;
    const day = renderedDays.find(d => d.start_ms === hover.day);
    const active = day?.placed.find(p => p.idx === hover.idx);
    if (!day || !active) return null;
    // Only roll up cards that this expansion completely covers. Staggered
    // overlaps (and a longer enclosing event) remain reachable above/below
    // the active card, so leave their labels visible and omit their markers.
    // Fractions such as 10/24 + 1/24 can slightly exceed 11/24; tolerate that
    // rounding when two cards share their start or end boundary.
    const peers = day.placed.filter(p => containsPlacement(active, p));
    const segments = peers.flatMap(p => {
      const ev = day.events[p.idx];
      const colors = ev.copies?.map(copy => copy.color) ?? [ev.color];
      return colors.map((color, i) => ({ color,
        left: (p.column + i / colors.length) / p.columns,
        width: 1 / (p.columns * colors.length) }));
    });
    const copyHoverLane = peers.length > 1
      ? { left: active.column / active.columns, width: 1 / active.columns } : undefined;
    return { day: day.start_ms, idx: active.idx, peers, segments, copyHoverLane };
  });
  const trackAlt = (e: KeyboardEvent | PointerEvent) => { altHeld = e.altKey; };
  function trackPointer(e: PointerEvent) {
    trackAlt(e);
    // Swapping the pointer-transparent label for copy panels under a stationary
    // cursor can skip Chromium's column leave event. The next move outside the
    // columns must still return that event to its resting appearance.
    // A handoff may remove the old copy target before this window handler
    // runs. The dispatch path still names its column; the detached target's
    // current ancestors do not, and would immediately cancel the new hover.
    const path = e.composedPath();
    if (!bodyEl || !path.includes(bodyEl) || !path.some(node => node instanceof Element && node.matches('.col'))) {
      hoveredOccurrence = null;
    }
  }

  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let feedbackWidth = $state(0);
  let feedbackHeight = $state(0);
  const sweepFeedback = $derived.by(() => {
    const ask = sweep?.ask;
    if (!ask) return null;
    if (ask.kind === 'allDay') {
      const days = renderedDays.filter((d) => d.start_ms >= ask.firstDayMs && d.start_ms <= ask.lastDayMs).length;
      return { duration: `${days} ${days === 1 ? 'day' : 'days'}`, range: 'All day' };
    }
    // Read the snapped span that will open in the form, including reverse
    // drags and boundary clamping, rather than measuring the pointer again.
    const minutes = Math.round((ask.endMs - ask.startMs) / 60_000);
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    const duration = hours ? `${hours}h${rest ? ` ${rest}m` : ''}` : `${minutes} min`;
    const range = `${formatClock(ask.startMs, clockFormat())} – ${formatClock(ask.endMs, clockFormat())}`;
    return { duration, range };
  });

  function startSweep(
    day: { start_ms: number; end_ms: number }, dayIndex: number, e: PointerEvent,
    box = (e.currentTarget as HTMLElement).getBoundingClientRect(),
  ) {
    // Primary button only, for the same reason `startDrag` says so.
    if (e.button !== 0) return;
    sweep = {
      dayStartMs: day.start_ms,
      dayMs: day.end_ms - day.start_ms,
      dayIndex,
      colHeight: box.height,
      colLeft: box.left,
      colWidth: box.width,
      originX: e.clientX,
      originY: e.clientY,
      fromFrac: box.height === 0 ? 0 : (e.clientY - box.top) / box.height,
      clientX: e.clientX,
      clientY: e.clientY,
      sweeping: false,
      ask: null,
    };

    window.addEventListener('pointermove', onSweepMove);
    window.addEventListener('pointerup', onSweepEnd);
    window.addEventListener('keydown', onSweepKey);
    window.addEventListener('pointercancel', cancelSweep);
    window.addEventListener('blur', cancelSweep);
  }

  function onSweepMove(e: PointerEvent) {
    if (!sweep) return;
    const dx = e.clientX - sweep.originX;
    const dy = e.clientY - sweep.originY;

    // Below the threshold this is still a click on empty grid, and a click
    // still creates at the half hour it landed in.
    if (!sweep.sweeping && !beganDrag(dx, dy)) return;
    sweep.sweeping = true;
    sweep.clientX = e.clientX;
    sweep.clientY = e.clientY;

    // The far end is the near end **plus how far the hand travelled**, never a
    // second absolute reading of where the pointer is. The column is 1200px
    // inside a pane that scrolls, so an absolute reading would need the box
    // re-measured on every move and would then make the span answer to the
    // *pane* as well as to the hand: flick the trackpad with the button held
    // and a sweep nobody moved would grow. A delta answers to the hand only,
    // which is what the move and the resize beside it already do.
    //
    // The geometry itself is `drag.ts`'s: the snap, the direction rule and the
    // minimum span all live there with a table each.
    const toFrac = sweep.colHeight === 0 ? 0 : sweep.fromFrac + dy / sweep.colHeight;
    // Sideways is read the same way a move-drag reads it, by whole columns,
    // so the two gestures agree about when a column has been crossed.
    sweep.ask = sweepAsk(
      renderedDays,
      sweep.dayIndex,
      colsMoved(dx, sweep.colWidth),
      sweep.fromFrac,
      toFrac,
      SNAP_MS,
    );
  }

  function onSweepEnd() {
    if (!sweep) return;
    const s = sweep;
    // Assigned on every release, never merely set — see `draggedNotClicked`.
    draggedNotClicked = s.sweeping;
    endSweep();
    // `span` is assigned only once the threshold has been passed, so this is
    // the whole question: a press that never became a sweep has none, and the
    // `click` behind this release is what creates. Asking `sweeping` as well
    // would read as prudent and be dead — the two are set on the same line.
    if (!s.ask) return;
    const rect = { top: s.clientY, left: s.colLeft, width: s.colWidth, height: 0 };
    if (s.ask.kind === 'allDay') {
      sweptAllDayFrac = s.fromFrac;
      oncreateallday(s.ask.firstDayMs, s.ask.lastDayMs, rect);
    } else {
      oncreate(s.ask.startMs, rect, s.ask.endMs);
    }
  }

  function onSweepKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !sweep) return;
    // Cancelled: no form is asked for, and the release that follows must not be
    // read as the end of a sweep.
    e.stopPropagation();
    cancelSweep();
  }

  function cancelSweep() {
    if (!sweep) return;
    draggedNotClicked = sweep.sweeping;
    endSweep();
  }

  /**
   * Ends a sweep and takes its handlers back off `window`.
   *
   * The three removals are **not** covered by a spec and cannot be: identical
   * `(handler, options)` pairs are deduplicated by `addEventListener`, so a
   * leak never doubles anything, and every handler above returns immediately
   * once `sweep` is null. They earn their place by cost rather than by
   * behaviour — a window-level `pointermove` that fires on every mouse move for
   * the life of the view. The case that *was* observable is the unmount below.
   */
  function endSweep() {
    sweep = null;
    window.removeEventListener('pointermove', onSweepMove);
    window.removeEventListener('pointerup', onSweepEnd);
    window.removeEventListener('keydown', onSweepKey);
    window.removeEventListener('pointercancel', cancelSweep);
    window.removeEventListener('blur', cancelSweep);
  }

  /**
   * **A gesture cannot outlive the grid it was made in.**
   *
   * Both gestures hang their handlers off `window` on purpose — a pointer that
   * leaves the column must still be followed — and both take them off again
   * when they end. Neither ends if the component goes away first: switching to
   * Month unmounts this grid with the button still down, and the release then
   * lands in a closure belonging to a grid nobody is looking at. A sweep asked
   * `App` for a form on a span from a week that had gone; a **drag wrote**,
   * which is the same shape and costs a request to Google.
   *
   * Nothing else here needed a teardown, which is why there was none: every
   * other listener in this file is `$effect`-owned and Svelte removes it. These
   * two are added from an event handler, so they are this component's to clean
   * up. The drag half of this has been true since Task 3; it is fixed here
   * rather than left because the sweep was about to be a second copy of it.
   */
  $effect(() => () => { endDrag(); endSweep(); });

  /**
   * The inline style for the ghost drawn over `day` while it is being swept,
   * or `null` when nothing is being swept there.
   *
   * §6 for the one gesture with no block to follow: without it the user drags
   * across nothing at all and a form appears afterwards carrying times they
   * never watched being chosen. Percentages of the column, like every other
   * position in this grid, so it lands wherever the column happens to be sized.
   */
  /** The form ghost's geometry in this day's column, clamped to it — a span
   *  that crosses midnight draws its slice in each column it touches, the
   *  same rule real events follow. */
  /** Where the all-day ribbon was drawn, kept past the release.
   *
   *  The draft has to stay *where the hand put it* — "the strip should not
   *  disappear but become an all-day strip which is not yet finalised"
   *  (2026-08-31). Anchoring the draft to the top of the column instead put
   *  it at midnight, which on a pane scrolled to the morning is off-screen:
   *  the ribbon still vanished, just for a different reason. Null when no
   *  sweep drew one, and then no grid ghost is drawn at all — an all-day
   *  form opened any other way has no position to claim. */
  let sweptAllDayFrac = $state<number | null>(null);

  /** How tall the all-day sweep's ribbon is drawn — half an hour of the
   *  column. Thin enough to read as a bar rather than a blackout, tall
   *  enough to see at a glance across seven columns. */
  const ALL_DAY_GHOST_MS = 30 * 60_000;

  /** Which sides of an all-day ribbon segment continue into the next column,
   *  so the run reads as one bar rather than a row of chips — the same
   *  `cl`/`cr` idiom `AllDayBand` uses for a span crossing a week edge. */
  function sweepEdges(day: { start_ms: number }): { cl: boolean; cr: boolean } {
    const ask = sweep?.ask;
    if (ask?.kind !== 'allDay') return { cl: false, cr: false };
    return { cl: day.start_ms > ask.firstDayMs, cr: day.start_ms < ask.lastDayMs };
  }

  /** `--ghost: <colour>;`, or nothing when there is no colour to declare and
   *  the stylesheet's `--accent` fallback should stand. A custom property
   *  rather than the declarations that read it, so the dashed border, the
   *  tint and the spine can never end up different colours. */
  const ghostInk = (color: string | null) => (color ? `--ghost:${color};` : '');

  function sweepStyle(day: { start_ms: number; end_ms: number }): string | null {
    const ask = sweep?.ask;
    if (!ask) return null;
    // The colour the form is about to open on. The gesture used to be
    // `--accent` on the grounds that no calendar had been chosen yet, but one
    // has: the create lands on the default calendar unless the form is told
    // otherwise, and drawing the sweep in anything else meant the tile changed
    // colour on release for no reason the user could see.
    const ink = ghostInk(createColor);
    // An all-day sweep draws a thin ribbon across the days it covers,
    // anchored at the height the press landed at. Not full columns: those
    // shouted, and half a screen of tint to say "three days" is more
    // emphasis than the fact deserves. The strip stays at the press height
    // rather than following the pointer, because for an all-day event the
    // vertical position means nothing and a ribbon that slid up and down
    // would keep implying it did.
    if (ask.kind === 'allDay') {
      if (day.start_ms < ask.firstDayMs || day.start_ms > ask.lastDayMs) return null;
      const span = day.end_ms - day.start_ms;
      const height = (ALL_DAY_GHOST_MS / span) * 100;
      const top = Math.min(Math.max(sweep!.fromFrac * 100, 0), 100 - height);
      return `${ink}top:${top}%;height:${height}%`;
    }
    if (sweep!.dayStartMs !== day.start_ms) return null;
    const span = day.end_ms - day.start_ms;
    const top = ((ask.startMs - day.start_ms) / span) * 100;
    const height = ((ask.endMs - ask.startMs) / span) * 100;
    return `${ink}top:${top}%;height:${height}%`;
  }

  // The remembered position belongs to one draft; when that draft is gone,
  // so is the claim on it.
  $effect(() => {
    if (!formPreview) sweptAllDayFrac = null;
  });

  function formPreviewStyle(day: { start_ms: number; end_ms: number }): string | null {
    if (!formPreview) return null;
    // The draft is drawn in the colour of the calendar it would be written to,
    // and follows the picker: switching calendar in the open form recolours
    // the tile, which is the whole point — "so it's visually more clear which
    // calendar I'm using to create a meeting" (2026-08-31).
    const ink = ghostInk(formPreview.color);
    // An all-day draft keeps the ribbon the sweep drew, moved to the top of
    // the column — where an all-day thing belongs, and a position that
    // claims no hour. Dates compared with dates: ISO strings sort
    // chronologically, and no missing midnight can shift the match.
    if (formPreview.kind === 'allDay') {
      if (sweptAllDayFrac === null) return null;
      const d = dateOf(day.start_ms);
      if (d < formPreview.firstDate || d > formPreview.lastDate) return null;
      const height = (ALL_DAY_GHOST_MS / (day.end_ms - day.start_ms)) * 100;
      const top = Math.min(Math.max(sweptAllDayFrac * 100, 0), 100 - height);
      return `${ink}top:${top}%;height:${height}%`;
    }
    const s = Math.max(formPreview.startMs, day.start_ms);
    const e = Math.min(formPreview.endMs, day.end_ms);
    if (e <= s) return null;
    const span = day.end_ms - day.start_ms;
    const top = ((s - day.start_ms) / span) * 100;
    const height = ((e - s) / span) * 100;
    return `${ink}top:${top}%;height:${height}%`;
  }

  /** The continuation edges for an all-day draft, so a multi-day one reads
   *  as one dashed bar exactly as its sweep did. */
  function formPreviewEdges(day: { start_ms: number }): { cl: boolean; cr: boolean } {
    if (formPreview?.kind !== 'allDay') return { cl: false, cr: false };
    const d = dateOf(day.start_ms);
    return { cl: d > formPreview.firstDate, cr: d < formPreview.lastDate };
  }

  /**
   * Hands the caller the clicked occurrence and closes this popover.
   *
   * `occ` and `rect` are captured by the *caller of this function* at render
   * time (see the `{@const}`s below), never read back off the module state
   * here: `closePopover` clears all of it on the next line, and both handlers
   * need values that survive that.
   */
  function relay(
    to: (occurrence: Occurrence, rect: Rect) => void,
    occurrence: Occurrence,
    rect: Rect,
  ) {
    closePopover();
    to(occurrence, rect);
  }

  // A successful "this one" RSVP against a bare master leaves `detail`
  // itself unchanged — the backend deliberately skips its local write-back
  // there (see `respond_to_event`'s own comment) — so nothing about the
  // response can be read back off `detail`. `EventPopover` reports the
  // response it just landed directly; recording it as an override, rather
  // than waiting on a re-fetch, is what makes the grid restyle without
  // waiting on the next sync.
  //
  // `id`/`startMs` are captured by the caller *at render time* (see the
  // `{@const}` below), not read back off `selectedId`/`selectedStartMs`
  // here: `respond()` is async and keeps running after this popover
  // unmounts, and a scrim click or another block opening in the meantime
  // would otherwise record the override under the wrong occurrence, or not
  // at all.
  //
  // Two things this deliberately does not do, both self-correcting:
  //
  // Answering with `scope: 'all'` restyles only the block that was clicked,
  // even though every occurrence of that series just changed. The override is
  // keyed by `id:startMs` and this only knows the one occurrence, so the rest
  // of the week keeps the payload's own colour until the next `week` lands —
  // at which point they all agree, and the clicked block's override is
  // evicted by the effect above for disagreeing with its baseline.
  //
  // A `scope: 'this'` RSVP against a bare master materialises an exception on
  // Google's side. The next sync stores that exception as a row of its own,
  // so the occurrence comes back with a *different* store row id — and this
  // override's key, built from the master's, can never match anything again.
  // It is inert rather than wrong (nothing renders against a key nothing
  // has), and the eviction effect cannot reach it either, since
  // `payloadResponse` returns `undefined` for it. It simply sits in the Map
  // until the app closes.
  function handleResponded(id: number, startMs: number, response: 'accepted' | 'tentative' | 'declined') {
    const baseline = payloadResponse(id, startMs);
    // Not in this week's payload (any more) — nothing to restyle, and
    // nothing to record a baseline against.
    if (baseline === undefined) { onresponded?.(); return; }
    const next = new Map(responseOverrides);
    next.set(overrideKey(id, startMs), { response, baseline });
    responseOverrides = next;
    // The override bridges the visible gap; this is what closes it for real.
    onresponded?.();
  }

  /** Horizontal panning (spec 2026-08-28 §3; continuous since 2026-09-03):
   *  the track follows the finger a column per column, hands whole days up
   *  as they are crossed, and settles on the nearest day after a lull. The
   *  old rule was a day per 90px of wheel with nothing moving in between —
   *  the finger's travel and the column's never matched, and every day was
   *  a fetch away. Only a dominantly horizontal wheel is consumed —
   *  vertical scrolling through the hours stays entirely native. */
  function wheelPan(e: WheelEvent) {
    // Ctrl+scroll zooms the hours (2026-09-03). `App` has already cancelled
    // the browser's own page zoom on every Ctrl+wheel, so this only has to
    // give the gesture a meaning; anchored on the pointer, and on the
    // *body's* frame even from the day headers above it, which share this
    // handler but not this scroller.
    if (e.ctrlKey) {
      e.preventDefault();
      if (bodyEl) zoomAnchorY = Math.max(0, e.clientY - bodyEl.getBoundingClientRect().top);
      hourPx = hourPxAfterWheel(hourPx, e.deltaY);
      return;
    }
    if (!onpan || Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault();
    if (!panActive) {
      panColWidth = colWidth();
      panSamples = [];
    }
    if (panColWidth <= 0) return;
    cancelAnimationFrame(snapRaf);
    panActive = true;
    const days = -(e.deltaX * PAN_GAIN) / panColWidth;
    panDays += days;
    panSamples.push({ t: performance.now(), days });
    if (panSamples.length > 12) panSamples.shift();
    const { shift, rest } = panCommit(panDays);
    if (shift !== 0) {
      // Both in one flush: the window moves a day and the track moves back
      // a column, and the day under the finger stays where it is.
      panDays = rest;
      onpan(shift);
    }
    clearTimeout(panLull);
    panLull = setTimeout(settlePan, PAN_LULL_MS);
  }
</script>

<svelte:window onkeydown={trackAlt} onkeyup={trackAlt} onpointermove={trackPointer}
  onblur={() => { altHeld = false; hoveredOccurrence = null; }}
  bind:innerWidth={viewportWidth} bind:innerHeight={viewportHeight} />

<div class="grid" style="--cols:{renderedDays.length}; --visible:{visible}; --vis:{renderVis}; --pan:{panDays}; --gutter:{gutterWidth()}" onwheel={wheelPan}>
  <div class="gutter head">
    {#if secondZone()}
      <!-- Which clock is which, Google's own layout: the convenience zone in
           the outer lane, the zone the grid actually lives in beside the
           columns it governs. Absent entirely with one clock — a ruler that
           has always been unlabelled does not grow a caption for nothing. -->
      <span class="zl z2">{zoneAbbrev(secondZone()!)}</span>
      <span class="zl z1">{zoneAbbrev(primaryZone)}</span>
    {/if}
  </div>
  <div class="track"><div class="cols" class:sliding={panActive}>
  {#each renderedDays as d (d.start_ms)}
    <div class="head" class:today={d.start_ms === todayStart}
         class:keyboard={keyboardCursor?.dayStartMs === d.start_ms}>
      <!-- One line since 2026-09-04, by request: the day name, the number and
           the sky side by side rather than the name stacked over the other
           two. It gives the grid back a row's worth of height at the top of
           every week. Three grid tracks (1fr / auto / 1fr) rather than a
           flow, because the number must sit at the column's centre whether
           or not that day has a forecast — a week half-covered by weather
           must not have its numbers zigzag, which is the invariant the old
           absolutely-positioned `.wx` existed to keep. -->
      <span class="dow">{dayName(d.start_ms)}</span>
      <b>{new Date(d.start_ms).getDate()}</b>
      {#if weather?.get(dateKey(d.start_ms))}
        {@const wx = weather.get(dateKey(d.start_ms))!}
        <!-- Absent for any day the forecast does not cover — the past, the
             far future — so the header never guesses; the empty third track
             holds the number in place regardless. -->
        <!-- A button since the card (2026-09-07): the same label it was,
             now the way into the forecast for that day and, first of all,
             for which place. -->
        <button type="button" class="wx" class:stale={weatherStale}
                aria-label="Weather for {dayName(d.start_ms)} {new Date(d.start_ms).getDate()}: {wx.bucket}, {formatTemp(wx.tmax, temperatureUnit())}°{weatherStale ? ', possibly out of date' : ''}"
                onclick={(e) => {
                  e.stopPropagation();
                  onweather?.(d.start_ms, (e.currentTarget as HTMLElement).getBoundingClientRect());
                }}>
          <WeatherGlyph bucket={wx.bucket} size={15} />{formatTemp(wx.tmax, temperatureUnit())}°
        </button>
      {/if}
    </div>
  {/each}
  </div></div>
</div>

<!-- `openPopover` unchanged, and deliberately so: a chip hands it the same
     `UiEvent` + viewport rect an `EventBlock` does, and an all-day
     occurrence's `start_ms` is its own day (`commands::assemble_week` calls
     `to_ui` per expanded occurrence), which is exactly what
     `occurrenceStartMs` has to carry. -->
<!-- Tasks get a row of their own rather than a place in the all-day band:
     an event is a commitment at a time and a task is something to finish,
     and a band holding both makes the busiest strip on the screen busier.
     Drawn only when the visible week has something due, so a week with no
     tasks carries no chrome for them. -->
{#if anyTasks}
  <div class="trow" class:dragging={taskDrag?.moving}>
    <div class="tgutter">TASKS</div>
    <div class="tcols" style="--cols:{renderedDays.length}">
      {#each renderedDays as d (d.start_ms)}
        <div class="tcell" class:drop={taskDropMs === d.start_ms && taskDropMs !== taskDrag?.fromMs}>
          {#each tasks?.get(dateKey(d.start_ms)) ?? [] as chip (chip.id)}
            {#if !inFlight(chip)}
              <div class="tchip" class:over={chip.overdue} style:--cal={chip.color ?? 'var(--muted)'}>
                <input
                  type="checkbox"
                  checked={false}
                  disabled={!chip.canWrite}
                  aria-label="Complete {chip.summary}"
                  onchange={() => ontasktoggle?.(chip.id, true)}
                />
                <!-- The title is the grab handle: a button rather than the
                     whole chip, so the checkbox beside it stays its own
                     control instead of nesting inside one. -->
                <button class="tt" disabled={!chip.canWrite}
                        onpointerdown={(e) => startTaskDrag(chip, d.start_ms, e)}>{chip.summary}</button>
              </div>
            {/if}
          {/each}
          <!-- The chip in flight, drawn in the column it would land in. -->
          {#if taskDropMs === d.start_ms && taskDrag}
            {#each tasks?.get(dateKey(taskDrag.fromMs)) ?? [] as chip (chip.id)}
              {#if inFlight(chip)}
                <div class="tchip landing" class:over={chip.overdue}
                     style:--cal={chip.color ?? 'var(--muted)'}>
                  <span class="tt">{chip.summary}</span>
                </div>
              {/if}
            {/each}
          {/if}
        </div>
      {/each}
    </div>
  </div>
{/if}

<AllDayBand
  lanes={renderedLanes}
  events={week.all_day_events}
  overflow={renderedHidden}
  expanded={bandExpanded}
  onexpand={() => (bandExpanded = !bandExpanded)}
  columns={renderedDays.length}
  {visible}
  vis={renderVis}
  pan={panDays}
  sliding={panActive}
  dayStarts={renderedDays.map((day) => day.start_ms)}
  {keyboardCursor}
  onopen={openPopover}
/>

<div class="grid body quiet-scroll" class:creating={createMode} style="--cols:{renderedDays.length}; --visible:{visible}; --vis:{renderVis}; --pan:{panDays}; --gutter:{gutterWidth()}; --hour-px:{Math.round(hourPx)}px" bind:this={bodyEl} data-testid="week-body" onwheel={wheelPan}>
  <div class="hour-crop ruler" style:height={`${visibleHeight}px`}><div class="gutter" style={columnStyle(gutterDay)}>
    {#each HOURS as h}
      {#if secondZone()}
        <!-- The second clock's reading of this same rule — one instant, two
             spellings, which is the entire feature. Same top, so the eye can
             run straight across. -->
        <span class="z2" style="top:{hourFrac(gutterDay, h) * 100}%">
          {zoneGutterLabel(hourMs(gutterDay, h), secondZone()!, clockFormat())}</span>
      {/if}
      <span style="top:{hourFrac(gutterDay, h) * 100}%">{gutterLabel(h, clockFormat())}</span>
    {/each}
  </div></div>

  <div class="track"><div class="cols" class:sliding={panActive}>
  {#each renderedDays as day, dayIndex (day.start_ms)}
    {@const isToday = day.start_ms === todayStart}
    {@const ghost = sweepStyle(day)}
    <div class="hour-crop" style:height={`${visibleHeight}px`}><div class="col" role="group" style={columnStyle(day)} class:today={isToday}
         onpointermove={(e) => trackEventHover(day, e)}
         onpointerleave={() => { hoveredOccurrence = null; }}
         class:keyboard={keyboardCursor?.dayStartMs === day.start_ms}
         data-start-ms={day.start_ms}
         data-kbd-selected-day={keyboardCursor?.dayStartMs === day.start_ms ? '' : undefined}>
      <!-- Empty grid space, as a real control rather than a click handler on
           the column div: the role, the pointer target and the accessible name
           come with the element. First in the column so every block, rule and
           now-line paints over it, and `tabindex="-1"` because seven identical
           invisible tab stops per week would be noise — the keyboard route to
           the same form is `n`, which needs no target at all.

           Both a click and a press: a click creates at the half hour it landed
           in, exactly as it always has, and a press that then travels 4px
           sweeps a span out instead. The threshold is what keeps the older of
           the two working. -->
      <button
        class="newhere"
        aria-label="New event"
        tabindex="-1"
        onclick={(e) => startCreate(day, e)}
        onpointerdown={(e) => {
          // Right-click is the other spelling of "new event here" — but not
          // right-DRAG, which "a right-button drag over empty grid sweeps
          // nothing" pins: a create cannot ride `contextmenu`, which fires at
          // the press, before the gesture has a shape. So the press is only
          // remembered here, and the release below decides — the same
          // threshold discipline the left button's click/sweep split uses.
          if (e.button === 2) {
            rightPress = { x: e.clientX, y: e.clientY };
            return;
          }
          startSweep(day, dayIndex, e);
        }}
        onpointerup={(e) => {
          if (e.button !== 2 || !rightPress) return;
          const still = !beganDrag(e.clientX - rightPress.x, e.clientY - rightPress.y);
          rightPress = null;
          if (still) startCreate(day, e);
        }}
        oncontextmenu={(e) => e.preventDefault()}
      ></button>

      {#each HOURS as h}
        <div class="rule" style="top:{hourFrac(day, h) * 100}%"></div>
      {/each}

      <!-- Raised above saved events, including the one under the pointer.
           Pointer-transparent so the preview cannot interrupt its own sweep. -->
      {#if ghost}
        {@const edge = sweepEdges(day)}
        <div class="sweep" class:cl={edge.cl} class:cr={edge.cr} style={ghost}></div>
      {/if}

      <!-- The open form's live ghost: above the blocks (a draft usually
           overlaps something) and translucent and dashed so it reads as
           not-yet-real.
           It takes the pointer when it is a *timed* draft, because a draft
           should be placeable by hand the way a saved event is; an all-day
           ghost stays transparent to it, since the gesture that places one
           is the sideways sweep it must not interrupt. -->
      {#if formPreviewStyle(day)}
        {@const fedge = formPreviewEdges(day)}
        {@const grabbable = !!ondraftmove && formPreview?.kind === 'timed'}
        <button
          type="button"
          class="formghost" class:cl={fedge.cl} class:cr={fedge.cr}
          class:grabbable data-testid="form-preview" style={formPreviewStyle(day)}
          tabindex="-1"
          aria-hidden={grabbable ? undefined : 'true'}
          aria-label={grabbable ? 'Draft event — drag to move, drag an edge to resize' : undefined}
          onpointerdown={grabbable ? (e) => startDraftDrag(e, day) : undefined}
        ></button>
      {/if}

      <!-- A drag or sync can reorder the packed array. Keep focus and copy
           panels with their occurrence, never with its old array position. -->
      {#each day.placed as p (`${day.events[p.idx].id}:${day.events[p.idx].start_ms}`)}
        <EventBlock
          event={day.events[p.idx]}
          placed={p}
          {createMode}
          {calendars}
          onfocuschange={(focused) => {
            const event = day.events[p.idx];
            if (focused) {
              hoveredOccurrence = { day: day.start_ms, id: event.id, startMs: event.start_ms };
            } else if (hoveredOccurrence?.day === day.start_ms && hoveredOccurrence.id === event.id
              && hoveredOccurrence.startMs === event.start_ms) {
              // Removing a copy panel can blur it after hover has already
              // moved to a neighbor. Only release this event's expansion.
              hoveredOccurrence = null;
            }
          }}
          onopencopy={(copy, rect, thenEdit = false) => {
            draggedNotClicked = false;
            void openPopover({ ...day.events[p.idx], ...copy }, rect, thenEdit);
          }}
          hovered={hoveredPlacement?.day === day.start_ms && hoveredPlacement.idx === p.idx}
          overlapColors={hoverContext?.day === day.start_ms && hoverContext.idx === p.idx
            && (!day.events[p.idx].copies?.length || hoverContext.peers.length > 1) ? hoverContext.segments : []}
          copyHoverLane={hoverContext?.day === day.start_ms && hoverContext.idx === p.idx ? hoverContext.copyHoverLane : undefined}
          obscured={hoverContext?.day === day.start_ms && hoverContext.idx !== p.idx
            && hoverContext.peers.some(peer => peer.idx === p.idx)}
          onopen={openPopover}
          onedit={(ev, r) => openPopover(ev, r, true)}
          ongrab={(ev, e) => startDrag(ev, day, e)}
          preview={previewFor(day.events[p.idx])}
          liveSpan={liveSpanFor(day.events[p.idx])}
          keyboardSelected={keyboardCursor
            ? cursorNamesEvent(keyboardCursor, day.start_ms, day.events[p.idx])
            : false}
        />
      {/each}

      {#if isToday}
        <div
          class="now"
          style="top:{((nowMs - day.start_ms) / (day.end_ms - day.start_ms)) * 100}%"
        ></div>
      {/if}
    </div></div>
  {/each}
  </div></div>
</div>

{#if sweep && sweepFeedback}
  <div class="sweep-feedback" role="status" aria-label="New event duration"
    bind:clientWidth={feedbackWidth} bind:clientHeight={feedbackHeight}
    style:left="{Math.max(8, Math.min(sweep.clientX + 16, viewportWidth - feedbackWidth - 8))}px"
    style:top="{Math.max(8, sweep.clientY + 16 + feedbackHeight > viewportHeight - 8
      ? sweep.clientY - feedbackHeight - 16 : sweep.clientY + 16)}px">
    <strong>{sweepFeedback.duration}</strong>
    <span>{sweepFeedback.range}</span>
  </div>
{/if}

{#if selectedId !== null && selectedStartMs !== null && anchor && detail}
  <!-- `id`/`startMs` are captured *now*, at this render, not read back off
       `selectedId`/`selectedStartMs` inside the callback below. `respond()`
       is async and keeps running after this block unmounts — a scrim click
       or Escape while an RSVP is still in flight clears the selection (or,
       after another block is opened, replaces it) before the response
       lands. Reading the module-level state at that point would restyle
       the wrong block or nothing at all; closing over the pair captured
       here restyles the one block this popover was ever open for,
       regardless of what has since been clicked. -->
  {@const id = selectedId}
  {@const startMs = selectedStartMs}
  <!-- `endMs` falls back to `startMs` only to satisfy the type: the `{#if}`
       above already proves a block is selected, and `selectedEndMs` is
       assigned and cleared in lockstep with `selectedStartMs`, so the
       fallback is unreachable. Written this way rather than added to the
       `{#if}` because a fourth condition there would suggest the three
       states can disagree. -->
  {@const occurrence = { detail, startMs, endMs: selectedEndMs ?? startMs }}
  {@const rect = anchor}
  {#key detail.id}
  <EventPopover
    {detail}
    {calendars}
    copies={selectedEvent?.copies}
    onchoosecopy={(copy) => { if (selectedEvent) void openPopover({ ...selectedEvent, ...copy }, rect); }}
    {anchor}
    occurrenceStartMs={startMs}
    occurrenceEndMs={occurrence.endMs}
    onclose={closePopover}
    onresponded={(r) => handleResponded(id, startMs, r)}
    onedit={() => relay(onedit, occurrence, rect)}
    ondelete={() => relay(ondelete, occurrence, rect)}
    oncopy={() => oncopy(occurrence)}
    onduplicate={onduplicate ? () => relay(onduplicate!, occurrence, rect) : null}
  />
  {/key}
{/if}

<style>
  /* Clipped **vertically only** (#116). The crop exists to cut the hours
     outside the visible range, which is a vertical question — but `overflow:
     clip` cut sideways too, and a move preview is a `translateX`: one column
     over put the whole block outside its own day's box and the user watched
     their event vanish mid-drag. The drop still landed correctly, so every
     write test passed and only the painting was wrong.

     `visible` survives beside `clip` rather than computing to `auto` — the
     one pairing the spec allows — so this keeps the vertical cut and adds no
     scroll container. The track below still clips the week horizontally,
     which is what stops a dragged block escaping into the ruler. */
  .hour-crop { overflow-x: visible; overflow-y: clip; min-width: 0; }
  /* Real headroom works in WebKit too; overflow-clip-margin alone does not.
     The negative margin keeps hour labels aligned with the event grid. */
  .ruler { padding-top: 8px; margin-top: -8px; }
  .creating .newhere { cursor: crosshair; }
  /* `--gutter` from `secondzone.svelte`'s one exported width: 44px alone,
     wider when the second clock takes the outer lane. A var rather than two
     hardcodings because the head row here, the body row below and the
     all-day band between them must all move together or the columns shear. */
  .grid { display: grid; grid-template-columns: var(--gutter, 44px) 1fr; }
  /* The track (2026-09-03): the ruler stays put and the columns slide past
     it. `.cols` is as many columns wide as the payload has days, each
     exactly a visible column wide, pulled left by the padding before the
     window and pushed by the finger's travel — so at rest, with the window
     alone drawn, it is 100% wide at offset zero, the grid this was before
     it could slide. A margin rather than a transform, deliberately: a
     transformed ancestor becomes the containing block of every
     `position: fixed` descendant, and EventBlock's tooltip is one. `clip`
     rather than `hidden`, because `hidden` on one axis turns the other into
     a scroller, and this must never scroll the hours on its own. */
  .track { min-width: 0; overflow-x: clip; }
  .cols { display: grid; grid-template-columns: repeat(var(--cols), 1fr);
          width: calc(100% * var(--cols) / var(--visible)); }
  /* Moved by transform only while sliding (2026-09-03): a margin re-laid
     out three weeks of columns on every wheel event, and that was the lag.
     A transform is the compositor's alone — but a transformed ancestor is
     the containing block of every `position: fixed` descendant, and
     EventBlock's tooltip is one, so the transform exists only during the
     gesture, when nothing is being hovered, and the tooltip is hidden for
     its duration. At rest there is no transform at all and the window's
     first column sits at the track's edge by construction. The percentage
     is of `.cols`' own width, `--cols` columns wide. */
  .cols.sliding { transform: translateX(calc((var(--pan) - var(--vis)) / var(--cols) * 100%));
                  will-change: transform; }
  .cols.sliding :global(.tip) { display: none; }
  /* The last of this component's three roots, and the only one that stretches:
     the day-name row and the all-day band above it are content-sized, so
     `flex: 1` here means "the rest of whatever App's `main` has left", rather
     than the `calc(100vh - 150px)` guess this replaced. It shrinks below its
     1200px of columns and scrolls them rather than pushing the window, and
     `overflow-y` is what buys that: a flex item whose overflow is not
     `visible` has no automatic minimum size, so no `min-height: 0` is needed
     beside it. Measured — adding one moves nothing, at 400px or at 720p. */
  /* 8px of headroom, for the ruler's first labels: every label centres on
     its rule (`translateY(-50%)` below), so hour 0's top half has always
     hung above this box's edge and been clipped at scroll-top. Half a "00"
     was furniture nobody missed; the second zone made that same label read
     "21:30" and the clipping started hiding information (reported
     2026-08-26, the first field run of v0.6.0). Padding rather than a
     special case for hour 0: the rules position as fractions *inside* the
     columns, so everything — labels, rules, blocks, the now line — shifts
     down together and nothing can shear. */
  .body { flex: 1; overflow-y: auto; position: relative; padding-top: 8px; }

  .head { text-align: center; font-size: 11px; color: var(--muted);
          letter-spacing: .05em; padding-bottom: 8px; }
  /* The day headers only: `.gutter.head` above the ruler carries the two
     zone labels and wants none of this. The outer tracks are equal, so the
     number holds the column's centre whatever sits beside it, and a
     container query rather than a window one because what runs out of room
     is the *column*, which the window's width alone cannot say (seven of
     them, or three, or one). */
  .head:not(.gutter) { display: grid; grid-template-columns: 1fr auto 1fr;
                       align-items: center; column-gap: 6px;
                       container-type: inline-size; }
  .dow { justify-self: end; }
  /* The sky, at the number's own size, in the day name's muted voice.
     Tabular °digits, so a week of temperatures forms a row the eye can run
     across. Dropped below a column width that cannot hold it: a clipped
     forecast is worth less than the date it would crowd. */
  .wx { justify-self: start; display: inline-flex; align-items: center; gap: 3px;
        font-size: 15px; font-weight: 500; letter-spacing: -.02em;
        color: var(--muted); font-variant-numeric: tabular-nums;
        /* A button that looks exactly like the label it replaced. */
        appearance: none; -webkit-appearance: none; background: none; border: 0;
        padding: 0; margin: 0; font-family: inherit; cursor: pointer; }
  .wx:hover, .wx:focus-visible { color: var(--text); }
  /* Faded, not hidden: the number is still the best one there is, and the
     fade is what makes somebody click and find out it is two days old. */
  .wx.stale { opacity: .45; }

  /* The tasks row. Quieter than the all-day band above it: a task is
     something to finish rather than a commitment at a time, and the row
     says so by being flatter — no fill of the calendar's colour, a tick
     instead of a spine, and a checkbox as the first thing in it. */
  .trow { display: flex; border-top: 1px solid var(--hairline);
          border-bottom: 1px solid var(--hairline);
          background: color-mix(in srgb, var(--text) 3%, transparent); }
  .trow.dragging { cursor: grabbing; }
  .tgutter { width: var(--gutter); flex: 0 0 var(--gutter); font-size: 9.5px;
             color: var(--muted); letter-spacing: .06em; text-align: right;
             padding: 7px 8px 0 0; }
  .tcols { flex-grow: 1; display: grid; grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
           padding: 4px 0; min-width: 0; }
  .tcell { min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-right: 3px; }
  .tcell.drop { background: color-mix(in srgb, var(--accent) 7%, transparent);
                box-shadow: inset 1px 0 0 0 color-mix(in srgb, var(--accent) 40%, transparent),
                            inset -1px 0 0 0 color-mix(in srgb, var(--accent) 40%, transparent); }
  /* Not `.chip`: the all-day band already owns that name, and a spec
     asking for one would find both. */
  .tchip { display: flex; align-items: center; gap: 6px; min-width: 0;
          font-size: 11.5px; font-weight: 500; padding: 2px 7px;
          border-radius: var(--event-chip-radius, 4px);
          background: color-mix(in srgb, var(--cal) 10%, var(--bg));
          color: var(--text); }
  .tchip.over { background: color-mix(in srgb, var(--error) 12%, var(--bg)); }
  .tchip.landing { outline: 1px solid var(--accent); cursor: grabbing; }
  .tchip input { width: 11px; height: 11px; flex: 0 0 11px; margin: 0; }
  .tt { appearance: none; -webkit-appearance: none; font: inherit; border: 0; background: none;
        color: inherit; padding: 0; text-align: left; min-width: 0; cursor: grab;
        touch-action: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tt:disabled { cursor: default; }
  @container (max-width: 104px) { .wx { display: none; } }
  .head b { font-size: 15px; color: var(--text);
            font-weight: 500; letter-spacing: -.02em; }
  .head.today b { background: var(--accent); color: var(--on-accent); width: 23px; height: 23px;
                  line-height: 23px; border-radius: 50%; font-weight: 600; }
  .head.keyboard:not(.today) b { color: var(--accent); font-weight: 650; }

  /* No column borders: the grid reads through alignment, not rules (spec §7.1). */
  /* 70px per hour (24 x 70 = 1680), up from 50 (2026-08-14): at 50 a
     typical laptop pane put ~19 hours on screen at once and every slot was
     a sliver — macOS shows about ten. This is a *floor on slot height*, not
     a cap on hours: the pane divided by 70 is what fits, so a small window
     shows ~12 hours and a tall monitor simply shows more. The initial
     scroll, the sweep math and the hour rules are all fractions of the
     column, so nothing else knows the number — which is what made it
     zoomable (2026-09-03): `--hour-px` is `hourPx`, 70 until a pinch,
     Ctrl+scroll or Ctrl+=/- says otherwise, and `zoom.ts` holds the range. */
  .col { position: relative; min-height: calc(var(--hour-px, 70px) * 24); }
  .col.today { background: var(--today-tint); border-radius: 6px; }
  .col.keyboard { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
                  border-radius: 6px; }

  .gutter { position: relative; }
  /* 11.5px at .85, up from 10.5 at .7 (2026-08-26, by request twice over):
     the ruler was the faintest text on the grid — fine as furniture, hard
     to actually read a meeting's hour off — and once a second clock stood
     beside it, the two lanes were indistinguishable in rank. */
  .gutter span { position: absolute; right: 8px; font-size: 11.5px; color: var(--muted);
                 opacity: .85; transform: translateY(-50%); font-variant-numeric: tabular-nums; }
  /* The second clock's lane, one step down in both size and wash — the
     hierarchy is the point: the primary is the ruler the grid obeys, the
     second an annotation about it, and quieting the annotation says so
     without the primary having to shout. Offset past the primary labels so
     the two columns of digits stay columns. */
  .gutter span.z2 { right: 60px; font-size: 10.5px; opacity: .55; }

  /* The zone captions over the ruler, only rendered when there are two
     clocks to tell apart. Small on purpose — they are column headers for
     digits, not content — and bottom-aligned so they sit just over the
     first labels the way the day names sit over their columns.

     The outer caption anchors LEFT while its digits anchor right, and the
     width cap is load-bearing: two "GMT+X:30"-shaped names right-anchored
     to adjacent lanes met in the middle and read as one mashed string
     ("GMT+3GMT+5:30" on the first field run, 2026-08-26). Left vs right
     anchoring keeps the gap where the names are widest, and the ellipsis
     bounds the worst pair the tz database can produce. */
  .zl { position: absolute; bottom: 8px; font-size: 9px; color: var(--muted);
        letter-spacing: .04em; max-width: 46px; overflow: hidden;
        text-overflow: ellipsis; white-space: nowrap; }
  /* The captions carry their lanes' own ranks. */
  .zl.z2 { left: 4px; opacity: .55; }
  .zl.z1 { right: 8px; opacity: .85; }

  /* Fills the column, paints nothing, and sits under everything else in it —
     it is first in the DOM and every sibling that could cover it is either
     positioned later or explicitly transparent to the pointer below. */
  .newhere { appearance: none; -webkit-appearance: none; position: absolute; inset: 0;
             background: none; border: 0; padding: 0; margin: 0; font: inherit;
             cursor: cell; }

  /* `pointer-events: none` on both, and load-bearing — measured, not assumed.
     They are positioned *after* `.newhere` in the column, so without this the
     hour lines and the current-time line swallow the click instead: probed in
     both engines, a point within half a pixel of an hour line returns `.rule`
     from `elementFromPoint`, and further away returns `.newhere`. That is a
     1px dead band every hour, sitting exactly on the line somebody aims
     at to make a 10:00 meeting. `WeekGrid`'s "clicking exactly on an hour
     line" spec fails the moment the declaration on `.rule` goes.

     `.now` is the same geometry — plus a 7px dot — and gets the same treatment
     for the same reason, but has no spec of its own: it renders only in
     today's column, and every fixture here is anchored on a Monday fixed in
     the past precisely so that nothing driven by the real wall clock can
     appear (see `MON` in fixtures.ts). Reaching it would mean a fixture whose
     week moves with the calendar, which is a worse trade than an unspec'd
     one-line declaration. */
  .rule { position: absolute; left: 0; right: 0; border-top: 1px solid var(--hour-rule);
          pointer-events: none; }

  /* The span being swept out, drawn as the event it is about to become: the
     same 6px radius and the same left spine an `EventBlock` has, so what the
     gesture promises and what appears afterwards read as the same object.
     `--ghost` is the calendar the create would land on, declared inline by
     `sweepStyle`; `--accent` stands in when there is no such calendar. */
  .sweep { position: absolute; left: 3px; right: 3px;
           border-radius: var(--event-card-radius, 6px);
           background: color-mix(in srgb, var(--ghost, var(--accent)) 14%, var(--bg));
           box-shadow: inset 2px 0 0 0 var(--ghost, var(--accent));
           pointer-events: none; z-index: 60; }
  .sweep-feedback { position: fixed; z-index: 70; pointer-events: none;
                    display: flex; flex-direction: column; gap: 3px;
                    padding: 7px 10px; border-radius: 6px;
                    border: 1px solid var(--accent); background: var(--surface);
                    color: var(--text); box-shadow: 0 3px 12px rgba(0, 0, 0, .3);
                    max-width: calc(100vw - 16px); font-size: 12px; }
  .sweep-feedback strong { font-size: 13px; }
  /* A multi-day ribbon: square off the edges that continue, and drop the
     spine on every segment but the first, so N columns read as one bar and
     not as N separate events. A continuing edge sits at **0** — the column's
     own boundary, which its neighbour also starts at, so the two meet
     exactly. Negative values put the edge *outside* the column instead: the
     first attempt used -4px both sides, overlapping by 8px, and two
     translucent fills stacked there drew a bright seam at every join — the
     very thing joining them was meant to remove. Measured, not eyeballed. */
  .sweep.cl { left: 0; border-top-left-radius: 0; border-bottom-left-radius: 0;
              box-shadow: none; }
  .sweep.cr { right: 0; border-top-right-radius: 0; border-bottom-right-radius: 0; }
  /* The dashed draft continues the same way, so releasing the button
     changes the ribbon's dress and not its shape. */
  .formghost.cl { left: 0; border-top-left-radius: 0; border-bottom-left-radius: 0;
                  border-left: 0; }
  .formghost.cr { right: 0; border-top-right-radius: 0; border-bottom-right-radius: 0;
                  border-right: 0; }

  /* The form's draft, dressed as not-yet-real: dashed where every real
     block is solid, tinted rather than filled so whatever it covers still
     reads through — and in the colour of the calendar it would be written
     to, which the open form can change under it (`--ghost`, declared inline
     by `formPreviewStyle`; `--accent` when the calendar lends no colour). */
  /* A <button> for the reason `.ev` is one: it is grabbed, and an element
     with a pointer handler owes the keyboard and the screen reader an
     answer. `appearance` cleared for `.ev`'s reason too — WKWebView keeps
     native chrome otherwise. Never a tab stop: the keyboard route to these
     times is the form's own fields, which are two tabs away and can say
     "09:30" exactly. */
  .formghost { appearance: none; -webkit-appearance: none; padding: 0;
               position: absolute; left: 3px; right: 3px;
               border-radius: var(--event-card-radius, 6px);
               background: color-mix(in srgb, var(--ghost, var(--accent)) 18%, transparent);
               border: 1.5px dashed var(--ghost, var(--accent));
               pointer-events: none; z-index: 6; }
  /* Only a timed draft is grabbed, and it wears the same cursors a saved
     block does — `grab` over the body, `ns-resize` in the same
     `RESIZE_EDGE_PX` band at each end — so the gesture is discoverable by
     the hand rather than only by being told about it. */
  /* Above the form's scrim, which is `position: fixed; inset: 0; z-index: 40`
     and otherwise swallows every press on the grid behind it — the draft
     would be visibly there and untouchable. 41 rather than 42 so the form
     itself still wins: `.pop` is also 41 and `App` renders the form after
     the grid, so paint order puts the panel over the ghost where they
     overlap. Neither `.col` nor `.body` creates a stacking context, so these
     numbers compare directly. */
  .formghost.grabbable { pointer-events: auto; cursor: grab; touch-action: none;
                         z-index: 41; }
  .formghost.grabbable::before,
  .formghost.grabbable::after { content: ''; position: absolute; left: 0; right: 0;
                                height: 6px; cursor: ns-resize; }
  .formghost.grabbable::before { top: -1.5px; }
  .formghost.grabbable::after { bottom: -1.5px; }

  /* The loudest thing on screen, deliberately. */
  .now { position: absolute; left: 0; right: 0; border-top: 1.5px solid var(--now); z-index: 5;
         pointer-events: none; }
  .now::before { content: ''; position: absolute; left: -3px; top: -3.5px;
                 width: 7px; height: 7px; border-radius: 50%; background: var(--now); }
</style>
