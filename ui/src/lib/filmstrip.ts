// Turning a grid's payload into the list the filmstrip toggle draws.
//
// **No new data path** (filmstrip spec §6). Everything below is a rearrangement
// of `WeekPayload`/`MonthPayload` — the same objects `WeekGrid` and `MonthGrid`
// are handed — into days and rows. Nothing here fetches, and nothing here needs
// a field the grid does not already receive.
//
// Deliberately separate from `Filmstrip.svelte`, for the reason `ink.ts` gives
// for sitting beside `BigYearRibbon`: this is a table of inputs to outputs and
// wants testing as one. Which day an event lands on, and in what order, are the
// two claims the whole feature rests on, and reading them back off rendered
// markup can only ever say that *something* rendered.

import type { Lane, MonthPayload, UiEvent, WeekPayload } from './api';

/**
 * The views a list is a rendering of, and the views the toggle exists in.
 *
 * Year and Big Year are absent rather than present-and-inert (spec §2): Big
 * Year exists to be a shape you scan across a whole year, and flattening it to
 * rows is a different idea rather than a different rendering of the same one.
 * **Both the control and the `F` key read this**, so the key cannot quietly set
 * a preference in a view that offers no way to see or undo it.
 *
 * Typed against a local union rather than `ViewSwitcher`'s exported `View`:
 * that type lives in a `<script module>` block, and plain `tsc` — which is what
 * checks this file's specs — resolves a `.svelte` import through the generic
 * ambient module, default export only. Same reason `tests/fixtures.ts` declares
 * its own copy. The two are structurally checked against each other by
 * `Filmstrip`'s and `Header`'s call sites, which pass a real `View` in.
 */
export const LISTABLE_VIEWS = ['day', 'week', 'month'] as const;
export type ListableView = (typeof LISTABLE_VIEWS)[number];

/** How many timed lines a Month cell shows before folding the rest into
 *  "+N more" — three being what a narrow cell has room for before a title
 *  stops being legible. Shared with keyboard navigation, so a cursor can never
 *  name a row the grid has already folded away.
 *
 *  **Not the bar lane cap**, though both are three. That one is how many
 *  all-day *lanes* stack above the timed lines, and it comes from the payload
 *  (`commands::GRID_LANE_CAP`); this one is the timed lines below them, and
 *  lives here because the cursor needs it. */
export const MONTH_GRID_TIMED_LIMIT = 3;

/** Where down the viewport a Today request parks the current hour or row.
 *
 *  Shared by `WeekGrid` and `Filmstrip` because they are two renderings of
 *  the same period and `F` switches between them: if the two drifted, Today
 *  would park the current hour at a different height depending on which was
 *  up, which reads as jitter rather than as a setting. It lived in both
 *  components, with Filmstrip's comment saying "As in WeekGrid" — an
 *  acknowledged copy is still a copy. */
export const NOW_VIEWPORT_FRACTION = 0.45;

export function listable(view: string): view is ListableView {
  return (LISTABLE_VIEWS as readonly string[]).includes(view);
}

/** One day of a period: its exact display-zone interval, and every event on it in draw order.
 * `allDaysFrom*` retains blank days for keyboard navigation; `daysFrom*`
 * removes them for the filmstrip's ordinary rendering. `endMs` is carried
 * from the backend rather than guessed as 24 hours so navigation still knows
 * which day contains the current instant across DST and differing zones. */
export type ListDay = { startMs: number; endMs: number; events: UiEvent[] };

/**
 * The all-day events of one lane-packed row, grouped by the column they cover.
 *
 * **The lanes are the placement**, never the events' own `start_ms`. An all-day
 * event has a date, not an instant: the store holds midnight in the
 * **calendar's** zone, which for a foreign calendar is a different instant from
 * midnight in the display zone and often a different day. Rust already resolved
 * that — `commands::date_column` compares a date to a date, and the answer is
 * `Lane.start_col`/`end_col`. Bucketing `start_ms` against the day columns here
 * would re-introduce the exact defect that rule was written to end.
 *
 * A span covering several columns is listed under **each** of them, which is
 * what makes the middle day of a three-day trip a day with something on it
 * rather than one the empty-day rule skips.
 *
 * **`overflow` is not reachable from here, and that is a payload limit rather
 * than a choice.** `pack_lanes` reports an overflowed segment as a bare index
 * and discards its columns (`lanes.rs`), so the payload says *that* an event
 * did not fit and never says *where* it would have gone. The grid folds those
 * into a `+N more` that opens nothing, so a list built from the lanes alone
 * shows exactly what the grid does — but it is a gap, not a rendering decision,
 * and closing it needs `pack_lanes` to carry the columns of what it overflowed.
 */
function allDayByColumn(lanes: Lane[], events: UiEvent[], columns: number): UiEvent[][] {
  const byColumn: UiEvent[][] = Array.from({ length: columns }, () => []);
  for (const lane of lanes) {
    const ev = events[lane.idx];
    // A lane whose index names nothing is a malformed payload rather than an
    // empty day; skipped rather than crashing the whole view over one row.
    if (!ev) continue;
    for (let c = lane.start_col; c <= lane.end_col && c < columns; c++) {
      byColumn[c].push(ev);
    }
  }
  return byColumn;
}

/**
 * A day column's timed events, in the order a list reads them.
 *
 * Copied before sorting: `week.days[].events` is the caller's payload, and a
 * view has no business reordering the object its parent holds.
 *
 * **`assemble_days` does not sort these.** It pushes each occurrence in the
 * order the store rows and the expansion happen to produce, because the grid
 * places them by geometry and never reads the order at all. `assemble_month`
 * *does* sort its cells; sorting here regardless is what keeps the two views
 * from disagreeing about which of two meetings comes first.
 */
function timedInOrder(events: UiEvent[]): UiEvent[] {
  return [...events].sort((a, b) => a.start_ms - b.start_ms);
}

/** All-day first, then timed by start (spec §5). The all-day events arrive in
 *  `pack_lanes`' own lane order, so a list reads them top-to-bottom in the
 *  order the band above the grid would have drawn them. */
function rowsForDay(allDay: UiEvent[], timed: UiEvent[]): UiEvent[] {
  return [...allDay, ...timedInOrder(timed)];
}

/**
 * Day and Week, which share one payload and one assembler (`assemble_days`).
 *
 * **Empty days are skipped** (spec §3), in Week as in the other two: a rule
 * that changes per view is one nobody can predict, and the gap in a week is
 * visible from the dates themselves.
 */
export function allDaysFromWeek(week: WeekPayload): ListDay[] {
  const allDay = allDayByColumn(week.all_day, week.all_day_events, week.days.length);
  return week.days
    .map((d, i) => ({
      startMs: d.start_ms,
      endMs: d.end_ms,
      events: rowsForDay(allDay[i], d.events),
    }));
}

export function daysFromWeek(week: WeekPayload): ListDay[] {
  return allDaysFromWeek(week).filter((d) => d.events.length > 0);
}

/**
 * Month.
 *
 * All 42 cells, the out-of-month ones included, because the toggle "changes how
 * a period is drawn, not which period" (spec §1) — and the six-row grid's
 * period is the 42 days it draws, not the calendar month it is named after.
 * They are dimmed in the grid and would be almost always empty here anyway,
 * which the skip rule already handles.
 *
 * Each row is lane-packed independently by `assemble_month`, so a bar crossing
 * a row boundary is two segments and each row places its own half.
 */
export function allDaysFromMonth(month: MonthPayload): ListDay[] {
  const out: ListDay[] = [];
  for (const row of month.rows) {
    const allDay = allDayByColumn(row.bars, row.bar_events, row.cells.length);
    row.cells.forEach((cell, i) => {
      const events = rowsForDay(allDay[i], cell.timed);
      out.push({ startMs: cell.start_ms, endMs: cell.end_ms, events });
    });
  }
  return out;
}

export function daysFromMonth(month: MonthPayload): ListDay[] {
  return allDaysFromMonth(month).filter((d) => d.events.length > 0);
}
