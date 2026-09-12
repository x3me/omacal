//! Opening an external URL — without handing the target application this
//! process's AppImage environment.
//!
//! The AppImage runtime exports `LD_LIBRARY_PATH` (and friends) pointing at
//! the image's bundled libraries, which is what lets *this* binary run on a
//! distro it wasn't built on. A child process inherits all of it — and a
//! browser started that way loads our bundled, older libraries against its
//! own binary and dies on the first symbol they lack. Concretely (issue #1,
//! verified on Arch): chromium needs `BrotliDecoderAttachDictionary`, the
//! bundled `libbrotlidec.so.1` predates it, and sign-in fails before Google
//! is ever contacted. The variables are for us, not our children.
//!
//! So: every place this app opens a browser goes through [`open_external`],
//! which strips the AppImage's environment from the launcher's child — and
//! only when `APPDIR` is set, which is the AppImage runtime's own signal and
//! set by nothing else. The .deb, the AUR package, the Flatpak and a dev run
//! take the untouched path, byte-for-byte what `open::that` always did.
//!
//! `PATH` is filtered rather than removed: the AppImage prepends its own
//! `usr/bin`, so an unfiltered child resolves the *bundled* Debian
//! `xdg-open` — which execs the browser directly and re-sanitises nothing —
//! instead of the host's own launcher, which knows how the host opens
//! things. (The release workflow also stops shipping `usr/bin/xdg-open` for
//! this reason; the filter stays because a user may be running an AppImage
//! from before that change for a long time.)
//!
//! Split as ever: which variables go, and what a filtered `PATH` looks like,
//! are pure and tested. The spawn loop is OS integration, the untested half.

use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What the AppImage runtime and its GTK hook export for this process's own
/// dynamic linking — the union of what linuxdeploy's `AppRun.wrapped` sets
/// and what `apprun-hooks/linuxdeploy-plugin-gtk.sh` adds, checked against a
/// shipped image rather than assumed. Every one of these poisons a child
/// that has its own idea of where its libraries live.
const APPIMAGE_ONLY_VARS: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "GTK_PATH",
    "GIO_EXTRA_MODULES",
    "GDK_PIXBUF_MODULE_FILE",
    "GSETTINGS_SCHEMA_DIR",
    "GST_PLUGIN_SYSTEM_PATH",
    "QT_PLUGIN_PATH",
    "PERLLIB",
    "PYTHONPATH",
    "PYTHONHOME",
];

/// `path` with every component that lives under `appdir` removed.
///
/// Component-prefix matching, not substring: `/tmp/.mount_om123/usr/bin`
/// goes because it *starts with* the AppDir; a hypothetical
/// `/home/user/tmp/.mount_om123-lookalike` stays because it does not.
fn path_without_appdir(path: &str, appdir: &str) -> String {
    path.split(':')
        .filter(|c| !c.starts_with(appdir))
        .collect::<Vec<_>>()
        .join(":")
}

/// Strips the AppImage environment from one launcher invocation.
///
/// Takes `appdir` as an argument rather than reading the environment so the
/// tests can exercise it without mutating process-global state — env vars
/// are shared across a parallel test run, and a test that sets `APPDIR`
/// poisons its neighbours exactly the way this module exists to stop.
fn sanitize(cmd: &mut Command, appdir: &OsStr) {
    for var in APPIMAGE_ONLY_VARS {
        cmd.env_remove(var);
    }
    // The child's PATH decides both which launcher name resolves and what
    // that launcher can see — `std::process` resolves the program against
    // the PATH *in the child's env* once one is set.
    if let (Ok(path), Some(dir)) = (std::env::var("PATH"), appdir.to_str()) {
        cmd.env("PATH", path_without_appdir(&path, dir));
    }
}

/// Turns the ordinary Zoom links calendars carry into the protocol handled by
/// Zoom itself. On Omarchy that handler opens the meeting directly in the Zoom
/// web app; a native Zoom installation can own the same protocol. Vanity URLs
/// stay in the browser because they do not contain the meeting number the
/// protocol requires.
fn zoom_join_uri(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "https" || !is_zoom_host(host) {
        return None;
    }

    let segments: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let meeting = match segments.as_slice() {
        ["j" | "w", meeting, ..] | ["wc", "join", meeting, ..] => *meeting,
        _ => return None,
    };
    if !meeting.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return None;
    }
    // Emptiness is asked of the *digits*, not of the segment they came from:
    // `/j/---` passes a dash-tolerant check and then filters down to nothing,
    // which built a join URI with `confno=` empty and handed it to Zoom
    // instead of falling back to the link the calendar actually carried.
    let meeting: String = meeting.chars().filter(char::is_ascii_digit).collect();
    if meeting.is_empty() {
        return None;
    }
    let password = url
        .query_pairs()
        .find(|(key, _)| key == "pwd")
        .map(|(_, value)| value.into_owned());

    let mut deep = reqwest::Url::parse("zoommtg://zoom.us/join").expect("static Zoom URI");
    {
        let mut query = deep.query_pairs_mut();
        query
            .append_pair("action", "join")
            .append_pair("confno", &meeting);
        if let Some(password) = password.filter(|p| !p.is_empty()) {
            query.append_pair("pwd", &password);
        }
    }
    Some(deep.to_string())
}

/// How long a launcher is given to fail before it is believed.
///
/// **A launcher that has not exited has not failed.** Issue #100: where
/// `xdg-open` runs the browser in the *foreground* — the ordinary case when
/// the browser was not already running — the launcher lives as long as the
/// browser session. Waiting for its exit meant waiting for the user to quit
/// their browser, so `open_external` never returned, the OAuth accept loop
/// below it never started, and sign-in hung with nothing on screen to say
/// why. The same launch with the browser already up returned in 0s, which is
/// why it looked like a browser problem and was reported against three of
/// them.
///
/// The exit status is still worth having, so the wait is bounded rather than
/// abandoned: issue #1's symptom was `xdg-open` exiting 4 *immediately*,
/// having drawn nothing, and that is a real failure the user should be told
/// about. Long enough to catch that, short enough that nobody feels a launch
/// that worked.
const LAUNCHER_VERDICT: Duration = Duration::from_millis(300);

/// How often the launcher is checked while inside [`LAUNCHER_VERDICT`].
const LAUNCHER_POLL: Duration = Duration::from_millis(10);

/// Starts `cmd`, and waits only long enough to catch a launcher that fails on
/// the spot.
///
/// A launcher still running when [`LAUNCHER_VERDICT`] is up has taken the URL
/// and is left to it — reaped on a detached thread rather than waited on, so
/// a browser session that outlives the sign-in does not leave a zombie behind
/// for the life of the app.
fn launch(cmd: &mut Command) -> std::io::Result<()> {
    let mut child = cmd.spawn()?;
    let started = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                return Err(std::io::Error::other(format!(
                    "launcher {:?} exited with {status}",
                    cmd.get_program()
                )))
            }
            None if started.elapsed() >= LAUNCHER_VERDICT => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            None => std::thread::sleep(LAUNCHER_POLL),
        }
    }
}

/// Every domain Zoom itself serves meetings from — the Rust twin of the
/// widget's `ZOOM_HOSTS` in `MeetingPresence.mjs` (#119). `zoom.us` alone
/// refused Zoom X, Telekom's German/EU Zoom, plus Zoom for Government and
/// Zoom China; a meeting on any of them is the same numbered Zoom meeting,
/// and `zoommtg://` takes the number, not the web host.
pub(crate) const ZOOM_HOSTS: &[&str] = &["zoom.us", "zoom.com", "zoom-x.de", "zoomgov.com", "zoom.com.cn"];

/// Whether `host` is Zoom or a subdomain of one of its domains.
pub(crate) fn is_zoom_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    ZOOM_HOSTS.iter().any(|z| h == *z || h.ends_with(&format!(".{z}")))
}

/// Opens one URI with the default handler, the AppImage's environment stripped
/// from the launcher when this process runs out of one.
///
/// The same launcher list and first-success-wins loop as `open::that`, which
/// this replaces at every call site; the additions are [`sanitize`] and
/// [`launch`]'s bounded wait.
fn open_one(url: &str) -> std::io::Result<()> {
    let appdir = std::env::var_os("APPDIR");
    let mut last_err = None;
    for mut cmd in open::commands(url) {
        if let Some(dir) = appdir.as_deref() {
            sanitize(&mut cmd, dir);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        match launch(&mut cmd) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no launcher available")))
}

/// Whether anything on this system actually claims `scheme`.
///
/// **The question has to be asked before the rewrite, not inferred after it.**
/// `xdg-open`'s own `open_generic` looks the scheme up, misses, and then falls
/// through to `$BROWSER` — so it hands `zoommtg://…` to a browser that cannot
/// open it and *exits 0*. Verified on this Hyprland box: an unregistered
/// scheme returns 0. A launcher's exit status therefore cannot tell "the app
/// took it" from "a browser shrugged at it", and a fallback conditioned on
/// that status never runs for exactly the people who need it — the ones
/// without the app installed.
#[cfg(target_os = "linux")]
fn scheme_has_handler(scheme: &str) -> bool {
    let mut cmd = Command::new("xdg-mime");
    cmd.args(["query", "default", &format!("x-scheme-handler/{scheme}")]);
    if let Some(dir) = std::env::var_os("APPDIR") {
        sanitize(&mut cmd, &dir);
    }
    // Empty stdout *with* a zero status is the "nothing claims it" answer;
    // the status alone says only that the query ran.
    match cmd.output() {
        Ok(out) => out.status.success() && !out.stdout.iter().all(u8::is_ascii_whitespace),
        Err(_) => false,
    }
}

/// macOS `open` refuses an unhandled scheme with a non-zero status, so there
/// the attempt is its own test and the try-then-fall-back shape is honest.
#[cfg(not(target_os = "linux"))]
fn scheme_has_handler(_scheme: &str) -> bool {
    true
}

/// Opens a URL with its preferred application. Zoom meeting links go to Zoom's
/// own protocol when something claims it, avoiding the disposable browser
/// handoff page; otherwise — and if the handler still fails — the original
/// HTTPS link is the fallback. All other URLs take their existing path.
pub(crate) fn open_external(url: &str) -> std::io::Result<()> {
    if let Some(direct) = zoom_join_uri(url) {
        if scheme_has_handler("zoommtg") && open_one(&direct).is_ok() {
            return Ok(());
        }
    }
    open_one(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #100, as the reporter timed it: with the browser not already
    /// running, `xdg-open` execs it in the foreground and does not come back
    /// — his measurement was "exit 124, still blocked after 20s" against
    /// "exit 0 in 0s" with the browser already up. A launcher standing in for
    /// that must be judged and left alone, not waited on, or the accept loop
    /// after it never starts.
    ///
    /// The assertion is on the clock rather than on a return value: the bug
    /// was never a wrong answer, it was no answer at all.
    #[cfg(unix)]
    #[test]
    fn a_launcher_that_keeps_running_is_taken_at_its_word() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());

        let started = Instant::now();
        let out = launch(&mut cmd);
        let waited = started.elapsed();

        assert!(out.is_ok(), "a launcher that has not exited has not failed: {out:?}");
        assert!(
            waited < Duration::from_secs(5),
            "waited {waited:?} on a launcher that outlives the call — the sign-in hang is back",
        );
    }

    /// The other half of the same trade, and the reason the wait is bounded
    /// rather than dropped: issue #1 was `xdg-open` exiting 4 on the spot with
    /// nothing drawn, and a user who is told "could not open a browser" can
    /// act on it.
    #[cfg(unix)]
    #[test]
    fn a_launcher_that_fails_on_the_spot_is_still_reported() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 4"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        assert!(launch(&mut cmd).is_err(), "an immediate non-zero exit is a failure");
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_that_succeeds_on_the_spot_is_a_success() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 0"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        assert!(launch(&mut cmd).is_ok());
    }

    #[test]
    fn zoom_meetings_use_the_registered_protocol() {
        assert_eq!(
            zoom_join_uri("https://us02web.zoom.us/j/123456789?pwd=x%2By%2Fz").as_deref(),
            Some("zoommtg://zoom.us/join?action=join&confno=123456789&pwd=x%2By%2Fz"),
        );
        assert_eq!(
            zoom_join_uri("https://zoom.us/wc/join/123-456-789").as_deref(),
            Some("zoommtg://zoom.us/join?action=join&confno=123456789"),
        );
        assert_eq!(
            zoom_join_uri("https://zoom.us/w/987654321?pwd=secret").as_deref(),
            Some("zoommtg://zoom.us/join?action=join&confno=987654321&pwd=secret"),
        );
    }

    /// Issue #119: the rewrite was keyed on `zoom.us` alone, so a meeting on
    /// Zoom X, Zoom for Government or Zoom China never reached the native
    /// client. The protocol takes the meeting number, not the web host, so
    /// the authority stays `zoom.us` for all of them.
    #[test]
    fn every_zoom_domain_is_rewritten_to_the_protocol() {
        for host in ["uni-kassel.zoom-x.de", "zoom-x.de", "zoomgov.com", "us02web.zoomgov.com", "zoom.com.cn", "zoom.com", "us05web.zoom.com"] {
            assert_eq!(
                zoom_join_uri(&format!("https://{host}/j/123456789?pwd=abc")).as_deref(),
                Some("zoommtg://zoom.us/join?action=join&confno=123456789&pwd=abc"),
                "{host}",
            );
        }
        // A suffix match, not a substring one.
        assert!(!is_zoom_host("zoom-x.de.evil.example"));
        assert!(!is_zoom_host("notzoom.us"));
        assert!(is_zoom_host("ZOOM.US"), "hosts are case-insensitive");
    }

    #[test]
    fn only_numbered_https_zoom_meetings_are_rewritten() {
        for url in [
            "https://meet.google.com/abc-defg-hij",
            "https://zoom.us/oauth/authorize",
            "https://zoom.us/my/team-room",
            "https://zoom.us.evil.example/j/123456789",
            "http://zoom.us/j/123456789",
            // Dash-tolerant, then filtered to nothing: this used to build a
            // join URI with an empty `confno` rather than fall back.
            "https://zoom.us/j/---",
        ] {
            assert_eq!(zoom_join_uri(url), None, "rewrote {url}");
        }
    }

    /// A scheme nothing will ever claim is reported unclaimed — the property
    /// `open_external` leans on, and the one an exit-status check got wrong:
    /// `xdg-open` returns 0 for this URI after handing it to a browser.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unclaimed_scheme_is_reported_unclaimed() {
        assert!(!scheme_has_handler("omacal-no-such-scheme-exists"));
    }

    /// The exact shape issue #1 reproduced: the AppDir's `usr/bin` prepended
    /// to an otherwise ordinary PATH. Only the AppDir components go.
    #[test]
    fn the_appdirs_path_entries_go_and_the_hosts_stay() {
        assert_eq!(
            path_without_appdir(
                "/tmp/.mount_omacal1/usr/bin:/usr/local/bin:/usr/bin",
                "/tmp/.mount_omacal1",
            ),
            "/usr/local/bin:/usr/bin",
        );
        // Prefix of the component, not substring anywhere in it.
        assert_eq!(
            path_without_appdir("/home/u/tmp/.mount_x-lookalike:/usr/bin", "/tmp/.mount_x"),
            "/home/u/tmp/.mount_x-lookalike:/usr/bin",
        );
        // Nothing to strip is a no-op, not a reshuffle.
        assert_eq!(path_without_appdir("/usr/local/bin:/usr/bin", "/tmp/.mount_x"),
                   "/usr/local/bin:/usr/bin");
    }

    /// Every poison variable is marked for removal on the child, and PATH is
    /// overridden rather than removed — a browser with no PATH at all is a
    /// different way to fail. `get_envs` shows removals as `None` values.
    #[test]
    fn a_sanitized_command_removes_the_appimage_env_and_keeps_a_path() {
        let mut cmd = Command::new("true");
        sanitize(&mut cmd, OsStr::new("/tmp/.mount_x"));

        let envs: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        for var in APPIMAGE_ONLY_VARS {
            assert_eq!(envs.get(OsStr::new(var)), Some(&None),
                       "{var} was not removed from the child");
        }
        assert!(matches!(envs.get(OsStr::new("PATH")), Some(&Some(_))),
                "PATH was removed instead of filtered");
    }
}
