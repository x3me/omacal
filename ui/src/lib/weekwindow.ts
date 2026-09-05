// The window on a padded week payload, and the arithmetic of sliding it.
//
// Since 2026-09-03 the day grids fetch more days than they show: `padFor`
// either side of the window, in one payload, so a sideways swipe has real
// columns to reveal before the fetch for the new window lands (the old
// pan jumped a day per 90px of wheel and refetched on every jump — content
// arrived in lurches, and nothing moved under the finger). Everything that
// is about *what is on screen* reads the window; only the grid's track ever
// draws the padding. Pure, so `weekwindow.spec.ts` pins each rule.

import type { Lane, WeekPayload } from './api';

/** Days of padding either side of a window of `visible` days: the window's
 *  own width, so a swipe can travel a whole window before it outruns the
 *  payload, and never fewer than three, so Day view has somewhere to go. */
export const padFor = (visible: number) => Math.max(3, visible);

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
 * Where a pan settles when the fingers lift: the nearest whole column. More
 * than half a column over commits one more day (`shift`), and the track
 * animates from what is then left (`from`) back to zero.
 */
export function snapPlan(panDays: number): { shift: number; from: number } {
  const nearest = Math.round(panDays);
  return { shift: -nearest + 0, from: panDays - nearest };
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

/** Momentum's time constant: after the fingers lift, the speed decays as
 *  e^(-t/τ), so a flick at `v` columns/ms travels `v * τ` more columns. 400
 *  puts a brisk flick two to three days on, a hard one a week. Linux has no
 *  inertia of its own on a wheel — libinput stops at lift — and macOS's own
 *  momentum events keep our lull from firing until they have decayed, by
 *  which time the speed is low and this adds nothing. */
export const FLING_TAU_MS = 400;
/** Below this, the fingers stopped rather than flicked: settle at once. */
export const FLING_MIN_V = 0.0015;
/** Never further than the padding on one side minus a column: momentum
 *  that outran the payload would slide into nothing. */
export const FLING_MAX_DAYS = 6;

/** How far a fling at `v` columns/ms will travel, capped. Signed. */
export function flingTravel(v: number, tau = FLING_TAU_MS, cap = FLING_MAX_DAYS): number {
  if (Math.abs(v) < FLING_MIN_V) return 0;
  const d = v * tau;
  return Math.max(-cap, Math.min(cap, d));
}

/** Where a fling stands `t` ms after lift, as a fraction of its travel:
 *  1 - e^(-t/τ), which is the integral of the decaying speed. */
export const flingProgress = (t: number, tau = FLING_TAU_MS) => 1 - Math.exp(-t / tau);
