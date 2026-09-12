<!-- ui/src/lib/SettingsModal.svelte -->
<script lang="ts">
  import { DEFAULT_MEETING_FORMAT, meetingLabel } from '../../../packaging/omarchy-plugin/Timeline.mjs';
  import { DATE_FORMATS, type DateFormat } from './datefmt';
  import { setDateFormatPreference } from './settings';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';

  import { escapeCloses } from './dismiss.svelte';
  import {
    applyAppearance, type AppearancePreferences, type EventCornerStyle,
  } from './appearance';
  import { REMINDER_UNITS, reminderAmountOf, reminderMax, reminderUnitOf } from './reminders';
  import CalendarList from './CalendarList.svelte';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { calendarColor, offerableCalendarId, writableCalendars, type Calendar } from './calendars';
  import { connectCaldav } from './tasks';
  import { listAccounts, signOut, type Account } from './accounts';
  import {
    getSettings, listTimezones, minutesOf, msOfMinutes, setAppearancePreferences,
    setDefaultCalendar, setDefaultView, setDefaultViewFollowsLast, DEFAULT_VIEW_OPTIONS,
    type DefaultViewChoice,
    setDefaultEventDuration, setStartOnLogin, START_ON_LOGIN_OPTIONS,
    setDisplayTimezone, setFallbackReminders, setNotificationsEnabled,
    setAppearance, APPEARANCE_OPTIONS,
    setQuitOnClose, setSecondTimezone, setSyncInterval, setTemperatureUnit, setTimeFormat,
    setMenubarLabelFormat, setMenubarDateFormat, setMenubarPreferences, setShowDate, setTrayIcon, setWeatherEnabled, setWeekStart,
    setWeekStartsToday, setWeekViewDays, setVisibleHours,
    type AppSettings, type Appearance, type StartOnLogin, type WeekViewDays,
    type WindowFrame, WINDOW_FRAME_OPTIONS, setWindowFrame,
  } from './settings';
  import { formatClock, type TimeFormat } from './timefmt';
  import type { TemperatureUnit } from './temperature';
  import type { WeekStartDay } from './weekstart';

  async function changeVisibleHours(start: number, end: number) {
    try { settings = await setVisibleHours(start, end); onsettingschange?.(settings); }
    catch (e) { note = { text: String(e), kind: "error" }; }
  }
  let {
    accounts,
    version = '',
    busy,
    signingIn = false, onCancelSignIn = () => {},
    calendars,
    onclose,
    onSignIn,
    oncalendarchange,
    onappearancechange,
    onsettingschange,
  }: {
    /** The connected accounts, from `AppStatus`. Read only — this modal adds
     *  one through `onSignIn` and cannot remove one, because nothing can yet. */
    accounts: string[];
    /** The running build's version, from `AppStatus` — the one place the app
     *  says what it is, which a bug report and the update notice both need
     *  the user able to find. Empty until status lands; the footer hides
     *  rather than claim "OmaCal " and nothing. */
    version?: string;
    busy: boolean;
    signingIn?: boolean;
    onCancelSignIn?: () => void;
    /** Every calendar the app knows about, handed straight to `CalendarList` —
     *  the same rows the header's popover shows, from the same component. */
    calendars: Calendar[];
    /** Told after every saved settings change, with the settings as the
     *  backend now holds them. `App` derives the create-default from these,
     *  and without this call its copy is stale until a restart. */
    onsettingschange?: (s: AppSettings) => void;
    /** Live appearance preview. Separate from `onsettingschange`: dragging a
     *  range should repaint the calendar behind this modal without causing
     *  every unrelated settings reaction (including a weather cache read). */
    onappearancechange?: (s: AppearancePreferences) => void;
    onclose: () => void;
    onSignIn: () => void;
    /** A calendar was shown, hidden, added or removed: reload. Passed through
     *  untouched, exactly as the popover passes it. */
    oncalendarchange: () => void;
  } = $props();

  /** The settings as the backend holds them, or `null` until they land. */
  let settings = $state<AppSettings | null>(null);
  /** What is in the interval box, in minutes, as a string — a form value, not a
   *  number, so a half-typed "1" is not read as one minute mid-keystroke. */
  let intervalText = $state('');
  let durationText = $state('');
  let menuDateCustom = $state('%-d');
  let menuDateNote = $state('');
  let meetingFormat = $state(DEFAULT_MEETING_FORMAT);
  /** The menu-bar pane's own three notes, each beside the control it is
   *  about — the `rownote` rule below, which this section did not follow
   *  when it arrived. The shared `note` at the foot of the modal was
   *  reporting these saves, and the foot of a scrolling modal is exactly
   *  where "Saved." goes unseen.
   *
   *  They say *Applying…* first and *Saved* after, rather than only the
   *  second: `set_menubar_preferences` rewrites the feed and then pokes the
   *  Omarchy widget over IPC, waiting up to two seconds for it. That pause
   *  is real work, and a control that looks inert for two seconds and then
   *  changes the bar by itself reads as a control that needed an Apply
   *  nobody could find. */
  let labelNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  let formatNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  let joinNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  let menubarBusy = $state(false);
  /** Whether the format box holds something the bar has not been told about.
   *  What makes Save mean "there is something to save" rather than being a
   *  button that is always there and usually does nothing. */
  const formatDirty = $derived(
    !!settings && meetingFormat !== (settings.menubarLabelFormat ?? DEFAULT_MEETING_FORMAT),
  );
  let note = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  /** The interval row's own feedback, rendered beside the field it is about.
   *  Not the shared `note` below: that one sits at the bottom of a modal
   *  that scrolls, so "Saved." was landing off-screen exactly when the
   *  General tab had enough content to scroll — reported as "no visual clue
   *  it saved" (2026-08-17), which for every practical purpose it was. */
  let intervalNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  let durationNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);

  $effect(() => {
    getSettings()
      .then((s) => {
        settings = s;
        menuDateCustom = s.menubarDateCustom ?? '%-d';
        meetingFormat = s.menubarLabelFormat ?? DEFAULT_MEETING_FORMAT;
        intervalText = String(minutesOf(s.syncIntervalMs));
        durationText = String(s.defaultEventDurationMinutes);
        applyAppearance(s);
        onappearancechange?.(s);
      })
      .catch((e) => (note = { text: String(e), kind: 'error' }));
  });

  async function saveMeetingFormat(template = meetingFormat) {
    menubarBusy = true;
    formatNote = { text: 'Applying…', kind: 'info' };
    try {
      settings = await setMenubarLabelFormat(template);
      meetingFormat = settings.menubarLabelFormat;
      formatNote = { text: 'Saved', kind: 'info' };
      onsettingschange?.(settings);
    } catch (e) { formatNote = { text: String(e), kind: 'error' }; }
    finally { menubarBusy = false; }
  }

  async function saveMenuDate(format: AppSettings['menubarDateFormat'], custom = settings?.menubarDateCustom ?? '%-d') {
    try {
      settings = await setMenubarDateFormat(format, custom);
      menuDateCustom = settings.menubarDateCustom;
      menuDateNote = '';
      onsettingschange?.(settings);
    } catch (e) { menuDateNote = String(e); }
  }

  /** `where` names the control that asked, so the answer lands beside it. */
  async function saveMenubar(
    label = settings?.menubarLabel ?? true,
    joinMinutes = settings?.menubarJoinMinutes ?? 5,
    where: 'label' | 'join' = 'label',
  ) {
    const say = (n: { text: string; kind: 'info' | 'error' } | null) => {
      if (where === 'label') labelNote = n; else joinNote = n;
    };
    menubarBusy = true;
    say({ text: 'Applying…', kind: 'info' });
    try {
      settings = await setMenubarPreferences(label, joinMinutes);
      onsettingschange?.(settings);
      say({ text: 'Saved', kind: 'info' });
    } catch (e) { say({ text: String(e), kind: 'error' }); }
    finally { menubarBusy = false; }
  }

  const floorMinutes = $derived(settings ? minutesOf(settings.minSyncIntervalMs) : 1);

  /** 13:30 on an arbitrary day — the sample the two options are drawn with.
   *  A fixed instant rather than `Date.now()`: an option list that reads
   *  differently every time the modal opens is a control that looks broken,
   *  and a screenshot of it could never be compared. */
  const SAMPLE_MS = new Date(2026, 0, 1, 13, 30).getTime();

  /**
   * Stores the clock format and takes the settings as they now are.
   *
   * No optimistic local write: `settings` is replaced by what the backend
   * answers, the same as every other control here, so the select can only ever
   * show a value the database actually holds. `onsettingschange` is what
   * repaints the calendar behind the modal — `App` owns the rune, and this
   * modal deliberately does not reach past its own props to set it.
   */
  type WeekStartChoice = WeekStartDay | 'today';

  /**
   * What Week view shows, as **one** question.
   *
   * This used to be two controls, and the second only appeared once the
   * first said "Today" — so the rolling three-day view existed and could not
   * be found: someone looking for it read the whole panel, saw a row called
   * "Week view starts on", and concluded the app had no such thing (issue
   * #31). A setting nobody can discover is not a setting.
   *
   * The two stored values behind it stay two, because they are not the same
   * fact: `weekStart` still aligns the rows of Month, Year and Big Year even
   * while Week rolls from today, and `weekViewDays` only means anything while
   * it does (`App`: `days: weekStartsToday ? weekViewDays : 7`). This list is
   * the crossing of the two that the grid can actually draw — which is why a
   * "3 days from Monday" option is absent rather than disabled: there is no
   * such shape.
   */
  type WeekViewChoice =
    | { id: WeekStartDay; label: string; days?: never }
    | { id: `today-${WeekViewDays}`; label: string; days: WeekViewDays };

  const WEEK_VIEWS: WeekViewChoice[] = [
    { id: 'monday', label: 'Whole week, from Monday' },
    { id: 'sunday', label: 'Whole week, from Sunday' },
    { id: 'saturday', label: 'Whole week, from Saturday' },
    { id: 'today-3', label: '3 days from today', days: 3 },
    { id: 'today-5', label: '5 days from today', days: 5 },
    { id: 'today-7', label: '7 days from today', days: 7 },
  ];

  /** The fixed day on its own, for the rolling hint — which has to name the
   *  alignment Month and Year still use while the select is showing a
   *  rolling row and cannot say it. */
  const DAY_LABEL: Record<WeekStartDay, string> = {
    monday: 'Monday',
    sunday: 'Sunday',
    saturday: 'Saturday',
  };

  /** Which row the stored pair of settings is currently on. */
  const weekViewChoice = $derived(
    settings?.weekStartsToday
      ? (`today-${settings.weekViewDays}` as const)
      : (settings?.weekStart ?? 'monday'),
  );

  /** Stores the first day of the week. Same shape as `saveTimeFormat`: the
   *  backend's answer replaces `settings`, and `onsettingschange` is what
   *  repaints the calendar behind the modal. */
  /** The zone picker's option list, fetched once when General first shows a
   *  settings object — ~600 rows, not worth loading for a modal opened to
   *  toggle a reminder. */
  let timezones = $state<string[]>([]);
  $effect(() => {
    if (settings && timezones.length === 0) {
      listTimezones().then((z) => (timezones = z)).catch(() => {});
    }
  });
  /** The zone dropdown's selection — '' is "System default". Seeded from the stored
   *  setting once it arrives; a change to a *valid* value enables Apply. */
  let tzChoice = $state('');
  let tzSeeded = false;
  $effect(() => {
    if (settings && !tzSeeded) {
      tzSeeded = true;
      tzChoice = settings.displayTimezone ?? '';
    }
  });
  /** Only exactly a known zone (or blank) may be applied — the backend
   *  refuses garbage too, but a live Restart button over a half-typed name
   *  is an invitation to find that out the loud way. */
  const tzValid = $derived(tzChoice === '' || timezones.includes(tzChoice));
  const tzChanged = $derived(
    settings !== null && tzValid && tzChoice !== (settings.displayTimezone ?? ''));
  /** Applying restarts omacal — this is the one settings write with a
   *  deliberate confirmation step (the button), and the note is the last
   *  thing painted before the window goes. */
  let tzRestarting = $state(false);
  async function applyTimezone() {
    tzRestarting = true;
    try {
      await setDisplayTimezone(tzChoice === '' ? null : tzChoice);
    } catch (e) {
      tzRestarting = false;
      note = { text: String(e), kind: 'error' };
    }
  }

  /** The second-zone dropdown — the display zone's selection pattern over the same
   *  fetched list; '' is "Off". Its Apply carries no restart: nothing
   *  process-level captures this zone, so the settings that come back are
   *  already in force, and `onsettingschange` starts the second clock
   *  drawing behind the modal in the same breath. */
  let z2Choice = $state('');
  let z2Seeded = false;
  $effect(() => {
    if (settings && !z2Seeded) {
      z2Seeded = true;
      z2Choice = settings.secondTimezone ?? '';
    }
  });
  const z2Valid = $derived(z2Choice === '' || timezones.includes(z2Choice));
  const z2Changed = $derived(
    settings !== null && z2Valid && z2Choice !== (settings.secondTimezone ?? ''));
  async function applySecondZone() {
    try {
      settings = await setSecondTimezone(z2Choice === '' ? null : z2Choice);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  /**
   * One choice, and for the rolling rows two writes — **in this order**.
   *
   * The day count goes first because it is invisible while the week is a
   * whole one: if that write lands and the second fails, the user sees
   * exactly what they saw before, plus the error. The other order would
   * leave Week rolling with the *old* count — a view they did not ask for
   * and no message explaining it.
   *
   * A whole-week row is one write: `set_week_start` clears the rolling flag
   * itself, so there is no second call to half-apply.
   */
  async function saveWeekView(id: string) {
    const choice = WEEK_VIEWS.find((w) => w.id === id);
    if (!choice) return;
    try {
      if (choice.days !== undefined) {
        await setWeekViewDays(choice.days);
        settings = await setWeekStartsToday(true);
      } else {
        settings = await setWeekStart(choice.id as WeekStartDay);
      }
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  /** Stores which view OmaCal opens on next — `defaultCalendarId`'s shape,
   *  not `saveTimeFormat`'s: the view currently on screen is whatever the
   *  user navigated to, and this setting has no opinion about that.
   *
   *  One control, two backend writes — `saveWeekView`'s own shape: "Last
   *  view" is `defaultViewFollowsLast` turned on, and every other row is a
   *  fixed `defaultView` (which itself turns that flag back off, atomically,
   *  on the backend). */
  async function saveDefaultView(choice: DefaultViewChoice) {
    try {
      settings = choice === 'last'
        ? await setDefaultViewFollowsLast(true)
        : await setDefaultView(choice);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  async function saveDateFormat(format: DateFormat) {
    try { settings = await setDateFormatPreference(format); onsettingschange?.(settings); }
    catch (e) { note = { text: String(e), kind: 'error' }; }
  }

  async function saveTimeFormat(format: TimeFormat) {
    try {
      settings = await setTimeFormat(format);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  /** Range movement previews without an IPC round trip. Its `change` event
   *  persists once the pointer/key interaction commits; shape choices persist
   *  immediately because each click is already one complete choice. */
  function previewAppearance(next: Partial<AppearancePreferences>) {
    if (!settings) return;
    settings = { ...settings, ...next };
    applyAppearance(settings);
    onappearancechange?.(settings);
  }

  let appearanceWrite = 0;
  /** IPC commands are async and two fast slider/shape commits can overlap.
   *  Queue them so an older tuple can never finish after a newer one and
   *  become the database's final value. `appearanceWrite` separately keeps an
   *  older response from repainting while its newer request waits in line. */
  let appearanceQueue: Promise<void> = Promise.resolve();
  function saveAppearancePreferences() {
    if (!settings) return;
    const requested = {
      backgroundTransparency: settings.backgroundTransparency,
      inactiveBackgroundTransparency: settings.inactiveBackgroundTransparency,
      eventTransparency: settings.eventTransparency,
      eventCornerStyle: settings.eventCornerStyle,
    };
    const write = ++appearanceWrite;
    note = null;
    appearanceQueue = appearanceQueue.then(async () => {
      try {
        const stored = await setAppearancePreferences(
          requested.backgroundTransparency,
          requested.eventTransparency,
          requested.eventCornerStyle,
          requested.inactiveBackgroundTransparency,
        );
        if (write !== appearanceWrite) return;
        settings = stored;
        applyAppearance(stored);
        onappearancechange?.(stored);
        onsettingschange?.(stored);
      } catch (e) {
        if (write !== appearanceWrite) return;
        note = { text: String(e), kind: 'error' };
        // The preview has already painted. Read the authoritative values back
        // so a refused/failed write does not leave an unsaved look on screen.
        try {
          const stored = await getSettings();
          if (write !== appearanceWrite) return;
          settings = stored;
          applyAppearance(stored);
          onappearancechange?.(stored);
        } catch { /* keep the useful error from the write itself */ }
      }
    });
  }

  function chooseEventCorners(style: EventCornerStyle) {
    previewAppearance({ eventCornerStyle: style });
    void saveAppearancePreferences();
  }

  /**
   * Saves the interval and shows whatever comes back.
   *
   * Spec §3: the floor still applies and the UI says so rather than silently
   * clamping — but **the refusal is `set_sync_interval`'s, not this form's**,
   * and that is a decision the mutation sweep forced. A duplicate check here
   * refused with its own wording, which meant no test could tell which of the
   * two guards had fired: deleting the form's changed nothing anybody could
   * observe, which is the definition of a rule that is not being tested. One
   * authority, one message, and the form's job is to put it on screen.
   *
   * The integer check stays, because it is not a duplicate: the box is a
   * string, and "abc" never reaches a command that could refuse it.
   */
  async function saveInterval() {
    const minutes = Number(intervalText);
    if (!Number.isFinite(minutes) || !Number.isInteger(minutes)) {
      intervalNote = { text: 'Enter a whole number of minutes.', kind: 'error' };
      return;
    }
    intervalNote = null;
    try {
      // The answer is deliberately **not** kept. Nothing on screen is derived
      // from it — the box holds what was typed and the floor does not move —
      // so assigning it changed nothing observable, which the sweep proved by
      // deleting the assignment and reddening no test at all. What the save
      // has to guarantee is that the value was *stored*, and the spec asserts
      // that by reopening the modal, which re-fetches.
      await setSyncInterval(msOfMinutes(minutes));
      intervalNote = { text: 'Saved.', kind: 'info' };
    } catch (e) {
      intervalNote = { text: String(e), kind: 'error' };
    }
  }

  /** Saves a new fallback list, keeping what the backend still holds when it
   *  refuses — the same repair `toggleNotifications` makes. */
  async function saveFallback(minutes: number[]) {
    note = null;
    try {
      settings = await setFallbackReminders(minutes);
      if (settings) onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      settings = settings ? { ...settings } : null;
    }
  }

  async function saveDefaultCalendar(id: number | null) {
    note = null;
    try {
      settings = await setDefaultCalendar(id);
      if (settings) onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      settings = settings ? { ...settings } : null;
    }
  }

  async function saveDefaultEventDuration() {
    const minutes = Number(durationText);
    if (!Number.isSafeInteger(minutes) || minutes < 1 || minutes > 0xffff_ffff) {
      durationNote = { text: 'Enter a positive whole number of minutes.', kind: 'error' };
      return;
    }

    durationNote = null;
    try {
      settings = await setDefaultEventDuration(minutes);
      durationText = String(settings.defaultEventDurationMinutes);
      onsettingschange?.(settings);
      durationNote = { text: 'Saved.', kind: 'info' };
    } catch (e) {
      durationNote = { text: String(e), kind: 'error' };
    }
  }

  /** What the unmade choice means, by name. The primary when there is one;
   *  the first writable otherwise — `offerableCalendarId`'s own order. */
  const primaryLabel = $derived.by(() => {
    const primary = calendars.find((c) => c.is_primary);
    const effective = primary ?? writableCalendars(calendars)[0];
    return effective ? `Your primary — ${effective.summary}` : 'Your primary calendar';
  });

  /** The colour the picker's dot wears: the calendar a create would actually
   *  land on — the stored choice through the same staleness guard the form
   *  uses, so the dot cannot promise a calendar a create cannot reach. */
  const defaultCalColor = $derived.by(() => {
    const id = offerableCalendarId(settings?.defaultCalendarId ?? null, calendars);
    return calendarColor(id, calendars) ?? 'var(--accent)';
  });

  /** Same repair as `toggleNotifications` when the backend refuses. */
  async function toggleWeather(on: boolean) {
    note = null;
    try {
      settings = await setWeatherEnabled(on);
      if (settings) onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      settings = settings ? { ...settings } : null;
    }
  }

  /** Same shape as `saveTimeFormat`: nothing to refuse, `TemperatureUnit` has
   *  no third variant a select could send. */
  async function saveTemperatureUnit(unit: TemperatureUnit) {
    try {
      settings = await setTemperatureUnit(unit);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  async function toggleNotifications(on: boolean) {
    note = null;
    try {
      settings = await setNotificationsEnabled(on);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      // The click already flipped the checkbox; put it back to what the
      // backend still holds, the same repair `CalendarPopover` makes.
      settings = settings ? { ...settings } : null;
    }
  }

  /** The accounts with their ids and providers — richer than the `accounts`
   *  prop (emails only), which stays as the fallback while this loads. */
  let accountRows = $state<Account[] | null>(null);
  /** The account whose Sign out is one click from happening. */
  let confirmingSignOut = $state<number | null>(null);
  let signingOut = $state(false);

  $effect(() => {
    listAccounts()
      .then((rows) => (accountRows = rows))
      .catch(() => (accountRows = null));
  });

  async function doSignOut(row: Account) {
    if (signingOut) return;
    note = null;
    signingOut = true;
    try {
      accountRows = await signOut(row.id);
      confirmingSignOut = null;
      oncalendarchange?.();
      note = {
        text:
          row.provider === 'caldav'
            ? `${row.email} signed out and its local data removed. To revoke the password itself, visit your provider (for iCloud: account.apple.com).`
            : `${row.email} signed out, its access revoked, and its local data removed.`,
        kind: 'info',
      };
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    } finally {
      signingOut = false;
    }
  }

  // The CalDAV connect form's own little state machine: which form is open
  // (none, iCloud-worded, or generic), its fields, and its in-flight flag —
  // separate from `busy`, which belongs to the Google flow.
  let caldavForm = $state<null | 'icloud' | 'caldav'>(null);
  let caldavUrl = $state('');
  let caldavEmail = $state('');
  let caldavUser = $state('');
  let caldavPassword = $state('');
  let caldavBusy = $state(false);

  function openCaldavForm(kind: 'icloud' | 'caldav') {
    note = null;
    caldavForm = kind;
    caldavUrl = '';
    caldavEmail = '';
    caldavUser = '';
    caldavPassword = '';
  }

  async function submitCaldav() {
    if (caldavForm === null || caldavBusy) return;
    note = null;
    caldavBusy = true;
    try {
      const email = await connectCaldav({
        kind: caldavForm,
        serverUrl: caldavForm === 'caldav' ? caldavUrl : undefined,
        email: caldavEmail,
        username: caldavForm === 'caldav' ? caldavUser : undefined,
        password: caldavPassword,
      });
      caldavForm = null;
      note = { text: `${email} connected — syncing…`, kind: 'info' };
      // The account's calendars are in the store already (connect wrote
      // them); the sync fills in their events and tasks.
      await invoke('sync_now').catch(() => {});
      oncalendarchange?.();
      note = { text: `${email} connected. Its calendars are in the list — pick which to show under Calendars.`, kind: 'info' };
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    } finally {
      caldavBusy = false;
    }
  }

  /** Beside the control, like the rest of this pane (2026-09-12, reported
   *  from macOS: "no Saved coming"). The save was real; the shared note only
   *  ever carried the error, so success looked identical to nothing. */
  let dateNote = $state<{ text: string; kind: 'info' | 'error' } | null>(null);
  async function toggleShowDate(on: boolean) {
    dateNote = { text: 'Applying…', kind: 'info' };
    try {
      settings = await setShowDate(on);
      dateNote = { text: 'Saved', kind: 'info' };
    } catch (e) {
      dateNote = { text: String(e), kind: 'error' };
      // Same checkbox repair as `toggleTrayIcon`.
      settings = settings ? { ...settings } : null;
    }
  }

  async function toggleTrayIcon(on: boolean) {
    note = null;
    try {
      settings = await setTrayIcon(on);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      // Same checkbox repair as `toggleNotifications`.
      settings = settings ? { ...settings } : null;
    }
  }

  /** `saveTimeFormat`'s shape: a closed set, so nothing to refuse, and the
   *  backend's answer replaces `settings`. The repaint is not this function's
   *  business — the backend emits `theme-changed` and `App`'s existing
   *  listener applies it, the same path an Omarchy theme switch takes. */
  async function saveAppearance(appearance: Appearance) {
    note = null;
    try {
      settings = await setAppearance(appearance);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  async function saveWindowFrame(frame: WindowFrame) {
    note = null;
    try {
      settings = await setWindowFrame(frame);
      onsettingschange?.(settings);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
    }
  }

  async function toggleQuitOnClose(on: boolean) {
    note = null;
    try {
      settings = await setQuitOnClose(on);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      // Same checkbox repair as `toggleNotifications`.
      settings = settings ? { ...settings } : null;
    }
  }

  async function saveStartOnLogin(mode: StartOnLogin) {
    note = null;
    try {
      settings = await setStartOnLogin(mode);
    } catch (e) {
      note = { text: String(e), kind: 'error' };
      // Same repair as `toggleNotifications`: re-assign so the select snaps
      // back to what is actually stored rather than keeping the value the
      // browser already painted.
      settings = settings ? { ...settings } : null;
    }
  }

  // Group controls by what they affect. Menu-bar controls live together;
  // calendar appearance stays separate from provider/account management.
  const TABS = ['General', 'Appearance', 'Menu bar', 'Calendars', 'Accounts', 'Notifications'] as const;
  type Tab = (typeof TABS)[number];

  let tab = $state<Tab>('General');

  let panelEl: HTMLDivElement | undefined = $state();
  let preferredHeight = $state<number | null>(null);
  $effect(() => {
    const panel = panelEl;
    if (!panel) return;
    // Every pane has the final dialog width, even while hidden/inert. Measure
    // natural contents so switching tabs cannot change the dialog's height.
    const measure = () => {
      const heights = Array.from(panel.querySelectorAll<HTMLElement>('.pane-content'), el => el.getBoundingClientRect().height);
      preferredHeight = Math.ceil(Math.max(0, ...heights) + (panel.querySelector('.tabs')?.getBoundingClientRect().height ?? 0) + (panel.querySelector('.version')?.getBoundingClientRect().height ?? 0) + 2);
    };
    const observer = new ResizeObserver(measure);
    panel.querySelectorAll('.pane-content, .tabs, .version').forEach(el => observer.observe(el));
    measure();
    return () => observer.disconnect();
  });

  onMount(() => {
    // The first tab, not the panel and not a close button. `role="dialog"` plus
    // `aria-modal` oblige focus to start inside, and the first tab is both
    // inside and the thing a keyboard user wants next — unlike `ConfirmPanel`,
    // where the safe end of a dialog with a write behind it is the one that
    // changes nothing. Nothing here writes on open.
    //
    // Found rather than bound, exactly as `ConfirmPanel` finds its cancel
    // button: `bind:this` inside an `{#each}` ends up holding the **last**
    // element it rendered, so a bound "first tab" would quietly be
    // Notifications.
    panelEl?.querySelector<HTMLButtonElement>('[role="tab"]')?.focus();
  });

  // Nothing opens over the settings modal, so it is always the topmost layer
  // while it exists. See `escapeCloses` for why this is a `window` listener.
  escapeCloses(() => true, () => onclose());
</script>

<!--
  **Not built on `ConfirmPanel`, and here is what genuinely differs.**

  It shares the parts that are cheap to share and were never the hard bit: a
  scrim, `role="dialog"` + `aria-modal`, Escape on `window`, focus moved inside
  on mount. It does not share the part that makes `ConfirmPanel` what it is —
  `placePopover` against an anchor rect. Every confirmation in this app sits
  *beside the thing it is about*: the block that was dropped, the chip that was
  clicked. Settings is about no particular thing, so there is no rect to sit
  beside, and the only way to reuse that component would be to fabricate an
  anchor that happens to centre it — a lie told to a function whose whole job is
  positioning.

  The other two differences are smaller and point the same way: a confirmation
  is a question with an answer row, and this has no actions at all because every
  control inside applies immediately; and a confirmation is one screen, while
  this is a tab list that has to keep its selection.

  What *is* worth noticing is that five components now write the same
  scrim-plus-Escape-plus-dialog preamble. That is a real duplication and the
  right time to extract it is once this modal has content — extracting against
  an empty shell would be guessing at what the fifth caller needs.
-->
<!-- A sibling of `.modal`, not a wrapper, so a click inside never reaches it.
     Spec §5: the modal does not close on a click inside itself. -->
<button class="scrim" aria-label="Close settings" onclick={onclose}></button>

<div
  class="modal"
  style:height={preferredHeight === null ? '80vh' : `${preferredHeight}px`}
  bind:this={panelEl}
  role="dialog"
  aria-modal="true"
  tabindex="-1"
  aria-label="Settings"
>
  <div class="tabs" role="tablist" aria-label="Settings sections">
    {#each TABS as t (t)}
      <button
        type="button"
        role="tab"
        aria-selected={tab === t}
        class:on={tab === t}
        onclick={() => (tab = t)}
      >{t}</button>
    {/each}
  </div>

  <div class="panes">
  {#each TABS as pane}
  <div class="body" class:inactive={pane !== tab} role="tabpanel" aria-label={pane} aria-hidden={pane !== tab} inert={pane !== tab}>
  <div class="pane-content">
    {#if pane === 'General'}
      <div class="row">
        <label class="lab" for="sync-interval">Sync every</label>
        <div class="inline">
          <input
            id="sync-interval"
            type="number"
            min={floorMinutes}
            step="1"
            bind:value={intervalText}
            oninput={() => (intervalNote = null)}
            onkeydown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                saveInterval();
              }
            }}
          />
          <span class="unit">minutes</span>
          <button
            type="button"
            aria-label="Save sync interval"
            onclick={saveInterval}
            disabled={!settings}
          >Save</button>
          {#if intervalNote}
            <span
              class="rownote"
              class:err={intervalNote.kind === 'error'}
              data-testid="interval-note"
            >{intervalNote.text}</span>
          {/if}
        </div>
      </div>
      <!-- Said, not enforced silently. A value accepted and then quietly
           changed is worse than one refused. -->
      <p class="hint">
        Not less than {floorMinutes} minute{floorMinutes === 1 ? '' : 's'} — Google's quota is
        finite, and a desktop app has no business polling faster.
      </p>

      <div class="row section-start">
        <label class="lab" for="default-cal">New events land on</label>
        <div class="inline">
          <span class="caldot" aria-hidden="true" style="background:{defaultCalColor}"></span>
          <select
            id="default-cal"
            disabled={!settings}
            value={settings?.defaultCalendarId ?? ''}
            onchange={(e) => {
              const v = (e.currentTarget as HTMLSelectElement).value;
              saveDefaultCalendar(v === '' ? null : Number(v));
            }}
          >
            <!-- Named, not alluded to: "your primary" is a fact the user
                 has to go look up, and the answer is one find() away. -->
            <option value="">{primaryLabel}</option>
            {#each writableCalendars(calendars) as c (c.id)}
              <option value={c.id} style="color: {c.color_hex ?? 'inherit'}">{c.summary}</option>
            {/each}
          </select>
        </div>
      </div>
      <p class="hint">
        Only calendars OmaCal can write to are offered; if the choice ever
        stops being writable, creates fall back to your primary.
      </p>

      <div class="row">
        <label class="lab" for="default-event-duration">Default meeting duration</label>
        <div class="inline">
          <input
            id="default-event-duration"
            type="number"
            min="1"
            step="1"
            disabled={!settings}
            bind:value={durationText}
            oninput={() => (durationNote = null)}
            onkeydown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                saveDefaultEventDuration();
              }
            }}
          />
          <span class="unit">minutes</span>
          <button
            type="button"
            aria-label="Save default meeting duration"
            disabled={!settings}
            onclick={saveDefaultEventDuration}
          >Save</button>
          {#if durationNote}
            <span
              class="rownote"
              class:err={durationNote.kind === 'error'}
              data-testid="duration-note"
            >{durationNote.text}</span>
          {/if}
        </div>
      </div>
      <p class="hint">
        Used when a new event has a start time but no end time selected yet.
        Dragging a range still uses the range you chose.
      </p>

      <div class="row section-start">
        <label class="lab" for="date-format">Show dates as</label>
        <div class="inline"><select id="date-format" disabled={!settings} value={settings?.dateFormat ?? 'locale'} onchange={(e) => saveDateFormat(e.currentTarget.value as DateFormat)}>
          {#each DATE_FORMATS as f}<option value={f.value}>{f.label}</option>{/each}
        </select></div>
      </div>
      <p class="hint">Used for displayed dates throughout OmaCal. Native date pickers follow your system’s format.</p>

      <div class="row">
        <label class="lab" for="time-format">Show times as</label>
        <div class="inline">
          <select
            id="time-format"
            disabled={!settings}
            value={settings?.timeFormat ?? '24h'}
            onchange={(e) =>
              saveTimeFormat((e.currentTarget as HTMLSelectElement).value as TimeFormat)}
          >
            <!-- Each option prints an actual time rather than naming the
                 convention, because "24-hour" is a word you have to translate
                 and `13:30` is the thing itself. Half past one on the sample
                 clock deliberately: it is the hour that reads differently in
                 the two formats, where 9am would not. -->
            {#each ['24h', '12h'] as const as f (f)}
              <option value={f}>{formatClock(SAMPLE_MS, f)}</option>
            {/each}
          </select>
        </div>
      </div>
      <p class="hint">
        Applies everywhere OmaCal prints a time, including the hour ruler down
        the side of Day and Week.
      </p>

      <div class="row section-start">
        <label class="lab" for="display-tz">Time zone</label>
        <div class="inline">
          <TimezoneSelect id="display-tz" label="Time zone" zones={timezones} bind:value={tzChoice}
            disabled={!settings || tzRestarting} emptyLabel="System default" />
          <button
            type="button"
            disabled={!tzChanged || tzRestarting}
            onclick={applyTimezone}
          >Apply &amp; restart</button>
          {#if tzRestarting}
            <span class="rownote" data-testid="tz-note">Restarting…</span>
          {/if}
        </div>
      </div>
      <p class="hint">
        Every time OmaCal shows reads in this zone — the grid, reminders, the
        bar widget's feed. Applying restarts OmaCal, which is what makes all
        of them agree.
      </p>

      <div class="row">
        <label class="lab" for="second-tz">Second time zone</label>
        <div class="inline">
          <TimezoneSelect id="second-tz" label="Second time zone" zones={timezones} bind:value={z2Choice}
            disabled={!settings} emptyLabel="Off" />
          <button
            type="button"
            disabled={!z2Changed}
            onclick={applySecondZone}
          >Apply</button>
        </div>
      </div>
      <p class="hint">
        A second clock beside the first — on the Week and Day hour ruler, and
        under the event form's times. Convenience only: events are still
        created and edited in the time zone above. Choose Off to turn it
        off; no restart either way.
      </p>

      <label class="check">
        <input
          type="checkbox"
          checked={settings?.quitOnClose ?? false}
          disabled={!settings}
          onchange={(e) => toggleQuitOnClose(e.currentTarget.checked)}
        />
        Closing the window quits OmaCal
      </label>
      <p class="hint">
        Off by default: closing hides the window and OmaCal keeps running, so
        reminders still arrive and the bar widget stays fed. Turn this on and
        a close ends the app — nothing will fire until you open it again.
      </p>

      <div class="row">
        <label class="lab" for="start-on-login">When you log in</label>
        <div class="inline">
          <select
            id="start-on-login"
            disabled={!settings}
            value={settings?.startOnLogin ?? 'open'}
            onchange={(e) =>
              saveStartOnLogin((e.currentTarget as HTMLSelectElement).value as StartOnLogin)}
          >
            {#each START_ON_LOGIN_OPTIONS as [mode, label] (mode)}
              <option value={mode}>{label}</option>
            {/each}
          </select>
        </div>
      </div>
      <p class="hint">
        Running in the background keeps reminders firing and the bar widget
        fed, without opening a window — open it any time from the widget, the
        tray, or your app launcher. Choosing not to start OmaCal means
        notifications arrive only once you have opened it yourself.
      </p>

    {:else if pane === 'Appearance'}
      <div class="row">
        <label class="lab" for="appearance">Theme</label>
        <div class="inline">
          <select
            id="appearance"
            disabled={!settings}
            value={settings?.appearance ?? 'auto'}
            onchange={(e) =>
              saveAppearance((e.currentTarget as HTMLSelectElement).value as Appearance)}
          >
            {#each APPEARANCE_OPTIONS as [id, label] (id)}
              <option value={id}>{label}</option>
            {/each}
          </select>
        </div>
      </div>
      <p class="hint">
        {#if settings?.desktop === 'omarchy'}Automatic follows your Omarchy theme as you switch.
        {:else}Choose Light or Dark for your preferred appearance.{/if}
        Light and Dark replace the whole palette, including the accent, without a restart.
      </p>

      <!-- Only where there is a choice: on macOS the backend reports none,
           because the overlay title bar *is* the frame. While settings are
           still loading the row shows disabled like its neighbours. -->
      {#if settings?.windowFrame !== null}
        <div class="row">
          <label class="lab" for="window-frame">Window frame</label>
          <div class="inline">
            <select
              id="window-frame"
              disabled={!settings}
              value={settings?.windowFrame ?? 'auto'}
              onchange={(e) =>
                saveWindowFrame((e.currentTarget as HTMLSelectElement).value as WindowFrame)}
            >
              {#each WINDOW_FRAME_OPTIONS as [id, label] (id)}
                <option value={id}>{label}</option>
              {/each}
            </select>
          </div>
        </div>
        <p class="hint">
          Hyprland tiles OmaCal and closes or moves it from the keyboard, so a
          title bar there only repeats the compositor and costs a bar's height
          of calendar. Following the desktop hides the frame there and shows
          it on any other desktop, where the frame is what you grab. Takes
          effect at once.
        </p>
      {/if}

      <div class="row">
        <label class="lab" for="default-view">Open OmaCal on</label>
        <div class="inline">
          <select
            id="default-view"
            disabled={!settings}
            value={settings?.defaultViewFollowsLast ? 'last' : (settings?.defaultView ?? 'week')}
            onchange={(e) =>
              saveDefaultView((e.currentTarget as HTMLSelectElement).value as DefaultViewChoice)}
          >
            {#each DEFAULT_VIEW_OPTIONS as [id, label] (id)}
              <option value={id}>{label}</option>
            {/each}
          </select>
        </div>
      </div>
      <p class="hint">
        Which of the five views OmaCal opens next time. Last view reopens
        wherever you left off instead of a fixed one.
      </p>

      <div class="row">
        <label class="lab" for="week-view">Week view</label>
        <div class="inline">
          <select
            id="week-view"
            disabled={!settings}
            value={weekViewChoice}
            onchange={(e) => saveWeekView((e.currentTarget as HTMLSelectElement).value)}
          >
            {#each WEEK_VIEWS as w (w.id)}
              <option value={w.id}>{w.label}</option>
            {/each}
          </select>
        </div>
      </div>
      {#if settings?.weekStartsToday}
        <p class="hint">
          Today is the first column, and the view rolls forward with it. Month,
          Year and Big Year keep their rows aligned to {DAY_LABEL[settings.weekStart]}
          — pick a whole week above to change that.
        </p>
      {:else}
        <p class="hint">
          Week, Month, Year and Big Year all start their rows on this day.
        </p>
      {/if}

      <!-- The canvas can only fade where the window can be seen through, and
           macOS's cannot (AppSettings.transparentWindow); a slider that moved
           nothing would read as broken, so there is no slider there. -->
      <section class="appearance-section" aria-labelledby="visible-hours-heading">
        <h2 id="visible-hours-heading">Visible hours</h2>
        <div class="visible-hours-controls">
          <label>Start time <select aria-label="Visible hours start time" value={settings?.visibleStartHour ?? 0} disabled={!settings} onchange={(e) => changeVisibleHours(Number(e.currentTarget.value), settings?.visibleEndHour ?? 24)}>
            {#each Array.from({ length: 24 }, (_, h) => h) as h}<option value={h} disabled={h >= (settings?.visibleEndHour ?? 24)}>{formatClock(new Date(2020, 0, 1, h).getTime(), settings?.timeFormat ?? '24h')}</option>{/each}
          </select></label>
          <label>End time <select aria-label="Visible hours end time" value={settings?.visibleEndHour ?? 24} disabled={!settings} onchange={(e) => changeVisibleHours(settings?.visibleStartHour ?? 0, Number(e.currentTarget.value))}>
            {#each Array.from({ length: 24 }, (_, h) => h + 1) as h}<option value={h} disabled={h <= (settings?.visibleStartHour ?? 0)}>{h === 24 ? 'Midnight (end of day)' : formatClock(new Date(2020, 0, 1, h).getTime(), settings?.timeFormat ?? '24h')}</option>{/each}
          </select></label>
        </div>
        <p class="hint">Hours shown in Day and Week. Events outside this range remain in the agenda and search.</p>
      </section>
      {#if settings?.transparentWindow ?? true}
      <section class="appearance-section" aria-labelledby="background-style-heading">
        <h2 id="background-style-heading">Calendar background</h2>
        <div class="range-row">
          <label for="background-transparency">Active window</label>
          <input
            id="background-transparency"
            aria-label="Active background transparency"
            type="range"
            min="0"
            max="50"
            step="0.1"
            value={settings?.backgroundTransparency ?? 0}
            disabled={!settings}
            oninput={(e) => previewAppearance({
              backgroundTransparency: e.currentTarget.valueAsNumber,
            })}
            onchange={() => { void saveAppearancePreferences(); }}
          />
          <output for="background-transparency">
            {settings?.backgroundTransparency ?? 0}%
          </output>
        </div>
        <div class="range-row">
          <label for="inactive-background-transparency">Inactive window</label>
          <input
            id="inactive-background-transparency"
            aria-label="Inactive background transparency"
            type="range"
            min="0"
            max="50"
            step="0.1"
            value={settings?.inactiveBackgroundTransparency ?? 0}
            disabled={!settings}
            oninput={(e) => previewAppearance({
              inactiveBackgroundTransparency: e.currentTarget.valueAsNumber,
            })}
            onchange={() => { void saveAppearancePreferences(); }}
          />
          <output for="inactive-background-transparency">
            {settings?.inactiveBackgroundTransparency ?? 0}%
          </output>
        </div>
        <p class="hint">
          0% is opaque; 50% is as clear as the canvas goes, in 0.1% steps.
          Inactive applies when another window has focus.
          {#if settings?.desktop === 'omarchy'}Omarchy blends every window a little on its own, so OmaCal starts at 4% to match, and the compositor's share stays on top.{/if}
        </p>
      </section>
      {/if}

      <section class="appearance-section" aria-labelledby="event-style-heading">
        <h2 id="event-style-heading">Event styling</h2>
        <!-- Offered only where the window can be seen through, like the
             canvas slider above (2026-09-12, reported from macOS as "not
             doing anything"). The fade goes toward `transparent`, and on
             the Day and Week grid the fill it fades is a 7% tint of the
             calendar colour over `--bg` — with an opaque `--bg` behind it,
             the slider's whole range turns 7% into 5.25%, which nobody can
             see. Only Big Year's full-colour fills ever showed it there,
             and a control that visibly does nothing on the surfaces people
             live in is worse than its absence. Corner shape stays: it is
             not about transparency. -->
        {#if settings?.transparentWindow ?? true}
        <div class="range-row">
          <label for="event-transparency">Transparency</label>
          <input
            id="event-transparency"
            aria-label="Event transparency"
            type="range"
            min="0"
            max="25"
            step="0.1"
            value={settings?.eventTransparency ?? 0}
            disabled={!settings}
            oninput={(e) => previewAppearance({
              eventTransparency: e.currentTarget.valueAsNumber,
            })}
            onchange={() => { void saveAppearancePreferences(); }}
          />
          <output for="event-transparency">
            {settings?.eventTransparency ?? 0}%
          </output>
        </div>
        <p class="hint">
          Only event fills fade; titles, colour spines, outlines and controls
          remain visible.
        </p>
        {/if}

        <fieldset class="shape" disabled={!settings}>
          <legend>Corner shape</legend>
          <label class:chosen={settings?.eventCornerStyle === 'rounded'}>
            <input
              type="radio"
              name="event-corner-style"
              value="rounded"
              checked={(settings?.eventCornerStyle ?? 'rounded') === 'rounded'}
              onchange={() => chooseEventCorners('rounded')}
            />
            <span class="event-sample rounded" aria-hidden="true"></span>
            Rounded
          </label>
          <label class:chosen={settings?.eventCornerStyle === 'square'}>
            <input
              type="radio"
              name="event-corner-style"
              value="square"
              checked={settings?.eventCornerStyle === 'square'}
              onchange={() => chooseEventCorners('square')}
            />
            <span class="event-sample square" aria-hidden="true"></span>
            Square
          </label>
        </fieldset>
      </section>

      <label class="check section-start">
        <input
          type="checkbox"
          checked={settings?.weatherEnabled ?? true}
          disabled={!settings}
          onchange={(e) => toggleWeather(e.currentTarget.checked)}
        />
        Weather in the day headers
      </label>
      <p class="hint">
        A small forecast icon and the day's high, from Open-Meteo.
        {#if settings?.desktop === 'omarchy'}The location uses your Omarchy weather widget setting when available, otherwise your IP address.
        {:else}The location comes from your IP address.{/if}
        Turning this off stops forecast requests.
      </p>

      {#if settings?.weatherEnabled}
        <div class="row">
          <label class="lab" for="temperature-unit">Show temperature as</label>
          <div class="inline">
            <select
              id="temperature-unit"
              disabled={!settings}
              value={settings?.temperatureUnit ?? 'celsius'}
              onchange={(e) =>
                saveTemperatureUnit(
                  (e.currentTarget as HTMLSelectElement).value as TemperatureUnit,
                )}
            >
              <!-- Each option prints a temperature rather than naming the
                   scale, `time-format`'s reason: "Celsius" is a word you have
                   to translate and `22°C` is the thing itself. -->
              <option value="celsius">22°C</option>
              <option value="fahrenheit">72°F</option>
            </select>
          </div>
        </div>
        <p class="hint">
          The day headers stay a bare number, same as always — this only
          decides which scale it's read in.
        </p>
      {/if}

    {:else if pane === 'Menu bar'}
      <label class="check section-start">
        <input
          type="checkbox"
          checked={settings?.trayIcon ?? true}
          disabled={!settings}
          onchange={(e) => toggleTrayIcon(e.currentTarget.checked)}
        />
        Show the tray icon
      </label>
      <!-- What the switch takes with it differs by desktop, and the hint says
           which — because on macOS the icon *is* the agenda's only door,
           while on Omarchy the bar widget is a separate thing that stays
           (v3.3.0–v3.5.0 hid it too, which the widget's README calls a
           mistake). A single sentence for all three would be wrong on two
           of them. -->
      <p class="hint" data-testid="tray-icon-hint">
        {#if settings?.desktop === 'macos'}
          On macOS this is the menu-bar item itself: the agenda popup, the
          meeting label and the Join button all go with it. The app window
          and its shortcuts stay.
        {:else if settings?.desktop === 'omarchy'}
          The bar widget is separate and stays — the tray icon is the
          redundant one once the widget is there. Add or remove the widget
          itself in the bar's own settings.
        {:else}
          The tray provides Quit and Sync. Keep another way to access these
          actions available when hiding it.
        {/if}
      </p>

      <div class="inline">
        <label class="check">
          <input
            type="checkbox"
            checked={settings?.showDate ?? false}
            disabled={!settings}
            onchange={(e) => toggleShowDate(e.currentTarget.checked)}
          />
          Show today's date
        </label>
        {#if dateNote}
          <span class="rownote" class:err={dateNote.kind === 'error'} data-testid="show-date-note">{dateNote.text}</span>
        {/if}
      </div>
      <p class="hint">Show the date beside the menu-bar icon. Existing day-only displays use Custom (%-d).</p>
      {#if settings?.showDate}
      <div class="row">
        <label class="lab" for="menubar-date-format">Menu-bar date format</label>
        <select id="menubar-date-format" disabled={!settings} value={settings?.menubarDateFormat ?? 'general'}
          onchange={e => saveMenuDate(e.currentTarget.value as AppSettings['menubarDateFormat'])}>
          <option value="general">Follow General date format</option>
          {#each DATE_FORMATS as f}<option value={f.value}>{f.label}</option>{/each}
          <option value="custom">Custom</option>
        </select>
      </div>
      {#if settings?.menubarDateFormat === 'custom'}
        <div class="row">
          <label class="lab" for="menubar-date-custom">Custom date format</label>
          <div class="inline">
            <input id="menubar-date-custom" type="text" maxlength="128" bind:value={menuDateCustom} />
            <button type="button" onclick={() => saveMenuDate('custom', menuDateCustom)}>Save</button>
          </div>
        </div>
        <p class="hint">%-d → 7 · %d → 07 · %b %-d → Sep 7.
          <a href="https://docs.rs/jiff/latest/jiff/fmt/strtime/index.html#conversion-specifications"
            onclick={e => { e.preventDefault(); void invoke('open_date_format_guide').catch(e => { menuDateNote = String(e); }); }}>Formatting guide</a>
        </p>
      {/if}
      {#if menuDateNote}<p class="note err" role="alert">{menuDateNote}</p>{/if}
      {/if}
      <section class="appearance-section" aria-labelledby="menubar-heading">
        <h2 id="menubar-heading">Menu bar calendar</h2>
        <div class="inline">
          <label class="check"><input type="checkbox" disabled={!settings || menubarBusy}
            checked={settings?.menubarLabel ?? true}
            onchange={(e) => saveMenubar(e.currentTarget.checked, undefined, 'label')} />
            Show meeting title and countdown</label>
          {#if labelNote}
            <span class="rownote" class:err={labelNote.kind === 'error'}
                  data-testid="menubar-label-note">{labelNote.text}</span>
          {/if}
        </div>
        <label class="lab" for="meeting-format">Meeting label format</label>
        <input id="meeting-format" class="format-template" type="text" maxlength="256"
               bind:value={meetingFormat} disabled={!settings || menubarBusy} />
        <div class="inline">
          <!-- Disabled when there is nothing to save, which is the button
               saying so. Reset likewise: at the default it would write the
               value already stored. -->
          <button type="button" disabled={!settings || menubarBusy || !formatDirty}
                  onclick={() => saveMeetingFormat()}>Save format</button>
          <button type="button" disabled={!settings || menubarBusy
                    || (meetingFormat === DEFAULT_MEETING_FORMAT
                        && settings?.menubarLabelFormat === DEFAULT_MEETING_FORMAT)}
                  onclick={() => saveMeetingFormat(DEFAULT_MEETING_FORMAT)}>Reset format</button>
          {#if formatDirty}
            <span class="rownote unsaved" data-testid="menubar-format-dirty">Unsaved</span>
          {:else if formatNote}
            <span class="rownote" class:err={formatNote.kind === 'error'}
                  data-testid="menubar-format-note">{formatNote.text}</span>
          {/if}
        </div>
        <p class="hint">Reorder or omit placeholders: {'{title}'}, {'{time}'}, {'{end_time}'}, {'{countdown}'}, {'{calendar}'}. Add your own separators.</p>
        <p class="hint" aria-label="Meeting label preview">Preview: {meetingLabel(meetingFormat, {
          title: 'Design sync', time: formatClock(SAMPLE_MS, settings?.timeFormat ?? '24h'),
          end_time: formatClock(SAMPLE_MS + 30 * 60000, settings?.timeFormat ?? '24h'), countdown: 'in 5m', calendar: 'Work'
        })}</p>
        {#if formatNote?.kind === 'error'}<p class="note err" role="alert">{formatNote.text}</p>{/if}
        <label class="lab" for="menubar-join">Show Join before a meeting</label>
        <div class="inline">
          <select id="menubar-join" disabled={!settings || menubarBusy} value={settings?.menubarJoinMinutes ?? 5}
            onchange={(e) => saveMenubar(undefined, Number(e.currentTarget.value), 'join')}>
            {#each [0, 1, 5, 10, 15, 30, 60] as minutes}
              <option value={minutes}>{minutes === 0 ? 'At start time' : `${minutes} minutes before`}</option>
            {/each}
          </select>
          {#if joinNote}
            <span class="rownote" class:err={joinNote.kind === 'error'}
                  data-testid="menubar-join-note">{joinNote.text}</span>
          {/if}
        </div>
        <p class="hint">Join stays available while the meeting is running. These preferences apply to the {settings?.desktop === 'macos' ? 'macOS menu bar' : settings?.desktop === 'omarchy' ? 'Omarchy widget' : 'menu bar'}.</p>
      </section>
    {:else if pane === 'Calendars'}
      <!-- **The same rows the header's popover shows, from the same
           component.** Extracted rather than reimplemented, which is what
           makes "rehomed, not rewritten" checkable: `CalendarPopover`'s own
           specs pass unchanged, because what they assert is `CalendarList`
           now.

           Per-calendar colour lands here next, on the row — which is why the
           row is a component with its own file rather than markup written out
           twice in two hosts. -->
      {#if calendars.length > 0}
        <div class="cals"><CalendarList {calendars} spacious onchange={oncalendarchange} /></div>
      {:else}
        <p class="soon">No calendars yet. Connect an account first.</p>
      {/if}

    {:else if pane === 'Accounts'}
      <ul class="accounts">
        {#each accountRows ?? accounts.map((email, i) => ({ id: -1 - i, email, provider: 'google' })) as row (row.id)}
          <li class="account-row">
            <span class="acct-email">{row.email}</span>
            <span class="acct-prov">{row.provider === 'caldav' ? 'CalDAV' : 'Google'}</span>
            {#if row.id >= 0}
              {#if confirmingSignOut === row.id}
                <button
                  type="button"
                  class="danger"
                  disabled={signingOut}
                  onclick={() => doSignOut(row)}
                >{signingOut ? 'Signing out…' : 'Really sign out'}</button>
                <button type="button" disabled={signingOut} onclick={() => (confirmingSignOut = null)}>
                  Keep
                </button>
              {:else}
                <button type="button" onclick={() => (confirmingSignOut = row.id)}>Sign out…</button>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
      {#if (accountRows ?? accounts).length === 0}
        <p class="soon">No account is connected.</p>
      {/if}
      <div class="provider-row">
        <button type="button" onclick={onSignIn} disabled={busy}>Add Google account</button>
        <button type="button" onclick={() => openCaldavForm('icloud')} disabled={busy || caldavBusy}>Add iCloud account</button>
        <button type="button" onclick={() => openCaldavForm('caldav')} disabled={busy || caldavBusy}>Add CalDAV account</button>
        {#if signingIn}
          <button type="button" onclick={onCancelSignIn}>Cancel sign-in</button>
        {/if}
      </div>
      {#if signingIn}<p class="hint" role="status">Waiting for Google sign-in in your browser.</p>{/if}
      <p class="hint">
        Signing out removes the account's local data (its calendars, events
        and tasks re-sync if you connect again). For Google, the app's access
        is revoked too; for iCloud and CalDAV, the password stays valid until
        you revoke it at your provider.
      </p>

      <!-- CalDAV: the auth story with no OAuth in it. One form serves both —
           "iCloud" only fixes the server address and words the fields. -->
      <div class="caldav" class:empty={caldavForm === null} role="group" aria-label="Connect a CalDAV account">
        {#if caldavForm !== null}
          <form
            class="caldav-form"
            onsubmit={(e) => {
              e.preventDefault();
              void submitCaldav();
            }}
          >
            {#if caldavForm === 'caldav'}
              <input
                type="url"
                placeholder="https://dav.example.com/"
                aria-label="Server address"
                bind:value={caldavUrl}
                disabled={caldavBusy}
              />
            {/if}
            <!-- `text`, not `email`, for a plain CalDAV server: the backend
                 asks for "an email (or account name)" and a self-hosted
                 Radicale account is normally a bare username, which the
                 browser's own email validation would refuse to submit
                 (issue #28). An Apple ID is always an address, so iCloud
                 keeps the check. -->
            <input
              type={caldavForm === 'icloud' ? 'email' : 'text'}
              placeholder={caldavForm === 'icloud' ? 'Apple ID' : 'Email or account name'}
              aria-label={caldavForm === 'icloud' ? 'Apple ID' : 'Email or account name'}
              bind:value={caldavEmail}
              disabled={caldavBusy}
            />
            {#if caldavForm === 'caldav'}
              <input
                type="text"
                placeholder="Username (when it differs from the email)"
                aria-label="Username"
                bind:value={caldavUser}
                disabled={caldavBusy}
              />
            {/if}
            <input
              type="password"
              placeholder={caldavForm === 'icloud' ? 'App-specific password' : 'Password'}
              aria-label="Password"
              bind:value={caldavPassword}
              disabled={caldavBusy}
            />
            {#if caldavForm === 'icloud'}
              <p class="hint">
                Not your Apple ID password: create an app-specific password at
                appleid.apple.com → Sign-In and Security.
              </p>
            {/if}
            <!-- Said before the attempt rather than after it: the same rule
                 the transport enforces, at the moment the address is typed. -->
            {#if caldavForm === 'caldav' && /^\s*http:\/\//i.test(caldavUrl)}
              <p class="hint">
                Plain http is accepted only for a server on this machine or
                your own network — the password travels unencrypted, so a
                server reachable from the internet needs https.
              </p>
            {/if}
            <div class="provider-row">
              <button type="submit" disabled={caldavBusy}>
                {caldavBusy ? 'Connecting…' : 'Connect'}
              </button>
              <button type="button" onclick={() => (caldavForm = null)} disabled={caldavBusy}>
                Cancel
              </button>
            </div>
          </form>
        {/if}
      </div>

    {:else}
      <label class="check">
        <input
          type="checkbox"
          checked={settings?.notificationsEnabled ?? true}
          disabled={!settings}
          onchange={(e) => toggleNotifications(e.currentTarget.checked)}
        />
        Show reminders
      </label>
      <!-- What fires is still each event's own Google reminders — with one
           addition this tab owns (fallback spec §1): when a timed event
           follows its calendar's defaults and the calendar has none, the rows
           below apply. That is exactly the shape of a shared calendar
           received from someone else, where this account sees no reminders
           at all and every meeting was silent. -->
      <div class="fallback" role="group" aria-label="Fallback reminders">
        <p class="hint">
          When an event has no reminders of its own and its calendar offers no
          defaults, notify:
        </p>
        {#each settings?.fallbackReminderMinutes ?? [] as m, i}
          <div class="frow">
            <span>Notify me</span>
            <input
              type="number"
              min="0"
              max={reminderMax(reminderUnitOf(m))}
              aria-label="Fallback amount"
              value={reminderAmountOf(m)}
              disabled={!settings}
              onchange={(e) => {
                const n = (e.currentTarget as HTMLInputElement).valueAsNumber;
                if (!Number.isFinite(n) || n < 0 || !settings) return;
                const next = [...settings.fallbackReminderMinutes];
                next[i] = Math.round(n) * REMINDER_UNITS[reminderUnitOf(m)];
                saveFallback(next);
              }}
            />
            <select
              aria-label="Fallback unit"
              value={reminderUnitOf(m)}
              disabled={!settings}
              onchange={(e) => {
                if (!settings) return;
                const unit = (e.currentTarget as HTMLSelectElement).value;
                const next = [...settings.fallbackReminderMinutes];
                next[i] = reminderAmountOf(m) * REMINDER_UNITS[unit];
                saveFallback(next);
              }}
            >
              <option value="minutes">minutes</option>
              <option value="hours">hours</option>
              <option value="days">days</option>
              <option value="weeks">weeks</option>
            </select>
            <span>before</span>
            <button
              type="button"
              class="unremind"
              aria-label="Remove fallback reminder"
              disabled={!settings}
              onclick={() => settings && saveFallback(settings.fallbackReminderMinutes.filter((_, j) => j !== i))}
            >×</button>
          </div>
        {/each}
        {#if (settings?.fallbackReminderMinutes.length ?? 5) < 5}
          <button
            type="button"
            class="remind"
            disabled={!settings}
            onclick={() => settings && saveFallback([...settings.fallbackReminderMinutes, 15])}
          >+ Add notification</button>
        {/if}
        <p class="hint">
          Timed events only, and never over an event's or calendar's own
          reminders — clear the list to turn this off.
        </p>
      </div>
    {/if}

    {#if note && pane === tab}
      <p class="note" class:err={note.kind === 'error'} data-testid="settings-note">{note.text}</p>
    {/if}

  </div>
  </div>
  {/each}
  </div>
  {#if version}<p class="version" data-testid="app-version">OmaCal {version}</p>{/if}
</div>

<style>
  .visible-hours-controls { display: flex; flex-wrap: wrap; gap: 12px; }
  .visible-hours-controls label { display: flex; flex-direction: column; gap: 8px; }
  .format-template { box-sizing: border-box; width: 100%; min-width: 0; }
  .hint a { color: var(--accent); }
  .appearance-section { align-self: stretch; display: flex; flex-direction: column;
                        gap: 14px; padding: 8px 0 16px; margin-top: 16px; }
  .appearance-section + .appearance-section {
    border-top: 1px solid var(--hairline); padding-top: 24px;
  }
  .appearance-section h2 { margin: 0; font-size: 13px; font-weight: 600;
                           color: var(--text); }
  .range-row { align-self: stretch; display: grid;
               grid-template-columns: 82px minmax(120px, 1fr) 42px;
               align-items: center; gap: 10px; }
  .range-row label { font-size: 12.5px; color: var(--text); }
  .range-row input[type='range'] { width: 100%; margin: 0; accent-color: var(--accent);
                                   cursor: pointer; }
  .range-row input[type='range']:disabled { cursor: default; opacity: .5; }
  .range-row output { color: var(--muted); font-size: 12px; text-align: right;
                      font-variant-numeric: tabular-nums; }

  .shape { align-self: stretch; border: 0; padding: 0; margin: 2px 0 0;
           display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
  .shape legend { grid-column: 1 / -1; padding: 0; margin: 0 0 6px;
                  font-size: 10.5px; color: var(--muted); letter-spacing: .05em; }
  .shape label { display: flex; align-items: center; gap: 8px; cursor: pointer;
                 border: 1px solid var(--hairline); border-radius: 7px;
                 padding: 8px 10px; color: var(--muted); }
  .shape label.chosen { border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
                        color: var(--text);
                        background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .shape:disabled label { cursor: default; opacity: .5; }
  .event-sample { width: 34px; height: 16px; flex: none;
                  background: color-mix(in srgb, var(--accent) var(--event-fill-opacity, 100%), transparent);
                  box-shadow: inset 2px 0 0 var(--accent); }
  .event-sample.rounded { border-radius: 6px; }
  .event-sample.square { border-radius: 0; }

  /* A colophon, not a control: the quietest text in the modal, at the very
     bottom, on every tab — where "what version am I on?" goes looking. */
  .version { flex: none; margin: 0; padding: 16px 24px 20px; font-size: 11.5px; color: var(--muted);
             text-align: left; }
  .fallback { display: flex; flex-direction: column; gap: 20px; margin-top: 0; align-items: flex-start; }
  .frow { display: flex; align-items: center; gap: 5px; font-size: 13px; }
  .frow input[type='number'] { width: 56px; }
  .unremind { font: inherit; font-size: 14px; color: var(--muted); cursor: pointer;
              background: none; border: 0; padding: 0 2px; }
  .unremind:hover { color: var(--text); }
  .remind { font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
            background: none; border: 1px solid var(--hairline); border-radius: 5px;
            padding: 2px 7px; }

  .scrim { position: fixed; inset: 0;  background: rgba(0, 0, 0, .35);
           border: 0; cursor: default; z-index: 60; }

  /* Centred rather than anchored — see the comment above the markup. `fixed`
     plus a translate keeps it centred without knowing its own size, which is
     what lets the body grow as the tabs are filled in. */
  .modal { box-sizing: border-box; position: fixed; z-index: 61; top: 50%; left: 50%;
           transform: translate(-50%, -50%);
           width: 650px; max-width: calc(100vw - 32px);
           max-height: min(80vh, calc(100vh - 64px));
           display: flex; flex-direction: column;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 10px; box-shadow: 0 12px 40px rgba(0, 0, 0, .5);
           font-size: 13px; color: var(--text); overflow: hidden; }
  .modal:focus { outline: none; }

  .tabs { display: flex; flex-wrap: wrap; gap: 2px; padding: 8px 8px 0;
          border-bottom: 1px solid var(--hairline); flex: none; }
  .tabs button { font: inherit; font-size: 12.5px; color: var(--muted); cursor: pointer;
                 background: none; border: 0; border-radius: 6px 6px 0 0;
                 padding: 6px 12px; }
  .tabs button.on { color: var(--text);
                    background: color-mix(in srgb, var(--text) 7%, transparent); }
  .tabs button:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }

  .panes { flex: 1; min-height: 0; position: relative; }
  .body { position: absolute; inset: 0; overflow-y: auto; }
  .body.inactive { visibility: hidden; pointer-events: none; }
  .pane-content { padding: 24px; display: flex; flex-direction: column; gap: 12px; align-items: flex-start; }
  .pane-content > * { flex-shrink: 0; }
  .soon { font-size: 12px; color: var(--muted); margin: 0; }

  .row { display: flex; flex-direction: column; gap: 8px; }
  .pane-content > .hint + .row, .pane-content > .hint + .check { margin-top: 8px; }
  .pane-content > .section-start, .pane-content > .hint + .section-start { margin-top: 24px; }
  .pane-content > .row, .pane-content > .check, .pane-content > .hint, .appearance-section { flex-shrink: 0; }
  .lab { font-size: 10.5px; color: var(--muted); letter-spacing: .05em; }
  .inline { display: flex; align-items: center; gap: 6px; }
  .unit { font-size: 12px; color: var(--muted); }
  /* No max-width: the 40ch cap that used to sit here wrapped every hint at
     about half the modal and left the right side conspicuously empty
     (reported 2026-08-17). Hints follow the responsive modal width. */
  .hint { font-size: 11px; color: var(--muted); opacity: .85; line-height: 1.45; margin: 0;
          align-self: stretch; }

  /* Text boxes too, not only number boxes: the two time-zone fields were the
     one kind of input this rule missed, and they rendered as the platform's
     own widget — bigger type, a different border — beside styled neighbours
     (reported 2026-09-02). */
  input[type='number'], input[type='text'], select {
    font: inherit; font-size: 13px; color: var(--text);
    background-color: color-mix(in srgb, var(--text) 5%, transparent);
    border: 1px solid var(--hairline); border-radius: 5px; padding: 4px 6px;
  }
  /* Scoped selectors outweigh app.css, so the shorthand above just undid the
     global chevron clearance — the text ran under the arrow. Restated here;
     any component that restyles a select's padding owes the right side 22px. */
  select { padding-right: 22px; }
  input[type='number'] { width: 72px; }
  input:focus, select:focus { outline: 1px solid var(--accent); outline-offset: -1px; }

  .caldot { width: 10px; height: 10px; border-radius: 3px; flex: none; }

  .check { display: flex; align-items: center; gap: 7px; font-size: 12.5px; cursor: pointer; }

  .account-row { display: flex; align-items: center; gap: 12px; padding: 6px 0; }
  .acct-email { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .acct-prov { color: var(--muted); font-size: 11px; letter-spacing: 0.04em;
    text-transform: uppercase; }
  .account-row .danger { color: var(--danger, #e66); border-color: var(--danger, #e66); }

  .caldav { align-self: stretch; min-width: 0; display: flex; flex-direction: column; gap: 16px; margin-top: 24px; }
  .caldav.empty { display: none; }
  .provider-row { display: flex; flex-wrap: wrap; gap: 8px; }
  .caldav-form { display: flex; flex-direction: column; gap: 6px; }
  .caldav-form input { box-sizing: border-box; width: 100%; min-width: 0; font: inherit; font-size: 12.5px; color: var(--text);
    background: var(--bg); border: 1px solid var(--hairline); border-radius: 5px;
    padding: 5px 8px; }
  .caldav-form input:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }

  .accounts { list-style: none; margin: 0; padding: 0; display: flex;
              flex-direction: column; gap: 3px; font-size: 12.5px; }

  /* Full width, unlike the other tabs' left-aligned controls: this is a list
     of rows whose right-hand Add/Remove buttons have to line up. */
  .cals { align-self: stretch; }

  .body button { font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
                 background: color-mix(in srgb, var(--text) 6%, transparent);
                 border: 0; border-radius: 6px; padding: 4px 10px; }
  .body button:disabled { opacity: .5; cursor: default; }

  /* Beside the Save button, where the eye already is when it lands. The
     accent for success, so "Saved." reads as a state change and not as one
     more piece of muted hint text. */
  .rownote { font-size: 11.5px; color: var(--accent); white-space: nowrap; }
  /* Not the accent: "Unsaved" is a state to notice, not an outcome to
     celebrate, and it sits beside a Save button that is lit for the same
     reason. */
  .rownote.unsaved { color: var(--muted); font-style: italic; }
  .rownote.err { color: var(--error); white-space: normal; }

  .note { font-size: 11.5px; color: var(--muted); line-height: 1.4; margin: 0;
          padding: 6px 8px; border-radius: 5px;
          background: color-mix(in srgb, var(--text) 6%, transparent); }
  .note.err { color: var(--error); background: color-mix(in srgb, var(--error) 9%, transparent); }
</style>
