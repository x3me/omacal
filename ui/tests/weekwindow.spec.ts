import { test, expect } from '@playwright/test';
import type { WeekPayload } from '../src/lib/api';
import {
  FLING_MIN_V, FLING_TAU_MS, filterWeekends, packBandLanes, padFor, panCommit, settleTarget, skipWeekendStart,
  sliceWeek, springAt, springPlan, velocityOf, visibleIndex, windowHeld,
} from '../src/lib/weekwindow';

const DAY = 86_400_000;
const days = (n: number, from = 0) =>
  Array.from({ length: n }, (_, i) => ({ start_ms: (from + i) * DAY, end_ms: (from + i + 1) * DAY, events: [], placed: [] }));
const lane = (start_col: number, end_col: number, idx = 0) =>
  ({ idx, lane: 0, start_col, end_col, cont_left: false, cont_right: false });

test.describe('the window on a padded week', () => {
  test('padding is a page and two more, and never less than three days', () => {
    // A whole page (the ‹ / › step a strong swipe takes) plus `windowHeld`'s
    // two-day margin, so the landing never triggers the refetch mid-motion.
    expect(padFor(7)).toBe(9);
    expect(padFor(5)).toBe(7);
    expect(padFor(3)).toBe(5);
    expect(padFor(1)).toBe(3);
  });

  test('the window is found by its first day, and a payload without it says so', () => {
    expect(visibleIndex(days(21), 7 * DAY)).toBe(7);
    expect(visibleIndex(days(21), 0)).toBe(0);
    // A payload from before the window jumped, or an unpadded stub: the
    // callers show it whole rather than a window's worth of its start.
    expect(visibleIndex(days(7, 3), 50 * DAY)).toBe(-1);
  });

  test('slicing keeps the days in the window and cuts the lanes to it', () => {
    const week: WeekPayload = {
      days: days(21),
      all_day: [
        lane(5, 9, 0),   // starts in the padding, runs into the window
        lane(8, 10, 1),  // inside
        lane(12, 16, 2), // runs out the far side
        lane(0, 3, 3),   // padding only
        lane(2, 20, 4),  // straddles the whole window
      ],
      all_day_events: [],
      overflow: [4],
    };
    const w = sliceWeek(week, 7, 7);
    expect(w.days.map((d) => d.start_ms / DAY)).toEqual([7, 8, 9, 10, 11, 12, 13]);
    // Rows are packed here, by real start: the span straddling everything
    // starts first and takes row 0; the one from the padding row 1; the
    // inside one overlaps both and takes row 2; the one running out the far
    // side fits back into row 1 beside the padding one.
    expect(w.all_day).toEqual([
      { ...lane(0, 6, 4), lane: 0, cont_left: true, cont_right: true },
      { ...lane(0, 2, 0), lane: 1, cont_left: true },
      { ...lane(1, 3, 1), lane: 2 },
      { ...lane(5, 6, 2), lane: 1, cont_right: true },
    ]);
    // Left whole: lanes index into the events. The overflow is not passed
    // through any more — it is the window's own hidden chips plus whatever
    // the backend could not position — and here nothing is hidden.
    expect(w.all_day_events).toBe(week.all_day_events);
    expect(w.overflow).toEqual(week.overflow);
  });

  test('the band\'s rows are the window\'s, at rest and while sliding, and never grow for the padding', () => {
    // 21 days, window at 7..13. A: from the padding into the window. B: in
    // the window, overlapping A. Three wholly in the padding: C, where row 0
    // is free; D, overlapping C and A, so it takes row 1; E, overlapping C,
    // A and D — no free row, so it is left out rather than adding a row the
    // window does not have.
    const lanes = [lane(3, 9, 0), lane(8, 10, 1), lane(0, 2, 2), lane(1, 4, 3), lane(2, 5, 4)];
    const rest = packBandLanes(lanes, 7, 7, false).lanes;
    expect(rest).toEqual([
      { ...lane(0, 2, 0), lane: 0, cont_left: true },
      { ...lane(1, 3, 1), lane: 1 },
    ]);
    const sliding = packBandLanes(lanes, 7, 7, true).lanes;
    expect(sliding).toEqual([
      { ...lane(3, 9, 0), lane: 0 },
      { ...lane(8, 10, 1), lane: 1 },
      { ...lane(0, 2, 2), lane: 0 },
      { ...lane(1, 4, 3), lane: 1 },
    ]);
    // The reported flicker: the same two chips got rows 2 and 3 from a
    // payload packed over a wider range. Rows are the window's, whatever
    // the payload's own numbering says.
    const renumbered = lanes.map((l, i) => ({ ...l, lane: 3 - i }));
    expect(packBandLanes(renumbered, 7, 7, false).lanes.map((l) => l.lane)).toEqual([0, 1]);
  });

  test('a row cap hides the chips that do not fit, and only the window\'s (#email 2026-09-04)', () => {
    // Five spans across the whole window, so each needs its own row.
    const five = [0, 1, 2, 3, 4].map((i) => lane(7, 13, i));
    // Plus two lying wholly in the padding, which are nobody's "+N more":
    // the count is about the week on screen.
    const padding = [lane(0, 6, 5), lane(0, 6, 6)];
    const capped = packBandLanes([...five, ...padding], 7, 7, false, 4);
    expect(capped.lanes.map((l) => l.idx)).toEqual([0, 1, 2, 3]);
    expect(capped.hidden).toEqual([4]);

    // Expanded: no cap, nothing hidden, and the rows keep the same order.
    const all = packBandLanes([...five, ...padding], 7, 7, false, Infinity);
    expect(all.lanes.map((l) => l.lane)).toEqual([0, 1, 2, 3, 4]);
    expect(all.hidden).toEqual([]);

    // The cap holds the band's height while sliding too: the padding chips
    // are drawn — they are what the track slides into — but only in rows
    // the window's own chips already opened, never in a fifth.
    const sliding = packBandLanes([...five, ...padding], 7, 7, true, 4);
    expect(Math.max(...sliding.lanes.map((l) => l.lane))).toBe(3);
    expect(sliding.lanes.filter((l) => l.idx > 4).map((l) => l.lane)).toEqual([0, 1]);
    expect(sliding.hidden).toEqual([4]);
  });

  test('the window\'s hidden chips are what the band counts, not the payload\'s', () => {
    // The count was taken from the payload's own `overflow`, which since the
    // padded window covers three weeks — so a week showing everything could
    // still say "+N more" about days nobody was looking at.
    const week: WeekPayload = {
      days: days(21),
      all_day: [0, 1, 2, 3, 4].map((i) => lane(7, 13, i)),
      all_day_events: [],
      overflow: [],
    };
    expect(sliceWeek(week, 7, 7, 4).overflow).toEqual([4]);
    expect(sliceWeek(week, 7, 7, Infinity).overflow).toEqual([]);
    // Whatever the backend itself could not position is added, never lost.
    expect(sliceWeek({ ...week, overflow: [99] }, 7, 7, 4).overflow).toEqual([4, 99]);
  });

  test('a lane already marked continuing stays so after the cut', () => {
    const week: WeekPayload = {
      days: days(21), all_day: [{ ...lane(7, 9), cont_left: true }], all_day_events: [], overflow: [],
    };
    expect(sliceWeek(week, 7, 7).all_day).toEqual([{ ...lane(0, 2), cont_left: true, lane: 0 }]);
  });

  test('the whole payload slices to itself', () => {
    const week: WeekPayload = { days: days(7), all_day: [lane(1, 2)], all_day_events: [], overflow: [] };
    expect(sliceWeek(week, 0, 7)).toBe(week);
  });

  test('whole columns crossed are handed up against the travel, the fraction stays', () => {
    // Content moved 1.3 columns right: the window moves a day earlier.
    expect(panCommit(1.3)).toEqual({ shift: -1, rest: expect.closeTo(0.3, 9) });
    // Content moved 2.5 columns left: two days later.
    expect(panCommit(-2.5)).toEqual({ shift: 2, rest: -0.5 });
    expect(panCommit(0.9)).toEqual({ shift: 0, rest: 0.9 });
  });

  test('a pan that stops settles on the nearest column, one more past half', () => {
    const o = { page: 7, pager: false, minV: FLING_MIN_V };
    expect(settleTarget(0.3, 0, o)).toBe(0);
    expect(settleTarget(0.6, 0, o)).toBe(1);
    expect(settleTarget(-0.6, 0, o)).toBe(-1);
    expect(settleTarget(2.4, 0, o)).toBe(2);
    expect(settleTarget(0, 0, o)).toBe(0);
    // A slow drag past a whole week is not pulled back when it stops.
    expect(settleTarget(8.2, 0, o)).toBe(8);
  });

  test('a gentle flick glides a few days, never as far as a page', () => {
    const o = { page: 7, pager: false, minV: FLING_MIN_V };
    // Below the flick speed: the nearest column, as if it had stopped.
    expect(settleTarget(0.2, 0.001, o)).toBe(0);
    // 5 columns/s projected for 0.4 s is two more: under half a page.
    expect(settleTarget(0.2, 0.005, o)).toBe(2);
    expect(settleTarget(-0.2, -0.005, o)).toBe(-2);
    expect(settleTarget(0.2, 0.008, o)).toBe(3);
    expect(FLING_TAU_MS).toBe(400);
  });

  test('a strong swipe is the ‹ / › button: exactly one page, from where the gesture began', () => {
    // Thresholds as the grid passes them, already in columns.
    const week = { page: 7, pager: false, minV: FLING_MIN_V, strongV: 0.02, strongX: 0.4 };
    expect(settleTarget(0.5, 0.03, week)).toBe(7);
    expect(settleTarget(-0.5, -0.03, week)).toBe(-7);
    expect(settleTarget(0.5, 1, week)).toBe(7);
    // Pages count from the gesture's start, so a swipe already two days in
    // still lands on the week the button would have, not two days past it.
    expect(settleTarget(2, 0.05, week)).toBe(7);
    // Flicked back from two days in: the page it began on, the next boundary
    // that way.
    expect(settleTarget(2, -0.05, week)).toBe(0);
    // Dragged past a whole page and flicked on: the next boundary beyond.
    expect(settleTarget(8, 0.05, week)).toBe(14);
    // A rolling week of three pages by three, as its buttons step.
    expect(settleTarget(0.5, 0.03, { ...week, page: 3 })).toBe(3);
    // Day view on a touchpad: the button is a day, and so is a strong flick.
    const day = { page: 1, pager: false, minV: FLING_MIN_V, strongV: 0.003, strongX: 0.06 };
    expect(settleTarget(0.3, 0.03, day)).toBe(1);
    expect(settleTarget(-0.3, -0.03, day)).toBe(-1);
  });

  test('strong is a speed of the hand: a slow scroll never pages, however narrow the columns', () => {
    // The regression Plamen caught in the first cut: in a half-width window a
    // slow touchpad scroll has momentum worth four 100px columns, which read
    // as "half a week" and paged. Below the speed it glides where the
    // momentum takes it, as before.
    const week = { page: 7, pager: false, minV: FLING_MIN_V, strongV: 0.04, strongX: 0.8 };
    expect(settleTarget(2, 0.01, week)).toBe(6);
    expect(settleTarget(2, 0.015, week)).toBe(8);
    // Fast but barely moved: a twitch, not a page.
    expect(settleTarget(0.3, 0.2, week)).not.toBe(7);
    // Without thresholds at all, nothing is ever strong.
    expect(settleTarget(0.4, 1, { page: 7, pager: false, minV: FLING_MIN_V })).toBe(6);
  });

  test('Day view under a finger moves one page per swipe, the way the flick points (#127)', () => {
    const o = { page: 1, pager: true, minV: 0.0002 };
    // A flick from a little way in goes on to the next page...
    expect(settleTarget(0.2, 0.001, o)).toBe(1);
    expect(settleTarget(-0.2, -0.001, o)).toBe(-1);
    // ...even from barely started, and back where it came from if flicked back.
    expect(settleTarget(0, 0.001, o)).toBe(1);
    expect(settleTarget(0.4, -0.001, o)).toBe(0);
    // Never more than one page, however hard or however far.
    expect(settleTarget(0.2, 0.5, o)).toBe(1);
    expect(settleTarget(1.6, 0.01, o)).toBe(1);
    expect(settleTarget(1, 0.01, o)).toBe(1);
    // No flick: the nearer page.
    expect(settleTarget(0.4, 0, o)).toBe(0);
    expect(settleTarget(0.7, 0.0001, o)).toBe(1);
    expect(settleTarget(-0.7, 0, o)).toBe(-1);
  });

  test('the settle is one spring: it keeps the speed it was given, lands exactly, and never overshoots', () => {
    const run = (from: number, target: number, v: number) => {
      const p = springPlan(from, target, v);
      const xs: number[] = [];
      let last = springAt(p, 0);
      for (let t = 0; t <= 2100 && !last.done; t += 4) { last = springAt(p, t); xs.push(last.x); }
      return { p, xs, last };
    };
    // From a standstill it rests on the target within half a second.
    const still = run(0.4, 0, 0);
    expect(still.last.done).toBe(true);
    expect(still.last.x).toBe(0);
    expect(still.xs.length * 4).toBeLessThan(500);
    // A speed toward the target is carried: the first instant moves with it.
    const flick = springPlan(0.2, 1, 0.01);
    expect(springAt(flick, 0).v).toBeCloseTo(0.01, 9);
    // A hard flick stiffens the spring rather than shooting past.
    const hard = run(0.2, 1, 0.05);
    expect(Math.max(...hard.xs)).toBeLessThanOrEqual(1 + 1e-9);
    expect(hard.last.x).toBe(1);
    // Every case, sampled: never beyond the target on the far side.
    for (const [from, target, v] of [[0.2, 1, 0.003], [-0.3, -1, -0.02], [0.9, 1, 0.1], [1.5, 1, -0.004]]) {
      const r = run(from, target, v);
      const beyond = r.xs.filter((x) => (target - from) * (x - target) > 1e-9);
      expect(beyond).toEqual([]);
      expect(r.last.x).toBe(target);
    }
    // A speed pointing away is dropped, so it never goes out and comes back.
    const away = springPlan(0.3, 0, 0.01);
    expect(away.v0).toBe(0);
    expect(springAt(away, 0).v).toBeCloseTo(0, 9);
  });
});

test.describe('hiding weekends', () => {
  // 2026-09-21 is a Monday, built through local Y/M/D fields rather than an
  // epoch-day multiple — `weekstart.spec.ts`'s reason: the weekday a `Date`
  // reports depends on the reader's zone, and constructing from local fields
  // is the one way to make it agree with itself wherever the suite runs.
  const local = (d: number) => new Date(2026, 8, d).getTime();
  const tenDays = Array.from({ length: 10 }, (_, i) =>
    ({ start_ms: local(21 + i), end_ms: local(22 + i), events: [], placed: [] }));
  // Mon 21, Tue 22, Wed 23, Thu 24, Fri 25, Sat 26, Sun 27, Mon 28, Tue 29, Wed 30.

  test('Saturday and Sunday columns are dropped, and the rest stay in order', () => {
    const week: WeekPayload = { days: tenDays, all_day: [], all_day_events: [], overflow: [] };
    const w = filterWeekends(week);
    expect(w.days.map((d) => d.start_ms)).toEqual(
      [21, 22, 23, 24, 25, 28, 29, 30].map((d) => local(d)),
    );
  });

  test('a payload with no weekend in it is returned as is', () => {
    const week: WeekPayload = { days: tenDays.slice(0, 5), all_day: [], all_day_events: [], overflow: [] };
    expect(filterWeekends(week)).toBe(week);
  });

  test('a lane crossing the weekend keeps its edges, now adjacent columns', () => {
    // Thu(3)..Sun(6): after Sat/Sun drop, Thu and Fri survive as the new 3
    // and 4 (Mon..Fri keep their old indices; nothing before them moved).
    const week: WeekPayload = {
      days: tenDays, all_day: [lane(3, 6, 0)], all_day_events: [], overflow: [],
    };
    expect(filterWeekends(week).all_day).toEqual([lane(3, 4, 0)]);
  });

  test('a lane that falls entirely on the weekend is dropped', () => {
    const week: WeekPayload = {
      days: tenDays, all_day: [lane(5, 6, 0), lane(2, 3, 1)], all_day_events: [], overflow: [],
    };
    expect(filterWeekends(week).all_day).toEqual([lane(2, 3, 1)]);
  });

  test('an anchor on the weekend advances to the Monday after it', () => {
    expect(skipWeekendStart(tenDays, local(26))).toBe(local(28)); // Saturday
    expect(skipWeekendStart(tenDays, local(27))).toBe(local(28)); // Sunday
  });

  test('a weekday anchor, or one the payload does not hold, is unchanged', () => {
    expect(skipWeekendStart(tenDays, local(24))).toBe(local(24));
    expect(skipWeekendStart(tenDays, local(50))).toBe(local(50));
  });

  test('a weekend anchor with no weekday left in the payload is unchanged', () => {
    const friSat = tenDays.slice(4, 6); // Fri 25, Sat 26
    expect(skipWeekendStart(friSat, local(26))).toBe(local(26));
  });
});
