import { test, expect } from '@playwright/test';
import type { MonthPayload, WeekPayload } from '../src/lib/api';
import {
  FIXTURES, bandAsWeek, busyDayMonth, labelledWeek, keyboardWeek, appWritableWeek,
  crossZoneWeek, APP_MON,
} from './fixtures';

// The fixtures themselves are under test here.
//
// `generated/cross-zone-week.json` settles the drift question for one payload
// the strong way: it is produced by the assembler that really answers
// `get_week`, and a Rust test fails the moment that assembler's output moves.
// Every other fixture in `fixtures.ts` is still written by hand, and a
// hand-written payload can describe something no assembler could ever return.
// `busyDayMonth` did exactly that — it committed a cell running 06:00 to the
// next midnight, eighteen hours long, and passed all 492 specs, because a UTC
// browser reads both ends as day 10 and nothing in the suite asked how long the
// day was. It was found by review, which is not a mechanism.
//
// This sweep is the cheap half of the answer for fixtures that are not
// generated. It cannot say "this is the payload the backend produces" — only a
// golden file can — but it can say "the backend could have produced this at
// all", which is the whole of what went wrong above. The invariants are read
// off the assembler that produces each payload rather than invented, and each
// names the line that guarantees it: `assemble_month` in
// `src-tauri/src/commands.rs` for the first `describe`, and `assemble_days`
// there plus `lay_out_day` (`crates/omacal-core/src/layout.rs`) and
// `pack_lanes` (`crates/omacal-core/src/lanes.rs`) for the second.
//
// No `page` anywhere below: these are assertions about committed data, so
// nothing here opens a browser context. That is deliberate —
// `playwright.config.ts` budgets contexts per worker, and a sweep that grows
// with the fixture count should not spend them.

const H = 3_600_000;

/** Every `MonthPayload` fixture the suite can serve, under the key it is served
 *  by. `FIXTURES.MonthGrid` really is the whole set: `augustMonth` and
 *  `twoBarsMonth` are module-private to `fixtures.ts` and reachable only
 *  through this table, and `busyDayMonth` — the one the App harness's
 *  `get_month` stub returns as well — is pinned to it by the last test here. */
const months: [string, MonthPayload][] = Object.entries(FIXTURES.MonthGrid).map(
  ([name, f]) => [name, f.month as MonthPayload],
);

/** Row `r`, column `c` of a 42-cell grid, flattened — the order
 *  `assemble_month` builds `bounds` in, and the order abutment runs in. */
const cellsOf = (m: MonthPayload) => m.rows.flatMap((row) => row.cells);

test.describe('the hand-written month fixtures describe payloads assemble_month could produce', () => {
  // The sweep has to sweep something. A `months` that came back empty — a
  // renamed `FIXTURES` key, a refactor that moved the month fixtures elsewhere
  // — would satisfy every loop below by iterating zero times, and this file
  // would go green while checking nothing at all. Four today: `august`,
  // `busy-day`, `two-bars`, `filmstrip`.
  test('the sweep finds the month fixtures at all', () => {
    expect(months.length).toBeGreaterThanOrEqual(4);
    expect(months.map(([name]) => name)).toEqual(
      expect.arrayContaining(['august', 'busy-day', 'two-bars', 'filmstrip']),
    );
  });

  test('every fixture is six rows of seven abutting days', () => {
    // Repeated in every looping test rather than left to the one above, and
    // two different holes need it. With an *empty* sweep `checked` and
    // `months.length * 42` are both zero and the count at the bottom passes.
    // With a *partial* one — `months.slice(0, 1)` — both counts agree as well,
    // and the per-fixture `checked > 0` floors in the later tests are satisfied
    // by one fixture out of three. Only a floor on `months.length` catches
    // that, so each test carries its own.
    expect(months.length).toBeGreaterThanOrEqual(3);
    let checked = 0;
    for (const [name, m] of months) {
      // `rows` is always 6 and `cells` always 7 — `(0..6)` and `(0..7)` in
      // `assemble_month`, fixed so the grid never changes height.
      expect(m.rows, name).toHaveLength(6);
      for (const [r, row] of m.rows.entries()) expect(row.cells, `${name} row ${r}`).toHaveLength(7);

      const cells = cellsOf(m);
      for (const [k, cell] of cells.entries()) {
        // A cell spans one civil day: `start_ms` is `row_bounds[c]` and
        // `end_ms` is `row_bounds[c + 1]`, consecutive entries of
        // `n_day_boundaries`, which steps by `checked_add(1.day())`. So a day
        // is 24 hours, or 23/25 across a DST transition — never 18, which is
        // what `busyDayMonth` used to commit. Bounded rather than fixed at 24h
        // so a future DST-week fixture is not forced to lie.
        const span = cell.end_ms - cell.start_ms;
        expect(span, `${name} cell ${k} spans ${span / H}h`).toBeGreaterThanOrEqual(23 * H);
        expect(span, `${name} cell ${k} spans ${span / H}h`).toBeLessThanOrEqual(25 * H);

        // And the grid has no gaps or overlaps. Row `r`'s last cell ends at
        // `bounds[r * 7 + 7]`, which *is* row `r + 1`'s first cell start, so
        // this holds across row boundaries too — hence the flattened sweep
        // rather than one per row. This is the zone-independent half: whatever
        // zone a grid is built in, consecutive cells meet.
        if (k + 1 < cells.length) {
          expect(cells[k + 1].start_ms, `${name} cell ${k}/${k + 1} do not meet`)
            .toBe(cell.end_ms);
        }
        checked++;
      }
    }
    // Every fixture contributed all 42 of its cells — a loop that skipped a
    // row, or a fixture whose rows were empty arrays, lands here.
    expect(checked).toBe(months.length * 42);
  });

  test('every fixture dims exactly the days of the year and month it claims', () => {
    expect(months.length).toBeGreaterThanOrEqual(4); // see above
    for (const [name, m] of months) {
      // The month it claims to be is one a grid can be built for.
      expect(m.month, name).toBeGreaterThanOrEqual(1);
      expect(m.month, name).toBeLessThanOrEqual(12);

      const inMonth = cellsOf(m)
        .map((c, i) => (c.in_month ? i : -1))
        .filter((i) => i >= 0);
      // `in_month` is `start_ms >= month_start_ms && start_ms < next_month_start_ms`
      // against strictly increasing cells, so the in-month days are one
      // unbroken run — a fixture that dimmed a day in the middle of its own
      // month, or lit one either side of it, could not have come from there.
      expect(inMonth.length, `${name} has no in-month days`).toBeGreaterThanOrEqual(28);
      expect(inMonth.length, `${name} has more in-month days than a month has`)
        .toBeLessThanOrEqual(31);
      expect(inMonth, `${name}'s in-month days are not contiguous`).toEqual(
        Array.from({ length: inMonth.length }, (_, i) => inMonth[0] + i),
      );

      // …and the run is the month the payload *says* it is. Everything above
      // is satisfied by any 28-31 day run anywhere in the grid: a fixture
      // claiming `month: 7` while dimming August's 31 days is contiguous, the
      // right length, and a legal month number. `year` was not read at all.
      //
      // This is `commands.rs:433` itself, which is the whole predicate rather
      // than a consequence of it. The two above are kept because they name
      // their own failure shapes; this one is the strong form.
      const monthStart = Date.UTC(m.year, m.month - 1, 1);
      // A month index of 12 rolls into January of the next year, which is
      // exactly `assemble_month`'s own `if month == 12` branch.
      const nextMonthStart = Date.UTC(m.year, m.month, 1);
      for (const [k, cell] of cellsOf(m).entries()) {
        // The boundary above is a UTC one, and these fixtures are UTC grids —
        // `fixtures.ts` says so, and `playwright.config.ts` pins the browser to
        // UTC for the same reason. Asserted rather than assumed: a month
        // fixture built in a real zone must fail *here*, with a reason, rather
        // than be compared against a boundary that does not apply to it.
        expect(cell.start_ms % (24 * H), `${name} cell ${k} is not a UTC midnight`).toBe(0);
        expect(cell.in_month, `${name} cell ${k} is dimmed against ${m.year}-${m.month}`)
          .toBe(cell.start_ms >= monthStart && cell.start_ms < nextMonthStart);
      }
    }
  });

  test('every timed event sits in the cell carrying it, in start order', () => {
    expect(months.length).toBeGreaterThanOrEqual(4); // see above
    let checked = 0;
    for (const [name, m] of months) {
      for (const [r, row] of m.rows.entries()) {
        for (const [c, cell] of row.cells.entries()) {
          const where = `${name} row ${r} cell ${c}`;
          // `timed.sort_by_key(|e| e.start_ms)` — the UI renders the list as
          // given and computes its own `+N more` from the tail, so an unsorted
          // fixture would drop the wrong events.
          const starts = cell.timed.map((e) => e.start_ms);
          expect(starts, `${where} is not sorted by start`).toEqual([...starts].sort((a, b) => a - b));

          for (const e of cell.timed) {
            // `timed_column` buckets by the column *containing* the start…
            expect(e.start_ms, `${where}: ${e.title} starts at or after the next day`)
              .toBeLessThan(cell.end_ms);
            // …with one exception, and it is a real one: an event that began
            // before the window and runs into it is clamped into column 0
            // rather than dropped, so there and only there a start may sit
            // before the cell. Everywhere else a start before the cell means
            // the event is in the wrong day.
            if (c === 0) {
              expect(e.end_ms, `${where}: ${e.title} does not reach the row`)
                .toBeGreaterThan(cell.start_ms);
            } else {
              expect(e.start_ms, `${where}: ${e.title} starts before its own cell`)
                .toBeGreaterThanOrEqual(cell.start_ms);
            }
            checked++;
          }
        }
      }
    }
    // Otherwise a fixture set with no timed events anywhere passes this by
    // iterating nothing. Seven today: one in `august`, four in `busy-day`, two
    // in `filmstrip`.
    expect(checked).toBeGreaterThan(0);
  });

  test('every bar indexes an event that exists and stays inside its row', () => {
    expect(months.length).toBeGreaterThanOrEqual(4); // see above
    let checked = 0;
    for (const [name, m] of months) {
      for (const [r, row] of m.rows.entries()) {
        for (const bar of row.bars) {
          const where = `${name} row ${r} bar ${bar.idx}`;
          // `Segment { idx: bar_events.len(), .. }` is pushed alongside the
          // event it names, so every lane indexes a real entry. This is the
          // `bar_events[lane.idx]` / `bar_events[lane.lane]` mix-up's other
          // half: `two-bars` proves the UI reads the right one, this proves no
          // fixture offers an index that cannot be read at all.
          expect(row.bar_events[bar.idx], `${where} indexes nothing`).toBeDefined();
          // `pack_lanes(&segments, 7, 3)` clips each segment to the row and
          // packs into at most three lanes.
          expect(bar.start_col, `${where} starts left of the row`).toBeGreaterThanOrEqual(0);
          expect(bar.end_col, `${where} ends right of the row`).toBeLessThanOrEqual(6);
          expect(bar.end_col, `${where} ends before it starts`).toBeGreaterThanOrEqual(bar.start_col);
          expect(bar.lane, `${where} is in a lane the month does not draw`).toBeLessThan(3);
          checked++;
        }
        for (const i of row.bar_overflow) {
          expect(row.bar_events[i], `${name} row ${r} overflow ${i} indexes nothing`).toBeDefined();
        }
      }
    }
    // Four today: one in `august`, two in `two-bars`, one in `filmstrip`.
    expect(checked).toBeGreaterThan(0);
  });

  test('the month payload the App harness serves is one of the fixtures swept above', () => {
    // `harness/tauri.ts`'s `get_month` stub answers with `busyDayMonth()`
    // directly rather than through `FIXTURES`. If those two ever stop being
    // the same payload, every check above would be sweeping a fixture the App
    // specs never see — which is the shape of gap this whole file exists to
    // close, one level up.
    expect(busyDayMonth()).toEqual(FIXTURES.MonthGrid['busy-day'].month);
  });
});

// --- The week half --------------------------------------------------------
//
// `MonthPayload` is now covered above and `crossZoneWeek` is pinned the strong
// way by `generated/cross-zone-week.json`. The other `WeekPayload` fixtures
// were covered by neither, and they carry a family the month grid has no
// equivalent for: `Placed`, whose `top`/`height` are day fractions computed by
// `lay_out_day` and are the only `f32` anything in `fixtures.ts` describes.

/** `lay_out_day`'s own `MIN_HEIGHT` (layout.rs:34) — the floor a zero-length or
 *  very short event's box is raised to so it stays clickable. It is the reason
 *  `top + height` is not bounded by 1 — see `every placed box is geometry
 *  lay_out_day could have produced` below. */
const MIN_HEIGHT = 0.004;

/** How close a recomputed `f32` fraction has to land. `toBeCloseTo`'s own rule
 *  is `|diff| < 0.5 * 10 ** -precision`, restated here so the offender scan in
 *  the same test uses the identical threshold rather than a second one that
 *  could disagree with the assertion beside it. */
const FRAC_PRECISION = 6;
const near = (a: number, b: number) => Math.abs(a - b) < 0.5 * 10 ** -FRAC_PRECISION;

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/** Every hand-written `WeekPayload` the suite can serve, under a name it can be
 *  found by.
 *
 *  `FIXTURES.WeekGrid` is only part of the set. The App specs are answered by
 *  `harness/tauri.ts`'s `get_week` out of three exported builders that are in
 *  no `FIXTURES` table at all — `labelledWeek`, `appWritableWeek`,
 *  `crossZoneWeek` — which is the same gap the month sweep's last test closes
 *  for `busyDayMonth`. Here it is closed by construction instead: this table
 *  calls the very functions the harness imports, so there is no second copy to
 *  drift. `labelledWeek` is uniform in its argument (one event, same shape, at
 *  whatever Monday it is given), so two samples stand for every week the
 *  harness can be asked for; the second is the adjacent week the App paging
 *  specs move to.
 *
 *  `app:cross-zone` is the one *generated* payload in the list, and it is swept
 *  alongside the hand-written ones deliberately. It is real `assemble_week`
 *  output, so it calibrates every invariant below: one that these fixtures pass
 *  while the backend's own output fails is an invariant that was invented. */
const weeks: [string, WeekPayload][] = [
  ...Object.entries(FIXTURES.WeekGrid).map(
    ([name, f]) => [name, f.week as WeekPayload] as [string, WeekPayload],
  ),
  ['app:labelled', labelledWeek(APP_MON)],
  ['app:labelled-next', labelledWeek(APP_MON + 7 * 24 * H)],
  ['app:keyboard', keyboardWeek(APP_MON)],
  ['app:writable', appWritableWeek()],
  ['app:cross-zone', crossZoneWeek()],
  // `AllDayBand`'s own props fixtures, lifted back into the payload they are a
  // projection of — see `bandAsWeek`. They were outside this sweep until one of
  // them was found describing a band whose `overflow` indexed nothing, which is
  // exactly the class of defect the sweep exists to catch; being a different
  // *shape* was never a reason to be a different *standard*.
  //
  // `corners` is not here, and that is a ruling rather than an oversight — see
  // `BAND_NOT_SWEPT` below.
  ...(['populated', 'three-days', 'overflow', 'empty'] as const).map(
    (n) => [`band:${n}`, bandAsWeek(FIXTURES.AllDayBand[n])] as [string, WeekPayload],
  ),
];

/**
 * The one `AllDayBand` fixture the lane sweep cannot hold to its invariants,
 * named here so the exemption is a decision on the record rather than a gap.
 *
 * `corners` puts four chips in lanes 0-3 and gives two of them `cont_right`
 * while they end in column 1. Both are things `pack_lanes` cannot emit: the
 * week band packs at `max_lanes = 2` (`commands.rs:342`), and `cont_right` is
 * set by the clip and only by the clip, so it implies the chip reaches the last
 * column (`lanes.rs:45-48`). Swept, it would fail on both.
 *
 * It is still the right fixture. Its job is to put each of the four corner
 * states — round/round, flat-left, flat-right, flat/flat — on screen *at once
 * and in isolation*, so `AllDayBand chip corners` can snapshot each chip on its
 * own at zero tolerance against a WKWebView artifact that squares the corners
 * away from a one-sided border. One lane per state is what makes them
 * separately framable; two lanes cannot show four states, and no real payload
 * puts all four on one band anyway.
 *
 * So the choice was to weaken the sweep, or destroy a fixture that guards a
 * real rendering bug, or exempt it explicitly. This is the third. Unlike the
 * `overflow` fixture it replaces in this role, nothing downstream can be misled
 * by it: it is consumed by four screenshot specs and by nothing that reads an
 * index.
 */
const BAND_NOT_SWEPT = ['corners'] as const;

test('the AllDayBand props fixtures are swept unless deliberately exempt', () => {
  // The exemption above is only honest while it stays exhaustive. A fifth
  // fixture added to `FIXTURES.AllDayBand` and quietly left out of both lists
  // is precisely how `overflow` sat unswept, so this fails rather than lets it.
  const all = Object.keys(FIXTURES.AllDayBand).sort();
  const swept = weeks.filter(([n]) => n.startsWith('band:')).map(([n]) => n.slice(5));
  expect([...swept, ...BAND_NOT_SWEPT].sort(), 'an AllDayBand fixture is neither swept nor exempt')
    .toEqual(all);
});

/**
 * The three fixtures whose day columns are deliberately **not** derived from
 * the timestamps of the events in them, and which `assemble_days` therefore
 * could not have produced.
 *
 * This is a real deviation, not an oversight, and each one's own comment in
 * `fixtures.ts` says so: a block's on-screen position comes from its `Placed`
 * entry, never from its event's `start_ms`, and these three exploit that to put
 * a block whose `start_ms` belongs to another day — or another year — into
 * Monday's column, because the gap between the two *is* what their specs test.
 * `popover`'s recurring block is the August 2026 occurrence its detail's
 * DTSTART is three days short of, sitting in a January 2024 column — a
 * different *year*, not a different day; `popover-two-occurrences` puts two
 * occurrences of one series a day apart in the same column, which is how one
 * store row gets clicked twice; `app:writable` carries a Thursday occurrence of
 * a series whose master is the Monday.
 *
 * So the two tests that read event timestamps skip them — and then assert the
 * skip list is **exactly** this set. An exemption that is merely tolerated goes
 * stale silently; one that must be exhaustive cannot. Repair a fixture and the
 * test fails until the name is removed; add a fourth fixture with the same
 * defect by accident and it fails until someone decides which of the two it is.
 */
const DECOUPLED = ['app:writable', 'popover', 'popover-two-occurrences'];

test.describe('the hand-written week fixtures describe payloads assemble_days could produce', () => {
  // Same reasoning as the month sweep's first test: a renamed `FIXTURES` key or
  // a moved export would leave `weeks` short or empty, and every loop below
  // would pass by iterating nothing.
  test('the sweep finds the week fixtures at all', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15);
    expect(weeks.map(([name]) => name)).toEqual(
      expect.arrayContaining([
        'empty', 'populated', 'popover', 'popover-two-occurrences', 'popover-all-day',
        'single-day', 'single-day-overlap', 'filmstrip',
        'app:labelled', 'app:labelled-next', 'app:keyboard', 'app:writable', 'app:cross-zone',
      ]),
    );
  });

  test('every fixture is a run of abutting day columns', () => {
    // Carried per test rather than left to the one above, for both holes the
    // month sweep names: an *empty* `weeks` satisfies the count at the bottom
    // (`0 === 0`), and a *partial* one satisfies it too.
    expect(weeks.length).toBeGreaterThanOrEqual(15);
    let checked = 0;
    for (const [name, w] of weeks) {
      // `assemble_days(events, start_ms, n, tz)` serves Day at `n = 1`, fixed
      // Week at `n = 7`, and rolling Week ranges at `n = 3`, `5`, or `7`.
      // No other column count is reachable, so a fixture with two days — or
      // none — is one nothing can serve.
      // 21 is the padded fetch: a week's padding either side of a fixed
      // week, which `get_week(…, pad: 7)` has served since 2026-09-03.
      expect([1, 3, 5, 7, 21], `${name} has ${w.days.length} day columns`)
        .toContain(w.days.length);

      for (const [d, col] of w.days.entries()) {
        // `DayColumn { start_ms: bounds[d], end_ms: bounds[d + 1], .. }`
        // (commands.rs:354) — consecutive entries of `n_day_boundaries`, which
        // steps by `checked_add(1.day())` (commands.rs:173). So a column is 24
        // hours, or 23/25 across a DST transition. Bounded rather than fixed at
        // 24h for the same reason the month sweep bounds it: so a future
        // DST-week fixture is not forced to lie.
        const span = col.end_ms - col.start_ms;
        expect(span, `${name} day ${d} spans ${span / H}h`).toBeGreaterThanOrEqual(23 * H);
        expect(span, `${name} day ${d} spans ${span / H}h`).toBeLessThanOrEqual(25 * H);

        // And the columns meet: column `d` ends at `bounds[d + 1]`, which *is*
        // column `d + 1`'s start. Zone-independent — whatever zone a week is
        // built in, its columns abut.
        if (d + 1 < w.days.length) {
          expect(w.days[d + 1].start_ms, `${name} days ${d}/${d + 1} do not meet`)
            .toBe(col.end_ms);
        }
        checked++;
      }
    }
    // Every fixture contributed all of its columns. A `days: []` is already
    // caught by the allowed-count check above; this catches a loop that skipped one.
    expect(checked).toBe(weeks.reduce((n, [, w]) => n + w.days.length, 0));
  });

  test('every day column carries timed events only, and the band all-day ones only', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15); // see above
    let checked = 0;
    for (const [name, w] of weeks) {
      // `if src.is_all_day { .. all_day_events.push(..) } else if let Some(col)
      // = timed_column(..) { day_events[col].push(..) }` (commands.rs:332-338):
      // the two are exclusive, and `to_ui` copies `is_all_day` through verbatim
      // (commands.rs:90). So the flag on an event says which array it came out
      // of, and a chip in a day column — or a timed meeting in the band — is a
      // payload no assembler wrote. There is no `EventBlock` path for an
      // all-day event at all, which is why several fixtures here say so in
      // their own comments; this is that claim as a mechanism.
      for (const [d, col] of w.days.entries()) {
        for (const e of col.events) {
          expect(e.is_all_day, `${name} day ${d}: ${e.title} is all-day in a day column`)
            .toBe(false);
          checked++;
        }
      }
      for (const e of w.all_day_events) {
        expect(e.is_all_day, `${name} band: ${e.title} is not an all-day event`).toBe(true);
        checked++;
      }
    }
    // Otherwise a fixture set with no events at all passes by iterating
    // nothing. Thirty-three today: twenty in day columns, thirteen in the bands.
    expect(checked).toBeGreaterThan(0);
  });

  test('every timed event sits in the column carrying it, except where a fixture says otherwise', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15); // see above
    let checked = 0;
    let asserted = 0;
    const offenders = new Set<string>();
    for (const [name, w] of weeks) {
      const decoupled = DECOUPLED.includes(name);
      for (const [d, col] of w.days.entries()) {
        for (const e of col.events) {
          const where = `${name} day ${d}: ${e.title}`;
          // Each rule is stated **once** and then either asserted or counted,
          // rather than written out in both branches: two copies of the same
          // predicate is exactly the drift this file exists to catch, and a
          // skip branch that had quietly stopped agreeing with the assert
          // branch would make the exhaustive-offenders check below meaningless
          // while still passing.

          // `timed_column` buckets by the column *containing* the start
          // (commands.rs:234-236, via `column_for` at :219-224)…
          const startsInside = e.start_ms < col.end_ms;
          // …with one exception, and it is a real one: the `or_else` clause at
          // commands.rs:236 clamps an event that began before the window and
          // runs into it into column 0 rather than dropping it, because
          // dropping it made the event vanish. There and only there may a start
          // sit before its column. Everywhere else a start before the column
          // means the event is in the wrong day.
          const belongsHere = d === 0
            ? e.start_ms >= col.start_ms || e.end_ms > col.start_ms
            : e.start_ms >= col.start_ms;

          if (decoupled) {
            if (!startsInside || !belongsHere) offenders.add(name);
          } else {
            expect(startsInside, `${where} starts at ${e.start_ms}, at or after the next day (${col.end_ms})`)
              .toBe(true);
            expect(belongsHere, d === 0
              ? `${where} neither starts in Monday's column nor reaches it`
              : `${where} starts at ${e.start_ms}, before its own column (${col.start_ms})`)
              .toBe(true);
            asserted++;
          }
          checked++;
        }
      }
    }
    // The skip list is exhaustive, not permissive — see `DECOUPLED`. This also
    // fails on an empty sweep, since an empty `weeks` finds no offenders while
    // `DECOUPLED` still names three.
    expect([...offenders].sort()).toEqual([...DECOUPLED].sort());
    // Twenty today: three in `populated`, four in `popover`, two in
    // `popover-two-occurrences`, two in `single-day-overlap`, three in
    // `filmstrip`, four in `app:writable`, one in each `labelled` week.
    expect(checked).toBeGreaterThan(0);
    // …of which ten reached a real assertion. `checked` alone does not say
    // that: a world where every fixture had drifted into `DECOUPLED` would
    // still count twenty events while asserting nothing about any of them.
    // This is the empty-sweep trap one level down, inside the skip branch.
    expect(asserted).toBeGreaterThan(0);
  });

  test('every placed box is geometry lay_out_day could have produced', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15); // see above
    let checked = 0;
    for (const [name, w] of weeks) {
      for (const [d, col] of w.days.entries()) {
        // `lay_out_day` maps over its input one-for-one
        // (layout.rs:105-115) and returns an empty `Vec` for an empty input
        // (layout.rs:43-45), and `assemble_days` builds that input from the
        // column's own events (commands.rs:347-353). So the two arrays are the
        // same length, and `Placed { idx, .. }` is the `enumerate` index
        // (layout.rs:108) — `placed[i].idx` is `i`, always, never a
        // hand-chosen number. `WeekGrid` reads `events[p.idx]` to title a
        // block, so a fixture that got this wrong would draw the right box
        // with the wrong event in it.
        expect(col.placed.length, `${name} day ${d} has ${col.events.length} events and ${col.placed.length} boxes`)
          .toBe(col.events.length);

        for (const [i, p] of col.placed.entries()) {
          const where = `${name} day ${d} box ${i}`;
          expect(p.idx, `${where} indexes event ${p.idx}`).toBe(i);

          // `columns_of` starts at 1 and is only ever set to a cluster's
          // `active.len()` (layout.rs:54, 98-101), and a column is a position
          // within that same `active` (layout.rs:81-92). So a box is always
          // inside its own cluster's width — a `column` at or past `columns`
          // renders a block off the right-hand edge of its day.
          expect(p.column, `${where} is at column ${p.column}`).toBeGreaterThanOrEqual(0);
          expect(p.column, `${where} is column ${p.column} of ${p.columns}`)
            .toBeLessThan(p.columns);

          // `top` is `(start - day_start_ms) / span` with `start` clamped into
          // the window (layout.rs:109, 111), so it is a fraction in [0, 1].
          expect(p.top, `${where} tops at ${p.top}`).toBeGreaterThanOrEqual(0);

          // `height` is `((end - start) / span).max(MIN_HEIGHT)` with `end`
          // clamped to `[start, day_end_ms]` (layout.rs:110, 112), so it is at
          // least the floor.
          expect(p.height, `${where} is ${p.height} high`).toBeGreaterThanOrEqual(MIN_HEIGHT);

          // **Three bounds a first draft of this test also asserted are gone,
          // deliberately, and this note is here so they are not put back as
          // oversights.** `columns >= 1`, `top <= 1` and `height <= 1` are each
          // *strictly implied* by the assertions that remain — `columns >= 1`
          // by `0 <= column < columns`; `top <= 1` by `top + height <=
          // 1 + MIN_HEIGHT` together with `height >= MIN_HEIGHT`; `height <= 1`
          // by `top >= 0` and the strong bound below, since any height past 1
          // is past MIN_HEIGHT and so cannot skip it. No mutation can reach one
          // of the three without tripping its implicant first, which makes them
          // exactly the shape this file has already shipped once and had to
          // find by hand: a live-looking assertion no failure can reach.

          // The unclamped fraction can never take `top + height` past 1 —
          // `end` is clamped to the window. The `.max(MIN_HEIGHT)` can, and
          // that is the one shape where a box legitimately runs past the
          // bottom of its day: an event shorter than 0.4% of the window,
          // sitting at the very end of it. So `1` is **not** the bound the
          // Rust guarantees, `1 + MIN_HEIGHT` is…
          expect(p.top + p.height, `${where} ends at ${p.top + p.height} of its day`)
            .toBeLessThanOrEqual(1 + MIN_HEIGHT);
          // …and the strong bound holds for every box that is not on the
          // floor, which today is all of them. Written as a conditional rather
          // than dropped, so the day a min-height box lands here it is exempted
          // by the same rule `lay_out_day` exempts it by, not by loosening the
          // bound for everything else.
          if (p.height > MIN_HEIGHT) {
            expect(p.top + p.height, `${where} ends at ${p.top + p.height} of its day`)
              .toBeLessThanOrEqual(1);
          }
          checked++;
        }
      }
    }
    // Twenty today, one per timed event — without this a fixture set with no
    // timed events anywhere passes every assertion above by iterating nothing,
    // and the `placed.length` check above is satisfied by `0 === 0`.
    expect(checked).toBeGreaterThan(0);
  });

  test('every placed box says the same thing as the event it belongs to', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15); // see above
    let checked = 0;
    let asserted = 0;
    const offenders = new Set<string>();
    for (const [name, w] of weeks) {
      const decoupled = DECOUPLED.includes(name);
      for (const [d, col] of w.days.entries()) {
        const span = Math.max(1, col.end_ms - col.start_ms);
        for (const [i, p] of col.placed.entries()) {
          const e = col.events[p.idx];
          if (e === undefined) continue; // already failed in the test above
          // `lay_out_day`'s own arithmetic, restated (layout.rs:104, 109-112).
          const start = clamp(e.start_ms, col.start_ms, col.end_ms);
          const end = clamp(e.end_ms, start, col.end_ms);
          const top = (start - col.start_ms) / span;
          const height = Math.max(MIN_HEIGHT, (end - start) / span);

          if (decoupled) {
            if (!near(p.top, top) || !near(p.height, height)) offenders.add(name);
          } else {
            // **`toBeCloseTo`, not `toBe`, and the reason is not tolerance for
            // its own sake.** `Placed.top`/`height` are `f32` in Rust and
            // arrive as a TypeScript `number`, which is an `f64`. A 30-minute
            // event in a 24-hour day is `1800000 / 86400000` — not a binary
            // fraction — so the `f32` the backend rounds it to, widened, is a
            // *different* double from the one the line above computes.
            // Everything in `weeks` today is hand-written in `f64` and would
            // survive `toBe`; the first populated golden file would not, and
            // this test has to still be true then. The bounds checks in the
            // test above are exact comparisons and stay exact — a bound is not
            // a recomputation.
            expect(p.top, `${name} day ${d} box ${i}: ${e.title} is drawn at ${p.top}, not ${top}`)
              .toBeCloseTo(top, FRAC_PRECISION);
            expect(p.height, `${name} day ${d} box ${i}: ${e.title} is ${p.height} high, not ${height}`)
              .toBeCloseTo(height, FRAC_PRECISION);
            asserted++;
          }
          checked++;
        }
      }
    }
    // Exhaustive, not permissive — and an empty sweep fails here too. See the
    // containment test above and `DECOUPLED` itself.
    expect([...offenders].sort()).toEqual([...DECOUPLED].sort());
    // Twenty today, the same boxes the test above measures…
    expect(checked).toBeGreaterThan(0);
    // …of which ten were really recomputed and ten skipped as decoupled.
    // See the containment test for why the second floor is not redundant.
    expect(asserted).toBeGreaterThan(0);
  });

  test('every all-day chip is a lane pack_lanes could have packed', () => {
    expect(weeks.length).toBeGreaterThanOrEqual(15); // see above
    let checked = 0;
    for (const [name, w] of weeks) {
      const n = w.days.length;
      const seen = new Set<number>();
      for (const l of w.all_day) {
        const where = `${name} chip ${l.idx}`;
        // `Segment { idx: all_day_events.len(), .. }` is pushed alongside the
        // event it names (commands.rs:334), so every lane indexes a real entry,
        // and no two segments share an index.
        expect(w.all_day_events[l.idx], `${where} indexes nothing`).toBeDefined();
        expect(seen.has(l.idx), `${where} is packed twice`).toBe(false);
        seen.add(l.idx);

        // `pack_lanes(&segments, n as u16, 2)` (commands.rs:342) — clipped to
        // `[0, n - 1]` (lanes.rs:45-46) and filtered to `start_col <= end_col`
        // (lanes.rs:41), so a backwards or off-row chip cannot survive it.
        expect(l.start_col, `${where} starts left of the week`).toBeGreaterThanOrEqual(0);
        expect(l.end_col, `${where} ends right of the week`).toBeLessThanOrEqual(n - 1);
        expect(l.end_col, `${where} ends before it starts`).toBeGreaterThanOrEqual(l.start_col);
        // `pack_lanes(&segments, n, ALL_DAY_LANES_MAX)` (commands.rs) — the
        // payload positions up to 64 rows since 2026-09-04, and the *band*
        // draws four of them with the rest behind an expandable "+N more".
        // So a fifth lane here is no longer a row nothing can fill; a
        // sixty-fifth still is.
        expect(l.lane, `${where} is in a lane the packer cannot produce`).toBeLessThan(64);
        expect(l.lane, `${where} is in lane ${l.lane}`).toBeGreaterThanOrEqual(0);

        // A continuation flag is set by the clip and *only* by the clip:
        // `cont_left: s.start_col < 0` beside `start_col: s.start_col.max(0)`,
        // and `cont_right: s.end_col > last` beside `end_col:
        // s.end_col.min(last)` (lanes.rs:45-48). So a chip that says it came
        // from last week must begin in column 0, and one that says it runs into
        // next week must end in the last column. A flat edge drawn anywhere
        // else is a chip claiming a neighbour it does not touch.
        if (l.cont_left) expect(l.start_col, `${where} continues left from column ${l.start_col}`).toBe(0);
        if (l.cont_right) expect(l.end_col, `${where} continues right from column ${l.end_col}`).toBe(n - 1);
        checked++;
      }

      // Two chips in one lane may not overlap — `pack_lanes` only reuses a lane
      // whose occupants all clear the incoming span (lanes.rs:64-66). This is
      // the invariant the Rust suite pins directly in
      // `no_two_segments_share_a_lane_while_overlapping`.
      for (const a of w.all_day) {
        for (const b of w.all_day) {
          if (a === b || a.lane !== b.lane) continue;
          expect(
            a.end_col < b.start_col || b.end_col < a.start_col,
            `${name}: chips ${a.idx} and ${b.idx} overlap in lane ${a.lane}`,
          ).toBe(true);
        }
      }

      // `placed.sort_by_key(|l| (l.lane, l.start_col))` (lanes.rs:83) — the
      // band renders the list as given.
      const order = w.all_day.map((l) => [l.lane, l.start_col]);
      expect(order, `${name}'s chips are not in lane order`)
        .toEqual([...order].sort((x, y) => x[0] - y[0] || x[1] - y[1]));

      // `overflow` holds the indices that did not fit, sorted (lanes.rs:79, 84)
      // — and a segment is placed *or* overflowed, never both (lanes.rs:70's
      // `continue 'next` against :79).
      //
      // This loop used to be empty for every fixture, and the comment here
      // said so: the one payload that overflowed was `AllDayBand`'s own props
      // fixture, excused as "a different shape, not a `WeekPayload`, and out of
      // this sweep" — while carrying `overflow: [2, 3]` against two events, so
      // both indices named nothing. It is swept now (`bandAsWeek`, and the
      // `band:*` entries in the table above), and it is the fixture that makes
      // these two assertions run at all.
      expect(w.overflow, `${name}'s overflow is not sorted`).toEqual([...w.overflow].sort((a, b) => a - b));
      for (const i of w.overflow) {
        expect(w.all_day_events[i], `${name} overflow ${i} indexes nothing`).toBeDefined();
        expect(seen.has(i), `${name} overflow ${i} is also packed`).toBe(false);
      }
    }
    // Eleven today: two in `populated`, two in `popover-all-day`, one in
    // `filmstrip`, one in `app:writable`, one in `app:cross-zone`, and two each
    // in `band:populated`, `band:three-days`, and `band:overflow`.
    expect(checked).toBeGreaterThan(0);
  });
});
