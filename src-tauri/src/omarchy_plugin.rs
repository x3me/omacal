//! Ships the Omarchy bar widget with the app itself.
//!
//! `omarchy plugin add` wants a git repo with the manifest at its root, and
//! asking every user to hand-copy QML is not an install story. So the app
//! embeds the plugin (`packaging/omarchy-plugin/`, compiled in below) and
//! installs it into `~/.config/omarchy/plugins/omacal.upcoming/` on startup —
//! which means every install path (curl/AppImage, .deb, .rpm, a future AUR
//! package repacking the deb) delivers a working bar widget with zero extra
//! steps.
//!
//! The rules, in order of who they protect:
//!
//! - **A user who removed the widget stays rid of it.** Once this module has
//!   installed the plugin (recorded in the settings table), a missing plugin
//!   directory is read as an uninstall, and nothing is ever written again.
//! - **A hand-installed copy is adopted, not fought.** Plugin dir present but
//!   no record of us installing it: refresh the files when ours are newer,
//!   record it, and never touch where the user put the widget in their bar.
//! - **Enablement happens once, at first install.** Updates rewrite files and
//!   rescan; they never call the enable path again, because Omarchy's enable
//!   also *moves* the widget, and a user's bar layout is theirs.
//!
//! Everything here is best-effort and non-fatal: a calendar app that fails to
//! start because a bar widget could not be copied has its priorities wrong.

use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// Recorded in the settings table after any install or update: the embedded
/// plugin version this app last wrote to disk. Its *presence* is what
/// distinguishes "never installed" from "user removed it".
const INSTALLED_KEY: &str = "omarchy_plugin_installed_version";

const PLUGIN_ID: &str = "omacal.upcoming";

/// The plugin, embedded file by file. `include_str!` keeps the QML's Nerd
/// Font glyphs byte-exact, and keeps this list the single place to extend
/// when the plugin grows a file.
const FILES: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../../packaging/omarchy-plugin/manifest.json")),
    ("Panel.qml", include_str!("../../packaging/omarchy-plugin/Panel.qml")),
    ("read-feed.py", include_str!("../../packaging/omarchy-plugin/read-feed.py")),
    ("CalendarColors.qml", include_str!("../../packaging/omarchy-plugin/CalendarColors.qml")),
    ("DayView.qml", include_str!("../../packaging/omarchy-plugin/DayView.qml")),
    ("MeetingPresence.mjs", include_str!("../../packaging/omarchy-plugin/MeetingPresence.mjs")),
    ("Timeline.mjs", include_str!("../../packaging/omarchy-plugin/Timeline.mjs")),
    ("Model.js", include_str!("../../packaging/omarchy-plugin/Model.js")),
    ("OmacalMark.qml", include_str!("../../packaging/omarchy-plugin/OmacalMark.qml")),
    ("README.md", include_str!("../../packaging/omarchy-plugin/README.md")),
];

/// What one startup pass decides to do. Pure data so the decision table is
/// testable without a filesystem, a database, or an Omarchy install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// First contact: write the files, enable the widget in the bar, record.
    Install,
    /// Ours are newer than what is on disk: rewrite files, rescan, record.
    /// Never re-enable — placement is the user's.
    Update,
    /// A copy we did not put there (hand-install, or a dev box): record it,
    /// and update files only if ours are newer.
    Adopt { refresh: bool },
    /// Installed and current, or removed by the user. Leave everything alone.
    Nothing,
}

/// The decision table described in the module docs.
pub(crate) fn plan(
    recorded: Option<&str>,
    dir_exists: bool,
    on_disk_version: Option<&str>,
    embedded_version: &str,
) -> Action {
    let newer = semver(embedded_version) > on_disk_version.map(semver).unwrap_or((0, 0, 0));
    match (recorded, dir_exists) {
        (None, false) => Action::Install,
        // The record exists and the directory does not: the user removed it.
        (Some(_), false) => Action::Nothing,
        (None, true) => Action::Adopt { refresh: newer },
        (Some(_), true) => {
            if newer {
                Action::Update
            } else {
                Action::Nothing
            }
        }
    }
}

/// "1.2.3" → (1, 2, 3); anything unparseable → (0, 0, 0), which loses every
/// comparison — a garbled manifest gets overwritten, never trusted.
fn semver(v: &str) -> (u64, u64, u64) {
    let mut parts = v.trim().splitn(3, '.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The version inside a manifest.json body, or `None` when it has none.
fn manifest_version(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// Where the plugin lives on an Omarchy system, or `None` off one. Gated on
/// `~/.config/omarchy` already existing: that directory is Omarchy's, and on
/// any other Linux this module must leave no trace.
fn plugin_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let omarchy = Path::new(&home).join(".config/omarchy");
    omarchy.exists().then(|| omarchy.join("plugins").join(PLUGIN_ID))
}

/// Runs one pass in the background. Never in demo mode — demo touches
/// nothing real, and a bar widget is as real as it gets.
pub fn spawn(pool: SqlitePool, demo: bool) {
    if demo {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_once(&pool).await {
            tracing::warn!(%e, "could not install the Omarchy bar widget");
        }
    });
}

async fn run_once(pool: &SqlitePool) -> anyhow::Result<()> {
    let Some(dir) = plugin_dir() else {
        return Ok(()); // Not an Omarchy machine; nothing to do, ever.
    };

    let embedded_version = manifest_version(FILES[0].1)
        .ok_or_else(|| anyhow::anyhow!("embedded manifest has no version"))?;
    let recorded = crate::settings::read(pool, INSTALLED_KEY).await;
    let on_disk_version = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|s| manifest_version(&s));

    let action = plan(
        recorded.as_deref(),
        dir.exists(),
        on_disk_version.as_deref(),
        &embedded_version,
    );
    tracing::debug!(?action, version = %embedded_version, "omarchy plugin pass");

    match action {
        Action::Nothing => return Ok(()),
        Action::Adopt { refresh: false } => {}
        Action::Install | Action::Update | Action::Adopt { refresh: true } => {
            write_files(&dir)?;
            // Loads (or hot-reloads) the code. `-q` because the shell may not
            // be running — a tty session, a moment before autostart — and
            // that is fine; it will discover the files when it starts.
            shell(&["-q", "shell", "rescanPlugins"]);
            if action == Action::Install {
                // Places the widget in the bar. Only ever here: Omarchy's
                // enable also moves the widget, and after first install the
                // bar layout belongs to the user.
                let _ = std::process::Command::new("omarchy-plugin-enable")
                    .args([PLUGIN_ID, "--section", "right"])
                    .output();
            }
        }
    }

    crate::settings::write(pool, INSTALLED_KEY, &embedded_version).await?;
    tracing::info!(?action, version = %embedded_version, "Omarchy bar widget installed");
    Ok(())
}

fn write_files(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, body) in FILES {
        std::fs::write(dir.join(name), body)?;
    }
    Ok(())
}

/// Fire-and-forget call into the running shell.
fn shell(args: &[&str]) {
    let _ = std::process::Command::new("omarchy-shell").args(args).output();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole decision table, one row per rule in the module docs.
    #[test]
    fn the_decision_table_holds() {
        // Fresh machine: install and enable.
        assert_eq!(plan(None, false, None, "1.0.0"), Action::Install);
        // We installed it once; the directory is gone: the user removed it.
        assert_eq!(plan(Some("1.0.0"), false, None, "1.1.0"), Action::Nothing);
        // Hand-installed copy, same version: adopt without touching files.
        assert_eq!(
            plan(None, true, Some("1.0.0"), "1.0.0"),
            Action::Adopt { refresh: false }
        );
        // Hand-installed copy, ours newer: adopt and refresh.
        assert_eq!(
            plan(None, true, Some("0.9.0"), "1.0.0"),
            Action::Adopt { refresh: true }
        );
        // Recorded and current: quiet.
        assert_eq!(plan(Some("1.0.0"), true, Some("1.0.0"), "1.0.0"), Action::Nothing);
        // Recorded and stale on disk: update files, never re-enable.
        assert_eq!(plan(Some("1.0.0"), true, Some("1.0.0"), "1.1.0"), Action::Update);
        // A DOWNGRADE does nothing: an older app must not clobber a newer
        // widget that a newer app (or the user) put there.
        assert_eq!(plan(Some("1.1.0"), true, Some("1.1.0"), "1.0.0"), Action::Nothing);
    }

    #[test]
    fn a_garbled_on_disk_manifest_loses_every_comparison() {
        assert_eq!(
            plan(None, true, Some("not-a-version"), "1.0.0"),
            Action::Adopt { refresh: true }
        );
        assert_eq!(plan(None, true, None, "1.0.0"), Action::Adopt { refresh: true });
    }

    #[test]
    fn the_embedded_manifest_parses_and_names_a_version() {
        // The guard the whole module leans on: if someone edits the packaged
        // manifest and drops the version, this fails at test time rather
        // than warn-and-skip at every user's startup.
        let v = manifest_version(FILES[0].1).expect("embedded manifest.json has a version");
        assert_ne!(semver(&v), (0, 0, 0), "version must be a real semver triple");
    }

    #[test]
    fn every_embedded_file_is_nonempty_and_the_qml_kept_its_glyphs() {
        for (name, body) in FILES {
            assert!(!body.trim().is_empty(), "{name} embedded empty");
        }
        // The join-button glyph (U+F03D) — the canary for the whole class of
        // silently-stripped Nerd Font glyphs.
        let panel = FILES.iter().find(|(n, _)| *n == "Panel.qml").unwrap().1;
        assert!(panel.contains('\u{f03d}'), "Panel.qml lost its glyphs");
    }
}
