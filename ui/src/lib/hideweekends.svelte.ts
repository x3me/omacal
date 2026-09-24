// Whether Saturday and Sunday columns are dropped from Week and Day view, as
// the rest of the UI reads it.
//
// A module-level rune for `visiblehours.svelte.ts`'s reason: the reader
// (`WeekGrid`, day-view navigation) and the writer (`App`, seeding it from
// settings) sit in different subtrees, and neither owns the preference.
//
// Seeded and kept fresh by `App`, the only writer, on startup and after
// every settings change. `false` until then — the fresh-install default,
// and never hides a day.

const state = $state({ hideWeekends: false });

/** Whether weekend columns are currently hidden. */
export const hideWeekends = (): boolean => state.hideWeekends;

/** `App` only. */
export function applyHideWeekends(on = false) {
  state.hideWeekends = on;
}
