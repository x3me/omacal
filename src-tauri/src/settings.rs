//! The preferences the settings modal edits, and the two commands behind it.
//!
//! Everything here lives in the same `settings` key/value table `sync_loop`
//! and `status` already use — one table, string values, read with a parse and
//! a fallback. That is deliberate rather than lazy: a typed column per
//! preference means a migration per preference, and these are a handful of
//! scalars that a hand-edited row must never be able to crash the app with.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::AppState;

const SYNC_INTERVAL_KEY: &str = "sync_interval_ms";
const NOTIFICATIONS_KEY: &str = "notifications_enabled";
const LIST_MODE_KEY: &str = "list_mode";
const TRAY_DATE_KEY: &str = "tray_date";
const HOUR_HEIGHT_KEY: &str = "hour_height";
/// Pixels per hour in Day and Week when nobody has zoomed: the grid's own
/// 70 (see `WeekGrid.svelte`'s `.col`), and what an unusable stored value
/// falls back to.
pub const HOUR_HEIGHT_DEFAULT: i64 = 70;
/// The reach of the zoom. 30 puts a whole day in a laptop pane with the
/// hour labels still a line apart; 160 is six hours to a tall pane. Mirrored
/// in `ui/src/lib/zoom.ts`, which clamps the gesture before it ever asks —
/// this pair is the floor and ceiling the row is held to regardless.
pub const HOUR_HEIGHT_MIN: i64 = 30;
pub const HOUR_HEIGHT_MAX: i64 = 160;
const FALLBACK_KEY: &str = "fallback_reminder_minutes";
const DEFAULT_CALENDAR_KEY: &str = "default_calendar_id";
const DEFAULT_EVENT_DURATION_KEY: &str = "default_event_duration_minutes";
const TIME_FORMAT_KEY: &str = "time_format";
const WEEK_START_KEY: &str = "week_start";
const WEEK_STARTS_TODAY_KEY: &str = "week_starts_today";
const WEEK_VIEW_DAYS_KEY: &str = "week_view_days";
const TRAY_ICON_KEY: &str = "tray_icon";
const QUIT_ON_CLOSE_KEY: &str = "quit_on_close";
const AUTOSTART_KEY: &str = "autostart";
const WEATHER_KEY: &str = "weather_enabled";
const APPEARANCE_KEY: &str = "appearance";
const WINDOW_FRAME_KEY: &str = "window_frame";
const TEMPERATURE_UNIT_KEY: &str = "temperature_unit";
const DISPLAY_TZ_KEY: &str = "display_timezone";
const SECOND_TZ_KEY: &str = "second_timezone";

/// The boot fast-path beside the database: `main()` must export `TZ` before
/// GTK and the webview initialise — both capture the zone at process start —
/// and at that moment there is no Tauri handle to resolve the data dir with,
/// let alone an async pool. So the setter writes the zone to this one-line
/// sidecar too, and `apply_display_tz_early` (lib.rs) reads it with plain
/// std::fs. The database row stays the source of truth for the settings UI;
/// `setup` re-syncs the sidecar from it on every launch, so a divergence
/// (a crash between the two writes) heals itself one restart later.
pub(crate) const DISPLAY_TZ_SIDECAR: &str = "display-tz";

/// Writes (or removes, for "system default") the sidecar. Split out and
/// handed a directory so a test can drive it against a tempdir.
pub(crate) fn write_tz_sidecar(dir: &std::path::Path, tz: Option<&str>) -> std::io::Result<()> {
    let path = dir.join(DISPLAY_TZ_SIDECAR);
    match tz {
        Some(tz) => std::fs::write(path, tz),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

/// Whether a clock is drawn as `13:30` or as `1:30 PM`.
///
/// An enum rather than the `String` the table actually holds, so the only two
/// values that exist are the two the app can draw. That is what lets
/// [`set_time_format`] take this type directly and skip a refusal path
/// entirely: a third value cannot be sent, so there is no user-facing error
/// to name, pin with a test and allowlist in `errors.rs` for a case the
/// select element makes unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "12h")]
    H12,
}

impl TimeFormat {
    /// The stored spelling. The same strings the wire uses, so a row read by
    /// eye in `sqlite3` says what the settings modal says.
    fn as_str(self) -> &'static str {
        match self {
            TimeFormat::H24 => "24h",
            TimeFormat::H12 => "12h",
        }
    }
}

/// Whether a temperature is drawn as `22°` or `72°` — Celsius or Fahrenheit.
///
/// [`TimeFormat`]'s reason, twice over: the set is closed, so [`set_temperature_unit`]
/// needs no refusal path, and `weather::DayWeather` carries unrounded Celsius
/// so this side can round once, in whichever unit this names, rather than
/// rounding at fetch and converting a rounded number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemperatureUnit {
    #[serde(rename = "celsius")]
    Celsius,
    #[serde(rename = "fahrenheit")]
    Fahrenheit,
}

impl TemperatureUnit {
    /// The stored spelling. The same strings the wire uses, so a row read by
    /// eye in `sqlite3` says what the settings modal says.
    fn as_str(self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "celsius",
            TemperatureUnit::Fahrenheit => "fahrenheit",
        }
    }
}

/// Whether the window draws a title bar (issue #36).
///
/// Three states for [`StartOnLogin`]'s reason: there are three answers
/// people want. `Auto` is the one every install had before the setting
/// existed, now spelled out — no frame where a tiling compositor already
/// closes and moves the window for you, a frame everywhere else. The two
/// others are for the person whose desktop the rule gets wrong.
///
/// The frame was switched off for Hyprland, where a GTK headerbar only
/// repeats what SUPER+W and SUPER+drag do and costs a bar's height of
/// calendar. That reasoning was then applied, silently, to every Linux
/// desktop — and on GNOME or KDE it left a window with nothing to grab and
/// no close button. The bug behind the issue was not the frameless look but
/// that there was no second answer, which is [`crate::theme::Appearance`]'s
/// story again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowFrame {
    Auto,
    Shown,
    Hidden,
}

impl WindowFrame {
    /// The stored spelling — the wire's, for [`TemperatureUnit::as_str`]'s
    /// reason.
    fn as_str(self) -> &'static str {
        match self {
            WindowFrame::Auto => "auto",
            WindowFrame::Shown => "shown",
            WindowFrame::Hidden => "hidden",
        }
    }
}

/// Whether the window should carry a frame, given the choice and whether a
/// tiling compositor is running the desktop. The decision, kept apart from
/// the two things that feed it (the row, the environment) so it can be
/// tested as a table — `tray::hide_instead_of_closing`'s split, and
/// `lib.rs`'s `setup` and [`apply_window_frame`] do the OS half.
pub(crate) fn decorated(choice: WindowFrame, tiled: bool) -> bool {
    match choice {
        WindowFrame::Shown => true,
        WindowFrame::Hidden => false,
        WindowFrame::Auto => !tiled,
    }
}

/// Whether this process is running under Hyprland — the one compositor the
/// frameless default was made for and verified on. Hyprland exports its
/// instance signature to every client it starts, and nothing else sets it.
/// Other tiling compositors are not detected on purpose: an install there
/// keeps a frame by default and has the setting, rather than inheriting a
/// guess made for a different desktop.
pub(crate) fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// What a login should do about omacal.
///
/// Three states rather than a switch, because there are three answers people
/// actually want and only two of them are the same question. `Open` is the
/// calendar you want on screen at the start of the day; `Background` is the
/// one that should be *available* — reminders firing, the bar widget fed —
/// without a window nobody asked for appearing every session.
///
/// An enum for [`TimeFormat`]'s reason: the set is closed, so the setter
/// needs no refusal path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartOnLogin {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "background")]
    Background,
}

impl StartOnLogin {
    /// The stored spelling, which is also the wire spelling.
    fn as_str(self) -> &'static str {
        match self {
            StartOnLogin::Off => "off",
            StartOnLogin::Open => "open",
            StartOnLogin::Background => "background",
        }
    }

    /// Whether a launch entry should exist at all.
    pub(crate) fn registers(self) -> bool {
        !matches!(self, StartOnLogin::Off)
    }

    /// Whether a launch *from that entry* should put a window on screen.
    ///
    /// Only ever asked about a login launch — a manual one opens the window
    /// whatever this says, which is the whole reason the entry carries a flag
    /// rather than this being read as "always start hidden". See
    /// [`crate::tray::opens_window`].
    pub(crate) fn opens_window(self) -> bool {
        !matches!(self, StartOnLogin::Background)
    }
}

/// The day a week begins on.
///
/// Three, not seven, and they are Google Calendar's own three — this is a
/// Google Calendar client, and a week starting on a Wednesday is a preference
/// no calendar this one syncs with can express. An enum for the same reason
/// [`TimeFormat`] is one: the set is closed, so [`set_week_start`] needs no
/// refusal path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeekStart {
    #[serde(rename = "monday")]
    Monday,
    #[serde(rename = "sunday")]
    Sunday,
    #[serde(rename = "saturday")]
    Saturday,
}

impl WeekStart {
    /// The stored spelling, which is also the wire spelling.
    fn as_str(self) -> &'static str {
        match self {
            WeekStart::Monday => "monday",
            WeekStart::Sunday => "sunday",
            WeekStart::Saturday => "saturday",
        }
    }

    /// This day as jiff's own weekday, for the grid anchors that walk
    /// backwards to it.
    pub(crate) fn weekday(self) -> jiff::civil::Weekday {
        use jiff::civil::Weekday;
        match self {
            WeekStart::Monday => Weekday::Monday,
            WeekStart::Sunday => Weekday::Sunday,
            WeekStart::Saturday => Weekday::Saturday,
        }
    }

    /// How many blank cells precede a month whose 1st falls on `first` — the
    /// month grid's `lead_blanks`.
    ///
    /// Monday-zero throughout rather than jiff's two offset helpers chosen per
    /// variant: one origin, one subtraction, and the modulo does the wrapping.
    /// Mixing the two origins is how this arithmetic goes wrong.
    pub(crate) fn lead_blanks(self, first: jiff::civil::Weekday) -> usize {
        let day = first.to_monday_zero_offset() as usize;
        let start = self.weekday().to_monday_zero_offset() as usize;
        (day + 7 - start) % 7
    }

    /// Whether the column at `index` in a week-aligned row is a weekend day.
    ///
    /// Read off the *index*, never off the date the column carries — the
    /// property Big Year's 28-day rows exist to guarantee (see
    /// `every_row_puts_its_weekends_in_the_same_columns`). Note that only a
    /// Monday start puts Saturday and Sunday next to each other; the other two
    /// split the pair to the ends of the row, exactly as they do in every
    /// month grid those readers have ever used.
    ///
    /// Used by the Rust suite rather than by the app: the shading itself is
    /// drawn in the browser, from `weekstart.ts`'s own copy of this rule. That
    /// is exactly why this exists — `the_ribbons_weekend_stripes_stay_straight_under_every_start`
    /// asserts the two agree against real dates, so the copy cannot drift.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn is_weekend_column(self, index: usize) -> bool {
        use jiff::civil::Weekday;
        let start = self.weekday().to_monday_zero_offset() as usize;
        let weekday = (start + index) % 7;
        weekday == Weekday::Saturday.to_monday_zero_offset() as usize
            || weekday == Weekday::Sunday.to_monday_zero_offset() as usize
    }
}

/// What the General and Notifications tabs show.
///
/// `sync_interval_ms` is reported as **stored**, not as clamped. The clamp in
/// [`crate::sync_loop::interval_ms`] is a defence against a row somebody
/// edited by hand with `sqlite3` — which the platform guides documented as the
/// only way to change this until now — and reporting the clamped value here
/// would make the form silently disagree with the database it is editing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub sync_interval_ms: i64,
    pub notifications_enabled: bool,
    /// The floor, published rather than duplicated in the UI. The form has to
    /// say what the minimum is in order to refuse a smaller one with a reason,
    /// and a second copy of the number in TypeScript is one that drifts.
    pub min_sync_interval_ms: i64,
    /// Whether Day, Week and Month draw as a list rather than a grid — the
    /// filmstrip toggle (filmstrip spec §4).
    ///
    /// Here rather than in a table of its own because it is a preference and
    /// belongs beside the others, and because the alternative — remembering it
    /// only for the session — cannot survive the restart the spec asks it to.
    /// No tab in the settings modal shows it: the control that sets it is the
    /// `▦`/`☰` beside the view switcher, and a second control for the same
    /// value in a modal would be a second place for it to disagree.
    pub list_mode: bool,
    /// Whether the tray wears today's date instead of the app's mark
    /// (2026-09-04). A tray host draws icons and nothing else, so a date
    /// there has to *be* the icon — see `tray_date`. Off by default: the
    /// mark is what the tray has always shown, and what says which app it
    /// is at a glance.
    pub tray_date: bool,
    /// Pixels per hour in Day and Week (2026-09-03): what a pinch,
    /// Ctrl+scroll or Ctrl+=/- left the grid at. Here for `list_mode`'s
    /// reason — a zoom that lasted one session would be redone every
    /// morning — and, like it, shown by no tab in the modal: the gesture is
    /// the control. Held to [`HOUR_HEIGHT_MIN`]..=[`HOUR_HEIGHT_MAX`] on
    /// write; a stored value outside it, or not a number, reads as
    /// [`HOUR_HEIGHT_DEFAULT`].
    pub hour_height: i64,
    /// Minutes-before for the fallback reminders (fallback spec §3): what
    /// fires for a timed event that follows its calendar's defaults when the
    /// calendar has none. Minutes alone, because the fallback is popup by
    /// construction — omacal never sends email, so a method field here could
    /// only ever hold one value.
    pub fallback_reminder_minutes: Vec<i64>,
    /// The calendar a new event lands on unless the user picks another, or
    /// `None` for "the primary, else the first writable" — the rule that
    /// existed before this setting did. **Stored unvalidated on purpose**: a
    /// valid id goes stale the moment its calendar is removed or loses write
    /// access, so the use-site guard (`offerableCalendarId`, which replaces
    /// an id a create cannot land on) has to exist regardless — and a
    /// write-time check would only duplicate it with a second rule to drift.
    pub default_calendar_id: Option<i64>,
    /// Minutes a new timed event lasts when the user names only its start.
    /// Sixty preserves the existing behavior for installs without this row.
    pub default_event_duration_minutes: u32,
    /// Whether the system tray icon is shown. **On by default** — the tray is
    /// where Quit lives, and an app that hides its only quit affordance on a
    /// fresh install has made a decision nobody asked it to. Turning it off
    /// is for setups where something else carries the tray's three actions —
    /// the Omarchy 4 bar widget being the case this was built for, driving
    /// the app over the single-instance flags (`--sync-now`, `--quit`).
    pub tray_icon: bool,
    /// Which palette the app wears: the desktop's, or one of the two built
    /// in. **`Auto` by default**, which is what omacal has always done — the
    /// Omarchy theme if there is one, the dark fallback if there is not.
    ///
    /// The setting exists because that second half was a dead end: omacal has
    /// no theme of its own, so every desktop that is not Omarchy got dark and
    /// had no way to ask for anything else (issue #30).
    pub appearance: crate::theme::Appearance,
    /// Whether the window draws a title bar, or `None` where the choice is
    /// not omacal's to make — macOS, whose `titleBarStyle: "Overlay"` puts
    /// the traffic lights over the content, and where hiding them would
    /// leave no way to close the window. The modal shows the row only when
    /// there is an answer, so the platform stays a fact of the backend
    /// rather than something the form has to know (issue #36).
    pub window_frame: Option<WindowFrame>,
    /// Whether closing the window quits omacal instead of hiding it.
    ///
    /// **Off by default, and it stays a setting rather than becoming the
    /// behaviour**: reminders only fire while the process runs, so quitting
    /// on close is the user choosing to give them up until the app is opened
    /// again. §2.6 turned that down as a default for exactly that reason and
    /// nothing here reverses it — what changed (issue #26, 2026-08-31) is
    /// that "the window is closed, therefore I am done with it" is a
    /// legitimate way to want a desktop app to behave, and refusing it
    /// outright left people with a process they could only end from a tray
    /// icon they may also have turned off.
    pub quit_on_close: bool,
    /// What a login does about omacal. **`Open` by default**, for §2.6's
    /// reason: a reminder can only fire while the process is running, so an
    /// app that waits to be opened is an app whose notifications silently do
    /// not arrive — and defaulting to `Off` would do that to every existing
    /// install at once, on upgrade.
    ///
    /// The setting exists because until now there was no way *out*. Startup
    /// registered the launch entry unconditionally on every run, so deleting
    /// `~/.config/autostart/omacal.desktop` (or the macOS LaunchAgent) got it
    /// written straight back the next time the app opened — the app
    /// overruling the user's own system configuration, which is the part
    /// issue #22 is really about.
    ///
    /// `Background` is the second half of the same complaint, asked
    /// separately (2026-08-31): what people want at login is the calendar
    /// *available*, not a window in their face. It is a third state rather
    /// than a second switch because "do not start" and "start without a
    /// window" are answers to one question.
    pub start_on_login: StartOnLogin,
    /// Whether the day headers carry the forecast — an icon and the high,
    /// from the same sources the Omarchy bar widget reads (`weather.rs`).
    /// On by default: the data is decoration and the cost is one keyless
    /// Open-Meteo call every three hours — but it *is* the one network
    /// destination beyond the calendar providers, which is why the off
    /// switch exists and the settings hint names where the data comes from.
    pub weather_enabled: bool,
    /// Whether the day headers' forecast high is drawn in Celsius or
    /// Fahrenheit — Celsius by default, so no installed copy changes under
    /// its user. Read by the same components as `weather_enabled` guards,
    /// and only meaningful while that toggle is on.
    pub temperature_unit: TemperatureUnit,
    /// Whether times are drawn as `13:30` or `1:30 PM`, everywhere the app
    /// prints one — event blocks, the filmstrip, the popover and the Week and
    /// Day hour gutter, which follows deliberately: a 12-hour reader given a
    /// 24-hour ruler has to convert in their head at exactly the moment the
    /// ruler exists to save them from it.
    pub time_format: TimeFormat,
    /// The day a week begins on, honoured by the Week grid's own anchor, the
    /// month grid's leading blanks, the Year view's twelve small grids, and
    /// Big Year's 392-day ribbon. When `week_starts_today` is on, this still
    /// aligns the three calendar-shaped views; it is preserved so switching
    /// back from the rolling Week view restores the user's last fixed day.
    pub week_start: WeekStart,
    /// Whether Week view is a rolling range whose first column is the current
    /// day instead of a calendar-aligned week. This deliberately does not
    /// change Month, Year or Big Year: "today" is not a stable weekday those
    /// grids can align rows to.
    pub week_starts_today: bool,
    /// Total columns in the rolling Week view, including today. Only 3, 5 and
    /// 7 are written; an absent or hand-edited value falls back to 7.
    pub week_view_days: u8,
    /// The IANA zone every time in the app is read in, or `None` for the
    /// system's. Applied by exporting `TZ` before the webview starts — the
    /// one mechanism that keeps the browser, Rust, notifications and the
    /// widget feed coherent without threading a zone through every date
    /// computation — which is also why changing it restarts the app: both
    /// the JS engine and libc capture the zone at process start and offer
    /// no runtime swap.
    pub display_timezone: Option<String>,
    /// A second zone shown *beside* times for convenience, or `None` for off
    /// — Google Calendar's own feature, for the reader who lives in one zone
    /// and meets in another. Display only, and that is the whole contract:
    /// events are stored, laid out, edited and fired in the display zone
    /// above, and this one never touches a write. Which is also why changing
    /// it does **not** restart the app the way `display_timezone` does — no
    /// process-level `TZ` is involved; the webview converts at render time
    /// from the IANA name itself.
    pub second_timezone: Option<String>,
}

pub(crate) async fn read(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub(crate) async fn write(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// The settings as stored, with defaults for anything absent.
///
/// Absent is the ordinary case on a fresh install and is not an error:
/// nothing writes these until the user opens the modal.
pub async fn read_settings(pool: &SqlitePool) -> AppSettings {
    AppSettings {
        sync_interval_ms: read(pool, SYNC_INTERVAL_KEY)
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(crate::sync_loop::DEFAULT_INTERVAL_MS),
        // **Reminders are on unless somebody turned them off.** The opposite
        // default would mean a fresh install silently firing nothing, which
        // looks exactly like the notification transport being broken — and on
        // macOS, where it may genuinely be, the two would be indistinguishable.
        notifications_enabled: read(pool, NOTIFICATIONS_KEY)
            .await
            .map(|v| v != "0")
            .unwrap_or(true),
        min_sync_interval_ms: crate::sync_loop::MIN_INTERVAL_MS,
        // **The grid is what a calendar looks like until somebody says
        // otherwise**, so the absent row reads as off — the opposite polarity
        // to `notifications_enabled` above, and for the opposite reason. A
        // reminder nobody sees is indistinguishable from a broken transport; a
        // grid nobody asked for is just the app as it has always looked.
        //
        // `== "1"` rather than `!= "0"`, so a value from a future version or a
        // hand-edited row lands on that same default rather than silently
        // turning the calendar into a list.
        list_mode: read(pool, LIST_MODE_KEY).await.map(|v| v == "1").unwrap_or(false),
        // The mark unless the row says otherwise, for `list_mode`'s reason:
        // a hand-edited value must land on what the app has always drawn.
        tray_date: read(pool, TRAY_DATE_KEY).await.map(|v| v == "1").unwrap_or(false),
        hour_height: read(pool, HOUR_HEIGHT_KEY)
            .await
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|px| (HOUR_HEIGHT_MIN..=HOUR_HEIGHT_MAX).contains(px))
            .unwrap_or(HOUR_HEIGHT_DEFAULT),
        // **Shipped as 60 and 10, not empty** (fallback spec §3): the gap
        // this fills is real meetings going silent on receive-only shared
        // calendars, and an empty default would leave a fresh install with
        // exactly that surprise. `[]` stored is a real choice — the feature
        // off — and survives; only an absent or unparseable row lands here.
        fallback_reminder_minutes: read(pool, FALLBACK_KEY)
            .await
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_else(|| vec![60, 10]),
        // Empty and absent both mean "system": the row is written as "" to
        // clear, and a blank zone name is nothing anyone chose.
        display_timezone: read(pool, DISPLAY_TZ_KEY)
            .await
            .filter(|v| !v.trim().is_empty()),
        // Same convention: "" is the feature off, which is the fresh-install
        // state and not an error.
        second_timezone: read(pool, SECOND_TZ_KEY)
            .await
            .filter(|v| !v.trim().is_empty()),
        // Absent, cleared ("" — see `set_default_calendar`) and garbage all
        // read as `None`: the old rule, never an error.
        default_calendar_id: read(pool, DEFAULT_CALENDAR_KEY)
            .await
            .and_then(|v| v.parse().ok()),
        default_event_duration_minutes: read(pool, DEFAULT_EVENT_DURATION_KEY)
            .await
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&minutes| minutes > 0)
            .unwrap_or(60),
        // `== "12h"` rather than `!= "24h"`, the same polarity `list_mode`
        // takes and for the same reason: absent, garbage, and a value written
        // by some future version all land on the format the app has always
        // drawn, rather than on the one nobody asked for.
        time_format: read(pool, TIME_FORMAT_KEY)
            .await
            .map(|v| if v == "12h" { TimeFormat::H12 } else { TimeFormat::H24 })
            .unwrap_or(TimeFormat::H24),
        // Same polarity rule as its two neighbours: only the two spellings
        // this version writes move the setting, and everything else — absent,
        // hand-edited, or written by a version that learned a fourth day —
        // lands on the week omacal has always drawn.
        week_start: match read(pool, WEEK_START_KEY).await.as_deref() {
            Some("sunday") => WeekStart::Sunday,
            Some("saturday") => WeekStart::Saturday,
            _ => WeekStart::Monday,
        },
        // Opt-in, like list mode: absent, garbage and a future spelling keep
        // the calendar-aligned week the app has always drawn.
        week_starts_today: read(pool, WEEK_STARTS_TODAY_KEY)
            .await
            .map(|v| v == "1")
            .unwrap_or(false),
        // The select offers exactly these three. A row edited by hand must not
        // become an unbounded query or a zero-column grid, so all other values
        // return to the old seven-day shape.
        week_view_days: match read(pool, WEEK_VIEW_DAYS_KEY).await.as_deref() {
            Some("3") => 3,
            Some("5") => 5,
            _ => 7,
        },
        // Same polarity as `notifications_enabled`, same reason: absent and
        // garbage both keep the icon — losing the quit affordance must take
        // an explicit "0", never a typo.
        tray_icon: read(pool, TRAY_ICON_KEY).await.map(|v| v != "0").unwrap_or(true),
        quit_on_close: quit_on_close(pool).await,
        appearance: appearance(pool).await,
        // `None` is an absence, not a default: on macOS the frame is the
        // system's, and a row that does nothing is worse than no row.
        // `cfg!` rather than `#[cfg]` so both arms compile on the
        // Linux-only CI.
        window_frame: if cfg!(target_os = "macos") {
            None
        } else {
            Some(window_frame(pool).await)
        },
        // Same "only a spelling this version writes moves the setting" rule
        // as its three neighbours, and here it is load bearing twice over:
        // absent is every install that predates this setting, and those all
        // *have* the launch entry already — landing them anywhere but `Open`
        // would change what their machine does at the next login, which is
        // precisely the silent change this setting exists to stop.
        start_on_login: start_on_login(pool).await,
        // `notifications_enabled`'s polarity and reasoning: on unless
        // somebody turned it off.
        weather_enabled: weather_enabled(pool).await,
        // Same "only a spelling this version writes moves the setting" rule
        // as `week_start`: absent, garbage and a future spelling all land on
        // Celsius, the unit omacal has always drawn.
        temperature_unit: match read(pool, TEMPERATURE_UNIT_KEY).await.as_deref() {
            Some("fahrenheit") => TemperatureUnit::Fahrenheit,
            _ => TemperatureUnit::Celsius,
        },
    }
}

/// What the user is told when a sync interval below the floor is refused.
///
/// A named constant for the same reason the other two are: it is pinned by a
/// test and allowlisted in `errors.rs`, and the two must not drift.
pub const INTERVAL_TOO_SHORT: &str =
    "OmaCal will not sync more often than once a minute — Google's quota is finite and a \
     desktop app has no business polling faster than that";

pub const EVENT_DURATION_TOO_SHORT: &str =
    "the default meeting duration must be at least one minute";

#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(read_settings(&state.pool).await)
}

/// Stores a new sync interval, **refusing anything below the floor**.
///
/// Refused rather than clamped, and that is the whole of the decision. A value
/// accepted and then quietly changed is worse than one that is turned down: the
/// user types 10 seconds, the form says nothing, and the app polls every minute
/// while they believe otherwise. `sync_loop::interval_ms` still clamps on the
/// way *out*, because a row edited by hand with `sqlite3` — the only way to set
/// this until now, documented in both platform guides — never passed through
/// here at all.
#[tauri::command]
pub async fn set_sync_interval(
    state: tauri::State<'_, AppState>,
    ms: i64,
) -> Result<AppSettings, String> {
    set_sync_interval_impl(&state.pool, ms)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_sync_interval_impl(pool: &SqlitePool, ms: i64) -> anyhow::Result<AppSettings> {
    if ms < crate::sync_loop::MIN_INTERVAL_MS {
        anyhow::bail!(INTERVAL_TOO_SHORT);
    }
    write(pool, SYNC_INTERVAL_KEY, &ms.to_string()).await?;
    Ok(read_settings(pool).await)
}

/// Every zone the picker may offer, straight from jiff's copy of the IANA
/// database — the same authority `TimeZone::get` validates against, so the
/// list and the validator cannot disagree.
#[tauri::command]
pub fn list_timezones() -> Vec<String> {
    let mut zones: Vec<String> =
        jiff::tz::db().available().map(|name| name.to_string()).collect();
    zones.sort();
    zones
}

/// Stores the display zone and restarts the app to apply it — see
/// [`AppSettings::display_timezone`] for why a restart is the mechanism.
/// `None` returns to the system zone.
///
/// The restart is spawned on a short delay so this command's reply reaches
/// the webview first and the form can say "restarting" instead of dying
/// mid-await. Validation refuses rather than stores: a zone jiff does not
/// know would come back at next launch as a `TZ` nothing honours, which is
/// the system zone wearing the wrong label.
#[tauri::command]
pub async fn set_display_timezone(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    tz: Option<String>,
) -> Result<(), String> {
    let tz = tz.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if let Some(name) = tz.as_deref() {
        jiff::tz::TimeZone::get(name)
            .map_err(|_| format!("OmaCal does not know the time zone \"{name}\""))?;
    }

    write(&state.pool, DISPLAY_TZ_KEY, tz.as_deref().unwrap_or(""))
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;

    use tauri::Manager;
    if let Ok(dir) = app.path().app_data_dir() {
        if let Err(e) = write_tz_sidecar(&dir, tz.as_deref()) {
            // The DB row is stored; setup's re-sync writes the sidecar on
            // the next launch, so this costs one restart of staleness.
            tracing::warn!(%e, "could not write the display-tz sidecar");
        }
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        drop(app);
        crate::restart::hard_restart();
    });
    Ok(())
}

/// Stores the second time zone — the convenience clock beside the real one —
/// or clears it with `None`/blank. Validated against the same authority the
/// display zone is, and for the same reason: a zone jiff does not know is a
/// name the webview's own converter will not know either, and storing it
/// would draw a gutter of blanks. **No restart**, unlike its neighbour: see
/// [`AppSettings::second_timezone`] — nothing process-level captures this
/// zone, so the reply alone is enough for the UI to start drawing it.
#[tauri::command]
pub async fn set_second_timezone(
    state: tauri::State<'_, AppState>,
    tz: Option<String>,
) -> Result<AppSettings, String> {
    // The refusal carries the name, so it is built here and returned
    // directly — `errors::user_facing` allowlists exact strings and would
    // withhold a message with a zone name in it (see `set_display_timezone`,
    // which routes its own refusal the same way).
    let tz = validate_second_timezone(tz)?;
    write(&state.pool, SECOND_TZ_KEY, tz.as_deref().unwrap_or(""))
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Blank and `None` collapse to off; a non-blank name must be one jiff knows.
/// Split from the command so the rule is reachable from a test without a
/// `tauri::State`.
fn validate_second_timezone(tz: Option<String>) -> Result<Option<String>, String> {
    let tz = tz.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if let Some(name) = tz.as_deref() {
        jiff::tz::TimeZone::get(name)
            .map_err(|_| format!("OmaCal does not know the time zone \"{name}\""))?;
    }
    Ok(tz)
}

/// Restarts the app because the user asked to — the system-tz banner's one
/// action. The same delayed shape as [`set_display_timezone`]'s restart and
/// for the same reason: the reply has to reach the webview before the
/// process re-execs, so the button can say "restarting" instead of dying
/// mid-await. No state to write first: the restart *is* the fix, because the
/// fresh process reads the zone the system already moved to. Through
/// [`crate::restart::hard_restart`], not `app.restart()` — the graceful
/// teardown is the thing that hangs (its module doc has the field story).
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        drop(app);
        crate::restart::hard_restart();
    });
}

/// The one settings read the weather loop repeats every tick, named so the
/// loop and `read_settings` cannot disagree on default or polarity.
pub(crate) async fn weather_enabled(pool: &SqlitePool) -> bool {
    read(pool, WEATHER_KEY).await.map(|v| v != "0").unwrap_or(true)
}

/// Which palette to wear (issue #30), named for [`weather_enabled`]'s reason:
/// `get_palette`, `setup`'s GTK hint and the theme watcher all need the
/// answer, and three parses of one row is three chances to disagree about
/// what an absent one means.
///
/// Absent, garbage and a spelling a future version writes all resolve to
/// `Auto` — the behaviour every installed copy already has.
pub(crate) async fn appearance(pool: &SqlitePool) -> crate::theme::Appearance {
    match read(pool, APPEARANCE_KEY).await.as_deref() {
        Some("light") => crate::theme::Appearance::Light,
        Some("dark") => crate::theme::Appearance::Dark,
        _ => crate::theme::Appearance::Auto,
    }
}

/// The window-frame choice (issue #36), named for [`weather_enabled`]'s
/// reason: `setup` reads it at launch to decorate the window before it is
/// shown, and `read_settings` reports it to the form.
///
/// Absent, garbage and a spelling a future version writes all resolve to
/// `Auto` — what every installed copy already had on Hyprland, and the frame
/// that GNOME and KDE installs were missing.
pub(crate) async fn window_frame(pool: &SqlitePool) -> WindowFrame {
    match read(pool, WINDOW_FRAME_KEY).await.as_deref() {
        Some("shown") => WindowFrame::Shown,
        Some("hidden") => WindowFrame::Hidden,
        _ => WindowFrame::Auto,
    }
}

/// Whether closing the window should quit (issue #26), named for
/// [`weather_enabled`]'s reason: `setup` reads it once to seed the flag the
/// window handler consults, and `read_settings` reports it to the form.
///
/// The polarity is the safety. Absent — every install that predates this
/// setting — and anything a hand-edited row could hold both mean "hide", so
/// giving up the reminders takes an explicit `"1"` this version wrote.
pub(crate) async fn quit_on_close(pool: &SqlitePool) -> bool {
    read(pool, QUIT_ON_CLOSE_KEY).await.map(|v| v == "1").unwrap_or(false)
}

/// What a login should do, named for the same reason [`weather_enabled`] is:
/// `setup` reads it at launch — twice, once to decide the launch entry and
/// once to decide the window — and `read_settings` reports it to the form.
/// Three call sites, one parse, no chance of them disagreeing about what an
/// absent row means.
///
/// `"1"`/`"0"` are accepted alongside the three names because the switch
/// shipped on `main` as a boolean for a few hours before it became a choice.
/// No tagged release ever wrote them, so this is for people building from
/// `main` — cheap enough that the alternative (their setting silently
/// reverting) is not worth the two lines saved.
pub(crate) async fn start_on_login(pool: &SqlitePool) -> StartOnLogin {
    match read(pool, AUTOSTART_KEY).await.as_deref() {
        Some("off" | "0") => StartOnLogin::Off,
        Some("background") => StartOnLogin::Background,
        _ => StartOnLogin::Open,
    }
}

/// Stores the weather preference — and on a turn-on, fetches now rather
/// than at the loop's next three-hour tick: a toggle that answers with an
/// unchanged header for an hour reads as broken, exactly like a tray icon
/// that only appears at next launch would.
#[tauri::command]
pub async fn set_weather_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, WEATHER_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    if on {
        crate::weather::refresh_soon(app, state.pool.clone(), state.demo, true);
    }
    Ok(read_settings(&state.pool).await)
}

#[tauri::command]
pub async fn set_notifications_enabled(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, NOTIFICATIONS_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the tray-icon preference and applies it to the running tray in the
/// same breath — a visibility toggle that only took effect next launch would
/// read as broken every single time.
#[tauri::command]
pub async fn set_tray_icon(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, TRAY_ICON_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    crate::tray::set_visible(&app, on);
    Ok(read_settings(&state.pool).await)
}

/// Stores the palette choice and repaints on the spot.
///
/// Both halves are the point. The webview is repainted by the same
/// `theme-changed` event the Omarchy watcher emits, so there is one repaint
/// path rather than two; and GTK's dark hint follows, because WebKitGTK draws
/// its `<select>` popups from the GTK theme rather than the page — a light app
/// with black dropdowns is the bug that hint exists for. Through the main
/// thread, since GTK settings must not be touched from a command's thread.
#[tauri::command]
pub async fn set_appearance(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    appearance: crate::theme::Appearance,
) -> Result<AppSettings, String> {
    write(&state.pool, APPEARANCE_KEY, appearance.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;

    let palette = crate::theme::resolve(
        crate::theme::omarchy_theme_dir().as_deref(),
        appearance,
    );
    let dark = palette.is_dark;
    let _ = app.run_on_main_thread(move || crate::apply_gtk_dark_hint(dark));
    use tauri::Emitter;
    let _ = app.emit("theme-changed", palette);

    Ok(read_settings(&state.pool).await)
}

/// Stores the window-frame choice and applies it in the same breath, for
/// `set_tray_icon`'s reason: a control whose effect waits for the next
/// launch cannot be told from one that does nothing. The frame is the
/// compositor's or GTK's to draw, and both take the change live.
#[tauri::command]
pub async fn set_window_frame(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    frame: WindowFrame,
) -> Result<AppSettings, String> {
    write(&state.pool, WINDOW_FRAME_KEY, frame.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    apply_window_frame(&app, frame);
    Ok(read_settings(&state.pool).await)
}

/// The OS half of [`decorated`]: sets the main window's decorations from a
/// choice and the desktop it is on. Called by `setup` before the window is
/// shown, and again from [`set_window_frame`]. A no-op on macOS, where the
/// overlay title bar is the frame and `read_settings` reports no choice —
/// `cfg!` so the Linux-only CI still compiles the macOS arm.
pub(crate) fn apply_window_frame(app: &tauri::AppHandle, frame: WindowFrame) {
    if cfg!(target_os = "macos") {
        return;
    }
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_decorations(decorated(frame, on_hyprland()));
    }
}

/// Stores the close-behaviour preference and updates the flag the window
/// handler reads, in the same breath and for `set_tray_icon`'s reason: a
/// setting that only took effect at the next launch would read as broken.
///
/// The database row is the source of truth; [`AppState::quit_on_close`] is a
/// mirror of it, because `WindowEvent::CloseRequested` is a synchronous
/// handler that has to answer before the window is gone and cannot await a
/// query. Seeded from the row at startup, so a crash between the two writes
/// heals on the next launch.
#[tauri::command]
pub async fn set_quit_on_close(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, QUIT_ON_CLOSE_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    state.quit_on_close.store(on, std::sync::atomic::Ordering::Relaxed);
    Ok(read_settings(&state.pool).await)
}

/// Stores the start-on-login choice and registers or unregisters the launch
/// entry in the same breath, for `set_tray_icon`'s reason: a control whose
/// effect waits for the next launch cannot be told from one that does
/// nothing, and *this* control's whole job is undoing something the app did
/// without being asked.
///
/// Nothing is refused: `StartOnLogin` has three variants and the select
/// offers all three, so there is no fourth value to turn down — the note on
/// [`set_time_format`] in full.
///
/// **Only the entry's existence is applied now**; whether that entry opens a
/// window is read at the next launch, because a session already running
/// cannot un-open its own window retroactively. That is not a gap: the whole
/// preference is about what the *next* login does.
#[tauri::command]
pub async fn set_start_on_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mode: StartOnLogin,
) -> Result<AppSettings, String> {
    write(&state.pool, AUTOSTART_KEY, mode.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    crate::tray::apply_autostart(&app, state.demo, mode.registers());
    Ok(read_settings(&state.pool).await)
}

/// Stores the fallback reminder rows, through the same bounds the event
/// form's rows are held to — `write::validate_reminders`, so the two cannot
/// drift apart — and refused with the limit named, never clamped (spec §3).
/// `[]` is accepted and meaningful: it is the feature turned off.
#[tauri::command]
pub async fn set_fallback_reminders(
    state: tauri::State<'_, AppState>,
    minutes: Vec<i64>,
) -> Result<AppSettings, String> {
    set_fallback_reminders_impl(&state.pool, minutes)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

pub(crate) async fn set_fallback_reminders_impl(
    pool: &SqlitePool,
    minutes: Vec<i64>,
) -> anyhow::Result<AppSettings> {
    let as_input = crate::write::RemindersInput {
        use_default: false,
        overrides: minutes
            .iter()
            .map(|&m| crate::write::ReminderInput { method: "popup".into(), minutes: m })
            .collect(),
    };
    crate::write::validate_reminders(&as_input).map_err(|m| anyhow::anyhow!(m))?;
    write(pool, FALLBACK_KEY, &serde_json::to_string(&minutes)?).await?;
    Ok(read_settings(pool).await)
}

/// Stores the default calendar for new events. `None` clears the choice —
/// written as an empty value rather than a deleted row, so `write`'s upsert
/// is the only statement this module ever makes about the table.
#[tauri::command]
pub async fn set_default_calendar(
    state: tauri::State<'_, AppState>,
    id: Option<i64>,
) -> Result<AppSettings, String> {
    write(&state.pool, DEFAULT_CALENDAR_KEY, &id.map(|v| v.to_string()).unwrap_or_default())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the length used when a new event names a start but no explicit end.
/// Zero is refused rather than repaired: a saved preference must be the value
/// the user entered, and a zero-length event cannot be created.
#[tauri::command]
pub async fn set_default_event_duration(
    state: tauri::State<'_, AppState>,
    minutes: u32,
) -> Result<AppSettings, String> {
    set_default_event_duration_impl(&state.pool, minutes)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_default_event_duration_impl(
    pool: &SqlitePool,
    minutes: u32,
) -> anyhow::Result<AppSettings> {
    if minutes == 0 {
        anyhow::bail!(EVENT_DURATION_TOO_SHORT);
    }
    write(pool, DEFAULT_EVENT_DURATION_KEY, &minutes.to_string()).await?;
    Ok(read_settings(pool).await)
}

/// Stores the filmstrip toggle. Nothing is refused and nothing is clamped —
/// unlike the sync interval, there is no value of a boolean the app has to
/// protect Google's quota from.
#[tauri::command]
pub async fn set_list_mode(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, LIST_MODE_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores whether the tray wears the date. Nothing to refuse, as with the
/// other booleans; the tray is redressed on the spot so the choice shows
/// without waiting for the minute tick.
#[tauri::command]
pub async fn set_tray_date(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, TRAY_DATE_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    crate::tray::refresh(&app);
    Ok(read_settings(&state.pool).await)
}

/// Stores the hour height, clamped rather than refused: the value comes off
/// a gesture, and the honest answer to "a little past the end" is the end,
/// not an error surfacing under somebody's fingers mid-pinch.
#[tauri::command]
pub async fn set_hour_height(
    state: tauri::State<'_, AppState>,
    px: i64,
) -> Result<AppSettings, String> {
    let px = px.clamp(HOUR_HEIGHT_MIN, HOUR_HEIGHT_MAX);
    write(&state.pool, HOUR_HEIGHT_KEY, &px.to_string())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the clock format. Like [`set_list_mode`] nothing is refused, and
/// here the *type* is the reason rather than the triviality of a boolean:
/// [`TimeFormat`] has no third variant for a caller to send.
#[tauri::command]
pub async fn set_time_format(
    state: tauri::State<'_, AppState>,
    format: TimeFormat,
) -> Result<AppSettings, String> {
    write(&state.pool, TIME_FORMAT_KEY, format.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the temperature unit. Like [`set_time_format`] nothing is refused
/// and the cache needs no refetch: `weather::DayWeather` carries unrounded
/// Celsius regardless of this setting, so a toggle changes only how the
/// headers round what is already cached.
#[tauri::command]
pub async fn set_temperature_unit(
    state: tauri::State<'_, AppState>,
    unit: TemperatureUnit,
) -> Result<AppSettings, String> {
    write(&state.pool, TEMPERATURE_UNIT_KEY, unit.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the day a calendar-aligned week begins on and leaves rolling mode.
/// Nothing to refuse: [`WeekStart`] has three variants and the select offers
/// all three. The two writes are one user choice; the helper keeps that shape
/// reachable from tests without constructing a Tauri `State`.
#[tauri::command]
pub async fn set_week_start(
    state: tauri::State<'_, AppState>,
    start: WeekStart,
) -> Result<AppSettings, String> {
    set_week_start_impl(&state.pool, start)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_week_start_impl(pool: &SqlitePool, start: WeekStart) -> anyhow::Result<AppSettings> {
    let mut tx = pool.begin().await?;
    for (key, value) in [(WEEK_START_KEY, start.as_str()), (WEEK_STARTS_TODAY_KEY, "0")] {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(read_settings(pool).await)
}

/// Turns the rolling Week view on or off. Turning it on preserves the concrete
/// `week_start` used by Month, Year and Big Year, and by Week when this is later
/// turned off again.
#[tauri::command]
pub async fn set_week_starts_today(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, WEEK_STARTS_TODAY_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the number of columns in a rolling Week view. The browser offers
/// only these values, and this guard keeps a hand-written invoke from asking
/// the backend to allocate an arbitrary number of day columns.
#[tauri::command]
pub async fn set_week_view_days(
    state: tauri::State<'_, AppState>,
    days: u8,
) -> Result<AppSettings, String> {
    set_week_view_days_impl(&state.pool, days)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_week_view_days_impl(pool: &SqlitePool, days: u8) -> anyhow::Result<AppSettings> {
    if !matches!(days, 3 | 5 | 7) {
        anyhow::bail!("the rolling week can show 3, 5, or 7 days");
    }
    write(pool, WEEK_VIEW_DAYS_KEY, &days.to_string()).await?;
    Ok(read_settings(pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        omacal_store::connect_memory().await.unwrap()
    }

    /// A fresh install has written none of these, and that is the ordinary
    /// case rather than an error.
    #[tokio::test]
    async fn absent_settings_read_as_their_defaults() {
        let s = read_settings(&pool().await).await;
        assert_eq!(s.sync_interval_ms, crate::sync_loop::DEFAULT_INTERVAL_MS);
        assert!(s.notifications_enabled, "reminders must be on until turned off");
        assert_eq!(s.min_sync_interval_ms, crate::sync_loop::MIN_INTERVAL_MS);
        assert!(!s.list_mode, "a fresh install draws the grid, not a list");
        assert!(!s.tray_date, "a fresh install wears the mark in the tray, not the date");
        assert_eq!(s.hour_height, HOUR_HEIGHT_DEFAULT, "a fresh install draws 70px hours");
        assert_eq!(
            s.fallback_reminder_minutes,
            vec![60, 10],
            "shipped as 60 and 10, not empty — an empty default is today's silence again"
        );
        assert_eq!(s.default_calendar_id, None, "no choice made is the old rule, not an id");
        assert_eq!(
            s.default_event_duration_minutes,
            60,
            "new events remain one hour long until somebody chooses otherwise",
        );
        assert_eq!(
            s.time_format,
            TimeFormat::H24,
            "the clock the app has always drawn, so no installed copy changes under its user"
        );
        assert_eq!(
            s.week_start,
            WeekStart::Monday,
            "the week omacal has always drawn"
        );
        assert!(!s.week_starts_today, "a fresh install keeps the calendar-aligned week");
        assert_eq!(s.week_view_days, 7, "the old Week view has seven columns");
        assert_eq!(
            s.start_on_login,
            StartOnLogin::Open,
            "absent is every install that predates this setting, and they all have the launch \
             entry already — landing anywhere else changes what their machine does at the \
             next login, without anybody asking for it"
        );
        assert_eq!(
            s.temperature_unit,
            TemperatureUnit::Celsius,
            "the unit omacal has always drawn, so no installed copy changes under its user"
        );
        assert_eq!(
            s.window_frame,
            if cfg!(target_os = "macos") { None } else { Some(WindowFrame::Auto) },
            "no frame under Hyprland and one everywhere else — what every install already \
             had, plus the frame the other desktops were missing; and on macOS no row at all"
        );
    }

    /// Every spelling round-trips, and an unrecognised row reads as `Auto` —
    /// the same polarity rule its neighbours take. Read through the parse
    /// rather than `read_settings`, which on macOS reports no row at all
    /// (pinned by [`absent_settings_read_as_their_defaults`]).
    #[tokio::test]
    async fn the_window_frame_round_trips_and_falls_back_to_auto() {
        let p = pool().await;
        for frame in [WindowFrame::Shown, WindowFrame::Hidden, WindowFrame::Auto] {
            write(&p, WINDOW_FRAME_KEY, frame.as_str()).await.unwrap();
            assert_eq!(window_frame(&p).await, frame);
        }
        for stored in ["", "Shown", "on", "1", "🪟"] {
            write(&p, WINDOW_FRAME_KEY, stored).await.unwrap();
            assert_eq!(
                window_frame(&p).await,
                WindowFrame::Auto,
                "{stored:?} is not a spelling this version writes",
            );
        }
    }

    /// The whole rule as a table: a pinned choice ignores the desktop, and
    /// `Auto` is the frame's absence exactly where a compositor tiles.
    #[test]
    fn the_frame_follows_the_desktop_only_when_asked_to() {
        for tiled in [true, false] {
            assert!(decorated(WindowFrame::Shown, tiled), "shown means shown, tiled or not");
            assert!(!decorated(WindowFrame::Hidden, tiled), "hidden means hidden, tiled or not");
        }
        assert!(!decorated(WindowFrame::Auto, true), "Hyprland closes and moves the window itself");
        assert!(decorated(WindowFrame::Auto, false), "anywhere else the frame is what you grab");
    }

    /// Every spelling the row can hold, and where each one lands.
    #[tokio::test]
    async fn the_login_choice_round_trips_and_defaults_forgivingly() {
        let p = pool().await;
        assert_eq!(start_on_login(&p).await, StartOnLogin::Open, "before anybody writes anything");

        for mode in [StartOnLogin::Off, StartOnLogin::Background, StartOnLogin::Open] {
            write(&p, AUTOSTART_KEY, mode.as_str()).await.unwrap();
            assert_eq!(start_on_login(&p).await, mode);
            assert_eq!(read_settings(&p).await.start_on_login, mode, "and the form is told");
        }

        // The boolean spelling `main` carried for a few hours, still read
        // rather than silently reverting somebody's choice.
        write(&p, AUTOSTART_KEY, "0").await.unwrap();
        assert_eq!(start_on_login(&p).await, StartOnLogin::Off);

        // Garbage, and a spelling some future version might write, both land
        // on the default rather than on the state that changes the machine.
        write(&p, AUTOSTART_KEY, "yes-please").await.unwrap();
        assert_eq!(start_on_login(&p).await, StartOnLogin::Open);
    }

    /// The two questions the mode answers, which are deliberately not the
    /// same question: `Background` still registers the entry.
    #[test]
    fn background_starts_on_login_it_just_does_not_open_a_window() {
        assert!(!StartOnLogin::Off.registers());
        assert!(StartOnLogin::Open.registers());
        assert!(
            StartOnLogin::Background.registers(),
            "background is a way of starting, not a way of not starting — reading it as \
             'no entry' would take the reminders away, which is the opposite of the ask"
        );

        assert!(StartOnLogin::Open.opens_window());
        assert!(!StartOnLogin::Background.opens_window());
    }

    /// `None` must clear a previously stored id back to the old rule — a
    /// choice that could only ever be changed, never unmade, is a trap.
    #[tokio::test]
    async fn the_default_calendar_round_trips_and_clears() {
        let p = pool().await;

        // Through the command's own body: the Tauri wrapper only adds State.
        write(&p, DEFAULT_CALENDAR_KEY, "8").await.unwrap();
        assert_eq!(read_settings(&p).await.default_calendar_id, Some(8));

        write(&p, DEFAULT_CALENDAR_KEY, "").await.unwrap();
        assert_eq!(read_settings(&p).await.default_calendar_id, None);
    }

    #[tokio::test]
    async fn the_default_event_duration_round_trips_and_refuses_zero() {
        let p = pool().await;

        let s = set_default_event_duration_impl(&p, 45).await.unwrap();
        assert_eq!(s.default_event_duration_minutes, 45);
        assert_eq!(read_settings(&p).await.default_event_duration_minutes, 45);

        assert!(set_default_event_duration_impl(&p, 0).await.is_err());
        assert_eq!(
            read_settings(&p).await.default_event_duration_minutes,
            45,
            "a refused duration must leave the stored choice alone",
        );

        for stored in ["", "0", "-1", "half an hour"] {
            write(&p, DEFAULT_EVENT_DURATION_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.default_event_duration_minutes,
                60,
                "{stored:?} is not a usable duration and must fall back",
            );
        }
    }

    /// `[]` stored is a real choice — the feature off — and must read back as
    /// itself, never as the shipped default (fallback spec §3).
    #[tokio::test]
    async fn fallback_reminders_round_trip_including_none() {
        let p = pool().await;

        let s = set_fallback_reminders_impl(&p, vec![15]).await.unwrap();
        assert_eq!(s.fallback_reminder_minutes, vec![15]);
        assert_eq!(read_settings(&p).await.fallback_reminder_minutes, vec![15]);

        let s = set_fallback_reminders_impl(&p, vec![]).await.unwrap();
        assert!(s.fallback_reminder_minutes.is_empty());
        assert!(read_settings(&p).await.fallback_reminder_minutes.is_empty());
    }

    /// The event form's own bounds, through the same function, refused with
    /// the limit named — and the stored value untouched by a refused write.
    #[tokio::test]
    async fn fallback_reminders_are_held_to_googles_bounds() {
        let p = pool().await;
        assert!(set_fallback_reminders_impl(&p, vec![40_321]).await.is_err());
        assert!(set_fallback_reminders_impl(&p, (0..6).collect()).await.is_err());
        assert!(set_fallback_reminders_impl(&p, vec![-1]).await.is_err());
        assert_eq!(
            read_settings(&p).await.fallback_reminder_minutes,
            vec![60, 10],
            "a refused write must leave the stored value alone"
        );
    }

    #[tokio::test]
    async fn an_interval_at_or_above_the_floor_is_stored_and_read_back() {
        let p = pool().await;
        let got = set_sync_interval_impl(&p, 120_000).await.unwrap();
        assert_eq!(got.sync_interval_ms, 120_000);
        assert_eq!(read_settings(&p).await.sync_interval_ms, 120_000);

        // Exactly the floor is allowed, so the refusal below cannot be
        // satisfied by a rule that refuses the boundary too.
        let at = set_sync_interval_impl(&p, crate::sync_loop::MIN_INTERVAL_MS).await.unwrap();
        assert_eq!(at.sync_interval_ms, crate::sync_loop::MIN_INTERVAL_MS);
    }

    /// **Refused, not clamped.** A value accepted and then quietly changed is
    /// worse than one turned down: the user believes they are polling every ten
    /// seconds and the app is not.
    #[tokio::test]
    async fn an_interval_below_the_floor_is_refused_and_nothing_is_stored() {
        let p = pool().await;
        set_sync_interval_impl(&p, 120_000).await.unwrap();

        let err = set_sync_interval_impl(&p, 10_000).await.unwrap_err();
        assert_eq!(err.to_string(), INTERVAL_TOO_SHORT);
        assert_eq!(
            read_settings(&p).await.sync_interval_ms,
            120_000,
            "a refused value must not half-land",
        );
        assert_eq!(crate::errors::user_facing(&err), INTERVAL_TOO_SHORT);
    }

    /// Fresh install: no second zone, which is the feature off — and `""`
    /// stored (how a clear is written) reads back as off too, not as a zone
    /// named nothing.
    #[tokio::test]
    async fn the_second_zone_is_absent_until_chosen_and_blank_reads_as_off() {
        let p = pool().await;
        assert_eq!(read_settings(&p).await.second_timezone, None);

        write(&p, SECOND_TZ_KEY, "").await.unwrap();
        assert_eq!(read_settings(&p).await.second_timezone, None);

        write(&p, SECOND_TZ_KEY, "Asia/Kolkata").await.unwrap();
        assert_eq!(read_settings(&p).await.second_timezone.as_deref(), Some("Asia/Kolkata"));
    }

    /// The validator is the command's whole opinion: a known zone passes
    /// trimmed, blank and `None` collapse to off, and an unknown name is
    /// refused with the name in the message — the same contract
    /// `set_display_timezone` gives its own input.
    #[test]
    fn a_second_zone_is_validated_against_jiffs_database() {
        assert_eq!(
            validate_second_timezone(Some("  Asia/Kolkata ".into())).unwrap().as_deref(),
            Some("Asia/Kolkata"),
        );
        assert_eq!(validate_second_timezone(Some("   ".into())).unwrap(), None);
        assert_eq!(validate_second_timezone(None).unwrap(), None);
        assert_eq!(
            validate_second_timezone(Some("Mars/Olympus_Mons".into())).unwrap_err(),
            "OmaCal does not know the time zone \"Mars/Olympus_Mons\"",
        );
    }

    /// The three grids' shared arithmetic, as a table.
    ///
    /// August 2026 opens on a Saturday, which is the month that separates all
    /// three starts: five blanks under Monday, six under Sunday, none at all
    /// under Saturday. A month opening mid-week would agree under two of them
    /// and hide a wrong subtraction.
    #[test]
    fn lead_blanks_are_counted_from_the_chosen_first_day() {
        use jiff::civil::Weekday;
        assert_eq!(WeekStart::Monday.lead_blanks(Weekday::Saturday), 5);
        assert_eq!(WeekStart::Sunday.lead_blanks(Weekday::Saturday), 6);
        assert_eq!(WeekStart::Saturday.lead_blanks(Weekday::Saturday), 0);

        // The first day of the week is always zero blanks, and the day before
        // it is always six. Anything else means the modulo wrapped wrong.
        for (start, day_before) in [
            (WeekStart::Monday, Weekday::Sunday),
            (WeekStart::Sunday, Weekday::Saturday),
            (WeekStart::Saturday, Weekday::Friday),
        ] {
            assert_eq!(start.lead_blanks(start.weekday()), 0, "{start:?}");
            assert_eq!(start.lead_blanks(day_before), 6, "{start:?}");
        }
    }

    /// Weekends land where the reader expects, and — the load-bearing half —
    /// **exactly two columns of every seven** are weekend under all three.
    /// A formula that drifted would still satisfy a single hand-written row.
    #[test]
    fn weekend_columns_follow_the_first_day() {
        // Monday start: the pair sits together, columns 5 and 6.
        assert_eq!(
            (0..7).filter(|&c| WeekStart::Monday.is_weekend_column(c)).collect::<Vec<_>>(),
            vec![5, 6],
        );
        // Sunday start splits the pair to the ends — as every Sunday-start
        // month grid in the world does.
        assert_eq!(
            (0..7).filter(|&c| WeekStart::Sunday.is_weekend_column(c)).collect::<Vec<_>>(),
            vec![0, 6],
        );
        // Saturday start puts it back together, at the front.
        assert_eq!(
            (0..7).filter(|&c| WeekStart::Saturday.is_weekend_column(c)).collect::<Vec<_>>(),
            vec![0, 1],
        );

        // Across a full 28-day Big Year row, every start marks eight columns —
        // and marks them in the same place in each of the four blocks, which
        // is the property the 28-day row exists for.
        for start in [WeekStart::Monday, WeekStart::Sunday, WeekStart::Saturday] {
            let marked: Vec<usize> = (0..28).filter(|&c| start.is_weekend_column(c)).collect();
            assert_eq!(marked.len(), 8, "{start:?} marked the wrong number of days");
            for block in 1..4 {
                for i in 0..2 {
                    assert_eq!(
                        marked[block * 2 + i],
                        marked[i] + block * 7,
                        "{start:?} drifted in block {block}",
                    );
                }
            }
        }
    }

    /// Both directions, because a format that could be turned on and not off
    /// is half a setting.
    #[tokio::test]
    async fn the_time_format_round_trips_both_ways() {
        let p = pool().await;

        write(&p, TIME_FORMAT_KEY, TimeFormat::H12.as_str()).await.unwrap();
        assert_eq!(read_settings(&p).await.time_format, TimeFormat::H12);

        write(&p, TIME_FORMAT_KEY, TimeFormat::H24.as_str()).await.unwrap();
        assert_eq!(read_settings(&p).await.time_format, TimeFormat::H24);
    }

    /// All three round-trip, and an unrecognised row reads as Monday — the
    /// same polarity rule the two settings beside this one take.
    #[tokio::test]
    async fn the_week_start_round_trips_and_falls_back_to_monday() {
        let p = pool().await;
        for start in [WeekStart::Sunday, WeekStart::Saturday, WeekStart::Monday] {
            write(&p, WEEK_START_KEY, start.as_str()).await.unwrap();
            assert_eq!(read_settings(&p).await.week_start, start);
        }
        for stored in ["", "Sunday", "sun", "wednesday", "🗓"] {
            write(&p, WEEK_START_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.week_start,
                WeekStart::Monday,
                "{stored:?} is not a day this version writes",
            );
        }
    }

    /// Both spellings round-trip, and an unrecognised row reads as Celsius —
    /// the same polarity rule [`the_week_start_round_trips_and_falls_back_to_monday`]
    /// takes.
    #[tokio::test]
    async fn the_temperature_unit_round_trips_and_falls_back_to_celsius() {
        let p = pool().await;
        for unit in [TemperatureUnit::Fahrenheit, TemperatureUnit::Celsius] {
            write(&p, TEMPERATURE_UNIT_KEY, unit.as_str()).await.unwrap();
            assert_eq!(read_settings(&p).await.temperature_unit, unit);
        }
        for stored in ["", "Fahrenheit", "F", "kelvin", "🌡"] {
            write(&p, TEMPERATURE_UNIT_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.temperature_unit,
                TemperatureUnit::Celsius,
                "{stored:?} is not a spelling this version writes",
            );
        }
    }

    #[tokio::test]
    async fn the_rolling_week_settings_round_trip_and_reject_other_day_counts() {
        let p = pool().await;

        write(&p, WEEK_STARTS_TODAY_KEY, "1").await.unwrap();
        for days in [3, 5, 7] {
            let s = set_week_view_days_impl(&p, days).await.unwrap();
            assert!(s.week_starts_today);
            assert_eq!(s.week_view_days, days);
        }

        let before = read(&p, WEEK_VIEW_DAYS_KEY).await;
        for days in [0, 1, 4, 6, 8, u8::MAX] {
            assert!(set_week_view_days_impl(&p, days).await.is_err());
            assert_eq!(read(&p, WEEK_VIEW_DAYS_KEY).await, before, "{days} changed the row");
        }

        for stored in ["", "0", "4", "8", "three", "255"] {
            write(&p, WEEK_VIEW_DAYS_KEY, stored).await.unwrap();
            assert_eq!(read_settings(&p).await.week_view_days, 7, "{stored:?} must fall back");
        }
    }

    #[tokio::test]
    async fn choosing_a_fixed_week_start_leaves_rolling_mode_atomically() {
        let p = pool().await;
        write(&p, WEEK_STARTS_TODAY_KEY, "1").await.unwrap();

        let s = set_week_start_impl(&p, WeekStart::Sunday).await.unwrap();
        assert_eq!(s.week_start, WeekStart::Sunday);
        assert!(!s.week_starts_today);
        assert_eq!(read(&p, WEEK_START_KEY).await.as_deref(), Some("sunday"));
        assert_eq!(read(&p, WEEK_STARTS_TODAY_KEY).await.as_deref(), Some("0"));
    }

    /// The polarity rule, witnessed by a value the app never writes. A row
    /// edited by hand — or written by a future version that learned a third
    /// format — must land on the clock the app has always drawn, not on the
    /// other one and not on a panic.
    #[tokio::test]
    async fn an_unrecognised_stored_format_reads_as_24h() {
        let p = pool().await;
        for stored in ["", "12", "H12", "twelve", "24h ", "🕐"] {
            write(&p, TIME_FORMAT_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.time_format,
                TimeFormat::H24,
                "{stored:?} is not 12h and must not be read as it"
            );
        }
    }

    /// The stored spelling is what the wire uses, so the row reads in
    /// `sqlite3` as the modal says. Pinned because the two are written in
    /// different places — `as_str` and a serde rename — and nothing else
    /// would notice them drifting apart.
    #[test]
    fn the_week_starts_stored_spelling_is_its_wire_spelling() {
        for w in [WeekStart::Monday, WeekStart::Sunday, WeekStart::Saturday] {
            assert_eq!(serde_json::to_string(&w).unwrap(), format!("\"{}\"", w.as_str()));
        }
    }

    #[test]
    fn the_stored_spelling_is_the_wire_spelling() {
        for f in [TimeFormat::H24, TimeFormat::H12] {
            assert_eq!(
                serde_json::to_string(&f).unwrap(),
                format!("\"{}\"", f.as_str()),
            );
        }
    }

    /// The interval the *loop* uses still clamps, because a row edited by hand
    /// with `sqlite3` never passed through the command that refuses.
    #[tokio::test]
    async fn a_hand_edited_row_below_the_floor_is_still_clamped_on_the_way_out() {
        let p = pool().await;
        write(&p, SYNC_INTERVAL_KEY, "100").await.unwrap();

        assert_eq!(read_settings(&p).await.sync_interval_ms, 100, "reported as stored");
        assert_eq!(
            crate::sync_loop::interval_ms(&p).await,
            crate::sync_loop::MIN_INTERVAL_MS,
            "and clamped where it is actually used",
        );
    }

    #[tokio::test]
    async fn notifications_can_be_turned_off_and_back_on() {
        let p = pool().await;
        write(&p, NOTIFICATIONS_KEY, "0").await.unwrap();
        assert!(!read_settings(&p).await.notifications_enabled);
        write(&p, NOTIFICATIONS_KEY, "1").await.unwrap();
        assert!(read_settings(&p).await.notifications_enabled);
    }

    /// A value nobody here wrote — hand-edited, or from a future version —
    /// reads as *on* rather than crashing or silently disabling reminders.
    #[tokio::test]
    async fn an_unrecognised_notifications_value_leaves_reminders_on() {
        let p = pool().await;
        write(&p, NOTIFICATIONS_KEY, "yes").await.unwrap();
        assert!(read_settings(&p).await.notifications_enabled);
    }

    /// **The half a UI spec cannot witness.** Flipping the toggle in one
    /// session proves a variable changed; only reading the row back out of a
    /// pool that was never told anything proves it was *stored*.
    #[tokio::test]
    async fn list_mode_is_stored_and_read_back() {
        let p = pool().await;
        write(&p, LIST_MODE_KEY, "1").await.unwrap();
        assert!(read_settings(&p).await.list_mode);
        write(&p, LIST_MODE_KEY, "0").await.unwrap();
        assert!(!read_settings(&p).await.list_mode);
    }

    /// Turning it on must not disturb the preferences stored beside it — one
    /// `settings` table, and a write that replaced the row rather than
    /// upserting its own key would take the sync interval with it.
    #[tokio::test]
    async fn storing_list_mode_leaves_its_neighbours_alone() {
        let p = pool().await;
        set_sync_interval_impl(&p, 120_000).await.unwrap();
        write(&p, NOTIFICATIONS_KEY, "0").await.unwrap();

        write(&p, LIST_MODE_KEY, "1").await.unwrap();

        let s = read_settings(&p).await;
        assert!(s.list_mode);
        assert_eq!(s.sync_interval_ms, 120_000);
        assert!(!s.notifications_enabled);
    }

    /// A value nobody here wrote reads as **off** — the grid, which is what a
    /// calendar looks like until somebody says otherwise. The opposite
    /// polarity to reminders above, and deliberately: a hand-edited row must
    /// not be able to turn the whole calendar into a list.
    #[tokio::test]
    async fn an_unrecognised_list_mode_value_leaves_the_grid() {
        let p = pool().await;
        write(&p, LIST_MODE_KEY, "yes").await.unwrap();
        assert!(!read_settings(&p).await.list_mode);
    }

    /// The tray-date switch round-trips, and an unrecognised value leaves
    /// the mark — the same polarity as `list_mode`, and for the same
    /// reason: a hand-edited row must not change the app's face.
    #[tokio::test]
    async fn the_tray_date_switch_is_stored_and_read_back() {
        let p = pool().await;
        write(&p, TRAY_DATE_KEY, "1").await.unwrap();
        assert!(read_settings(&p).await.tray_date);
        write(&p, TRAY_DATE_KEY, "0").await.unwrap();
        assert!(!read_settings(&p).await.tray_date);
        write(&p, TRAY_DATE_KEY, "yes").await.unwrap();
        assert!(!read_settings(&p).await.tray_date);
    }

    /// The hour height round-trips, and a stored value the gesture could
    /// never have produced — a hand-edited row, a future build's wider
    /// range — reads as the default rather than as a 2px or 2000px hour.
    #[tokio::test]
    async fn the_hour_height_is_stored_and_read_back_within_its_range() {
        let p = pool().await;
        write(&p, HOUR_HEIGHT_KEY, "112").await.unwrap();
        assert_eq!(read_settings(&p).await.hour_height, 112);
        for bad in ["2000", "12", "tall", ""] {
            write(&p, HOUR_HEIGHT_KEY, bad).await.unwrap();
            assert_eq!(read_settings(&p).await.hour_height, HOUR_HEIGHT_DEFAULT, "stored {bad:?}");
        }
    }

    /// Absent and empty both read as "system" — "" is how the setter clears
    /// the row, and a blank zone name is nothing anyone chose.
    #[tokio::test]
    async fn the_display_zone_defaults_to_system_and_round_trips() {
        let p = pool().await;
        assert_eq!(read_settings(&p).await.display_timezone, None);

        write(&p, DISPLAY_TZ_KEY, "Europe/Sofia").await.unwrap();
        assert_eq!(read_settings(&p).await.display_timezone.as_deref(), Some("Europe/Sofia"));

        write(&p, DISPLAY_TZ_KEY, "").await.unwrap();
        assert_eq!(read_settings(&p).await.display_timezone, None);
    }

    /// The sidecar is what `main()` reads before Tauri exists; writing Some
    /// creates it, None removes it, and removing what is absent is not an
    /// error (a fresh install clears to system).
    #[test]
    fn the_tz_sidecar_writes_and_clears() {
        let dir = std::env::temp_dir().join(format!("omacal-tz-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        write_tz_sidecar(&dir, None).unwrap(); // absent: still fine
        write_tz_sidecar(&dir, Some("Europe/Sofia")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(DISPLAY_TZ_SIDECAR)).unwrap(),
            "Europe/Sofia"
        );
        write_tz_sidecar(&dir, None).unwrap();
        assert!(!dir.join(DISPLAY_TZ_SIDECAR).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The picker's list comes from the same database the validator asks, so
    /// everything offered is acceptable — spot-checked on the two zones this
    /// project lives between.
    #[test]
    fn the_zone_list_is_sorted_and_knows_the_real_world() {
        let zones = list_timezones();
        assert!(zones.iter().any(|z| z == "Europe/Sofia"));
        assert!(zones.iter().any(|z| z == "Asia/Kolkata"));
        let mut sorted = zones.clone();
        sorted.sort();
        assert_eq!(zones, sorted);
        for z in ["Europe/Sofia", "Asia/Kolkata"] {
            assert!(jiff::tz::TimeZone::get(z).is_ok(), "the validator must accept {z}");
        }
    }
}
