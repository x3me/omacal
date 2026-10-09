// The event form's Save, as a pending change (part 2 spec, 2026-10-09).
// Pure: `pendingedit.spec.ts` drives every rule.

import type { Calendar } from './calendars';
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
  const when = repeatChanged || seriesSwitch
    ? null
    : f.when.kind === 'timed'
      ? { allDay: false, startMs: f.when.startMs, endMs: f.when.endMs }
      : { allDay: true, startMs: dayMs(f.when.startDate), endMs: dayMs(f.when.endDate) };

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
