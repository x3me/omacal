// A task carried from the Tasks pane onto the calendar (#115).
//
// Dropped on an hour, the task becomes due then; dropped on a day's TASKS
// row, it becomes due that day with no hour. It stays a task either way:
// @xmha97 asked for an event only because a timed task did not seem to exist,
// and Google Calendar's own pane-to-grid drag schedules the task the same way.
//
// The pane and the grid are siblings, so the carry lives here rather than in
// either: the pane starts it and writes the drop, the grid answers "what is
// under this point" and draws where the task would land. Pointer events, not
// HTML5 drag and drop, which Tauri's own file-drop handling (the `.ics`
// import) intercepts on some platforms.

/** Where a carried task would land. */
export type TaskLanding = {
  /** The day's start, which is how the grid knows which column to light. */
  dayStartMs: number;
  dueMs: number;
  /** Dropped on the TASKS row: a date, not an hour. */
  allDay: boolean;
};

/** `priority` rides along so the chip the grid draws where it would land
 *  carries the marks of the chip it will become (#190). */
type Carry = { id: number; summary: string; color: string | null; priority: number; x: number; y: number };

let carry = $state<Carry | null>(null);
let landing = $state<TaskLanding | null>(null);
let resolve: ((x: number, y: number) => TaskLanding | null) | null = null;

/** The task in flight, if any, and where the pointer is. */
export const carried = () => carry;

/** Where it would land if dropped now, or `null` over nowhere it can go. */
export const taskLanding = () => landing;

/** The grid registers what answers "what is under this point". One grid is
 *  on screen at a time; the answer to the call removes it again. */
export function registerTaskLanding(fn: (x: number, y: number) => TaskLanding | null): () => void {
  resolve = fn;
  return () => {
    if (resolve === fn) resolve = null;
  };
}

export function beginCarry(task: Omit<Carry, 'x' | 'y'>, x: number, y: number) {
  carry = { ...task, x, y };
  landing = resolve?.(x, y) ?? null;
}

export function moveCarry(x: number, y: number) {
  if (!carry) return;
  carry = { ...carry, x, y };
  landing = resolve?.(x, y) ?? null;
}

/** Ends the carry and answers with where it landed — `null` for a drop on
 *  nowhere, or a carry cancelled with Escape. */
export function endCarry(commit: boolean): (TaskLanding & { id: number }) | null {
  const at = commit && carry && landing ? { ...landing, id: carry.id } : null;
  carry = null;
  landing = null;
  return at;
}
