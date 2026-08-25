// Every key omacal answers to, in one table.
//
// **The table is the binding and the documentation at once**, which is the
// whole reason it exists. Before this, `App` held a `KEY_VIEW` map and a
// `switch`, and nothing anywhere told the user either existed — a
// keyboard-first calendar whose keyboard was a secret. The obvious fix is a
// help sheet listing the keys; the fix that stays true is a help sheet reading
// the same array the handler dispatches from, so a key added to one cannot go
// missing from the other.
//
// The exhaustiveness is not by convention: `ShortcutId` is derived from this
// array, and `App`'s action map is typed `Record<ShortcutId, () => void>`, so
// an entry added here without a handler is a compile error rather than a line
// in the sheet that does nothing when pressed.

import type { View } from './views';

export type ShortcutGroup = 'Views' | 'Getting around' | 'Doing things';

/** The order groups appear in the sheet. Views first because they are the keys
 *  most likely to be looked up, and `?` last because whoever is reading it has
 *  already found that one. */
export const SHORTCUT_GROUPS: ShortcutGroup[] = ['Views', 'Getting around', 'Doing things'];

export type Shortcut = {
  id: string;
  /** As `KeyboardEvent.key` **lowercased** — see `App`'s handler. Lowercasing
   *  is what makes `H` step like `h` (the old `switch` did the same), and it
   *  leaves digits and punctuation alone: `'?'.toLowerCase()` is `'?'`. */
  key: string;
  /** What the sheet prints in the key column. Usually the key itself; spelled
   *  out where the character is not what you press (`⇧/` would be a lie about
   *  a layout we do not know). */
  label: string;
  group: ShortcutGroup;
  /** A second line under the label, for a shortcut whose meaning depends on
   *  where you are. Only the two steppers need one. */
  hint?: string;
  /** The view this key switches to, for the five that do. Carried here rather
   *  than in a second map beside it — that map was `KEY_VIEW`, and a table
   *  that lists a key without saying what it does is half the drift back. */
  view?: View;
  /** Whether the handler must consume the keystroke. `/` and `q` protect the
   * fields they mount and focus in WebKitGTK. */
  consumes?: true;
};

export const SHORTCUTS = [
  { id: 'day',      key: '1', label: '1', group: 'Views', view: 'day' },
  { id: 'week',     key: '2', label: '2', group: 'Views', view: 'week' },
  { id: 'month',    key: '3', label: '3', group: 'Views', view: 'month' },
  { id: 'year',     key: '4', label: '4', group: 'Views', view: 'year' },
  { id: 'bigyear',  key: '5', label: '5', group: 'Views', view: 'bigyear' },

  { id: 'prev',     key: 'h', label: 'h', group: 'Getting around',
    hint: 'a day, a week, a month — whatever the view shows' },
  { id: 'next',     key: 'l', label: 'l', group: 'Getting around',
    hint: 'a day, a week, a month — whatever the view shows' },
  { id: 'today',    key: 't', label: 't', group: 'Getting around' },
  { id: 'search',   key: '/', label: '/', group: 'Getting around', consumes: true },

  { id: 'create',   key: 'n', label: 'n', group: 'Doing things' },
  { id: 'quickCreate', key: 'q', label: 'q', group: 'Doing things', consumes: true },
  { id: 'list',     key: 'f', label: 'f', group: 'Doing things' },
  { id: 'help',     key: '?', label: '?', group: 'Doing things' },
] as const satisfies readonly Shortcut[];

export type ShortcutId = (typeof SHORTCUTS)[number]['id'];

/** The same array, widened.
 *
 *  `as const satisfies` above is what derives `ShortcutId` from the rows, and
 *  the price is that each row's type is its own literal — so `consumes` exists
 *  on the `/` row and on no other, and a lookup over the array cannot ask for
 *  it. This alias is the table as *code that searches it* needs to see it:
 *  every optional field present and every `id` still narrowed to the union, so
 *  `SHORTCUT_ACTIONS[hit.id]` stays exhaustively typed. */
export const SHORTCUT_LIST: readonly (Shortcut & { id: ShortcutId })[] = SHORTCUTS;

/** What the sheet prints beside each key. Separate from the table so the two
 *  columns read as two columns; joined by id, so neither can grow a row the
 *  other lacks without TypeScript saying so. */
export const SHORTCUT_TEXT: Record<ShortcutId, string> = {
  day: 'Day',
  week: 'Week',
  month: 'Month',
  year: 'Year',
  bigyear: 'Big Year',
  prev: 'Back one step',
  next: 'Forward one step',
  today: 'Back to today',
  search: 'Search',
  quickCreate: 'Quick add from natural language',
  create: 'New event',
  list: 'Switch between the grid and the list',
  help: 'This list',
};

/**
 * The two modifier chords, beside the table rather than in it.
 *
 * Not rows of `SHORTCUTS`, and the reason is the invariant that table
 * exists for: every row there is dispatched from `App`'s one bare-key
 * handler through `SHORTCUT_ACTIONS`, and the exhaustive `Record` is what
 * proves each documented key does something. These two are genuinely not
 * dispatched there — copy lives in `EventPopover`, the only component that
 * knows a popover is open in every view, and paste is a chord `App` handles
 * before its bare-key path. Forcing them into the table would mean either a
 * lying no-op handler or teaching the dispatcher about scopes it does not
 * have; a second export in the same file keeps them one screen away from
 * the keys they must not drift from, which is the practical half of the
 * invariant.
 *
 * `MOD` follows the platform so the sheet shows the key the reader will
 * actually press — the handlers themselves accept Ctrl and ⌘ alike.
 */
export const MOD_LABEL =
  typeof navigator !== 'undefined' && /Mac/.test(navigator.platform) ? '⌘' : 'Ctrl';

export const CHORDS: { label: string; text: string; hint?: string }[] = [
  { label: `${MOD_LABEL} C`, text: 'Copy the event',
    hint: 'with its popover open — click the event first' },
  { label: `${MOD_LABEL} V`, text: 'Paste as a new event',
    hint: 'opens the form on the day you are looking at, to fine-tune first' },
];

/** The table in the order the sheet draws it. A group with no shortcuts is
 *  dropped rather than drawn empty, so `SHORTCUT_GROUPS` can name a heading
 *  before anything is filed under it. */
export function groupedShortcuts(): {
  group: ShortcutGroup;
  items: readonly (Shortcut & { id: ShortcutId })[];
}[] {
  return SHORTCUT_GROUPS
    .map((group) => ({ group, items: SHORTCUT_LIST.filter((s) => s.group === group) }))
    .filter((g) => g.items.length > 0);
}
