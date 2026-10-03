//! Restarting omacal without trusting the teardown that betrayed it.
//!
//! `app.restart()` leaves through `exit()`, and `exit()` runs every
//! exit-time destructor GTK, WebKitGTK and zbus have registered — a
//! gauntlet that, on the field evidence of 2026-08-26, the old instance
//! does not survive: after each in-app update the outgoing main hung
//! inside that teardown forever (a windowless zombie executing a deleted
//! AppImage, one per update, with its stale `/tmp/.mount_omacal*` beside
//! it), and its WebKit renderer aborted out of glibc's exit-time
//! consistency checks — a coredump notification per update.
//!
//! So a restart here spawns the fresh image and then leaves through
//! `_exit`, running no exit handlers at all. That is not a shortcut but
//! the design: everything this app must not lose is already durable —
//! calendar state lives in SQLite behind WAL, tokens in the keyring,
//! settings in the same database, every write transactional — and the one
//! thing teardown "orderliness" was buying was the hang.
//!
//! The renderer was first left to itself: orphaned instead of walked
//! through WebKit's shutdown, it would get its sockets closed and go down
//! the boring path. The field evidence of 2026-09-06 says the boring path
//! is `exit()`, and `exit()` is the same gauntlet — the renderer aborted
//! out of glibc's exit-time checks sixteen seconds after every update, one
//! coredump notification each. Worse, while it and the network process
//! were still standing they held the AppImage mount busy, so the AppImage
//! runtime under us could not unmount and never left: after two updates
//! the box carried two idle runtimes executing deleted files, and three
//! `/tmp/.mount_omacal*` mounts. So before spawning, [`stop_webkit_helpers`]
//! sends WebKit's helper processes SIGTERM — they are about to lose their
//! UI process either way, and a signal ends them without any `exit()` —
//! waits the moment they need, and the runtime then finds its mount free.
//!
//! The image to spawn is `$APPIMAGE` when set — the file the updater just
//! replaced, which is exactly the point — and the current executable
//! otherwise (a .deb's binary, the macOS .app, a dev build). Resolution is
//! the pure, tested half; the spawn and the `_exit` are the shell.

use std::path::PathBuf;

/// How the fresh instance should be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Restart {
    /// Run this file directly: the AppImage, a .deb's binary, a dev build.
    Exec(PathBuf),
    /// Hand this `.app` to LaunchServices via `open`.
    ///
    /// **A macOS app is the bundle, not the Mach-O inside it.** Spawning
    /// `Contents/MacOS/omacal` yields a process LaunchServices never
    /// registered: no Dock ownership, no activation, and a bundle the
    /// updater has just replaced underneath it. `open -n` is how a Mac
    /// starts an app, so it is how omacal restarts itself into one.
    Bundle(PathBuf),
}

/// What a restart should start.
///
/// The AppImage the runtime names wins — post-update those are the *new*
/// bytes at that same path, which is the whole point of restarting. Failing
/// that, on macOS the enclosing `.app` if this executable lives inside one,
/// and otherwise the executable itself.
pub(crate) fn restart_target(
    appimage: Option<std::ffi::OsString>,
    current_exe: Option<PathBuf>,
    is_macos: bool,
) -> Option<Restart> {
    if let Some(img) = appimage {
        return Some(Restart::Exec(PathBuf::from(img)));
    }
    let exe = current_exe?;
    if is_macos {
        if let Some(bundle) = enclosing_app_bundle(&exe) {
            return Some(Restart::Bundle(bundle));
        }
    }
    Some(Restart::Exec(exe))
}

/// The `.app` directory containing `exe`, for the canonical
/// `Foo.app/Contents/MacOS/foo` layout — and only that layout, so a stray
/// `.app` component somewhere else in the path cannot capture the launch.
fn enclosing_app_bundle(exe: &std::path::Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? == "app" {
        Some(bundle.to_path_buf())
    } else {
        None
    }
}

/// The initial GUI launch has the same macOS ownership rule as a restart:
/// an executable entered directly through `Contents/MacOS` is not a
/// LaunchServices app. CLI invocations exit before this decision is reached,
/// so only an unregistered GUI process is handed back to its bundle.
pub(crate) fn initial_gui_launch_target(
    current_exe: Option<PathBuf>,
    is_macos: bool,
    bundle_registered: bool,
) -> Option<PathBuf> {
    if is_macos && !bundle_registered {
        return current_exe.as_deref().and_then(enclosing_app_bundle);
    }
    None
}

/// Reopen a macOS GUI launch through LaunchServices when the app binary was
/// invoked directly. Keep running if `open` fails so a broken association
/// never turns a usable app launch into a silent exit.
pub(crate) fn relaunch_initial_macos_gui_if_unregistered() {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSBundle;

        let bundle_registered = NSBundle::mainBundle().bundleIdentifier().is_some();
        if let Some(bundle) = initial_gui_launch_target(
            std::env::current_exe().ok(),
            true,
            bundle_registered,
        ) {
            match std::process::Command::new("/usr/bin/open").arg(&bundle).spawn() {
                Ok(_) => std::process::exit(0),
                Err(e) => eprintln!("omacal: could not reopen bundled app: {e}"),
            }
        }
    }
}

/// Spawns the fresh instance and leaves without teardown. Never returns.
///
/// The 400ms callers sleep before invoking this (the "reply must reach the
/// webview first" rule from `settings::restart_app`) is unchanged; what
/// changed is only how the old process leaves once the reply is out.
pub(crate) fn hard_restart() -> ! {
    // Before the spawn, not after: the new instance must not find these
    // still alive (nothing of the new one is a child of this process, so
    // the sweep cannot touch it), and the old window going blank for the
    // tens of milliseconds this takes is a restart looking like one.
    if cfg!(target_os = "linux") {
        stop_webkit_helpers();
    }
    match restart_target(
        std::env::var_os("APPIMAGE"),
        std::env::current_exe().ok(),
        cfg!(target_os = "macos"),
    ) {
        Some(Restart::Exec(target)) => {
            let mut cmd = std::process::Command::new(&target);
            // **Never hand the replacement a path into the mount we are
            // leaving.** See `env_without_appdir` for what that costs.
            if let Some(appdir) = std::env::var("APPDIR").ok().filter(|d| !d.is_empty()) {
                for (key, value) in
                    env_without_appdir(&appdir, std::env::vars().collect::<Vec<_>>())
                {
                    match value {
                        Some(v) => cmd.env(key, v),
                        None => cmd.env_remove(key),
                    };
                }
                // The working directory is inside that mount too — AppRun
                // moves there — and an inherited cwd pins it just as a
                // mapping does.
                cmd.current_dir("/");
            }
            if let Err(e) = cmd.spawn() {
                tracing::error!(%e, ?target, "restart could not spawn the new instance");
            }
        }
        Some(Restart::Bundle(bundle)) => {
            // `-n` because the instance asking for this restart is still
            // alive for the next microsecond: without it `open` would find
            // the dying app and merely try to activate it, and nothing
            // would come back. The single-instance socket sorts out any
            // overlap — and a stale one left by `_exit` refuses connections,
            // so the newcomer proceeds rather than deferring to a ghost.
            if let Err(e) = std::process::Command::new("/usr/bin/open").arg("-n").arg(&bundle).spawn()
            {
                tracing::error!(%e, ?bundle, "restart could not open the app bundle");
            }
        }
        None => tracing::error!("restart found nothing to spawn"),
    }
    // `_exit`, not `exit`: no atexit handlers, no destructor gauntlet, no
    // zombie. See the module doc for why nothing of value is lost.
    #[cfg(unix)]
    unsafe {
        libc::_exit(0)
    }
    #[cfg(not(unix))]
    std::process::exit(0)
}

/// The environment changes that stop a replacement pinning the mount its
/// predecessor is leaving, as `(key, Some(new value) | None to drop)`.
///
/// **The leak this closes.** An AppImage runs from a squashfs mounted at
/// `/tmp/.mount_omacalXXXX` and served by a FUSE process; `AppRun` points a
/// dozen variables at that directory. `hard_restart` spawns the replacement
/// with this process's environment, so the *new* instance loaded
/// `GSETTINGS_SCHEMA_DIR`'s `gschemas.compiled` from the *old* mount and
/// mmapped it — which pins that mount for the life of the new process. Its
/// FUSE server can then never unmount, and sits in `fuse_dev_do_read`
/// forever. One stranded process and one stale mount per restart, found on
/// the real box with three mounts where there should have been one
/// (2026-09-09).
///
/// #63 reaped the WebKit helpers, which is a different leak with the same
/// symptom; this is the one that outlived it.
///
/// **Colon lists lose only the offending entries.** `XDG_DATA_DIRS` carries
/// the system's own directories beside the bundle's, and dropping it whole
/// would take `/usr/share` with it — the new `AppRun` prepends to what it
/// inherits rather than rebuilding it.
pub(crate) fn env_without_appdir(
    appdir: &str,
    vars: Vec<(String, String)>,
) -> Vec<(String, Option<String>)> {
    vars.into_iter()
        .filter(|(_, v)| v.contains(appdir))
        .map(|(k, v)| {
            // A single value that *is* the path goes; a list keeps whatever
            // does not point inside the mount.
            let kept: Vec<&str> = v.split(':').filter(|part| !part.contains(appdir)).collect();
            (k, (!kept.is_empty()).then(|| kept.join(":")))
        })
        .collect()
}

/// One line of `/proc/<pid>/stat`: the pid, the parent's pid, the state
/// letter and the command name.
///
/// The name sits in parentheses and may itself hold spaces or a `)`, so the
/// fields after it are counted from the *last* `)`, never the first.
pub(crate) fn parse_stat(stat: &str) -> Option<(u32, u32, char, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let pid: u32 = stat[..open].trim().parse().ok()?;
    let comm = stat[open + 1..close].to_string();
    let mut rest = stat[close + 1..].split_whitespace();
    let state = rest.next()?.chars().next()?;
    let ppid: u32 = rest.next()?.parse().ok()?;
    Some((pid, ppid, state, comm))
}

/// Whether a process is one of ours to stop: a live child of `me` whose
/// name is one of WebKit's helpers. The kernel keeps fifteen bytes of a
/// name, so the renderer is `WebKitWebProces` and the network process
/// `WebKitNetworkPr`; the prefix is the stable part. `bwrap` and
/// `xdg-dbus-proxy` are the sandbox WebKit puts its D-Bus proxy in where it
/// can (seen under a private session bus, not on the box's own), and they
/// bind the same mount. A zombie is already gone and holds nothing, so it is
/// not a target — counting one would only make the wait below run out its
/// clock.
pub(crate) fn is_webkit_helper_of(me: u32, pid: u32, ppid: u32, state: char, comm: &str) -> bool {
    ppid == me
        && pid != me
        && state != 'Z'
        && state != 'X'
        && (comm.starts_with("WebKit") || comm == "bwrap" || comm == "xdg-dbus-proxy")
}

/// The pids of this process's live WebKit helpers, from `/proc`. Empty
/// where there is no `/proc` to read, which is every platform but Linux.
fn webkit_helpers(me: u32) -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|entry| {
        let entry = entry.ok()?;
        entry.file_name().to_str()?.parse::<u32>().ok()?;
        let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
        let (pid, ppid, state, comm) = parse_stat(&stat)?;
        is_webkit_helper_of(me, pid, ppid, state, &comm).then_some(pid)
    })
    .collect()
}

/// Stops the WebKit helpers this process launched — the renderer and the
/// network process — and reaps them, so that nothing of ours is left
/// holding the AppImage mount when this process leaves. See the module doc
/// for the field evidence.
///
/// SIGTERM first, and a short wait for them to go; anything still standing
/// after that gets SIGKILL, and a last sweep right before returning catches
/// a helper WebKit relaunched in the meantime. Reaping is what makes a
/// stopped helper disappear from the table rather than sit there as a
/// zombie for the whole wait.
pub(crate) fn stop_webkit_helpers() {
    let me = std::process::id();
    let first = webkit_helpers(me);
    if first.is_empty() {
        return;
    }
    for &pid in &first {
        // SAFETY: plain libc calls on pids read from /proc; a pid that has
        // already gone makes kill fail, which is the outcome wanted anyway.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    // Wait for the ones signalled to exit, reaping as they go. The check is
    // on those pids, not on a fresh listing: a fresh listing hides a zombie
    // (which still needs reaping) and shows a relaunch (which the sweep
    // below is for).
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        if reap(&first) || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Whatever stands now — a straggler, or a helper WebKit relaunched in
    // the meantime — is killed outright and reaped.
    let left = webkit_helpers(me);
    for &pid in &left {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    // The live-process sweep excludes zombies. Keep the original children
    // in the reap set too: one may have exited as the first deadline elapsed.
    let mut pending = first.clone();
    pending.extend(left.iter().copied());
    pending.sort_unstable();
    pending.dedup();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    loop {
        if reap(&pending) || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    tracing::info!(stopped = first.len(), killed = left.len(), "webkit helpers stopped before the restart");
}

/// Collects exited children and reports whether every child has been reaped.
/// A child can exit just after WNOHANG returns zero. Checking its liveness
/// separately would then see a zombie and stop waiting before collecting it.
fn reap(pids: &[u32]) -> bool {
    let mut done = true;
    for &pid in pids {
        let mut status = 0;
        let result = unsafe {
            libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG)
        };
        // Another owner (e.g. WebKit) may already have collected this child.
        done &= result == pid as libc::pid_t
            || (result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD));
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AppImage path wins even though a mounted AppImage's
    /// `current_exe` exists — the mount holds the *old* bytes, and the
    /// whole point of restarting after an update is executing the new
    /// ones at `$APPIMAGE`.
    #[test]
    fn the_appimage_outranks_the_mounted_executable() {
        assert_eq!(
            restart_target(
                Some("/home/u/.local/bin/omacal".into()),
                Some(PathBuf::from("/tmp/.mount_omacal1/usr/bin/omacal")),
                false,
            ),
            Some(Restart::Exec(PathBuf::from("/home/u/.local/bin/omacal")))
        );
        assert_eq!(
            restart_target(None, Some(PathBuf::from("/usr/bin/omacal")), false),
            Some(Restart::Exec(PathBuf::from("/usr/bin/omacal")))
        );
        assert_eq!(restart_target(None, None, false), None);
    }

    /// **A macOS app is the bundle.** Restarting by exec'ing the Mach-O
    /// inside `Contents/MacOS` gives a process LaunchServices never
    /// registered — which is what the field report of an update that does
    /// not come back properly looks like.
    #[test]
    fn macos_restarts_the_bundle_and_not_the_binary_inside_it() {
        assert_eq!(
            restart_target(
                None,
                Some(PathBuf::from("/Applications/omacal.app/Contents/MacOS/omacal")),
                true,
            ),
            Some(Restart::Bundle(PathBuf::from("/Applications/omacal.app")))
        );
    }

    #[test]
    fn unregistered_macos_gui_launch_reopens_the_bundle() {
        assert_eq!(
            initial_gui_launch_target(
                Some(PathBuf::from("/Applications/OmaCal.app/Contents/MacOS/omacal")),
                true,
                false,
            ),
            Some(PathBuf::from("/Applications/OmaCal.app"))
        );
    }

    #[test]
    fn registered_macos_gui_launch_stays_in_the_bundle_process() {
        assert_eq!(
            initial_gui_launch_target(
                Some(PathBuf::from("/Applications/OmaCal.app/Contents/MacOS/omacal")),
                true,
                true,
            ),
            None
        );
    }

    #[test]
    fn non_macos_gui_launch_stays_direct_even_without_bundle_registration() {
        assert_eq!(
            initial_gui_launch_target(
                Some(PathBuf::from("/Applications/OmaCal.app/Contents/MacOS/omacal")),
                false,
                false,
            ),
            None
        );
    }

    #[test]
    fn macos_gui_launch_outside_a_bundle_stays_direct() {
        assert_eq!(
            initial_gui_launch_target(
                Some(PathBuf::from("/Users/u/omacal/target/debug/omacal")),
                true,
                false,
            ),
            None
        );
    }

    /// Only the canonical layout counts. A bare binary on a Mac — a
    /// `cargo tauri dev` build, a Homebrew-style install — is exec'd as it
    /// always was, and a stray `.app` elsewhere in the path captures
    /// nothing.
    #[test]
    fn a_mac_binary_outside_a_bundle_is_still_exec_d() {
        assert_eq!(
            restart_target(None, Some(PathBuf::from("/Users/u/omacal/target/debug/omacal")), true),
            Some(Restart::Exec(PathBuf::from("/Users/u/omacal/target/debug/omacal")))
        );
        assert_eq!(
            restart_target(None, Some(PathBuf::from("/Users/u/x.app/bin/omacal")), true),
            Some(Restart::Exec(PathBuf::from("/Users/u/x.app/bin/omacal")))
        );
    }

    /// Linux is untouched by the macOS branch even for a path that looks
    /// like a bundle — the flag decides, not the shape.
    #[test]
    fn the_bundle_rule_is_macos_only() {
        assert_eq!(
            restart_target(None, Some(PathBuf::from("/opt/omacal.app/Contents/MacOS/omacal")), false),
            Some(Restart::Exec(PathBuf::from("/opt/omacal.app/Contents/MacOS/omacal")))
        );
    }

    /// The two lines the box actually shows, and one with a name that
    /// would fool a parser counting from the first `)`.
    #[test]
    fn a_stat_line_yields_pid_parent_state_and_name() {
        assert_eq!(
            parse_stat("325025 (WebKitWebProces) S 324939 1889743 1889743 0 -1 41943"),
            Some((325025, 324939, 'S', "WebKitWebProces".to_string()))
        );
        assert_eq!(
            parse_stat("325006 (WebKitNetworkPr) S 324939 1889743 1889743 0 -1 41943"),
            Some((325006, 324939, 'S', "WebKitNetworkPr".to_string()))
        );
        assert_eq!(
            parse_stat("12 (a b) c) R 1 12 12 0 -1 4194560"),
            Some((12, 1, 'R', "a b) c".to_string()))
        );
        assert_eq!(parse_stat("garbage"), None);
    }

    /// Only a live, WebKit-named child of this process is a target: not the
    /// new instance we are about to spawn, not a grandchild, not a zombie,
    /// and not the process itself.
    #[test]
    fn only_live_webkit_children_are_stopped() {
        let me = 100;
        assert!(is_webkit_helper_of(me, 200, me, 'S', "WebKitWebProces"));
        assert!(is_webkit_helper_of(me, 201, me, 'S', "WebKitNetworkPr"));
        assert!(is_webkit_helper_of(me, 205, me, 'S', "bwrap"), "WebKit's sandbox for its proxy");
        assert!(is_webkit_helper_of(me, 206, me, 'S', "xdg-dbus-proxy"));
        assert!(!is_webkit_helper_of(me, 202, me, 'S', "omacal"), "the new instance");
        assert!(!is_webkit_helper_of(me, 203, 200, 'S', "WebKitWebProces"), "a grandchild");
        assert!(!is_webkit_helper_of(me, 204, me, 'Z', "WebKitWebProces"), "a zombie");
        assert!(!is_webkit_helper_of(me, me, 1, 'S', "WebKitWebProces"), "itself");
    }

    /// The real thing, on Linux: a child that is named like a WebKit helper
    /// (a symlink to `sleep`, since the kernel names a process after the
    /// file it executed) is found, stopped and reaped, and a child that is
    /// not named like one is left alone.
    /// The real environment from the box this was found on, trimmed to the
    /// variables that mattered. `GSETTINGS_SCHEMA_DIR` is the one that
    /// actually pinned the mount — the new instance mmapped
    /// `gschemas.compiled` out of its predecessor's squashfs and held it
    /// open for good.
    #[test]
    fn the_replacement_inherits_nothing_pointing_into_the_old_mount() {
        let appdir = "/tmp/.mount_omacalbkIfJe";
        let changes = env_without_appdir(appdir, vec![
            ("APPDIR".into(), appdir.into()),
            ("GSETTINGS_SCHEMA_DIR".into(), format!("{appdir}/usr/share/glib-2.0/schemas")),
            ("LD_LIBRARY_PATH".into(), format!("{appdir}/usr/lib")),
            ("HOME".into(), "/home/plamen".into()),
            ("APPIMAGE".into(), "/home/plamen/.local/bin/omacal".into()),
        ]);
        let by_key = |k: &str| changes.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());

        // Every path into the mount is dropped outright.
        assert_eq!(by_key("APPDIR"), Some(None));
        assert_eq!(by_key("GSETTINGS_SCHEMA_DIR"), Some(None), "the one that pinned it");
        assert_eq!(by_key("LD_LIBRARY_PATH"), Some(None));
        // Everything else is left alone — not merely re-set to itself, but
        // absent from the changes, so the spawn does not touch it.
        assert_eq!(by_key("HOME"), None, "an unrelated variable was disturbed");
        assert_eq!(
            by_key("APPIMAGE"), None,
            "APPIMAGE names the file, not the mount — the replacement needs it",
        );
    }

    /// A colon list keeps the entries that are not in the mount. Dropping
    /// `XDG_DATA_DIRS` whole would take `/usr/share` with it, and the new
    /// `AppRun` prepends to what it inherits rather than rebuilding it.
    #[test]
    fn a_path_list_loses_only_the_entries_inside_the_old_mount() {
        let appdir = "/tmp/.mount_omacalbkIfJe";
        let changes = env_without_appdir(appdir, vec![(
            "XDG_DATA_DIRS".into(),
            format!("{appdir}/usr/share:/usr/local/share:/usr/share"),
        )]);
        assert_eq!(
            changes,
            vec![("XDG_DATA_DIRS".to_string(), Some("/usr/local/share:/usr/share".to_string()))],
        );
    }

    /// Off an AppImage there is no `APPDIR`, and nothing should be touched —
    /// a dev build and a `.deb` install both restart through this path.
    #[test]
    fn a_build_that_is_not_an_appimage_has_its_environment_left_whole() {
        let changes = env_without_appdir("/tmp/.mount_omacalXXXX", vec![
            ("HOME".into(), "/home/plamen".into()),
            ("XDG_DATA_DIRS".into(), "/usr/local/share:/usr/share".into()),
        ]);
        assert!(changes.is_empty(), "changed {changes:?}");
    }

    #[test]
    fn a_child_named_like_a_webkit_helper_is_stopped_and_an_ordinary_one_is_not() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("omacal-restart-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("WebKitFakeHelper");
        let _ = std::fs::remove_file(&fake);
        std::os::unix::fs::symlink("/bin/sleep", &fake).unwrap();
        let mut helper = std::process::Command::new(&fake).arg("30").spawn().unwrap();
        let mut bystander = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let me = std::process::id();
        // Give the kernel a moment to have the child exec'd and named.
        let seen = (0..50).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            webkit_helpers(me).contains(&helper.id())
        });
        assert!(seen, "the fake helper must be listed as ours");

        stop_webkit_helpers();

        assert!(!webkit_helpers(me).contains(&helper.id()), "the fake helper must be gone");
        let reaped = (0..50).any(|_| {
            let gone = !std::path::Path::new(&format!("/proc/{}", helper.id())).exists();
            if !gone {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            gone
        });
        assert!(reaped, "and reaped, not left as a zombie");
        assert!(
            std::path::Path::new(&format!("/proc/{}", bystander.id())).exists(),
            "the ordinary child is untouched"
        );
        bystander.kill().unwrap();
        bystander.wait().unwrap();
        // Already reaped by `stop_webkit_helpers`; this only satisfies the
        // handle (and the zombie lint) and gets ECHILD back.
        let _ = helper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
