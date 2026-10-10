// The event form's Save, as a pending change (part 2 spec, 2026-10-09:
// edits; part 3 spec, 2026-10-10: creates). Pure: `pendingedit.spec.ts`
// drives every rule.

import type { Calendar } from './calendars';
import type { WhenInput } from './eventdetail';
import type { EventFormResult } from './eventform';
import type { EditPatch, PendingChange } from './pendingview';

/** What the form knew about the occurrence when it opened. */
export type EditRequest = {
  id: number;
  /** The occurrence's own start, never the series' DTSTART. */
  occurrenceStartMs: number;
  wasAllDay: boolean;
  isRecurring: boolean;
  calendarId: number | null;
};

/**
 * The change a Save makes, for the overlays to draw.
 *
 * Its time waits for the save (`when: null`) when only the backend can say
 * where the occurrences go: the repeat controls changed (`toEventInput` sends
 * them only then), or all-day was switched for more than this occurrence of a
 * series. Everything else is drawn at once.
 */
export function editChange(
  req: EditRequest, result: EventFormResult,
  calendars: Pick<Calendar, 'id' | 'color_hex'>[], dayMs: (ymd: string) => number,
): PendingChange {
  const f = result.fields;
  const repeatChanged = f.repeat !== undefined || f.weeklyDays !== undefined || f.repeatEnd !== undefined;
  const allDay = f.when.kind === 'allDay';
  const seriesSwitch = req.isRecurring && result.scope !== 'this' && allDay !== req.wasAllDay;
  const when = repeatChanged || seriesSwitch ? null : drawnWhen(f.when, dayMs);

  const patch: EditPatch = { title: f.summary ?? '(no title)', location: f.location };
  if (result.calendarId !== req.calendarId) {
    patch.calendar_id = result.calendarId;
    const color = calendars.find((c) => c.id === result.calendarId)?.color_hex;
    if (color) patch.color = color;
  }
  // The form's list is everyone stored on the event, the user and the
  // organiser included (`valueFromDetail`), which is what the backend counts.
  if (f.guests) patch.attendees = f.guests.length;
  if (f.conference === 'none') patch.conference = null;

  return {
    kind: 'edit', id: req.id, occurrenceStartMs: req.occurrenceStartMs, scope: result.scope, when, patch,
    detail: {
      description: f.description,
      ...(f.guests ? { guests: f.guests.map(({ email, optional }) => ({ email, optional })) } : {}),
    },
  };
}

/** Where a form's `WhenInput` is drawn: a timed one at its own instants, an
 *  all-day one from the display-zone midnight of its first day to that of the
 *  day after its last (the form's `endDate` is already exclusive). */
export function drawnWhen(
  when: WhenInput, dayMs: (ymd: string) => number,
): { allDay: boolean; startMs: number; endMs: number } {
  return when.kind === 'timed'
    ? { allDay: false, startMs: when.startMs, endMs: when.endMs }
    : { allDay: true, startMs: dayMs(when.startDate), endMs: dayMs(when.endDate) };
}

/** `commands.rs`' `DEFAULT_EVENT_COLOR`: what the backend draws for a
 *  calendar with no colour of its own. */
const DEFAULT_EVENT_COLOR = '#5b8def';

/**
 * A new event, as the overlays draw it until Google has it (part 3 spec §3).
 *
 * `id` is the temporary negative id `unsavedId()` hands out. A repeating
 * event is drawn as the form's own first occurrence: only the backend expands
 * a rule. `toEventInput` sends the repeat fields on a create only when a
 * repeat is set. A requested Meet link exists only once Google mints it.
 */
export function createChange(
  result: EventFormResult, calendars: Pick<Calendar, 'id' | 'color_hex'>[],
  dayMs: (ymd: string) => number, id: number,
): PendingChange {
  const f = result.fields;
  const w = drawnWhen(f.when, dayMs);
  return {
    kind: 'create', id,
    event: {
      id, calendar_id: result.calendarId,
      color: calendars.find((c) => c.id === result.calendarId)?.color_hex ?? DEFAULT_EVENT_COLOR,
      title: f.summary ?? '(no title)', location: f.location,
      start_ms: w.startMs, end_ms: w.endMs, is_all_day: w.allDay,
      response: 'accepted', attendees: f.guests?.length ?? 0,
      recurring: f.repeat !== undefined, conference: null, all_guests_declined: false,
      pending: true,
    },
  };
}
