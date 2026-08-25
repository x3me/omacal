<!-- ui/src/App.svelte -->
<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { applyPalette, setPalette, type Palette } from './lib/theme';
  import {
    getWeek, getDay, getMonth, getYear, getBigYear, weekStart,
    type WeekPayload, type MonthPayload, type YearPayload, type BigYearPayload, type UiEvent,
  } from './lib/api';
  import { getStatus, installUpdate, openLatestRelease, restartApp, signIn, syncNow, takeOpenDate, type AppStatus } from './lib/status';
  import { getWeather, weatherByDate, type DayWeather } from './lib/weather';
  import { changedMeetings, declinedGuests, pendingInvites } from './lib/invites';
  import { getCalendars, offerableCalendarId, type Calendar } from './lib/calendars';
  import {
    createEvent, deleteEvent, getEventDetail, updateEvent,
    type EventDetail, type Occurrence, type SendUpdates,
  } from './lib/eventdetail';
  import {
    blankValue, blankValueAt, dateOf, pastedValue, previewSpan, timeOf, toEventInput,
    valueFromDetail, type EventFormResult, type EventFormValue, type Scope,
  } from './lib/eventform';
  import type { Rect } from './lib/position';
  import { daysFromMonth, daysFromWeek, listable } from './lib/filmstrip';
  import { getSettings, setListMode } from './lib/settings';
  import { setClockFormat } from './lib/clock.svelte';
  import { setSecondZone } from './lib/secondzone.svelte';
  import { setWeekStartDay } from './lib/weekstartstore.svelte';
  import ShortcutSheet from './lib/ShortcutSheet.svelte';
  import { SHORTCUT_LIST, type ShortcutId } from './lib/shortcuts';
  import Filmstrip from './lib/Filmstrip.svelte';
  import WeekGrid from './lib/WeekGrid.svelte';
  import MonthGrid from './lib/MonthGrid.svelte';
  import YearGrid from './lib/YearGrid.svelte';
  import BigYearRibbon from './lib/BigYearRibbon.svelte';
  import Header from './lib/Header.svelte';
  import EventPopover from './lib/EventPopover.svelte';
  import EventForm from './lib/EventForm.svelte';
  import SearchOverlay from './lib/SearchOverlay.svelte';
  import QuickEventModal from './lib/QuickEventModal.svelte';
  import DeleteConfirm from './lib/DeleteConfirm.svelte';
  import MoveConfirm from './lib/MoveConfirm.svelte';
  import ViewSwitcher, { type View } from './lib/ViewSwitcher.svelte';

  /** Midnight local on the day `ms` falls in — `T`'s target, and Day view's
   *  own boundary. */
  function dayStart(ms: number): number {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }

  /** Midnight local on an ISO `YYYY-MM-DD` — the shape the backend's date
   *  parser guarantees (`tray::parse_date`), which is why this needs no
   *  validation of its own: nothing else ever sends one. Split by hand
   *  rather than `new Date(ymd)`, because the string form parses as UTC
   *  midnight and lands on the previous day for every zone east of nowhere. */
  function ymdMs(ymd: string): number {
    const [y, m, d] = ymd.split('-').map(Number);
    return new Date(y, m - 1, d).getTime();
  }

  // The single date every view reads against. Switching views never touches
  // it — that's the whole point (spec §5): Month -> Day has to land on the
  // day you were looking at, not on today. Only `T`, the header's Today
  // button, and Month's own `ondaypick` ever assign it.
  let anchorMs = $state(dayStart(Date.now()));
  let view = $state<View>('week');

  // Year and Big Year read a bare calendar year rather than a millisecond
  // anchor — there is no single day inside them for `anchorMs` to name — so
  // each gets its own counter instead of borrowing `anchorMs`'s year.
  // Keeping them off `anchorMs` also protects the invariant above: opening
  // Big Year must never drag a past `anchorMs` forward to satisfy its own
  // bound (spec §4). Both start on the real current year, same reasoning as
  // `anchorMs` starting on today.
  //
  // `yearNum` is re-seeded from `anchorMs` on the way into Year view (see
  // `pick`) — a separate counter is how Year *steps*, not licence for it to
  // open somewhere other than where you were looking. `bigYearNum` is not,
  // for the bound above.
  let yearNum = $state(new Date().getFullYear());
  let bigYearNum = $state(new Date().getFullYear());

  // `jiff` rejects a civil date outside -9999..=9999, and `yearNum` feeds
  // `get_year`, which asks for `year + 1` (`commands::year_start_ms` at
  // `src-tauri/src/lib.rs:131`), so `year_start_ms(10000, ..)` panics rather
  // than erroring — a stuck `L` key is enough to reach it. The lower bound is
  // the epoch: nothing below it is reachable through any other view, and
  // negative millisecond boundaries are untested the whole way down. Year
  // view is freely navigable in both directions (spec §4), so this is a
  // guard against a crash, not a policy about which years are interesting —
  // no year anyone can have data for is on the far side of it.
  const YEAR_MIN = 1970;
  const YEAR_MAX = 9998;

  // Derived purely for Header's title, and only in Week view: the week of Mon
  // 29 Jan reads "January" even though it runs into February. Day and Month
  // title themselves from `anchorMs` instead — see `Header`'s own `titleMs`.
  const weekStartMs = $derived(weekStart(new Date(anchorMs)));

  let week = $state<WeekPayload | null>(null);
  let month = $state<MonthPayload | null>(null);
  let year = $state<YearPayload | null>(null);
  let bigYear = $state<BigYearPayload | null>(null);
  let status = $state<AppStatus | null>(null);
  let calendars = $state<Calendar[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  // Opened right after every sign-in (Task 7) — see `handleSignIn` — so a
  // freshly imported set of calendars, all switched on by default, is never
  // left silently syncing without the user having seen the list.
  let pickerOpen = $state(false);

  $effect(() => { applyPalette(); });

  // Live theme reload (spec §10): repaint when the Rust watcher notices
  // `omarchy-theme-set` replaced the theme symlink. A no-op off Linux, since
  // the watcher itself never emits there.
  $effect(() => {
    const un = listen<Palette>('theme-changed', (e) => setPalette(e.payload));
    return () => { un.then((f) => f()); };
  });

  // The system zone moved out from under the process (tz_watch). The fact
  // itself rides on `get_status`, so a refetch is the whole reaction — the
  // header grows its banner from the same field a fresh mount would read.
  $effect(() => {
    const un = listen('system-tz-changed', () => { void refreshStatus(); });
    return () => { un.then((f) => f()); };
  });

  // A newer release was just noticed (update::check_once) — the focus-driven
  // check exists so the banner can appear while the window sits open, and
  // this refetch is the half that actually draws it.
  $effect(() => {
    const un = listen('update-notice', () => { void refreshStatus(); });
    return () => { un.then((f) => f()); };
  });

  // The two external entrances (`omacal 2026-09-01`, a clicked reminder).
  // Anchor-only for a date: the view the user last used is the view they
  // get, exactly as `T` behaves. A clicked reminder also opens the popover
  // on its occurrence — the same landing a chosen search hit gets, down to
  // the keyboard rect, because a notification click has no pointer position
  // either.
  $effect(() => {
    const un = listen<string>('open-date', (e) => { anchorMs = ymdMs(e.payload); });
    return () => { un.then((f) => f()); };
  });
  $effect(() => {
    const un = listen<{ id: number; startMs: number; endMs: number }>('open-event', (e) => {
      anchorMs = dayStart(e.payload.startMs);
      void openOccurrence(e.payload.id, e.payload.startMs, e.payload.endMs, keyboardAnchor());
    });
    return () => { un.then((f) => f()); };
  });

  // A dated *fresh* launch parks its date on the backend (the webview did
  // not exist to hear an event); collect it once on mount. `take` semantics
  // backend-side, so a hot-reload remount cannot replay it.
  $effect(() => {
    void takeOpenDate().then((ymd) => {
      if (ymd) anchorMs = ymdMs(ymd);
    });
  });

  async function refreshStatus() {
    try { status = await getStatus(); } catch (e) { error = String(e); }
  }

  /** The forecast for the day headers, by ISO date. Null until the first
   *  answer; empty when the setting is off or nothing is cached — the
   *  headers treat all three the same, as no sky. A failed fetch keeps the
   *  last map for the tray's reason: weather is a convenience surface. */
  let weather = $state<Map<string, DayWeather> | null>(null);
  async function refreshWeather() {
    try { weather = weatherByDate(await getWeather()); } catch { /* keep the last sky */ }
  }
  // Hourly, between the backend's own three-hour fetches — this only reads
  // the cache, so the cost is a local IPC round trip. Refetched immediately
  // when settings change (the toggle lives there) and at startup below.
  $effect(() => {
    const id = setInterval(() => { void refreshWeather(); }, 3_600_000);
    return () => clearInterval(id);
  });
  // A forecast just landed backend-side (weather::refresh). The launch
  // fetch — wttr.in can take most of a minute — finishes *after* the mount
  // read above, and without this the headers stayed empty until the hourly
  // tick. The update notice's exact race, and its exact fix.
  $effect(() => {
    const un = listen('weather-changed', () => { void refreshWeather(); });
    return () => { un.then((f) => f()); };
  });

  async function refreshCalendars() {
    try { calendars = await getCalendars(); } catch (e) { error = String(e); }
  }

  /** Unanswered invitations, for the header's tray. Refetched wherever the
   *  world may have changed under it: startup, every finished sync, and an
   *  answer given from the tray itself. A failed fetch keeps the last list
   *  rather than raising a banner — the tray is a convenience surface, and
   *  the next sync retries it anyway. */
  let invites = $state<import('./lib/invites').PendingInvite[]>([]);
  /** The tray's other section: guests who declined the user's own meetings.
   *  Same lifecycle, same failure policy. */
  let declines = $state<import('./lib/invites').DeclineNotice[]>([]);
  /** And the attendee's: meetings that moved or were cancelled under them. */
  let changes = $state<import('./lib/invites').ChangeNotice[]>([]);
  async function refreshInvites() {
    try { invites = await pendingInvites(); } catch { /* keep the last list */ }
    try { declines = await declinedGuests(); } catch { /* keep the last list */ }
    try { changes = await changedMeetings(); } catch { /* keep the last list */ }
  }

  // Calendars ride along with status on startup: both describe what's
  // connected, and neither is meaningful before an account exists.
  $effect(() => { refreshStatus(); refreshCalendars(); refreshInvites(); refreshWeather(); });

  /**
   * Whether Day, Week and Month draw as a list rather than a grid.
   *
   * **The stored preference, not a session variable** (filmstrip spec §4). It
   * is seeded from the settings table on startup, which is the whole of what
   * makes it survive a restart — and the only thing a test that flips it in one
   * session cannot tell apart from a variable.
   *
   * Seeded rather than `$derived` from a settings object: this is written from
   * two places (the header's control and `F`) and read on every render, and a
   * round trip to SQLite between the keystroke and the repaint would be visible.
   *
   * A failure to *read* it leaves the grid, silently: an app that will not draw
   * a calendar because it could not recall a display preference is worse than
   * one that draws the default. A failure to *write* it is reported, because
   * the user asked for something and it will not survive the restart.
   */
  let listMode = $state(false);

  /**
   * How many times the user has decided this, so a slow read cannot undo a fast
   * hand.
   *
   * The same stamp `loadWeek` and its three neighbours use, for the same shape
   * of race one layer over: `get_settings` is a round trip, and `F` is a bare
   * key that works the instant the window is listening. Press it while the read
   * is still in flight and the answer — which describes the world *before* the
   * keystroke — lands afterwards and puts the calendar back, having also
   * silently disagreed with the row `set_list_mode` has by then written. Small
   * window, wrong outcome, and no way for the user to tell it from the key not
   * working.
   */
  let listModeChoices = 0;

  /** The stored default for new events, or `null` for the old rule. Seeded
   *  below and kept fresh by `SettingsModal`'s `onsettingschange` — without
   *  that call this copy is stale until a restart, and the form would keep
   *  landing creates on a calendar the user has stopped choosing. */
  let defaultCalendarId = $state<number | null>(null);

  /** Whether the keyboard-shortcut sheet is up. A session flag and not a
   *  setting: it is a thing you look at, not a thing you configure. */
  let helpOpen = $state(false);

  $effect(() => {
    const before = listModeChoices;
    getSettings()
      .then((s) => {
        defaultCalendarId = s.defaultCalendarId;
        setClockFormat(s.timeFormat);
        setWeekStartDay(s.weekStart);
        setSecondZone(s.secondTimezone);
        if (listModeChoices !== before) return; // superseded by the user's own choice
        listMode = s.listMode;
      })
      .catch(() => {});
  });

  async function toggleList() {
    // Same guard the header's control is rendered behind (spec §2). Without it
    // `F` in Big Year would store a preference in the one place that offers no
    // way to see or undo it.
    if (!listable(view)) return;
    listModeChoices += 1;
    listMode = !listMode;
    try {
      await setListMode(listMode);
    } catch (e) {
      error = `The list setting could not be saved: ${e}`;
    }
  }

  // The popover's own reload trigger — a show/hide takes effect the moment
  // the grid re-fetches, since `get_week` filters on `selected` server-side.
  async function handleCalendarChange() {
    // Status rides along because the Accounts list is drawn from it: a
    // freshly connected account must appear in the modal the moment its
    // calendars do, not at the next status poll — "did it work?" deserves
    // an answer the user can see.
    await Promise.all([refreshCalendars(), reload(), refreshStatus()]);
  }

  // What to fetch for the view currently on screen, at the date currently
  // anchored — the "$derived picks which loader to call" half of this task.
  type FetchPlan =
    | { kind: 'day' | 'week'; target: number }
    | { kind: 'month'; year: number; monthNum: number }
    | { kind: 'year'; year: number }
    | { kind: 'bigyear'; year: number };

  const fetchPlan = $derived<FetchPlan>((() => {
    // Day view fetches `anchorMs` itself, not `dayStart(anchorMs)`: `anchorMs`
    // is already maintained at day granularity by every writer (the initial
    // value, `goToday`, `step`, and Month's `handleDayPick`, which hands it
    // Month's own cell boundary verbatim). Re-flooring it here would use the
    // *browser's* local midnight, which can disagree with the boundary the
    // day actually started on — exactly the case for a Month cell whose own
    // start isn't the browser's local midnight (spec §5's anchor-survival
    // guarantee depends on this value reaching Day view unmodified).
    if (view === 'day') return { kind: 'day', target: anchorMs };
    if (view === 'week') return { kind: 'week', target: weekStart(new Date(anchorMs)) };
    if (view === 'month') {
      const d = new Date(anchorMs);
      return { kind: 'month', year: d.getFullYear(), monthNum: d.getMonth() + 1 };
    }
    if (view === 'year') return { kind: 'year', year: yearNum };
    return { kind: 'bigyear', year: bigYearNum };
  })());

  // Every `week` assignment goes through `loadWeek`, and every `loadWeek`
  // call is stamped — same reasoning as before this task, just widened to
  // cover Day alongside Week, since both render through the same `WeekGrid`
  // and the same `week` state. Three callers can have a fetch in flight at
  // once — the navigation effect, `handleSync`, and the `sync-finished`
  // listener — and they do not resolve in the order they were issued. Only
  // the newest request for `week` wins; `month` gets its own independent
  // stamp for the same reason.
  let weekReq = 0;
  let monthReq = 0;
  let yearReq = 0;
  let bigYearReq = 0;

  async function loadWeek(kind: 'day' | 'week', target: number) {
    const req = ++weekReq;
    try {
      const w = kind === 'day' ? await getDay(target) : await getWeek(target);
      if (req !== weekReq) return; // superseded while we were awaiting
      week = w;
      error = null;
    } catch (e) {
      if (req !== weekReq) return;
      error = String(e);
    }
  }

  async function loadMonth(year: number, monthNum: number) {
    const req = ++monthReq;
    try {
      const m = await getMonth(year, monthNum);
      if (req !== monthReq) return;
      month = m;
      error = null;
    } catch (e) {
      if (req !== monthReq) return;
      error = String(e);
    }
  }

  async function loadYear(y: number) {
    const req = ++yearReq;
    try {
      const p = await getYear(y);
      if (req !== yearReq) return;
      year = p;
      error = null;
    } catch (e) {
      if (req !== yearReq) return;
      error = String(e);
    }
  }

  async function loadBigYear(y: number) {
    const req = ++bigYearReq;
    try {
      const p = await getBigYear(y);
      if (req !== bigYearReq) return;
      bigYear = p;
      error = null;
    } catch (e) {
      if (req !== bigYearReq) return;
      error = String(e);
    }
  }

  function runFetchPlan(plan: FetchPlan): Promise<void> {
    if (plan.kind === 'month') return loadMonth(plan.year, plan.monthNum);
    if (plan.kind === 'year') return loadYear(plan.year);
    if (plan.kind === 'bigyear') return loadBigYear(plan.year);
    return loadWeek(plan.kind, plan.target);
  }

  $effect(() => {
    // Reading it here, synchronously, is what makes this effect depend on
    // `fetchPlan` — and, transitively, on `view` and `anchorMs`.
    const plan = fetchPlan;
    // A new view or a new date is a new attempt: a stale failure must not
    // outlive the switch.
    error = null;
    runFetchPlan(plan);
  });

  // The other half of that story: a sync that *fails* has to say so. Nothing
  // else on screen can — the "Synced N ago" label is computed from the last
  // successful sync, so it cannot report its own staleness.
  // The webview is a browser and zooms like one: Ctrl+scroll — and a
  // touchpad pinch, which the engine reports as the same gesture — scales
  // the whole page. Brushing Ctrl while scrolling the grid did exactly
  // that, and "why is everything suddenly huge" is not a question a
  // calendar should pose. The wheel default is cancelable, so cancelling
  // it when Ctrl is down keeps scroll as scroll; `passive: false` is what
  // makes the preventDefault count.
  $effect(() => {
    const blockZoomWheel = (e: WheelEvent) => {
      if (e.ctrlKey) e.preventDefault();
    };
    window.addEventListener('wheel', blockZoomWheel, { passive: false, capture: true });
    return () => window.removeEventListener('wheel', blockZoomWheel, { capture: true });
  });

  $effect(() => {
    const un = listen<{ message?: string }>('sync-failed', (e) => {
      error = e.payload?.message ?? 'Sync failed.';
      // The same cycle may also have marked an account as needing re-consent,
      // and that state rides on `status` (`needs_reauth`), not on the event —
      // without this refetch the reconnect prompt waits for the next
      // successful sync to say so, which for a single dead account is never.
      void refreshStatus();
    });
    return () => { un.then((f) => f()); };
  });

  // Background syncs (Task 4's ticker, focus, wake-from-sleep) land silently;
  // refresh the header and grid so the user sees them without clicking Sync.
  // `reload()` re-runs whatever `fetchPlan` currently says, so it follows the
  // view actually on screen rather than assuming Week.
  async function reload(): Promise<void> {
    await runFetchPlan(fetchPlan);
  }

  $effect(() => {
    const un = listen('sync-finished', async () => {
      await refreshStatus();
      await refreshInvites();
      await reload();
    });
    return () => { un.then((f) => f()); };
  });

  async function handleSignIn() {
    busy = true; error = null;
    try {
      await signIn();
      await refreshStatus();
      // A second account's calendars exist in the store the moment sign_in
      // returns, but nothing else here fetches them: handleSync refreshes
      // status and the events, not the calendar list. Without this, the
      // newly connected account is invisible in the popover until the app
      // is relaunched.
      await refreshCalendars();
      // Open the picker now — calendars are loaded and durable (sign_in wrote
      // them to SQLite before it resolved), even though events are still
      // syncing. Every account imports switched on by default, holidays and
      // room calendars included; this is where the user first gets a say.
      pickerOpen = true;
      await handleSync();
    }
    catch (e) { error = String(e); }
    finally { busy = false; }
  }

  async function handleSync() {
    busy = true; error = null;
    try {
      await syncNow();
      await refreshStatus();
      await reload();
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }

  function goToday() {
    anchorMs = dayStart(Date.now());
  }

  // The chokepoint both the switcher's buttons and the number keys go
  // through, so neither path can diverge from the other. All five slots are
  // live (spec §10) — nothing left to turn away here.
  function pick(v: View) {
    // Spec §5 and the DoD: the anchor survives every switch, and Year is a
    // switch like any other. `yearNum` starts on the real current year, so
    // without this an anchor on 28 Dec 2022 opened Year on the current year
    // instead — a jump of however long the app had been running against that
    // anchor. Re-seeded on every entry rather than only the first, so Year
    // agrees with the Month view the user just came from; Year's own `‹`/`›`
    // move `yearNum` alone, and that navigation is not meant to outlive a
    // trip through another view.
    //
    // Deliberately not `bigYearNum`: Big Year is bounded to the current year
    // and the next, so seeding it from a past anchor would have to either
    // break that bound or drag the anchor forward past it — see its
    // declaration, and `step` below.
    if (v === 'year') yearNum = new Date(anchorMs).getFullYear();
    view = v;
  }

  // `H`/`L` — and the header's own `‹`/`›`, which are the same motion by
  // mouse — step by the current view's unit (spec §7.6): a day, a week, a
  // calendar month, or a calendar year.
  //
  // `setDate`-based throughout (Fix round 1, finding 5): the raw-millisecond
  // arithmetic this replaced (`anchorMs -= WEEK`) shifts the *wall-clock
  // hour* across a real DST transition rather than the calendar day, which
  // can walk `anchorMs` off a day boundary for good — every later step
  // compounds the drift.
  function step(dir: 1 | -1) {
    // Year and Big Year step `yearNum`/`bigYearNum`, not `anchorMs` — see
    // their declaration above for why the two are kept apart.
    if (view === 'year') {
      yearNum = Math.min(Math.max(yearNum + dir, YEAR_MIN), YEAR_MAX);
      return;
    }
    if (view === 'bigyear') {
      // Spec §4: Big Year is a planning surface — what is coming, not what
      // happened — so it is bounded to the real current year and next, and
      // `‹` does nothing once it is already on the earlier bound. Read off
      // the real clock rather than `bigYearNum` itself, so the bound holds
      // even after the tab has sat open across a year rollover.
      const currentYear = new Date().getFullYear();
      bigYearNum = Math.min(Math.max(bigYearNum + dir, currentYear), currentYear + 1);
      return;
    }
    const d = new Date(anchorMs);
    if (view === 'day') d.setDate(d.getDate() + dir);
    else if (view === 'week') d.setDate(d.getDate() + dir * 7);
    else if (view === 'month') {
      // A bare `setMonth` overflows for a day-of-month the target month
      // doesn't have — Jan 31 `+1` rolls past February into Mar 3, not Feb
      // 28/29, and repeating it walks the 3rd of every month forever
      // (Fix round 1, finding 1). Stepping from the 1st avoids the overflow
      // during the month change itself, then clamping to the target month's
      // real last day is the standard fix — it isn't perfectly invertible
      // (Jan 31 `+1``-1` lands on Jan 28/29, not back on 31), but no month
      // is ever skipped or duplicated, which is the actual bug.
      const dom = d.getDate();
      d.setDate(1);
      d.setMonth(d.getMonth() + dir);
      const lastDayOfTarget = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
      d.setDate(Math.min(dom, lastDayOfTarget));
    }
    else return;
    anchorMs = d.getTime();
  }

  // Asked by Month's `+N more` and its day-number click alike (`MonthGrid`
  // makes no distinction between the two — see its own `pickDay`), and by a
  // `YearGrid` date the same way. Setting `anchorMs` here is the entire
  // point of this task (spec §5): without it, Day view opens on today
  // instead of the day that was actually clicked.
  function handleDayPick(startMs: number) {
    anchorMs = startMs;
    view = 'day';
  }

  // Month's and Big Year's shared popover. `WeekGrid` owns this end-to-end
  // for Day/Week, but `MonthGrid` and `BigYearRibbon` only ever hand an
  // `{ event, rect }` pair up through `onopen` (see each one's own doc
  // comment) — the same contract `EventBlock`/`AllDayBand` chips use with
  // WeekGrid, one layer further out. `onresponded` below refreshes, and used
  // to be a no-op on the claim that nothing on screen needs catching up —
  // true of the pixels (neither grid colours its chip by response status) and
  // wrong about identity: an occurrence-scoped RSVP writes an *exception
  // row*, the chip keeps carrying the master's id until the payload reloads,
  // and reopening it read the master's own answer — eternally the old one
  // (seen live, 2026-08-11: decline, reopen, "Yes", decline again).
  //
  // Primitives, not the `UiEvent` object, mirroring `WeekGrid`'s own
  // `selectedId`/`selectedStartMs` — see that component's comment for why
  // (proxy identity of an object reassigned into `$state` is not reliable
  // for a later `===`).
  /** The search overlay, open. Spec §1: it sits *over* the calendar, and
   *  closing without choosing changes nothing behind it. */
  let searchOpen = $state(false);
  /** Captured when quick-add opens so “tomorrow” and the default slot cannot
   * move underneath a line left open across midnight. */
  let quickAdd = $state<{ nowMs: number; anchorDayMs: number } | null>(null);

  function openQuickAdd() {
    quickAdd = { nowMs: Date.now(), anchorDayMs: createDayMs() };
  }

  /**
   * A result was chosen (spec §6): move the calendar to that date **in the
   * view the user is already in**, close search, and open the popover on it.
   *
   * The order matters. Search closes first so it does not linger behind the
   * popover; the anchor moves before the popover opens so the block the
   * popover is about is the one on screen.
   */
  async function goToHit(hit: { eventId: number; startMs: number; endMs: number }) {
    searchOpen = false;
    anchorMs = dayStart(hit.startMs);
    await openOccurrence(hit.eventId, hit.startMs, hit.endMs, keyboardAnchor());
  }

  let gridSelId = $state<number | null>(null);
  let gridSelStart = $state<number | null>(null);
  // Carried for the same reason `WeekGrid`'s own `selectedEndMs` is: the event
  // form needs the clicked occurrence's whole span, and the master's duration
  // is not it. Not part of `isGridSelected` — `id` + `start_ms` already name an
  // occurrence uniquely.
  let gridSelEnd = $state<number | null>(null);
  let gridAnchor = $state<Rect | null>(null);
  let gridDetail = $state<EventDetail | null>(null);

  function isGridSelected(event: UiEvent): boolean {
    return gridSelId === event.id && gridSelStart === event.start_ms;
  }

  async function openGridEvent(event: UiEvent, rect: Rect) {
    await openOccurrence(event.id, event.start_ms, event.end_ms, rect);
  }

  /**
   * Opens the event popover on one occurrence.
   *
   * **The one way to reach an event's detail**, which is search spec §7's
   * constraint rather than tidiness: a second path would be a second set of
   * guards to keep in step, and the popover already owns every one it has.
   * `openGridEvent` is this with a `UiEvent` unpacked; search calls it with a
   * hit's three numbers.
   */
  async function openOccurrence(id: number, startMs: number, endMs: number, rect: Rect) {
    gridSelId = id;
    gridSelStart = startMs;
    gridSelEnd = endMs;
    gridAnchor = rect;
    gridDetail = null;
    // Captured, never re-read: `getEventDetail` is async and another block may
    // be clicked while it is in flight, exactly as `openGridEvent` always
    // guarded.
    const mine = () => gridSelId === id && gridSelStart === startMs;
    try {
      const d = await getEventDetail(id);
      if (mine()) gridDetail = d;
    } catch {
      if (mine()) closeGridEvent();
    }
  }

  function closeGridEvent() {
    gridSelId = null;
    gridSelStart = null;
    gridSelEnd = null;
    gridAnchor = null;
    gridDetail = null;
  }

  // --- Creating, editing and deleting --------------------------------------
  //
  // Every write lives here rather than in the component that offers the
  // control, for one reason: each of the three needs something the component
  // does not have. A create needs a calendar to land on, which is App's
  // `calendars`. An edit and a delete need the *clicked block's* own
  // `start_ms` — never `detail.start_ms` (see `eventdetail.ts`) — which the
  // grid has and the popover does not. And all three need the grid refreshed
  // afterwards, which only App can do.

  /**
   * The calendar a new event lands on unless the user picks another: the
   * signed-in user's own primary, when a create can actually land on it.
   *
   * `offerableCalendarId` is what makes that "when" true. There may be no
   * primary at all — before the first sign-in, or for an account whose own
   * calendar is not synced — and the list can lead with calendars this app
   * cannot write to, a subscribed holiday calendar being the ordinary case.
   * Seeding the form with one of those renders a *blank* select, because no
   * option matches the value, and then saves that id anyway; see
   * `offerableCalendarId`'s own comment for the whole shape of it.
   */
  const createCalendarId = $derived(
    offerableCalendarId(
      // The stored choice first (settings spec: "New events land on"), the
      // primary as the rule when none is stored — and `offerableCalendarId`
      // repairs either one the moment it stops being a calendar a create can
      // land on.
      defaultCalendarId ?? calendars.find((c) => c.is_primary)?.id ?? null,
      calendars,
    ),
  );

  /** What the open form is for. `id` and `occurrenceStartMs` are captured when
   *  the form opens, never re-read from the popover state afterwards: the
   *  popover is already closed by then, and `occurrenceStartMs` is the one
   *  value an edit cannot be allowed to guess. */
  type FormRequest =
    | { mode: 'create'; anchor: Rect; initial: EventFormValue }
    | { mode: 'edit'; anchor: Rect; initial: EventFormValue; id: number; occurrenceStartMs: number };

  let form = $state<FormRequest | null>(null);
  /** The span the open form currently describes, for the grid's live ghost —
   *  the block that appears where the event will land and follows the times
   *  as they are typed (2026-08-20, by request). Null when no form is open,
   *  or when its value describes nothing timed. */
  let formPreview = $state<{ startMs: number; endMs: number } | null>(null);
  $effect(() => {
    if (!form) formPreview = null;
  });
  let pendingDelete = $state<{ occurrence: Occurrence; anchor: Rect } | null>(null);

  /**
   * What Ctrl+V pastes: the last event Ctrl+C copied, held as the form value
   * `valueFromDetail` read off its occurrence (2026-08-20, by request). A
   * value rather than the occurrence, because the detail behind an occurrence
   * can be edited or deleted after the copy — the buffer must keep saying
   * what was copied, not what became of it. Survives popover closes and view
   * changes, replaced by the next copy, and deliberately not the OS
   * clipboard: what lands *there* is a text summary for other apps
   * (`EventPopover` writes it), and paste inside omacal reads this.
   */
  let copiedEvent = $state<EventFormValue | null>(null);

  function copyOccurrence(occurrence: Occurrence) {
    copiedEvent = valueFromDetail(
      occurrence.detail, occurrence.startMs, occurrence.endMs,
    );
  }

  // Where the mouse last was, for paste: Ctrl+V lands the copy on the day
  // under the pointer, the way a click creates where the mouse is
  // (2026-08-20, by request). Plain variables, never `$state` — nothing
  // renders from these, and a reactive assignment per mousemove would be a
  // per-pixel invalidation the whole app pays for.
  let pointerX = 0;
  let pointerY = 0;
  function trackPointer(e: MouseEvent) {
    pointerX = e.clientX;
    pointerY = e.clientY;
  }

  /**
   * Ctrl+V: the copied event as a **new** event on the day under the mouse —
   * every view marks its day elements with `data-start-ms`, so one hit-test
   * answers for Week columns, Month cells, Year days and Big Year rows alike.
   * The form opens beside the pointer, exactly as a click's would, and the
   * grid's ghost appears on the target day the same way. When the pointer is
   * over no day at all — the header, empty chrome — the paste falls back to
   * the day being looked at (`createDayMs`, the `n` answer) and the keyboard
   * anchor, so the chord still works with the mouse parked anywhere.
   *
   * Opened in the form rather than created outright, so guests, notes and
   * the exact times are fine-tuned before anything is written or anybody is
   * mailed. `pastedValue` decides what crosses over and what a create keeps;
   * the calendar is the copied event's own when a create can land on it,
   * repaired exactly as `createCalendarId` repairs the stored choice.
   */
  function pasteCopied() {
    if (!copiedEvent) return;
    const calendarId = offerableCalendarId(
      copiedEvent.calendarId ?? defaultCalendarId ?? null, calendars,
    );
    const host = document
      .elementFromPoint(pointerX, pointerY)
      ?.closest?.('[data-start-ms]') as HTMLElement | null;
    const pointedDayMs = host ? Number(host.dataset.startMs) : NaN;
    const onPointedDay = Number.isFinite(pointedDayMs);
    form = {
      mode: 'create',
      anchor: onPointedDay
        ? { top: pointerY, left: pointerX, width: 0, height: 0 }
        : keyboardAnchor(),
      initial: pastedValue(
        copiedEvent,
        blankValue(Date.now(), calendarId, onPointedDay ? pointedDayMs : createDayMs()),
      ),
    };
  }

  /** A rect for a form nothing was clicked to open — `n`. Zero-sized, a quarter
   *  of the way down and half the panel's width left of centre, which is where
   *  `placePopover`'s prefer-the-right rule then lands the panel: roughly
   *  centred, rather than against an edge. */
  function keyboardAnchor(): Rect {
    return { top: window.innerHeight / 4, left: window.innerWidth / 2 - 170, width: 0, height: 0 };
  }

  /**
   * The day a new event opens on: `anchorMs`, except in Year and Big Year.
   *
   * Those two navigate their own counters and deliberately leave `anchorMs`
   * alone — see their declarations for why — so the anchor is simply not what
   * is on screen there. `n` in Year after two `l`s would otherwise open a form
   * two years behind the grid being read, and `n` is the *only* way to create
   * from Year, which has no empty grid space left to click.
   *
   * The anchor's month and day are kept and moved into the displayed year,
   * which is the inverse of `pick`'s own re-seeding of `yearNum` from
   * `anchorMs`. Clamped to the target month's last day for exactly the reason
   * `step` clamps: 29 February moved into a non-leap year overflows into March
   * rather than failing.
   */
  function createDayMs(): number {
    const shownYear = view === 'year' ? yearNum : view === 'bigyear' ? bigYearNum : null;
    if (shownYear === null) return anchorMs;
    const d = new Date(anchorMs);
    const dom = d.getDate();
    d.setDate(1);
    d.setFullYear(shownYear);
    const lastDayOfTarget = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
    d.setDate(Math.min(dom, lastDayOfTarget));
    return d.getTime();
  }

  /** `n`: a new event on the date the user is looking at, at the next half
   *  hour. Not on *today* — the anchor is the whole point (spec §5), and the
   *  two differ the moment anybody navigates. */
  function newEventOnAnchor() {
    form = {
      mode: 'create',
      anchor: keyboardAnchor(),
      initial: blankValue(Date.now(), createCalendarId, createDayMs()),
    };
  }

  /**
   * Day and Week: the grid names a real time, so the form opens at it.
   *
   * `endMs` arrives only from a **sweep** — a drag across empty grid, which
   * names both ends. A click names only the moment it landed on, and leaves the
   * duration to `blankValueAt`'s own hour; passing one there would move
   * the decision out of the form module and into the grid.
   *
   * Nothing is created here either way. The form does the creating, through the
   * path it has always used, which is why a sweep needs no command of its own —
   * a second way to make an event is a second thing to keep correct, and the
   * less-used one rots.
   */
  function newEventAt(startMs: number, rect: Rect, endMs?: number) {
    form = {
      mode: 'create', anchor: rect, initial: blankValueAt(startMs, createCalendarId, endMs),
    };
  }

  /** Month and Big Year: the grid names a date and no time, so the form takes
   *  the same default hour `n` would have used — the next half hour, moved to
   *  the day that was clicked. */
  function newEventOnDay(dayStartMs: number, rect: Rect) {
    form = {
      mode: 'create',
      anchor: rect,
      initial: blankValue(Date.now(), createCalendarId, dayStartMs),
    };
  }

  /**
   * Edit, from either popover.
   *
   * `occurrence.startMs`/`endMs` are the clicked block's own, and they are what
   * `valueFromDetail` is given as well as what `updateEvent` will be handed
   * below. Both from the same pair is the invariant `updateEvent`'s doc comment
   * rests on: the Rust side reads a time change as the difference between
   * `fields.startMs` and `occurrenceStartMs`, so two values from different
   * sources make an untouched time look like a move of weeks.
   */
  function openEdit(occurrence: Occurrence, rect: Rect) {
    closeGridEvent();
    form = {
      mode: 'edit',
      anchor: rect,
      id: occurrence.detail.id,
      occurrenceStartMs: occurrence.startMs,
      initial: valueFromDetail(occurrence.detail, occurrence.startMs, occurrence.endMs),
    };
  }

  /** Delete, from either popover. Nothing is deleted here — this opens the
   *  confirmation, which is where the three scopes, the guest count and the
   *  "no undo" live. */
  function askDelete(occurrence: Occurrence, rect: Rect) {
    closeGridEvent();
    pendingDelete = { occurrence, anchor: rect };
  }

  /**
   * Where every successful write ends.
   *
   * A *sync*, not just a re-read, and that is the whole point of the function.
   * Two of the write paths deliberately leave the local store alone: a `'this'`
   * edit or delete against a bare recurring master patches a Google resource
   * this app has no row for, so the backend correctly skips its write-back
   * (see `update_event`'s and `delete_event_cmd`'s own comments), and a
   * `'following'` delete opened from an exception row is the same shape. Asking
   * the database again would repaint exactly what is already on screen — the
   * old title, or the block the user just deleted — for up to a sync interval.
   *
   * The local reload runs first and unconditionally, so whatever the write
   * *did* fold back shows immediately and a sync that cannot reach Google still
   * leaves the grid as current as the store is. The failure is reported without
   * claiming the write itself did not happen, because it did.
   */
  async function refreshAfterWrite() {
    await reload();
    // The invitation badge is a function of self_response, and an RSVP from
    // the popover is a write that changes it — without this, answering an
    // invitation on its block left the tray claiming it for up to a sync
    // interval (noticed live, 2026-08-17, minutes after the tray shipped).
    await refreshInvites();
    busy = true;
    try {
      await syncNow();
      await refreshStatus();
      await reload();
    } catch (e) {
      error = `The change was made, but omacal could not refresh from Google: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function saveForm(result: EventFormResult) {
    const request = form;
    if (!request) return;
    form = null;
    busy = true;
    error = null;
    try {
      if (request.mode === 'create') {
        // **`result.notify`, never a constant** — the same rule the edit arm
        // below states at length. A create used to be structurally unable to
        // mail anybody, so `create_event` sent `sendUpdates=none` on the Rust
        // side and there was nothing here to carry. Now a create can invite
        // people, the form asks, and this carries the answer.
        await createEvent(result.calendarId, result.fields, result.notify);
      } else {
        // `request.occurrenceStartMs`, never `detail.start_ms`: for a series
        // the second is the master's DTSTART, and an edit aimed at it patches
        // occurrence #0 with the whole form as its payload. The scope comes
        // from the form's own chooser (Task 9).
        //
        // **`result.notify`, never a constant.** This used to be `'all'`, on
        // the reasoning that a time typed on purpose and saved is exactly the
        // change guests need to hear about. That was right while the form
        // could only change the event; guest-list spec §3 makes it a choice,
        // because the same Save now also fixes a typo in an address, and
        // mailing the whole room about that is the outcome the choice exists
        // to prevent. `App` does not decide it — the form asks, and this
        // carries the answer.
        await updateEvent(
          request.id, result.scope, request.occurrenceStartMs, result.fields, result.notify,
        );
      }
    } catch (e) {
      error = String(e);
      // One failure is not a failure: a create that reached Google but not
      // the local store answers with the backend's fixed created-not-stored
      // sentence (events.rs, safelisted verbatim). The event exists — guests
      // are already mailed — so this must NOT stop like the errors above,
      // where stopping invites the user to create the event again. Falling
      // through to refreshAfterWrite runs the ordinary post-write sync,
      // which fetches the event like any other; the banner keeps the
      // sentence so the user knows what happened.
      if (!(request.mode === 'create' && String(e).startsWith('The event was created on Google'))) {
        return;
      }
    } finally {
      busy = false;
    }
    await refreshAfterWrite();
  }

  /** Quick-add's direct create. It deliberately mirrors the create arm above,
   * including the “created on Google but not stored yet” recovery: the event
   * already exists in that case and retrying would duplicate it and its mail. */
  async function saveQuick(result: EventFormResult) {
    if (!quickAdd) return;
    quickAdd = null;
    busy = true;
    error = null;
    try {
      await createEvent(result.calendarId, result.fields, result.notify);
    } catch (e) {
      error = String(e);
      if (!String(e).startsWith('The event was created on Google')) return;
    } finally {
      busy = false;
    }
    await refreshAfterWrite();
  }

  function continueQuick(value: EventFormValue) {
    quickAdd = null;
    form = { mode: 'create', anchor: keyboardAnchor(), initial: value };
  }

  /**
   * A drop that needs asking before it writes.
   *
   * Held here, not written: the dialog is mounted from this, and the write
   * happens in `commitMove` once the user has answered. `anchor` is the block
   * as it was dropped, so the panel appears beside what was moved.
   */
  let pendingMove = $state<{
    event: UiEvent;
    span: { startMs: number; endMs: number };
    detail: EventDetail;
    anchor: Rect;
  } | null>(null);

  /**
   * A dropped drag.
   *
   * **Two questions decide whether anything is asked**, and both come off the
   * detail rather than off the gesture: does this event have anybody to notify,
   * and does it repeat. Neither → write immediately, `'none'`, no dialog. §2 is
   * explicit that silence is correct where there is nobody to tell, and a
   * confirmation nobody needs is a confirmation people learn to dismiss.
   *
   * Either or both → one dialog (`MoveConfirm`), never two in sequence. When an
   * event both repeats and has guests the scope prompt carries the notify
   * choice, which is spec §3's "at most one dialog per drop".
   */
  async function moveOccurrence(event: UiEvent, span: { startMs: number; endMs: number }) {
    busy = true;
    error = null;
    let detail: EventDetail;
    try {
      detail = await getEventDetail(event.id);
    } catch (e) {
      error = String(e);
      return;
    } finally {
      busy = false;
    }

    const guests = detail.attendees.filter((a) => !a.is_self).length;
    if (guests === 0 && !detail.is_recurring) {
      // Nobody to tell and one occurrence to move: nothing to ask.
      await commitMove(event, span, { scope: 'all', sendUpdates: 'none' });
      return;
    }

    const anchor = gridRectFor(event);
    pendingMove = { event, span, detail, anchor };
  }

  /**
   * The write behind a drop, once there is an answer.
   *
   * `sendUpdates` arrives from the caller and is **never chosen here**: an
   * unasked drop passes `'none'` above, and the only other caller is the
   * dialog's own confirm. That is what makes `MoveConfirm`'s "Move and notify
   * guests" button the one place in this path where `'all'` can come from.
   */
  async function commitMove(
    event: UiEvent,
    span: { startMs: number; endMs: number },
    choice: { scope: Scope; sendUpdates: SendUpdates },
  ) {
    busy = true;
    error = null;
    try {
      const detail = await getEventDetail(event.id);
      const value = valueFromDetail(detail, event.start_ms, event.end_ms);
      // The source instants are carried through untouched, and that is
      // deliberate rather than an oversight: `instantOf` passes one through
      // only while the civil pair beside it still reads as that instant, which
      // after a move it never does. Nulling them would be dead code here — a
      // mutation keeping them reddened nothing — and actively wrong for the
      // resize this grows into, where the *untouched* end should be sent as
      // the instant it was read off rather than re-derived without its
      // seconds.
      const moved: EventFormValue = {
        ...value,
        date: dateOf(span.startMs),
        endDate: dateOf(span.endMs),
        start: timeOf(span.startMs),
        end: timeOf(span.endMs),
      };

      await updateEvent(
        event.id,
        choice.scope,
        event.start_ms,
        toEventInput(moved, value),
        choice.sendUpdates,
      );
    } catch (e) {
      // The block is already back where it started — the grid returns it on
      // drop and only a refresh moves it — so a failure needs no undo, just
      // saying. §6: a drag that appears to have worked and silently did not is
      // worse than one that visibly refuses.
      error = String(e);
      return;
    } finally {
      busy = false;
    }
    await refreshAfterWrite();
  }

  /** The dropped block's own rect, for the dialog to sit beside. Falls back to
   *  the viewport centre if the block is no longer in the DOM — a background
   *  reload can replace the week between the drop and this call. */
  function gridRectFor(event: UiEvent): Rect {
    const el = [...document.querySelectorAll<HTMLElement>('.ev')].find(
      (n) => n.getAttribute('title') === (event.title ?? ''),
    );
    if (!el) return { top: window.innerHeight / 2, left: window.innerWidth / 2, width: 0, height: 0 };
    const r = el.getBoundingClientRect();
    return { top: r.top, left: r.left, width: r.width, height: r.height };
  }

  async function runDelete(scope: Scope) {
    const target = pendingDelete;
    if (!target) return;
    pendingDelete = null;
    busy = true;
    error = null;
    try {
      // Same rule, and it bites hardest here: `'this'` aimed at the master's
      // DTSTART removes the series' *first* occurrence rather than the one the
      // user clicked, and mails everybody about it.
      await deleteEvent(target.occurrence.detail.id, scope, target.occurrence.startMs);
    } catch (e) {
      error = String(e);
      return;
    } finally {
      busy = false;
    }
    await refreshAfterWrite();
  }

  // Keys are dropped when the user is typing (an `input`/`textarea`) or when
  // focus is inside the event popover — RSVP buttons and a description live
  // there, and a stray `3` while it has focus must not switch views behind
  // it. `.pop` is `EventPopover`'s own root class.
  function isTypingTarget(e: KeyboardEvent): boolean {
    const t = e.target as HTMLElement | null;
    if (!t) return false;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA') return true;
    return !!t.closest?.('.pop');
  }

  /**
   * What each shortcut does. Keyed by `ShortcutId`, so `Record` makes an entry
   * added to `SHORTCUTS` without a handler here a **compile error** rather
   * than a row in the help sheet that does nothing when pressed. That is the
   * whole reason the table and this map are two halves of one thing: the
   * sheet and the handler read the same array, and neither can grow a key the
   * other lacks.
   *
   * The five view keys are numbers, not initials, because `Y` is wanted for
   * both "year" and "yes, accept" (spec §7.6) — the reason `KEY_VIEW` gave
   * before this map replaced it. Their target now travels in the table's own
   * `view` field, so a key that switches views says so in one place.
   */
  const SHORTCUT_ACTIONS: Record<ShortcutId, () => void> = {
    day: () => pick('day'),
    week: () => pick('week'),
    month: () => pick('month'),
    year: () => pick('year'),
    bigyear: () => pick('bigyear'),
    prev: () => step(-1),
    next: () => step(1),
    today: goToday,
    // `/` joins the bare-key family rather than inventing a modifier chord: it
    // is the search key everywhere that has one, it collides with nothing
    // here, and it is unshifted on the layouts this ships to. `isTypingTarget`
    // keeps it out of every field, which is what makes a punctuation key safe
    // to claim. Its `consumes` flag is why — see below.
    search: () => (searchOpen = true),
    quickCreate: openQuickAdd,
    create: newEventOnAnchor,
    // `F`, joining the same bare-key family as the rest (spec §1). It is a
    // no-op in Year and Big Year, where the control it duplicates is absent —
    // see `toggleList`.
    list: toggleList,
    help: () => (helpOpen = true),
  };

  function handleKeydown(e: KeyboardEvent) {
    if (isTypingTarget(e)) return;
    // The one modifier chord omacal claims: Ctrl+V (⌘V), paste the copied
    // event. Ahead of the modifier bail-out below, and narrower than it
    // looks — an empty buffer passes the chord through untouched, so V means
    // nothing new until a copy has meant something first. Copy's half lives
    // in `EventPopover`, the only place that knows a popover is open in
    // every view; the same guard set as the bare keys keeps a paste from
    // opening a second form over one already up.
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'v') {
      if (copiedEvent && !form && !pendingDelete && !searchOpen && !quickAdd && !helpOpen) {
        e.preventDefault();
        pasteCopied();
      }
      return;
    }
    // A modifier means the key belongs to the browser or the OS, not to
    // omacal: ⌘N opens a window and ⌘L focuses a location bar. Every shortcut
    // below is a bare key, so this turns nothing off that ever worked — and it
    // keeps ⌘N in particular from opening an event form behind whatever the
    // platform does with it.
    //
    // Shift is deliberately *not* here: `?` is Shift-`/` on the layouts this
    // ships to, and the sheet it opens is the one shortcut a reader needs
    // before they know any of the others.
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    // Nothing on the keyboard reaches the views while a form, a delete
    // confirmation, the search overlay or the shortcut sheet is open. Their
    // scrims have already made everything behind them unclickable, and `n`
    // opening a second form on top of the first is the same mistake by
    // keyboard. Escape is unaffected: each panel listens for it on `window`
    // itself.
    if (form || pendingDelete || searchOpen || quickAdd || helpOpen) return;

    // Lowercased, which is what makes `H` step like `h` — the old `switch`
    // did the same, and digits and punctuation are unaffected (`'?'` and
    // `'/'` lowercase to themselves).
    const hit = SHORTCUT_LIST.find((s) => s.key === e.key.toLowerCase());
    if (!hit) return;
    // Consumed where the table says so, which today is `/` alone: WebKitGTK
    // runs the keydown's default action — inserting the character — *after*
    // the overlay has mounted and focused its field, so an unconsumed `/`
    // lands in the input it just opened and "sync" is searched as "/sync".
    if (hit.consumes) e.preventDefault();
    SHORTCUT_ACTIONS[hit.id]();
  }
</script>

<svelte:window onkeydown={handleKeydown} onmousemove={trackPointer} />

<!-- The webview's own context menu — Reload, Back, View Source — is browser
     chrome inside what presents itself as a native app, so it is suppressed
     everywhere except the places right-click genuinely works for the user:
     text fields, where it carries copy and paste. The grid's own right-click
     behaviour (create at that slot) lives in WeekGrid; this only decides
     whether the browser menu may appear. -->
<main
  oncontextmenu={(e) => {
    const t = e.target as HTMLElement;
    if (t.closest('input, textarea, select, [contenteditable="true"]')) return;
    e.preventDefault();
  }}
>
  <Header
    {status} {anchorMs} {weekStartMs} {busy} {error} {calendars} {view} {listMode}
    onToggleList={toggleList}
    onPrev={() => step(-1)}
    onNext={() => step(1)}
    onToday={goToday}
    onQuickAdd={openQuickAdd}
    onSearch={() => (searchOpen = true)}
    onsettingschange={(s) => {
      defaultCalendarId = s.defaultCalendarId;
      setClockFormat(s.timeFormat);
      setWeekStartDay(s.weekStart);
      setSecondZone(s.secondTimezone);
      // The weather toggle lives in the same modal; refetching on every
      // settings change is one cache read, and it is what makes flipping
      // the switch change the headers now rather than within the hour.
      void refreshWeather();
    }}
    onSignIn={handleSignIn}
    onWhatsNew={() => { void openLatestRelease(); }}
    onRestart={() => { void restartApp(); }}
    onUpdate={installUpdate}
    onSync={handleSync}
    oncalendarchange={handleCalendarChange}
    {invites}
    {declines}
    {changes}
    oninvitesanswered={() => { void refreshInvites(); void reload(); }}
    onpick={pick}
    bind:open={pickerOpen}
  />
  <!-- **One preference, branched per view.** `listMode` is a single stored
       value (spec §4) and Year and Big Year simply have no branch for it (§2);
       the two that do reach the same `Filmstrip`, differing only in which
       payload it was built from.

       `WeekGrid` and `MonthGrid` are not mounted at all while it is on, which
       is what makes "drag is absent, not disabled" (spec §6) a property of the
       markup rather than a flag somebody has to keep passing down. -->
  {#if view === 'month'}
    {#if month}
      {#if listMode}
        <Filmstrip days={daysFromMonth(month)} {weather} onopen={openGridEvent} />
      {:else}
        <MonthGrid {month} onopen={openGridEvent} ondaypick={handleDayPick} oncreate={newEventOnDay} />
      {/if}
    {/if}
  {:else if view === 'year'}
    <!-- No `oncreate` here, and deliberately: every day in Year view is
         already a button that opens that day (`ondaypick`, spec §5), so there
         is no empty space in the grid left to mean anything else. The route to
         a new event from Year is the one the view is for — pick the day, then
         create in it, or press `n`. -->
    {#if year}
      <YearGrid {year} ondaypick={handleDayPick} />
    {/if}
  {:else if view === 'bigyear'}
    {#if bigYear}
      <!-- `gridSelId`/`gridSelStart` are handed straight down: they already
           name the occurrence whose popover is open — `isGridSelected` above
           tests exactly this pair — and the ribbon keeps every segment of it
           lit while it is. Nothing new is tracked here; the state existed. -->
      <BigYearRibbon
        ribbon={bigYear}
        openId={gridSelId}
        openStart={gridSelStart}
        onopen={openGridEvent}
        oncreate={newEventOnDay}
      />
    {/if}
  {:else if week}
    {#if listMode}
      <Filmstrip days={daysFromWeek(week)} {weather} onopen={openGridEvent} />
    {:else}
      <WeekGrid {week} {weather} {formPreview} oncreate={newEventAt} onedit={openEdit} ondelete={askDelete}
                oncopy={copyOccurrence}
        onmove={moveOccurrence} onresponded={refreshAfterWrite} />
    {/if}
  {/if}
</main>

{#if pendingMove}
  <!-- One dialog per drop, never two: `MoveConfirm` carries the scope choice
       and the notify choice together when the event needs both. Cancelling
       clears this and writes nothing — the block is already home. -->
  {@const p = pendingMove}
  <MoveConfirm
    detail={p.detail}
    anchor={p.anchor}
    onconfirm={(choice) => {
      // Read into plain locals **before** clearing `pendingMove`. `{@const}`
      // is a lazily re-derived value in Svelte 5, not a snapshot taken at
      // render, so `p` follows the state it was derived from — clearing first
      // and reading `p.event` afterwards throws on null, which is precisely
      // what it did.
      const { event, span } = p;
      pendingMove = null;
      commitMove(event, span, choice);
    }}
    oncancel={() => (pendingMove = null)}
  />
{/if}

{#if searchOpen}
  <SearchOverlay onclose={() => (searchOpen = false)} onpick={goToHit} />
{/if}

{#if quickAdd}
  <QuickEventModal
    nowMs={quickAdd.nowMs}
    anchorDayMs={quickAdd.anchorDayMs}
    calendarId={createCalendarId}
    defaultDurationMinutes={30}
    {calendars}
    oncreate={saveQuick}
    onedit={continueQuick}
    onclose={() => (quickAdd = null)}
  />
{/if}

{#if helpOpen}
  <ShortcutSheet onclose={() => (helpOpen = false)} />
{/if}

{#if gridSelId !== null && gridSelStart !== null && gridAnchor && gridDetail}
  {@const startMs = gridSelStart}
  {@const rect = gridAnchor}
  <!-- Captured at this render, not read back off the module state inside the
       callbacks: `openEdit`/`askDelete` both call `closeGridEvent()` first, so
       by the time they need these values the state they came from is null.
       `endMs` falls back to `startMs` only to satisfy the type — the `{#if}`
       above already proves a block is selected, and `gridSelEnd` is assigned
       and cleared in lockstep with `gridSelStart`. -->
  {@const occurrence = { detail: gridDetail, startMs, endMs: gridSelEnd ?? startMs }}
  <EventPopover
    detail={gridDetail}
    anchor={gridAnchor}
    occurrenceStartMs={startMs}
    occurrenceEndMs={occurrence.endMs}
    onclose={closeGridEvent}
    onresponded={refreshAfterWrite}
    onedit={() => openEdit(occurrence, rect)}
    ondelete={() => askDelete(occurrence, rect)}
    oncopy={() => copyOccurrence(occurrence)}
  />
{/if}

{#if form}
  <EventForm
    anchor={form.anchor}
    initial={form.initial}
    {calendars}
    onsave={saveForm}
    oncancel={() => (form = null)}
    onvaluechange={(v) => (formPreview = previewSpan(v))}
  />
{/if}

{#if pendingDelete}
  <DeleteConfirm
    detail={pendingDelete.occurrence.detail}
    anchor={pendingDelete.anchor}
    onconfirm={runDelete}
    oncancel={() => (pendingDelete = null)}
  />
{/if}

<style>
  /* The flex chain every view hangs off, and the whole of this app's opinion
     about height.

     Each of the four views used to size itself with `calc(100vh - <a guess at
     what surrounds it>)` — 150px in three of them, 190px in the ribbon — and
     every guess was too big. Measured at 1920x1080 before this changed: Week
     left 42px of the window unclaimed below its last hour, Month 69px, Year
     79px, and Big Year 123px with no legend on it (~95px with one, which is
     what got reported). The guesses could not have been right for long
     anyway: `Header`'s own height is whatever its buttons and its
     `flex-wrap` come to, and it grows by a whole error banner the moment
     anything fails.

     So nothing here names a chrome height. `main` fills the window, `Header`
     takes what it needs, and the view takes the rest — see each view's own
     `flex: 1` for the other half of it. */
  :global(html), :global(body), :global(#app) { height: 100%; }
  :global(body) { background: var(--bg); color: var(--text); margin: 0;
                  font-family: -apple-system, 'SF Pro Text', Inter, system-ui, sans-serif; }

  /* `border-box` is load-bearing: without it the 28px of vertical padding
     lands *outside* the 100%, and the window scrolls by exactly that much.
     28px is still what this costs vertically — `Header` carries the top 14 as
     its own `padding-top` rather than taking it from here.

     That move is layout-neutral by construction and not cosmetic. With
     `titleBarStyle: "Overlay"` the webview reaches the top edge of the window,
     and a drag region only covers its own box: 14px of `main`'s padding above
     the header left the strip a macOS user reaches for first doing nothing at
     all. Inside `Header` the same 14px is part of an element that *is* a drag
     region. Nothing moves — the header's content still starts at y=14, its
     `margin-bottom` still puts a view at y=49, and `APP_CHROME_PX` is still
     63 (0 + 37 + 12 + 14 rather than 14 + 23 + 12 + 14); `app.spec.ts`'s "a
     standalone view gets the same box the app gives it" is what proves that
     rather than this comment.

     The horizontal 16 stays here, because it is the gutter every *view* is
     drawn to and not just the header's. */
  main { padding: 0 16px 14px; box-sizing: border-box; height: 100%;
         display: flex; flex-direction: column; }
</style>
