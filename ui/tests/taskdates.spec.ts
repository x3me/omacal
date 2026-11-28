import { test, expect } from '@playwright/test';
import {
  doneLabel, doneToday, dueFromInputs, dueLabel, isOverdue, quickDue, timeInputValue, todayStartMs,
  whenOf,
} from '../src/lib/taskdates';
import type { Task } from '../src/lib/tasks';

/**
 * The date rules behind the tasks sidebar, as a table of inputs to outputs.
 * Read back off a rendered row these would be untellable from each other:
 * "tomorrow" and "in one day" agree on most clocks and disagree on the ones
 * that matter.
 */
const task = (over: Partial<Task> = {}): Task => ({
  id: 1, calendarId: 1, summary: 'x', notes: null, dueMs: null, dueAllDay: true,
  completed: false, status: 'needs-action', completedMs: null, calendar: 'Work', color: null, priority: 0, canWrite: true, ...over,
});

// A Monday, mid-morning: 2026-09-07 10:00 local.
const MON = new Date(2026, 8, 7, 10, 0).getTime();
const DAY = 86_400_000;
const at = (days: number, hour = 9) =>
  new Date(2026, 8, 7 + days, hour, 0).getTime();

test.describe('when a task is due', () => {
  test('groups by the day, not by the hour', () => {
    expect(whenOf(task(), MON)).toBe('none');
    expect(whenOf(task({ dueMs: at(-1) }), MON)).toBe('overdue');
    // Earlier today is still today: a task due at 09:00 is not overdue at 10.
    expect(whenOf(task({ dueMs: at(0, 9) }), MON)).toBe('today');
    expect(whenOf(task({ dueMs: at(0, 23) }), MON)).toBe('today');
    expect(whenOf(task({ dueMs: at(1) }), MON)).toBe('tomorrow');
    expect(whenOf(task({ dueMs: at(4) }), MON)).toBe('week');
    // Sunday closes the week; the Monday after is Later.
    expect(whenOf(task({ dueMs: at(6) }), MON)).toBe('week');
    expect(whenOf(task({ dueMs: at(7) }), MON)).toBe('later');
  });

  /** Found 2026-09-17: on a Sunday the old arithmetic called the whole next
   *  week "this week", and it assumed weeks start on Monday. */
  test('this week ends where the calendar\'s week ends, whichever day starts it', () => {
    const sun = at(6, 10); // Sun 13 Sep
    // Monday weeks: Sunday is the last day, so Tuesday is next week's.
    expect(whenOf(task({ dueMs: at(8) }), sun)).toBe('later');
    expect(whenOf(task({ dueMs: at(12) }), sun)).toBe('later');
    // Sunday weeks: the same Sunday starts one, and Saturday closes it.
    expect(whenOf(task({ dueMs: at(8) }), sun, 'sunday')).toBe('week');
    expect(whenOf(task({ dueMs: at(12) }), sun, 'sunday')).toBe('week');
    expect(whenOf(task({ dueMs: at(13) }), sun, 'sunday')).toBe('later');
    // Saturday weeks, from Monday: Friday closes it.
    expect(whenOf(task({ dueMs: at(4) }), MON, 'saturday')).toBe('week');
    expect(whenOf(task({ dueMs: at(5) }), MON, 'saturday')).toBe('later');
  });

  /** A finished task never shouts in red, whatever its date: the overdue
   *  group is what still needs doing. */
  test('a completed task is never overdue', () => {
    const done = task({ dueMs: at(-3), completed: true });
    expect(whenOf(done, MON)).not.toBe('overdue');
    expect(isOverdue(done, MON)).toBe(false);
    expect(isOverdue(task({ dueMs: at(-3) }), MON)).toBe(true);
  });

  /** #222: a cancelled task has nothing left to do, so it is not late. */
  test('a cancelled task is never overdue', () => {
    expect(isOverdue(task({ dueMs: at(-3), status: 'cancelled' }), MON)).toBe(false);
    expect(isOverdue(task({ dueMs: at(-3), status: 'in-process' }), MON)).toBe(true);
  });

  test('the row label says the day, and the time only when there is one', () => {
    expect(dueLabel(task({ dueMs: at(0) }), MON)).toBe('Today');
    expect(dueLabel(task({ dueMs: at(1) }), MON)).toBe('Tomorrow');
    expect(dueLabel(task({ dueMs: at(3) }), MON)).toBe('Thu 10');
    expect(dueLabel(task(), MON)).toBe('');
    // The hour follows the app's clock setting, not the locale's.
    const timed = task({ dueMs: new Date(2026, 8, 7, 18, 0).getTime(), dueAllDay: false });
    expect(dueLabel(timed, MON)).toBe('18:00');
    expect(dueLabel(timed, MON, '12h')).toBe('6:00 PM');
    const thu = task({ dueMs: new Date(2026, 8, 10, 18, 0).getTime(), dueAllDay: false });
    expect(dueLabel(thu, MON)).toBe('Thu 10 18:00');
  });
});

test.describe('setting a due date', () => {
  test('the quick answers land at the start of their day', () => {
    expect(quickDue('today', MON)).toBe(new Date(2026, 8, 7).getTime());
    expect(quickDue('tomorrow', MON)).toBe(new Date(2026, 8, 8).getTime());
    // Next week is the coming Monday, which is what people mean by it.
    expect(quickDue('nextWeek', MON)).toBe(new Date(2026, 8, 14).getTime());
    // From a Friday it is still the next Monday, not seven days on.
    expect(quickDue('nextWeek', MON + 4 * DAY)).toBe(new Date(2026, 8, 14).getTime());
  });

  /** A time with no day is not a due date, and keeping the old day would be
   *  the app deciding something the user did not. */
  test('an empty date clears the due date whatever the time says', () => {
    expect(dueFromInputs('', '18:00')).toEqual({ ms: null, allDay: true });
    expect(dueFromInputs('', '')).toEqual({ ms: null, allDay: true });
  });

  test('a date alone is all-day; a date and a time is an instant', () => {
    expect(dueFromInputs('2026-09-10', '')).toEqual({
      ms: new Date(2026, 8, 10).getTime(), allDay: true,
    });
    expect(dueFromInputs('2026-09-10', '18:30')).toEqual({
      ms: new Date(2026, 8, 10, 18, 30).getTime(), allDay: false,
    });
  });

  test('the time field is empty for an all-day task and filled for a timed one', () => {
    expect(timeInputValue(task({ dueMs: at(0), dueAllDay: true }))).toBe('');
    expect(timeInputValue(task({ dueMs: new Date(2026, 8, 7, 18, 5).getTime(), dueAllDay: false })))
      .toBe('18:05');
    expect(timeInputValue(task())).toBe('');
  });
});

/**
 * The Done list (2026-09-17): what counts as done *today*, and how an
 * earlier one says when it was done.
 */
test.describe('done', () => {
  const done = (completedMs: number | null) => task({ completed: true, completedMs });

  test('today starts at local midnight', () => {
    expect(todayStartMs(MON)).toBe(new Date(2026, 8, 7).getTime());
  });

  test('a task is done today when it was completed since midnight', () => {
    expect(doneToday(done(new Date(2026, 8, 7, 0, 0).getTime()), MON)).toBe(true);
    expect(doneToday(done(MON - 60_000), MON)).toBe(true);
    // A minute before midnight is yesterday's.
    expect(doneToday(done(new Date(2026, 8, 6, 23, 59).getTime()), MON)).toBe(false);
    // No stamp cannot be dated, so it is never today's.
    expect(doneToday(done(null), MON)).toBe(false);
    // An open task is not done at all, whatever a stale stamp says.
    expect(doneToday(task({ completed: false, status: 'needs-action', completedMs: MON }), MON)).toBe(false);
  });

  test('an earlier one says yesterday, then the date, with a year only when it differs', () => {
    expect(doneLabel(done(MON - 60_000), MON)).toBe('Today');
    expect(doneLabel(done(MON - DAY), MON)).toBe('Yesterday');
    const lastWeek = new Date(2026, 8, 1, 9).getTime();
    expect(doneLabel(done(lastWeek), MON)).toBe(
      new Date(lastWeek).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }));
    const lastYear = new Date(2025, 11, 20, 9).getTime();
    expect(doneLabel(done(lastYear), MON)).toBe(
      new Date(lastYear).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }));
    expect(doneLabel(done(lastYear), MON, 'iso')).toBe('2025-12-20');
    expect(doneLabel(done(null), MON)).toBe('');
  });
});

/** The quick answers across a clock change, in a zone that has one. The
 *  suite's own zone is UTC, so this runs in the page, as `drag.spec.ts`'s
 *  daylight-saving cases do. */
test.describe('quick due dates on the day the clocks go back', () => {
  test.use({ timezoneId: 'Europe/Sofia' });

  test('tomorrow and next week are civil days, not multiples of 24 hours', async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=eventform&f=none');
    const got = await page.evaluate(() => {
      const td = (window as any).__taskdates;
      const day = (ms: number) => { const d = new Date(ms); return [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours()]; };
      // Sun 25 Oct 2026 is 25 hours long in Sofia; Thu 22 Oct's next Monday is past it.
      const sunday = new Date(2026, 9, 25, 10).getTime();
      const thursday = new Date(2026, 9, 22, 10).getTime();
      return {
        tomorrow: day(td.quickDue('tomorrow', sunday)),
        nextWeek: day(td.quickDue('nextWeek', thursday)),
        today: day(td.quickDue('today', sunday)),
      };
    });
    expect(got.tomorrow).toEqual([2026, 10, 26, 0]);
    expect(got.nextWeek).toEqual([2026, 10, 26, 0]);
    expect(got.today).toEqual([2026, 10, 25, 0]);
  });
});
