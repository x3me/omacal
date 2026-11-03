// The hours the Day and Week grids show, as the rest of the UI reads them.
//
// A module-level rune for `secondzone.svelte.ts`'s reason: the reader
// (`WeekGrid`) and the writer (`App`, seeding it from settings) sit in
// different subtrees, and neither owns the preference.
//
// **Read through a snapshot, not the store.** This used to return the
// `$state` object itself, so any reader could write `visibleHours().start = 5`
// and change the grid from outside `App` — the one invariant every other store
// in this family states in its header. Nothing did, but nothing stopped it.
// The getter reads every field, so a reader stays subscribed exactly as before.
//
// **Cropped or fitted.** By default the grids draw these hours and nothing
// else. With `fit` they draw the whole day and size an hour so that these
// fill the pane, the rest of the day a scroll away (`WeekGrid`'s `drawnPx`).
//
// Seeded and kept fresh by `App`, the only writer, on startup and after every
// settings change. 0–24 until then: the whole day, which is the fresh-install
// default and never hides an event.

const state = $state({ start: 0, end: 24, fit: false });

/** The visible range, `start` inclusive and `end` exclusive, in hours, and
 *  whether it is fitted to the pane rather than cropped to. */
export const visibleHours = (): { readonly start: number; readonly end: number; readonly fit: boolean } =>
  ({ start: state.start, end: state.end, fit: state.fit });

/** `App` only. */
export function applyVisibleHours(start = 0, end = 24, fit = false) {
  state.start = start;
  state.end = end;
  state.fit = fit;
}
