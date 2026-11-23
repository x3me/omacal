// A task as the week grid draws it, and how a week's worth are found.
//
// The grid takes chips rather than `Task`s so it never has to know what a
// task list is, only what belongs on Thursday — the same bargain the
// forecast makes with `weather.ts`.

import { dateKey } from './weather';
import type { Task } from './tasks';

export type TaskChip = {
  id: number;
  summary: string;
  color: string | null;
  completed: boolean;
  overdue: boolean;
  canWrite: boolean;
  /** The raw wire priority, 0 for none; `priorityLabel` reads it. The grid
   *  draws it as marks (#190). */
  priority: number;
  /** The due instant. A timed chip is drawn at it; a row chip only needs
   *  its day, which is the key it is filed under. */
  dueMs: number;
};

/** A week's tasks, by where on the grid each is drawn. Both maps are keyed
 *  by the ISO date of the column. */
export type WeekTasks = {
  /** The row above the all-day band: tasks due on a day, and overdue ones. */
  row: Map<string, TaskChip[]>;
  /** In the hour grid, among the meetings: tasks due at an hour. */
  timed: Map<string, TaskChip[]>;
};

function file(map: Map<string, TaskChip[]>, key: string, chip: TaskChip) {
  const at = map.get(key);
  if (at) at.push(chip);
  else map.set(key, [chip]);
}

/**
 * The tasks the grid should draw, and where.
 *
 * Three rules, all about not lying:
 *
 * - A task due **at an hour** is drawn at that hour, among the meetings
 *   (2026-09-17, macOS Calendar's shape): "by 15:00" is a time, and a row
 *   that says only "Thursday" drops half of it. A task due **on a day** has
 *   no hour to be drawn at, and keeps the row.
 * - An **overdue** task is drawn in the row on today, not on the day it was
 *   due — timed or not. Its date has passed and the column for it may not
 *   even be on screen; a task that quietly leaves the week when the week
 *   moves is one nobody does. It is marked, so the day it lands on is not
 *   read as its due date, and it is not pinned to an hour of today that was
 *   never its hour.
 * - A **completed** task is left out. The grid is what still needs doing,
 *   and the sidebar's Done section is where a finished one goes.
 */
export function taskChips(
  tasks: Task[],
  nowMs: number,
  lists: { calendarId: number; color: string | null }[] = [],
): WeekTasks {
  const today = dateKey(nowMs);
  const out: WeekTasks = { row: new Map(), timed: new Map() };
  for (const t of tasks) {
    if (t.completed || t.dueMs === null) continue;
    const overdue = dateKey(t.dueMs) < today;
    const chip: TaskChip = {
      id: t.id,
      summary: t.summary,
      color: t.color ?? lists.find((l) => l.calendarId === t.calendarId)?.color ?? null,
      completed: false,
      overdue,
      canWrite: t.canWrite,
      priority: t.priority,
      dueMs: t.dueMs,
    };
    if (overdue) file(out.row, today, chip);
    else file(t.dueAllDay ? out.row : out.timed, dateKey(t.dueMs), chip);
  }
  return out;
}
