import { formatDate, type DateFormat } from './datefmt';
// When a task is due, in the words the sidebar uses — and which group it
// falls in. Pure, and separate from the panel, for `eventform.ts`'s reason:
// this is a table of inputs to outputs and wants testing as one. A spec that
// could only read a rendered row back could not tell "tomorrow" from "in one
// day" when the clock happens to make them the same.

import type { Task } from './tasks';
import { formatClock } from './timefmt';
import { startOfWeek, type WeekStartDay } from './weekstart';
import type { TimeFormat } from './timefmt';

/** The groups the "By when" list shows, in the order it shows them. */
export type When = 'overdue' | 'today' | 'tomorrow' | 'week' | 'later' | 'none';

export const WHEN_ORDER: When[] = ['overdue', 'today', 'tomorrow', 'week', 'later', 'none'];

export const WHEN_LABEL: Record<When, string> = {
  overdue: 'Overdue',
  today: 'Today',
  tomorrow: 'Tomorrow',
  week: 'This week',
  later: 'Later',
  none: 'No date',
};

/** Midnight at the start of `ms`, in the viewer's own zone — which is the
 *  display zone, fixed at launch, the same one the grid's columns are built
 *  in. Dates are a civil question: a task due today is due today whatever
 *  hour it is now. */
const dayStart = (ms: number): number => {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

/** Whole days from today to the task's due date, negative for the past. */
export const daysAway = (dueMs: number, nowMs: number): number =>
  Math.round((dayStart(dueMs) - dayStart(nowMs)) / 86_400_000);

/**
 * Which group a task belongs to.
 *
 * A **completed** task is never overdue, whatever its date: the point of
 * the overdue group is what still needs doing, and a finished task shouting
 * in red is how a list stops being read. Its own section holds it instead.
 */
export function whenOf(task: Task, nowMs: number, weekStart: WeekStartDay = 'monday'): When {
  if (task.dueMs === null) return 'none';
  const away = daysAway(task.dueMs, nowMs);
  if (away < 0) return task.completed ? 'today' : 'overdue';
  if (away === 0) return 'today';
  if (away === 1) return 'tomorrow';
  // "This week" is the days still ahead in the week the calendar draws
  // rather than a rolling seven: a task due Friday stops being "this week"
  // on Saturday, which is what a person means by it. The week is the one
  // the setting starts — the old arithmetic assumed Monday and, on a
  // Sunday, counted the whole next week as this one (found 2026-09-17).
  const next = new Date(startOfWeek(new Date(nowMs), weekStart));
  next.setDate(next.getDate() + 7);
  return dayStart(task.dueMs) < next.getTime() ? 'week' : 'later';
}

/** The short due label on a row: glanceable, never a timestamp.
 *
 *  The hour goes through the app's own clock format, not the locale's, for
 *  the reason every other time in the window does: the setting is the
 *  user's answer and a task reading 6:00 PM beside a grid reading 18:00
 *  would be the app disagreeing with itself. */
export function dueLabel(task: Task, nowMs: number, format: TimeFormat = '24h', dateFormat: DateFormat = 'locale'): string {
  if (task.dueMs === null) return '';
  const away = daysAway(task.dueMs, nowMs);
  const time = task.dueAllDay ? '' : formatClock(task.dueMs, format);
  if (away === 0) return time || 'Today';
  if (away === 1) return time ? `Tomorrow ${time}` : 'Tomorrow';
  // Composed rather than asked of the locale: given only a weekday and a
  // day number, engines order them differently ("Thu 10" against "10 Thu"),
  // and a column of dates wants one order. The weekday name is still the
  // locale's.
  const due = new Date(task.dueMs);
  const day = dateFormat !== 'locale' ? formatDate(task.dueMs, dateFormat) : `${due.toLocaleDateString(undefined, { weekday: 'short' })} ${due.getDate()}`;
  return time ? `${day} ${time}` : day;
}

/** Midnight at the start of today, for the Done list's cutoff: what was
 *  completed since is "today", and the history asks for what came before. */
export const todayStartMs = (nowMs: number): number => dayStart(nowMs);

/** Whether a task was completed today. One with no completion stamp cannot
 *  be dated, so it is never today's — it waits in the history instead. */
export const doneToday = (task: Task, nowMs: number): boolean =>
  task.completed && task.completedMs !== null && task.completedMs >= dayStart(nowMs);

/** When a task in the Done history was completed, as the row says it:
 *  "Yesterday", then the date — with its year only once it is not this
 *  year's. Empty for a task with no stamp, which is better than a guess. */
export function doneLabel(task: Task, nowMs: number, dateFormat: DateFormat = 'locale'): string {
  if (task.completedMs === null) return '';
  const away = daysAway(task.completedMs, nowMs);
  if (away === 0) return 'Today';
  if (away === -1) return 'Yesterday';
  if (dateFormat !== 'locale') return formatDate(task.completedMs, dateFormat);
  const at = new Date(task.completedMs);
  const sameYear = at.getFullYear() === new Date(nowMs).getFullYear();
  return at.toLocaleDateString(undefined, sameYear
    ? { month: 'short', day: 'numeric' }
    : { month: 'short', day: 'numeric', year: 'numeric' });
}

/** Whether the row's date should be shown as a warning. Completed tasks are
 *  excluded for `whenOf`'s reason, and so are cancelled ones (#222): nothing
 *  is left to do. */
export const isOverdue = (task: Task, nowMs: number): boolean =>
  task.dueMs !== null && !task.completed && task.status !== 'cancelled' && daysAway(task.dueMs, nowMs) < 0;

/** The quick answers the date row offers, as the instants they mean.
 *
 *  Each lands at the start of its day, so a task set to "Tomorrow" is due
 *  tomorrow rather than tomorrow-at-whatever-time-it-is-now — the caller
 *  pairs them with `dueAllDay: true`. "Next week" is the coming Monday,
 *  which is what people mean by it far more often than "in seven days". */
export function quickDue(kind: 'today' | 'tomorrow' | 'nextWeek', nowMs: number): number {
  // Civil days, not multiples of 24 hours: the day the clocks go back is 25
  // hours long, and "tomorrow" pressed on it landed on the same Sunday
  // (found 2026-09-17).
  const d = new Date(nowMs);
  const on = (days: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + days).getTime();
  if (kind === 'today') return on(0);
  if (kind === 'tomorrow') return on(1);
  return on((8 - d.getDay()) % 7 || 7); // 0 = Sunday
}

/** An `<input type="date">` value for a due date, in the viewer's zone. */
export function dateInputValue(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** An `<input type="time">` value, or empty for an all-day due. */
export function timeInputValue(task: Task): string {
  if (task.dueMs === null || task.dueAllDay) return '';
  const d = new Date(task.dueMs);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * The instant a date field and an optional time field name together.
 *
 * An empty date is no due date at all, whatever the time says — a time
 * without a day is not a due date, and silently keeping the old day would
 * be the app deciding something the user did not.
 */
export function dueFromInputs(date: string, time: string): { ms: number | null; allDay: boolean } {
  if (!date) return { ms: null, allDay: true };
  const [y, m, d] = date.split('-').map(Number);
  if (!y || !m || !d) return { ms: null, allDay: true };
  if (!time) return { ms: new Date(y, m - 1, d).getTime(), allDay: true };
  const [hh, mm] = time.split(':').map(Number);
  return { ms: new Date(y, m - 1, d, hh || 0, mm || 0).getTime(), allDay: false };
}
