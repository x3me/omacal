# The form learns some manners, and the week learns to slide

Two requests from the same screenshot session (2026-08-28, macOS Calendar
as the reference): the event form should feel closer to macOS Calendar's
editor — concretely, the calendar selector collapses to a **colour dot**
that expands into the full account-grouped list on click — and the Week
view should **pan horizontally by days** with a trackpad/wheel gesture, so
"a few more days into next week" is a flick, not a full week-step. Today
snaps back to the settings-standard alignment.

## 1. The calendar dot (`CalendarPicker.svelte`)

What macOS does: the editor's top-right corner is a small colour swatch;
clicking it drops a list grouped by account — muted account heading, a
colour dot and name per calendar, a check on the current one.

omacal's version:

- The old labelled `<select>` row disappears. The **title row** becomes
  `[title input][dot button]` — the dot shows the chosen calendar's
  colour, `aria-label="Calendar"`, `aria-expanded`, `aria-haspopup=
  "listbox"`.
- The popover lists `writableCalendars(calendars)` grouped by
  `account_email` in first-seen order. Groups get the email as a muted
  heading **only when more than one account offers calendars** — a
  single-account user reads their own email nowhere, same rule as the old
  select's `summary · email` suffix.
- Rows are `role="option"` buttons: dot, summary, and `aria-selected` on
  the current one (drawn as a ✓ plus accent tint). Click chooses and
  closes.
- **Edit mode**: the dot button is disabled with the same tooltip the
  select had — an event cannot be moved between calendars from omacal.
  **Superseded 2026-09-01.** A week of daily use found the cost of that:
  with several calendars, an event created on the wrong one could only be
  fixed by deleting it and making it again, which throws away every guest's
  answer. `update_event` now takes a target calendar — Google's own
  `events.move`, or a PUT-then-DELETE across CalDAV collections — and the
  picker is enabled on an edit, offering the writable calendars of the
  event's *own account*. Across accounts there is no move, only a copy that
  re-invites everyone, so that stays refused and says so.
- Escape closes the picker and not the form: the form's `escapeCloses`
  guard adds `!calendarOpen`, and listener registration order (form
  before picker) means the form's guard is evaluated while the picker is
  still open — the same layering ConfirmPanel already relies on.
- A transparent scrim sibling closes on outside click, as every popover
  here does.

## 2. The form's grouped-card polish

macOS groups the editor into rounded blocks. The same reading order,
as quiet cards (`.card`): when (all-day + dates + second-zone hint) ·
location + video call · notify · description · repeat (+ its dependents)
· guests. Existing controls, labels and specs keep their names — the
cards are wrappers and CSS, not new semantics. The title input drops its
box (borderless, 15px) so the row reads as a heading, exactly like the
reference. The panel itself gains a 120ms fade/scale entrance, disabled
under `prefers-reduced-motion`.

## 3. Week view pans by days

- `WeekGrid` gets a root `onwheel`: when `|deltaX| > |deltaY|` the event
  is consumed and accumulated; every 90px emits `onpan(±days)`. The
  accumulator decays after 250ms so residue never leaks into the next
  gesture.
- `App` keeps `weekPanDays`. The rendered window start is
  `addDays(weekStartMs, weekPanDays)` — day arithmetic through
  `Date.setDate`, never `±86400000`, for the DST reasons keyboardnav
  already documents.
- Fetching: a panned week is by definition unaligned, so any
  `weekPanDays ≠ 0` fetches through `get_range(start, weekViewDays)` —
  the machinery PR 4's rolling week already built. Aligned weeks keep
  their `get_week` path byte-for-byte.
- Day view: the same gesture moves `anchorMs` itself by the emitted days;
  there is no separate offset to reconcile.
- **Today** (`t` or the header button) zeroes `weekPanDays` before
  moving the anchor — the user's stated contract: "pressing today moves
  it to standard view (as per settings)".
- `h`/`l` step by the view's unit and deliberately **keep** the pan
  offset: a window shifted 2 days into the week stays shifted while
  stepping weeks; only Today re-aligns.
- The header title reads from the panned start, so the label always
  names what the columns show.

## 4. What this deliberately does not do

- No pixel-continuous panning (macOS renders adjacent days off-screen;
  that is a virtualized-strip rewrite). Day-stepped panning with the
  local-DB fetch (a few ms) is the honest version of "smooth" here.
- No pan in Month/Year/Big Year, and none in list mode — the request was
  the Week grid; the others have no day-sized column to slide by. (Month and
  Year later gained discrete **paging** — one period per wheel or swipe, a step
  rather than a pan; see `2026-10-06-omacal-month-year-paging-design.md`. The
  "no pan" rule still holds.)
- The native `<select>`s elsewhere on the form (repeat, reminders, video)
  stay native — they got their dark popups via the GTK hint (38f4805);
  replacing them wholesale is a separate decision.
