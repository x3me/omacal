// ui/src/lib/calendargroups.ts
import { invoke } from '@tauri-apps/api/core';
import { MOD_LABEL } from './shortcuts';
import { setSetting } from './settings';
import type { Calendar } from './calendars';

/**
 * A named, saved choice of which calendars are **shown** — the `selected`
 * flag, never `sync_enabled`. Applying one hides and shows; nothing stops
 * syncing and nothing reaches Google. Stored as one list (the settings row
 * `calendar_groups`), in the order the switcher lists them.
 *
 * Only calendars the picker can toggle count — `sync_enabled`. A removed
 * calendar and an "On this device" local list have a disabled checkbox, so a
 * group that hid one could never be undone by hand; they are neither saved into
 * a group, compared against one, nor touched by applying one.
 *
 * Which group is *active* is not stored anywhere. It is whichever group's
 * calendars are exactly the ones shown right now ([`activeGroupIndex`]), so
 * ticking a calendar in the picker by hand can never leave a stale "active"
 * mark behind: the mark simply moves, or goes.
 */
export type CalendarGroup = { name: string; calendarIds: number[] };

/** ⌘1–⌘9 reach this many; a tenth group is in the list but not on a key. */
export const GROUP_SHORTCUTS = 9;

/** The ids of the calendars drawn right now that the picker can toggle,
 *  ascending — what "Save current as group" captures. Matches the header's
 *  count, `c.sync_enabled && c.selected`. */
export function shownIds(cals: Calendar[]): number[] {
  return cals.filter((c) => c.sync_enabled && c.selected).map((c) => c.id).sort((a, b) => a - b);
}

/**
 * A group's ids that still name a calendar the picker can toggle. A removed
 * calendar's id stays in the stored group (it rejoins if sync is turned back
 * on, which keeps the id; removing the account and connecting it again mints
 * new ids, and those do not) and is ignored here, which is what makes the
 * comparison and the apply agree about it.
 */
export function liveIds(group: CalendarGroup, cals: Calendar[]): number[] {
  const known = new Set(cals.filter((c) => c.sync_enabled).map((c) => c.id));
  return [...new Set(group.calendarIds)].filter((id) => known.has(id)).sort((a, b) => a - b);
}

/** The first group whose calendars are exactly the ones shown, or `-1`. */
export function activeGroupIndex(groups: CalendarGroup[], cals: Calendar[]): number {
  const shown = shownIds(cals);
  return groups.findIndex((g) => {
    const ids = liveIds(g, cals);
    return ids.length === shown.length && ids.every((id, i) => id === shown[i]);
  });
}

/** `Digit1`..`Digit9` to `0`..`8`. The physical key, not `e.key`: on a layout
 *  that writes its own script the top row does not write ASCII digits, and the
 *  chord would silently stop working there (issue #38; see `shortcutKeyFor`).
 *  The numpad is left alone — ⌘ plus a numpad digit is not a chord anyone makes
 *  on purpose. */
export function groupIndexForCode(code: string): number | null {
  const m = /^Digit([1-9])$/.exec(code);
  return m ? Number(m[1]) - 1 : null;
}

/** The chord that applies group `i`, as the reader will press it, or `null`
 *  past the ninth. */
export function groupChordLabel(i: number): string | null {
  return i < GROUP_SHORTCUTS ? `${MOD_LABEL}${i + 1}` : null;
}

/** Why `group` cannot be applied, or `null` when it can. A group that names
 *  calendars but none of them is live (the account was removed and connected
 *  again, so every id is new) would hide everything and say nothing. */
export function applyRefusal(group: CalendarGroup, cals: Calendar[]): string | null {
  return group.calendarIds.length > 0 && liveIds(group, cals).length === 0
    ? 'This group\u2019s calendars are gone. Update or delete it.'
    : null;
}

/** Shows exactly `group`'s calendars and hides every other the picker can
 *  toggle. One transaction on the backend, so the grid is never refreshed
 *  against half a switch. Rejects with the [`applyRefusal`] sentence. */
export const applyCalendarGroup = (group: CalendarGroup, cals: Calendar[]) => {
  const why = applyRefusal(group, cals);
  return why ? Promise.reject(why) : invoke<void>('set_calendars_shown', { ids: liveIds(group, cals) });
};

/** Replaces the whole list; resolves to the list as stored (names trimmed,
 *  ids de-duplicated), and rejects with a sentence for one it refuses. */
export const saveCalendarGroups = (groups: CalendarGroup[]) =>
  setSetting('calendarGroups', groups).then((s) => s.calendarGroups);
