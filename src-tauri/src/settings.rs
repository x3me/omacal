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
const SHOW_DATE_KEY: &str = "show_date";
const HOUR_HEIGHT_KEY: &str = "hour_height";
/// Pixels per hour in Day and Week when nobody has zoomed: the grid's own
/// 70 (see `WeekGrid.svelte`'s `.col`), and what an unusable stored value
/// falls back to.
pub const HOUR_HEIGHT_DEFAULT: i64 = 70;
/// The reach of the zoom. 30 puts a whole day in a laptop pane with the
/// hour labels still a line apart; 160 is six hours to a tall pane. Mirrored
/// in `ui/src/lib/zoom.ts`, which clamps the gesture before it ever asks —
/// this pair is the floor and ceiling the row is held to regardless.
pub const HOUR_HEIGHT_MIN: i64 = 48;
pub const HOUR_HEIGHT_MAX: i64 = 160;
const FALLBACK_KEY: &str = "fallback_reminder_minutes";
const DEFAULT_CALENDAR_KEY: &str = "default_calendar_id";
const DEFAULT_EVENT_DURATION_KEY: &str = "default_event_duration_minutes";
const INACTIVE_BACKGROUND_TRANSPARENCY_KEY: &str = "inactive_background_transparency";
const BACKGROUND_TRANSPARENCY_KEY: &str = "background_transparency";
const EVENT_TRANSPARENCY_KEY: &str = "event_transparency";
const EVENT_CORNER_STYLE_KEY: &str = "event_corner_style";
const APPEARANCE_TRANSPARENCY_SEMANTICS_KEY: &str = "appearance_transparency_semantics";
const ABSOLUTE_TRANSPARENCY_SEMANTICS: &str = "absolute-v1";
/// Omarchy's `default-opacity` rule blends every window a little — 98.5%
/// focused, 96% unfocused — this one included, and did before these controls
/// existed. On Omarchy the ranges start here, so an install that never
/// touched them looks as it always did. The compositor's share stays on top:
/// the app edits nobody's Hyprland rules (running-on-omarchy.md shows the
/// opt-out).
pub const DEFAULT_APPEARANCE_TRANSPARENCY: u8 = 4;

/// The transparency a fresh install starts at: the Omarchy baseline where
/// Omarchy blended the window already, opaque anywhere else.
///
/// No other desktop ever blended the window, so a 4 there would be a change
/// nobody asked for — and on X11 without a compositor a transparent region
/// is not blended at all but drawn black. `omarchy` is the theme directory's
/// existence, the same fact the palette reads (`theme::omarchy_theme_dir`),
/// passed in rather than read here so a test can tell both stories on one
/// host.
pub(crate) fn appearance_baseline(omarchy: bool) -> u8 {
    if omarchy {
        DEFAULT_APPEARANCE_TRANSPARENCY
    } else {
        0
    }
}
const TIME_FORMAT_KEY: &str = "time_format";
const DEFAULT_VIEW_KEY: &str = "default_view";
const DEFAULT_VIEW_FOLLOWS_LAST_KEY: &str = "default_view_follows_last";
const LAST_VIEW_KEY: &str = "last_view";
const WEEK_START_KEY: &str = "week_start";
const WEEK_STARTS_TODAY_KEY: &str = "week_starts_today";
const WEEK_VIEW_DAYS_KEY: &str = "week_view_days";
const TRAY_ICON_KEY: &str = "tray_icon";
const QUIT_ON_CLOSE_KEY: &str = "quit_on_close";
const AUTOSTART_KEY: &str = "autostart";
const WEATHER_KEY: &str = "weather_enabled";
const APPEARANCE_KEY: &str = "appearance";
const WINDOW_FRAME_KEY: &str = "window_frame";
pub(crate) const TEMPERATURE_UNIT_KEY: &str = "temperature_unit";
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

/// Which of the five view-switcher slots OmaCal opens on.
///
/// An enum for [`TimeFormat`]'s reason: the set is closed and mirrors the
/// switcher's own five buttons exactly, so [`set_default_view`] needs no
/// refusal path — a sixth value cannot be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultView {
    #[serde(rename = "day")]
    Day,
    #[serde(rename = "week")]
    Week,
    #[serde(rename = "month")]
    Month,
    #[serde(rename = "year")]
    Year,
    #[serde(rename = "bigyear")]
    BigYear,
}

impl DefaultView {
    /// The stored spelling, which is also the wire spelling — the switcher's
    /// own `View` union in `views.ts`.
    fn as_str(self) -> &'static str {
        match self {
            DefaultView::Day => "day",
            DefaultView::Week => "week",
            DefaultView::Month => "month",
            DefaultView::Year => "year",
            DefaultView::BigYear => "bigyear",
        }
    }
}

/// [`AppSettings::default_view`] and [`AppSettings::last_view`]'s shared
/// fallback: absent, garbage, or a spelling only a future version writes
/// lands on Week rather than an error.
fn parse_default_view(stored: Option<&str>) -> DefaultView {
    match stored {
        Some("day") => DefaultView::Day,
        Some("month") => DefaultView::Month,
        Some("year") => DefaultView::Year,
        Some("bigyear") => DefaultView::BigYear,
        _ => DefaultView::Week,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DateFormat { Locale, Mdy, Dmy, Iso, LongMdy, LongDmy }
impl DateFormat {
    fn as_str(self) -> &'static str { match self {
        Self::Locale => "locale", Self::Mdy => "mdy", Self::Dmy => "dmy",
        Self::Iso => "iso", Self::LongMdy => "long-mdy", Self::LongDmy => "long-dmy",
    } }
    pub fn display(self, date: jiff::civil::Date) -> String {
        match self {
            Self::Mdy => date.strftime("%m/%d/%Y").to_string(),
            Self::Dmy => date.strftime("%d/%m/%Y").to_string(),
            Self::Iso => date.to_string(),
            Self::LongDmy => format!("{} {} {}", date.day(), date.strftime("%b"), date.year()),
            _ => format!("{} {}, {}", date.strftime("%b"), date.day(), date.year()),
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

/// The corner treatment shared by every event shape in the calendar.
///
/// The UI presents exactly these two values. Keeping the choice typed here
/// also makes a hand-written invoke unable to store a spelling no renderer
/// understands; an unrecognised database row still falls back to `Rounded`
/// in [`read_settings`] for forwards compatibility and safe hand-editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventCornerStyle {
    #[serde(rename = "rounded")]
    Rounded,
    #[serde(rename = "square")]
    Square,
}

impl EventCornerStyle {
    fn as_str(self) -> &'static str {
        match self {
            EventCornerStyle::Rounded => "rounded",
            EventCornerStyle::Square => "square",
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
    pub combine_identical_events: bool,
    /// Whether the surfaces that can show today's date do (2026-09-04):
    /// the tray icon, which *becomes* the date because a tray host draws
    /// icons and nothing else, and the Omarchy bar widget, which reads it
    /// from the feed. One switch for both, because they are one idea; the
    /// widget can still opt out on its own side. Off by default: the mark
    /// is what says which app it is at a glance.
    pub show_date: bool,
    pub menubar_label: bool,
    pub menubar_join_minutes: u32,
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
    /// Calendar-canvas transparency in tenths of a percent, capped at 50. Zero is opaque;
    /// the default is [`appearance_baseline`]: Omarchy's former whole-window
    /// blend there, opaque anywhere else.
    pub background_transparency: f64,
    /// Falls back to the active value for existing installations.
    pub inactive_background_transparency: f64,
    /// Event-fill transparency in tenths of a percent, capped at 25. Text,
    /// colour spines, outlines and controls stay fully painted throughout.
    pub event_transparency: f64,
    /// Rounded preserves the shapes omacal shipped with; square removes the
    /// corner radius from every event representation, not from other UI.
    pub event_corner_style: EventCornerStyle,
    /// Whether the window can be seen through at all. The Linux window has a
    /// transparent backing store (`tauri.linux.conf.json`); the macOS one
    /// does not, since that needs Tauri's private-API feature, which this
    /// app does not enable. Where this is false the background slider would
    /// move nothing, so the modal leaves it out — `window_frame`'s `None` is
    /// the precedent: the platform reaches the form as a fact about the
    /// window, never as an OS name.
    pub transparent_window: bool,
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
    pub date_format: DateFormat,
    /// Which desktop this build is running on, so the settings copy can name
    /// it. Read-only: a fact about the host, never a stored preference.
    pub desktop: String,
    /// Which of the five view-switcher slots OmaCal opens on, when
    /// [`Self::default_view_follows_last`] is off. **Week by default** — the
    /// view every existing install already opens to; this setting only
    /// makes the choice visible and changeable rather than changing what a
    /// fresh install does.
    pub default_view: DefaultView,
    /// Whether OmaCal ignores `default_view` and opens on
    /// [`Self::last_view`] instead — `week_starts_today`'s shape for
    /// `week_start`: a flag beside the fixed choice rather than a sixth
    /// `DefaultView` variant, which would let `last_view` itself name "last".
    pub default_view_follows_last: bool,
    /// The view the switcher was most recently on, tracked on every switch
    /// regardless of `default_view_follows_last`, so turning that mode on
    /// opens on a real memory rather than a blank one. Week until anything
    /// has been recorded.
    pub last_view: DefaultView,
    pub menubar_date_format: String,
    pub menubar_date_custom: String,
    pub menubar_label_format: String,
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
    pub visible_start_hour: u8,
    pub visible_end_hour: u8,
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

/// Reads one percentage under either side of the one-time semantics change,
/// against the desktop's `baseline` ([`appearance_baseline`]).
///
/// Legacy values were extra transparency multiplied after the compositor's
/// blend. Preserve their effective alpha with
/// `baseline + round(legacy * (100 - baseline) / 100)`, which off Omarchy is
/// the value itself. New rows are already absolute. Garbage on either side
/// lands on the baseline, never on an extreme.
fn appearance_transparency(stored: Option<String>, absolute: bool, baseline: u8) -> u8 {
    let fallback = if absolute { baseline } else { 0 };
    let value = stored
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&percent| percent <= 100)
        .unwrap_or(fallback);
    if absolute {
        value
    } else {
        let remaining = 100_u16 - u16::from(baseline);
        baseline + ((u16::from(value) * remaining + 50) / 100) as u8
    }
}

fn surface_percentage(value: f64, cap: f64) -> f64 {
    (value.min(cap) * 10.0).round() / 10.0
}

fn read_surface(stored: Option<String>, absolute: bool, baseline: u8, cap: f64) -> f64 {
    if !absolute { return surface_percentage(appearance_transparency(stored, false, baseline) as f64, cap); }
    let value = stored.and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        .unwrap_or(baseline as f64);
    surface_percentage(value, cap)
}

/// The settings as stored, with defaults for anything absent.
///
/// Absent is the ordinary case on a fresh install and is not an error:
/// nothing writes these until the user opens the modal.
pub async fn read_settings(pool: &SqlitePool) -> AppSettings {
    let baseline = appearance_baseline(crate::theme::omarchy_theme_dir().is_some());
    read_settings_with(pool, baseline).await
}

/// [`read_settings`] with the appearance baseline supplied, so a test can
/// tell the Omarchy story and the other one on whichever host runs it.
pub(crate) async fn read_settings_with(pool: &SqlitePool, baseline: u8) -> AppSettings {
    let (visible_start_hour, visible_end_hour) = visible_hours(pool).await;
    let absolute_transparency = read(pool, APPEARANCE_TRANSPARENCY_SEMANTICS_KEY)
        .await
        .as_deref()
        == Some(ABSOLUTE_TRANSPARENCY_SEMANTICS);
    let background_transparency = read_surface(
        read(pool, BACKGROUND_TRANSPARENCY_KEY).await, absolute_transparency, baseline, 50.0,
    );
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
        combine_identical_events: combine_identical_events(pool).await,
        // The mark unless the row says otherwise, for `list_mode`'s reason:
        // a hand-edited value must land on what the app has always drawn.
        show_date: read(pool, SHOW_DATE_KEY).await.map(|v| v == "1").unwrap_or(false),
        menubar_label: read(pool, "menubar_label").await.as_deref() != Some("0"),
        menubar_join_minutes: read(pool, "menubar_join_minutes").await
            .and_then(|v| v.parse().ok()).filter(|v| *v <= 60).unwrap_or(5),
        // **Clamped, not discarded.** A number outside the range is still an
        // answer to "how tall do you like your hours" — when the floor rose
        // from 30 to 48, discarding sent everyone who had zoomed out past it
        // back to the default 70, which is further from what they chose than
        // the floor is. Only an unparseable or absent value takes the
        // default. (A hand-edited value still lands on something the app
        // draws, which is what the rule above this one is for.)
        hour_height: read(pool, HOUR_HEIGHT_KEY)
            .await
            .and_then(|v| v.parse::<i64>().ok())
            .map(|px| px.clamp(HOUR_HEIGHT_MIN, HOUR_HEIGHT_MAX))
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
        // These are absolute percentages now. Before `absolute-v1`, stored
        // values meant "extra alpha after Omarchy's 4% baseline"; lazily
        // translate those rows so an old 0 opens at 4 instead of changing the
        // user's appearance. The next write stores the marker atomically.
        background_transparency,
        inactive_background_transparency: read(pool, INACTIVE_BACKGROUND_TRANSPARENCY_KEY).await
            .and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            .map(|v| surface_percentage(v, 50.0)).unwrap_or(background_transparency),
        event_transparency: read_surface(
            read(pool, EVENT_TRANSPARENCY_KEY).await, absolute_transparency, baseline, 25.0,
        ),
        transparent_window: cfg!(target_os = "linux"),
        event_corner_style: match read(pool, EVENT_CORNER_STYLE_KEY).await.as_deref() {
            Some("square") => EventCornerStyle::Square,
            _ => EventCornerStyle::Rounded,
        },
        // `== "12h"` rather than `!= "24h"`, the same polarity `list_mode`
        // takes and for the same reason: absent, garbage, and a value written
        // by some future version all land on the format the app has always
        // drawn, rather than on the one nobody asked for.
        menubar_date_format: match read(pool, "menubar_date_format").await { Some(v) => v, None => if read(pool, SHOW_DATE_KEY).await.is_some() { "custom".into() } else { "general".into() } },
        menubar_label_format: read(pool, "menubar_label_format").await.unwrap_or_else(|| DEFAULT_MENU_LABEL.into()),
        menubar_date_custom: read(pool, "menubar_date_custom").await.unwrap_or_else(|| "%-d".into()),
        date_format: read(pool, "date_format").await.and_then(|v| serde_json::from_value(serde_json::Value::String(v)).ok()).unwrap_or(DateFormat::Locale),
        desktop: if cfg!(target_os = "macos") { "macos" } else if crate::theme::omarchy_theme_dir().is_some() { "omarchy" } else { "linux" }.into(),
        time_format: read(pool, TIME_FORMAT_KEY)
            .await
            .map(|v| if v == "12h" { TimeFormat::H12 } else { TimeFormat::H24 })
            .unwrap_or(TimeFormat::H24),
        // Week is the view every existing install already opens on; absent,
        // garbage, or a spelling only a future version writes all land there
        // rather than on a switcher slot nobody chose.
        default_view: parse_default_view(read(pool, DEFAULT_VIEW_KEY).await.as_deref()),
        // Opt-in, `week_starts_today`'s reason and polarity: absent, garbage
        // and a future spelling all keep the fixed `default_view` in force.
        default_view_follows_last: read(pool, DEFAULT_VIEW_FOLLOWS_LAST_KEY)
            .await
            .map(|v| v == "1")
            .unwrap_or(false),
        // Same fallback as `default_view`, for the same reason: nothing has
        // been recorded yet reads as the view every install already opens
        // on, not as an error.
        last_view: parse_default_view(read(pool, LAST_VIEW_KEY).await.as_deref()),
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
        visible_start_hour,
        visible_end_hour,
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

pub const TRANSPARENCY_OUT_OF_RANGE: &str =
    "background transparency must be between 0 and 50 percent; event transparency between 0 and 25 percent";

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
    refresh_menu_surfaces(&app, &state).await;
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

/// Stores appearance choices as one decision. The sliders and
/// the corner picker share one preview, so a crash or database failure must
/// not leave half of that preview persisted for the next launch.
#[tauri::command]
pub async fn set_appearance_preferences(
    state: tauri::State<'_, AppState>,
    background_transparency: f64,
    event_transparency: f64,
    event_corner_style: EventCornerStyle,
    inactive_background_transparency: Option<f64>,
) -> Result<AppSettings, String> {
    set_appearance_preferences_impl(
        &state.pool,
        background_transparency,
        event_transparency,
        event_corner_style,
        inactive_background_transparency,
    )
    .await
    .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_appearance_preferences_impl(
    pool: &SqlitePool,
    background_transparency: f64,
    event_transparency: f64,
    event_corner_style: EventCornerStyle,
    inactive_background_transparency: Option<f64>,
) -> anyhow::Result<AppSettings> {
    let inactive = inactive_background_transparency.unwrap_or(background_transparency);
    if !background_transparency.is_finite() || !(0.0..=50.0).contains(&background_transparency)
        || !inactive.is_finite() || !(0.0..=50.0).contains(&inactive) || !event_transparency.is_finite() || !(0.0..=25.0).contains(&event_transparency) {
        anyhow::bail!(TRANSPARENCY_OUT_OF_RANGE);
    }

    let values = [
        (BACKGROUND_TRANSPARENCY_KEY, surface_percentage(background_transparency, 50.0).to_string()),
        (INACTIVE_BACKGROUND_TRANSPARENCY_KEY, surface_percentage(inactive, 50.0).to_string()),
        (EVENT_TRANSPARENCY_KEY, surface_percentage(event_transparency, 25.0).to_string()),
        (EVENT_CORNER_STYLE_KEY, event_corner_style.as_str().to_string()),
        (
            APPEARANCE_TRANSPARENCY_SEMANTICS_KEY,
            ABSOLUTE_TRANSPARENCY_SEMANTICS.to_string(),
        ),
    ];
    let mut tx = pool.begin().await?;
    for (key, value) in values {
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

/// Stores whether today's date is shown. Nothing to refuse, as with the
/// other booleans; the tray is redressed on the spot and the widget's feed
/// rewritten, so the choice shows on both surfaces without waiting for
/// either one's tick.
#[tauri::command]
pub async fn set_show_date(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    if read(&state.pool, "menubar_date_format").await.is_none() {
        let selected = read_settings(&state.pool).await.menubar_date_format;
        write(&state.pool, "menubar_date_format", &selected).await.map_err(|e| e.to_string())?;
    }
    write(&state.pool, SHOW_DATE_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

pub(crate) const DEFAULT_MENU_LABEL: &str = "{title} @ {time}  {countdown}";
pub(crate) fn format_menu_label(template: &str, values: &[(&str, &str)]) -> Result<String, String> {
    if template.trim().is_empty() || template.len() > 256 || template.chars().any(char::is_control) {
        return Err("Use a meeting format between 1 and 256 characters on one line.".into());
    }
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let end = tail.find('}').ok_or("Close each placeholder with }." )?;
        let key = &tail[..end];
        let value = values.iter().find(|(name, _)| *name == key).ok_or("Use {title}, {time}, {end_time}, {countdown}, or {calendar}.")?;
        out.extend(value.1.chars().take(256));
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out.chars().take(256).collect())
}

#[tauri::command]
pub async fn set_menubar_label_format(app: tauri::AppHandle, state: tauri::State<'_, AppState>, template: String) -> Result<AppSettings, String> {
    format_menu_label(&template, &[("title", ""), ("time", ""), ("end_time", ""), ("countdown", ""), ("calendar", "")])?;
    write(&state.pool, "menubar_label_format", &template).await.map_err(|e| e.to_string())?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

/// Custom text is formatted by Jiff, with a bounded writer so padding cannot
/// allocate an oversized bar label. Only civil-date fields are available.
pub(crate) fn custom_menu_date(pattern: &str, date: jiff::civil::Date) -> Result<String, String> {
    if pattern.is_empty() || pattern.len() > 128 || pattern.chars().any(char::is_control) {
        return Err("Use a date format between 1 and 128 characters.".into());
    }
    struct Label(String);
    impl std::fmt::Write for Label {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            if self.0.len() + text.len() > 128 { return Err(std::fmt::Error); }
            self.0.push_str(text); Ok(())
        }
    }
    let mut label = Label(String::new());
    jiff::fmt::strtime::BrokenDownTime::from(date).format(pattern, jiff::fmt::StdFmtWrite(&mut label))
        .map_err(|_| "Invalid date format. Check the formatting guide; time fields are not supported.".to_string())?;
    if label.0.trim().is_empty() || label.0.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return Err("The date must be visible text on one line.".into());
    }
    Ok(label.0)
}

pub(crate) fn menu_date(settings: &AppSettings, date: jiff::civil::Date) -> String {
    match settings.menubar_date_format.as_str() {
        "general" => settings.date_format.display(date),
        "custom" => custom_menu_date(&settings.menubar_date_custom, date).unwrap_or_else(|_| date.day().to_string()),
        value => serde_json::from_value::<DateFormat>(serde_json::Value::String(value.into()))
            .unwrap_or(settings.date_format).display(date),
    }
}

async fn store_menu_date(pool: &SqlitePool, format: &str, custom: &str) -> Result<(), String> {
    if !matches!(format, "general" | "custom") && serde_json::from_value::<DateFormat>(serde_json::Value::String(format.into())).is_err() {
        return Err("Choose a menu-bar date format.".into());
    }
    custom_menu_date(custom, jiff::civil::date(2026, 9, 7))?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (key, value) in [("menubar_date_format", format), ("menubar_date_custom", custom)] {
        sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(key).bind(value).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_menubar_date_format(app: tauri::AppHandle, state: tauri::State<'_, AppState>, format: String, custom: String) -> Result<AppSettings, String> {
    store_menu_date(&state.pool, &format, &custom).await?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

#[tauri::command]
pub fn open_date_format_guide() -> Result<(), String> {
    crate::browser::open_external("https://docs.rs/jiff/latest/jiff/fmt/strtime/index.html#conversion-specifications")
        .map_err(|e| crate::errors::user_facing(&e.into()))
}

/// Persist the snapshot before notifying either popup; neither waits for a poll.
pub(crate) async fn refresh_menu_surfaces(app: &tauri::AppHandle, state: &AppState) {
    use tauri::Emitter;
    crate::upcoming::refresh(&state.pool, state.demo).await;
    crate::tray::refresh(app);
    let _ = app.emit("menubar-changed", ());
    #[cfg(target_os = "linux")]
    if !state.demo && std::path::Path::new("/usr/share/omarchy/shell/shell.qml").is_file() {
        // Fixed IPC arguments, no output retained, and no orphaned process
        // if the shell is unavailable. The widget keeps its polling fallback.
        if let Ok(mut child) = tokio::process::Command::new("/usr/bin/qs")
            .args(["ipc", "-n", "-p", "/usr/share/omarchy/shell", "call", "--", "omacal.upcoming", "refresh"])
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).kill_on_drop(true).spawn() {
            if tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
        }
    }
}

/// Shared preferences for the Omarchy widget and macOS menu bar popup.
#[tauri::command]
pub async fn set_menubar_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    label: bool,
    join_minutes: u32,
) -> Result<AppSettings, String> {
    store_menubar_preferences(&state.pool, label, join_minutes).await?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

async fn store_menubar_preferences(pool: &SqlitePool, label: bool, join_minutes: u32) -> Result<(), String> {
    if join_minutes > 60 { return Err("Choose a Join window from 0 to 60 minutes.".into()); }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (key, value) in [
        ("menubar_label", if label { "1".into() } else { "0".into() }),
        ("menubar_join_minutes", join_minutes.to_string()),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key).bind(value).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
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

#[tauri::command]
pub async fn set_date_format(app: tauri::AppHandle, state: tauri::State<'_, AppState>, format: DateFormat) -> Result<AppSettings, String> {
    write(&state.pool, "date_format", format.as_str()).await.map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

/// Stores the clock format. Like [`set_list_mode`] nothing is refused, and
/// here the *type* is the reason rather than the triviality of a boolean:
/// [`TimeFormat`] has no third variant for a caller to send.
#[tauri::command]
pub async fn set_time_format(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    format: TimeFormat,
) -> Result<AppSettings, String> {
    write(&state.pool, TIME_FORMAT_KEY, format.as_str())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
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
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    days: u8,
) -> Result<AppSettings, String> {
    let result = set_week_view_days_impl(&state.pool, days)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(result)
}

async fn set_week_view_days_impl(pool: &SqlitePool, days: u8) -> anyhow::Result<AppSettings> {
    if !matches!(days, 3 | 5 | 7) {
        anyhow::bail!("the rolling week can show 3, 5, or 7 days");
    }
    write(pool, WEEK_VIEW_DAYS_KEY, &days.to_string()).await?;
    Ok(read_settings(pool).await)
}

async fn visible_hours(pool: &SqlitePool) -> (u8, u8) {
    read(pool, "visible_hours").await.and_then(|value| {
        let (start, end) = value.split_once(',')?;
        let (start, end) = (start.parse::<u8>().ok()?, end.parse::<u8>().ok()?);
        (start < end && end <= 24).then_some((start, end))
    }).unwrap_or((0, 24))
}

#[tauri::command]
pub async fn set_visible_hours(app: tauri::AppHandle, state: tauri::State<'_, AppState>, start: u8, end: u8) -> Result<AppSettings, String> {
    if start >= end || end > 24 { return Err("Start time must be before end time.".into()); }
    write(&state.pool, "visible_hours", &format!("{start},{end}")).await.map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

/// Display grouping is opt-in and never changes the stored calendar events.
pub(crate) async fn combine_identical_events(pool: &SqlitePool) -> bool {
    read(pool, "combine_identical_events").await.as_deref() == Some("1")
}

#[tauri::command]
pub async fn set_combine_identical_events(
    app: tauri::AppHandle, state: tauri::State<'_, AppState>, on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, "combine_identical_events", if on { "1" } else { "0" })
        .await.map_err(|e| crate::errors::user_facing(&e))?;
    refresh_menu_surfaces(&app, &state).await;
    Ok(read_settings(&state.pool).await)
}

/// Stores a fixed view and leaves "Last view" mode — [`set_week_start`]'s
/// shape, atomic for the same reason: a crash between two writes must never
/// leave the mode on with a choice the user just picked to replace it.
#[tauri::command]
pub async fn set_default_view(
    state: tauri::State<'_, AppState>,
    view: DefaultView,
) -> Result<AppSettings, String> {
    set_default_view_impl(&state.pool, view)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_default_view_impl(pool: &SqlitePool, view: DefaultView) -> anyhow::Result<AppSettings> {
    let mut tx = pool.begin().await?;
    for (key, value) in [(DEFAULT_VIEW_KEY, view.as_str()), (DEFAULT_VIEW_FOLLOWS_LAST_KEY, "0")] {
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

/// Turns "Last view" mode on or off — [`set_week_starts_today`]'s shape:
/// `default_view` is set aside, not discarded, and is what's used again once
/// this is turned back off.
#[tauri::command]
pub async fn set_default_view_follows_last(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<AppSettings, String> {
    write(&state.pool, DEFAULT_VIEW_FOLLOWS_LAST_KEY, if on { "1" } else { "0" })
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(read_settings(&state.pool).await)
}

/// Stores the view the switcher was most recently on, called on every
/// switch regardless of mode — see [`AppSettings::last_view`]. Nothing to
/// refuse, [`set_default_view`]'s reason.
#[tauri::command]
pub async fn set_last_view(
    state: tauri::State<'_, AppState>,
    view: DefaultView,
) -> Result<AppSettings, String> {
    set_last_view_impl(&state.pool, view)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn set_last_view_impl(pool: &SqlitePool, view: DefaultView) -> anyhow::Result<AppSettings> {
    write(pool, LAST_VIEW_KEY, view.as_str()).await?;
    Ok(read_settings(pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        omacal_store::connect_memory().await.unwrap()
    }

    #[tokio::test]
    async fn visible_hours_default_and_reject_invalid_stored_ranges() {
        let p = pool().await;
        assert_eq!(visible_hours(&p).await, (0, 24));
        write(&p, "visible_hours", "5,23").await.unwrap();
        let settings = read_settings(&p).await;
        assert_eq!((settings.visible_start_hour, settings.visible_end_hour), (5, 23));
        for value in ["5,5", "23,5", "0,25", "-1,23", "oops"] {
            write(&p, "visible_hours", value).await.unwrap();
            assert_eq!(visible_hours(&p).await, (0, 24));
        }
    }

    #[tokio::test]
    async fn menubar_preferences_persist_and_refuse_invalid_windows_atomically() {
        let p = pool().await;
        let initial = read_settings(&p).await;
        assert!(initial.menubar_label);
        assert_eq!(initial.menubar_join_minutes, 5);
        store_menubar_preferences(&p, false, 0).await.unwrap();
        let stored = read_settings(&p).await;
        assert!(!stored.menubar_label);
        assert_eq!(stored.menubar_join_minutes, 0);
        assert!(store_menubar_preferences(&p, true, 61).await.is_err());
        assert!(!read_settings(&p).await.menubar_label);
        store_menubar_preferences(&p, true, 60).await.unwrap();
        assert_eq!(read_settings(&p).await.menubar_join_minutes, 60);
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
        assert!(!s.show_date, "a fresh install wears the mark, not the date");
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
        let baseline = appearance_baseline(crate::theme::omarchy_theme_dir().is_some());
        assert_eq!(
            s.background_transparency,
            baseline as f64,
            "a fresh install starts at this desktop's baseline: 4 where Omarchy blended the window, 0 elsewhere",
        );
        assert_eq!(
            s.event_transparency,
            baseline as f64,
            "event alpha starts at the same baseline",
        );
        assert_eq!(
            s.event_corner_style,
            EventCornerStyle::Rounded,
            "existing event shapes stay rounded",
        );
        assert_eq!(
            s.time_format,
            TimeFormat::H24,
            "the clock the app has always drawn, so no installed copy changes under its user"
        );
        assert_eq!(
            s.default_view,
            DefaultView::Week,
            "the view every existing install already opens on"
        );
        assert!(!s.default_view_follows_last, "fixed by default, not \"last\"");
        assert_eq!(s.last_view, DefaultView::Week, "nothing recorded yet");
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

    #[tokio::test]
    async fn appearance_round_trips_as_one_choice_and_rejects_bad_percentages() {
        let p = pool().await;

        let s = set_appearance_preferences_impl(&p, 35.0, 20.0, EventCornerStyle::Square, None)
            .await
            .unwrap();
        assert_eq!(s.background_transparency, 35.0);
        assert_eq!(s.event_transparency, 20.0);
        assert_eq!(s.event_corner_style, EventCornerStyle::Square);

        for (background, events) in [(50.1, 20.0), (35.0, 25.1)] {
            let err = set_appearance_preferences_impl(&p, background, events, EventCornerStyle::Rounded, None)
                .await
                .unwrap_err();
            assert_eq!(err.to_string(), TRANSPARENCY_OUT_OF_RANGE);
            assert_eq!(crate::errors::user_facing(&err), TRANSPARENCY_OUT_OF_RANGE);
            let unchanged = read_settings(&p).await;
            assert_eq!(unchanged.background_transparency, 35.0);
            assert_eq!(unchanged.event_transparency, 20.0);
            assert_eq!(unchanged.event_corner_style, EventCornerStyle::Square);
        }
    }

    #[tokio::test]
    async fn inactive_transparency_falls_back_then_persists_independently() {
        let p = omacal_store::connect_memory().await.unwrap();
        write(&p, BACKGROUND_TRANSPARENCY_KEY, "35").await.unwrap();
        let legacy = read_settings(&p).await;
        assert_eq!(legacy.inactive_background_transparency, legacy.background_transparency);
        write(&p, APPEARANCE_TRANSPARENCY_SEMANTICS_KEY, ABSOLUTE_TRANSPARENCY_SEMANTICS).await.unwrap();
        write(&p, BACKGROUND_TRANSPARENCY_KEY, "80").await.unwrap();
        write(&p, INACTIVE_BACKGROUND_TRANSPARENCY_KEY, "90").await.unwrap();
        let capped = read_settings(&p).await;
        assert_eq!((capped.background_transparency, capped.inactive_background_transparency), (50.0, 50.0));
        write(&p, EVENT_TRANSPARENCY_KEY, "90").await.unwrap();
        assert_eq!(read_settings(&p).await.event_transparency, 25.0);
        let saved = set_appearance_preferences_impl(&p, 1.5, 20.5, EventCornerStyle::Rounded, Some(4.1)).await.unwrap();
        assert_eq!((saved.background_transparency, saved.inactive_background_transparency), (1.5, 4.1));
        assert_eq!(read_settings(&p).await.inactive_background_transparency, 4.1);
        assert!(set_appearance_preferences_impl(&p, 10.0, 20.0, EventCornerStyle::Square, Some(50.1)).await.is_err());
        let unchanged = read_settings(&p).await;
        assert_eq!((unchanged.background_transparency, unchanged.inactive_background_transparency, unchanged.event_transparency), (1.5, 4.1, 20.5));
    }

    #[tokio::test]
    async fn legacy_extra_transparency_is_migrated_to_the_same_absolute_alpha() {
        let p = pool().await;

        // No semantics row is the additive version. On Omarchy its 0 was
        // really the compositor's 4%; 50 was 50% alpha multiplied by 96%, or
        // 52% total, now capped at 25%.
        write(&p, BACKGROUND_TRANSPARENCY_KEY, "0").await.unwrap();
        write(&p, EVENT_TRANSPARENCY_KEY, "50").await.unwrap();
        let legacy = read_settings_with(&p, DEFAULT_APPEARANCE_TRANSPARENCY).await;
        assert_eq!(legacy.background_transparency, 4.0);
        assert_eq!(legacy.event_transparency, 25.0);
        // Off Omarchy nothing multiplied the window, so a legacy value was
        // already the whole alpha, still subject to the 25% event cap.
        let plain = read_settings_with(&p, 0).await;
        assert_eq!(plain.background_transparency, 0.0);
        assert_eq!(plain.event_transparency, 25.0);

        // Any new write marks the whole tuple absolute, including a real 0.
        let absolute = set_appearance_preferences_impl(&p, 0.0, 25.0, EventCornerStyle::Rounded, None)
            .await
            .unwrap();
        assert_eq!(absolute.background_transparency, 0.0);
        assert_eq!(absolute.event_transparency, 25.0);
        assert_eq!(
            read(&p, APPEARANCE_TRANSPARENCY_SEMANTICS_KEY).await.as_deref(),
            Some(ABSOLUTE_TRANSPARENCY_SEMANTICS),
        );
    }

    #[tokio::test]
    async fn malformed_appearance_rows_fall_back_to_the_existing_look() {
        let p = pool().await;

        for stored in ["", "101", "-1", "half", "255"] {
            write(&p, BACKGROUND_TRANSPARENCY_KEY, stored).await.unwrap();
            write(&p, EVENT_TRANSPARENCY_KEY, stored).await.unwrap();
            for baseline in [0, DEFAULT_APPEARANCE_TRANSPARENCY] {
                let s = read_settings_with(&p, baseline).await;
                assert_eq!(
                    s.background_transparency,
                    baseline as f64,
                    "{stored:?} changed the canvas at baseline {baseline}",
                );
                assert_eq!(
                    s.event_transparency,
                    baseline as f64,
                    "{stored:?} changed event fills at baseline {baseline}",
                );
            }
        }

        write(&p, EVENT_CORNER_STYLE_KEY, "square").await.unwrap();
        assert_eq!(read_settings(&p).await.event_corner_style, EventCornerStyle::Square);
        for stored in ["", "Square", "round", "future-style"] {
            write(&p, EVENT_CORNER_STYLE_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.event_corner_style,
                EventCornerStyle::Rounded,
                "{stored:?} is not a style this version writes",
            );
        }
    }

    #[tokio::test]
    async fn a_fresh_install_starts_opaque_unless_omarchy_blended_the_window_already() {
        let p = pool().await;
        let plain = read_settings_with(&p, appearance_baseline(false)).await;
        assert_eq!((plain.background_transparency, plain.event_transparency), (0.0, 0.0));
        let omarchy = read_settings_with(&p, appearance_baseline(true)).await;
        assert_eq!(
            (omarchy.background_transparency, omarchy.event_transparency),
            (DEFAULT_APPEARANCE_TRANSPARENCY as f64, DEFAULT_APPEARANCE_TRANSPARENCY as f64),
        );
        // The real read passes the host's own answer, whichever it is.
        let detected = appearance_baseline(crate::theme::omarchy_theme_dir().is_some());
        assert_eq!(read_settings(&p).await.background_transparency, detected as f64);
    }

    #[test]
    fn the_corner_styles_stored_spelling_is_its_wire_spelling() {
        for style in [EventCornerStyle::Rounded, EventCornerStyle::Square] {
            assert_eq!(
                serde_json::to_string(&style).unwrap(),
                format!("\"{}\"", style.as_str()),
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

    #[tokio::test]
    async fn menu_date_formats_preserve_legacy_and_follow_general_or_custom() {
        let p = omacal_store::connect_memory().await.unwrap();
        let date = jiff::civil::date(2026, 9, 7);
        assert_eq!(read_settings(&p).await.menubar_date_format, "general");
        write(&p, SHOW_DATE_KEY, "1").await.unwrap();
        let legacy = read_settings(&p).await;
        assert_eq!(legacy.menubar_date_format, "custom");
        assert_eq!(menu_date(&legacy, date), "7");
        store_menu_date(&p, "general", "%-d").await.unwrap();
        write(&p, "date_format", "dmy").await.unwrap();
        assert_eq!(menu_date(&read_settings(&p).await, date), "07/09/2026");
        store_menu_date(&p, "iso", "%-d").await.unwrap();
        assert_eq!(menu_date(&read_settings(&p).await, date), "2026-09-07");
        store_menu_date(&p, "custom", "%a %b %-d").await.unwrap();
        assert_eq!(menu_date(&read_settings(&p).await, date), "Mon Sep 7");
        assert_eq!(custom_menu_date("%d", date).unwrap(), "07");
        for invalid in ["", "%", "%Q", "%H", "%n", "%1000000000d"] {
            assert!(store_menu_date(&p, "custom", invalid).await.is_err(), "{invalid}");
        }
        assert_eq!(menu_date(&read_settings(&p).await, date), "Mon Sep 7");
        assert_eq!(crate::upcoming::current(&p, 1_788_804_000_000).await.unwrap().today.unwrap().label,
            menu_date(&read_settings(&p).await, jiff::Timestamp::from_millisecond(1_788_804_000_000).unwrap().to_zoned(jiff::tz::TimeZone::system()).date()));
    }

    #[test]
    fn meeting_templates_reorder_omit_and_do_not_expand_event_text() {
        let values = [("title", "Design {time}"), ("time", "13:30"), ("end_time", "14:00"), ("countdown", "in 5m"), ("calendar", "Work")];
        assert_eq!(format_menu_label("{countdown} · {title} ({calendar})", &values).unwrap(), "in 5m · Design {time} (Work)");
        assert_eq!(format_menu_label("{time}–{end_time}", &values).unwrap(), "13:30–14:00");
        for invalid in ["", "{bad}", "{title", "a\nb"] { assert!(format_menu_label(invalid, &values).is_err()); }
    }

    #[tokio::test]
    async fn date_formats_round_trip_and_render_unambiguous_dates() {
        let p = pool().await;
        let date: jiff::civil::Date = "2026-09-07".parse().unwrap();
        for (format, expected) in [(DateFormat::Mdy, "09/07/2026"), (DateFormat::Dmy, "07/09/2026"),
            (DateFormat::Iso, "2026-09-07"), (DateFormat::LongMdy, "Sep 7, 2026"), (DateFormat::LongDmy, "7 Sep 2026")] {
            write(&p, "date_format", format.as_str()).await.unwrap();
            assert_eq!(read_settings(&p).await.date_format, format);
            assert_eq!(format.display(date), expected);
        }
        write(&p, "date_format", "garbage").await.unwrap();
        assert_eq!(read_settings(&p).await.date_format, DateFormat::Locale);
    }

    /// All five round-trip, and an unrecognised row reads as Week — the same
    /// polarity rule [`the_week_start_round_trips_and_falls_back_to_monday`]
    /// takes, and the same view every install already opened on before this
    /// setting existed.
    #[tokio::test]
    async fn the_default_view_round_trips_and_falls_back_to_week() {
        let p = pool().await;
        for view in [
            DefaultView::Day, DefaultView::Month, DefaultView::Year,
            DefaultView::BigYear, DefaultView::Week,
        ] {
            write(&p, DEFAULT_VIEW_KEY, view.as_str()).await.unwrap();
            assert_eq!(read_settings(&p).await.default_view, view);
        }
        for stored in ["", "Week", "WEEK", "quarter", "🗓"] {
            write(&p, DEFAULT_VIEW_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.default_view,
                DefaultView::Week,
                "{stored:?} is not a spelling this version writes",
            );
        }
    }

    /// `last_view` takes the same five spellings and the same fallback as
    /// `default_view` — proven separately because the two rows are read by
    /// the same helper and a copy-paste could point one at the other's key
    /// without either test noticing.
    #[tokio::test]
    async fn the_last_view_round_trips_and_falls_back_to_week() {
        let p = pool().await;
        for view in [DefaultView::Month, DefaultView::BigYear, DefaultView::Day] {
            let s = set_last_view_impl(&p, view).await.unwrap();
            assert_eq!(s.last_view, view);
            assert_eq!(read_settings(&p).await.last_view, view);
        }
        for stored in ["", "Week", "garbage"] {
            write(&p, LAST_VIEW_KEY, stored).await.unwrap();
            assert_eq!(
                read_settings(&p).await.last_view,
                DefaultView::Week,
                "{stored:?} is not a spelling this version writes",
            );
        }
    }

    /// [`choosing_a_fixed_week_start_leaves_rolling_mode_atomically`]'s exact
    /// shape: picking a fixed default view while "Last view" mode is on
    /// turns that mode off in the same write, and turning it back on
    /// restores the `last_view` recorded independently of either.
    #[tokio::test]
    async fn choosing_a_fixed_default_view_leaves_last_view_mode_atomically() {
        let p = pool().await;
        write(&p, DEFAULT_VIEW_FOLLOWS_LAST_KEY, "1").await.unwrap();
        set_last_view_impl(&p, DefaultView::Year).await.unwrap();

        let s = set_default_view_impl(&p, DefaultView::Month).await.unwrap();
        assert_eq!(s.default_view, DefaultView::Month);
        assert!(!s.default_view_follows_last);
        assert_eq!(read(&p, DEFAULT_VIEW_KEY).await.as_deref(), Some("month"));
        assert_eq!(read(&p, DEFAULT_VIEW_FOLLOWS_LAST_KEY).await.as_deref(), Some("0"));

        // Turning the mode back on does not disturb the fixed choice or the
        // memory of what "last" meant.
        write(&p, DEFAULT_VIEW_FOLLOWS_LAST_KEY, "1").await.unwrap();
        let s = read_settings(&p).await;
        assert!(s.default_view_follows_last);
        assert_eq!(s.default_view, DefaultView::Month);
        assert_eq!(s.last_view, DefaultView::Year);
    }

    /// Both directions, `the_time_format_round_trips_both_ways`'s reason.
    #[tokio::test]
    async fn default_view_follows_last_round_trips_both_ways() {
        let p = pool().await;
        assert!(!read_settings(&p).await.default_view_follows_last, "off until chosen");

        write(&p, DEFAULT_VIEW_FOLLOWS_LAST_KEY, "1").await.unwrap();
        assert!(read_settings(&p).await.default_view_follows_last);

        write(&p, DEFAULT_VIEW_FOLLOWS_LAST_KEY, "0").await.unwrap();
        assert!(!read_settings(&p).await.default_view_follows_last);
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

    /// The show-date switch round-trips, and an unrecognised value leaves
    /// the mark — the same polarity as `list_mode`, and for the same
    /// reason: a hand-edited row must not change the app's face.
    #[tokio::test]
    async fn the_show_date_switch_is_stored_and_read_back() {
        let p = pool().await;
        write(&p, SHOW_DATE_KEY, "1").await.unwrap();
        assert!(read_settings(&p).await.show_date);
        write(&p, SHOW_DATE_KEY, "0").await.unwrap();
        assert!(!read_settings(&p).await.show_date);
        write(&p, SHOW_DATE_KEY, "yes").await.unwrap();
        assert!(!read_settings(&p).await.show_date);
    }

    /// The hour height round-trips, and a stored value the gesture could
    /// never have produced — a hand-edited row, a future build's wider
    /// range — reads as the default rather than as a 2px or 2000px hour.
    #[tokio::test]
    async fn the_hour_height_is_stored_and_read_back_within_its_range() {
        let p = pool().await;
        write(&p, HOUR_HEIGHT_KEY, "112").await.unwrap();
        assert_eq!(read_settings(&p).await.hour_height, 112);

        // **A number out of range is clamped, not thrown away.** It is still
        // an answer to "how tall do you like your hours", and the nearest
        // height the app will draw is closer to it than the default is.
        for (stored, want) in [("2000", HOUR_HEIGHT_MAX), ("12", HOUR_HEIGHT_MIN)] {
            write(&p, HOUR_HEIGHT_KEY, stored).await.unwrap();
            assert_eq!(read_settings(&p).await.hour_height, want, "stored {stored:?}");
        }

        // The case this rule exists for: the floor rose from 30 to 48 when
        // half-hour events turned out to be unreadable below it. Anyone who
        // had zoomed out past the new floor lands **on** it rather than
        // snapping back to a default they never chose.
        write(&p, HOUR_HEIGHT_KEY, "30").await.unwrap();
        assert_eq!(read_settings(&p).await.hour_height, HOUR_HEIGHT_MIN);
        assert_ne!(read_settings(&p).await.hour_height, HOUR_HEIGHT_DEFAULT);

        // Only something that is not a height at all takes the default.
        for bad in ["tall", ""] {
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
