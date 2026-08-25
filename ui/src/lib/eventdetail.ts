import { invoke } from '@tauri-apps/api/core';

export type Attendee = {
  email: string;
  display_name: string | null;
  response_status: string;
  optional: boolean;
  is_self: boolean;
};

/** iCalendar's weekday vocabulary, shared by the weekly picker and the
 * command boundary. Sunday-first display order is defined in `eventform.ts`. */
export type WeekdayCode = 'SU' | 'MO' | 'TU' | 'WE' | 'TH' | 'FR' | 'SA';

/** The finite endings RFC 5545 allows on a recurrence. `never` means the
 * series is unbounded; `on` is an inclusive calendar date. */
export type RepeatEnd =
  | { kind: 'never' }
  | { kind: 'on'; date: string }
  | { kind: 'after'; count: number };

export type EventDetail = {
  id: number;
  calendar_id: number;
  title: string | null;
  description: string | null;
  location: string | null;
  conference_uri: string | null;
  start_ms: number;
  end_ms: number;
  /**
   * The first day an all-day event covers, `yyyy-mm-dd`, already read in the
   * **calendar's** zone. `null` for a timed event, which has no date of its own.
   *
   * Read it; never derive one from `start_ms` here. The store holds an instant
   * for an all-day event too — midnight in the calendar's zone, because Google
   * sends a bare `date` and sync resolves it against `calendars.timezone` — and
   * this browser has no idea what that zone is. `dateOf(start_ms)` answers in
   * the *browser's* zone, which for any user east of the calendar is the
   * previous day: a trip on the 10th opens the form showing the 9th, and Save
   * writes a two-day event starting the 9th to every guest with
   * `sendUpdates=all`.
   */
  start_date: string | null;
  /**
   * The **last** day an all-day event covers — the day the user would point at
   * — `yyyy-mm-dd`, in the same zone as `start_date` and `null` on the same
   * condition.
   *
   * **Inclusive**, unlike `end_ms` and unlike the `endDate` a write sends: both
   * of those are the exclusive midnight *after* the last day. This is the form's
   * own `endDate` shape, so it goes straight into a form value with no arithmetic
   * — a single-day event reports the same date twice.
   */
  end_date: string | null;
  is_all_day: boolean;
  is_recurring: boolean;
  /** The raw `RRULE`, carried through unchanged so the UI can show a rule it
   *  cannot represent back to the user in words. Display only — never parse it
   *  to decide what the app can express; that is `repeat`'s job. */
  recurrence: string | null;
  /** Which Repeat option represents `recurrence` completely, or `'custom'` for a
   *  rule this app cannot express.
   *
   *  Computed on the Rust side by `write::repeat_from_rrule`, which matches
   *  base rules exactly and strictly parses only plain weekly BYDAY rules.
   *  Never re-derive it here: one authority decides what omacal can express,
   *  and a second copy in TypeScript drifts the moment either side gains an
   *  option. It fails
   *  silently when it does — an unrepresentable rule read as a representable
   *  one means the next save rewrites "every 2nd Tuesday" as "weekly", and
   *  mails the whole guest list about it. */
  repeat: string;
  /** Weekdays from an exactly representable weekly `BYDAY` rule. Empty for a
   *  plain weekly rule, whose day comes from DTSTART. */
  weekly_days: WeekdayCode[];
  repeat_end: RepeatEnd;
  color: string | null;
  organizer_email: string | null;
  self_response: string | null;
  can_respond: boolean;
  can_edit: boolean;
  attendees: Attendee[];
  /** What this event asks for: the calendar's defaults, or its own
   *  overrides — the two fields are alternatives (reminders spec §3). */
  reminders: { use_default: boolean; overrides: { method: string; minutes: number }[] };
  /** What "the calendar's defaults" means for this event, so the form can
   *  show the effective rows when `use_default`. */
  calendar_default_reminders: { method: string; minutes: number }[];
};

export const getEventDetail = (id: number) => invoke<EventDetail>('event_detail', { id });

/**
 * An `EventDetail` together with the one thing it cannot supply: which
 * occurrence of it the user actually clicked.
 *
 * Every write command below takes an `occurrenceStartMs`, and for a recurring
 * series the detail's own `start_ms` is the wrong answer to that question every
 * single time (see `respondToEvent` and `updateEvent` below). Carrying the two
 * together, from the grid that had the `UiEvent` in its hand, is what stops a
 * caller further up the stack having to remember which of two plausible numbers
 * is the safe one.
 *
 * `startMs` and `endMs` are the clicked block's own, taken from the `UiEvent` —
 * `endMs` for the same reason as `startMs`: `valueFromDetail` needs the
 * occurrence's real span, and deriving it from the master's would be wrong for
 * any occurrence whose duration crosses a daylight-saving transition the
 * master's does not.
 */
export type Occurrence = {
  detail: EventDetail;
  startMs: number;
  endMs: number;
};

/**
 * What `create_event` takes on the Rust side (`write::EventInput`) — the
 * UI's own vocabulary, not an RRULE: `repeat` is one of `'never'`,
 * `'daily'`, `'weekdays'`, `'weekly'`, `'monthly'`, `'yearly'`, mapped to an
 * actual rule by `write::rrule_for` (refined by `weeklyDays` for BYDAY).
 * Omit it entirely to create a one-off
 * event; `'never'` and "omitted" are the same thing on a create, since there
 * is no existing rule to leave alone.
 */
export type EventInput = {
  summary: string | null;
  location: string | null;
  description: string | null;
  when: WhenInput;
  tz: string;
  repeat?: string;
  /** Present with `repeat: 'weekly'` when the user chose the weekly pattern. */
  weeklyDays?: WeekdayCode[];
  /** Present when the repeat controls were created or changed. */
  repeatEnd?: RepeatEnd;
  /**
   * The guest list the event should end up with, or **absent** when the user
   * did not touch it.
   *
   * The distinction is the whole of guest-list spec §2 on this side of the
   * wire. Google's `attendees` is a whole-list replace, so absent is the only
   * safe thing to send for a list nobody edited: present, it would rewrite
   * every attendee from whatever omacal last read, un-inviting anyone added
   * elsewhere since. `[]` is a third thing again — remove everyone — and a
   * caller that conflated it with absent would make removing the last guest
   * impossible.
   *
   * Each entry carries an address and an optional flag and nothing else. What
   * each person already answered is echoed back on the Rust side from what is
   * stored (`events::attendees_for_edit`); there is deliberately no field here
   * through which a form could overwrite an RSVP.
   */
  guests?: { email: string; optional: boolean }[];
  /**
   * The reminder settings the event should end up with, or **absent** when
   * the user did not touch them — `guests`' distinction again, and again
   * load bearing: Google's `reminders` is a whole-object replace (reminders
   * spec §2). Overrides carry both the edited popup rows and the event's
   * email rows echoed back verbatim, because a replace without them would
   * strip them.
   */
  reminders?: { useDefault: boolean; overrides: { method: string; minutes: number }[] };
  /** A conference change, or absent when the event's conference is untouched.
   * `googleMeet` asks Google to mint a fresh, unique Meet conference;
   * `none` removes structured conference data. Zoom links travel in `location`
   * because creating a Zoom meeting requires a separately-authorized Zoom
   * account, which omacal does not currently hold. */
  conference?: 'googleMeet' | 'none';
};

/**
 * When the event happens, mirroring `write::WhenInput`. **An all-day event
 * carries dates and no zone at all** — that is Google's own model, and the
 * reason this is a union rather than three loose fields: the old shape passed
 * an instant and a flag, and the two sides of the boundary turned the same date
 * into an instant in *different* zones, so a date nobody touched came back
 * looking moved.
 *
 * `kind` is the discriminant Rust reads (`#[serde(tag = "kind")]`), and every
 * name here has to match the wire exactly — Rust deliberately carries no
 * default, so a payload it cannot read fails the command rather than becoming a
 * timed event at the epoch. `write.rs`'s
 * `the_payload_the_ui_sends_deserializes_as_written` pins these strings from
 * the other side.
 *
 * `endDate` is **exclusive** — the day after the last one — as on Google's wire
 * and in the store. The form shows the inclusive last day, so exactly one
 * conversion happens between the two, in `eventform.ts`.
 */
export type WhenInput =
  | { kind: 'timed'; startMs: number; endMs: number }
  | { kind: 'allDay'; startDate: string; endDate: string };

/** A freshness check on an already-open popover, not a load — see `WeekGrid`,
 *  which fires this after paint and ignores a rejection. */
export const refreshEvent = (id: number) => invoke<EventDetail>('refresh_event', { id });

/**
 * `occurrenceStartMs` is the `start_ms` of the block that was actually
 * clicked — the `UiEvent` from the grid, never `detail.start_ms`.
 *
 * For a recurring series, every expanded occurrence shares its master's
 * store row id (`commands::to_ui`), and `event_detail_impl` sets a
 * `EventDetail`'s `start_ms` to `event.start_utc`, which for that master row
 * *is* the series DTSTART. Passing `detail.start_ms` here type-checks and
 * reads correctly, and it silently patches occurrence #0 for everyone —
 * `sendUpdates=all`, so the wrong date's decline goes out to the whole guest
 * list. The caller must thread the clicked block's own `start_ms` through
 * instead, alongside the anchor rect it already carries for positioning.
 */
export const respondToEvent = (
  id: number,
  response: string,
  scope: 'this' | 'all',
  occurrenceStartMs: number,
) => invoke<EventDetail>('respond_to_event', { id, response, scope, occurrenceStartMs });

/**
 * `sendUpdates` is Google's own vocabulary — `'all'` or `'none'` — and is
 * required rather than defaulted, for the reason `SendUpdates`' own doc comment
 * gives: this is the one value where nobody choosing is how somebody gets an
 * email they should not have. The form asks whenever there is anybody to ask
 * about and answers `'none'` when there is not, so `'all'` reaches here only
 * where a person chose it.
 */
export const createEvent = (calendarId: number, fields: EventInput, sendUpdates: SendUpdates) =>
  invoke<EventDetail>('create_event', { calendarId, fields, sendUpdates });

/**
 * Saves an edit and returns the freshly-written detail.
 *
 * `occurrenceStartMs` carries the same rule as `respondToEvent`'s, and carries
 * it harder: it is the clicked block's own `start_ms`, never
 * `detail.start_ms`. For a recurring series that second value is the master
 * row's DTSTART, and an edit aimed at it patches occurrence #0 — with the
 * whole event as the payload and `sendUpdates=all` mailing the result to every
 * guest.
 *
 * `fields` is the form's whole state, not a diff: what the user actually
 * changed is worked out on the Rust side, against the event as it was loaded,
 * so a field nobody touched is never sent. `repeat` left out means "the user
 * did not touch Repeat" and the event's existing rule — which may be one this
 * app cannot express — is left alone.
 *
 * **The invariant the whole thing rests on:** when the user has not touched
 * the time, `fields.startMs` must equal `occurrenceStartMs` *exactly*. The
 * Rust side reads a time change as the difference between the two and applies
 * that movement to whatever resource the scope resolves to, so two equal
 * values mean "no movement" and no `start`/`end` is sent at all. Pass
 * `detail.start_ms` as the anchor — the mistake this file already warns about
 * above — and a recurring event's untouched time reads as a move of weeks:
 * with scope `'all'` that drags the series' first occurrence onto the edited
 * date and drops everything before it. Both values must come from the same
 * clicked block.
 *
 * `'following'` splits the series in two: a new one starting at the clicked
 * occurrence and carrying `fields`, and the original shortened to end just
 * before it. That is two writes with no transaction across them, so it is the
 * one scope that can partly succeed — it reports a leftover duplicate rather
 * than pretending otherwise, and the message is meant to be shown as-is. It is
 * also the one scope that notifies guests of a *creation*: the new series
 * carries the whole guest list of the one it continues.
 *
 * `'following'` also refuses two shapes outright, before writing anything, and
 * both messages are written to be shown to the user as they are: a series that
 * ends after a set number of times, and one whose later occurrences have been
 * moved or deleted on their own — a split cannot carry those across, and losing
 * them silently would be worse than not splitting.
 *
 * A scope this command does not implement is refused outright rather than
 * treated as "this occurrence".
 */
export const updateEvent = (
  id: number,
  scope: 'this' | 'all' | 'following',
  occurrenceStartMs: number,
  fields: EventInput,
  sendUpdates: SendUpdates,
) => invoke<EventDetail>('update_event', { id, scope, occurrenceStartMs, fields, sendUpdates });

/**
 * Who Google mails about a write. Google's own vocabulary, and a required
 * argument rather than a defaulted one on purpose: every caller has to say,
 * and the two answers are opposite instructions.
 *
 * `'all'` is the **form's**, and the reasoning `patch_event` carries is still
 * exactly right for it — a time typed on purpose and saved is the change
 * guests need to hear about. `'none'` is a **drag's**, because a gesture can
 * happen by accident and a slip of the mouse must not mail a meeting's whole
 * guest list. Drag spec §2 is the ruling.
 *
 * There is no default. A default would be the value nobody chose, and this is
 * the one argument in this file where nobody choosing is how somebody gets an
 * email they should not have.
 */
export type SendUpdates = 'all' | 'none';

/**
 * Deletes an event, or part of a recurring series, and resolves with nothing.
 *
 * `occurrenceStartMs` carries the same rule as `respondToEvent`'s and
 * `updateEvent`'s, and carries it hardest of the three: it is the clicked
 * block's own `start_ms`, never `detail.start_ms`. For a recurring series that
 * second value is the master row's DTSTART, so a `'this'` delete aimed at it
 * removes the series' *first* occurrence rather than the one on screen.
 *
 * The three scopes are three different operations, not three sizes of the same
 * one:
 *
 * - `'this'` removes the clicked occurrence only. The rest of the series stays.
 * - `'all'` removes the whole series, past occurrences included.
 * - `'following'` shortens the series to end just before the clicked
 *   occurrence. Nothing is deleted: the occurrences before it are in the same
 *   Google event, so deleting it would take them too.
 *
 * **Every one of them notifies the guest list** — the delete goes out with
 * `sendUpdates=all`, because a meeting that vanishes for the organiser alone is
 * worse than an email. A confirmation shown before calling this should say so,
 * and can say how many people from `detail.attendees`.
 *
 * There is no undo. Google keeps no copy this app can reach, and for `'all'` the
 * past occurrences go with the series.
 *
 * Nothing is returned, so the caller reloads the grid itself. For `'this'` on a
 * series that reload has to be a *sync* rather than a re-read: the local store
 * cannot know an occurrence is gone until Google says so, so the block stays on
 * screen until the next sync picks up the cancellation.
 *
 * A scope this command does not implement is refused outright rather than
 * treated as "this occurrence".
 */
export const deleteEvent = (
  id: number,
  scope: 'this' | 'all' | 'following',
  occurrenceStartMs: number,
) => invoke<void>('delete_event_cmd', { id, scope, occurrenceStartMs });
