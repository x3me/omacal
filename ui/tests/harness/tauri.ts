// A stand-in for the Tauri IPC layer, so `App.svelte` — four `$effect`s, two
// event listeners, and every interaction the final review flagged — can be
// mounted and driven from a spec.
//
// Both halves of the app's Tauri surface bottom out in one object:
// `invoke()` is `window.__TAURI_INTERNALS__.invoke(...)`, and `listen()` is
// itself an `invoke('plugin:event|listen', …)` carrying a callback registered
// through `window.__TAURI_INTERNALS__.transformCallback`. Replacing that one
// object therefore stubs commands and events together, with no module mocking
// and no build-time aliasing — the app imports the real `@tauri-apps/api`.

import type { WeekPayload, MonthPayload, YearPayload, BigYearPayload } from '../../src/lib/api';
import type { AppStatus } from '../../src/lib/status';
import type { Calendar } from '../../src/lib/calendars';
import type { EventDetail } from '../../src/lib/eventdetail';
import type { TimeFormat } from '../../src/lib/timefmt';
import type { WeekStartDay } from '../../src/lib/weekstart';
import {
  labelledWeek, weekLabel, APP_FIVE_MIN_AGO, APP_NOW, APP_SERIES_ID, APP_SERIES_OCCURRENCE,
  APP_ONE_OFF_ID, APP_ONE_OFF_START, APP_GUESTS_ID, APP_SOLO_SERIES_ID,
  POPOVER_DETAILS, busyDayMonth,
  appWritableWeek, APP_WRITE_CALENDARS, CREATED_DETAIL, crossZoneWeek,
  XZONE_WEEK_START,
} from '../fixtures';

/** What the real `get_palette` returns; the same fallback_dark values. */
const PALETTE = {
  bg: '#17171a', surface: '#1e1e22', text: '#e8e8ea',
  muted: '#8a8a90', accent: '#5b8def', is_dark: true,
};

/** The first-run failure a user is most likely to meet: no config file. */
export const NO_CONFIG_ERROR =
  'no config at /Users/someone/.config/omacal/config.toml: No such file or directory ' +
  '(os error 2). Create it with client_id and client_secret.';

type Deferred = { resolve: (w: WeekPayload) => void; reject: (e: unknown) => void };

export type Harness = {
  /**
   * Fire a Tauri event at the app, once it is actually listening.
   *
   * `listen()` is itself an async round trip, so an event fired the instant
   * after `goto` can land before the app has subscribed and simply vanish —
   * which showed up as a spec that passed under WebKit and failed under
   * Chromium. Waiting for the subscription makes the ordering a fact rather
   * than a hope; a missing subscriber is a real failure and throws.
   */
  emit(event: string, payload: unknown): Promise<void>;
  /** Park the *next* `get_week` for this week start instead of answering it. */
  hold(weekStartMs: number): void;
  /** How many `get_week` calls are currently parked. */
  held(): number;
  /** Answer a parked `get_week`, then let its `.then` chain run. */
  release(weekStartMs: number): Promise<void>;
  /** Make the next `get_week` reject, whoever asks for it. */
  failNextWeek(message: string): void;
  /** Make the next `update_event` reject — what a drag spec uses to prove a
   *  failed write is reported rather than silently swallowed. */
  failNextUpdate(message: string): void;
  /** Make the next `create_event` reject — what the duplicate-create guard
   *  spec uses to stand in for "Google succeeded, the local half did not". */
  failNextCreate(message: string): void;
  /** Make the next call to `set_calendar_selected` or `set_calendar_sync` reject —
   *  what a CalendarPopover spec uses to drive the failed-toggle path. */
  failNextCalendarCall(cmd: 'set_calendar_selected' | 'set_calendar_sync', message: string): void;
  /** Park the *next* call to this calendar command instead of answering it —
   *  what a CalendarPopover spec uses to prove a second row's toggle doesn't
   *  free up a first row whose own call is still pending. */
  holdNextCalendarCall(cmd: 'set_calendar_selected' | 'set_calendar_sync'): void;
  /** Answer the parked call for this command, then let its `.then` chain run. */
  releaseCalendarCall(cmd: 'set_calendar_selected' | 'set_calendar_sync', value: unknown): Promise<void>;
  /** Park the *next* `event_detail`, `refresh_event`, or `respond_to_event`
   *  call for this exact event id instead of answering it — what a
   *  `WeekGrid` spec uses to drive the supersession guard (open one block
   *  while another's detail is still loading), the after-paint refresh
   *  (control when it lands), and closing a popover while its own RSVP is
   *  still in flight (control when *that* lands). */
  /** Parks the next `search_events` call. What it is for is the race a
   *  search-as-you-type overlay has: a response to an *earlier* query arriving
   *  after a later one and overwriting it. Holding the first and letting the
   *  second answer immediately is the only way to produce that ordering
   *  deliberately. */
  holdNextSearch(): void;
  /** Releases the oldest parked search call. */
  releaseSearch(): Promise<void>;
  /** Parks the next `get_settings` call. What it is for is the race the
   *  filmstrip toggle has at startup: `F` works the instant the window is
   *  listening, and the startup read of the stored preference describes the
   *  world *before* that keystroke. Holding the read and pressing the key while
   *  it is parked is the only way to produce that ordering deliberately. */
  holdNextSettings(): void;
  /** Releases the parked `get_settings` call, answering with the settings as
   *  the stub now holds them. */
  releaseSettings(): Promise<void>;
  holdNextEventCall(cmd: 'event_detail' | 'refresh_event' | 'respond_to_event', id: number): void;
  /** Answer the parked call for this command and id, then let its `.then`
   *  chain — including `WeekGrid`'s own `detail = fresh` — actually run. */
  releaseEventCall(
    cmd: 'event_detail' | 'refresh_event' | 'respond_to_event',
    id: number,
    value: EventDetail,
  ): Promise<void>;
  /** Make the next call to this command for this exact id reject — what a
   *  `WeekGrid` spec uses to drive the load-failure-closes path
   *  (`event_detail`) without disturbing every other id's fixture. */
  failNextEventCall(cmd: 'event_detail' | 'refresh_event' | 'respond_to_event', id: number, message: string): void;
  /** Reject the *parked* call for this command and id — the counterpart to
   *  `releaseEventCall`, for proving a *late failure* for a superseded
   *  click does not close a popover that opened successfully after it.
   *  `failNextEventCall` alone cannot drive that: it rejects immediately,
   *  before a second click could ever land. */
  rejectEventCall(cmd: 'event_detail' | 'refresh_event' | 'respond_to_event', id: number, message: string): Promise<void>;
  /** Every command the app has invoked, in order. */
  calls: { cmd: string; args: unknown }[];
};

/** What `set_calendar_sync(id, false)` reports removing, absent a forced failure. */
export const CALENDAR_SYNC_REMOVED = 143;

/** A well-shaped but otherwise unused `EventDetail`, returned by the
 *  `respond_to_event` stub — see the case below for why its content never
 *  matters to a spec. */
const RESPOND_STUB_DETAIL = {
  id: 0, title: null, description: null, location: null, conference_uri: null,
  start_ms: 0, end_ms: 0, is_all_day: false, is_recurring: false, color: null,
  recurrence: null, repeat: 'never', weekly_days: [], repeat_end: { kind: 'never' },
  organizer_email: null, self_response: null, can_respond: true, can_edit: false,
  attendees: [],
};

const listeners = new Map<string, Set<(e: unknown) => void>>();
const callbacks = new Map<number, (e: unknown) => void>();
const hold = new Set<number>();
const parked = new Map<number, Deferred>();

type CalendarDeferred = { resolve: (v: unknown) => void; reject: (e: unknown) => void };

let nextId = 1;
let failWeekOnce: string | null = null;
let failUpdateOnce: string | null = null;
let failCreateOnce: string | null = null;
let failCalendarOnce: { cmd: string; message: string } | null = null;
/** Held search calls, oldest first — see `holdNextSearch`. Module level, like
 *  the calendar and event parks beside it: the harness object that releases
 *  them is module level too. */
const parkedSearch: Array<{ query: string; resolve: () => void }> = [];
let holdSearchOnce = false;
/** A held `get_settings`, and the flag that arms one. Module level, like the
 *  parks beside it, because the harness object that releases them is. */
let holdSettingsOnce = false;
let parkedSettings: (() => void) | null = null;
let holdCalendarOnce: string | null = null;
const parkedCalendar = new Map<string, CalendarDeferred>();

type EventDeferred = { resolve: (d: EventDetail) => void; reject: (e: unknown) => void };
type EventCmd = 'event_detail' | 'refresh_event' | 'respond_to_event';

// Keyed by `${cmd}:${id}`, not just `id`: a spec driving the after-paint
// refresh needs to hold `refresh_event` for an id while `event_detail` for
// that same id answers normally, and a single `id`-only key couldn't tell
// the two commands' holds apart.
function eventCallKey(cmd: EventCmd, id: number): string {
  return `${cmd}:${id}`;
}
const holdEventCallOnce = new Set<string>();
const parkedEventCall = new Map<string, EventDeferred>();
let failEventCallOnce: { key: string; message: string } | null = null;

/**
 * Resolves once something has subscribed to `event`; throws if none does.
 * Counts polls rather than watching the clock — the specs freeze `Date.now()`.
 */
async function whenListening(event: string, polls = 300): Promise<void> {
  for (let i = 0; i < polls; i++) {
    if (listeners.get(event)?.size) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error(`nothing is listening for "${event}"`);
}

const harness: Harness = {
  calls: [],
  async emit(event, payload) {
    await whenListening(event);
    for (const fn of listeners.get(event) ?? []) fn({ event, id: 0, payload });
  },
  hold(weekStartMs) {
    hold.add(weekStartMs);
  },
  held() {
    return parked.size;
  },
  async release(weekStartMs) {
    parked.get(weekStartMs)?.resolve(labelledWeek(weekStartMs));
    parked.delete(weekStartMs);
    // Let the resolution — and anything it schedules — actually run, so a
    // spec asserting "the stale response did not land" is asserting about a
    // response that has had its chance rather than one still in the queue.
    await new Promise((r) => setTimeout(r, 50));
  },
  failNextWeek(message) {
    failWeekOnce = message;
  },
  failNextCreate(message) {
    failCreateOnce = message;
  },
  failNextUpdate(message) {
    failUpdateOnce = message;
  },
  failNextCalendarCall(cmd, message) {
    failCalendarOnce = { cmd, message };
  },
  holdNextCalendarCall(cmd) {
    holdCalendarOnce = cmd;
  },
  async releaseCalendarCall(cmd, value) {
    parkedCalendar.get(cmd)?.resolve(value);
    parkedCalendar.delete(cmd);
    // Same reasoning as `release` above: let the resolution's `.then` chain —
    // `onchange()`, `markBusy(id, false)`, the focus restore — actually run
    // before the spec asserts on it.
    await new Promise((r) => setTimeout(r, 50));
  },
  holdNextSearch() {
    holdSearchOnce = true;
  },
  async releaseSearch() {
    parkedSearch.shift()?.resolve();
    // Let the resolution's own `.then` run before a spec asserts on it, the
    // same reason `release` above waits.
    await new Promise((r) => setTimeout(r, 50));
  },
  holdNextSettings() {
    holdSettingsOnce = true;
  },
  async releaseSettings() {
    parkedSettings?.();
    parkedSettings = null;
    // Same reason `release` above waits: let the `.then` that assigns
    // `listMode` actually run before the spec asserts about it.
    await new Promise((r) => setTimeout(r, 50));
  },
  holdNextEventCall(cmd, id) {
    holdEventCallOnce.add(eventCallKey(cmd, id));
  },
  async releaseEventCall(cmd, id, value) {
    const key = eventCallKey(cmd, id);
    parkedEventCall.get(key)?.resolve(value);
    parkedEventCall.delete(key);
    await new Promise((r) => setTimeout(r, 50));
  },
  failNextEventCall(cmd, id, message) {
    failEventCallOnce = { key: eventCallKey(cmd, id), message };
  },
  async rejectEventCall(cmd, id, message) {
    const key = eventCallKey(cmd, id);
    parkedEventCall.get(key)?.reject(message);
    parkedEventCall.delete(key);
    await new Promise((r) => setTimeout(r, 50));
  },
};

/** Checks whether a spec armed a hold or a forced failure for `(cmd, id)`
 *  and, if so, returns the promise to answer with. `null` means neither is
 *  armed — the caller falls through to its own default response. Shared by
 *  `event_detail`/`refresh_event` (via `eventCallResult` below) and
 *  `respond_to_event` (inline in the command switch), which otherwise have
 *  nothing else in common: their "not held" defaults come from entirely
 *  different places. */
function heldOrFailed(cmd: EventCmd, id: number): Promise<EventDetail> | null {
  const key = eventCallKey(cmd, id);
  if (failEventCallOnce?.key === key) {
    const { message } = failEventCallOnce;
    failEventCallOnce = null;
    return Promise.reject(message);
  }
  if (holdEventCallOnce.has(key)) {
    holdEventCallOnce.delete(key);
    return new Promise<EventDetail>((resolve, reject) => {
      parkedEventCall.set(key, { resolve, reject });
    });
  }
  return null;
}

/** Resolves against `POPOVER_DETAILS` unless a spec armed a failure or a
 *  hold for this exact command and id. Both `event_detail` and
 *  `refresh_event` default to the same, unchanged entry — the ordinary
 *  case, where nothing moved since the popover opened. A *different* value
 *  (e.g. `POPOVER_REFRESHED_DETAIL`) only ever reaches a spec through an
 *  explicit `releaseEventCall(cmd, id, thatValue)`, never through this
 *  default path. */
function eventCallResult(cmd: 'event_detail' | 'refresh_event', id: number): Promise<EventDetail> {
  const held = heldOrFailed(cmd, id);
  if (held) return held;
  const d = POPOVER_DETAILS[id];
  if (!d) throw new Error(`no ${cmd} fixture for event id ${id}`);
  return Promise.resolve(d);
}

/** Resolves normally unless a spec armed a failure or a hold for this exact command. */
function calendarResult<T>(cmd: string, ok: T): Promise<T> {
  if (failCalendarOnce?.cmd === cmd) {
    const { message } = failCalendarOnce;
    failCalendarOnce = null;
    return Promise.reject(message);
  }
  if (holdCalendarOnce === cmd) {
    holdCalendarOnce = null;
    return new Promise<T>((resolve, reject) => {
      parkedCalendar.set(cmd, { resolve, reject } as CalendarDeferred);
    });
  }
  return Promise.resolve(ok);
}

/**
 * Command responses for a named scenario. `default` is a connected account on
 * a working backend; the rest exist to reach a failure the UI has to render.
 */
function statusFor(scenario: string): AppStatus {
  switch (scenario) {
    case 'no-config':
    case 'disconnected':
    // Starts with nobody connected, so `handleSignIn` runs the
    // "Connect Google Calendar" path rather than "Add account" — Task 7's
    // picker opens after either.
    case 'sign-in-adds-account':
      return {
        accounts: [], needs_reauth: [], update: null, system_tz_change: null, version: '9.9.9', last_sync_ms: null, demo: false,
        overlay_titlebar: false, self_update: false,
      };
    // A newer release, as the daily check would have left it on AppState.
    case 'update-available':
      return {
        accounts: ['me@x.com'], needs_reauth: [], version: '9.9.9', system_tz_change: null,
        update: { version: '0.2.0', url: 'https://github.com/x3me/omacal/releases/tag/v0.2.0' },
        last_sync_ms: APP_FIVE_MIN_AGO, demo: false, overlay_titlebar: false, self_update: false,
      };
    // A connected account the backend has stopped syncing: a dead refresh
    // token, discovered by a sync, waiting on a re-consent.
    case 'needs-reauth':
      return {
        accounts: ['me@x.com'], needs_reauth: ['me@x.com'], update: null, system_tz_change: null, version: '9.9.9',
        last_sync_ms: APP_FIVE_MIN_AGO, demo: false, overlay_titlebar: false, self_update: false,
      };
    // A macOS window whose controls are drawn over the webview. The default
    // below is the other platform — Omarchy, where they have a strip of their
    // own — which is also what every scenario that predates this one wants,
    // since none of them is about the title bar.
    case 'overlay-titlebar':
      return {
        accounts: ['me@x.com'], needs_reauth: [], update: null, system_tz_change: null, version: '9.9.9', last_sync_ms: APP_FIVE_MIN_AGO,
        demo: false, overlay_titlebar: true, self_update: false,
      };
    default:
      return {
        accounts: ['me@x.com'], needs_reauth: [], update: null, system_tz_change: null, version: '9.9.9', last_sync_ms: APP_FIVE_MIN_AGO,
        demo: false, overlay_titlebar: false, self_update: false,
      };
  }
}

/** What `get_calendars` returns for the `sign-in-adds-account` scenario, once
 *  `sign_in` has actually been called — one freshly imported account, all of
 *  it switched on by default, exactly as a real `sign_in` leaves it. */
const SIGNED_IN_CALENDARS: Calendar[] = [
  { id: 1, account_id: 1, account_email: 'new@x.com', summary: 'Personal',
    color_hex: '#5b8def', color_override: null, selected: true, sync_enabled: true, is_primary: true,
    access_role: 'owner', provider: 'google' },
  // A subscribed holiday calendar really is a `reader`, and this is the one
  // fixture in the suite that stands in for a real `sign_in` import — so it
  // carries the role a real one would rather than a uniformly writable list.
  { id: 2, account_id: 1, account_email: 'new@x.com', summary: 'Holidays',
    color_hex: '#e2a03f', color_override: null, selected: true, sync_enabled: true, is_primary: false,
    access_role: 'reader', provider: 'google' },
];

function getWeek(scenario: string, weekStartMs: number): Promise<WeekPayload> {
  if (failWeekOnce !== null) {
    const message = failWeekOnce;
    failWeekOnce = null;
    return Promise.reject(message);
  }
  if (hold.has(weekStartMs)) {
    hold.delete(weekStartMs);
    return new Promise<WeekPayload>((resolve, reject) => {
      parked.set(weekStartMs, { resolve, reject });
    });
  }
  // The `writable` scenario answers with the same two editable events whatever
  // week is asked for — same shortcut, and the same reasoning, as `getMonth`
  // below: its specs pin literal instants, and none of them needs the payload
  // to match the week it requested.
  if (scenario === 'writable') return Promise.resolve(appWritableWeek());
  // `cross-zone` is the one scenario where the week actually asked for is part
  // of the claim: its payload's columns are `Europe/Sofia` midnights, and it
  // only describes what is on screen if the app requested that same Monday.
  // Answering unconditionally would let a browser in the wrong zone — or an
  // `App` that opened on a different week — read this fixture's columns against
  // a header that had moved, with the agreement spec none the wiser. So it
  // refuses rather than substitutes, and the spec asserts the request too.
  if (scenario === 'cross-zone') {
    if (weekStartMs !== XZONE_WEEK_START) {
      return Promise.reject(
        `cross-zone: asked for week ${weekStartMs}, fixture is ${XZONE_WEEK_START}`,
      );
    }
    return Promise.resolve(crossZoneWeek());
  }
  return Promise.resolve(labelledWeek(weekStartMs));
}

const DAY_MS = 24 * 3_600_000;

/** Day view's own `get_day` stub: a one-column `WeekPayload`, echoing back
 *  whatever `dayStartMs` was actually asked for — same reasoning as
 *  `getWeek`'s `labelledWeek` above, just with no events to label. No App
 *  spec needs a populated day, only that the column it renders carries the
 *  date that was actually requested. */
function getDay(dayStartMs: number): WeekPayload {
  return {
    days: [{ start_ms: dayStartMs, end_ms: dayStartMs + DAY_MS, events: [], placed: [] }],
    all_day: [],
    all_day_events: [],
    overflow: [],
  };
}

/** Month view's own `get_month` stub. Unlike `getWeek`/`getDay`, this
 *  deliberately ignores `year`/`month` and always returns the same fixed
 *  grid (`MonthGrid`'s own `busy-day` fixture) — the anchor-survival spec
 *  pins a literal cell (`1_786_320_000_000`, Mon 10 Aug) as the value a
 *  click has to carry through to Day view, and no App spec needs the grid to
 *  actually match the requested month (that's `assemble_month`'s own
 *  Rust-side coverage, and `MonthGrid`'s). */
function getMonth(): MonthPayload {
  return busyDayMonth();
}

/** Year view's own `get_year` stub: twelve otherwise-empty months, echoing
 *  back whatever year was actually asked for — same reasoning as `getDay`'s
 *  `dayStartMs` above. No App spec needs a populated grid, only that it
 *  carries the year that was actually requested (`YearGrid`'s own `y2026`
 *  fixture already covers the populated case). */
function getYearStub(y: number): YearPayload {
  return {
    year: y,
    months: Array.from({ length: 12 }, (_, i) => ({ month: i + 1, lead_blanks: 0, days: [] })),
  };
}

/**
 * Big Year view's own `get_big_year` stub: fourteen otherwise-empty 28-day
 * rows, echoing back whatever year was actually asked for — same reasoning as
 * `getYearStub` above (`BigYearRibbon`'s own `y2026`/`crossing` fixtures
 * already cover the populated case).
 *
 * The days carry **real** `start_ms` values. They used to be `0` throughout,
 * on the reasoning that no App spec read them; Task 10 made a ribbon day
 * clickable, and a click through a day whose start is `0` opens an event form
 * dated 1 January 1970 — a fixture that would have made the create-from-Big-Year
 * witness assert the epoch and call it a pass. Anchored the way
 * `assemble_big_year` anchors: the Monday on or before 1 January of the year
 * asked for, then 28 days per row.
 */
function getBigYearStub(y: number): BigYearPayload {
  const jan1 = new Date(y, 0, 1);
  // `getDay()` is 0 for Sunday, so Sunday steps back six days, not none.
  const back = (jan1.getDay() + 6) % 7;
  const ribbonStart = new Date(y, 0, 1 - back).getTime();
  return {
    year: y,
    rows: Array.from({ length: 14 }, (_, r) => ({
      days: Array.from({ length: 28 }, (_, c) => ({
        start_ms: ribbonStart + (r * 28 + c) * DAY_MS,
        in_year: true,
        unsynced: false,
      })),
      pills: [],
      pill_events: [],
      overflow: [],
    })),
    legend: [],
  };
}

/**
 * The stub's stand-in for the `settings` table, **and it outlives a reload**.
 *
 * That is the whole reason it is not a plain object. The filmstrip toggle is a
 * stored preference (filmstrip spec §4), and the only way to tell a stored
 * preference from a session variable is to end the session: press the key,
 * reload, and see whether it is still on. A stub rebuilt from literals on every
 * page load answers "no" to that question regardless of what the app did, so
 * the spec would fail against a correct implementation — and a stub that
 * *seeded* the value instead would pass against one that never wrote anything.
 *
 * `sessionStorage`, not `localStorage`: Playwright gives each test its own
 * browser context, so both are already isolated per test, but sessionStorage is
 * scoped to the tab and cannot outlive one even by accident. Every value below
 * is the same default `read_settings` reports for an absent row — five minutes
 * and a one-minute floor are `sync_loop`'s own `DEFAULT_INTERVAL_MS` and
 * `MIN_INTERVAL_MS`; reminders are on; the calendar is a grid.
 */
type StubSettings = {
  syncIntervalMs: number;
  notificationsEnabled: boolean;
  minSyncIntervalMs: number;
  listMode: boolean;
  fallbackReminderMinutes: number[];
  defaultCalendarId: number | null;
  timeFormat: TimeFormat;
  weekStart: WeekStartDay;
  displayTimezone: string | null;
  secondTimezone: string | null;
  weatherEnabled: boolean;
};

const SETTINGS_KEY = 'omacal-stub-settings';

/** What `list_timezones` answers and `set_second_timezone` validates
 *  against — one list, so the stub cannot offer a zone it then refuses. */
const STUB_TIMEZONES = ['Asia/Kolkata', 'Europe/Sofia', 'UTC'];

const DEFAULT_SETTINGS: StubSettings = {
  syncIntervalMs: 5 * 60_000,
  notificationsEnabled: true,
  minSyncIntervalMs: 60_000,
  // The backend's own shipped default (fallback spec §3).
  fallbackReminderMinutes: [60, 10],
  defaultCalendarId: null,
  listMode: false,
  // The clock the app has always drawn, so every existing spec and every
  // committed screenshot golden goes on describing the same pixels.
  timeFormat: '24h',
  // The week omacal has always drawn, so every golden holds.
  weekStart: 'monday',
  displayTimezone: null,
  // Off, the backend's fresh-install default — and what keeps every
  // committed gutter golden describing a 44px ruler with one clock.
  secondTimezone: null,
  // The backend's default: on unless somebody turned it off.
  weatherEnabled: true,
};

function loadSettings(): StubSettings {
  try {
    const raw = sessionStorage.getItem(SETTINGS_KEY);
    return raw ? { ...DEFAULT_SETTINGS, ...JSON.parse(raw) } : { ...DEFAULT_SETTINGS };
  } catch {
    // A storage that refuses to answer leaves the defaults, exactly as an
    // absent row does. Never a throw: this runs during the stub's own install.
    return { ...DEFAULT_SETTINGS };
  }
}

function saveSettings(s: StubSettings): StubSettings {
  try {
    sessionStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    // Nothing to do about it here; the in-memory copy still answers this
    // session, which is what every non-reloading spec reads.
  }
  return s;
}

/** Installs the stub. Call before mounting anything that talks to Tauri. */
/**
 * What `search_events` answers from, filtered by the query.
 *
 * Both sides of the app's frozen clock (`APP_NOW`, Mon 29 Jan 2024) on
 * purpose: with everything in the future, nearest-first and soonest-first are
 * the same order and the ordering assertion would say nothing. The Rust side
 * owns the real ordering; this exists so the overlay can be driven.
 */
const SEARCHABLE = [
  {
    eventId: APP_SERIES_ID, title: 'Standup',
    startMs: APP_SERIES_OCCURRENCE, endMs: APP_SERIES_OCCURRENCE + 30 * 60_000,
  },
  {
    eventId: APP_ONE_OFF_ID, title: 'Board prep',
    startMs: APP_ONE_OFF_START, endMs: APP_ONE_OFF_START + 60 * 60_000,
  },
  {
    eventId: APP_GUESTS_ID, title: 'Standup review',
    startMs: APP_NOW - 48 * 3_600_000, endMs: APP_NOW - 47 * 3_600_000,
  },
  // **A different month from the one the app opens on**, which is what makes
  // "the calendar moves to that date" witnessable at all: every other entry
  // here sits in the same January the anchor already starts in, so choosing
  // one moves nothing and a version that never moved would pass.
  {
    eventId: APP_SOLO_SERIES_ID, title: 'Dentist',
    startMs: APP_NOW + 45 * 24 * 3_600_000, endMs: APP_NOW + 45 * 24 * 3_600_000 + 3_600_000,
  },
];

export function installTauriStub(scenario: string): Harness {
  // Reassigned by `sign_in` for the `sign-in-adds-account` scenario: a real
  // `sign_in` leaves the account durably connected, so the next `get_status`
  // must reflect it too, not just `get_calendars`.
  let status = statusFor(scenario);
  let signedIn = false;
  /** Whether `take_open_date` has answered — the real command clears on
   *  read, and a stub that kept answering would hide a remount replaying
   *  the date, which is exactly the defect `take` semantics exist to stop. */
  let openDateTaken = false;
  /** The settings modal's preferences, per page. Mutable for the reason the
   *  `get_settings` case gives: a stub answering the same thing forever cannot
   *  tell a saved setting from an ignored one. Five minutes and a one-minute
   *  floor are `sync_loop`'s own `DEFAULT_INTERVAL_MS`/`MIN_INTERVAL_MS`. */
  let settings = loadSettings();

  const invoke = async (cmd: string, args: Record<string, any> = {}): Promise<unknown> => {
    harness.calls.push({ cmd, args });
    switch (cmd) {
      case 'plugin:event|listen': {
        const fn = callbacks.get(args.handler);
        if (!fn) throw new Error(`listen with an unknown handler ${args.handler}`);
        const set = listeners.get(args.event) ?? new Set();
        set.add(fn);
        listeners.set(args.event, set);
        return args.handler; // the real one returns an event id; the id works
      }
      case 'plugin:event|unlisten': {
        const fn = callbacks.get(args.eventId);
        if (fn) listeners.get(args.event)?.delete(fn);
        return null;
      }
      case 'get_palette':
        return PALETTE;
      // Recorded in `calls` like everything else; the real one opens a
      // browser, which a test can only assert was *asked for*.
      case 'open_latest_release':
        return null;
      // Same posture as `open_latest_release`: the real command replaces the
      // AppImage and restarts, which a test can only assert was *asked for*.
      case 'install_update':
        return null;
      // The browser-open again, one more remove: the backend resolves the
      // URL from its own store by id and spawns sanitized. The test's fact
      // is the call and its id, in `harness.calls`.
      case 'open_conference':
        return null;
      case 'get_status':
        return status;
      // The date a dated fresh launch parked on the backend. One scenario
      // carries one; everything else launched bare.
      case 'take_open_date': {
        if (scenario === 'launched-with-date' && !openDateTaken) {
          openDateTaken = true;
          return '2024-03-14';
        }
        return null;
      }
      case 'list_accounts':
        // The richer per-account shape the Accounts tab fetches: the same
        // emails the scenario's status carries, as Google accounts.
        return (status as { accounts: string[] }).accounts.map((email, i) => ({
          id: i + 1, email, provider: 'google',
        }));
      case 'sign_out':
        // Signing out the only fixture account leaves none.
        return [];
      case 'get_week':
        return getWeek(scenario, args.weekStartMs);
      case 'get_day':
        return getDay(args.dayStartMs);
      case 'get_month':
        return getMonth();
      case 'get_year':
        return getYearStub(args.year);
      case 'get_big_year':
        return getBigYearStub(args.year);
      // App's own effect fetches calendars alongside status on mount. None of
      // the App specs exercise the popover, and Header only renders it once
      // `calendars.length > 0`, so an empty list keeps every existing
      // assertion undisturbed. CalendarPopover specs never take this path —
      // they mount the component directly with fixture props instead.
      case 'get_calendars':
        // `cross-zone` needs a writable list for the same reason `writable`
        // does: without one the form's calendar select has nothing to seed
        // from and Save refuses, so the edit half of its agreement spec could
        // never run.
        if (scenario === 'writable' || scenario === 'cross-zone') return APP_WRITE_CALENDARS;
        return scenario === 'sign-in-adds-account' && signedIn
          ? SIGNED_IN_CALENDARS
          : ([] as Calendar[]);
      // The settings modal's three. Held in one mutable object rather than
      // returned as constants, because half of what its specs assert is that a
      // value **came back changed** — a stub answering the same thing forever
      // cannot tell a saved setting from an ignored one.
      // Search. Results come from the scenario's own events so a spec can
      // assert on titles it seeded, and the *query* is honoured here rather
      // than in the app — a stub that answered the same list for every query
      // could not tell a superseded response from a current one.
      case 'search_events': {
        const q = String(args.query ?? '').trim().toLowerCase();
        const hits = q === ''
          ? []
          : SEARCHABLE.filter((h) => h.title.toLowerCase().includes(q));
        if (holdSearchOnce) {
          holdSearchOnce = false;
          return new Promise((resolve) => {
            parkedSearch.push({ query: q, resolve: () => resolve(hits) });
          });
        }
        return hits;
      }
      case 'get_settings': {
        // `__holdSettings` as well as the harness flag, because the one read
        // worth holding is the one `App` issues **on mount** — before
        // `window.__harness` exists for a spec to call `holdNextSettings` on.
        // A spec arms it through `page.addInitScript`, which runs before the
        // harness module does.
        const held = holdSettingsOnce || (window as any).__holdSettings === true;
        if (held) {
          holdSettingsOnce = false;
          (window as any).__holdSettings = false;
          // **Snapshotted at the park, not read again at the release**, and
          // that is what makes the race reproducible: a slow read answers the
          // question it was asked, which describes the world before whatever
          // the user has done since. Resolving with the current object instead
          // would answer with the value the keystroke had already written, and
          // a missing supersession guard would look correct.
          const asOfTheQuestion = { ...settings };
          return new Promise((resolve) => {
            parkedSettings = () => resolve(asOfTheQuestion);
          });
        }
        return { ...settings };
      }
      case 'set_sync_interval': {
        // The backend refuses below the floor rather than clamping, and so
        // does this: a stub that accepted anything would let a form which
        // forgot its own guard pass every spec.
        if ((args.ms as number) < settings.minSyncIntervalMs) {
          throw new Error(
            "omacal will not sync more often than once a minute — Google's quota is finite " +
            'and a desktop app has no business polling faster than that',
          );
        }
        settings = saveSettings({ ...settings, syncIntervalMs: args.ms as number });
        return { ...settings };
      }
      // A short list, not the real ~600: the spec's premise is that choosing
      // one and applying sends it, not that jiff's database is complete.
      case 'list_timezones':
        return [...STUB_TIMEZONES];
      case 'set_display_timezone':
        settings = saveSettings({ ...settings, displayTimezone: (args.tz as string | null) ?? null });
        // The real command restarts the app after replying; the stub only
        // replies, which is exactly what lets a spec see the "Restarting…"
        // state the window would otherwise take with it.
        return null;
      case 'set_second_timezone': {
        // The backend's own refusal, mirrored against the stub's short list
        // so a spec can watch the form surface it; the real validator asks
        // jiff the same question of the same name.
        const tz = (args.tz as string | null) ?? null;
        if (tz !== null && !STUB_TIMEZONES.includes(tz)) {
          throw new Error(`omacal does not know the time zone "${tz}"`);
        }
        settings = saveSettings({ ...settings, secondTimezone: tz });
        return { ...settings };
      }
      // Empty on purpose: App-level scenarios stay weatherless, so no
      // existing spec or screenshot grows a sky it never asked for. The
      // rendering itself is proven component-side, where the fixture hands
      // the map in as a prop.
      case 'get_weather':
        return { days: [], place: null };
      case 'set_weather_enabled':
        settings = saveSettings({ ...settings, weatherEnabled: args.on as boolean });
        return { ...settings };
      case 'set_notifications_enabled':
        settings = saveSettings({ ...settings, notificationsEnabled: args.on as boolean });
        return { ...settings };
      case 'set_list_mode':
        settings = saveSettings({ ...settings, listMode: args.on as boolean });
        return { ...settings };
      case 'set_week_start':
        settings = saveSettings({ ...settings, weekStart: args.start as WeekStartDay });
        return { ...settings };
      case 'set_time_format':
        settings = saveSettings({ ...settings, timeFormat: args.format as TimeFormat });
        return { ...settings };
      case 'set_default_calendar':
        settings = saveSettings({ ...settings, defaultCalendarId: (args.id as number | null) ?? null });
        return { ...settings };
      case 'set_fallback_reminders': {
        const minutes = args.minutes as number[];
        // The backend's own refusals, mirrored so a spec can watch the form
        // surface them: 5 rows, four weeks, nothing negative.
        if (minutes.length > 5) throw new Error('an event can carry at most 5 reminders');
        if (minutes.some((m) => m < 0 || m > 40_320)) {
          throw new Error('a reminder must be 0 to 40320 minutes (four weeks) ahead');
        }
        settings = saveSettings({ ...settings, fallbackReminderMinutes: minutes });
        return { ...settings };
      }
      case 'set_calendar_color':
        return calendarResult(cmd, undefined);
      case 'set_calendar_selected':
        return calendarResult(cmd, undefined);
      case 'set_calendar_sync':
        return calendarResult(cmd, CALENDAR_SYNC_REMOVED);
      case 'sync_now':
        return 0;
      // The header's invitation tray. Empty by default so every App spec that
      // predates it keeps describing a header without a badge; Header specs
      // that want rows mount the component with fixture props instead.
      case 'pending_invites':
        return [];
      case 'declined_guests':
        return [];
      // Recorded in `calls` like everything else; the row's disappearance is
      // the component's own optimistic hide, so nothing needs answering.
      case 'dismiss_decline_notice':
        return null;
      case 'dismiss_all_decline_notices':
        return 0;
      case 'changed_meetings':
        return [];
      // The guest field's autocomplete corpus. A small fixed cast, present in
      // every scenario: the addresses are chosen to collide with nothing any
      // other spec types, so the dropdown only ever appears when a spec asks
      // for it by typing a matching fragment.
      case 'known_guests':
        return [
          { email: 'iskren.h@x3me.net', display_name: 'Iskren Hadzhinedev', met: 12 },
          { email: 'eva.m@x3me.net', display_name: null, met: 5 },
        ];
      case 'dismiss_change_notice':
        return null;
      case 'dismiss_all_change_notices':
        return 0;
      case 'event_detail':
        return eventCallResult('event_detail', args.id);
      case 'refresh_event':
        return eventCallResult('refresh_event', args.id);
      case 'respond_to_event': {
        // Exposed so a spec can assert on exactly what `EventPopover` sent —
        // in particular, that the fourth argument is the clicked block's own
        // `start_ms` and never `detail.start_ms` (the task brief's trap).
        (window as any).__lastRespondCall = args;
        // Held/rejected the same way as event_detail/refresh_event — what a
        // WeekGrid spec uses to prove closing the popover mid-RSVP still
        // restyles the block once the response actually lands.
        const held = heldOrFailed('respond_to_event', args.id);
        if (held) return held;
        if (scenario === 'respond-fails') return Promise.reject('could not reach Google right now.');
        // Only the `writes-back` scenario simulates a real write-back (what
        // the backend actually returns for every non-recurring event, and
        // for `scope: 'all'`) — an attendee list carrying the new response,
        // so a spec can assert the guest list's own "you" row catches up to
        // it. Every other scenario's stand-in is never read by EventPopover
        // at all (see below), so its content doesn't matter.
        if (scenario === 'writes-back') {
          return {
            ...RESPOND_STUB_DETAIL,
            attendees: [
              { email: 'me@x.com', display_name: null, response_status: args.response,
                optional: false, is_self: true },
            ],
          };
        }
        // The resolved detail is deliberately never read by EventPopover: a
        // "this one" RSVP against a bare master leaves the backend's own
        // detail unchanged (see respond_to_event's doc comment), so the
        // popover shows the choice optimistically rather than trust this
        // return value. Any well-shaped stand-in satisfies its type.
        return RESPOND_STUB_DETAIL;
      }
      // The three write commands. None of them needs a hold, a forced failure
      // or a per-scenario answer: what every spec asserts on is the *arguments*
      // they were given — above all `occurrenceStartMs`, which must be the
      // clicked block's own `start_ms` and never `detail.start_ms` — and
      // `harness.calls` above already records those for every command, in
      // order. A second capture on `window` would be a second thing to keep in
      // step with it.
      case 'create_event':
        if (failCreateOnce !== null) {
          const m = failCreateOnce;
          failCreateOnce = null;
          // A bare string, exactly as Tauri rejects a `Result<_, String>` —
          // an `Error` wrapper would prefix "Error: " and defeat the exact
          // sentence-matching the duplicate-create guard rides on.
          return Promise.reject(m);
        }
        return CREATED_DETAIL;
      case 'update_event':
        if (failUpdateOnce !== null) {
          const m = failUpdateOnce;
          failUpdateOnce = null;
          throw new Error(m);
        }
        // What the real command answers with: the freshly written detail. `App`
        // never reads it (it reloads the grid instead), so the unchanged
        // fixture is a truthful enough stand-in.
        return POPOVER_DETAILS[args.id] ?? CREATED_DETAIL;
      case 'delete_event_cmd':
        // Returns nothing, exactly as the Rust command does: the event the
        // popover was showing is gone, and reading it back would fail on the
        // runs that succeeded.
        return null;
      case 'sign_in':
        // Tauri rejects a `Result<_, String>` with the bare string, so the
        // app sees exactly the sentence Rust produced.
        if (scenario === 'no-config') return Promise.reject(NO_CONFIG_ERROR);
        if (scenario === 'sign-in-adds-account') {
          signedIn = true;
          // Spread, not a fresh literal: signing in changes who is connected,
          // and nothing about the window it is connected from. Rebuilding the
          // object from scratch would quietly reset `overlay_titlebar` to
          // whatever this line happened to say.
          status = { ...status, accounts: ['new@x.com'], last_sync_ms: null };
          return 'new@x.com';
        }
        if (scenario === 'needs-reauth') {
          // A real sign_in clears the account's reauth mark
          // (`forget_stale_credentials`), so the next get_status stops asking.
          status = { ...status, needs_reauth: [] };
          return 'me@x.com';
        }
        return 'me@x.com';
      default:
        throw new Error(`unstubbed command: ${cmd}`);
    }
  };

  (window as any).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback(cb: (e: unknown) => void) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
  };
  (window as any).__harness = harness;
  return harness;
}

export { weekLabel };
