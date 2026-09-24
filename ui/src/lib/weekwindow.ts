// The window on a padded week payload, and the arithmetic of sliding it.
//
// Since 2026-09-03 the day grids fetch more days than they show: `padFor`
// either side of the window, in one payload, so a sideways swipe has real
// columns to reveal before the fetch for the new window lands (the old
// pan jumped a day per 90px of wheel and refetched on every jump — content
// arrived in lurches, and nothing moved under the finger). Everything that
// is about *what is on screen* reads the window; only the grid's track ever
// draws the padding. Pure, so `weekwindow.spec.ts` pins each rule.

import type { DayColumn, Lane, WeekPayload } from './api';

/** Days of padding either side of a window of `visible` days: the window's
 *  own width plus two, so a strong swipe can page a whole window (the ‹ / ›
 *  step, see `settleTarget`) and still land inside what is already on the
 *  track, with `windowHeld`'s two-day margin to spare. That margin is what
 *  defers the refetch until the settle is over instead of replacing the
 *  payload under the last frames of the motion. Never fewer than three, so
 *  Day view has somewhere to go. */
export const padFor = (visible: number) => Math.max(3, visible + 2);

/**
 * Where the window starts in `days`: the index of the day beginning at
 * `visibleStartMs`, or -1 for a payload that does not hold it — one still
 * on screen from before the window jumped (a view switch, Today from far
 * away), or an unpadded one from a stub. The callers then show the whole
 * payload, which is what showed before it was padded: the honest answer to
 * "the days you asked for are not here yet" is the days that are, not a
 * window's worth of whatever happens to sit at the start.
 */
export function visibleIndex(days: { start_ms: number }[], visibleStartMs: number): number {
  return days.findIndex((d) => d.start_ms === visibleStartMs);
}

/**
 * The payload as if only `count` days from `from` had been fetched.
 *
 * Days are sliced; an all-day lane is kept if any of it falls inside, cut to
 * the edges, and marked continuing where it was cut — the same `cont_left`/
 * `cont_right` the backend sets for a span crossing the fetched range, so
 * the band draws a padded payload's window exactly as it drew the unpadded
 * fetch. `all_day_events` is left whole: lanes index into it. `overflow` is
 * left whole too — an event the wider packing pushed off the band is a
 * judgement about the wider range, and re-packing here would be a second
 * lane packer to disagree with the first.
 */
export function sliceWeek(
  week: WeekPayload, from: number, count: number, maxRows = Infinity,
): WeekPayload {
  if (from === 0 && count >= week.days.length && maxRows === Infinity) return week;
  const { lanes, hidden } = packBandLanes(week.all_day, from, count, false, maxRows);
  return {
    ...week,
    days: week.days.slice(from, from + count),
    all_day: lanes,
    // **The window's own hidden events, not the payload's.** `overflow`
    // arrives counted over everything fetched, which since the padded
    // window (v1.1.0) is three weeks — so the band's "+N more" counted
    // events from the weeks either side of the one on screen. Whatever the
    // backend could not position is added, so a count is never short.
    overflow: [...hidden, ...week.overflow],
  };
}

/**
 * The payload with Saturday and Sunday columns dropped, and every all-day
 * lane re-clipped to what survives.
 *
 * Column removal, not a window: the surviving days keep their relative
 * order, so they land on new, still-contiguous indices with no sparse
 * mapping for a caller to carry. A lane that crossed a weekend keeps both
 * edges, now adjacent columns — it draws as one unbroken bar over the
 * (invisible) gap, the same simplification the grid itself makes by never
 * drawing those columns. One that fell wholly on a dropped day is dropped
 * with it, uncounted: the day it lived on is not merely folded away, it
 * is gone, so there is nothing left for a "+N more" to point at.
 */
export function filterWeekends(week: WeekPayload): WeekPayload {
  const keep = week.days.map((d) => {
    const day = new Date(d.start_ms).getDay();
    return day !== 0 && day !== 6;
  });
  if (keep.every(Boolean)) return week;
  const newIndex = new Map<number, number>();
  const days: DayColumn[] = [];
  week.days.forEach((d, i) => {
    if (keep[i]) { newIndex.set(i, days.length); days.push(d); }
  });
  const all_day: Lane[] = [];
  for (const l of week.all_day) {
    let start_col = -1, end_col = -1;
    for (let i = l.start_col; i <= l.end_col; i++) {
      const mapped = newIndex.get(i);
      if (mapped === undefined) continue;
      if (start_col === -1) start_col = mapped;
      end_col = mapped;
    }
    if (start_col === -1) continue;
    all_day.push({ ...l, start_col, end_col });
  }
  return { ...week, days, all_day };
}

/**
 * `ms`, or — when it names a Saturday or Sunday in `days` — the next day in
 * `days` that isn't one. Unchanged when `ms` isn't in `days` at all (a stale
 * anchor is `visibleIndex`'s problem, not this one's) or when nothing after
 * it survives the weekend.
 */
export function skipWeekendStart(days: { start_ms: number }[], ms: number): number {
  const idx = days.findIndex((d) => d.start_ms === ms);
  if (idx < 0) return ms;
  for (let i = idx; i < days.length; i++) {
    const day = new Date(days[i].start_ms).getDay();
    if (day !== 0 && day !== 6) return days[i].start_ms;
  }
  return ms;
}

/** How many rows of chips the band shows before folding the rest behind
 *  "+N more". Four, which is what the backend used to pack (2026-08-31:
 *  "four is where a glance stops being a glance"); the difference is that
 *  the rest can now be expanded into rather than dropped. */
export const BAND_ROWS = 4;

/**
 * The all-day band's rows for a window, packed here rather than taken from
 * the payload.
 *
 * The backend packs lanes first-fit over whatever range it assembled, so
 * the same chips land in different rows depending on how much padding was
 * fetched around them — and the band re-drew its rows at the start of every
 * swipe, at the end, and again when the refetch landed (reported
 * 2026-09-03: "this zone flickers"). Packed from the window's own chips, in
 * an order that depends only on them — real start, real end, then index —
 * the rows are the same whatever range the payload covers.
 *
 * `extend` is the sliding state. At rest (`false`) the window's chips are
 * cut to the window with their continuing marks, exactly as before. While
 * sliding (`true`) they keep their real extents — so a chip runs on into
 * the padding as the track reveals it — and chips lying wholly in the
 * padding fill rows the window's chips left free, or are left out: a row
 * that exists only while sliding would change the band's height under the
 * gesture, which is the other half of the flicker. The window's chips get
 * identical rows in both modes: two contiguous spans that both touch the
 * window overlap in full if and only if they overlap inside it, and the
 * order is the same, so first-fit agrees.
 *
 * `maxRows` caps the rows drawn. A window chip that finds no room in them is
 * returned in `hidden` instead — the band's "+N more", which expands by
 * packing again with no cap (2026-09-04).
 */
export function packBandLanes(
  lanes: Lane[], from: number, count: number, extend: boolean, maxRows = Infinity,
): { lanes: Lane[]; hidden: number[] } {
  const last = from + count - 1;
  const inWindow = (l: Lane) => l.end_col >= from && l.start_col <= last;
  const byStart = (a: Lane, b: Lane) =>
    a.start_col - b.start_col || a.end_col - b.end_col || a.idx - b.idx;
  // Rows as the columns they hold, one span list per row.
  const rows: { start: number; end: number }[][] = [];
  const free = (row: { start: number; end: number }[], s: number, e: number) =>
    row.every((o) => e < o.start || s > o.end);
  const place = (s: number, e: number, grow: boolean): number => {
    for (let r = 0; r < rows.length; r++) {
      if (free(rows[r], s, e)) { rows[r].push({ start: s, end: e }); return r; }
    }
    if (!grow || rows.length >= maxRows) return -1;
    rows.push([{ start: s, end: e }]);
    return rows.length - 1;
  };
  const out: Lane[] = [];
  const hidden: number[] = [];
  for (const lane of lanes.filter(inWindow).sort(byStart)) {
    const start_col = extend ? lane.start_col : Math.max(lane.start_col, from) - from;
    const end_col = extend ? lane.end_col : Math.min(lane.end_col, last) - from;
    const row = place(start_col, end_col, true);
    // No room in the rows the band is drawing: it becomes part of "+N more"
    // rather than being drawn on a row that is not there.
    if (row < 0) { hidden.push(lane.idx); continue; }
    out.push({
      ...lane,
      lane: row,
      start_col,
      end_col,
      cont_left: lane.cont_left || (!extend && lane.start_col < from),
      cont_right: lane.cont_right || (!extend && lane.end_col > last),
    });
  }
  if (extend) {
    for (const lane of lanes.filter((l) => !inWindow(l)).sort(byStart)) {
      const row = place(lane.start_col, lane.end_col, false);
      if (row >= 0) out.push({ ...lane, lane: row });
    }
  }
  return { lanes: out, hidden };
}

/**
 * The whole days a pan has crossed, and what is left.
 *
 * `panDays` is the finger's travel in columns, positive when the content has
 * moved right (towards earlier days). Whole columns are handed to the app as
 * a shift of the window — the opposite sign, since content moving right
 * means the window moving left — and the fraction stays on the track.
 */
export function panCommit(panDays: number): { shift: number; rest: number } {
  const whole = Math.trunc(panDays);
  // `+ 0` turns `-0` into `0` — `drag.ts`'s `colsMoved` has the story.
  return { shift: -whole + 0, rest: panDays - whole };
}

/**
 * Whether `days` holds the window *and* `margin` days beyond it on each
 * side — the case in which a fetch can wait. A pan inside the padding needs
 * nothing fetched to draw, so the refetch that recentres the padding is
 * deferred until the gesture settles (App); fetching per day crossed
 * re-rendered three weeks of blocks under every wheel event, and that was
 * the lag (2026-09-03). At the padding's edge the fetch is immediate again:
 * one more column and there would be nothing to slide into.
 */
export function windowHeld(
  days: { start_ms: number }[], visibleStartMs: number, visible: number, margin: number,
): boolean {
  const i = visibleIndex(days, visibleStartMs);
  return i >= margin && i + visible + margin <= days.length;
}

/** A wheel event's contribution to the pan, for the velocity estimate. */
export type PanSample = { t: number; days: number };

/**
 * The pan's speed in columns per millisecond over the samples' span, or 0
 * for fewer than two of them. Only the last `WINDOW_MS` of samples count,
 * so a long slow drag that ends in a flick reports the flick.
 */
export function velocityOf(samples: PanSample[], windowMs = 100): number {
  if (samples.length < 2) return 0;
  const last = samples[samples.length - 1].t;
  const recent = samples.filter((s) => last - s.t <= windowMs);
  if (recent.length < 2) return 0;
  const span = recent[recent.length - 1].t - recent[0].t;
  if (span <= 0) return 0;
  // The first sample marks the start of the span; its own days were
  // travelled before it.
  const days = recent.slice(1).reduce((acc, s) => acc + s.days, 0);
  return days / span;
}

/** Momentum's reach: a flick at `v` columns/ms is projected `v * τ` columns
 *  on before it is rounded to a column. 400 puts a brisk touchpad flick two to
 *  three days on, a hard one a week. Linux has no inertia of its own on a
 *  wheel (libinput stops at lift), and macOS's own momentum events keep the
 *  lull from firing until they have decayed, by which time the projection
 *  adds nothing. */
export const FLING_TAU_MS = 400;
/** Below this, the fingers stopped rather than flicked: settle on the nearest
 *  column. */
export const FLING_MIN_V = 0.0015;
/** A finger on glass faster than this, in pixels per ms, is a flick for Day
 *  view's one-page-per-swipe rule (#127), however little it travelled. A
 *  deliberate slow swipe is about 0.4 px/ms; a finger resting and then
 *  lifting is well under 0.1. */
export const PAGE_FLICK_PX_PER_MS = 0.25;

/** A swipe released faster than this, in pixels of input per ms, is a
 *  strong one: the ‹ / › step. Measured on the omarchy box 2026-09-15 with a
 *  logger window: touchpad gentle 0.02–0.87 and strong 4.1–7.9 px/ms; finger
 *  gentle 0.44–1.19 and strong 2.96–7.2. Two sits in the middle of both gaps.
 *  It is a speed of the hand, not of the calendar. The first cut asked for
 *  momentum worth half a page in *days*, which in a half-width window (100px
 *  columns) a slow scroll already had, so slow scrolling paged whole weeks
 *  and the events seemed to vanish. */
export const STRONG_SWIPE_PX_PER_MS = 2;
/** ...and it must have travelled this far, so a quick twitch never pages. */
export const STRONG_SWIPE_MIN_PX = 40;

/**
 * Where a released pan comes to rest, in pan units (columns; positive is
 * content moved right, toward earlier days) counted from where the gesture
 * began. `x` is how far it has travelled, `v` its speed at lift in columns/ms,
 * and `page` the window's width in days: what the ‹ / › buttons step.
 * `strongV` and `strongX` are the strong-swipe thresholds already turned into
 * columns by the caller, which knows the column width and the gain.
 *
 * - **A strong swipe is the ‹ / › button.** It lands on the next page
 *   boundary in its own direction, pages counted from where the gesture
 *   began: exactly where the button would have taken the view from there,
 *   and a week slid off its alignment by an earlier swipe keeps its offset,
 *   as the button keeps it.
 * - **Anything gentler follows the hand:** the flick's momentum (`v * tau`)
 *   rounded to a day, never as far as a page from where the fingers left it.
 * - **A pager** (Day view under a finger, #127) pages on every flick, however
 *   slow, the way Android's calendar does, and never more than one page from
 *   where it began. Without a flick it goes to the nearer page.
 */
export function settleTarget(
  x: number, v: number,
  o: { page: number; pager: boolean; minV: number; strongV?: number; strongX?: number; tau?: number },
): number {
  const page = Math.max(1, o.page);
  const moving = Math.abs(v) >= o.minV;
  const strong = o.pager
    ? moving
    : o.strongV !== undefined && Math.abs(v) >= o.strongV && Math.abs(x) >= (o.strongX ?? 0);
  if (strong) {
    const u = x / page;
    // `1e-9` so a page boundary already exactly reached counts as reached,
    // not as one still to go: floor(1) + 1 would be 2.
    let next = v > 0 ? Math.floor(u + 1e-9) + 1 : Math.ceil(u - 1e-9) - 1;
    if (o.pager) next = Math.max(-1, Math.min(1, next));
    return next * page + 0;
  }
  const near = Math.round(x);
  if (o.pager) return Math.max(-1, Math.min(1, near)) + 0;
  const reach = moving ? v * (o.tau ?? FLING_TAU_MS) : 0;
  const lim = Math.max(1, page - 1);
  return Math.max(near - lim, Math.min(near + lim, Math.round(x + reach))) + 0;
}

/** The settle's stiffness from a standstill, per ms: at rest in about 350ms. */
export const SPRING_OMEGA = 0.024;

/**
 * One critically damped spring from `from` to `target`, carrying the speed
 * the fingers left with. This is the whole settle: no separate glide and
 * snap, so the motion never pauses between two curves (the old pair took up
 * to 1.9 s and stalled between them).
 *
 * Two rules keep it from ever looking like rubber. A speed pointing away
 * from the target is dropped rather than carried, so it never goes out and
 * comes back. A speed toward it stiffens the spring (`omega >= |v / a|`), so
 * a hard flick lands sooner instead of shooting past: that is exactly the
 * condition under which `a + b t` keeps its sign, and so never crosses the
 * target.
 */
export function springPlan(from: number, target: number, v: number) {
  const a = from - target;
  const v0 = v * (target - from) > 0 ? v : 0;
  const omega = a === 0 ? SPRING_OMEGA : Math.max(SPRING_OMEGA, Math.abs(v0 / a));
  return { a, v0, omega, target };
}

/** The spring `t` ms in: position, speed, and whether it has come to rest. */
export function springAt(p: ReturnType<typeof springPlan>, t: number) {
  const b = p.v0 + p.omega * p.a;
  const e = Math.exp(-p.omega * t);
  const off = (p.a + b * t) * e;
  const speed = (b - p.omega * (p.a + b * t)) * e;
  const done = (Math.abs(off) < 1e-3 && Math.abs(speed) < 1e-4) || t > 2000;
  return { x: done ? p.target : p.target + off, v: done ? 0 : speed, done };
}

/** How long after a finger lifts WebKitGTK's own wheel event for that lift
 *  arrives (measured 0–11 ms on 2.52.6), with room to spare. */
export const TOUCH_LIFT_WHEEL_MS = 60;
