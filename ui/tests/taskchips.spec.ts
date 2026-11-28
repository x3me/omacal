import { test, expect } from '@playwright/test';
import { taskChips } from '../src/lib/taskchips';
import type { Task } from '../src/lib/tasks';

/**
 * Which day a task is drawn on, and whether in the row or at its hour. The
 * rules are about not lying, and all invisible from a rendered grid: a task
 * with an hour is drawn at it, an overdue task moves to today's row rather
 * than off the screen, and a finished one is not drawn at all.
 */
const task = (over: Partial<Task> = {}): Task => ({
  id: 1, calendarId: 1, summary: 'x', notes: null, dueMs: null, dueAllDay: true,
  completed: false, status: 'needs-action', completedMs: null, calendar: 'Work', color: '#2dd4bf', priority: 0, canWrite: true, ...over,
});
const NOW = new Date(2026, 8, 9, 10, 0).getTime(); // Wed 9 Sep 2026
const on = (day: number, hour = 9) => new Date(2026, 8, day, hour, 0).getTime();

test('a task is drawn on the day it is due', () => {
  const { row, timed } = taskChips([task({ id: 1, dueMs: on(11) })], NOW);
  expect([...row.keys()]).toEqual(['2026-09-11']);
  expect(row.get('2026-09-11')![0]).toMatchObject({ id: 1, overdue: false, summary: 'x' });
  expect(timed.size).toBe(0);
});

/** The chip is all the grid sees of a task, so the priority marks (#190)
 *  ride on it: the raw value, which `priorityLabel` reads. */
test('a chip carries the task\'s priority', () => {
  const { row, timed } = taskChips([
    task({ id: 1, dueMs: on(11), priority: 1 }),
    task({ id: 2, dueMs: on(11, 15), dueAllDay: false, priority: 7 }),
  ], NOW);
  expect(row.get('2026-09-11')![0].priority).toBe(1);
  expect(timed.get('2026-09-11')![0].priority).toBe(7);
});

/** macOS Calendar's shape: "by 15:00" is a time, and the row would drop it. */
test('a task due at an hour is drawn at it, not in the row', () => {
  const { row, timed } = taskChips([task({ id: 1, dueMs: on(11, 15), dueAllDay: false })], NOW);
  expect(row.size).toBe(0);
  expect([...timed.keys()]).toEqual(['2026-09-11']);
  expect(timed.get('2026-09-11')![0]).toMatchObject({ id: 1, dueMs: on(11, 15) });
});

/** Today's hour that has already gone is still today's hour: the grid's now
 *  line says it has passed. Only a past *day* moves a task. */
test('a timed task due earlier today stays at its hour', () => {
  const { row, timed } = taskChips([task({ id: 1, dueMs: on(9, 8), dueAllDay: false })], NOW);
  expect(row.size).toBe(0);
  expect(timed.get('2026-09-09')![0].overdue).toBe(false);
});

/** Its own day may not be on screen, and a task that leaves the week when
 *  the week moves is one nobody does. It is marked, so the day it lands on
 *  is not read as its due date. */
test('an overdue task is drawn on today, and marked', () => {
  const { row } = taskChips([task({ id: 2, dueMs: on(4) })], NOW);
  expect([...row.keys()]).toEqual(['2026-09-09']);
  expect(row.get('2026-09-09')![0].overdue).toBe(true);
});

/** Not pinned to an hour of today that was never its hour. */
test('an overdue timed task goes to today\'s row, not to its hour', () => {
  const { row, timed } = taskChips([task({ id: 2, dueMs: on(4, 15), dueAllDay: false })], NOW);
  expect(timed.size).toBe(0);
  expect(row.get('2026-09-09')![0]).toMatchObject({ id: 2, overdue: true });
});

test('a completed task and an undated one are not drawn at all', () => {
  const map = taskChips([
    task({ id: 3, dueMs: on(11), completed: true }),
    task({ id: 4, dueMs: null }),
    // Completed *and* overdue: still not drawn, and not piled onto today.
    task({ id: 5, dueMs: on(4), completed: true }),
    task({ id: 8, dueMs: on(11, 15), dueAllDay: false, completed: true }),
  ], NOW);
  expect(map.row.size).toBe(0);
  expect(map.timed.size).toBe(0);
});

/** #222: a cancelled task is not drawn on the grid; one in progress is. */
test('a cancelled task is not drawn, and one in progress is', () => {
  const { row } = taskChips([
    task({ id: 6, dueMs: on(11), status: 'cancelled' }),
    task({ id: 7, dueMs: on(11), status: 'in-process' }),
  ], NOW);
  expect([...row.values()].flat().map((c) => c.id)).toEqual([7]);
});

test('several tasks on one day keep their order', () => {
  const { row } = taskChips([
    task({ id: 6, dueMs: on(11), summary: 'first' }),
    task({ id: 7, dueMs: on(11, 14), summary: 'second' }),
  ], NOW);
  expect(row.get('2026-09-11')!.map((c) => c.summary)).toEqual(['first', 'second']);
});

/** A task's own colour wins; without one it takes its list's, and without
 *  that the grid falls back rather than drawing nothing. */
test('a chip takes its colour from the task, else from its list', () => {
  const lists = [{ calendarId: 9, color: '#7aa2f7' }];
  expect(taskChips([task({ dueMs: on(11) })], NOW, lists).row.get('2026-09-11')![0].color).toBe('#2dd4bf');
  expect(taskChips([task({ dueMs: on(11), calendarId: 9, color: null })], NOW, lists)
    .row.get('2026-09-11')![0].color).toBe('#7aa2f7');
  expect(taskChips([task({ dueMs: on(11), calendarId: 1, color: null })], NOW, lists)
    .row.get('2026-09-11')![0].color).toBe(null);
});
