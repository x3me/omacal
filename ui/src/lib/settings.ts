import { invoke } from '@tauri-apps/api/core';

import type { TemperatureUnit } from './temperature';
import type { TimeFormat } from './timefmt';
import type { WeekStartDay } from './weekstart';
import type { EventCornerStyle } from './appearance';

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
export type Appearance = 'auto' | 'light' | 'dark';

/** The three rows, in the order the select offers them, and the labels the
 *  spec drives them by. Here rather than in the markup so the component and
 *  its test read one list. */
export const APPEARANCE_OPTIONS: ReadonlyArray<[Appearance, string]> = [
  ['auto', 'Follow the desktop theme'],
  ['light', 'Light'],
  ['dark', 'Dark'],
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
  minSyncIntervalMs: number;
  /** Whether Day, Week and Month draw as a list rather than a grid (filmstrip
   *  spec §4). No settings tab shows it — the control is the `▦`/`☰` beside the
   *  view switcher — but it is a preference and is stored beside the others,
   *  which is what makes it survive a restart. */
  listMode: boolean;
  /** Pixels per hour in Day and Week — what a pinch, Ctrl+scroll or Ctrl+=/-
   *  left the grid at. Stored for `listMode`'s reason and, like it, shown by
   *  no tab: the gesture is the control. `zoom.ts` owns the range. */
  hourHeight: number;
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
  /** Absolute calendar-canvas transparency, 0 (opaque) through 100 (clear). */
  backgroundTransparency: number;
  /** Absolute event-fill transparency, without fading event text or outlines. */
  eventTransparency: number;
  /** The shared corner treatment for every event representation. */
  eventCornerStyle: EventCornerStyle;
  /** Whether the app draws `13:30` or `1:30 PM`. Read by `timefmt.ts` through
   *  the `clock.svelte.ts` rune rather than as a prop — six components print a
   *  time and none of them owns the preference. */
  timeFormat: TimeFormat;
  /** The day a week begins on. Read by the grids through the
   *  `weekstartstore.svelte.ts` rune, for the same reason `timeFormat` is. */
  weekStart: WeekStartDay;
  /** Whether Week view begins on the current day instead of the fixed
   *  `weekStart`. Month, Year and Big Year continue using the fixed day. */
  weekStartsToday: boolean;
  /** Number of columns in that rolling Week view, including today. */
  weekViewDays: WeekViewDays;
  /** Whether the system tray icon is shown. On by default — the tray is where
   *  Quit lives. Turning it off is for setups where something else carries
   *  those actions, like Omarchy 4's bar widget. */
  trayIcon: boolean;
  /** Whether the surfaces that can show today's date do: the tray icon,
   *  which *becomes* the date because a tray host draws icons and nothing
   *  else, and the Omarchy bar widget, which reads it from the feed. */
  showDate: boolean;
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
  /** A second zone shown beside times for convenience, or `null` for off.
   *  Display only — every write still happens in the display zone — and read
   *  through the `secondzone.svelte.ts` rune for `timeFormat`'s reason: the
   *  gutter and the form both print it and neither owns it. */
  secondTimezone: string | null;
};

export const getSettings = () => invoke<AppSettings>('get_settings');

/**
 * Stores a new sync interval and answers with the settings as they now are.
 *
 * **Rejects below the floor rather than clamping**, and the rejection is the
 * point: a value accepted and then quietly changed is worse than one turned
 * down. The form refuses first, so this is the second of two guards rather
 * than the only one — but it is the one that holds if the form ever forgets.
 */
export const setSyncInterval = (ms: number) =>
  invoke<AppSettings>('set_sync_interval', { ms });

export const setNotificationsEnabled = (on: boolean) =>
  invoke<AppSettings>('set_notifications_enabled', { on });

/** Stores the tray-icon preference; the backend also applies it to the
 *  running tray immediately, so the icon reacts to the click. */
/** Stores whether today's date is shown. The backend redresses the tray on
 *  the spot and rewrites the widget's feed, so the choice shows on both
 *  surfaces without waiting for either one's tick. */
export const setShowDate = (on: boolean) =>
  invoke<AppSettings>('set_show_date', { on });

export const setTrayIcon = (on: boolean) =>
  invoke<AppSettings>('set_tray_icon', { on });

/** Stores the palette choice. The backend repaints on the spot — it emits the
 *  same `theme-changed` event the Omarchy theme watcher does, so there is one
 *  repaint path rather than two, and no restart. */
export const setAppearance = (appearance: Appearance) =>
  invoke<AppSettings>('set_appearance', { appearance });

/** Stores the frame choice. The backend redecorates the window on the spot,
 *  so there is nothing to wait for and no restart. */
export const setWindowFrame = (frame: WindowFrame) =>
  invoke<AppSettings>('set_window_frame', { frame });

/** Stores the close-behaviour preference. Takes effect on the next close, not
 *  the next launch — the backend keeps its own copy of this one for the
 *  window handler to read. */
export const setQuitOnClose = (on: boolean) =>
  invoke<AppSettings>('set_quit_on_close', { on });

/** Stores the start-on-login choice; the backend registers or unregisters
 *  the launch entry in the same call, so *that* half is true the moment the
 *  modal reports it. Whether the entry opens a window is read at the next
 *  launch, which is the only place the question can be asked. */
export const setStartOnLogin = (mode: StartOnLogin) =>
  invoke<AppSettings>('set_start_on_login', { mode });

/** Stores the weather preference; a turn-on also fetches now, backend-side,
 *  so the headers change while the modal is still open. */
export const setWeatherEnabled = (on: boolean) =>
  invoke<AppSettings>('set_weather_enabled', { on });

/** Stores the temperature unit. Nothing is refetched: the cache is already
 *  unrounded Celsius regardless of this setting, so the headers just round
 *  differently on the next paint. */
export const setTemperatureUnit = (unit: TemperatureUnit) =>
  invoke<AppSettings>('set_temperature_unit', { unit });

/** Stores the clock format. Nothing is refused: `settings::TimeFormat` has two
 *  variants and the select offers both, so there is no third value to turn
 *  down — see the note on `set_time_format`. */
export const setTimeFormat = (format: TimeFormat) =>
  invoke<AppSettings>('set_time_format', { format });

/** Stores the day a week begins on. Nothing is refused: the select offers
 *  exactly the three variants `settings::WeekStart` has. */
export const setWeekStart = (start: WeekStartDay) =>
  invoke<AppSettings>('set_week_start', { start });

/** Enters or leaves the rolling Week view without changing the concrete day
 *  used to align Month, Year and Big Year. */
export const setWeekStartsToday = (on: boolean) =>
  invoke<AppSettings>('set_week_starts_today', { on });

/** Stores the rolling Week view's total column count. */
export const setWeekViewDays = (days: WeekViewDays) =>
  invoke<AppSettings>('set_week_view_days', { days });

/** Stores the filmstrip toggle. Nothing is refused: unlike the sync interval
 *  there is no value of a boolean the app has to protect anything from. */
export const setListMode = (on: boolean) =>
  invoke<AppSettings>('set_list_mode', { on });

/** Stores the hour height. The backend clamps rather than refuses — the
 *  value comes off a gesture, and "a little past the end" means the end. */
export const setHourHeight = (px: number) =>
  invoke<AppSettings>('set_hour_height', { px });

/** Stores the fallback reminder rows. The backend refuses out-of-bounds
 *  values with the limit named (fallback spec §3); `[]` is accepted and is
 *  the feature turned off. */
export const setFallbackReminders = (minutes: number[]) =>
  invoke<AppSettings>('set_fallback_reminders', { minutes });

/** Stores the default calendar for new events; `null` clears the choice. */
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

/**
 * Stores the second time zone; `null` turns the feature off. No restart,
 * unlike the display zone: nothing process-level captures this one — the
 * webview converts at render time from the IANA name — so the settings that
 * come back are already the settings in force.
 */
export const setSecondTimezone = (tz: string | null) =>
  invoke<AppSettings>('set_second_timezone', { tz });

export const setDefaultCalendar = (id: number | null) =>
  invoke<AppSettings>('set_default_calendar', { id });

/** Stores the free-form default length for new timed events, in minutes. */
export const setDefaultEventDuration = (minutes: number) =>
  invoke<AppSettings>('set_default_event_duration', { minutes });

/** Stores the canvas opacity, event-fill opacity, and event corner shape in a
 *  single transaction. Settings previews locally while a slider moves and
 *  invokes this once when that interaction commits. */
export const setAppearancePreferences = (
  backgroundTransparency: number,
  eventTransparency: number,
  eventCornerStyle: EventCornerStyle,
) => invoke<AppSettings>('set_appearance_preferences', {
  backgroundTransparency,
  eventTransparency,
  eventCornerStyle,
});

/** Minutes, as the General tab shows them. Stored in milliseconds because
 *  that is what `sync_loop` compares against a clock. */
export const minutesOf = (ms: number): number => Math.round(ms / 60_000);
export const msOfMinutes = (min: number): number => Math.round(min * 60_000);
