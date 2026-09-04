import { startOfWeek } from './weekstart';
import { weekStartDay } from './weekstartstore.svelte';
import { invoke } from '@tauri-apps/api/core';

export type UiEvent = {
  id: number; title: string; location: string | null;
  start_ms: number; end_ms: number; color: string;
  response: 'accepted' | 'needsAction' | 'tentative' | 'declined';
  is_all_day: boolean;
  /** Invitees, the organizer included; `0` is a solo event, not unknown. */
  attendees: number;
  /** One instance of something that repeats — the series' own expansion or
   *  an exception overriding one occurrence of it. */
  recurring: boolean;
  /** Google's structured conference link. The list row's Join treats this
   *  and a recognised meeting URL in `location` as one fact, the same pair
   *  of places the popover reads. */
  conference: string | null;
  /** Every invitee other than you has declined, and there was at least one.
   *  The block and the list row mark it; the popover counts it. Your own
   *  "no" is not this — that is `response`, and it already strikes the
   *  block through. */
  all_guests_declined: boolean;
};

/** Opens an event's meeting link in the system browser, backend-side. The
 *  webview sends only the event's id — never the URL — for
 *  `open_latest_release`'s reason: the backend resolves what the browser is
 *  pointed at from its own store, so a compromised webview can only name an
 *  event, not choose a destination. It is also what routes the click around
 *  the AppImage environment that crashes a spawned browser (issue #1). */
export const openConference = (id: number) => invoke<void>('open_conference', { id });
export type Placed = { idx: number; column: number; columns: number; top: number; height: number };
export type Lane = {
  idx: number; lane: number; start_col: number; end_col: number;
  cont_left: boolean; cont_right: boolean;
};
/** `end_ms` is midnight on the next day, so a 23- or 25-hour DST day reports
 *  its true span rather than a nominal 24 hours. */
export type DayColumn = {
  start_ms: number; end_ms: number; events: UiEvent[]; placed: Placed[];
};
export type WeekPayload = {
  days: DayColumn[]; all_day: Lane[]; all_day_events: UiEvent[]; overflow: number[];
};

/** One day of a month grid. `in_month` is false for the leading/trailing days
 *  that belong to a neighbouring month — drawn dimmed, not hidden, so the
 *  grid stays rectangular. `timed` is complete and sorted by start; the UI
 *  decides how many lines fit and computes its own `+N more`. */
export type MonthCell = { start_ms: number; end_ms: number; in_month: boolean; timed: UiEvent[] };
/** One week-row of a month grid: seven cells plus the multi-day/all-day bars
 *  spanning them, already lane-packed and row-clipped (`bars: Lane[]`, each
 *  entry one placed segment — not a container of segments). */
export type MonthRow = {
  cells: MonthCell[]; bars: Lane[]; bar_events: UiEvent[]; bar_overflow: number[];
};
/** `rows` is always 6, even when the month fits in five, so the grid never
 *  changes height as you page through the year. */
export type MonthPayload = { rows: MonthRow[]; year: number; month: number };

/** Midnight local on the first day of the week containing `d`.
 *
 *  The day itself comes from the stored preference rather than a parameter:
 *  every caller wants the user's week, and an argument here would be one more
 *  place to pass the wrong one. The arithmetic lives in `weekstart.ts`, where
 *  a spec can reach it. */
export function weekStart(d: Date): number {
  return startOfWeek(d, weekStartDay());
}

/** `pad` on the three day-grid fetches (2026-09-03): that many days either
 *  side of the window named, in the same payload, so the grid can slide into
 *  them under a finger before the fetch for the new window lands. The window
 *  is still what the caller names; `weekwindow.ts` finds it in the payload. */
export const getWeek = (weekStartMs: number, pad = 0) =>
  invoke<WeekPayload>('get_week', { weekStartMs, pad });

/** A rolling Week-view range beginning on the supplied day. */
export const getRange = (dayStartMs: number, dayCount: 3 | 5 | 7, pad = 0) =>
  invoke<WeekPayload>('get_range', { dayStartMs, dayCount, pad });

export const getDay = (dayStartMs: number, pad = 0) =>
  invoke<WeekPayload>('get_day', { dayStartMs, pad });

export const getMonth = (year: number, month: number) =>
  invoke<MonthPayload>('get_month', { year, month });

/** One day of the year grid. `has_all_day` is set only by an all-day event —
 *  a timed meeting does not dot this view. `unsynced` marks a day outside
 *  the window the app actually keeps fetched (`synced_window` in Rust);
 *  drawn distinctly from an in-window day with nothing on it, since absence
 *  of a dot must never read as "free". */
export type YearDay = { start_ms: number; day: number; has_all_day: boolean; unsynced: boolean };
/** One month of the year grid: `lead_blanks` empty cells (Monday-first)
 *  before day 1, so every month's weekday columns line up. */
export type YearMonth = { month: number; lead_blanks: number; days: YearDay[] };
export type YearPayload = { year: number; months: YearMonth[] };

export const getYear = (year: number) =>
  invoke<YearPayload>('get_year', { year });

/** One day of the Big Year ribbon. `in_year` is false for the days a
 *  Monday-aligned 28-day row spills into the neighbouring year with — the
 *  ribbon never lines up exactly on 1 Jan or 31 Dec, so this is drawn dimmed
 *  rather than hidden, same principle as `MonthCell.in_month`. `unsynced`
 *  mirrors `YearDay.unsynced`. */
export type RibbonDay = { start_ms: number; in_year: boolean; unsynced: boolean };
/** One 28-day row of the ribbon: the days themselves, plus the all-day/
 *  multi-day spans lane-packed across them (`pills: Lane[]`, `pill_events`
 *  the events `Lane.idx` indexes into) — same shape as `MonthRow.bars`. */
export type RibbonRow = {
  days: RibbonDay[]; pills: Lane[]; pill_events: UiEvent[]; overflow: number[];
};
/** The legend under the ribbon is not part of the payload: it is the
 *  app's own calendar list, which already knows every calendar's colour and
 *  whether it is shown, and which the legend now toggles (2026-09-03). */
export type BigYearPayload = { year: number; rows: RibbonRow[] };

export const getBigYear = (year: number) =>
  invoke<BigYearPayload>('get_big_year', { year });
