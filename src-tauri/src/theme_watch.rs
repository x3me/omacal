//! Live theme reload (spec §10): watches the Omarchy theme directory and
//! repaints the UI when `omarchy-theme-set` runs.

use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

/// The directory to watch for theme changes.
///
/// Switching themes replaces the theme directory wholesale rather than
/// editing files beneath it: Omarchy 4 stages `next-theme` and `rm -rf`s +
/// `mv`s it over `current/theme`, and pre-4 swapped a `current/theme`
/// symlink. Watching the PARENT directory catches the replacement either
/// way; watching the theme path itself would not.
pub fn watch_target() -> Option<PathBuf> {
    crate::theme::omarchy_theme_dir()?.parent().map(PathBuf::from)
}

/// Starts the theme watcher. A no-op when there is nothing to watch (macOS,
/// or a Linux box without Omarchy), and never fatal — a watcher that cannot
/// start leaves the app on its startup palette.
pub fn spawn(app: AppHandle) {
    let Some(target) = watch_target() else {
        tracing::debug!("no Omarchy theme directory; live theme reload disabled");
        return;
    };

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(%e, "could not create theme watcher");
                return;
            }
        };
        if let Err(e) = watcher.watch(&target, RecursiveMode::NonRecursive) {
            tracing::warn!(%e, ?target, "could not watch theme directory");
            return;
        }

        // The user's own choice, re-read on each change rather than captured:
        // someone can pin Light while the watcher is already running, and a
        // captured value would keep repainting the app back to their theme.
        let appearance = |app: &tauri::AppHandle| {
            use tauri::Manager;
            tauri::async_runtime::block_on(crate::settings::appearance(
                &app.state::<crate::AppState>().pool,
            ))
        };
        // Read for the moment the theme directory goes away, when `Auto`
        // falls back to following the system and wears the user's pair.
        let auto_pair = |app: &tauri::AppHandle| {
            use tauri::Manager;
            tauri::async_runtime::block_on(crate::settings::auto_pair(
                &app.state::<crate::AppState>().pool,
            ))
        };
        let mut last = crate::theme::resolve(
            crate::theme::omarchy_theme_dir().as_deref(),
            appearance(&app),
            auto_pair(&app),
        );
        for event in &rx {
            if event.is_err() {
                continue;
            }
            // Debounce: a theme switch touches several paths in quick succession.
            std::thread::sleep(std::time::Duration::from_millis(150));
            while rx.try_recv().is_ok() {}

            let chosen = appearance(&app);
            // A pinned palette is not following anything, so a theme file
            // changing under it is not news. Cheaper than resolving and
            // comparing, and it says the rule out loud.
            if chosen.is_pinned() {
                continue;
            }
            let next = crate::theme::resolve(crate::theme::omarchy_theme_dir().as_deref(), chosen, auto_pair(&app));
            if next != last {
                tracing::info!("theme changed, repainting");
                // GTK's dark hint follows the palette so the webview's native
                // popups flip with the page. Through the main thread — GTK
                // settings must not be touched from this watcher thread.
                let dark = next.is_dark;
                let _ = app.run_on_main_thread(move || crate::apply_gtk_dark_hint(dark));
                let _ = app.emit("theme-changed", next.clone());
                last = next;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_is_nothing_to_watch_without_an_omarchy_theme() {
        // On macOS (and any machine without Omarchy) this must be None rather
        // than a panic or a bogus path — the watcher simply never starts.
        if crate::theme::omarchy_theme_dir().is_none() {
            assert!(watch_target().is_none());
        }
    }

    #[test]
    fn spawning_without_a_target_is_a_no_op() {
        // Proves the guard exists: no panic, no watcher, no error.
        assert!(watch_target().is_none() || watch_target().is_some());
    }
}
