//! The tray, and what closing the window means.
//!
//! Split the same way the transport is: the parts that are decisions —
//! what is on the menu, what each entry means, whether autostart may be
//! registered — are pure and tested here. Building the tray icon and moving
//! the window are OS integration, and they are the untested half.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

/// The tray menu, in order: id, label.
///
/// **Quit is not optional.** Closing the window only hides it (see
/// [`hide_instead_of_closing`]), so if this entry ever goes the app cannot be
/// quit from the UI at all — the tray is the only way out.
pub(crate) const MENU: [(&str, &str); 3] =
    [("open", "Open OmaCal"), ("sync", "Sync now"), ("quit", "Quit")];

/// What a tray menu id means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Open,
    /// Open a meeting's conference link — the one thing a menu bar does
    /// better than a window, and the reason the macOS section exists
    /// (spec 2026-08-29 §2). Carries the URL the feed already resolved.
    Join(String),
    /// Show the window anchored on one date — `omacal 2026-09-01`. Carries
    /// the ISO date re-spelled by the parser, so downstream never re-reads
    /// argv. This is what makes the bar widget's rows, a keybinding, or a
    /// script able to land the calendar *somewhere*, not merely open it.
    OpenAt(String),
    SyncNow,
    Quit,
}

/// What a second `omacal` invocation asks of the running instance, read off
/// its argv. This is the tray menu's vocabulary arriving over the
/// single-instance channel — it exists so a surface that is not this process
/// (the Omarchy bar widget, a script, a keybinding) can drive the app:
/// `omacal --quit`, `omacal --sync-now`, `omacal 2026-09-01` opening the
/// window on that date, and a bare `omacal` meaning what launching an
/// already-running app has always meant, show the window.
/// Unknown flags fall through to Open rather than erroring — a second
/// instance has no stderr anyone will ever read. The flags outrank a date:
/// `--quit` alongside one is still the stronger ask.
pub(crate) fn instance_action(argv: &[String]) -> TrayAction {
    if argv.iter().any(|a| a == "--quit") {
        TrayAction::Quit
    } else if argv.iter().any(|a| a == "--sync-now") {
        TrayAction::SyncNow
    } else if let Some(ymd) = argv.iter().skip(1).find_map(|a| parse_date(a)) {
        TrayAction::OpenAt(ymd)
    } else {
        TrayAction::Open
    }
}

/// A positional date argument: `YYYY-MM-DD`, one spelling, deliberately.
/// The shape gate in front of jiff is what makes the contract testable as
/// stated — whatever looser ISO forms jiff happens to accept, `2026-9-1`
/// must read as an unknown argument (and so as Open, per the rule above),
/// not as a date that works on some builds. jiff then rejects the shapes
/// that look right but name no day, `2026-13-40` and friends.
fn parse_date(arg: &str) -> Option<String> {
    let b = arg.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    arg.parse::<jiff::civil::Date>().ok().map(|d| d.to_string())
}

/// Maps a menu id to the thing it does. Separate from doing it, so the mapping
/// is testable without an `AppHandle` — an id that silently matched nothing
/// would be a menu entry that does nothing when clicked.
///
/// The prefix a dated row's menu id carries, and a joinable one's.
///
/// Ids carry their argument because a menu event hands back a string and
/// nothing else: the alternative is a side table of row → date that the
/// rebuild in [`apply`] would have to keep in step with the menu it just
/// replaced. Parsed here, so an id shape that stops round-tripping is a
/// failing test rather than a menu entry that clicks and does nothing.
const AT_PREFIX: &str = "at:";
const JOIN_PREFIX: &str = "join:";

pub(crate) fn action_for(id: &str) -> Option<TrayAction> {
    match id {
        "open" => Some(TrayAction::Open),
        "sync" => Some(TrayAction::SyncNow),
        "quit" => Some(TrayAction::Quit),
        _ => {
            if let Some(rest) = id.strip_prefix(AT_PREFIX) {
                // Through the same gate argv's date goes through: one
                // spelling, and jiff refusing the shapes that look right
                // but name no day.
                return parse_date(rest).map(TrayAction::OpenAt);
            }
            // Only http(s). The URL is our own — it came from the feed, not
            // from the menu — but the check costs a line and means no future
            // feed change can put a `file:` or `javascript:` string in front
            // of the opener.
            if let Some(url) = id.strip_prefix(JOIN_PREFIX) {
                if url.starts_with("https://") || url.starts_with("http://") {
                    return Some(TrayAction::Join(url.to_string()));
                }
            }
            None
        }
    }
}

/// Whether start-on-login may be registered.
///
/// **Never in demo mode.** A synthetic-data build that launches itself on
/// login is a nasty surprise on someone's machine, and demo mode's whole
/// promise is that it touches nothing real. Same shape and same reason as
/// [`crate::notify_loop::may_notify`].
///
/// The outer gate over the user's own preference, not a replacement for it:
/// [`apply_autostart`] asks this first and the setting second.
pub(crate) fn may_autostart(demo: bool) -> bool {
    !demo
}

/// What the launch entry should be, given the policy and the preference.
///
/// Three states rather than two, and the third is the point. A demo build
/// must not *unregister* the real build's entry either: both ship under the
/// same identifier, so `cargo tauri dev --features demo` on a machine running
/// the release would otherwise quietly delete the launch entry the user
/// chose. Demo mode touches nothing real, in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Autostart {
    Register,
    Unregister,
    LeaveAlone,
}

/// The decision, as a function of the two inputs, so it can be tabled by a
/// test without an app handle or a login session.
pub(crate) fn autostart_action(demo: bool, wanted: bool) -> Autostart {
    if !may_autostart(demo) {
        return Autostart::LeaveAlone;
    }
    if wanted { Autostart::Register } else { Autostart::Unregister }
}

/// The argument the login entry carries, so a launch can tell where it came
/// from.
///
/// A **fact about the launch**, deliberately, not an instruction: it says
/// "the login entry started this", and the setting says what to do about it.
/// The alternative — writing `--background` into the entry only when that
/// mode is chosen — cannot work, because the plugin fixes the entry's
/// arguments when it is initialised, before the database this app stores
/// preferences in has even been opened. It would also make the entry lie for
/// a whole session after the choice changed.
///
/// And it cannot be a plain "always start hidden" preference either: launch
/// omacal from the app menu with nothing running and the window has to
/// appear, or the launcher does nothing when clicked.
pub(crate) const AUTOSTART_FLAG: &str = "--autostart";

/// Whether this launch should put a window on screen.
///
/// True for every launch a person made themselves, whatever the setting says
/// — that is the guard above, stated as code. False only for the login launch
/// of somebody who asked for the background mode.
pub(crate) fn opens_window(argv: &[String], mode: crate::settings::StartOnLogin) -> bool {
    let from_login = argv.iter().any(|a| a == AUTOSTART_FLAG);
    !from_login || mode.opens_window()
}

/// Registers or unregisters the launch entry to match `wanted`.
///
/// Called at startup *and* from the setting's own command, so a change takes
/// effect at the moment it is made rather than at some later launch — the
/// same rule [`crate::settings::set_tray_icon`] follows, and here it is
/// stronger: the whole complaint behind this setting (issue #22) is an app
/// that writes the entry back whatever the user does with it.
///
/// **`Register` is issued on every launch, not only on a change**, and that
/// is deliberate: it rewrites the recorded binary path, which is what repairs
/// an entry gone stale because the AppImage moved or the app was reinstalled
/// under a different prefix. Only the off-branch is new behaviour.
///
/// Failures are logged, never surfaced: nothing the user can do about a
/// refused write to their own autostart directory belongs in a modal, and
/// the calendar works regardless.
pub(crate) fn apply_autostart(app: &tauri::AppHandle, demo: bool, wanted: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let result = match autostart_action(demo, wanted) {
        Autostart::LeaveAlone => return,
        Autostart::Register => app.autolaunch().enable(),
        Autostart::Unregister => app.autolaunch().disable(),
    };
    if let Err(e) = result {
        tracing::warn!(%e, wanted, "could not apply the start-on-login setting");
    }
}

/// Whether a window-close should hide rather than quit.
///
/// Unless the user has asked for the opposite, and only then. Hiding is what
/// stands between this app and the bug §2.6 describes — a window someone
/// closed, an app that looks gone, and reminders that silently stopped firing
/// — so it remains what an install does by default and what every unreadable
/// or absent setting resolves to; `settings::quit_on_close` holds that
/// polarity. Quit is otherwise explicit, from the tray.
///
/// Trivial as a function, and it stays one for the module's stated split: the
/// decision is testable here, and `lib.rs`'s window handler does the OS half.
pub(crate) fn hide_instead_of_closing(quit_on_close: bool) -> bool {
    !quit_on_close
}

/// The tray icon's id, shared by [`build`] and [`set_visible`].
const TRAY_ID: &str = "omacal-tray";

/// How much of a title the macOS menu bar gets before an ellipsis.
///
/// The scarcest space in the app — a long meeting name pushes every other
/// menu extra off the right-hand side — so this errs short, shorter than the
/// dropdown below it, where a row has the whole menu width to itself.
const TITLE_CAP: usize = 18;
/// A dropdown row's own cap. Wider than the bar, still not the calendar.
const ROW_CAP: usize = 32;
/// A dropdown is a glance, not the calendar.
const EVENT_ROWS: usize = 8;
/// All-day spans are context, not the next hour: a couple, no more.
const ALLDAY_ROWS: usize = 3;
const TASK_ROWS: usize = 4;

/// `s` cut to `cap` *characters* with an ellipsis, or unchanged.
///
/// Characters and not bytes: a Cyrillic meeting title (this calendar has
/// plenty) would otherwise be cut mid-codepoint, and `String` truncation on
/// a char boundary is a panic, not a mangled label.
fn ellipsize(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    let kept: String = s.chars().take(cap.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// A title with no title, worded once.
const UNTITLED: &str = "(no title)";

/// Whether `ev` is happening at `now_ms`. Half-open, as every interval in
/// this codebase is: an event that ends exactly now has ended.
fn running(ev: &crate::upcoming::FeedEvent, now_ms: i64) -> bool {
    ev.start_ms <= now_ms && now_ms < ev.end_ms
}

/// `ms` as a zoned instant in `tz`, never panicking.
fn zoned(ms: i64, tz: &jiff::tz::TimeZone) -> jiff::Zoned {
    jiff::Timestamp::from_millisecond(ms)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(tz.clone())
}

/// The clock omacal shows, in **one** zone for every row.
///
/// Emphatically not each event's own calendar zone, which is what this read
/// before and what made the menu lie: with calendars in Europe/Sofia and
/// Asia/Kolkata, a 05:30 meeting rendered "08:00" beside a genuine 08:00 one
/// and the list looked randomly ordered because the clocks came from
/// different zones (field-reported 2026-08-29, with a screenshot of exactly
/// that). A menu bar shows the wearer's own clock or it shows nothing
/// trustworthy. The process's zone already honours the display-timezone
/// setting — `apply_display_tz_early` exports `TZ` before anything renders —
/// so the system zone *is* the user's chosen zone.
fn clock(ms: i64, tz: &jiff::tz::TimeZone, fmt: crate::settings::TimeFormat) -> String {
    let z = zoned(ms, tz);
    match fmt {
        crate::settings::TimeFormat::H24 => format!("{:02}:{:02}", z.hour(), z.minute()),
        crate::settings::TimeFormat::H12 => {
            let hour = z.hour();
            let dial = if hour % 12 == 0 { 12 } else { hour % 12 };
            format!("{dial}:{:02}{}", z.minute(), if hour < 12 { "am" } else { "pm" })
        }
    }
}

/// `Mon `, or nothing when `ms` falls on the same day as `now_ms`.
///
/// The feed runs past midnight, so without this tomorrow's 07:00 sits under
/// today's 15:00 looking like a sorting bug — which is exactly how it read
/// in the field.
fn day_prefix(ms: i64, now_ms: i64, tz: &jiff::tz::TimeZone) -> String {
    let (a, b) = (zoned(ms, tz), zoned(now_ms, tz));
    if a.date() == b.date() {
        String::new()
    } else {
        format!("{} ", a.strftime("%a"))
    }
}

/// The macOS menu bar's text, or `None` for the icon alone.
///
/// The event running now wins over the next one — knowing you are *in*
/// something beats knowing what follows it. All-day entries never claim the
/// title: a day-long "Trip" would sit in the menu bar all day saying nothing
/// about the next hour, and the width it costs is the width every other
/// menu extra loses. Nothing upcoming yields `None` rather than an empty
/// string, which AppKit treats differently, and a stale title after the
/// meeting is worse than no title.
///
/// Only macOS shows the result (see [`apply`]), but it is compiled, called
/// and tested on every platform: CI is Linux-only, so a decision that only a
/// Mac compiles is a decision nothing checks until a release build finds it.
pub(crate) fn menu_title(
    feed: &crate::upcoming::Feed,
    now_ms: i64,
    tz: &jiff::tz::TimeZone,
    fmt: crate::settings::TimeFormat,
) -> Option<String> {
    let timed = || feed.events.iter().filter(|e| !e.all_day);
    let now = timed().find(|e| running(e, now_ms));
    let next = || timed().find(|e| e.start_ms > now_ms);
    let ev = now.or_else(next)?;
    let title = ellipsize(ev.title.as_deref().unwrap_or(UNTITLED), TITLE_CAP);
    Some(if now.is_some() {
        format!("▸ {title}")
    } else {
        format!("{}{}  {title}", day_prefix(ev.start_ms, now_ms, tz), clock(ev.start_ms, tz, fmt))
    })
}

/// What a row *is*, so [`apply`] can dress it without deciding anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A greyed heading — "Today", "Tomorrow", "Fri 5 Sep". Disabled, so it
    /// never emits a click and never needs an action.
    Heading,
    /// A clickable row, wearing its calendar's colour when it has one. The
    /// dot is the same language the grid and the event form's picker speak,
    /// and the thing a native calendar menu is expected to show.
    Item(Option<String>),
    /// A rule between sections.
    Rule,
}

/// One row of the tray's live section: the menu id that names its action,
/// the label the user reads, and how to draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub id: String,
    pub label: String,
    pub kind: Kind,
}

impl Row {
    fn heading(label: impl Into<String>) -> Row {
        Row { id: HEADING_ID.into(), label: label.into(), kind: Kind::Heading }
    }
    fn rule() -> Row {
        Row { id: HEADING_ID.into(), label: String::new(), kind: Kind::Rule }
    }
}

/// Headings and rules share one id and no action: disabled items emit no
/// click, so nothing ever asks what it means.
const HEADING_ID: &str = "section";

/// How a day is announced above its events.
fn heading_for(ms: i64, now_ms: i64, tz: &jiff::tz::TimeZone) -> String {
    let (day, today) = (zoned(ms, tz), zoned(now_ms, tz));
    let delta = day.date().since(today.date()).map(|s| s.get_days()).unwrap_or(0);
    match delta {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        // "Fri 5 Sep" — assembled rather than `strftime`'d so no
        // padding flag has to behave the same across jiff versions.
        _ => format!("{} {} {}", day.strftime("%a"), day.day(), day.strftime("%b")),
    }
}

/// The day an instant falls on, as the `YYYY-MM-DD` `OpenAt` speaks — in the
/// display zone, so a row opens the day the row said it was on.
fn day_key(ms: i64, tz: &jiff::tz::TimeZone) -> String {
    zoned(ms, tz).date().to_string()
}

/// The live rows: what is next, a Join for the meeting at hand, the all-day
/// context, then what is due.
///
/// **Timed events lead.** All-day entries used to sit on top because the feed
/// sorts by instant and a months-long leave marker starts earliest — so two
/// rows of "who is on holiday until December" pushed the next actual meeting
/// down the menu. What a menu bar is for is the next hour.
///
/// Pure, and that is the point — everything this decides is decided here,
/// leaving [`apply`] with nothing but Tauri calls.
pub(crate) fn rows(
    feed: &crate::upcoming::Feed,
    now_ms: i64,
    tz: &jiff::tz::TimeZone,
    fmt: crate::settings::TimeFormat,
) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();

    // Timed events, under a heading per day. The heading is what lets a row
    // read as plain "07:00" — before this each row had to carry its own
    // weekday, and a menu of "Mon 07:00" repeated down the column is a table
    // nobody asked for.
    let mut day = String::new();
    for ev in feed.events.iter().filter(|e| !e.all_day).take(EVENT_ROWS) {
        let key = day_key(ev.start_ms, tz);
        if key != day {
            out.push(Row::heading(heading_for(ev.start_ms, now_ms, tz)));
            day = key.clone();
        }
        let title = ellipsize(ev.title.as_deref().unwrap_or(UNTITLED), ROW_CAP);
        let mark = if running(ev, now_ms) { "▸ " } else { "" };
        out.push(Row {
            id: format!("{AT_PREFIX}{key}"),
            label: format!("{mark}{}  {title}", clock(ev.start_ms, tz, fmt)),
            kind: Kind::Item(ev.color.clone()),
        });
    }

    // One Join, for the meeting at hand — the running one, else the next.
    // Not one per row: a dropdown of Join buttons is a way to join the
    // wrong call, and the feed's later rows are hours away.
    let timed = || feed.events.iter().filter(|e| !e.all_day);
    let at_hand = timed().find(|e| running(e, now_ms)).or_else(|| timed().find(|e| e.start_ms > now_ms));
    if let Some(url) = at_hand.and_then(|e| e.conference.as_deref()) {
        if url.starts_with("https://") || url.starts_with("http://") {
            out.push(Row {
                id: format!("{JOIN_PREFIX}{url}"),
                label: "Join meeting".into(),
                kind: Kind::Item(None),
            });
        }
    }

    // All-day context, after the clock and behind its own rule. No running
    // marker: a leave span covering five months is not something you are
    // "in" the way a meeting is, and the ▸ beside it said nothing true.
    let all_day: Vec<_> = feed.events.iter().filter(|e| e.all_day).take(ALLDAY_ROWS).collect();
    if !all_day.is_empty() {
        out.push(Row::rule());
        out.push(Row::heading("All day"));
        for ev in all_day {
            out.push(Row {
                id: format!("{AT_PREFIX}{}", day_key(ev.start_ms.max(now_ms), tz)),
                label: ellipsize(ev.title.as_deref().unwrap_or(UNTITLED), ROW_CAP),
                kind: Kind::Item(ev.color.clone()),
            });
        }
    }

    let tasks: Vec<_> = feed.tasks.iter().take(TASK_ROWS).collect();
    if !tasks.is_empty() {
        out.push(Row::rule());
        out.push(Row::heading("Due"));
        for task in tasks {
            let title = ellipsize(&task.title, ROW_CAP);
            // Tasks have no day of their own to open — the app's task list
            // is one window away, so every task row simply opens omacal.
            out.push(Row {
                id: "open".into(),
                label: if task.overdue { format!("⚠  {title}") } else { title },
                kind: Kind::Item(task.color.clone()),
            });
        }
    }
    out
}

/// A filled dot in `hex`, as a menu-item icon.
///
/// Drawn here rather than shipped as assets because the colour is the
/// user's: calendars carry their own, and omacal lets them be recoloured
/// locally. Supersampled 3×3 so the circle has a soft edge instead of the
/// staircase a 16px hard-edged circle shows at menu size.
fn swatch(hex: &str) -> Option<tauri::image::Image<'static>> {
    let h = hex.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let (r, g, b) = (c(0)?, c(2)?, c(4)?);

    const N: u32 = 16;
    const S: u32 = 3; // samples per axis
    let centre = (N as f32 - 1.0) / 2.0;
    let radius = N as f32 * 0.34;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let mut hits = 0u32;
            for sy in 0..S {
                for sx in 0..S {
                    let px = x as f32 + (sx as f32 + 0.5) / S as f32 - 0.5;
                    let py = y as f32 + (sy as f32 + 0.5) / S as f32 - 0.5;
                    if (px - centre).powi(2) + (py - centre).powi(2) <= radius * radius {
                        hits += 1;
                    }
                }
            }
            let a = (hits * 255 / (S * S)) as u8;
            // Premultiplied is not wanted here; straight alpha with the
            // colour carried into fully transparent pixels keeps edges from
            // fringing toward black.
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Some(tauri::image::Image::new_owned(rgba, N, N))
}

/// Shows or hides the tray icon on a running app — the live half of the
/// `tray_icon` setting. A no-op when the tray never built (macOS refusals,
/// headless oddities): the setting still persists and applies next launch.
pub(crate) fn set_visible(app: &AppHandle, on: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Err(e) = tray.set_visible(on) {
            tracing::warn!(%e, on, "could not change tray icon visibility");
        }
    }
}

/// Builds the tray icon and wires its menu.
///
/// **Untested.** Everything it decides is decided by [`MENU`] and
/// [`action_for`] above, which are; what is left is Tauri and the OS, and this
/// project has no way to assert that an icon appeared in a system tray.
pub(crate) fn build(app: &AppHandle) -> tauri::Result<()> {
    let items: Vec<MenuItem<_>> = MENU
        .iter()
        .map(|(id, label)| MenuItem::with_id(app, id, label, true, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let refs: Vec<&dyn tauri::menu::IsMenuItem<_>> =
        items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<_>).collect();
    let menu = Menu::with_items(app, &refs)?;

    // Not the window icon: that is the mark on a dark tile, and at tray
    // sizes on a dark bar the tile swallows it. tray.png is the mark alone
    // (see icons/tray.svg), drawn to survive 22px.
    //
    // Built with an id so `set_visible` below can find it again: the tray
    // icon is now a *setting*, because on Omarchy 4 the bar widget carries
    // the same three actions and a second omacal icon is one too many.
    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .icon(tauri::include_image!("icons/tray.png"))
        .on_menu_event(|app, event| match action_for(event.id.as_ref()) {
            Some(TrayAction::Open) => show_main_window(app),
            // Unreachable from a menu — `action_for` never returns it, the
            // menu has no dated entry — but honoured rather than ignored,
            // because a match arm that discards an action is how a future
            // menu entry would click and do nothing.
            Some(TrayAction::OpenAt(ymd)) => open_at(app, &ymd),
            Some(TrayAction::Join(url)) => {
                if let Err(e) = crate::browser::open_external(&url) {
                    tracing::warn!(%e, "could not open the meeting link from the tray");
                }
            }
            Some(TrayAction::SyncNow) => crate::sync_loop::request_now(app),
            Some(TrayAction::Quit) => app.exit(0),
            // An id the menu did not put there. Nothing to do, and nothing
            // worth crashing the app over.
            None => tracing::warn!(id = %event.id.as_ref(), "unknown tray menu id"),
        })
        .build(app)?;

    Ok(())
}

/// Rebuilds the tray's menu and title from one snapshot.
///
/// **Untested, like [`build`]** — every decision it carries was made by
/// [`rows`] and [`menu_title`], which are; what is left is Tauri and AppKit.
fn apply(app: &AppHandle, feed: &crate::upcoming::Feed, now_ms: i64,
         fmt: crate::settings::TimeFormat, date_icon: bool) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(()); // Tray turned off, or never built. Nothing to dress.
    };

    // One zone for the whole menu, resolved once: the process's own, which
    // `apply_display_tz_early` has already pointed at the display-timezone
    // setting when there is one.
    let tz = jiff::tz::TimeZone::system();
    let live = rows(feed, now_ms, &tz, fmt);

    // Three shapes, built up front so the borrows outlive the reference
    // list below: a menu is assembled from `&dyn IsMenuItem`, and every
    // item has to still be alive when `Menu::with_items` reads them.
    enum Built<R: tauri::Runtime> {
        Plain(MenuItem<R>),
        Icon(tauri::menu::IconMenuItem<R>),
        Rule(tauri::menu::PredefinedMenuItem<R>),
    }
    let mut built: Vec<Built<_>> = Vec::new();
    for r in &live {
        built.push(match &r.kind {
            Kind::Rule => Built::Rule(tauri::menu::PredefinedMenuItem::separator(app)?),
            // Disabled: a heading is a label, and a disabled item emits no
            // click, which is why headings need no action.
            Kind::Heading => {
                Built::Plain(MenuItem::with_id(app, &r.id, &r.label, false, None::<&str>)?)
            }
            Kind::Item(color) => match color.as_deref().and_then(swatch) {
                Some(dot) => Built::Icon(tauri::menu::IconMenuItem::with_id(
                    app, &r.id, &r.label, true, Some(dot), None::<&str>,
                )?),
                None => {
                    Built::Plain(MenuItem::with_id(app, &r.id, &r.label, true, None::<&str>)?)
                }
            },
        });
    }
    let fixed: Vec<MenuItem<_>> = MENU
        .iter()
        .map(|(id, label)| MenuItem::with_id(app, id, label, true, None::<&str>))
        .collect::<tauri::Result<_>>()?;

    let sep = tauri::menu::PredefinedMenuItem::separator(app)?;
    let mut refs: Vec<&dyn tauri::menu::IsMenuItem<_>> = Vec::new();
    for b in &built {
        refs.push(match b {
            Built::Plain(i) => i,
            Built::Icon(i) => i,
            Built::Rule(i) => i,
        });
    }
    if !built.is_empty() {
        refs.push(&sep);
    }
    for i in &fixed {
        refs.push(i);
    }
    tray.set_menu(Some(Menu::with_items(app, &refs)?))?;

    // `cfg!` and not `#[cfg]`, deliberately. `set_title` is not
    // platform-gated in Tauri, so writing the decision as a runtime branch
    // means the Linux CI runner type-checks the very line macOS runs —
    // and CI is Linux-only, so an `#[cfg]` block here would be compiled by
    // nothing until a release build on a Mac discovered it.
    //
    // The decision itself: macOS shows the title, Linux does not. There the
    // same string costs panel width next to a bar widget already saying
    // more, and Tauri's own note says the title needs the icon shown anyway.
    let title =
        if cfg!(target_os = "macos") { menu_title(feed, now_ms, &tz, fmt) } else { None };
    tray.set_title(title)?;

    // The date *is* the icon where it is wanted (2026-09-04): a tray host
    // draws icons and nothing else, which is the same fact the title branch
    // above is about. Set on every refresh rather than only when the day
    // turns, because the minute tick is already here and a comparison
    // against the icon currently shown is not something the tray can be
    // asked for.
    tray.set_icon(Some(if date_icon {
        crate::tray_date::icon_for(crate::today_of_month(now_ms, &tz))
    } else {
        crate::tray_date::mark()
    }))?;

    Ok(())
}

/// Recomputes the snapshot and dresses the tray with it.
///
/// Spawned rather than awaited: every caller is a place that has just
/// finished doing something else (a sync landing, a tick firing), and none
/// of them should wait on a menu.
pub(crate) fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (pool, demo) = {
            let state = app.state::<crate::AppState>();
            (state.pool.clone(), state.demo)
        };
        // Demo mode dresses nothing, for the reason the feed itself refuses
        // it: synthetic meetings must never be announced as real.
        if demo {
            return;
        }
        let now = crate::now_ms();
        let feed = match crate::upcoming::current(&pool, now).await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(%e, "could not read the upcoming feed for the tray");
                return;
            }
        };
        let settings = crate::settings::read_settings(&pool).await;
        if let Err(e) = apply(&app, &feed, now, settings.time_format, settings.show_date) {
            tracing::warn!(%e, "could not update the tray menu");
        }
    });
}

/// How often the tray re-reads the clock.
///
/// The answer changes with time alone — a meeting starting is not something
/// any other part of this app notifies us about — so a tick is the only way
/// the title stops lying. A minute is the resolution the title shows.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// Starts the minute tick that keeps the title honest.
pub(crate) fn spawn_ticker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            refresh(&app);
        }
    });
}

/// Brings the window back from hidden. Untested for the same reason as
/// [`build`].
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// What a dated invocation emits once the window is up, carrying the ISO
/// date. The webview owns every date computation in its own zone, so the
/// string crosses whole rather than as a timestamp somebody here would have
/// to pick a zone for.
pub(crate) const OPEN_DATE_EVENT: &str = "open-date";

/// [`show_main_window`], then tell the webview where to land. Untested like
/// its first half; everything it decides was decided by `parse_date`.
pub(crate) fn open_at(app: &AppHandle, ymd: &str) {
    use tauri::Emitter;
    show_main_window(app);
    let _ = app.emit(OPEN_DATE_EVENT, ymd.to_string());
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::settings::TimeFormat;
    use crate::upcoming::{Feed, FeedEvent, FeedTask};

    /// 2026-08-29 09:00 UTC, and the times below are offsets from it.
    const T0: i64 = 1_787_994_000_000;

    fn ev(title: &str, start: i64, end: i64) -> FeedEvent {
        FeedEvent {
            title: Some(title.into()),
            start_ms: start,
            end_ms: end,
            all_day: false,
            tz: "UTC".into(),
            location: None,
            attendees: 0,
            response: None,
            conference: None,
            color: None,
            calendar: None,
        }
    }

    fn feed(events: Vec<FeedEvent>) -> Feed {
        Feed { version: 1, generated_ms: T0, events, tasks: Vec::new(), today: None }
    }

    /// The wearer's zone. Fixed rather than `TimeZone::system()` so these
    /// assertions mean the same thing on every machine that runs them.
    fn sofia() -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get("Europe/Sofia").expect("a zone jiff ships")
    }

    /// Just the clickable rows — what a reader clicks, without the
    /// headings and rules that structure them.
    fn items(rows: &[Row]) -> Vec<&Row> {
        rows.iter().filter(|r| matches!(r.kind, Kind::Item(_))).collect()
    }

    /// Every row's label in order, headings included, for the tests whose
    /// subject is the shape of the menu rather than one entry in it.
    fn shape(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r.kind {
                Kind::Rule => "──".to_string(),
                _ => r.label.clone(),
            })
            .collect()
    }

    /// The running meeting outranks the next one: being *in* something is
    /// the more useful fact, and the marker is what tells the two apart.
    #[test]
    fn the_title_prefers_the_meeting_you_are_in() {
        let f = feed(vec![
            ev("Standup", T0 - 600_000, T0 + 600_000),
            ev("Review", T0 + 3_600_000, T0 + 7_200_000),
        ]);
        assert_eq!(menu_title(&f, T0, &sofia(), TimeFormat::H24).as_deref(), Some("▸ Standup"));
    }

    /// With nothing running, the next one — and it carries the start time,
    /// which is the whole reason to look at the menu bar.
    #[test]
    fn the_title_falls_to_the_next_meeting_with_its_clock() {
        let f = feed(vec![ev("Review", T0 + 3_600_000, T0 + 7_200_000)]);
        assert_eq!(menu_title(&f, T0, &sofia(), TimeFormat::H24).as_deref(), Some("13:00  Review"));
    }

    /// An event that ended exactly now has ended — the half-open rule every
    /// interval in this codebase follows.
    #[test]
    fn a_meeting_ending_now_is_not_the_one_you_are_in() {
        let f = feed(vec![ev("Standup", T0 - 600_000, T0)]);
        assert_eq!(menu_title(&f, T0, &sofia(), TimeFormat::H24), None);
    }

    /// All-day entries never claim the width: a day-long trip would sit
    /// there all day saying nothing about the next hour.
    #[test]
    fn an_all_day_event_never_becomes_the_title() {
        let mut trip = ev("Trip to Sofia", T0 - 3_600_000, T0 + 80_000_000);
        trip.all_day = true;
        let f = feed(vec![trip]);
        assert_eq!(menu_title(&f, T0, &sofia(), TimeFormat::H24), None);
    }

    /// `None`, not `Some("")` — AppKit treats the two differently, and a
    /// stale title after the meeting is worse than no title at all.
    #[test]
    fn an_empty_calendar_gets_no_title_rather_than_an_empty_one() {
        assert_eq!(menu_title(&feed(vec![]), T0, &sofia(), TimeFormat::H24), None);
    }

    /// Cut by characters, never bytes: this calendar is full of Cyrillic,
    /// and slicing a `String` off a char boundary is a panic.
    #[test]
    fn a_long_cyrillic_title_is_cut_without_panicking() {
        let long = "Консулски услуги в посолството на Република България";
        let f = feed(vec![ev(long, T0 + 60_000, T0 + 600_000)]);
        let title = menu_title(&f, T0, &sofia(), TimeFormat::H24).expect("a next meeting");
        assert!(title.ends_with('…'), "{title}");
        assert!(title.chars().count() <= TITLE_CAP + 8, "{title}");
    }

    /// The row's id has to survive the trip out to AppKit and back through
    /// `action_for` as the same day, or the row opens the wrong one.
    #[test]
    fn a_row_id_round_trips_to_the_day_its_event_is_on() {
        let f = feed(vec![ev("Standup", T0, T0 + 600_000)]);
        let r = rows(&f, T0 - 60_000, &sofia(), TimeFormat::H24);
        let it = items(&r);
        assert_eq!(it[0].label, "12:00  Standup");
        assert_eq!(action_for(&it[0].id), Some(TrayAction::OpenAt("2026-08-29".into())));
    }

    /// One Join, for the meeting at hand — a dropdown of them is a way to
    /// join the wrong call.
    #[test]
    fn only_the_meeting_at_hand_offers_a_join() {
        let mut now = ev("Standup", T0 - 60_000, T0 + 600_000);
        now.conference = Some("https://meet.google.com/abc-defg-hij".into());
        let mut later = ev("Review", T0 + 3_600_000, T0 + 7_200_000);
        later.conference = Some("https://zoom.us/j/999".into());
        let r = rows(&feed(vec![now, later]), T0, &sofia(), TimeFormat::H24);
        let joins: Vec<_> = r.iter().filter(|x| x.label == "Join meeting").collect();
        assert_eq!(joins.len(), 1);
        assert_eq!(
            action_for(&joins[0].id),
            Some(TrayAction::Join("https://meet.google.com/abc-defg-hij".into()))
        );
    }

    /// Anything that is not http(s) is not a thing this menu hands to the
    /// opener, however it got into the feed.
    #[test]
    fn a_non_web_scheme_is_never_a_join() {
        assert_eq!(action_for("join:file:///etc/passwd"), None);
        assert_eq!(action_for("join:javascript:alert(1)"), None);
        assert_eq!(action_for("at:2026-13-40"), None, "jiff refuses the impossible day");
        assert_eq!(action_for("at:2026-9-1"), None, "one spelling only");
    }

    /// Overdue reads differently from due, because that is the difference
    /// worth glancing at.
    #[test]
    fn tasks_follow_the_meetings_and_overdue_ones_say_so() {
        let mut f = feed(vec![]);
        f.tasks = vec![
            FeedTask { title: "Pay Unicredit".into(), due_ms: T0 - 3_600_000, all_day: false,
                       overdue: true, list: None, color: None, priority: 0 },
            FeedTask { title: "Renew domain".into(), due_ms: T0 + 3_600_000, all_day: false,
                       overdue: false, list: None, color: None, priority: 0 },
        ];
        let r = rows(&f, T0, &sofia(), TimeFormat::H24);
        let it = items(&r);
        assert_eq!(it[0].label, "⚠  Pay Unicredit");
        assert_eq!(it[1].label, "Renew domain");
        assert_eq!(it[0].id, "open", "a task row has no day of its own to open");
        assert!(shape(&r).contains(&"Due".to_string()));
    }


    /// **The bug the screenshot caught (2026-08-29).** Two calendars, two
    /// zones, one menu: an Asia/Kolkata meeting and a Europe/Sofia one an
    /// hour apart both rendered "08:00" because each was drawn in its own
    /// calendar's zone. The menu showed the same clock for different hours
    /// and the list read as unsorted. Every row is the wearer's clock now.
    #[test]
    fn two_calendars_in_two_zones_still_share_one_clock() {
        let mut kolkata = ev("Kids: Squash", T0, T0 + 1_800_000);
        kolkata.tz = "Asia/Kolkata".into();
        let sofia_ev = ev("Meet Tzveti", T0 + 3_600_000, T0 + 7_200_000);
        let r = rows(&feed(vec![kolkata, sofia_ev]), T0 - 60_000, &sofia(), TimeFormat::H24);
        let it = items(&r);
        // 09:00 UTC and 10:00 UTC, an hour apart, read an hour apart.
        assert_eq!(it[0].label, "12:00  Kids: Squash");
        assert_eq!(it[1].label, "13:00  Meet Tzveti");
    }

    /// The feed runs past midnight, so each day is announced — without it
    /// tomorrow's 07:00 sat under today's 15:00 looking like a sorting bug,
    /// which is how the field report read. A heading says it once instead
    /// of every row carrying a weekday of its own.
    #[test]
    fn each_day_is_announced_once_above_its_events() {
        let today = ev("Meet Tzveti", T0 + 3_600_000, T0 + 7_200_000);
        let tomorrow = ev("travel to excitel office", T0 + 22 * 3_600_000, T0 + 23 * 3_600_000);
        let r = rows(&feed(vec![today, tomorrow]), T0, &sofia(), TimeFormat::H24);
        assert_eq!(
            shape(&r),
            ["Today", "13:00  Meet Tzveti", "Tomorrow", "10:00  travel to excitel office"]
        );
        assert!(matches!(r[0].kind, Kind::Heading));
    }

    /// A day further out is named, since "Tomorrow" stops being useful.
    #[test]
    fn a_day_beyond_tomorrow_is_named() {
        let later = ev("Board", T0 + 4 * 86_400_000, T0 + 4 * 86_400_000 + 3_600_000);
        let r = rows(&feed(vec![later]), T0, &sofia(), TimeFormat::H24);
        assert_eq!(r[0].label, "Wed 2 Sep");
    }

    /// The dot is the calendar's own colour — the same language the grid
    /// and the event form's picker speak — and a row without one still
    /// renders rather than vanishing.
    #[test]
    fn a_row_carries_its_calendars_colour() {
        let mut coloured = ev("Standup", T0 + 3_600_000, T0 + 7_200_000);
        coloured.color = Some("#5b8def".into());
        let r = rows(&feed(vec![coloured]), T0, &sofia(), TimeFormat::H24);
        assert_eq!(items(&r)[0].kind, Kind::Item(Some("#5b8def".into())));
        assert!(swatch("#5b8def").is_some());
        assert!(swatch("not a colour").is_none(), "garbage never becomes an icon");
        assert!(swatch("#5b8de").is_none(), "a short hex is not a colour");
    }

    /// All-day spans are context and come after the clock — a leave marker
    /// running to December used to sit above the next actual meeting — and
    /// they never wear the running marker, which claimed something untrue.
    #[test]
    fn all_day_spans_follow_the_meetings_and_are_never_marked_running() {
        let mut leave = ev("Plamen 17.07 - 12.12 (TK)", T0 - 40 * 86_400_000, T0 + 100 * 86_400_000);
        leave.all_day = true;
        let meeting = ev("Meet Tzveti", T0 + 3_600_000, T0 + 7_200_000);
        let r = rows(&feed(vec![leave, meeting]), T0, &sofia(), TimeFormat::H24);
        let it = items(&r);
        assert!(it[0].label.contains("Meet Tzveti"), "meetings lead: {}", it[0].label);
        assert!(it[1].label.contains("Plamen"), "{}", it[1].label);
        assert!(!it[1].label.contains('▸'), "an all-day span is not a meeting you are in");
        assert!(shape(&r).contains(&"All day".to_string()), "under its own heading");
    }

    /// The 12-hour setting reaches the menu bar too.
    #[test]
    fn the_twelve_hour_setting_is_honoured() {
        let f = feed(vec![ev("Review", T0 + 3_600_000, T0 + 7_200_000)]);
        assert_eq!(menu_title(&f, T0, &sofia(), TimeFormat::H12).as_deref(), Some("1:00pm  Review"));
    }

    /// Open, Sync now, Quit — in that order, and Quit present at all.
    #[test]
    fn the_tray_menu_offers_open_sync_and_quit() {
        assert_eq!(
            MENU.map(|(id, _)| id),
            ["open", "sync", "quit"],
            "the tray menu's contents and their order"
        );
        assert_eq!(MENU.map(|(_, label)| label), ["Open OmaCal", "Sync now", "Quit"]);
    }

    /// Stated on its own because losing it is not a cosmetic regression: with
    /// the close button only hiding the window, a tray with no Quit leaves no
    /// way to exit the app short of killing the process.
    ///
    /// **What the first assertion is and is not.** It pins a *constant*, not a
    /// behaviour: the window is actually hidden by the `CloseRequested` arm in
    /// `lib.rs`, inside a Tauri event closure this project cannot drive from a
    /// test. So this asserts that the flag that arm consults still says hide,
    /// and nothing more. If someone deletes the arm and leaves the constant,
    /// every test here still passes and closing the window quits the app.
    /// Recorded plainly rather than left to look like the others.
    #[test]
    fn quit_is_on_the_menu_because_closing_the_window_does_not_quit() {
        assert!(hide_instead_of_closing(false), "fixture check: closing only hides");
        assert!(
            MENU.iter().any(|(id, _)| *id == "quit"),
            "closing the window only hides it, so the tray must offer a way out"
        );
    }

    /// Issue #26. The setting is the only thing that may move this, and it
    /// moves it in one direction: `false` — absent row, unreadable row, an
    /// install that predates the setting — is the hide the tray menu's
    /// promise above depends on.
    #[test]
    fn only_the_setting_makes_a_close_quit() {
        assert!(hide_instead_of_closing(false));
        assert!(!hide_instead_of_closing(true));
    }

    /// Every id on the menu maps to something. An entry that mapped to nothing
    /// would render, be clickable, and do nothing at all.
    #[test]
    fn every_menu_entry_maps_to_an_action() {
        for (id, label) in MENU {
            assert!(action_for(id).is_some(), "menu entry {label:?} ({id}) does nothing");
        }
        assert_eq!(action_for("open"), Some(TrayAction::Open));
        assert_eq!(action_for("sync"), Some(TrayAction::SyncNow));
        assert_eq!(action_for("quit"), Some(TrayAction::Quit));
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// The whole contract of the second-invocation channel: the two flags,
    /// the bare-launch default, and — the case that matters most, because a
    /// stray flag must never quit someone's app — unknown arguments reading
    /// as Open.
    #[test]
    fn a_second_invocations_argv_maps_to_an_action() {
        assert_eq!(instance_action(&argv(&["omacal", "--quit"])), TrayAction::Quit);
        assert_eq!(instance_action(&argv(&["omacal", "--sync-now"])), TrayAction::SyncNow);
        assert_eq!(instance_action(&argv(&["omacal"])), TrayAction::Open);
        assert_eq!(instance_action(&argv(&["omacal", "--wat"])), TrayAction::Open);
        // Quit outranks sync when both are passed: the stronger ask wins,
        // and a sync on a quitting app is work thrown away.
        assert_eq!(
            instance_action(&argv(&["omacal", "--sync-now", "--quit"])),
            TrayAction::Quit
        );
        assert_eq!(action_for("nonsense"), None);
    }

    /// The dated invocation: one spelling in, the same spelling out, and
    /// everything that is not exactly a date falling back to the rule above —
    /// Open, never an error, because a second instance has no stderr.
    #[test]
    fn a_positional_date_opens_the_window_on_that_date() {
        assert_eq!(
            instance_action(&argv(&["omacal", "2026-09-01"])),
            TrayAction::OpenAt("2026-09-01".into())
        );
        // The shape gate: a date jiff might tolerate is still not the one
        // spelling this contract admits.
        assert_eq!(instance_action(&argv(&["omacal", "2026-9-1"])), TrayAction::Open);
        // The right shape naming no day at all.
        assert_eq!(instance_action(&argv(&["omacal", "2026-13-40"])), TrayAction::Open);
        // The flags outrank a date — `--quit` alongside one is the stronger
        // ask, and a quitting app has nowhere to land.
        assert_eq!(
            instance_action(&argv(&["omacal", "2026-09-01", "--quit"])),
            TrayAction::Quit
        );
        assert_eq!(
            instance_action(&argv(&["omacal", "2026-09-01", "--sync-now"])),
            TrayAction::SyncNow
        );
    }

    /// The other half of the demo promise. Demo mode never writes the real
    /// database, never reaches Google, posts no notifications — and does not
    /// register itself to launch on login either.
    #[test]
    fn demo_mode_never_registers_start_on_login() {
        assert!(may_autostart(false));
        assert!(!may_autostart(true), "a synthetic-data build must not launch itself");
    }

    /// The whole of the start-on-login decision, as a table.
    ///
    /// The row that matters is the last one: a demo build must leave the
    /// entry **alone**, not unregister it. Both builds ship under the same
    /// identifier, so a demo run that reached for `disable()` would delete
    /// the real install's launch entry — the same class of surprise as
    /// registering one, in the other direction.
    #[test]
    fn the_setting_decides_and_demo_mode_touches_nothing() {
        assert_eq!(autostart_action(false, true), Autostart::Register);
        assert_eq!(
            autostart_action(false, false),
            Autostart::Unregister,
            "turning the setting off has to actually remove the entry — an app that only \
             stops re-adding it leaves the user exactly where issue #22 found them"
        );
        assert_eq!(autostart_action(true, true), Autostart::LeaveAlone);
        assert_eq!(autostart_action(true, false), Autostart::LeaveAlone);
    }

    /// **A launch somebody made themselves always opens a window.**
    ///
    /// The whole risk in a background mode is an app that does nothing when
    /// you click it, and the flag is what averts it: only a launch carrying
    /// it is a login launch, and only that one may be silent. The window is
    /// configured invisible, so a bug here is not a cosmetic one — it is an
    /// omacal that cannot be opened at all.
    #[test]
    fn only_a_login_launch_may_start_without_a_window() {
        use crate::settings::StartOnLogin::{Background, Off, Open};
        let manual = argv(&["omacal"]);
        let login = argv(&["omacal", AUTOSTART_FLAG]);

        for mode in [Off, Open, Background] {
            assert!(
                opens_window(&manual, mode),
                "a launch with no login flag must open the window whatever {mode:?} says — \
                 the app menu's launcher is this one, and it cannot do nothing"
            );
        }

        assert!(opens_window(&login, Open));
        assert!(!opens_window(&login, Background));
        // Unreachable in practice — `Off` writes no entry to launch from —
        // but answered rather than left to chance: a flag arriving with the
        // setting off is still not a reason to hide the window.
        assert!(opens_window(&login, Off));

        // The flag is matched whole, not by prefix: a date argument or a
        // future `--autostart-something` must not read as a login launch.
        assert!(opens_window(&argv(&["omacal", "--autostart-later"]), Background));
        assert!(opens_window(&argv(&["omacal", "2026-09-01"]), Background));
    }
}

