// The event form's own value type, and the pure functions either side of it:
// what a form value is made of, how one is built from an `EventDetail`, and
// how one becomes the `EventInput` the Rust write commands take.
//
// Deliberately separate from `EventForm.svelte` so the fiddly parts — the
// next-half-hour default, the inclusive/exclusive all-day end, the
// three-state `repeat` — are testable as functions rather than only through a
// rendered form.

import type {
  EventDetail, EventInput, RepeatEnd, SendUpdates, WeekdayCode, WhenInput,
} from './eventdetail';
export type { RepeatEnd, WeekdayCode } from './eventdetail';
import { meetingProvider, meetingUrl } from './location';

/** Which occurrences an edit applies to. Mirrors `update_event`'s own scopes. */
export type Scope = 'this' | 'all' | 'following';

/**
 * What the form hands back on save — everything a write command needs that the
 * form itself knows, and nothing it does not.
 *
 * There is no event id and no `occurrenceStartMs` here on purpose: both belong
 * to the block that was clicked, the caller has them already, and threading
 * them through the form would give it a second chance to hand `updateEvent` the
 * series DTSTART instead of the occurrence. See `updateEvent`'s doc comment.
 */
export type EventFormResult = {
  calendarId: number;
  /** Meaningless for a create, and for a one-off edit; always `'this'` there. */
  scope: Scope;
  fields: EventInput;
  /**
   * Who Google mails about this save (spec §3).
   *
   * A required field rather than a defaulted one, for the reason
   * `SendUpdates`' own doc comment gives: this is the one value where nobody
   * choosing is how somebody gets an email they should not have. The form
   * asks when there is anybody to ask about, and answers `'none'` when there
   * is not — so `'all'` appears only where a person chose it.
   */
  notify: SendUpdates;
};

/**
 * The Repeat options omacal can express, in the order they are offered, with
 * the exact keys `write::rrule_for` maps to an RRULE.
 *
 * This is a *label* table, not a second copy of the mapping: no rule text
 * appears here, and nothing in this file decides whether a given RRULE is one
 * of these — that answer arrives already made, on `EventDetail.repeat`, from
 * `write::repeat_from_rrule`. That Rust function also strictly recognises the
 * plain weekly BYDAY shape represented by `weeklyDays`; the UI never parses
 * raw recurrence to make this decision.
 */
export const REPEAT_OPTIONS: Array<[key: string, label: string]> = [
  ['never', 'Does not repeat'],
  ['daily', 'Daily'],
  ['weekdays', 'Every weekday (Mon–Fri)'],
  ['weekly', 'Weekly'],
  ['monthly', 'Monthly'],
  ['yearly', 'Yearly'],
];

/** The value `write::repeat_from_rrule` answers for a rule omacal cannot
 *  express. Not in `REPEAT_OPTIONS`: it is never something a user may pick. */
export const CUSTOM_REPEAT = 'custom';

/** The visual pattern the user asked for: `S M T W R F S`, with `R` for
 * Thursday so both Tuesdays and Thursdays stay one keystroke/wide button. */
export const WEEKDAY_OPTIONS: ReadonlyArray<{
  code: WeekdayCode; short: string; name: string;
}> = [
  { code: 'SU', short: 'S', name: 'Sunday' },
  { code: 'MO', short: 'M', name: 'Monday' },
  { code: 'TU', short: 'T', name: 'Tuesday' },
  { code: 'WE', short: 'W', name: 'Wednesday' },
  { code: 'TH', short: 'R', name: 'Thursday' },
  { code: 'FR', short: 'F', name: 'Friday' },
  { code: 'SA', short: 'S', name: 'Saturday' },
];

/**
 * Everything the form edits, in the shapes its inputs actually hold — dates as
 * `yyyy-mm-dd` and times as `HH:MM`. Converting to instants is `whenOf`'s job
 * and happens once, on save.
 *
 * The two `source…Ms` fields are the exception, and are not editable state: see
 * their own comment for why a value has to remember what its civil fields were
 * read off.
 */
export type EventFormValue = {
  title: string;
  /**
   * `yyyy-mm-dd`.
   *
   * For a **timed** event, the browser's reading of the start instant — which
   * day an instant falls on is a question about the reader, and the reader is
   * who is looking at the form.
   *
   * For an **all-day** event, the day the *calendar* keeps the event on,
   * carried here from `EventDetail.start_date` and never derived from an
   * instant. An all-day event has no instant to read: the one the store holds
   * is midnight in the calendar's zone, and any other zone reads it as the
   * neighbouring day.
   */
  date: string;
  /**
   * `yyyy-mm-dd`. For an all-day event this is the **inclusive** last day — the
   * day the user would point at and call the end.
   *
   * Google's wire format is exclusive (`end.date` is the day *after* the last
   * one), and so is the store's `end_utc`. Exactly one conversion sits between
   * the two, in `whenOf`, on the way out. Nothing converts on the way in:
   * `EventDetail.end_date` is already the inclusive day, worked out once on the
   * Rust side. A form that showed the exclusive date would read a day long, and
   * one that sent the inclusive date would silently shorten every all-day event
   * it saved.
   */
  endDate: string;
  /** `HH:MM`, 24-hour. Ignored when `isAllDay`. */
  start: string;
  /** `HH:MM`, 24-hour. Ignored when `isAllDay`. */
  end: string;
  /**
   * The instants `date`+`start` and `endDate`+`end` were **read off**, or
   * `null` when they were not read off anything — a value assembled from two
   * sources (`blankValue` moving a default onto a chosen day) or typed from
   * nothing has no earlier instant behind it.
   *
   * Not editable state, and the only numbers on a type that otherwise holds
   * civil strings. They are here because the reading is lossy in **both**
   * directions and re-deriving what nobody touched therefore *moves* the event:
   *
   *   - `HH:MM` cannot say 09:00:37, so a start with seconds comes back 37
   *     seconds early;
   *   - a repeated wall-clock hour names two instants an hour apart with one
   *     `yyyy-mm-dd HH:MM` between them, and `toMs` resolves that pair to the
   *     earlier — a full hour early, for 12 block starts a year in
   *     `Europe/Sofia`, 12 in `America/New_York`, 6 in `Australia/Lord_Howe`.
   *
   * `write::shifted_like` short-circuits only on an *exact* match, so a
   * difference of any size is applied as a real move: renaming a meeting sends
   * a start/end PATCH dragging it an hour earlier, with `sendUpdates=all`
   * behind it.
   *
   * `whenOf` sends one of these unchanged exactly when the civil fields beside
   * it still read as it — see `instantOf`. Nothing compares them against the
   * form's `initial`, which is deliberate; that comment says why.
   */
  sourceStartMs: number | null;
  sourceEndMs: number | null;
  isAllDay: boolean;
  location: string;
  description: string;
  /** The video call described by the form, kept separate from the place.
   *
   * Google Meet and a connected Zoom account can both be requested without a
   * URI (`source: 'new'`); their providers fill it after the write. A pasted
   * Zoom link still has a URI. Existing third-party conference data is carried
   * as `other` so an unrelated edit preserves it. */
  videoCall: VideoCall | null;
  /** The calendar to write to. `null` when there is not a writable one. */
  calendarId: number | null;
  /** A key from `REPEAT_OPTIONS`, or `CUSTOM_REPEAT`. */
  repeat: string;
  /** The selected days when `repeat === 'weekly'`. Always Sunday-first and
   *  non-empty for values built by this module. */
  weeklyDays: WeekdayCode[];
  /** How the series stops. Kept even while Repeat is Never so switching a
   * cadence off and back on does not discard what was typed. */
  repeatEnd: RepeatEnd;
  /** The raw RRULE behind a `custom` repeat, for showing it in words. Display
   *  only — never parsed to decide what the app can express. */
  recurrence: string | null;
  /**
   * **The guest list the event would end up with — everyone, including the
   * signed-in user.**
   *
   * Not to be confused with `mailableGuests`, which is deliberately one
   * smaller on an event you are on: that counts who a save would *mail*, and
   * telling somebody they are about to notify themselves is wrong. This is what
   * the attendee array becomes, and a version that dropped the self row to
   * match the count would take the user off every event they saved — and,
   * since a drag builds its input from this same value, off every event they
   * dragged.
   *
   * Two fields per guest, because two are all a user can author. Everything
   * else an attendee carries — their answer, their display name, their comment
   * — belongs to that person and is echoed back on the Rust side from what is
   * stored (`events::attendees_for_edit`). There is no field here through which
   * this form could overwrite somebody's RSVP.
   */
  guests: Guest[];
  /** Whoever owns the event, or `null` when Google names nobody. §5: the
   *  organizer cannot be removed, and this is how the row is recognised. */
  organizerEmail: string | null;
  /** The signed-in user's own address, when they are on the event at all.
   *  §5 again: removing yourself is a real thing and is *not* declining, so the
   *  row has to be tellable from the others in order to say so. */
  selfEmail: string | null;
  /**
   * The popup reminders the form shows and edits, as minutes before the
   * start — the event's own overrides, or the calendar's defaults when the
   * event runs on those (reminders spec §3). Rows only; whether anything is
   * *sent* is `toEventInput`'s unchanged-means-absent rule, same as guests.
   */
  popupReminders: number[];
  /**
   * The event's `email` reminders, preserved verbatim and never shown as
   * editable — Google sends those, not omacal (spec §1). Carried so an edit
   * that touches popups re-sends them unchanged: `reminders` is a
   * whole-object replace, and a payload without them would strip them.
   * Seeded from the calendar's defaults when the event runs on those, so
   * flipping to explicit overrides does not silently drop a default email.
   */
  emailReminders: { method: string; minutes: number }[];
  /** Display only: the rows above came from the calendar's defaults rather
   *  than the event's own overrides. Decides the hint, nothing else. */
  remindersWereDefault: boolean;
  /** An edit, rather than a create. Decides the Save label, whether the scope
   *  chooser and the guest editor appear, and which calendars the picker
   *  offers — an edit may re-file the event, but only onto another calendar
   *  of the same account (`update_event`'s `MOVE_ACROSS_ACCOUNTS`). */
  isEdit: boolean;
  /** Part of a series — the only case where a scope choice means anything. */
  isRecurring: boolean;
  /** The two facts `editReach` reads (`eventdetail.ts`): whether this is the
   *  user's own event, and whether its organizer lets guests change it for
   *  everyone. A create is the user's own by definition. Carried on the value
   *  because the save panel is mounted from it, not from the detail. */
  isOrganizer: boolean;
  guestsCanModify: boolean;
};

export type VideoCall = {
  provider: 'googleMeet' | 'zoom' | 'other';
  uri: string | null;
  source: 'conference' | 'location' | 'new';
};

/** One guest, as the form holds them. Mirrors `write::Guest` on the Rust side,
 *  field for field, and for the reason that type's own comment gives. */
export type Guest = {
  email: string;
  optional: boolean;
};

const MIN_MS = 60_000;
const HALF_HOUR_MS = 30 * MIN_MS;
/** How long a brand-new event is when no stored preference is available. */
const DEFAULT_EVENT_MINUTES = 60;

/**
 * The civil times an **all-day** value carries in its (hidden) time fields.
 *
 * An all-day event has no time, so these are never shown and never sent —
 * `whenOf`'s all-day arm sends dates. They exist for exactly one moment: the
 * user unticking **All day**, when the form stops hiding the time inputs and
 * whatever is in them becomes the event's span.
 *
 * A real pair rather than a reading of the stored instant, and that is the
 * whole of §7.2. The stored instant is midnight in the *calendar's* zone; read
 * in the browser's it is 03:00 for a UTC calendar seen from Sofia and 15:00 for
 * an Auckland one — a number nobody chose, differing by calendar. Worse, an
 * all-day event's two ends read as the *same* clock whenever it covers a single
 * day, because `end_date` is the inclusive last day: the span came out zero and
 * Save refused a form with nothing on it visibly wrong.
 *
 * An hour apart, preserving the longstanding all-day-to-timed conversion.
 * New events may use the duration selected in General settings instead.
 */
const ALL_DAY_START = '09:00';
const ALL_DAY_END = '10:00';
const DAY_MS = 24 * 3_600_000;

// --- The guest list -------------------------------------------------------
//
// Pure, and here rather than in the component for the reason the module's own
// header gives: these are the fiddly parts, and every one of §5's rules is a
// predicate somebody could otherwise write inline in Svelte with no table
// under it.

/** An address as it is **compared**: trimmed and lower-cased. Google treats a
 *  mailbox case-insensitively, and so must anything deciding whether two rows
 *  are the same person — otherwise `Ana@X.com` typed beside a stored
 *  `ana@x.com` produces a second row, and that second row carries no answer,
 *  which is the RSVP reset the whole design is about wearing a duplicate's
 *  clothes. The Rust side compares the same way. */
const sameAddress = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * Whether `address` is one somebody could be mailed at.
 *
 * §5: **refused in the form, before Save**, the way every other invalid field
 * already is — never by a 400 coming back from Google, which arrives after the
 * user has stopped looking and says nothing they can act on.
 *
 * Deliberately not RFC 5322. That grammar admits quoted local parts, comments
 * and bare-hostname domains, and implementing it would refuse nothing anybody
 * types while accepting `ana@localhost` — an address Google will reject anyway.
 * This asks the three questions a typo actually fails: one `@`, something
 * either side of it, and a domain with a dot and no empty label.
 */
export const isAddress = (address: string): boolean => {
  const at = address.trim();
  if (/\s/.test(at)) return false;
  const parts = at.split('@');
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  if (local.length === 0 || domain.length === 0) return false;
  const labels = domain.split('.');
  return labels.length >= 2 && labels.every((l) => l.length > 0);
};

/**
 * `guests` with `address` invited, or **the very same array** when there is
 * nothing to do.
 *
 * §5: a duplicate address is a no-op rather than an error. Somebody typing a
 * name that is already on the list has asked for a state the list is already
 * in, and answering that with an error message would be the form inventing a
 * problem. Returning the identical array — not a copy — is what lets a caller
 * tell "nothing happened" from "nothing changed" by identity alone.
 *
 * An empty address is the same answer for the same reason: a stray Return in
 * the field is not a request.
 */
export const addGuest = (guests: Guest[], address: string): Guest[] => {
  const email = address.trim();
  if (email === '') return guests;
  if (guests.some((g) => sameAddress(g.email, email))) return guests;
  return [...guests, { email, optional: false }];
};

/** `guests` without `email`, matched however either side spells it. */
export const removeGuest = (guests: Guest[], email: string): Guest[] =>
  guests.filter((g) => !sameAddress(g.email, email));

/**
 * Whether the row for `email` may be taken off the event.
 *
 * §5: **the organizer cannot be removed.** Google refuses it outright, so a UI
 * offering it would produce a save that fails for a reason nothing on screen
 * explains. The control is absent rather than present-and-disappointing, which
 * is what this predicate is for.
 *
 * An event Google names no organizer for makes nobody protected rather than
 * everybody — the failure mode of getting that backwards is a guest list nobody
 * can edit at all.
 */
export const removableGuest = (email: string, organizerEmail: string | null): boolean =>
  organizerEmail === null || !sameAddress(email, organizerEmail);

/** `guests` with `email`'s optional flag flipped. §4: it rides on the same
 *  whole-list replace as everything else, so there is nothing special about it
 *  beyond being the one field of somebody else's row this form may author. */
export const toggledGuestOptional = (guests: Guest[], email: string): Guest[] =>
  guests.map((g) => (sameAddress(g.email, email) ? { ...g, optional: !g.optional } : g));

/**
 * Whether two guest lists say the same thing.
 *
 * **Order is not part of it.** The Rust side builds the array it would send in
 * the event's own stored order and compares that against what the event
 * already has, so a reorder comes out equal there — and a form that called a
 * reshuffle a change would send a whole-list replace on an event nobody edited.
 * Membership and the optional flag are the whole question.
 */
export const sameGuests = (a: Guest[], b: Guest[]): boolean => {
  if (a.length !== b.length) return false;
  return a.every((g) =>
    b.some((h) => sameAddress(g.email, h.email) && g.optional === h.optional));
};

/**
 * **How many people this save could mail.**
 *
 * Everyone on the resulting list, plus everyone removed from it, minus the
 * signed-in user. One rule, and deliberately with no `isEdit` branch in it: a
 * create is the case where `initial.guests` is empty and `selfEmail` is null,
 * which this arithmetic already handles.
 *
 * The **union** matters, not just the resulting list. A removal saved with
 * notify on sends that person a cancellation (guest-list spec §3), so they are
 * somebody this save could mail; counting only who is left would let a save
 * whose entire purpose was to un-invite somebody skip the question.
 *
 * **Yourself is excluded from both sides.** `sendUpdates=all` mails the other
 * guests, and telling somebody they are about to notify themselves is wrong —
 * the same exclusion `MoveConfirm` and `DeleteConfirm` make.
 *
 * This replaces `EventFormValue.guestCount`, which answered a narrower
 * question: who was on the event when the form *opened*. That was right while
 * a save could only change the event around a fixed guest list, and wrong in
 * two ways once the list itself became editable — hard-coded `0` on a create,
 * so the notify choice never appeared; and `0` for the first guest added to a
 * guestless event, which took the form's straight-to-save shortcut and mailed
 * a brand-new invitee nothing at all.
 */
export function mailableGuests(value: EventFormValue, initial: EventFormValue): number {
  const self = value.selfEmail;
  const mailable = new Set<string>();
  for (const g of [...value.guests, ...initial.guests]) {
    if (self !== null && sameAddress(g.email, self)) continue;
    // Keyed by the *compared* form, so one person spelled two ways is one
    // entry — the same normalisation `sameAddress` applies, which is what
    // every other rule in this block compares by.
    mailable.add(g.email.trim().toLowerCase());
  }
  return mailable.size;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** `yyyy-mm-dd` for an instant, read in the browser's zone. */
export const dateOf = (ms: number): string => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

/** `HH:MM` for an instant, read in the browser's zone. */
export const timeOf = (ms: number): string => {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

/**
 * The instant a `yyyy-mm-dd` and an `HH:MM` name in the browser's own zone.
 *
 * Built through the `Date` constructor rather than `Date.parse`, on purpose:
 * `new Date('2026-08-05T09:30')` is local but `new Date('2026-08-05')` is UTC,
 * so a string-parsing version of this would silently disagree with itself
 * between the timed and all-day paths by the host's offset.
 *
 * `NaN` for anything unparseable, which every caller here treats as "the user
 * has not finished typing" rather than as a time.
 */
export function toMs(date: string, time: string): number {
  const [y, m, d] = date.split('-').map(Number);
  const [hh, mm] = (time || '00:00').split(':').map(Number);
  if ([y, m, d, hh, mm].some((n) => !Number.isFinite(n))) return NaN;
  return new Date(y, m - 1, d, hh, mm, 0, 0).getTime();
}

/**
 * The span the form's working value describes, for the grid's live preview —
 * the block that appears where the event will land and moves as the times
 * are typed. `null` when there is nothing timed to draw: an all-day value
 * (the band is not previewed), a half-typed time, or an end at or before its
 * start. Display only; `whenOf` remains the save path's sole authority, and
 * in particular the `source*Ms` passthrough that preserves untouched seconds
 * is deliberately not reproduced here — a preview is not off by anything a
 * pixel can show.
 */
/**
 * What the open form currently describes, for the grid's live ghost.
 *
 * Two arms, mirroring the sweep that may have started it: a timed event is a
 * span of instants, an all-day one is a run of *dates*. The all-day arm
 * carries `yyyy-mm-dd` rather than milliseconds deliberately — the grid then
 * compares dates with dates, and no midnight that does not exist on a
 * spring-forward day can push the ghost off the day it belongs to.
 *
 * `color` is what the draft is drawn in: the colour of the calendar the form
 * would write to, so the tile on the grid says which calendar is being
 * created on (2026-08-31, by request). It rides on the ghost rather than
 * beside it because the two must never disagree — a shape from one value and
 * a colour from another is exactly the pairing that drifts. `null` when the
 * calendar has no colour to lend, which the grid draws as `--accent`.
 */
export type FormGhost =
  | { kind: 'timed'; startMs: number; endMs: number; color: string | null }
  | { kind: 'allDay'; firstDate: string; lastDate: string; color: string | null };

/**
 * The ghost for `value` in `color`, or `null` while it describes nothing
 * drawable.
 *
 * The colour is a parameter rather than something read off `value`: a form
 * value carries a calendar *id*, and which colour that id draws in is the
 * calendar list's business, not this module's.
 *
 * All-day used to yield nothing at all, so the ribbon a sideways sweep drew
 * vanished the instant the form opened — the draft stopped being visible at
 * exactly the moment the user was deciding about it (reported 2026-08-31).
 */
export function previewGhost(value: EventFormValue, color: string | null): FormGhost | null {
  if (value.isAllDay) {
    const firstDate = value.date;
    const lastDate = value.endDate || value.date;
    if (!firstDate || lastDate < firstDate) return null;
    return { kind: 'allDay', firstDate, lastDate, color };
  }
  const span = previewSpan(value);
  return span ? { kind: 'timed', ...span, color } : null;
}

export function previewSpan(
  value: EventFormValue,
): { startMs: number; endMs: number } | null {
  // Empty times checked here, not left to `toMs`: its `'' → 00:00` fallback
  // is right for its other callers and wrong for a preview, where a
  // mid-edit cleared field would snap the ghost to the top of the day.
  if (value.isAllDay || !value.start || !value.end) return null;
  const startMs = toMs(value.date, value.start);
  const endMs = toMs(value.endDate || value.date, value.end);
  if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || endMs <= startMs) return null;
  return { startMs, endMs };
}

// --- Date arithmetic on dates, not on instants ----------------------------
//
// The three below work on `yyyy-mm-dd` through `Date.UTC`, which has no
// transitions, and never touch the browser's zone. That is the point: the
// number of days between two dates is a property of the calendar, and it must
// not change because a clock somewhere went forward an hour. Doing the same
// sum in local milliseconds lands on 23:00 the previous day across a
// spring-forward, which `dateOf` then reports as the wrong date entirely.

/** A `yyyy-mm-dd` as a UTC instant, for counting only. `NaN` if unparseable. */
function utcOf(date: string): number {
  const [y, m, d] = date.split('-').map(Number);
  if ([y, m, d].some((n) => !Number.isFinite(n))) return NaN;
  return Date.UTC(y, m - 1, d);
}

/** `date` moved by whole days, back as `yyyy-mm-dd`. */
function addDays(date: string, days: number): string {
  const [y, m, d] = date.split('-').map(Number);
  const at = new Date(Date.UTC(y, m - 1, d + days));
  return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`;
}

/** A form date's weekday. Inputs produced by this module are real dates; the
 * Monday fallback only keeps a half-typed native date from breaking render. */
export function weekdayCodeForDate(date: string): WeekdayCode {
  const [y, m, d] = date.split('-').map(Number);
  const at = new Date(y, m - 1, d, 12);
  const index = Number.isFinite(at.getTime()) ? at.getDay() : 1;
  return WEEKDAY_OPTIONS[index]?.code ?? 'MO';
}

/** Stable Sunday-first order and no duplicates, regardless of click/NLP order. */
export function normalizedWeeklyDays(days: readonly WeekdayCode[]): WeekdayCode[] {
  const selected = new Set(days);
  return WEEKDAY_OPTIONS.map((day) => day.code).filter((day) => selected.has(day));
}

/** Set comparison in display order. Exported because recurrence's
 * unchanged-means-absent wire rule depends on the days as well as the select. */
export function sameWeeklyDays(a: readonly WeekdayCode[], b: readonly WeekdayCode[]): boolean {
  const left = normalizedWeeklyDays(a);
  const right = normalizedWeeklyDays(b);
  return left.length === right.length && left.every((day, i) => day === right[i]);
}

/** Value equality for the tagged repeat-ending union. */
export function sameRepeatEnd(a: RepeatEnd, b: RepeatEnd): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'on' && b.kind === 'on') return a.date === b.date;
  if (a.kind === 'after' && b.kind === 'after') return a.count === b.count;
  return true;
}

/** The repeat-ending error a Save must name instead of silently repairing. */
export function repeatEndProblem(value: EventFormValue): string | null {
  if (value.repeat === 'never' || value.repeat === CUSTOM_REPEAT
      || value.repeatEnd.kind === 'never') return null;
  if (value.repeatEnd.kind === 'after') {
    return Number.isInteger(value.repeatEnd.count) && value.repeatEnd.count >= 1
      ? null
      : 'End after must be at least 1 occurrence.';
  }
  const end = utcOf(value.repeatEnd.date);
  const start = utcOf(value.date);
  if (!Number.isFinite(end)) return 'Choose a valid repeat end date.';
  if (end < start) return 'The repeat end date cannot be before the event starts.';
  return null;
}

/**
 * Where the end date goes when the start date moves from `from` to `to`.
 *
 * The span is kept, which is what every calendar does and what stops a form
 * turning a valid event into a refused one because the user changed the date
 * it starts on and nothing else.
 *
 * `endDate` is returned untouched when anything will not parse, and when the
 * range is already backwards. Repairing a backwards range as a side effect of
 * an unrelated edit would be exactly the silent correction the Save guard
 * refuses to make — the user should be told, not quietly fixed.
 */
export function shiftedEndDate(from: string, to: string, endDate: string): string {
  const span = (utcOf(endDate) - utcOf(from)) / DAY_MS;
  if (!Number.isInteger(span) || span < 0 || !Number.isFinite(utcOf(to))) return endDate;
  return addDays(to, span);
}

/** Toggle one weekly button. The last selected day cannot be removed. If the
 * event's first date is no longer in the pattern, move the whole date span to
 * the next selected day; otherwise DTSTART would create a stray first
 * occurrence outside the cadence shown by the buttons. */
export function toggledWeeklyDay(value: EventFormValue, code: WeekdayCode): EventFormValue {
  const selected = value.weeklyDays.includes(code);
  if (selected && value.weeklyDays.length === 1) return value;
  const weeklyDays = normalizedWeeklyDays(selected
    ? value.weeklyDays.filter((day) => day !== code)
    : [...value.weeklyDays, code]);
  if (weeklyDays.includes(weekdayCodeForDate(value.date))) {
    return { ...value, weeklyDays };
  }
  const current = WEEKDAY_OPTIONS.findIndex((day) => day.code === weekdayCodeForDate(value.date));
  const offsets = weeklyDays.map((day) => {
    const target = WEEKDAY_OPTIONS.findIndex((candidate) => candidate.code === day);
    return (target - current + 7) % 7;
  }).filter((offset) => offset > 0);
  const nextDate = addDays(value.date, Math.min(...offsets));
  return {
    ...value,
    weeklyDays,
    date: nextDate,
    endDate: shiftedEndDate(value.date, nextDate, value.endDate),
    repeatEnd: value.repeatEnd.kind === 'on'
      ? { kind: 'on', date: shiftedEndDate(value.date, nextDate, value.repeatEnd.date) }
      : value.repeatEnd,
  };
}

/** The next half-hour boundary strictly after `nowMs`: 09:12 and 09:30 both
 *  give 09:30 and 10:00 respectively, so opening the form never offers a time
 *  that has already gone. */
export const nextHalfHour = (nowMs: number): number =>
  Math.floor(nowMs / HALF_HOUR_MS) * HALF_HOUR_MS + HALF_HOUR_MS;

/**
 * A form value for a brand-new event starting at `startMs`, on `calendarId`,
 * running until `endMs` — or the requested default duration, when nothing
 * names an end.
 *
 * The counterpart to `blankValue`: there the *clock* names the time, here the
 * *grid* does. A click on empty space in Day or Week view already knows which
 * instant it landed on, and substituting "the next half hour" for it would move
 * the event away from where the user pointed.
 *
 * **`endMs` is optional because a click genuinely does not name one.** A click
 * names a moment and the duration is the caller's stored default; a sweep
 * names both ends and the grid's answer is the one that must survive. Defaulting here
 * rather than at each call site keeps the fallback and the duration arithmetic
 * in one place.
 *
 * `endDate` is read off the end instant rather than copied from the start date,
 * which is the whole reason this is one function and not two lines at each call
 * site: a 23:30 event ends at 00:00 the following morning, and an `endDate` left
 * on the start's own day makes `endAfterStart` refuse a form the user never got
 * wrong.
 *
 * Both instants are kept as well as read, because a create has the same
 * boundary as an edit: an hour from 02:30 can land on a 02:30 that is *later*
 * across a fall-back, and re-parsing the pair `dateOf`/`timeOf` just produced
 * puts the end at or before the start — a form that opens already
 * refusing to save with no field on it visibly wrong. See `sourceStartMs`.
 */
/**
 * A blank **all-day** event over `firstDayMs`..`lastDayMs`, last day
 * inclusive — what a sweep across day columns asks for.
 *
 * Built beside `blankValueAt` rather than as a flag on it: an all-day value
 * differs in what its fields *mean*, not merely in one boolean. `endDate` is
 * the last day the event covers (see `EventFormValue.endDate`), which is the
 * same reading the grid drew, so the ghost and the saved event agree.
 *
 * The times are still filled in, from the same default a timed create would
 * have used. They are not shown while All day is ticked, but un-ticking it
 * must land on a sane hour rather than midnight-to-midnight — the boundary
 * `toggledAllDay` exists to protect.
 */
export function blankAllDayValue(
  firstDayMs: number,
  lastDayMs: number,
  calendarId: number | null,
  durationMinutes: number = DEFAULT_EVENT_MINUTES,
): EventFormValue {
  const base = blankValueAt(firstDayMs, calendarId, undefined, durationMinutes);
  return { ...base, isAllDay: true, date: dateOf(firstDayMs), endDate: dateOf(lastDayMs) };
}

export function blankValueAt(
  startMs: number,
  calendarId: number | null,
  endMs?: number,
  durationMinutes: number = DEFAULT_EVENT_MINUTES,
): EventFormValue {
  endMs ??= startMs + durationMinutes * MIN_MS;
  return {
    title: '',
    date: dateOf(startMs),
    endDate: dateOf(endMs),
    start: timeOf(startMs),
    end: timeOf(endMs),
    sourceStartMs: startMs,
    sourceEndMs: endMs,
    isAllDay: false,
    location: '',
    description: '',
    videoCall: null,
    calendarId,
    repeat: 'never',
    weeklyDays: [weekdayCodeForDate(dateOf(startMs))],
    repeatEnd: { kind: 'never' },
    recurrence: null,
    // Nobody, until the user types somebody in. The guest editor is on this
    // path now; `toEventInput` sends the list only when it differs from this
    // empty one, which for a create means "only when somebody was invited".
    guests: [],
    organizerEmail: null,
    selfEmail: null,
    // No rows: untouched means absent means the calendar's defaults
    // (reminders spec §3), which is what every other client gives a new event.
    popupReminders: [],
    emailReminders: [],
    remindersWereDefault: true,
    isEdit: false,
    isRecurring: false,
    isOrganizer: true,
    guestsCanModify: false,
  };
}

/**
 * A form value for a brand-new event: the next half hour, using the requested
 * default duration, on `calendarId`.
 *
 * `dayStartMs` is the day the user was looking at when they asked for a new
 * event. Passing one that is not today keeps the *time* — the next half hour —
 * and moves it to that day, which is what pressing `n` on next Tuesday and
 * getting "next Tuesday at the next half hour" means.
 *
 * **The move re-anchors: it asks what the moved pair names, and rebuilds the
 * whole value from that instant.** Not `{ ...at, date }` with the end date
 * shifted alongside, which is what this used to do and which built a value from
 * two sources — a day from one place, a clock from another, and a pair that was
 * therefore read off no instant at all. Two things went wrong with that, one of
 * them shipped for eight tasks:
 *
 *   - a pair naming **no instant** on the chosen day. Where a zone's midnight is
 *     skipped — America/Santiago 6 Sep 2026, Africa/Cairo 24 Apr 2026 — asking
 *     for a new event just after midnight offered 00:30–01:30, and 00:30 does
 *     not exist that day. Re-parsing normalised the *start* forward to 01:30
 *     while the end stayed at 01:30: a form that opened on a zero span,
 *     refused by Save, and no field on it visibly wrong. Rebuilding
 *     from `toMs(date, start)` gives 01:30–02:30 instead — an hour, forward,
 *     saveable — because the end is derived from the start's own instant rather
 *     than left where a separate string put it.
 *   - the two source instants going stale. Nothing had to *clear* them, because
 *     `instantOf` asks whether an instant still reads as the fields beside it,
 *     and a moved date stops reading as one — so they were silently ignored
 *     rather than used, and every time on this path was re-derived. Re-anchoring
 *     makes them true again instead: both come off the very instant the moved
 *     pair names.
 *
 * The day the end lands on is preserved by construction, which is what the
 * `shiftedEndDate` call here used to be for. The version before that wrote
 * `endDate: date` and was wrong for an hour every evening: asked for a new
 * event between 23:00 and 23:30 it offered 23:30–00:30 with both dates on today,
 * which `endAfterStart` reads as an end twenty-three hours *before*
 * the start. `blankValueAt` reads the end date off the end instant, so that
 * case — and the fall-back case where the hour after 23:30 is a *different*
 * civil hour — come out right without a second rule.
 *
 * Passing today's own day moves nothing, and re-anchoring then returns exactly
 * what `blankValueAt` already built. That is not a case to special-case; it is
 * the same arithmetic with a zero in it.
 */
export function blankValue(
  nowMs: number,
  calendarId: number | null,
  dayStartMs?: number,
  durationMinutes: number = DEFAULT_EVENT_MINUTES,
): EventFormValue {
  const at = blankValueAt(nextHalfHour(nowMs), calendarId, undefined, durationMinutes);
  if (dayStartMs === undefined) return at;
  return blankValueAt(toMs(dateOf(dayStartMs), at.start), calendarId, undefined, durationMinutes);
}

/**
 * One of an all-day detail's own dates, moved onto the occurrence that was
 * actually clicked.
 *
 * `date` is `detail.start_date` or `detail.end_date`, and `rowMs` is the
 * instant on the *same side of the same row*: `detail.start_ms` or
 * `detail.end_ms`. Both describe **the store row**, which for a recurring
 * series is the master — its dates are the series' DTSTART, not the day on
 * screen. `occurrenceMs` is the clicked block's own instant on that side.
 *
 * Taking `detail.start_date` verbatim would be `detail.start_ms` all over
 * again — the mistake `updateEvent`'s doc comment spends a paragraph on, and
 * the one §4 of the design names under "what must not regress". A daily all-day
 * series clicked on its third day would open the form showing its *first*, and
 * a title-only save would send that difference as a deliberate two-day move of
 * the occurrence, with `sendUpdates=all` behind it. `app.spec.ts`'s "editing
 * from an all-day chip sends the chip's own day" is the witness.
 *
 * The distance is measured between two instants on the same side of the same
 * event, both midnight in the same — the calendar's — zone, so the subtraction
 * has no zone in it at all and `Math.round` has only to absorb a daylight-saving
 * hour between two otherwise whole days. That is **not** the half-day slack
 * this file used to apply to a *single* instant read in a *foreign* zone: that
 * one silently absorbed the zone offset itself, which is how a trip's last day
 * came out right while its first day was a day early, and how a one-day trip
 * was saved as a two-day one.
 *
 * `Math.round` rather than `floor` or `ceil`, and the difference is not
 * cosmetic: a series straddling a spring-forward is 95 hours across four days
 * and `floor` answers three, a fall-back is 97 and `ceil` answers five. Either
 * opens the form a day off the chip that was clicked. Both are pinned, one
 * fixture each, in `eventform.spec.ts`.
 *
 * The bound it works within, stated rather than left to be discovered: rounding
 * recovers the right number of days while the offset changes by strictly less
 * than 12 hours between the two instants. Every daylight-saving transition is
 * inside that by an order of magnitude; a zone *redefinition* need not be.
 * `Pacific/Apia` moved from UTC−11 to UTC+13 in December 2011 — 30 December
 * never existed there — and this returns `2012-01-02` for a chip whose civil
 * date is `2012-01-03`. Historical and exotic, and not a UI artefact: the same
 * instants come out of `recur.rs`. Left unhandled deliberately, because the
 * alternative is civil date arithmetic in a zone this browser does not know.
 *
 * Each side is measured on its own rather than sharing one shift, because the
 * clicked block's `endMs` is carried for exactly that reason — see
 * `Occurrence`'s doc comment in `eventdetail.ts`.
 *
 * Exported, and used outside this module by exactly one other caller:
 * `EventPopover` asks the same question the form does — *which day is the block
 * I clicked on* — and answered it its own way until Task 6, with
 * `toLocaleDateString(detail.start_ms)`. That is both halves of this function's
 * reason for existing gone at once: a browser-zone reading of a date that has
 * no instant, taken off the **master's** row. The popover and the form then
 * disagreed on screen. One authority for the question, not two, is why this is
 * exported rather than copied — and living here rather than in `eventdetail.ts`
 * keeps `addDays` and the date-arithmetic block above it together.
 *
 * A `null` date on an all-day event is **not a case to handle**.
 * `event_detail_impl` fills both for every `is_all_day` row and neither for any
 * other, and the only substitute available here is a date derived from an
 * instant in the browser's zone — precisely the defect this function was
 * rewritten to remove. A `?? dateOf(startMs)` would keep that path alive with
 * every test green, so a detail that cannot be true throws instead of being
 * quietly accommodated.
 */
export function occurrenceDate(date: string | null, rowMs: number, occurrenceMs: number): string {
  // `!date`, not `=== null`. The type says `string | null`, but the type is a
  // claim about the wire, and the failure this guard exists to report is
  // exactly the one that falsifies it: a field that stops arriving under this
  // name reaches here as `undefined`, which `=== null` waves through — and
  // `addDays` then throws from somewhere else, with a message about splitting a
  // string rather than about a malformed detail. The diagnostic is the point.
  if (!date) {
    throw new Error('an all-day event with no date: an EventDetail carries both or neither');
  }
  return addDays(date, Math.round((occurrenceMs - rowMs) / DAY_MS));
}

/**
 * A form value for an existing event.
 *
 * `startMs`/`endMs` are the **clicked occurrence's** own times, not
 * `detail.start_ms`/`detail.end_ms`. For a recurring series those two are the
 * master row's DTSTART — see `updateEvent`'s doc comment in `eventdetail.ts`
 * for what happens when they are used as the anchor of an edit. The caller has
 * the clicked block's times already (it needs them for `occurrenceStartMs`
 * anyway); this function is not able to work them out and does not try.
 *
 * **An all-day event's dates are read, never derived.** They arrive on the
 * detail already in the calendar's own zone, and this browser does not know
 * what that zone is: `dateOf(startMs)` answers in the *browser's*, which for
 * any user east of the calendar is the previous day. See `EventDetail`'s
 * `start_date` for the whole shape of that defect. `occurrenceDate` above is
 * the only thing done to them, and it moves the day, never the zone.
 *
 * **The two instants are kept as well as read**, on both arms. For a timed
 * event that is the whole of Task 5's fix: `timeOf` cannot carry the seconds and
 * a repeated wall-clock hour cannot be re-parsed back to the pass it came from,
 * so a time nobody edited has to travel as the instant it arrived as. See
 * `sourceStartMs` and `instantOf`.
 *
 * On the **all-day** arm they are `null`, and the times beside them are
 * [`ALL_DAY_START`]/[`ALL_DAY_END`] rather than readings of those instants.
 *
 * An earlier version of this comment said the two answers "coincide" on the one
 * path that could ask — the user toggling All day *off* — and that sentence was
 * part of why §7.2 survived. They do not coincide. `timeOf(startMs)` on this arm
 * is the browser's reading of a midnight stored in the *calendar's* zone, which
 * is 03:00 or 15:00 or anything else; and for a single-day event, where
 * `end_date` is the same date as `start_date`, both ends read as the same clock
 * and the toggled-off span is **zero**, which Save refuses. The comment was true
 * of the conversions and false of the value, which is the most expensive kind of
 * comment to be right about.
 *
 * `null` rather than the instants for the same reason: `instantOf` passes a
 * source through when this browser reads it as exactly the date and time beside
 * it, and there is a calendar zone for which a stored midnight reads as exactly
 * `ALL_DAY_START` on exactly that date. Keeping the instants would make the
 * toggle's answer depend on the calendar's offset in a way no test would think
 * to look for. There is nothing to pass through on this arm, so it says so.
 */
export function valueFromDetail(
  detail: EventDetail,
  startMs: number,
  endMs: number,
): EventFormValue {
  const displayedDate = detail.is_all_day
    ? occurrenceDate(detail.start_date, detail.start_ms, startMs)
    : dateOf(startMs);
  const conferenceProvider = meetingProvider(detail.conference_uri);
  const locationUri = detail.conference_uri ? null : meetingUrl(detail.location);
  const locationProvider = meetingProvider(locationUri);
  const videoCall: VideoCall | null = detail.conference_uri
    ? {
        provider: conferenceProvider === 'Google Meet'
          ? 'googleMeet'
          : conferenceProvider === 'Zoom' ? 'zoom' : 'other',
        uri: detail.conference_uri,
        source: 'conference',
      }
    : locationUri
      ? {
          provider: locationProvider === 'Google Meet'
            ? 'googleMeet'
            : locationProvider === 'Zoom' ? 'zoom' : 'other',
          uri: locationUri,
          source: 'location',
        }
      : null;
  return {
    title: detail.title ?? '',
    date: displayedDate,
    // Inclusive on both arms, and inclusive already on the all-day one:
    // `end_date` is the last day a person would point at, worked out from the
    // exclusive `end_ms` once, on the Rust side, in the calendar's zone.
    endDate: detail.is_all_day
      ? occurrenceDate(detail.end_date, detail.end_ms, endMs)
      : dateOf(endMs),
    start: detail.is_all_day ? ALL_DAY_START : timeOf(startMs),
    end: detail.is_all_day ? ALL_DAY_END : timeOf(endMs),
    sourceStartMs: detail.is_all_day ? null : startMs,
    sourceEndMs: detail.is_all_day ? null : endMs,
    isAllDay: detail.is_all_day,
    location: detail.location ?? '',
    // Verbatim. Never through `sanitize.ts`: that module exists for *rendering*
    // a description, and running it here would put the stripped text into an
    // editable field and then save the stripped text back over the real event —
    // silently deleting whatever its author actually wrote.
    description: detail.description ?? '',
    videoCall,
    calendarId: detail.calendar_id,
    repeat: detail.repeat,
    weeklyDays: detail.weekly_days.length > 0
      ? normalizedWeeklyDays(detail.weekly_days)
      : [weekdayCodeForDate(displayedDate)],
    repeatEnd: { ...detail.repeat_end },
    recurrence: detail.recurrence,
    // **Everyone** — see `EventFormValue.guests` for how this differs from
    // `mailableGuests`.
    guests: detail.attendees.map((a) => ({ email: a.email, optional: a.optional })),
    organizerEmail: detail.organizer_email,
    selfEmail: detail.attendees.find((a) => a.is_self)?.email ?? null,
    // The *effective* rows (reminders spec §3): the event's own overrides, or
    // the calendar's defaults when it runs on those. The email rows ride
    // along from the same source, so an edit that flips this event to
    // explicit overrides carries them rather than silently dropping them.
    popupReminders: effectiveReminders(detail)
      .filter((r) => r.method === 'popup')
      .map((r) => r.minutes),
    emailReminders: effectiveReminders(detail).filter((r) => r.method === 'email'),
    remindersWereDefault: detail.reminders.use_default,
    isEdit: true,
    isRecurring: detail.is_recurring,
    isOrganizer: detail.is_organizer,
    guestsCanModify: detail.guests_can_modify,
  };
}

/**
 * A copied event, landed on the day a paste is aimed at — the create-form
 * value Ctrl+V opens.
 *
 * `copied` is `valueFromDetail`'s reading of the source occurrence; `blank`
 * is the value a plain `n` would have opened on the target day, and it is
 * the base on purpose: everything a paste should *not* carry is whatever a
 * new event gets. The repeat stays `'never'` and the recurrence `null` — a
 * paste is one event, not a second series — the reminder rows stay empty so
 * the target calendar's own defaults apply, and `isEdit`/`isRecurring`,
 * organizer and self are a create's. What crosses over is what the user
 * copied *for*: title, place, notes, the guest list (everyone on it except
 * yourself — see the filter below; the original organizer stays when it is
 * somebody else, because pasting a meeting is how you re-invite the same
 * room), whether it is all-day, and its shape in time — the clocks and
 * the day-span, moved onto `blank`'s day by the same `shiftedEndDate` a
 * date edit in the form uses.
 *
 * The source instants are dropped, not carried: the pair here is assembled
 * from two places — `blank`'s day, `copied`'s clock — which is precisely the
 * "read off no instant" case `sourceStartMs`'s own comment names. A clock
 * that does not exist on the target day (a spring-forward) is the form's
 * §7.3 to refuse and name, same as if it had been typed.
 */
export function pastedValue(copied: EventFormValue, blank: EventFormValue): EventFormValue {
  return {
    ...blank,
    title: copied.title,
    location: copied.location,
    description: copied.description,
    // Everyone except yourself. The creator of the pasted event is its
    // organizer, and an organizer sent as a guest row is an *invitation to
    // yourself*: Google files you as an unanswered attendee on your own
    // meeting, and the invite pass then rings for it (seen live 2026-08-20,
    // hours after paste shipped — copying a meeting you organize produced an
    // invitation toast for the copy). The source's organizer row stays when
    // it is somebody else — re-inviting the same room is half of why a
    // meeting gets pasted.
    guests: copied.guests
      .filter((g) => copied.selfEmail === null || g.email !== copied.selfEmail)
      .map((g) => ({ ...g })),
    isAllDay: copied.isAllDay,
    start: copied.start,
    end: copied.end,
    endDate: shiftedEndDate(copied.date, blank.date, copied.endDate),
    sourceStartMs: null,
    sourceEndMs: null,
  };
}

/** The rows the event actually runs on — its overrides, or the calendar's
 *  defaults when `use_default` (reminders spec §3). */
function effectiveReminders(detail: EventDetail): { method: string; minutes: number }[] {
  return detail.reminders.use_default
    ? detail.calendar_default_reminders
    : detail.reminders.overrides;
}

/**
 * The instant a form's civil pair names: `source` itself when the pair is still
 * `source`'s own reading, and `toMs`'s answer otherwise.
 *
 * The whole of the pass-through, in one expression, and deliberately a question
 * about the **value alone** rather than about the form's `initial`. `dateOf`
 * and `timeOf` are exactly what put `date` and `time` there, so "this pair
 * still reads as `source`" and "the user has not touched this time" are the
 * same statement — and asking it locally also answers it correctly for a time
 * that was edited and then typed back, which a comparison against `initial`
 * would call a change and re-derive.
 *
 * Living here, under `whenOf`, rather than in `toEventInput` is what keeps
 * `endAfterStart` honest: the Save guard asks `whenOf` too, so it judges the
 * very instants that will be sent. A pass-through one layer higher would leave
 * an event running 03:30 to 03:00 across a fall-back — thirty real minutes —
 * refused by a button reading a re-derived span of minus thirty.
 *
 * Each side is asked on its own, because each is a function of exactly two
 * civil fields and nothing else. Editing the end must not drag the start
 * through a round trip that loses its seconds or its pass.
 */
const instantOf = (source: number | null, date: string, time: string): number =>
  source !== null && dateOf(source) === date && timeOf(source) === time
    ? source
    : toMs(date, time);

/**
 * The `WhenInput` a value names — **dates for an all-day event, instants for a
 * timed one, and never a translation between the two.**
 *
 * There is no `instantsOf` any more, and that is the point of this plan rather
 * than a tidy-up: an all-day form value has no instants, in this browser or
 * anywhere else it could be asked. The version this replaces built a local
 * midnight from each date and then read the dates back off those instants, a
 * browser→browser round trip that returned the same answer in every ordinary
 * case and a different one across a daylight-saving transition — and which
 * existed only because the wire used to carry instants.
 *
 * The one conversion on the all-day arm is inclusive→exclusive: the form shows
 * the last day a person would point at, Google's `end.date` is the day after
 * it. `addDays` does that on whole days, once, with no zone in it.
 *
 * The timed arm goes through `instantOf` rather than `toMs` alone, so a time
 * nobody edited is sent as the instant it was read off instead of re-parsed
 * out of a civil pair that cannot express it. Read that function for why the
 * check belongs here and not in `toEventInput`.
 *
 * Either instant on the timed arm may be `NaN` while the user is mid-edit, and
 * either date on the all-day arm may be unparseable for the same reason.
 * `endAfterStart` is what every caller checks before any of it is sent.
 */
export function whenOf(value: EventFormValue): WhenInput {
  if (value.isAllDay) {
    return { kind: 'allDay', startDate: value.date, endDate: addDays(value.endDate, 1) };
  }
  return {
    kind: 'timed',
    startMs: instantOf(value.sourceStartMs, value.date, value.start),
    endMs: instantOf(value.sourceEndMs, value.endDate, value.end),
  };
}

/**
 * The value after the **All day** switch is flicked.
 *
 * A pure boolean flip, and that is the design rather than an omission.
 *
 * Toggling a switch must be reversible — flick it twice and you are where you
 * started — and the only way to guarantee that is to change nothing else. It
 * rules out the tempting reading of "toggle off", which is to convert the
 * all-day extent into instants: first day at 00:00 through to the day *after*
 * the last at 00:00. That is the more faithful answer to "how long is this
 * event", and it is not reversible. `endDate` on the all-day arm is the
 * **inclusive** last day, so reading that exclusive end back grows the trip by
 * one day on every round trip; a switch that lengthens an event each time you
 * flick it twice is worse than one that leaves you to type an end time.
 *
 * It is a function rather than the `bind:checked` it replaces so that the
 * property has somewhere to be tested. §7.2 happened because none of the four
 * all-day↔timed combinations had a spec, and a bare bind gives a spec nothing
 * to point at but a re-implementation of itself — which stays green however the
 * component later changes.
 *
 * **What makes the flip safe is upstream, in `valueFromDetail`.** The times an
 * all-day value carries are [`ALL_DAY_START`]/[`ALL_DAY_END`], a real pair, so
 * unticking the box reveals a span Save accepts. When they were read off the
 * stored instants they were a zone artefact, equal on both ends for any
 * single-day event, and the form arrived at a state its own Save button
 * refused.
 */
export const toggledAllDay = (value: EventFormValue, isAllDay: boolean): EventFormValue => ({
  ...value,
  isAllDay,
});

/**
 * Whether a civil pair names **no instant at all** in the browser's zone.
 *
 * On a spring-forward date the local clock jumps and an hour never happens: in
 * `America/Santiago` on 6 Sep 2026 there is no 00:30, and in
 * `America/New_York` on 8 Mar 2026 there is no 02:30. `new Date(y, m, d, h,
 * min)` does not refuse such a pair — it **normalises it forward by the size of
 * the gap**, so 00:30 comes back as 01:30 — which is why the check is "does
 * this pair read back as itself" rather than anything about transitions. A pair
 * that names an instant reads back as that pair; one that names none does not.
 *
 * The **time** alone decides it, and the date deliberately does not enter into
 * it. A gap late in the day does normalise across midnight — 23:45 on a zone
 * that springs forward at 23:30 lands on 00:45 the next day — but the time
 * differs there too, so comparing dates as well adds a clause that can never
 * be the one that fires. It would take a transition of a whole number of days
 * to make the time match while the date did not, and there is no such
 * transition. An earlier version compared both; a mutation sweep found the date
 * half survived deletion, which is what unreachable code looks like from the
 * outside.
 *
 * `false` for an unparseable pair. Half-typed is not nonexistent, and every
 * caller here already treats `NaN` as "not finished" rather than as a time.
 */
export const skippedLocalTime = (date: string, time: string): boolean => {
  // `date` is read by `toMs` — which day it is decides whether the hour exists
  // at all — even though only the time is compared afterwards.
  const ms = toMs(date, time);
  if (!Number.isFinite(ms)) return false;
  return timeOf(ms) !== time;
};

/**
 * Whether a civil pair names **two** instants — the autumn repeat, where the
 * clock goes back and an hour happens twice.
 *
 * The mirror of [`skippedLocalTime`] and, deliberately, **not an error**. The
 * time the user typed does exist; it exists twice, and `toMs` answers with the
 * earlier pass. There is nothing to refuse and nothing for them to correct, so
 * nothing is said — see `EventForm`'s save handler, which asks only about the
 * skipped case.
 *
 * It is exported and specified because the *silence* is the decision. §3 of the
 * design records an earlier version of this plan that proposed resolving
 * ambiguity explicitly to the first pass, and why that was wrong: it does not
 * close the drift it claims to, it makes the drift deliberate. `instantOf` is
 * what actually protects a second-pass instant, by never re-deriving one the
 * user did not touch. Warning here would put a message on the very case that is
 * already right.
 *
 * Probes an hour either side rather than reading a zone database: if some other
 * instant reads back as this same pair, the pair does not name one instant.
 * A transition of some other size — Lord Howe's half hour — is not detected,
 * which costs nothing while this answer is advisory.
 */
export const ambiguousLocalTime = (date: string, time: string): boolean => {
  const ms = toMs(date, time);
  if (!Number.isFinite(ms)) return false;
  const other = ms + 60 * MIN_MS;
  return dateOf(other) === date && timeOf(other) === time;
};

/** Which field a form value cannot honour, and what to say about it. */
export type TimeProblem = { field: 'start' | 'end'; message: string };

/**
 * The one thing a form value can be wrong about that **no field on it looks
 * wrong for**: a time typed into an hour that does not exist.
 *
 * This is §7.3. `toMs` normalises such a time forward silently, so the two
 * inputs go on showing exactly what was typed while naming instants an hour
 * from them — and when the shifted start lands on the instant the end already
 * names, the span is zero and Save dies with nothing to point at. Worse is the
 * case that does *not* die: two times both inside the gap both shift, the span
 * survives, and the event saves an hour from where it was typed without a word.
 *
 * **Refused, not repaired**, and that follows the ruling this form already
 * made rather than being a fresh opinion: an incoherent pair is answered
 * honestly instead of being fixed by moving a field the user did not touch.
 * Advancing the start to the far side of the gap — and dragging the end along
 * to keep the duration — would move a time somebody typed, silently, which is
 * the act every other rule here exists to prevent.
 *
 * `null` for an all-day value: its times are `ALL_DAY_START`/`ALL_DAY_END`,
 * never shown and never sent, so a gap they happened to land in would be a
 * message about a field that is not on screen.
 *
 * The start is reported before the end when both are unhonourable, because the
 * start is the field above.
 */
export function timeProblem(value: EventFormValue): TimeProblem | null {
  if (value.isAllDay) return null;

  const say = (which: string, time: string, date: string) =>
    `The ${which} time ${time} does not exist on ${date} — the clocks go forward that day. ` +
    'Pick a later time.';

  if (skippedLocalTime(value.date, value.start)) {
    return { field: 'start', message: say('start', value.start, value.date) };
  }
  if (skippedLocalTime(value.endDate, value.end)) {
    return { field: 'end', message: say('end', value.end, value.endDate) };
  }
  return null;
}

/**
 * Whether a value's end is strictly after its start.
 *
 * Asked of the very values `whenOf` will send, in the units it will send them
 * in, so the guard cannot pass a form that the wire then reads differently.
 *
 * The all-day end is exclusive by the time it gets here, which is what makes
 * "the same day" pass: a single-day all-day event starts on the 10th and ends
 * on the 11th. Only a last day genuinely *before* the first fails.
 *
 * Dates go through `utcOf` rather than being compared as strings. Both order
 * two well-formed `yyyy-mm-dd`s correctly, but `utcOf` also answers `NaN` for
 * one the user has not finished typing, and every comparison against `NaN` is
 * `false` — the same answer a Save button wants for "not yet valid" and
 * "invalid" alike. Compared as strings, an unparseable date reaches here as
 * `addDays`' `'NaN-NaN-NaN'`, which sorts *after* every real date and would
 * enable Save on a half-typed one.
 */
export const endAfterStart = (value: EventFormValue): boolean => {
  const when = whenOf(value);
  return when.kind === 'allDay'
    ? utcOf(when.endDate) > utcOf(when.startDate)
    : when.endMs > when.startMs;
};

/**
 * The `EventInput` a write command takes.
 *
 * `initial` is the value the form opened with, and is here only to decide
 * `repeat`'s three states, which is the one field that cannot be read off the
 * current value alone: **unchanged means absent**, and absent means "the user
 * did not touch Repeat, leave the existing rule alone". That is what keeps a
 * rule omacal cannot express — a `custom` one — from being overwritten by the
 * act of saving an unrelated field. Comparing against `initial` rather than
 * taking a "touched" flag from the caller is deliberate: a flag can be
 * forgotten, and the failure is silent and irreversible.
 *
 * **`repeat` is the only field `initial` decides.** The other thing that turns
 * on "did the user touch this" — whether a time is sent as the instant it came
 * from or re-derived from a civil pair — deliberately does *not* ask `initial`,
 * and does not happen here. It is `instantOf`, under `whenOf`, so that the Save
 * guard and the wire cannot disagree about what a form contains.
 *
 * Empty strings become `null`, not `""`: `changed_fields` sends a `null` to
 * clear a field, and an empty summary sent as `""` would leave a Google event
 * titled with an empty string rather than untitled.
 */
export function toEventInput(value: EventFormValue, initial: EventFormValue): EventInput {
  const blank = (s: string) => (s.trim() === '' ? null : s);
  const repeatChanged = value.repeat !== initial.repeat
    || (value.repeat === 'weekly' && !sameWeeklyDays(value.weeklyDays, initial.weeklyDays))
    || (value.repeat !== 'never' && value.repeat !== CUSTOM_REPEAT
      && !sameRepeatEnd(value.repeatEnd, initial.repeatEnd));
  const authoredRepeat = {
    repeat: value.repeat,
    ...(value.repeat === 'weekly'
      ? { weeklyDays: normalizedWeeklyDays(value.weeklyDays) }
      : {}),
    ...(value.repeat !== 'never' && value.repeat !== CUSTOM_REPEAT
      && value.repeatEnd.kind !== 'never'
      ? { repeatEnd: { ...value.repeatEnd } }
      : {}),
  };
  const repeatFields = value.isEdit
    ? (repeatChanged
        ? authoredRepeat
        : {})
    : (value.repeat === 'never'
        ? {}
        : authoredRepeat);
  // A create has no location-backed call to preserve. Quick-add hands the full
  // editor an already-parsed `initial`, so treating that as a server-side
  // before would make its Zoom URI look unchanged and omit it from Location.
  const location = locationForVideoCall(
    value.location, value.videoCall, value.isEdit ? initial.videoCall : null,
  );
  const conference = conferenceChange(value, initial);
  return {
    summary: blank(value.title),
    location: blank(location),
    description: blank(value.description),
    when: whenOf(value),
    tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
    // A create has no rule to leave untouched. This matters for quick-add and
    // paste-style seeds whose repeat value is already populated when the full
    // editor opens: comparing only with `initial` would omit that real choice.
    ...repeatFields,
    ...(conference ? { conference } : {}),
    // **Unchanged means absent**, the same three-state `repeat` runs on, and
    // on an edit it is load bearing rather than tidy: `attendees` is a
    // whole-list replace on Google's side, so a payload that carried the list
    // on every save would rewrite every attendee of every event this form
    // touched, from whatever omacal last read — quietly un-inviting anyone
    // added elsewhere since. Absent is a PATCH saying "leave the list alone".
    //
    // It is also what makes a **drag** structurally unable to change a guest
    // list. A drag builds its input from this same value with only the times
    // moved, so the two lists compare equal and no `guests` key is sent at all
    // — the same shape as `sendUpdates: 'none'`, one field along.
    //
    // A **create** does not get that rule, because it has no server-side list
    // to leave alone: there, "absent" does not mean "unchanged", it means
    // *nobody*. The difference went unseen while the only way guests reached
    // a create's `initial` was typing them in — typed guests differ from the
    // blank list and were sent either way — and it surfaced the day paste
    // arrived (2026-08-20): a pasted form opens with the copied invitees
    // already in `initial`, the two lists compare equal, and the diff rule
    // read a full guest list as nothing to say. Everyone on the list of a
    // new event is being invited by definition.
    ...(value.isEdit
      ? (sameGuests(value.guests, initial.guests) ? {} : { guests: value.guests })
      : (value.guests.length > 0 ? { guests: value.guests } : {})),
    // The same rule a third time, and here the object is a whole replace too:
    // absent leaves the event's reminders exactly as they are, which is the
    // only safe thing for rows nobody touched (reminders spec §2). A change
    // sends explicit overrides — the edited popups plus the preserved email
    // rows — never `useDefault: true`, because the form edits rows and a row
    // is an override by definition.
    ...(sameReminders(value, initial)
      ? {}
      : {
          reminders: {
            useDefault: false,
            overrides: [
              ...value.popupReminders.map((minutes) => ({ method: 'popup', minutes })),
              ...value.emailReminders,
            ],
          },
        }),
  };
}

/** Whether two form calls describe the same conferencing choice. `source` is
 * deliberately ignored: a Zoom URL read from structured data and the same URL
 * pasted into the Zoom field still describe the same call. */
export function sameVideoCall(a: VideoCall | null, b: VideoCall | null): boolean {
  if (a === null || b === null) return a === b;
  return a.provider === b.provider && (a.uri ?? '') === (b.uri ?? '');
}

/** The conference instruction for the backend. Manual meeting URLs are kept
 * in `location`, where the existing Join path already recognises them. */
function conferenceChange(
  value: EventFormValue,
  initial: EventFormValue,
): EventInput['conference'] | undefined {
  const current = value.videoCall;

  if (!value.isEdit) {
    if (current?.provider === 'googleMeet' && current.uri === null) return 'googleMeet';
    if (current?.provider === 'zoom' && current.uri === null) return 'zoom';
    return undefined;
  }
  if (sameVideoCall(current, initial.videoCall)) return undefined;
  if (current?.provider === 'googleMeet' && current.uri === null) return 'googleMeet';
  if (current?.provider === 'zoom' && current.uri === null) return 'zoom';
  if (initial.videoCall?.source === 'conference') return 'none';
  return undefined;
}

/** Removes a meeting URL that used to live in Location without disturbing a
 * real place beside it. This is intentionally conservative: only the exact
 * URI represented by the previous video-call value is removed. */
function withoutCall(raw: string, call: VideoCall | null): string {
  if (call?.source !== 'location' || !call.uri || !raw.includes(call.uri)) return raw.trim();
  const provider = call.provider === 'zoom' ? 'Zoom' : call.provider === 'googleMeet'
    ? 'Google Meet' : '';
  return raw
    .replace(call.uri, '')
    .replace(provider ? new RegExp(`\\b${provider.replace(' ', '\\s+')}\\s*:?`, 'i') : /$^/, '')
    .replace(/^[\s,;:·–—-]+|[\s,;:·–—-]+$/g, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

/** The place sent on the wire after a video-call selection. Google Meet uses
 * structured conference data. A manual Zoom/Meet URL is appended only once;
 * replacing or removing a location-backed call first removes its exact URI. */
export function locationForVideoCall(
  raw: string,
  current: VideoCall | null,
  initial: VideoCall | null,
): string {
  if (sameVideoCall(current, initial)) return raw.trim();
  const clean = withoutCall(raw, initial);
  if (!current?.uri || current.source === 'conference') return clean;
  if (clean.includes(current.uri)) return clean;
  const label = current.provider === 'zoom' ? 'Zoom' : current.provider === 'googleMeet'
    ? 'Google Meet' : 'Video call';
  return clean ? `${clean} · ${label}: ${current.uri}` : `${label}: ${current.uri}`;
}

/** A form-level validation message for conferencing, kept pure so quick-add
 * and the full editor enforce the same rule. */
export function videoCallProblem(
  value: EventFormValue,
  calendarProvider: string,
  zoomCanCreate = false,
): string | null {
  const call = value.videoCall;
  if (!call) return null;
  if (call.provider === 'googleMeet' && call.uri === null && calendarProvider !== 'google') {
    return 'Google Meet can only be added to a Google calendar.';
  }
  if (call.provider === 'zoom') {
    if (!call.uri && value.isAllDay) {
      return 'Automatic Zoom meetings need a start time. Paste an existing Zoom link for an all-day event.';
    }
    if (!call.uri && !zoomCanCreate) {
      return 'Connect Zoom in Settings → Accounts, or paste an existing Zoom meeting link.';
    }
    if (call.uri && meetingProvider(call.uri) !== 'Zoom') {
      return 'That is not a zoom.us meeting link.';
    }
  }
  return null;
}

/**
 * Whether the reminder rows still read as they opened. Sorted before
 * comparing: the form can only add, remove or edit rows, so order carries no
 * meaning, and a stored order Google happens to permute must not read as an
 * edit — that would flip an event from "calendar defaults" to frozen copies
 * of them on a save that touched the title.
 */
function sameReminders(value: EventFormValue, initial: EventFormValue): boolean {
  const sorted = (xs: number[]) => [...xs].sort((a, b) => a - b);
  const a = sorted(value.popupReminders);
  const b = sorted(initial.popupReminders);
  return a.length === b.length && a.every((m, i) => m === b[i]);
}

// --- Showing an RRULE in words -------------------------------------------
//
// Display only. Nothing below decides whether a rule is one omacal can
// express — `EventDetail.repeat` already carries that answer — and nothing
// below is ever fed back into a value that gets saved. Its one job is to let
// the disabled `Custom` entry say what the rule the user is about to overwrite
// actually does.

/** A rule longer than this is not one anybody reads in a dropdown, and the
 *  text arrives from whoever created the event. Caps the RRULE line offered
 *  for parsing and the verbatim fallback — not an EXDATE list, whose length
 *  never reaches the description (`ruleInWords` shows it as a count). */
const MAX_RULE_LENGTH = 200;

/** Singular phrase, plural noun. */
const FREQ_WORDS: Record<string, [string, string]> = {
  SECONDLY: ['Every second', 'seconds'],
  MINUTELY: ['Every minute', 'minutes'],
  HOURLY: ['Hourly', 'hours'],
  DAILY: ['Daily', 'days'],
  WEEKLY: ['Weekly', 'weeks'],
  MONTHLY: ['Monthly', 'months'],
  YEARLY: ['Yearly', 'years'],
};

const DAY_WORDS: Record<string, string> = {
  MO: 'Monday', TU: 'Tuesday', WE: 'Wednesday', TH: 'Thursday',
  FR: 'Friday', SA: 'Saturday', SU: 'Sunday',
};

const ORDINAL_WORDS: Record<string, string> = {
  '1': 'the first', '2': 'the second', '3': 'the third', '4': 'the fourth',
  '5': 'the fifth', '-1': 'the last', '-2': 'the second-to-last',
};

/** The parts this function claims to understand. A rule carrying anything else
 *  is shown verbatim instead — see `ruleInWords`. */
const KNOWN_PARTS = new Set(['FREQ', 'INTERVAL', 'BYDAY', 'COUNT', 'UNTIL']);

/** `MO` → `Monday`, `-1FR` → `the last Friday`. `null` when it is neither. */
function dayInWords(token: string): string | null {
  const m = /^([+-]?\d+)?(MO|TU|WE|TH|FR|SA|SU)$/i.exec(token.trim());
  if (!m) return null;
  const day = DAY_WORDS[m[2].toUpperCase()];
  if (!m[1]) return day;
  const ordinal = ORDINAL_WORDS[String(Number(m[1]))];
  return ordinal ? `${ordinal} ${day}` : null;
}

/** `20261231` or `20261231T235959Z` → `Dec 31, 2026`. `null` when it is
 *  neither — an UNTIL this cannot read must not become a wrong date. */
function untilInWords(value: string): string | null {
  const m = /^(\d{4})(\d{2})(\d{2})(T\d{6}Z?)?$/.exec(value.trim());
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const ms = Date.UTC(y, mo - 1, d);
  const at = new Date(ms);
  // `Date.UTC` normalises rather than rejects: month 13 rolls into next year,
  // 31 February into March. `20261345` came back as "Feb 14, 2027" — a
  // confident wrong date, in the one control whose job is to say what rule is
  // about to be replaced. Only a value that survives the round trip is real.
  if (at.getUTCFullYear() !== y || at.getUTCMonth() !== mo - 1 || at.getUTCDate() !== d) {
    return null;
  }
  return at.toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC',
  });
}

const listInWords = (items: string[]): string =>
  items.length < 2
    ? (items[0] ?? '')
    : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;

/** The body of one RRULE (no `RRULE:` prefix) in English, or `null` the
 *  moment it meets a part it does not model, an unreadable value, or no
 *  `FREQ` at all — never a partial description. */
function rruleBodyInWords(body: string): string | null {
  const parts = new Map<string, string>();
  for (const part of body.split(';')) {
    if (part.trim() === '') continue;
    const eq = part.indexOf('=');
    if (eq <= 0) return null;
    const key = part.slice(0, eq).trim().toUpperCase();
    if (!KNOWN_PARTS.has(key) || parts.has(key)) return null;
    parts.set(key, part.slice(eq + 1).trim());
  }

  const freq = FREQ_WORDS[(parts.get('FREQ') ?? '').toUpperCase()];
  if (!freq) return null;

  const interval = Number(parts.get('INTERVAL') ?? '1');
  if (!Number.isInteger(interval) || interval < 1) return null;
  let words = interval > 1 ? `Every ${interval} ${freq[1]}` : freq[0];

  const byday = parts.get('BYDAY');
  if (byday !== undefined) {
    const tokens = byday.split(',');
    const days = tokens.map(dayInWords);
    if (days.some((d) => d === null)) return null;
    words += ` on ${listInWords(days as string[])}`;
  }

  const count = parts.get('COUNT');
  if (count !== undefined) {
    const n = Number(count);
    if (!Number.isInteger(n) || n < 1) return null;
    words += `, ${n} time${n === 1 ? '' : 's'}`;
  }

  const until = parts.get('UNTIL');
  if (until !== undefined) {
    const when = untilInWords(until);
    if (!when) return null;
    words += `, until ${when}`;
  }

  return words;
}

/** How many dates an `EXDATE`/`RDATE` line names, or `null` for a line this
 *  cannot honestly count: any other property, a `PERIOD`-valued `RDATE`
 *  (those name spans, not dates), or a value that does not read as an
 *  iCalendar date or date-time. Values are shape-checked but not interpreted:
 *  a count needs to know how many they are, not when they fall. */
function datesListed(line: string): { kind: 'EXDATE' | 'RDATE'; count: number } | null {
  const m = /^(EXDATE|RDATE)[;:]/i.exec(line);
  if (!m) return null;
  const colon = line.indexOf(':');
  if (colon < 0) return null;
  if (line.slice(0, colon).toUpperCase().includes('VALUE=PERIOD')) return null;
  const values = line.slice(colon + 1).split(',').map((v) => v.trim());
  if (!values.every((v) => /^\d{8}(T\d{6}Z?)?$/.test(v))) return null;
  return { kind: m[1].toUpperCase() as 'EXDATE' | 'RDATE', count: values.length };
}

/**
 * A recurrence, described in English.
 *
 * Falls back to the raw text — never to a partial description — whenever it
 * meets anything it does not fully model. That is the whole design: showing
 * "Monthly" for `FREQ=MONTHLY;BYMONTHDAY=15;BYSETPOS=1` would tell the user
 * the rule is something simpler than it is, immediately before offering to
 * replace it. The raw rule is ugly and always honest.
 *
 * `recurrence` is newline-joined (`convert.rs`), and the commonest `custom`
 * in real data is not an exotic RRULE at all: it is an ordinary rule plus an
 * `EXDATE` naming occurrences somebody deleted — which is *why* it is custom.
 * That blob is described too, with the deletions as a count — "Monthly on the
 * third Wednesday, except 6 dates". The count is what keeps the description
 * honest: the deletions stay in the sentence, without the six raw timestamps
 * the popover was caught printing where its cadence line goes (2026-08-16).
 * Exactly one `RRULE` line, every other line a countable `EXDATE`/`RDATE`,
 * or the whole blob shows verbatim.
 */
export function ruleInWords(rule: string | null): string {
  if (!rule) return '';
  const raw = rule.trim();
  if (raw === '') return '';

  // The verbatim fallback: visibly cut over the cap. A cut rule still *looks*
  // like a rule; a cut description does not look cut — which is why nothing
  // over the cap is ever offered for parsing (below), only shown.
  const verbatim = raw.length > MAX_RULE_LENGTH ? `${raw.slice(0, MAX_RULE_LENGTH)}…` : raw;

  const lines = raw.split('\n').map((l) => l.trim()).filter((l) => l !== '');

  // One line: the rule itself, its `RRULE:` prefix optional. Never parsed
  // over the cap: describing a rule's readable half in full, confident
  // English — `…;BYSETPOS=2` at character 205 silently gone — is the failure
  // mode the cap exists for.
  if (lines.length === 1) {
    const line = lines[0];
    if (line.length > MAX_RULE_LENGTH) return verbatim;
    const body = /^rrule:/i.test(line) ? line.slice('RRULE:'.length) : line;
    return rruleBodyInWords(body) ?? verbatim;
  }

  // A multi-line blob: the rule, plus the deletions/additions that made it
  // custom. An EXDATE list has no cap of its own — a long-lived series with
  // a dozen deletions is exactly the event this path exists for, and its
  // count stays one word however long the list grows.
  let ruleBody: string | null = null;
  let added = 0;
  let except = 0;
  for (const line of lines) {
    if (/^rrule:/i.test(line)) {
      // Two rules in one recurrence is legal (events.rs models it) and
      // beyond this vocabulary.
      if (ruleBody !== null || line.length > MAX_RULE_LENGTH) return verbatim;
      ruleBody = line.slice('RRULE:'.length);
      continue;
    }
    const dates = datesListed(line);
    if (dates === null) return verbatim;
    if (dates.kind === 'RDATE') added += dates.count;
    else except += dates.count;
  }
  // Dates with no rule to hang them on: nothing to describe them against.
  if (ruleBody === null) return verbatim;

  let words = rruleBodyInWords(ruleBody);
  if (words === null) return verbatim;
  if (added > 0) words += `, plus ${added} added date${added === 1 ? '' : 's'}`;
  if (except > 0) words += `, except ${except} date${except === 1 ? '' : 's'}`;
  return words;
}
