//! The Omarchy font (#179): the monospace family `omarchy font set` chose,
//! for the UI to put at the head of its font stack while "Use the Omarchy
//! font" is on. The palette has followed the Omarchy theme from the start;
//! this is the other half of looking like the desktop.
//!
//! **Read from fontconfig, as Omarchy itself reads it.** Omarchy 4's
//! `omarchy-font-current` is `fc-match monospace -f '%{family}\n' | head -n1
//! | cut -d, -f1`, and `omarchy-font-set` says why: fontconfig is "the
//! canonical source of truth", and what it does is write
//! `~/.config/fontconfig/fonts.conf`. Asking `fc-match` directly rather than
//! running Omarchy's script gives the same answer without depending on
//! Omarchy's `bin` being on a desktop app's `PATH`, and it is right on the
//! Omarchy before 4 too, whose `font-set` wrote the same file.

use sqlx::SqlitePool;
use std::path::PathBuf;
use std::time::Duration;

/// How long fontconfig gets to answer. It is normally a few milliseconds,
/// but the first `fc-match` after a font is installed can rebuild the cache;
/// past this the stack stays as it is and the next theme switch asks again.
const ASK_TIMEOUT: Duration = Duration::from_secs(2);

/// Longer than any family a font ships with. Output beyond it is not a name,
/// and a name is all this hands on.
const MAX_FAMILY_CHARS: usize = 128;

/// The family `fc-match` printed, or `None` for output that is not one.
///
/// `%{family}` is the font's names comma-separated, its own first
/// (`JetBrainsMono Nerd Font,JetBrainsMono NF`), so the first line's first
/// entry is the family, which is what `omarchy font current` prints.
///
/// It goes on to become CSS, so a name with a quote, a backslash or a control
/// character in it is refused rather than passed on: those are what could
/// end the string it is quoted in, and no font is named with them. The UI
/// refuses the same set at the point it writes the CSS (`font.ts`), which
/// does not trust this side to have done it.
pub(crate) fn parse_family(stdout: &str) -> Option<String> {
    let family = stdout.lines().next()?.split(',').next()?.trim();
    let plain = !family.is_empty()
        && family.chars().count() <= MAX_FAMILY_CHARS
        && !family.chars().any(|c| c.is_control() || c == '"' || c == '\\');
    plain.then(|| family.to_string())
}

/// Runs `cmd` and reads a family off what it prints. `None` when it cannot
/// start, does not succeed, prints no family, or has not finished inside
/// `timeout` — when it is killed rather than left behind.
async fn ask(mut cmd: tokio::process::Command, timeout: Duration) -> Option<String> {
    cmd.stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_family(&String::from_utf8(out.stdout).ok()?)
}

/// `omarchy-font-current`'s own question, argument for argument. From an
/// AppImage it runs without the image's library environment, through
/// `browser.rs`'s `sanitize` and for its reason: that environment is for
/// this binary, and a host `fc-match` loaded against the bundled libraries
/// can fail outright.
fn fc_match() -> tokio::process::Command {
    let mut cmd = std::process::Command::new("fc-match");
    cmd.args(["monospace", "-f", r"%{family}\n"]);
    if let Some(dir) = std::env::var_os("APPDIR") {
        crate::browser::sanitize(&mut cmd, &dir);
    }
    cmd.into()
}

/// Whether to ask fontconfig at all: the switch is on, this is Linux, and
/// there is an Omarchy theme. Anywhere else `fc-match monospace` may well
/// still answer, DejaVu Sans Mono say, but with a font nobody chose.
fn wanted(on: bool, linux: bool, omarchy: bool) -> bool {
    on && linux && omarchy
}

/// The family for the UI to put first, or `None` to leave its stack as it
/// is. The setting is read before anything else, so with it off — every
/// install until somebody turns it on — nothing is spawned.
pub(crate) async fn current(pool: &SqlitePool) -> Option<String> {
    let on = crate::settings::use_omarchy_font(pool).await;
    if !wanted(on, cfg!(target_os = "linux"), crate::theme::omarchy_theme_dir().is_some()) {
        return None;
    }
    ask(fc_match(), ASK_TIMEOUT).await
}

/// Where `omarchy font set` writes, for the theme watcher to watch beside
/// the theme: the font changes there and nowhere under the theme directory.
/// `$HOME/.config` rather than an XDG lookup, because that is the path
/// Omarchy's own script writes. `None` when it does not exist, which it may
/// not until the first `omarchy font set`.
pub(crate) fn fontconfig_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOME")?).join(".config/fontconfig");
    dir.is_dir().then_some(dir)
}

/// The font for the UI's stack, or `null`: off, not Omarchy, or nothing
/// usable from fontconfig. Asked at startup when the setting is on, when it
/// is switched, and whenever the theme watcher says the desktop changed.
#[tauri::command]
pub async fn get_omarchy_font(state: tauri::State<'_, crate::AppState>) -> Result<Option<String>, String> {
    Ok(current(&state.pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    /// Omarchy's default font, as fontconfig prints it: the family first,
    /// then its other names. Only the first line counts, and only its first
    /// entry, trimmed — what `omarchy font current` would print.
    #[test]
    fn the_family_is_the_first_name_on_the_first_line() {
        assert_eq!(
            parse_family("JetBrainsMono Nerd Font,JetBrainsMono NF\n").as_deref(),
            Some("JetBrainsMono Nerd Font"),
        );
        assert_eq!(parse_family("CaskaydiaMono Nerd Font\nIosevka\n").as_deref(), Some("CaskaydiaMono Nerd Font"));
        assert_eq!(parse_family("  Iosevka Term \n").as_deref(), Some("Iosevka Term"));
    }

    #[test]
    fn output_with_no_family_in_it_is_none() {
        for out in ["", "\n", "   \n", ",JetBrainsMono NF\n"] {
            assert_eq!(parse_family(out), None, "{out:?}");
        }
    }

    /// The name is going to be quoted into CSS: anything that could end the
    /// quotes or the declaration is not a name.
    #[test]
    fn a_name_that_could_break_out_of_its_quotes_is_refused() {
        for out in ["Evil\" , fantasy\n", "Evil\\Mono\n", "Evil\u{1b}[31mMono\n", "Evil\tMono\n"] {
            assert_eq!(parse_family(out), None, "{out:?}");
        }
    }

    #[test]
    fn a_name_longer_than_any_font_has_is_refused() {
        let longest = "M".repeat(MAX_FAMILY_CHARS);
        assert_eq!(parse_family(&longest).as_deref(), Some(longest.as_str()));
        assert_eq!(parse_family(&"M".repeat(MAX_FAMILY_CHARS + 1)), None);
    }

    /// Room for a loaded machine, #206's lesson: these tests are about what a
    /// command does, never about how quickly `sh` starts.
    const PATIENT: Duration = Duration::from_secs(30);

    /// The spawn: what the command prints reaches the parser, and a command
    /// that fails, or cannot start, is no font.
    #[tokio::test]
    async fn a_command_that_fails_or_cannot_start_is_no_font() {
        let printed = ask(sh("printf 'JetBrainsMono Nerd Font,JetBrainsMono NF\\n'"), PATIENT).await;
        assert_eq!(printed.as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(ask(sh("echo 'Fake Mono'; exit 1"), PATIENT).await, None);
        let missing = tokio::process::Command::new("/nonexistent/fc-match");
        assert_eq!(ask(missing, PATIENT).await, None);
    }

    /// A fontconfig that hangs costs the timeout and then nothing: the stack
    /// stays as it is rather than the answer arriving whenever it arrives. A
    /// minute's hang against a bound of half that, so a loaded machine has
    /// room and the test is still over in a fifth of a second. `exec`, so
    /// what the timeout kills is the `sleep` itself and nothing is left
    /// behind.
    #[tokio::test]
    async fn a_command_that_hangs_is_given_up_on() {
        let started = std::time::Instant::now();
        let answer = ask(sh("exec sleep 60"), Duration::from_millis(200)).await;
        assert_eq!(answer, None);
        assert!(started.elapsed() < PATIENT, "waited {:?}", started.elapsed());
    }

    /// Off is the default, and then nothing is asked; so is macOS, and any
    /// Linux without an Omarchy theme.
    #[test]
    fn fontconfig_is_asked_only_when_on_and_on_omarchy() {
        assert!(wanted(true, true, true));
        assert!(!wanted(false, true, true), "off");
        assert!(!wanted(true, false, true), "not Linux");
        assert!(!wanted(true, true, false), "no Omarchy theme");
    }
}
