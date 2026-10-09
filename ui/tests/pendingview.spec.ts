import { test, expect } from '@playwright/test';
import type { UiEvent, WeekPayload, MonthPayload, BigYearPayload, Lane } from '../src/lib/api';
import {
  covers, locks, overlayWeek, overlayMonth, overlayBigYear, type PendingChange,
} from '../src/lib/pendingview';

const H = 3_600_000;
const DAY = 24 * H;
const MON = Date.UTC(2024, 0, 29);

const ev = (id: number, start: number, end: number, extra: Partial<UiEvent> = {}): UiEvent => ({
  id, title: `e${id}`, location: null, start_ms: start, end_ms: end, color: '#5b8def',
  response: 'accepted', is_all_day: false, attendees: 0, recurring: false, conference: null,
  all_guests_declined: false, ...extra,
});
const lane = (idx: number, lane: number, start_col: number, end_col: number): Lane =>
  ({ idx, lane, start_col, end_col, cont_left: false, cont_right: false });
const place = (i: number) => ({ idx: i, column: 0, columns: 1, top: 0, height: 0.04 });

/** Two days: Monday carries a 09:00 and a 10:00 meeting, Tuesday a 09:00. */
const week = (): WeekPayload => ({
  days: [
    { start_ms: MON, end_ms: MON + DAY, events: [ev(1, MON + 9 * H, MON + 10 * H), ev(2, MON + 10 * H, MON + 11 * H)], placed: [place(0), place(1)] },
    { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(3, MON + DAY + 9 * H, MON + DAY + 10 * H)], placed: [place(0)] },
  ],
  all_day: [lane(0, 0, 0, 0), lane(1, 0, 1, 1)],
  all_day_events: [ev(7, MON, MON + DAY, { is_all_day: true }), ev(8, MON + DAY, MON + 2 * DAY, { is_all_day: true })],
  overflow: [0, 0],
});

const move = (id: number, from: number, to: number, length = H, scope: PendingChange['scope'] = 'this'): PendingChange =>
  ({ kind: 'move', id, occurrenceStartMs: from, scope, startMs: to, endMs: to + length });
const del = (id: number, from: number, scope: PendingChange['scope'] = 'this'): PendingChange =>
  ({ kind: 'delete', id, occurrenceStartMs: from, scope });

test.describe('which occurrences a pending change speaks for', () => {
  test('this, following and all', () => {
    const c = move(1, MON + 9 * H, MON + 12 * H);
    expect(covers(c, ev(1, MON + 9 * H, 0))).toBe(true);
    expect(covers(c, ev(1, MON + DAY + 9 * H, 0))).toBe(false);
    expect(covers({ ...c, scope: 'following' }, ev(1, MON + DAY + 9 * H, 0))).toBe(true);
    expect(covers({ ...c, scope: 'following' }, ev(1, MON - DAY + 9 * H, 0))).toBe(false);
    expect(covers({ ...c, scope: 'all' }, ev(1, MON - DAY + 9 * H, 0))).toBe(true);
    expect(covers(c, ev(2, MON + 9 * H, 0))).toBe(false);
  });

  test('a block combining several calendars is never covered', () => {
    const copies: UiEvent['copies'] = [
      { id: 1, calendar_id: 1, start_ms: MON + 9 * H, end_ms: MON + 10 * H, color: '#000' },
      { id: 4, calendar_id: 2, start_ms: MON + 9 * H, end_ms: MON + 10 * H, color: '#fff' },
    ];
    expect(covers(move(1, MON + 9 * H, MON + 12 * H), ev(1, MON + 9 * H, 0, { copies }))).toBe(false);
  });

  test('a pending move locks the occurrence where it landed, not only where it was', () => {
    const c = move(1, MON + 9 * H, MON + 12 * H);
    expect(locks(c, 1, MON + 9 * H)).toBe(true);
    expect(locks(c, 1, MON + 12 * H)).toBe(true);
    expect(locks(c, 1, MON + 15 * H)).toBe(false);
    expect(locks(c, 2, MON + 12 * H)).toBe(false);
    // Moved earlier with "following": the dragged one now starts before the
    // occurrence it came from, and is still locked.
    const back = move(1, MON + 9 * H, MON + 7 * H, H, 'following');
    expect(locks(back, 1, MON + 7 * H)).toBe(true);
    expect(locks(back, 1, MON + DAY + 7 * H)).toBe(true);
  });
});

test.describe('the week, with pending changes drawn over it', () => {
  test('no changes hands back the same payload', () => {
    const w = week();
    expect(overlayWeek(w, [])).toBe(w);
  });

  test('a move leaves its day and lands, laid out, in the day it was dropped on', () => {
    const w = overlayWeek(week(), [move(1, MON + 9 * H, MON + DAY + 9 * H)]);
    expect(w.days[0].events.map((e) => e.id)).toEqual([2]);
    expect(w.days[0].placed.map((p) => p.idx)).toEqual([0]);
    const tue = w.days[1];
    expect(tue.events.map((e) => [e.id, e.start_ms, e.pending ?? false])).toEqual([
      [3, MON + DAY + 9 * H, false], [1, MON + DAY + 9 * H, true],
    ]);
    // Two meetings at the same hour: laid out side by side, not stacked.
    expect(tue.placed.map((p) => p.columns)).toEqual([2, 2]);
    expect(new Set(tue.placed.map((p) => p.column))).toEqual(new Set([0, 1]));
  });

  test('a resize keeps its start and takes its new length', () => {
    const w = overlayWeek(week(), [move(2, MON + 10 * H, MON + 10 * H, 2 * H)]);
    const moved = w.days[0].events.find((e) => e.id === 2)!;
    expect([moved.start_ms, moved.end_ms, moved.pending]).toEqual([MON + 10 * H, MON + 12 * H, true]);
  });

  test("'all' shifts every occurrence of the series by the same amount", () => {
    const w: WeekPayload = {
      ...week(),
      days: [
        { start_ms: MON, end_ms: MON + DAY, events: [ev(5, MON + 9 * H, MON + 10 * H, { recurring: true })], placed: [place(0)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(5, MON + DAY + 9 * H, MON + DAY + 10 * H, { recurring: true })], placed: [place(0)] },
      ],
    };
    const out = overlayWeek(w, [move(5, MON + 9 * H, MON + 11 * H, H, 'all')]);
    expect(out.days.map((d) => d.events.map((e) => [e.start_ms, e.pending]))).toEqual([
      [[MON + 11 * H, true]], [[MON + DAY + 11 * H, true]],
    ]);
  });

  test("'following' shifts from the dragged occurrence on, and leaves the ones before", () => {
    const w: WeekPayload = {
      ...week(),
      days: [
        { start_ms: MON, end_ms: MON + DAY, events: [ev(5, MON + 9 * H, MON + 10 * H, { recurring: true })], placed: [place(0)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(5, MON + DAY + 9 * H, MON + DAY + 10 * H, { recurring: true })], placed: [place(0)] },
      ],
    };
    const out = overlayWeek(w, [move(5, MON + DAY + 9 * H, MON + DAY + 11 * H, H, 'following')]);
    expect(out.days[0].events.map((e) => [e.start_ms, e.pending ?? false])).toEqual([[MON + 9 * H, false]]);
    expect(out.days[1].events.map((e) => [e.start_ms, e.pending])).toEqual([[MON + DAY + 11 * H, true]]);
  });

  test('a delete hides the meeting, and the rest keep valid layout indices', () => {
    const w = overlayWeek(week(), [del(1, MON + 9 * H)]);
    expect(w.days[0].events.map((e) => e.id)).toEqual([2]);
    expect(w.days[0].placed).toHaveLength(1);
    expect(w.days[0].placed[0].idx).toBe(0);
    expect(w.days[1].events.map((e) => e.id)).toEqual([3]);
  });

  test('a delete of an all-day item drops its lane and remaps the others', () => {
    const w = overlayWeek(week(), [del(7, MON)]);
    expect(w.all_day_events.map((e) => e.id)).toEqual([8]);
    expect(w.all_day).toEqual([lane(0, 0, 1, 1)]);
  });

  test('a meeting moved across midnight is drawn in both days it touches', () => {
    const w = overlayWeek(week(), [move(2, MON + 10 * H, MON + 23 * H, 2 * H)]);
    expect(w.days[0].events.some((e) => e.id === 2 && e.pending)).toBe(true);
    expect(w.days[1].events.some((e) => e.id === 2 && e.pending)).toBe(true);
  });
});

test.describe('the month, with pending changes drawn over it', () => {
  const month = (): MonthPayload => ({
    year: 2024, month: 1, lane_cap: 3,
    rows: [{
      cells: [
        { start_ms: MON, end_ms: MON + DAY, in_month: true, timed: [ev(1, MON + 9 * H, MON + 10 * H), ev(2, MON + 14 * H, MON + 15 * H)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, in_month: true, timed: [ev(3, MON + DAY + 12 * H, MON + DAY + 13 * H)] },
      ],
      bars: [lane(0, 0, 0, 1)],
      bar_events: [ev(9, MON, MON + 2 * DAY, { is_all_day: true })],
      bar_overflow: [0, 0],
    }],
  });

  test('a moved meeting leaves its cell and joins the new one in time order, flagged', () => {
    const out = overlayMonth(month(), [move(1, MON + 9 * H, MON + DAY + 8 * H)]);
    expect(out.rows[0].cells[0].timed.map((e) => e.id)).toEqual([2]);
    expect(out.rows[0].cells[1].timed.map((e) => [e.id, e.pending ?? false])).toEqual([[1, true], [3, false]]);
  });

  test('a delete removes a chip, and a deleted bar drops its lane', () => {
    const out = overlayMonth(month(), [del(2, MON + 14 * H), del(9, MON)]);
    expect(out.rows[0].cells[0].timed.map((e) => e.id)).toEqual([1]);
    expect(out.rows[0].bar_events).toEqual([]);
    expect(out.rows[0].bars).toEqual([]);
  });
});

test.describe('the big year, with pending deletes drawn over it', () => {
  test('a deleted pill disappears and the others keep valid indices', () => {
    const big: BigYearPayload = {
      year: 2024, lane_cap: 3,
      rows: [{ days: [], pills: [lane(0, 0, 0, 1), lane(1, 1, 2, 3)],
        pill_events: [ev(7, MON, MON + DAY, { is_all_day: true }), ev(8, MON, MON + DAY, { is_all_day: true })], overflow: [] }],
    };
    const out = overlayBigYear(big, [del(7, MON)]);
    expect(out.rows[0].pill_events.map((e) => e.id)).toEqual([8]);
    expect(out.rows[0].pills).toEqual([lane(0, 1, 2, 3)]);
  });
});
