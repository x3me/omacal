import { test, expect } from '@playwright/test';

/**
 * The drag geometry, exercised in the page.
 *
 * In the page rather than in Node because every one of these answers depends on
 * the browser's zone: the vertical axis is a fraction of a **day's own span**
 * (a DST day is 23 or 25 hours), snapping is in local wall-clock minutes, and
 * moving a day is civil-date arithmetic. Playwright's `timezoneId` reaches the
 * browser context and not Node, so a Node-side spec could not put any of that
 * under test — it would pass in one zone and say nothing about the rest.
 *
 * No DOM, no pointer events, no component: this module is the arithmetic on its
 * own, which is the whole point of Task 2. What needs a browser is the gesture,
 * and that is Task 3.
 */
const PURE = '/tests/harness/index.html?c=eventform&f=none';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const SNAP = 15 * MIN;

/** 2026-08-10T09:00:00Z, a Monday well clear of any transition. */
const T0900Z = Date.UTC(2026, 7, 10, 9, 0);

/** The origin span every case starts from: 09:00–09:30. */
const ORIGIN = { startMs: T0900Z, endMs: T0900Z + 30 * MIN };

type Span = { startMs: number; endMs: number };

const drag = (page: import('@playwright/test').Page) =>
  page.evaluate(() => Object.keys((window as any).__drag ?? {}));

/** `spanForMove` in the page, with the fixture's own constants. */
const move = (
  page: import('@playwright/test').Page,
  args: { origin?: Span; dyFrac: number; dayMs?: number; dayCols?: number; dxCols: number },
): Promise<Span> =>
  page.evaluate((a) => {
    const d = (window as any).__drag;
    return d.spanForMove(a.origin, a.dyFrac, a.dayMs, a.dayCols, a.dxCols, a.snap);
  }, {
    origin: args.origin ?? ORIGIN,
    dyFrac: args.dyFrac,
    dayMs: args.dayMs ?? DAY,
    dayCols: args.dayCols ?? 7,
    dxCols: args.dxCols,
    snap: SNAP,
  });

/** `spanForResize` in the page. */
const resize = (
  page: import('@playwright/test').Page,
  args: { origin?: Span; edge: 'start' | 'end'; dyFrac: number; dayMs?: number },
): Promise<Span> =>
  page.evaluate((a) => {
    const d = (window as any).__drag;
    return d.spanForResize(a.origin, a.edge, a.dyFrac, a.dayMs, a.snap);
  }, {
    origin: args.origin ?? ORIGIN,
    edge: args.edge,
    dyFrac: args.dyFrac,
    dayMs: args.dayMs ?? DAY,
    snap: SNAP,
  });

test.describe('snapping', () => {
  /**
   * The table. `within` is minutes past the hour going in, `expect` is minutes
   * past the hour coming out, on a 15-minute snap.
   *
   * **The midpoint is a decision, not an accident.** 7.5 minutes is exactly
   * between two slots and rounds **up**, which is `Math.round`'s own rule and
   * the least surprising one to describe: "nearest, and ties go forward". The
   * row below it — 7.49 — is what stops that being satisfied by a function that
   * always rounds up.
   */
  const cases: Array<{ within: number; expect: number; why: string }> = [
    { within: 0, expect: 0, why: 'already on a slot: unmoved' },
    { within: 15, expect: 15, why: 'already on a slot, mid-hour' },
    { within: 1, expect: 0, why: 'just past a slot rounds back' },
    { within: 7.49, expect: 0, why: 'below the midpoint rounds down' },
    { within: 7.5, expect: 15, why: 'exactly the midpoint rounds up' },
    { within: 7.51, expect: 15, why: 'above the midpoint rounds up' },
    { within: 14, expect: 15, why: 'just short of a slot rounds on' },
    { within: 52.5, expect: 60, why: 'the midpoint of the last slot rolls the hour' },
    { within: 59, expect: 60, why: 'nearly the hour rolls the hour' },
  ];

  for (const c of cases) {
    test(`${c.within} minutes past the hour snaps to ${c.expect} — ${c.why}`, async ({ page }) => {
      await page.goto(PURE);
      const got = await page.evaluate(
        (a) => (window as any).__drag.snapMs(a.ms, a.snap),
        { ms: T0900Z + c.within * MIN, snap: SNAP },
      );
      expect(got).toBe(T0900Z + c.expect * MIN);
    });
  }
});

test.describe('moving', () => {
  /**
   * Time only: the column does not change, so nothing about the day may.
   * Separate from the day-only case below **on purpose** — a fixture that moved
   * both axes could not say which one was wrong.
   */
  test('a move down the column changes the time and not the day', async ({ page }) => {
    await page.goto(PURE);
    // One hour down a 24-hour day.
    const got = await move(page, { dyFrac: 1 / 24, dxCols: 0 });

    expect(got.startMs).toBe(T0900Z + HOUR);
    expect(got.endMs).toBe(T0900Z + HOUR + 30 * MIN);
  });

  test('a move across columns changes the day and not the time', async ({ page }) => {
    await page.goto(PURE);
    // Two columns right, no vertical movement at all.
    const got = await move(page, { dyFrac: 0, dxCols: 2 });

    expect(got.startMs).toBe(T0900Z + 2 * DAY);
    expect(got.endMs).toBe(T0900Z + 2 * DAY + 30 * MIN);
  });

  /**
   * **Its own assertion, and not a corollary of where the block landed.** A
   * move implemented as "set the start from the pointer, then recompute the end
   * from the pointer too" passes a landing test and silently resizes the event
   * by whatever the two roundings disagree about. Several offsets, including
   * ones that do not land on a slot, so the rounding is exercised rather than
   * avoided.
   */
  test('a move never changes the duration', async ({ page }) => {
    await page.goto(PURE);
    const origin = { startMs: T0900Z, endMs: T0900Z + 95 * MIN }; // deliberately not a slot multiple
    const want = origin.endMs - origin.startMs;

    for (const dyFrac of [0, 1 / 24, 0.013, -0.2, 0.5, 0.37]) {
      for (const dxCols of [0, 1, -3]) {
        const got = await move(page, { origin, dyFrac, dxCols });
        expect(got.endMs - got.startMs, `dyFrac ${dyFrac}, dxCols ${dxCols}`).toBe(want);
      }
    }
  });

  /**
   * §4: putting an event back where it came from must be free.
   *
   * **The origin is deliberately not on a slot.** An event at 09:07 — created
   * elsewhere, or imported — grabbed and put back must come out at 09:07. Run
   * through the ordinary path it would be snapped to 09:15, which is a move
   * nobody asked for, on an event nobody dragged anywhere. A fixture starting
   * at 09:00 cannot witness that: snapping an instant already on a boundary
   * returns it, so both paths agree and the rule looks tested when it is not.
   */
  test('a move of nothing returns exactly what it was given', async ({ page }) => {
    await page.goto(PURE);
    const unaligned = { startMs: T0900Z + 7 * MIN, endMs: T0900Z + 37 * MIN };

    const got = await move(page, { origin: unaligned, dyFrac: 0, dxCols: 0 });

    expect(got).toEqual(unaligned);
  });

  /**
   * And the converse, so the rule above is not satisfied by a function that
   * never snaps at all: an actual move *from* that unaligned origin lands on a
   * slot. 09:07 dragged an hour down is 10:07, which snaps back to 10:00.
   */
  test('a move from an unaligned origin lands on a slot', async ({ page }) => {
    await page.goto(PURE);
    const unaligned = { startMs: T0900Z + 7 * MIN, endMs: T0900Z + 37 * MIN };

    const got = await move(page, { origin: unaligned, dyFrac: 1 / 24, dxCols: 0 });

    expect(got.startMs).toBe(T0900Z + HOUR);
    expect(got.endMs - got.startMs, 'and the duration is still its own').toBe(30 * MIN);
  });
});

test.describe('moving across a daylight-saving boundary', () => {
  // Sofia's clocks go forward at 03:00 on 29 Mar 2026. Two things here are not
  // 24 hours: the day the event starts on, and the gap between the same wall
  // time on either side of it.
  test.use({ timezoneId: 'Europe/Sofia' });

  /**
   * A day is not `+ 86400000`.
   *
   * Dragging a 09:00 meeting from Saturday to Sunday across the spring-forward
   * must leave it at **09:00 on Sunday**, which is 23 hours later, not 24. The
   * naive arithmetic lands it at 10:00 — the same defect this project has now
   * closed twice elsewhere, and the reason the day axis is civil rather than
   * additive.
   */
  test('a day is a civil day, not twenty-four hours', async ({ page }) => {
    await page.goto(PURE);
    const sat0900 = Date.parse('2026-03-28T09:00:00+02:00');
    const sun0900 = Date.parse('2026-03-29T09:00:00+03:00');

    // The premise, asserted rather than trusted: these two are 23 hours apart,
    // so a fixture that stopped straddling the transition fails here first.
    expect(sun0900 - sat0900).toBe(23 * HOUR);

    const got = await move(page, {
      origin: { startMs: sat0900, endMs: sat0900 + 30 * MIN },
      dyFrac: 0,
      dxCols: 1,
    });

    expect(got.startMs).toBe(sun0900);
    expect(got.endMs).toBe(sun0900 + 30 * MIN);
  });

  /**
   * And the vertical axis is a fraction of **that day's own span**, which is
   * what `WeekGrid`'s `hourFrac` and `slotAt` already do. Half way down a
   * 23-hour Sunday is 11.5 hours, not 12.
   */
  test('the vertical axis is a fraction of the day it is dragged in', async ({ page }) => {
    await page.goto(PURE);
    const sunStart = Date.parse('2026-03-29T00:00:00+02:00');
    const dayMs = Date.parse('2026-03-30T00:00:00+03:00') - sunStart;
    expect(dayMs, 'fixture check: a spring-forward Sunday is 23 hours').toBe(23 * HOUR);

    // The wall clock is read **in the page**, never here: `timezoneId` reaches
    // the browser context and not Node, so `new Date(ms).getHours()` in this
    // process answers in the host's zone and would assert nothing about Sofia.
    // The harness says the same thing about `__eventform`; it caught this spec
    // out first.
    const got = await page.evaluate((a) => {
      const d = (window as any).__drag;
      const span = d.spanForMove(a.origin, 0.5, a.dayMs, 7, 0, a.snap);
      const at = new Date(span.startMs);
      return { ...span, localHour: at.getHours(), localMinute: at.getMinutes() };
    }, {
      origin: { startMs: sunStart, endMs: sunStart + 30 * MIN },
      dayMs,
      snap: SNAP,
    });

    // 11.5 hours past local midnight, snapped — and 11.5h after 00:00 on a day
    // that loses an hour at 03:00 is 12:30 on the wall clock.
    expect(got.startMs).toBe(sunStart + 11.5 * HOUR);
    expect(got.localHour).toBe(12);
    expect(got.localMinute).toBe(30);
  });
});

test.describe('resizing', () => {
  test('dragging the start edge moves the start and leaves the end', async ({ page }) => {
    await page.goto(PURE);
    // Half an hour up.
    const got = await resize(page, { edge: 'start', dyFrac: -0.5 / 24 });

    expect(got.startMs).toBe(T0900Z - 30 * MIN);
    expect(got.endMs, 'the other edge stays put').toBe(ORIGIN.endMs);
  });

  test('dragging the end edge moves the end and leaves the start', async ({ page }) => {
    await page.goto(PURE);
    const got = await resize(page, { edge: 'end', dyFrac: 1 / 24 });

    expect(got.endMs).toBe(ORIGIN.endMs + HOUR);
    expect(got.startMs, 'the other edge stays put').toBe(ORIGIN.startMs);
  });

  /**
   * §5: **a resize may not invert an event.** `endAfterStart` refuses a
   * negative span in the form, so a grid able to construct one produces a block
   * the form would reject — that inconsistency is the defect, not the negative
   * number itself.
   *
   * The minimum is the snap interval, deliberately rather than a second
   * constant: the smallest span the grid can express is the smallest one it
   * should be able to produce.
   */
  test('dragging the start past the end clamps to a minimum instead of inverting', async ({ page }) => {
    await page.goto(PURE);
    // Three hours down, from a half-hour event: far past its own end.
    const got = await resize(page, { edge: 'start', dyFrac: 3 / 24 });

    expect(got.endMs - got.startMs).toBeGreaterThan(0);
    expect(got.endMs - got.startMs).toBe(SNAP);
    expect(got.endMs, 'the edge not being dragged is still untouched').toBe(ORIGIN.endMs);
    expect(got.startMs).toBe(ORIGIN.endMs - SNAP);
  });

  test('dragging the end before the start clamps to a minimum instead of inverting', async ({ page }) => {
    await page.goto(PURE);
    const got = await resize(page, { edge: 'end', dyFrac: -3 / 24 });

    expect(got.endMs - got.startMs).toBe(SNAP);
    expect(got.startMs, 'the edge not being dragged is still untouched').toBe(ORIGIN.startMs);
    expect(got.endMs).toBe(ORIGIN.startMs + SNAP);
  });

  /**
   * The boundary of the clamp, from the safe side: a resize that lands exactly
   * on the minimum is honoured rather than clamped, so the clamp cannot be
   * satisfied by a function that always returns the minimum.
   */
  test('a resize down to exactly the minimum is left alone', async ({ page }) => {
    await page.goto(PURE);
    // The end back by 15 minutes, from a 30-minute event: exactly the minimum.
    const got = await resize(page, { edge: 'end', dyFrac: -0.25 / 24 });

    expect(got.endMs - got.startMs).toBe(SNAP);
    expect(got.endMs).toBe(ORIGIN.endMs - 15 * MIN);
  });
});

/**
 * The threshold, as arithmetic. Its *behavioural* witness is
 * `drag-gesture.spec.ts`'s "a click still opens the popover" — this is the
 * table underneath it, and the reason the predicate is not written inline in
 * Svelte.
 */
test.describe('the drag threshold', () => {
  const cases: Array<{ dx: number; dy: number; began: boolean; why: string }> = [
    { dx: 0, dy: 0, began: false, why: 'a press that has not moved is a click' },
    { dx: 0, dy: 3, began: false, why: 'the jitter every real click has' },
    { dx: 0, dy: 4, began: true, why: 'exactly the threshold begins a drag' },
    { dx: 4, dy: 0, began: true, why: 'and the same sideways' },
    { dx: 0, dy: -4, began: true, why: 'and upward: it is distance, not direction' },
    { dx: 3, dy: 3, began: true, why: 'three each way is 4.24 of travel, not three' },
    { dx: 2, dy: 2, began: false, why: 'two each way is 2.83, still a click' },
  ];

  for (const c of cases) {
    test(`dx ${c.dx}, dy ${c.dy} ${c.began ? 'begins a drag' : 'is still a click'} — ${c.why}`,
      async ({ page }) => {
        await page.goto(PURE);
        const got = await page.evaluate(
          (a) => (window as any).__drag.beganDrag(a.dx, a.dy),
          { dx: c.dx, dy: c.dy },
        );
        expect(got).toBe(c.began);
      });
  }
});

/**
 * Which end of a block a press grabs. Arithmetic, so it has a table; whether
 * the grid then *resizes* is `drag-gesture.spec.ts`'s.
 */
test.describe('the grab bands at a block’s ends', () => {
  const H = 40; // comfortably more than three 6px bands

  const cases: Array<{ y: number; h?: number; edge: string | null; why: string }> = [
    { y: 0, edge: 'start', why: 'the very top' },
    { y: 6, edge: 'start', why: 'the last pixel of the top band' },
    { y: 7, edge: null, why: 'one past it is the middle' },
    { y: 20, edge: null, why: 'the middle is a move' },
    { y: 33, edge: null, why: 'one short of the bottom band' },
    { y: 34, edge: 'end', why: 'the first pixel of the bottom band' },
    { y: 40, edge: 'end', why: 'the very bottom' },
    // A 15-minute block on a 1200px day is 12.5px: two bands would leave
    // nothing between them, so it has no edges and stays draggable.
    { y: 0, h: 12, edge: null, why: 'a short block has no top band' },
    { y: 12, h: 12, edge: null, why: 'nor a bottom one' },
    { y: 6, h: 12, edge: null, why: 'and no middle to lose' },
  ];

  for (const c of cases) {
    test(`y ${c.y} of ${c.h ?? H} grabs ${c.edge ?? 'nothing'} — ${c.why}`, async ({ page }) => {
      await page.goto(PURE);
      const got = await page.evaluate(
        (a) => (window as any).__drag.edgeAt(a.y, a.h),
        { y: c.y, h: c.h ?? H },
      );
      expect(got).toBe(c.edge);
    });
  }
});

/** How far sideways is one day. */
test.describe('crossing day columns', () => {
  const W = 100;

  const cases: Array<{ dx: number; cols: number; why: string }> = [
    { dx: 0, cols: 0, why: 'no movement crosses nothing' },
    { dx: 49, cols: 0, why: 'not yet halfway stays put' },
    { dx: 50, cols: 1, why: 'exactly halfway lands on the next' },
    { dx: 100, cols: 1, why: 'a whole column' },
    { dx: 260, cols: 3, why: 'and three of them, rounded' },
    { dx: -49, cols: 0, why: 'the same going left' },
    // The mirror of the 50 above. `Math.round(-0.5)` is -0, so the naive
    // version stays put going left while moving going right.
    { dx: -50, cols: -1, why: 'exactly halfway left lands on the previous' },
    { dx: -100, cols: -1, why: 'a whole column left' },
  ];

  for (const c of cases) {
    test(`dx ${c.dx} over a ${W}px column is ${c.cols} — ${c.why}`, async ({ page }) => {
      await page.goto(PURE);
      const got = await page.evaluate(
        (a) => (window as any).__drag.colsMoved(a.dx, a.w),
        { dx: c.dx, w: W },
      );
      expect(got).toBe(c.cols);
    });
  }

  test('a column with no width crosses nothing rather than dividing by zero', async ({ page }) => {
    await page.goto(PURE);
    const got = await page.evaluate(() => (window as any).__drag.colsMoved(120, 0));
    expect(got).toBe(0);
  });
});

/**
 * Task 6: the span a sweep over empty grid produces.
 *
 * Nothing is created here — the form is, and it is opened pre-filled. So this
 * answers exactly one question: given where a press landed in a column and
 * where the pointer got to, which two instants does the form open on?
 */
test.describe('sweeping out a new span', () => {
  /** Midnight of the day `T0900Z` is in. */
  const DAY_START = Date.UTC(2026, 7, 10);

  /** A fraction of a 24-hour column for a whole number of hours. */
  const at = (hours: number) => hours / 24;

  /** `spanForSweep` in the page, over an ordinary 24-hour day starting at
   *  midnight UTC. Fractions are of the column's own height. */
  const sweep = (
    page: import('@playwright/test').Page,
    args: { from: number; to: number; dayStartMs?: number; dayMs?: number },
  ): Promise<Span> =>
    page.evaluate((a) => {
      const d = (window as any).__drag;
      return d.spanForSweep(a.dayStartMs, a.dayMs, a.from, a.to, a.snap);
    }, {
      dayStartMs: args.dayStartMs ?? DAY_START,
      dayMs: args.dayMs ?? DAY,
      from: args.from,
      to: args.to,
      snap: SNAP,
    });

  /** `sweepAsk` in the page, over a week of ordinary days from `DAY_START`. */
  const ask = (
    page: import('@playwright/test').Page,
    args: { from: number; cols: number; fromFrac?: number; toFrac?: number },
  ): Promise<any> =>
    page.evaluate((a) => {
      const days = Array.from({ length: 7 }, (_, i) => ({
        start_ms: a.dayStart + i * a.dayMs,
        end_ms: a.dayStart + (i + 1) * a.dayMs,
      }));
      return (window as any).__drag.sweepAsk(
        days, a.from, a.cols, a.fromFrac, a.toFrac, a.snap,
      );
    }, {
      dayStart: DAY_START, dayMs: DAY, snap: SNAP,
      from: args.from, cols: args.cols,
      fromFrac: args.fromFrac ?? at(9), toFrac: args.toFrac ?? at(10),
    });

  test('staying in one column is still a timed event', async ({ page }) => {
    await page.goto(PURE);
    expect(await ask(page, { from: 1, cols: 0 })).toEqual({
      kind: 'timed',
      startMs: DAY_START + DAY + 9 * HOUR,
      endMs: DAY_START + DAY + 10 * HOUR,
    });
  });

  /** The requested gesture (2026-08-31): cross a column and the sweep names
   *  days rather than hours. Last day inclusive — the form's own reading. */
  test('crossing columns asks for an all-day span, last day inclusive', async ({ page }) => {
    await page.goto(PURE);
    expect(await ask(page, { from: 1, cols: 2 })).toEqual({
      kind: 'allDay',
      firstDayMs: DAY_START + DAY,
      lastDayMs: DAY_START + 3 * DAY,
    });
  });

  /** Right-to-left names the same days: a set has no direction. */
  test('sweeping backwards names the same days as sweeping forwards', async ({ page }) => {
    await page.goto(PURE);
    const forward = await ask(page, { from: 1, cols: 3 });
    const back = await ask(page, { from: 4, cols: -3 });
    expect(back).toEqual(forward);
  });

  /** Off the edge of the week stops at the week's edge — a sweep cannot name
   *  a day the grid is not showing. */
  test('a sweep past the last column stops at it', async ({ page }) => {
    await page.goto(PURE);
    expect(await ask(page, { from: 5, cols: 9 })).toEqual({
      kind: 'allDay',
      firstDayMs: DAY_START + 5 * DAY,
      lastDayMs: DAY_START + 6 * DAY,
    });
  });

  /** Vertical travel is irrelevant once a column has been crossed: the ask is
   *  about days, and reading an hour off it would be reading something the
   *  gesture never named. */
  test('an all-day ask ignores how far the hand went vertically', async ({ page }) => {
    await page.goto(PURE);
    const flat = await ask(page, { from: 0, cols: 1, fromFrac: at(9), toFrac: at(9) });
    const steep = await ask(page, { from: 0, cols: 1, fromFrac: at(1), toFrac: at(23) });
    expect(steep).toEqual(flat);
  });

  test('a downward sweep is the span it swept', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: at(14), to: at(15) });

    expect(got.startMs).toBe(DAY_START + 14 * HOUR);
    expect(got.endMs).toBe(DAY_START + 15 * HOUR);
  });

  /**
   * **Upward, and this is its own case rather than a corollary.**
   *
   * Sweeping from 15:00 up to 14:00 is the same gesture read backwards and must
   * produce 14:00–15:00, not a span an hour long and negative. `endAfterStart`
   * would refuse the negative one and the form would open dead — the same
   * family as the resize clamp, reached through a different door, which is why
   * it does not inherit that one's proof.
   */
  test('an upward sweep is the same span, not a negative one', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: at(15), to: at(14) });

    expect(got.endMs - got.startMs, 'forwards').toBeGreaterThan(0);
    expect(got.startMs).toBe(DAY_START + 14 * HOUR);
    expect(got.endMs).toBe(DAY_START + 15 * HOUR);
  });

  /**
   * **The minimum is not cosmetic.** A hand that twitches between press and
   * release sweeps a few pixels, both ends snap to the same slot, and the span
   * is zero — which `endAfterStart` refuses, so the form would open already
   * unable to save with no field on it visibly wrong. The clamp is what keeps
   * the form *openable*, which is the whole of this task's deliverable.
   */
  test('a sweep shorter than the snap still yields a usable span', async ({ page }) => {
    await page.goto(PURE);
    // Four minutes: past nothing, and well inside one 15-minute slot.
    const got = await sweep(page, { from: at(14), to: at(14) + 4 / (24 * 60) });

    expect(got.endMs - got.startMs).toBe(SNAP);
    expect(got.startMs).toBe(DAY_START + 14 * HOUR);
  });

  test('a sweep that did not move at all still yields a usable span', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: at(14), to: at(14) });

    expect(got.endMs - got.startMs).toBe(SNAP);
  });

  /**
   * And upward into the same slot, which is the twitch's mirror image: the
   * minimum extends the **end** forward from the earlier of the two points,
   * always, so a sweep of nothing gives the same answer whichever way the hand
   * moved. Stated as a rule rather than left to whichever branch ran.
   */
  test('a sub-snap upward sweep extends the end, not the start', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: at(14) + 4 / (24 * 60), to: at(14) });

    expect(got.startMs).toBe(DAY_START + 14 * HOUR);
    expect(got.endMs).toBe(DAY_START + 14 * HOUR + SNAP);
  });

  /**
   * The boundary from the safe side, so the clamp cannot be satisfied by a
   * function that always returns the minimum: a sweep of exactly one slot is
   * honoured as swept.
   */
  test('a sweep of exactly the minimum is left alone', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: at(14), to: at(14.25) });

    expect(got.endMs - got.startMs).toBe(SNAP);
    expect(got.endMs).toBe(DAY_START + 14 * HOUR + SNAP);
  });

  /**
   * Both ends snap, and to the *nearest* slot rather than the one the pointer
   * is inside. Without this the sweep could be raw pixel arithmetic and every
   * case above — all of which sit on whole hours — would still pass.
   */
  test('both ends land on a snap step', async ({ page }) => {
    await page.goto(PURE);
    // 14:10 to 15:20 — neither on a slot, and they round in *opposite*
    // directions, so a version that only ever floored (or only ever ceiled)
    // both ends fails on one of them.
    const got = await sweep(page, {
      from: at(14) + 10 / (24 * 60),
      to: at(15) + 20 / (24 * 60),
    });

    expect(got.startMs, 'rounded forward').toBe(DAY_START + 14 * HOUR + 15 * MIN);
    expect(got.endMs, 'rounded back').toBe(DAY_START + 15 * HOUR + 15 * MIN);
  });

  /** A pointer dragged off the top or bottom of the column pins to the day's
   *  own ends rather than sweeping into the day before or after — the same
   *  clamp `slotAt` applies to a click. */
  test('a sweep off the ends of the column pins to the day', async ({ page }) => {
    await page.goto(PURE);
    const got = await sweep(page, { from: -0.4, to: 1.6 });

    expect(got.startMs).toBe(DAY_START);
    expect(got.endMs).toBe(DAY_START + DAY);
  });
});

test.describe('sweeping on a daylight-saving day', () => {
  test.use({ timezoneId: 'Europe/Sofia' });

  /**
   * The vertical axis is a fraction of **that day's own span**, for the sweep
   * exactly as for a move. Half way down a 23-hour Sunday is 11.5 hours past
   * midnight, which the wall clock reads as 12:30 — a version dividing by a
   * fixed 24 opens the form an hour out for every sweep after the transition.
   */
  test('a sweep is a fraction of the day it is swept in', async ({ page }) => {
    await page.goto(PURE);
    const sunStart = Date.parse('2026-03-29T00:00:00+02:00');
    const dayMs = Date.parse('2026-03-30T00:00:00+03:00') - sunStart;
    expect(dayMs, 'fixture check: a spring-forward Sunday is 23 hours').toBe(23 * HOUR);

    // Read in the page: `timezoneId` reaches the browser context and not Node.
    const got = await page.evaluate((a) => {
      const d = (window as any).__drag;
      const span = d.spanForSweep(a.sunStart, a.dayMs, 0.5, 0.75, a.snap);
      const s = new Date(span.startMs);
      return { ...span, hour: s.getHours(), minute: s.getMinutes() };
    }, { sunStart, dayMs, snap: SNAP });

    expect(got.startMs).toBe(sunStart + 11.5 * HOUR);
    expect(got.hour).toBe(12);
    expect(got.minute).toBe(30);
  });
});

test('the module exports the geometry the plan names, and the gesture constants', async ({ page }) => {
  await page.goto(PURE);
  // A guard on the shape rather than on any answer: Task 3 wires a component to
  // these names, and a rename here would otherwise surface as a runtime
  // undefined in a gesture spec rather than as a failure here.
  expect((await drag(page)).sort()).toEqual(
    ['DRAG_THRESHOLD_PX', 'RESIZE_EDGE_PX', 'SNAP_MS', 'beganDrag', 'colsMoved', 'edgeAt',
     'hasResizeEdges', 'snapMs', 'spanForMove', 'spanForResize', 'spanForSweep', 'sweepAsk'],
  );
});
