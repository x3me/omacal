//! The system's light-or-dark setting, for a desktop with no Omarchy theme.
//!
//! "Follow the desktop theme" only ever followed Omarchy: anywhere else the
//! app resolved its dark palette whatever the system said, so a Mac in light
//! mode opened dark, every time. This reads the setting at launch and keeps
//! it current, repainting when the user flips it — the same live repaint
//! `theme_watch` gives an Omarchy theme switch.
//!
//! - **macOS**: the window's own appearance, and `WindowEvent::ThemeChanged`
//!   (wired in `lib.rs`).
//! - **Linux**: the freedesktop portal's `org.freedesktop.appearance`
//!   `color-scheme`, which GNOME and KDE both publish, and its
//!   `SettingChanged` signal. Not GTK's own dark preference: this app *sets*
//!   that hint (`apply_gtk_dark_hint`), so reading it would read our answer.

use crate::theme::{self, SystemScheme};
use tauri::{AppHandle, Emitter, Manager};

/// A new system setting: remember it, and repaint if the app is following it
/// — `Auto`, and no Omarchy theme taking precedence. A pinned palette is not
/// following anything, so the flip is not news to it.
pub fn changed(app: &AppHandle, scheme: SystemScheme) {
    if theme::system_scheme() == scheme {
        return;
    }
    theme::set_system_scheme(scheme);
    if theme::omarchy_theme_dir().is_some() {
        return;
    }
    let appearance =
        tauri::async_runtime::block_on(crate::settings::appearance(&app.state::<crate::AppState>().pool));
    if appearance.is_pinned() {
        return;
    }
    let palette = theme::resolve(None, appearance);
    tracing::info!(?scheme, "system appearance changed, repainting");
    let dark = palette.is_dark;
    let _ = app.run_on_main_thread(move || crate::apply_gtk_dark_hint(dark));
    let _ = app.emit("theme-changed", palette);
}

/// macOS's answer, from the main window. `None` before there is one.
pub fn from_window_theme(theme: tauri::Theme) -> SystemScheme {
    match theme {
        tauri::Theme::Light => SystemScheme::Light,
        tauri::Theme::Dark => SystemScheme::Dark,
        _ => SystemScheme::NoPreference,
    }
}

/// Reads the setting once, before the first palette is resolved, and on
/// Linux starts listening for changes. Never fatal: without an answer the
/// app is on no preference, which is dark, as it always was.
pub fn init(app: &AppHandle) {
    if cfg!(target_os = "macos") {
        if let Some(t) = app.get_webview_window("main").and_then(|w| w.theme().ok()) {
            theme::set_system_scheme(from_window_theme(t));
        }
        return;
    }
    #[cfg(target_os = "linux")]
    {
        if theme::omarchy_theme_dir().is_some() {
            return; // Omarchy's theme decides; there is nothing to follow here.
        }
        let read = tauri::async_runtime::block_on(async {
            tokio::time::timeout(std::time::Duration::from_millis(500), portal::read()).await
        });
        match read {
            Ok(Ok(scheme)) => theme::set_system_scheme(scheme),
            Ok(Err(e)) => tracing::debug!(%e, "no portal colour scheme; staying on no preference"),
            Err(_) => tracing::debug!("portal did not answer in time; staying on no preference"),
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = portal::watch(app).await {
                tracing::debug!(%e, "portal colour-scheme watch ended");
            }
        });
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use super::*;
    use zbus::zvariant::{OwnedValue, Value};

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "color-scheme";

    async fn proxy(conn: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
        zbus::Proxy::new(
            conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .await
    }

    /// The `u32` inside, however many variants deep: `ReadOne` answers one
    /// level, the older `Read` wraps it in a second.
    pub(super) fn scheme_of(value: &Value<'_>) -> Option<SystemScheme> {
        match value {
            Value::U32(v) => Some(SystemScheme::from_portal(*v)),
            Value::Value(inner) => scheme_of(inner),
            _ => None,
        }
    }

    pub(super) async fn read() -> anyhow::Result<SystemScheme> {
        let conn = zbus::Connection::session().await?;
        let proxy = proxy(&conn).await?;
        let value: OwnedValue = match proxy.call("ReadOne", &(NAMESPACE, KEY)).await {
            Ok(v) => v,
            Err(_) => proxy.call("Read", &(NAMESPACE, KEY)).await?,
        };
        scheme_of(&value).ok_or_else(|| anyhow::anyhow!("color-scheme is not a u32"))
    }

    pub(super) async fn watch(app: AppHandle) -> anyhow::Result<()> {
        use futures_util::StreamExt;
        let conn = zbus::Connection::session().await?;
        let proxy = proxy(&conn).await?;
        let mut signals = proxy.receive_signal("SettingChanged").await?;
        while let Some(msg) = signals.next().await {
            let Ok((namespace, key, value)) = msg.body().deserialize::<(String, String, OwnedValue)>() else {
                continue;
            };
            if namespace == NAMESPACE && key == KEY {
                if let Some(scheme) = scheme_of(&value) {
                    changed(&app, scheme);
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Both shapes the portal answers in, and a value that is not one.
        #[test]
        fn the_scheme_is_read_through_either_wrapping() {
            assert_eq!(scheme_of(&Value::U32(2)), Some(SystemScheme::Light));
            assert_eq!(scheme_of(&Value::Value(Box::new(Value::U32(1)))), Some(SystemScheme::Dark));
            assert_eq!(
                scheme_of(&Value::Value(Box::new(Value::Value(Box::new(Value::U32(0)))))),
                Some(SystemScheme::NoPreference)
            );
            assert_eq!(scheme_of(&Value::Str("dark".into())), None);
        }
    }
}
