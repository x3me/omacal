# omacal — saved calendar groups

**Status:** proposed 2026-10-15
**Base:** `main` @ `220a831` — which calendars are drawn is the per-calendar `selected` flag, toggled one at a time in Menu -> Calendars or the Big Year legend

---

## 1. What this delivers

A **calendar group** is a named, saved set of shown calendars, like
Fantastical's Calendar Sets. Applying one shows exactly its calendars and hides
every other calendar the picker can toggle. It is a view choice: it writes
`selected`, never `sync_enabled`, so nothing stops syncing and nothing reaches
the provider.

- **Switcher.** A button after the list toggle in the header. Its label is the
  active group's name, "All calendars", or "Custom". The popover lists the
  groups (click to apply) and carries the same calendar checkboxes as
  Menu -> Calendars, so a set can be built and saved from one place.
- **Save, edit, delete.** "Save current as group…" names what is shown now.
  Each group can be renamed, updated to what is shown now, or deleted behind an
  inline confirm.
- **⌘1–⌘9** (Ctrl elsewhere) applies the Nth group. It is listed in the
  shortcuts panel.

**Not in scope:** per-group colours or icons, groups that change sync, syncing
groups between machines, reordering by drag.

## 2. Rules

- **The active group is derived, not stored.** It is the first group whose live
  ids equal the shown set. Ticking a calendar by hand moves the label or turns
  it to "Custom", so no stale mark is left behind.
- **Only calendars the picker can toggle count.** A removed calendar and an
  "On this device" list have a disabled checkbox. A group that hid one could
  not be undone by hand, so they are never saved into a group, compared, or
  touched by an apply. This matches the header's count.
- **Stale ids are kept and ignored.** An id whose calendar is gone stays in the
  stored group. If sync is turned back on the id comes back with it. Removing
  and reconnecting an account mints new ids, and those don't match.
- **Empty applies are refused.** A group that names calendars but has none live
  would hide everything. It is refused with a message in the error banner
  ("This group's calendars are gone. Update or delete it."), for the click and
  the chord alike, and nothing is changed. With no calendar shown, Save and
  Update are disabled and a short note says why.
- **The chord reads `e.code`** (`Digit1`..`Digit9`), so layouts whose top row
  doesn't write ASCII digits still work (#38). It ignores auto-repeat and is
  held back by the same open-panel guards as paste.

## 3. Mechanism

- **Storage.** One JSON array in the `calendar_groups` settings row
  (`Setting::CalendarGroups`), in switcher order. Writes are normalised: names
  trimmed, unique case-insensitively, at most 60 characters; ids sorted and
  de-duplicated; at most 50 groups. A row that doesn't parse reads back as no
  groups.
- **Apply.** A new `set_calendars_shown(ids)` command calls
  `set_selected_exactly` in `omacal-store`: one transaction that sets
  `selected = (id in ids)` on `sync_enabled` rows only, so the grid never
  refreshes against half a switch. `App` then runs the same refresh as any
  other visibility change.
- **Ownership.** `Header` and the switcher only call back. `App` owns the
  writes, as it does for every other setting.

## 4. Testing

- `omacal-store`: `set_selected_exactly` shows the named calendars and only
  those, and leaves non-`sync_enabled` rows alone.
- `settings.rs`: groups round-trip; blank, duplicate, over-long and over-count
  lists are refused and store nothing; a mangled row reads as none.
- `ui/tests/calendar-groups.spec.ts`: the pure helpers (active group, stale ids,
  scoping, refusal, `e.code` mapping), then the App: save and switch by click
  and by chord, rename/update/delete, tick a box in the switcher and save, a
  local list is neither saved nor hidden, refusal by click and by chord, and
  saving refused with nothing shown.
- The `header-connected` goldens gain the switcher button and need
  regenerating on the Linux baseline machine.
