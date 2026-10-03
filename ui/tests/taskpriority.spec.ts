import { test, expect } from '@playwright/test';
import { PRIORITY_CHOICES, priorityLabel, priorityOption, sortTasks, type Task } from '../src/lib/tasks';

/**
 * The priority rules behind the tasks pane, kept separate from the pixels:
 * the read bands (RFC 5545 §3.8.1.9), the two within-group orders, and the
 * level choices the editor offers. A rendered row cannot tell a "4" from a
 * "5" apart, and the ordering is exactly the kind of thing a refactor moves
 * without noticing.
 */
const task = (over: Partial<Task> = {}): Task => ({
  id: 1, calendarId: 1, summary: 'x', notes: null, dueMs: null, dueAllDay: true,
  completed: false, completedMs: null, calendar: 'Work', color: null, priority: 0, canWrite: true,
  ...over,
});

test.describe('the level a stored priority reads as', () => {
  test('follows the RFC bands, and 0 is no level', () => {
    expect(priorityLabel(0)).toBeNull();
    expect(priorityLabel(1)).toBe('High');
    expect(priorityLabel(4)).toBe('High');
    expect(priorityLabel(5)).toBe('Medium');
    expect(priorityLabel(6)).toBe('Low');
    expect(priorityLabel(9)).toBe('Low');
    expect(priorityLabel(10)).toBeNull();
  });

  test('offers exactly the four levels, none first', () => {
    expect(PRIORITY_CHOICES.map((c) => c.value)).toEqual([0, 1, 5, 9]);
    expect(PRIORITY_CHOICES.map((c) => c.label)).toEqual(['None', 'High', 'Medium', 'Low']);
  });

  test('the editor shows the band for a non-canonical value', () => {
    expect(priorityOption(0)).toBe(0);
    expect(priorityOption(1)).toBe(1);
    expect(priorityOption(4)).toBe(1);
    expect(priorityOption(5)).toBe(5);
    expect(priorityOption(7)).toBe(9);
    expect(priorityOption(9)).toBe(9);
  });
});

test.describe('ordering tasks within a group', () => {
  // Two dated, one undated, one with no priority: every tie-break in play.
  const a = task({ id: 1, summary: 'A', dueMs: 2000, priority: 5 });
  const b = task({ id: 2, summary: 'B', dueMs: 1000, priority: 9 });
  const c = task({ id: 3, summary: 'C', dueMs: null, priority: 1 });
  const d = task({ id: 4, summary: 'D', dueMs: 3000, priority: 0 });

  test('by date: due first, then priority, undated last', () => {
    expect(sortTasks([a, b, c, d], 'date').map((t) => t.summary)).toEqual(['B', 'A', 'D', 'C']);
  });

  test('by priority: highest first, none and undated last', () => {
    expect(sortTasks([a, b, c, d], 'priority').map((t) => t.summary)).toEqual(['C', 'A', 'B', 'D']);
  });

  test('never mutates the rows it was handed', () => {
    const rows = [a, b];
    sortTasks(rows, 'priority');
    expect(rows.map((t) => t.summary)).toEqual(['A', 'B']);
  });
});
