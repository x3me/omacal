import { test, expect } from '@playwright/test';
import type { WeekPayload } from '../src/lib/api';
import {
  FLING_TAU_MS, flingProgress, flingTravel, packBandLanes, padFor, panCommit, sliceWeek, snapPlan,
  velocityOf, visibleIndex, windowHeld,
} from '../src/lib/weekwindow';

const DAY = 86_400_000;
const days = (n: number, from = 0) =>
  Array.from({ length: n }, (_, i) => ({ start_ms: (from + i) * DAY, end_ms: (from + i + 1) * DAY, events: [], placed: [] }));
const lane = (start_col: number, end_col: number, idx = 0) =>
  ({ idx, lane: 0, start_col, end_col, cont_left: false, cont_right: false });

test.describe('the window on a padded week', () => {
  test('padding is the window\'s own width, and never less than three days', () => {
    expect(padFor(7)).toBe(7);
    expect(padFor(5)).toBe(5);
    expect(padFor(3)).toBe(3);
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

  test('the fingers lifting settle on the nearest column, and one more past half', () => {
    expect(snapPlan(0.3)).toEqual({ shift: 0, from: expect.closeTo(0.3, 9) });
    expect(snapPlan(0.6)).toEqual({ shift: -1, from: expect.closeTo(-0.4, 9) });
    expect(snapPlan(-0.6)).toEqual({ shift: 1, from: expect.closeTo(0.4, 9) });
    expect(snapPlan(0)).toEqual({ shift: 0, from: 0 });
  });
});

test.describe('the pan\'s feel: what waits, and what flies', () => {
  test('a window inside the padding, with a margin, holds; at the edge it does not', () => {
    // 21 days, window of 7 at index 7: seven days of padding each side.
    expect(windowHeld(days(21), 7 * DAY, 7, 2)).toBe(true);
    // At index 2 the margin is exactly met; at 1 it is not.
    expect(windowHeld(days(21), 2 * DAY, 7, 2)).toBe(true);
    expect(windowHeld(days(21), 1 * DAY, 7, 2)).toBe(false);
    // At the far edge likewise: 12 + 7 + 2 = 21 holds, 13 does not.
    expect(windowHeld(days(21), 12 * DAY, 7, 2)).toBe(true);
    expect(windowHeld(days(21), 13 * DAY, 7, 2)).toBe(false);
    // Not in the payload at all: fetch now.
    expect(windowHeld(days(21), 40 * DAY, 7, 2)).toBe(false);
    // An unpadded payload never holds.
    expect(windowHeld(days(7), 0, 7, 2)).toBe(false);
  });

  test('the speed at lift is read off the last hundred milliseconds', () => {
    // A slow drag, then a flick: 0.1 columns over 100ms each for a while,
    // then 0.5 columns in 40ms.
    const samples = [
      { t: 0, days: 0 }, { t: 100, days: 0.1 }, { t: 200, days: 0.1 }, { t: 300, days: 0.1 },
      { t: 320, days: 0.25 }, { t: 340, days: 0.25 },
    ];
    expect(velocityOf(samples)).toBeCloseTo(0.5 / 40, 6);
    expect(velocityOf([{ t: 0, days: 1 }])).toBe(0);
    expect(velocityOf([])).toBe(0);
  });

  test('a stop settles at once, a flick travels with its speed, and never past the padding', () => {
    expect(flingTravel(0.001)).toBe(0);
    expect(flingTravel(0.005)).toBeCloseTo(2, 9);   // 5 col/s × 0.4s
    expect(flingTravel(-0.005)).toBeCloseTo(-2, 9);
    expect(flingTravel(0.1)).toBe(6);
    expect(flingTravel(-0.1)).toBe(-6);
    // The glide's shape: nothing at lift, all of it eventually, 63% at τ.
    expect(flingProgress(0)).toBe(0);
    expect(flingProgress(FLING_TAU_MS)).toBeCloseTo(1 - Math.exp(-1), 9);
    expect(flingProgress(FLING_TAU_MS * 10)).toBeCloseTo(1, 4);
  });
});
