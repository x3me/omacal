// The Tasks pane's within-group order, as the pane reads it.
//
// A module-level rune for `clock.svelte.ts`'s reason: the pane reads it at
// render time and no single component owns the preference. `App` seeds it from
// the stored setting; the pane's own Date/Priority toggle writes it, and
// persists through `set_setting` in the same gesture. Absent a stored value it
// is `date`, the order the pane has always used — a failed read here must not
// reorder anybody's list.
import type { TaskSort } from './tasks';

const state = $state<{ sort: TaskSort }>({ sort: 'date' });

export const taskSort = () => state.sort;

/** `App` on startup and after a settings change; the pane's toggle for the
 *  instant repaint (the write that makes it stick is the caller's). */
export const setTaskSort = (s: TaskSort) => (state.sort = s);
