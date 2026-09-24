import type { DateFormat } from './datefmt';
import { invoke } from '@tauri-apps/api/core';

import type { TemperatureUnit } from './temperature';
import type { TimeFormat } from './timefmt';
import type { WeekStartDay } from './weekstart';
import type { EventCornerStyle } from './appearance';
import type { View } from './views';

/** A `View`, plus `'last'` — "wherever the switcher was most recently,"
 *  which is `defaultViewFollowsLast`, not a sixth `View` value: `View` also
 *  names what `lastView` holds, and that can never be "last". */
export type DefaultViewChoice = View | 'last';

/** The five switcher slots, in the order the select offers them —
 *  `ViewSwitcher`'s own `SLOTS` labels, so the settings row and the switcher
 *  itself cannot describe the same view two different ways — plus "Last
 *  view", the row this modal derives rather than stores directly; see
 *  `SettingsModal`'s `saveDefaultView`. */
export const DEFAULT_VIEW_OPTIONS: ReadonlyArray<[DefaultViewChoice, string]> = [
  ['day', 'Day'],
  ['week', 'Week'],
  ['month', 'Month'],
  ['year', 'Year'],
  ['bigyear', 'Big Year'],
  ['last', 'Last view'],
];

/** Total columns in the rolling Week view, including today. */
export type WeekViewDays = 3 | 5 | 7;

/**
 * What a login does about omacal, in the exact spellings
 * `settings::StartOnLogin` stores and serialises.
 *
 * `'background'` starts omacal without a window: reminders fire and the bar
 * widget's feed stays current, and nothing appears on screen. It is a way of
 * starting, not a way of not starting — the distinction the `'off'` row makes.
 */
export type StartOnLogin = 'off' | 'open' | 'background';

/** The three options in the order the select offers them: least to most
 *  running. Labels here rather than in the markup so the spec that drives the
 *  select and the component that renders it read one list. */
export const START_ON_LOGIN_OPTIONS: ReadonlyArray<[StartOnLogin, string]> = [
  ['off', "Don't start OmaCal"],
  ['open', 'Start OmaCal'],
  ['background', 'Start OmaCal in the background'],
];

/**
 * The preferences the settings modal edits.
 *
 * Mirrors `settings::AppSettings` field for field. `minSyncIntervalMs` is
 * published by the backend rather than written here on purpose: the form has
 * to say what the minimum is in order to refuse a smaller one with a reason,
 * and a second copy of that number in TypeScript is one that drifts from the
 * `sync_loop::MIN_INTERVAL_MS` actually enforced.
 */
/** Which palette the app wears. `'auto'` is the desktop's — the Omarchy
 *  theme if there is one, dark if there is not — and the two others are the
 *  built-in palettes, chosen explicitly. */
export type Appearance =
  | 'auto' | 'light' | 'dark'
  | 'tokyo-night' | 'catppuccin-mocha' | 'catppuccin-latte' | 'rose-pine-dawn';

/** The rows, in the order the select offers them, and the labels the spec
 *  drives them by. Here rather than in the markup so the component and its
 *  test read one list. The four named themes are Omarchy's own most used,
 *  for a desktop with no Omarchy theme to follow (`theme::Palette::named`). */
export const APPEARANCE_OPTIONS: ReadonlyArray<[Appearance, string]> = [
  ['auto', 'Follow the desktop'],
  ['light', 'Light'],
  ['dark', 'Dark'],
  ['tokyo-night', 'Tokyo Night'],
  ['catppuccin-mocha', 'Catppuccin Mocha'],
  ['catppuccin-latte', 'Catppuccin Latte'],
  ['rose-pine-dawn', 'Rosé Pine Dawn'],
];

/** Whether the window draws a title bar. `'auto'` is no frame where a tiling
 *  compositor already closes and moves the window for you (Hyprland), and a
 *  frame everywhere else; the two others are for the desktop the rule gets
 *  wrong (issue #36). */
export type WindowFrame = 'auto' | 'shown' | 'hidden';

/** The three rows, in the order the select offers them — `APPEARANCE_OPTIONS`'s
 *  shape, for the same reason: the component and its test read one list. */
export const WINDOW_FRAME_OPTIONS: ReadonlyArray<[WindowFrame, string]> = [
  ['auto', 'Follow the desktop'],
  ['shown', 'Shown'],
  ['hidden', 'Hidden'],
];

export type AppSettings = {
  /** As **stored**, not as clamped. The loop clamps on the way out, because a
   *  row edited by hand with `sqlite3` — until now the only way to set this,
   *  documented in both platform guides — never passed through the command
   *  that refuses. A form showing the clamped value would silently disagree
   *  with the database it is editing. */
  syncIntervalMs: number;
  notificationsEnabled: boolean;
  /** Whether a task with a time announces itself when it comes due (#137).
   *  Its own switch, apart from the events one. */
  taskNotificationsEnabled: boolean;
  minSyncIntervalMs: number;
  /** Whether Day, Week and Month draw as a list rather than a grid (filmstrip
   *  spec §4). No settings tab shows it — the control is the `▦`/`☰` beside the
   *  view switcher — but it is a preference and is stored beside the others,
   *  which is what makes it survive a restart. */
  listMode: boolean;
  combineIdenticalEvents: boolean;
  /** Pixels per hour in Day and Week — what a pinch, Ctrl+scroll or Ctrl+=/-
   *  left the grid at. Stored for `listMode`'s reason and, like it, shown by
   *  no tab: the gesture is the control. `zoom.ts` owns the range. */
  hourHeight: number;
  /** How wide the tasks sidebar is, in pixels (#130). Dragged by its edge
   *  and kept, for `hourHeight`'s reason: a size set once should not be set
   *  again every morning. Clamped by the backend to a readable minimum and
   *  a width that keeps the calendar the larger half. */
  tasksWidth: number;
  /** Minutes-before for the fallback reminders (fallback spec §3): what fires
   *  for a timed event that follows its calendar's defaults when the calendar
   *  has none. Popup by construction — omacal never sends email. */
  fallbackReminderMinutes: number[];
  /** The calendar a new event lands on unless the user picks another, or
   *  `null` for the old rule — primary, else first writable. Stored
   *  unvalidated; `offerableCalendarId` guards staleness at every use. */
  defaultCalendarId: number | null;
  /** Minutes used when a new timed event has a start but no explicit end. */
  defaultEventDurationMinutes: number;
  /** The whole interface's size in percent, 75–200 (#138): the window's own
   *  zoom, applied by the backend. */
  interfaceScalePercent: number;
  /** Absolute calendar-canvas transparency, 0 (opaque) through 50, in 0.1% steps. */
  backgroundTransparency: number;
  inactiveBackgroundTransparency: number;
  /** Event-fill transparency, 0–25 in 0.1% steps, without fading text or outlines. */
  eventTransparency: number;
  /** The shared corner treatment for every event representation. */
  eventCornerStyle: EventCornerStyle;
  /** Whether the window can be seen through at all: true on Linux, whose
   *  window has a transparent backing store, false on macOS, whose does not.
   *  The modal offers the background slider only when it is true, the way
   *  `windowFrame`'s `null` hides the frame row — a fact about the window,
   *  not an OS name. */
  transparentWindow: boolean;
  /** Whether the app draws `13:30` or `1:30 PM`. Read by `timefmt.ts` through
   *  the `clock.svelte.ts` rune rather than as a prop — six components print a
   *  time and none of them owns the preference. */
  timeFormat: TimeFormat;
  dateFormat: DateFormat;
  /** Which desktop this build runs on, so the copy can name it. A fact about
   *  the host, not a stored preference — there is no setter. */
  desktop: 'macos' | 'omarchy' | 'linux';
  /** Which of the five view-switcher slots OmaCal opens on, when
   *  `defaultViewFollowsLast` is off. `'week'` by default — the view every
   *  existing install already opened to before this setting existed. */
  defaultView: View;
  /** Whether OmaCal ignores `defaultView` and opens on `lastView` instead —
   *  `weekStartsToday`'s shape for `weekStart`. */
  defaultViewFollowsLast: boolean;
  /** The view the switcher was most recently on, tracked regardless of
   *  `defaultViewFollowsLast` so turning that mode on opens on a real
   *  memory. `'week'` until anything has been recorded. */
  lastView: View;
  menubarDateFormat: DateFormat | 'general' | 'custom';
  menubarDateCustom: string;
  menubarLabelFormat: string;
  /** The day a week begins on. Read by the grids through the
   *  `weekstartstore.svelte.ts` rune, for the same reason `timeFormat` is. */
  weekStart: WeekStartDay;
  /** Whether Week view begins on the current day instead of the fixed
   *  `weekStart`. Month, Year and Big Year continue using the fixed day. */
  weekStartsToday: boolean;
  /** Number of columns in that rolling Week view, including today. */
  weekViewDays: WeekViewDays;
  visibleStartHour: number;
  visibleEndHour: number;
  /** Whether Saturday and Sunday columns are dropped from Week and Day
   *  view. Month, Year and Big Year are unaffected. */
  hideWeekends: boolean;
  /** Whether the system tray icon is shown. On by default — the tray is where
   *  Quit lives. Turning it off is for setups where something else carries
   *  those actions, like Omarchy 4's bar widget. */
  trayIcon: boolean;
  /** Whether the surfaces that can show today's date do: the tray icon,
   *  which *becomes* the date because a tray host draws icons and nothing
   *  else, and the Omarchy bar widget, which reads it from the feed. */
  showDate: boolean;
  menubarDayView?: boolean;
  menubarLabel?: boolean;
  menubarJoinMinutes?: number;
  /** The agenda popups' section choices — the Omarchy widget and the macOS
   *  menu bar alike, through the feed. Today is always shown and is not a
   *  choice; these are what comes before and after it. */
  menubarEarlier?: 'folded' | 'off';
  menubarTomorrow?: boolean;
  menubarDaysAhead?: number;
  /** Which palette the app wears. `'auto'` by default — omacal has no theme
   *  of its own and wears Omarchy's, which is exactly why the other two rows
   *  exist: off Omarchy there was no theme to wear and dark was the only
   *  answer the app had (issue #30). */
  appearance: Appearance;
  /** Whether the window draws a title bar, or `null` where the choice is not
   *  omacal's — macOS, whose overlay title bar *is* the frame. The modal
   *  shows the row only when there is an answer, so this is the one place
   *  the platform reaches the form, and it arrives as an absence rather than
   *  an OS name (issue #36). */
  windowFrame: WindowFrame | null;
  /** Whether closing the window quits omacal rather than hiding it. Off by
   *  default: reminders only fire while the app runs, so this is the user
   *  choosing to give them up until they open it again, and the hint under
   *  the checkbox says so. */
  quitOnClose: boolean;
  /** What a login does about omacal: nothing, open it, or run it without a
   *  window. `'open'` by default — a reminder can only fire while the app is
   *  running, and the row's hint says so, because that is the cost of `'off'`.
   *  Three states rather than two switches: "do not start" and "start without
   *  a window" are answers to one question. */
  startOnLogin: StartOnLogin;
  /** Whether the day headers carry the forecast — an icon and the high. On
   *  by default; the hint under the toggle names the sources (Open-Meteo,
   *  the Omarchy widget's location or the IP), because this is the one
   *  network destination beyond the calendar providers. */
  weatherEnabled: boolean;
  /** The place set for the forecast in Settings (#117), as the geocoder
   *  named it; `null` for the bar's setting, else the connection. */
  weatherLocation?: string | null;
  /** Whether the Location field asks Photon for place suggestions. Off
   *  until chosen — typed text leaves the machine, and Photon's public
   *  server asks for light personal use. History stays on either way. */
  photonPlaces: boolean;
  /** Whether the forecast high is drawn in Celsius or Fahrenheit — Celsius by
   *  default. Read by `WeekGrid` and `Filmstrip` through the
   *  `tempunit.svelte.ts` rune, for `timeFormat`'s reason: both print a
   *  temperature and neither owns the preference. */
  temperatureUnit: TemperatureUnit;
  /** The IANA zone every time in the app reads in, or `null` for the
   *  system's. Applied by exporting `TZ` before the webview starts, which is
   *  why changing it restarts omacal — the JS engine and libc both capture
   *  the zone at process start and offer no runtime swap. */
  displayTimezone: string | null;
  /** The zone the app is *actually* running in — `displayTimezone` when one
   *  is set, the machine's otherwise — named by the backend and frozen at
   *  launch. Every zone name the UI shows, and the `TZID` it writes, comes
   *  from here rather than from `Intl`, which resolves `Europe/Kyiv` to
   *  `Europe/Kiev` and so named a zone the picker never offered (#140).
   *  Read through the `zonename.svelte.ts` rune for `timeFormat`'s reason. */
  effectiveTimezone: string;
  /** A second zone shown beside times for convenience, or `null` for off.
   *  Display only — every write still happens in the display zone — and read
   *  through the `secondzone.svelte.ts` rune for `timeFormat`'s reason: the
   *  gutter and the form both print it and neither owns it. */
  secondTimezone: string | null;
};

export const getSettings = () => invoke<AppSettings>('get_settings');

/** Every appearance with the palette it would paint, for the theme picker. */
export const appearancePreviews = () =>
  invoke<{ appearance: Appearance; palette: import('./theme').Palette }[]>('appearance_previews');

/**
 * Everything `setSetting` stores, keyed as the backend's `settings::Setting`
 * spells it — the `AppSettings` field each lands in wherever there is one.
 * The grouped keys store several fields as one write, so a failure can never
 * leave half of one decision behind.
 *
 * Why each is refused, clamped or applied at once is documented once, on the
 * Rust variant; the backend answers every key with the settings now in force.
 */
export type SettingValues = {
  /** Refused below `minSyncIntervalMs`, never clamped. */
  syncIntervalMs: number;
  /** `null` turns the second clock off. */
  secondTimezone: string | null;
  weatherEnabled: boolean;
  photonPlaces: boolean;
  notificationsEnabled: boolean;
  taskNotificationsEnabled: boolean;
  trayIcon: boolean;
  appearance: Appearance;
  windowFrame: WindowFrame;
  quitOnClose: boolean;
  startOnLogin: StartOnLogin;
  /** `[]` is the feature turned off. */
  fallbackReminderMinutes: number[];
  /** `null` clears the choice. */
  defaultCalendarId: number | null;
  defaultEventDurationMinutes: number;
  interfaceScalePercent: number;
  appearancePreferences: {
    backgroundTransparency: number;
    eventTransparency: number;
    eventCornerStyle: EventCornerStyle;
    /** Absent means "the same as the active background". */
    inactiveBackgroundTransparency?: number;
  };
  listMode: boolean;
  showDate: boolean;
  menubarLabelFormat: string;
  menubarDateFormat: { format: AppSettings['menubarDateFormat']; custom: string };
  menubarPreferences: { dayView: boolean; label: boolean; joinMinutes: number };
  menubarDayView: boolean;
  menubarSections: { earlier: 'folded' | 'off'; tomorrow: boolean; daysAhead: number };
  /** Clamped rather than refused: the value comes off a gesture. */
  hourHeight: number;
  /** Clamped rather than refused: the value comes off a drag. */
  tasksWidth: number;
  dateFormat: DateFormat;
  timeFormat: TimeFormat;
  temperatureUnit: TemperatureUnit;
  /** Also leaves rolling mode. */
  weekStart: WeekStartDay;
  weekStartsToday: boolean;
  weekViewDays: WeekViewDays;
  visibleHours: { start: number; end: number };
  hideWeekends: boolean;
  combineIdenticalEvents: boolean;
  /** Also leaves "Last view" mode. */
  defaultView: View;
  defaultViewFollowsLast: boolean;
  lastView: View;
};

/** Stores one setting and answers with the settings now in force. */
export const setSetting = <K extends keyof SettingValues>(key: K, value: SettingValues[K]) =>
  invoke<AppSettings>('set_setting', { setting: { key, value } });

/** Sets the forecast's place by name, or clears it with `null`. Refused,
 *  with a sentence to show, for a name the geocoder does not know. */
export const setWeatherLocation = (name: string | null) =>
  invoke<AppSettings>('set_weather_location', { name });

/** Every zone the picker may offer — jiff's copy of the IANA database, the
 *  same authority the setter validates against. */
export const listTimezones = () => invoke<string[]>('list_timezones');

/**
 * Stores the display zone and **restarts omacal** to apply it; `null`
 * returns to the system zone. The reply arrives just before the restart, so
 * the form has one breath to say what is about to happen.
 */
export const setDisplayTimezone = (tz: string | null) =>
  invoke<void>('set_display_timezone', { tz });

/** Minutes, as the General tab shows them. Stored in milliseconds because
 *  that is what `sync_loop` compares against a clock. */
export const minutesOf = (ms: number): number => Math.round(ms / 60_000);
export const msOfMinutes = (min: number): number => Math.round(min * 60_000);

