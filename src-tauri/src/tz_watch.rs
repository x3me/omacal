//! Noticing that the system time zone moved out from under a running app.
//!
//! The display zone is captured at process start, on purpose (see
//! `AppSettings::display_timezone`): GTK, the webview and everything else
//! resolve the local zone once and never ask again, so following a zone
//! change live is not something this process can honestly do. What it *can*
//! do is stop pretending nothing happened. On a laptop distro, travelling —
//! suspend in Sofia, resume in Delhi, `timedatectl` (or auto-timezone)
//! re-points `/etc/localtime` — is the normal case, and until now the grid
//! kept drawing Sofia wall-clock times with no word anywhere.
//!
//! So: watch `/etc` the way `theme_watch` watches the theme's parent —
//! `timedatectl set-timezone` swaps the `localtime` symlink, and watching
//! the symlink itself would follow it to a zoneinfo file that never changes.
//! When the zone really moved, record the new name on [`crate::AppState`]
//! (for `get_status`) and emit `system-tz-changed` (for a window already
//! open). The UI turns that into a banner whose one action is the restart
//! that makes it true.
//!
//! Split as ever: which changes deserve the banner is pure and tested; the
//! inotify plumbing is the untested OS half, like `theme_watch::spawn`.
//!
//! macOS too, since 2026-11-19: `/etc/localtime` is a symlink there as well
//! (into `/var/db/timezone/zoneinfo/`), re-pointed when the user travels with
//! automatic time zone on, and the app had no banner at all. The same watch,
//! through FSEvents, on `/etc`'s real path.

/// The event a moved zone emits, carrying the new IANA name.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) const EVENT: &str = "system-tz-changed";

/// Whether a `/etc/localtime` transition deserves the banner. Only asked of
/// a process that follows the system zone — a pinned one never watches (see
/// [`spawn`]).
///
/// Both names must be known and different. A zone that merely *becomes*
/// readable (or stops being readable) is not evidence anything moved — a
/// distro swapping a copied file for a symlink lands here — and a banner
/// that cries wolf once has taught the user to dismiss the one that
/// matters.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn should_announce(last: Option<&str>, next: Option<&str>) -> bool {
    matches!((last, next), (Some(a), Some(b)) if a != b)
}

/// The IANA name inside a `localtime` symlink target —
/// `../usr/share/zoneinfo/Europe/Sofia` → `Europe/Sofia`. The `posix/` and
/// `right/` trees are the same zones filed twice, so their prefix is not
/// part of the name; without stripping it, `zoneinfo/Europe/Sofia` →
/// `zoneinfo/posix/Europe/Sofia` would banner a move that never happened.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn zone_name_of(target: &str) -> Option<String> {
    let (_, name) = target.rsplit_once("zoneinfo/")?;
    let name = name
        .strip_prefix("posix/")
        .or_else(|| name.strip_prefix("right/"))
        .unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

/// The system zone's current name. The symlink is the rule; `/etc/timezone`
/// is the fallback for distros that copy the zoneinfo file instead of
/// linking it, where the link read fails but the name is written down.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_system_zone() -> Option<String> {
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        if let Some(name) = zone_name_of(&target.to_string_lossy()) {
            return Some(name);
        }
    }
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Starts the watcher. Never fatal — a box where `/etc` cannot be watched
/// just goes without the banner, exactly as every install did before this
/// module existed.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn spawn(app: tauri::AppHandle) {
    use notify::{RecursiveMode, Watcher};
    use tauri::{Emitter, Manager};

    // `TZ` in the environment at launch — our own display-zone export or the
    // user's shell — pins this process's zone, and it is process state, fixed
    // for this process's whole life (the same reason the banner exists at
    // all). A pinned process has no move to announce, whatever the system
    // does, so it holds no watch: `/etc` is busy, and a watcher there wakes
    // for every `passwd` and `ld.so.cache` read on the box (#134).
    if std::env::var_os("TZ").is_some() {
        tracing::debug!("TZ is pinned; not watching /etc for timezone changes");
        return;
    }

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(%e, "could not create timezone watcher");
                return;
            }
        };
        // `/etc` by its real path: on macOS it is a symlink to `/private/etc`,
        // and FSEvents reports what happens at the real one.
        let etc = std::fs::canonicalize("/etc").unwrap_or_else(|_| "/etc".into());
        if let Err(e) = watcher.watch(&etc, RecursiveMode::NonRecursive) {
            tracing::warn!(%e, "could not watch /etc for timezone changes");
            return;
        }

        let mut last = read_system_zone();
        for event in &rx {
            // `/etc` is a busy directory; only `localtime` is our business.
            let Ok(event) = event else { continue };
            let localtime = std::ffi::OsStr::new("localtime");
            if !event.paths.iter().any(|p| p.file_name() == Some(localtime)) {
                continue;
            }
            // Debounce: the symlink swap arrives as a remove/create burst.
            std::thread::sleep(std::time::Duration::from_millis(150));
            while rx.try_recv().is_ok() {}

            let next = read_system_zone();
            if should_announce(last.as_deref(), next.as_deref()) {
                let name = next.clone().expect("announce requires a known next zone");
                tracing::info!(%name, "system timezone changed under a running app");
                if let Some(state) = app.try_state::<crate::AppState>() {
                    *state.system_tz_change.lock().expect("tz change poisoned") =
                        Some(name.clone());
                }
                let _ = app.emit(EVENT, name);
            }
            // Track every readable state, announced or not: after A→B→A the
            // calendar is right again, and B→A must compare against B.
            if next.is_some() {
                last = next;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule's two gates, each closed on its own: an unknown edge never
    /// banners, and the same zone re-announced never banners. Only known →
    /// different acts. (A pinned process never gets this far: it never
    /// watches.)
    #[test]
    fn only_a_real_move_of_the_zone_is_announced() {
        assert!(
            should_announce(Some("Europe/Sofia"), Some("Asia/Kolkata")),
            "the travel case is the whole feature"
        );
        assert!(
            !should_announce(Some("Europe/Sofia"), Some("Europe/Sofia")),
            "re-pointing to the same zone is not news"
        );
        assert!(
            !should_announce(None, Some("Europe/Sofia")),
            "a zone becoming readable is not evidence it moved"
        );
        assert!(
            !should_announce(Some("Europe/Sofia"), None),
            "a zone becoming unreadable is not a move either"
        );
    }

    /// The symlink shapes that exist in the field: absolute, relative,
    /// `posix/`- and `right/`-prefixed — and the two-level zone names that
    /// must survive whole.
    #[test]
    fn the_zone_name_survives_every_symlink_shape() {
        for target in [
            // macOS: `/etc/localtime` points into `/var/db/timezone`.
            "/var/db/timezone/zoneinfo/Europe/Sofia",
            "/usr/share/zoneinfo/Europe/Sofia",
            "../usr/share/zoneinfo/Europe/Sofia",
            "/usr/share/zoneinfo/posix/Europe/Sofia",
            "/usr/share/zoneinfo/right/Europe/Sofia",
        ] {
            assert_eq!(zone_name_of(target).as_deref(), Some("Europe/Sofia"), "{target}");
        }
        assert_eq!(
            zone_name_of("/usr/share/zoneinfo/America/Argentina/Ushuaia").as_deref(),
            Some("America/Argentina/Ushuaia"),
            "three-level names exist and must not be truncated"
        );
    }

    /// Targets that are not zoneinfo entries at all resolve to nothing — and
    /// nothing is what keeps `should_announce`'s unknown gate closed.
    #[test]
    fn a_target_outside_the_zoneinfo_tree_is_not_a_zone() {
        assert_eq!(zone_name_of("/etc/localtime.bak"), None);
        assert_eq!(zone_name_of(""), None);
        assert_eq!(zone_name_of("/usr/share/zoneinfo/"), None, "the tree root names no zone");
    }
}
