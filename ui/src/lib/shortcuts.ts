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
  /** Other `KeyboardEvent.key` spellings for the same action. Kept off the
   * printed label so `Enter` can share `o` without becoming a duplicate row. */
  aliases?: readonly string[];
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
  /** Whether the handler must consume the keystroke. `/`, `n`, and `q`
   * protect the fields they mount and focus in WebKitGTK; open protects its
   * calendar surface. */
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
  { id: 'prevDay',  key: 'b', label: 'b', group: 'Getting around',
    hint: 'select the previous day' },
  { id: 'nextDay',  key: 'w', label: 'w', group: 'Getting around',
    hint: 'select the next day' },
  { id: 'prevEvent', key: 'k', label: 'k', group: 'Getting around',
    hint: 'up, then into the previous day' },
  { id: 'nextEvent', key: 'j', label: 'j', group: 'Getting around',
    hint: 'down, then into the next day' },
  { id: 'today',    key: 't', label: 't', group: 'Getting around' },
  { id: 'search',   key: '/', label: '/', group: 'Getting around', consumes: true },

  { id: 'openSelected', key: 'o', aliases: ['enter'], label: 'o / Enter',
    group: 'Doing things', consumes: true },
  { id: 'deleteSelected', key: 'backspace', label: 'Backspace',
    group: 'Doing things', consumes: true },
  { id: 'create',   key: 'n', label: 'n', group: 'Doing things', consumes: true },
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
  prevDay: 'Previous day',
  nextDay: 'Next day',
  prevEvent: 'Previous event',
  nextEvent: 'Next event',
  today: 'Back to today',
  search: 'Search',
  quickCreate: 'Quick add from natural language',
  openSelected: 'Open the selected event',
  deleteSelected: 'Delete the selected event (asks first)',
  create: 'New event',
  list: 'Switch between the grid and the list',
  help: 'This list',
};

/**
 * The modifier chords, beside the table rather than in it.
 *
 * Not rows of `SHORTCUTS`, and the reason is the invariant that table
 * exists for: every row there is dispatched from `App`'s one bare-key
 * handler through `SHORTCUT_ACTIONS`, and the exhaustive `Record` is what
 * proves each documented key does something. These are genuinely not
 * dispatched there — copy lives in `EventPopover`, the only component that
 * knows a popover is open in every view, while paste and Settings are chords
 * `App` handles before its bare-key path. Forcing them into the table would
 * mean either a lying no-op handler or teaching the dispatcher about scopes it
 * does not have; a second export in the same file keeps them one screen away
 * from the keys they must not drift from, which is the practical half of the
 * invariant.
 *
 * `MOD` follows the platform so the sheet shows the key the reader will
 * actually press — the handlers themselves accept Ctrl and ⌘ alike.
 */
export const MOD_LABEL =
  typeof navigator !== 'undefined' && /Mac/.test(navigator.platform) ? '⌘' : 'Ctrl';
const ALT_LABEL = MOD_LABEL === '⌘' ? 'Option' : 'Alt';

export const CHORDS: { label: string; text: string; hint?: string }[] = [
  { label: `${MOD_LABEL} ,`, text: 'Open settings' },
  { label: `${MOD_LABEL} C`, text: 'Copy the event',
    hint: 'with its popover open — click the event first' },
  { label: `${MOD_LABEL} V`, text: 'Paste as a new event',
    hint: 'opens the form on the day you are looking at, to fine-tune first' },
  { label: `${MOD_LABEL} =`, text: 'Taller hours',
    hint: 'Day and Week — a trackpad pinch or Ctrl+scroll over the grid does the same' },
  { label: `${MOD_LABEL} -`, text: 'Shorter hours' },
  { label: `${MOD_LABEL} 0`, text: 'Hours back to their usual height' },
  { label: `${MOD_LABEL} 1–9`, text: 'Show a saved calendar group',
    hint: 'the Nth group in the header switcher — it shows those calendars and hides the rest' },
  { label: `${ALT_LABEL} drag`, text: 'Create over an existing event',
    hint: 'Day and Week — hold the key before dragging to select a new time range' },
];

/** A form chord rather than a bare key: it bubbles from any field to the form
 * and submits through the same validation and guest-notification path as its
 * Save button. */
export const EDIT_CHORDS: { label: string; text: string; hint?: string }[] = [
  { label: `${MOD_LABEL} Enter`, text: 'Save edits', hint: 'from anywhere in the edit form' },
];

/** Bare keys that belong to an open event rather than to the calendar.
 *
 * Kept out of `SHORTCUTS` because `App` deliberately ignores every key whose
 * target is inside a dialog. `EventPopover` dispatches this table itself, and
 * the shortcut sheet renders these same rows, preserving the binding-and-docs
 * invariant without making `n` both "new event" and "RSVP no" globally. */
export type EventShortcut = {
  id: string;
  key: string;
  aliases?: readonly string[];
  label: string;
  text: string;
};

export const EVENT_SHORTCUTS = [
  { id: 'edit',   key: 'e',     label: 'e',     text: 'Edit event' },
  { id: 'delete', key: 'd', aliases: ['backspace'], label: 'd / Backspace',
    text: 'Delete event (asks first)' },
  { id: 'yes',    key: 'y',     label: 'y',     text: 'RSVP yes' },
  { id: 'maybe',  key: 'm',     label: 'm',     text: 'RSVP maybe' },
  { id: 'no',     key: 'n',     label: 'n',     text: 'RSVP no' },
  { id: 'join',   key: 'enter', label: 'Enter', text: 'Join meeting' },
] as const satisfies readonly EventShortcut[];

export type EventShortcutId = (typeof EVENT_SHORTCUTS)[number]['id'];
export const EVENT_SHORTCUT_LIST: readonly (EventShortcut & { id: EventShortcutId })[] =
  EVENT_SHORTCUTS;

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

/**
 * The key a keydown names for the tables above.
 *
 * `KeyboardEvent.key` is what the layout *writes*, lowercased so `H` steps
 * like `h`. On a layout that writes its own script that is not a Latin
 * letter or digit at all — Persian writes ۱ for the top-row 1, Bulgarian
 * writes х for the key under H — so no row ever matched, and only the
 * numpad, which writes Latin digits on every layout, switched views
 * (issue #38, 2026-09-03). For a single character outside ASCII the key is
 * read off `KeyboardEvent.code` instead: the physical key, named by its
 * US-layout cap, which is the letter the tables mean. Latin layouts with
 * their own arrangement (Dvorak, AZERTY) write ASCII and keep `key`, so a
 * Dvorak `d` stays `d` and never becomes the QWERTY `h` under it.
 */
export function shortcutKeyFor(key: string, code: string): string {
  const k = key.toLowerCase();
  // One character, and none of it ASCII: a script of its own.
  if (!/^[^\x00-\x7f]$/u.test(k)) return k;
  const m = /^(?:Key([A-Z])|(?:Digit|Numpad)([0-9]))$/.exec(code);
  if (!m) return k;
  return (m[1] ?? m[2]).toLowerCase();
}
