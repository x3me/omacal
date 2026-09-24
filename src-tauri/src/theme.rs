#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub bg: String,
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub is_dark: bool,
}

impl Palette {
    /// Used whenever a theme cannot be read or parsed. The app must always
    /// start (spec §10).
    pub fn fallback_dark() -> Self {
        Self {
            bg: "#17171a".into(),
            surface: "#1e1e22".into(),
            text: "#e8e8ea".into(),
            muted: "#8a8a90".into(),
            accent: "#5b8def".into(),
            is_dark: true,
        }
    }

    /// [`fallback_dark`](Self::fallback_dark)'s counterpart, and the whole of
    /// issue #30: omacal has no theme of its own, it wears Omarchy's — so on
    /// any desktop that is not Omarchy, [`omarchy_theme_dir`] answered `None`,
    /// `resolve` answered the dark fallback, and there was no second answer
    /// anywhere in the app. Every non-Omarchy user was on dark for good.
    ///
    /// Built rather than derived: inverting the dark palette's channels gives
    /// grey text on grey, because the two ends of a screen are not symmetric.
    ///
    /// `surface` is a shade *darker* than `bg`, which is the opposite of the
    /// dark palette and deliberate: it is the same direction [`parse_colors`]
    /// shifts a light Omarchy theme's surface, so a card here sits the way a
    /// card does under Rose Pine Dawn. The CSS has been rendering light themes
    /// that way since v0.1.8; this is not the place to introduce a second
    /// convention.
    ///
    /// Every value is contrast-led, measured against both `bg` and `surface`
    /// because text lands on each: `text` at 16.6:1 and 15.2:1, `muted` at
    /// 5.8:1 and 5.3:1, and `accent` at 5.1:1 and 4.6:1 — all clear of the
    /// 4.5:1 that small text needs, which the first accent tried here (a
    /// brighter `#3b6fe0`) did not at 4.1:1 on a card.
    pub fn fallback_light() -> Self {
        Self {
            bg: "#fbfbfd".into(),
            surface: "#f1f1f4".into(),
            text: "#1b1b1f".into(),
            muted: "#63636b".into(),
            accent: "#3566d6".into(),
            is_dark: false,
        }
    }

    /// The built-in palette a system's light-or-dark setting asks for.
    /// No preference is dark: what every copy showed before it asked.
    pub fn for_system(scheme: SystemScheme) -> Self {
        match scheme {
            SystemScheme::Light => Palette::fallback_light(),
            SystemScheme::Dark | SystemScheme::NoPreference => Palette::fallback_dark(),
        }
    }

    /// A named theme's palette, for a desktop that has no Omarchy theme to
    /// follow — macOS above all, where the choice was Light or our own grey
    /// Dark. Two dark and two light, the most used of Omarchy's own themes.
    ///
    /// The backgrounds are Omarchy's own theme files', so a Tokyo Night chosen
    /// on a Mac looks like Tokyo Night on Omarchy, and so are the dark themes'
    /// surfaces ([`parse_colors`]'s shift). The rest is **contrast-led**, as
    /// the built-in two are, and every text colour is from the theme's
    /// official palette:
    ///
    /// - Tokyo Night takes its `fg`/`fg_dark` pair for text and muted: the
    ///   theme file resolves muted *brighter* than text, which inverts them.
    /// - Catppuccin takes **mauve**, its own default accent, in both flavours:
    ///   Latte's blue measures 4.3:1 on its background and 4.0:1 on a card.
    /// - Rosé Pine Dawn takes **pine**: foam, which the theme file names,
    ///   measures 3.1:1.
    /// - The light themes' cards sit a shade nearer their background than the
    ///   shift would put them (1.05:1, not 1.07:1): at the shifted shade,
    ///   Latte's mauve and Dawn's muted grey fell just under 4.5:1 on a card,
    ///   and neither palette has a darker colour to trade them for.
    ///
    /// Every text colour clears 4.5:1 on `bg` and on `surface`
    /// (`every_named_palette_is_legible` measures all of it).
    pub fn named(theme: Appearance) -> Option<Self> {
        let p = |bg: &str, surface: &str, text: &str, muted: &str, accent: &str, is_dark| Self {
            bg: bg.into(), surface: surface.into(), text: text.into(),
            muted: muted.into(), accent: accent.into(), is_dark,
        };
        Some(match theme {
            Appearance::TokyoNight => p("#1a1b26", "#21222d", "#c0caf5", "#a9b1d6", "#7aa2f7", true),
            Appearance::CatppuccinMocha => p("#1e1e2e", "#252535", "#cdd6f4", "#a6adc8", "#cba6f7", true),
            Appearance::CatppuccinLatte => p("#eff1f5", "#e9ebef", "#4c4f69", "#5c5f77", "#8839ef", false),
            Appearance::RosePineDawn => p("#faf4ed", "#f5efe8", "#575279", "#6e6a86", "#286983", false),
            Appearance::Auto | Appearance::Light | Appearance::Dark => return None,
        })
    }
}

/// The system's light-or-dark setting, for a desktop with no Omarchy theme:
/// macOS's appearance, or the freedesktop portal's `color-scheme` (GNOME,
/// KDE and the rest). Numbered as the portal numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemScheme {
    NoPreference = 0,
    Dark = 1,
    Light = 2,
}

impl SystemScheme {
    /// The portal's `org.freedesktop.appearance` `color-scheme` value.
    /// Anything it may add later reads as no preference.
    pub fn from_portal(value: u32) -> Self {
        match value {
            1 => SystemScheme::Dark,
            2 => SystemScheme::Light,
            _ => SystemScheme::NoPreference,
        }
    }
}

/// The last setting the system reported, kept by `system_theme`'s watchers
/// and read by [`resolve`]. Process-wide because the answer is: every
/// window, the GTK hint and the tray all wear one palette.
static SYSTEM_SCHEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_system_scheme(scheme: SystemScheme) {
    SYSTEM_SCHEME.store(scheme as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_scheme() -> SystemScheme {
    SystemScheme::from_portal(u32::from(SYSTEM_SCHEME.load(std::sync::atomic::Ordering::Relaxed)))
}

/// Which palette the app wears.
///
/// `Auto` stays the default: the Omarchy theme if there is one, and
/// otherwise the system's own light-or-dark ([`SystemScheme`]) — which until
/// 2026-09-24 it never asked, so every Mac on `Auto` was dark for good.
///
/// `Light` and `Dark` are the user overruling that, and they take the built-in
/// palette **whole** — including on Omarchy, where the theme's own accent is
/// deliberately *not* kept. An accent picked to sit on a dark terminal has no
/// obligation to be legible on white (Omarchy ships pale yellows and washed
/// greens), and the entire point of choosing a palette explicitly is getting
/// one that works. Someone who wants their theme's colours wants `Auto`.
///
/// The four named themes are the same kind of choice — see
/// [`Palette::named`] — spelled as Omarchy spells its themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Auto,
    Light,
    Dark,
    #[serde(rename = "tokyo-night")]
    TokyoNight,
    #[serde(rename = "catppuccin-mocha")]
    CatppuccinMocha,
    #[serde(rename = "catppuccin-latte")]
    CatppuccinLatte,
    #[serde(rename = "rose-pine-dawn")]
    RosePineDawn,
}

impl Appearance {
    /// The stored spelling — the same string the wire uses, so a row read by
    /// eye in `sqlite3` says what the settings modal says.
    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::Auto => "auto",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::TokyoNight => "tokyo-night",
            Appearance::CatppuccinMocha => "catppuccin-mocha",
            Appearance::CatppuccinLatte => "catppuccin-latte",
            Appearance::RosePineDawn => "rose-pine-dawn",
        }
    }

    /// The stored spelling read back: `as_str`'s inverse, and `None` for
    /// anything this version did not write.
    pub fn parse(stored: &str) -> Option<Self> {
        [
            Appearance::Auto, Appearance::Light, Appearance::Dark, Appearance::TokyoNight,
            Appearance::CatppuccinMocha, Appearance::CatppuccinLatte, Appearance::RosePineDawn,
        ]
        .into_iter()
        .find(|a| a.as_str() == stored)
    }

    /// Whether this choice is the user's own rather than the desktop's — the
    /// question `theme_watch` asks before repainting on a theme file it has
    /// been told not to follow.
    pub fn is_pinned(self) -> bool {
        !matches!(self, Appearance::Auto)
    }
}

#[derive(Deserialize)]
struct AlacrittyFile {
    colors: Option<AlacrittyColors>,
}

#[derive(Deserialize)]
struct AlacrittyColors {
    primary: Option<AlacrittyPrimary>,
    normal: Option<AlacrittyNormal>,
}

#[derive(Deserialize)]
struct AlacrittyPrimary {
    background: Option<String>,
    foreground: Option<String>,
}

#[derive(Deserialize)]
struct AlacrittyNormal {
    blue: Option<String>,
    white: Option<String>,
}

/// The palette file Omarchy themes declare their colours in: flat keys,
/// `accent = "#7aa2f7"` first among them. Alacritty's `normal.blue` was only
/// ever a guess at this value — right for Tokyo Night, wrong for any theme
/// whose accent is not its terminal blue — so when the theme author wrote
/// the accent down, believe them.
///
/// Omarchy 4 grew this file into a full palette (`mode`, `background`,
/// `foreground`, `muted`, …), making it the authoritative source; pre-4
/// themes typically carry only `accent`, and every missing key just means
/// "fall back to the alacritty guess".
#[derive(Deserialize)]
struct ColorsFile {
    mode: Option<String>,
    accent: Option<String>,
    background: Option<String>,
    foreground: Option<String>,
    muted: Option<String>,
    light_foreground: Option<String>,
    blue: Option<String>,
}

/// The explicit accent from a theme's `colors.toml`, normalised, or `None`
/// when the file is not TOML, has no accent, or the accent is not a colour —
/// each of which means "keep guessing", never "break the theme".
pub fn parse_colors_accent(toml_src: &str) -> Option<String> {
    let file: ColorsFile = toml::from_str(toml_src).ok()?;
    parse_hex(&file.accent?).map(format_hex)
}

/// A full palette from an Omarchy 4 `colors.toml`. Requires `background` and
/// `foreground` — a file without both (every pre-4 theme) returns `None` and
/// the alacritty chain takes over. `mode` is believed over luminance when the
/// author stated it; a garbled `mode` falls back to measuring.
pub fn parse_colors(toml_src: &str) -> Option<Palette> {
    let file: ColorsFile = toml::from_str(toml_src).ok()?;
    let bg_bytes = parse_hex(&file.background?)?;
    let text_bytes = parse_hex(&file.foreground?)?;

    let is_dark = match file.mode.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => luminance(bg_bytes) < 0.5,
    };

    let accent = file
        .accent
        .as_deref()
        .and_then(parse_hex)
        .or_else(|| file.blue.as_deref().and_then(parse_hex))
        .map(format_hex)
        .unwrap_or_else(|| format_hex(text_bytes));

    // Omarchy's `muted` is a quiet palette swatch, not necessarily readable
    // text: Nord puts it at 1.69:1 against its background, and most stock
    // themes make the same distinction. `light_foreground` is the secondary
    // text tier and is the right first choice for the labels omacal publishes
    // as `--muted`. Older themes may not have it, so retain the historical
    // `muted`/derived chain and make whichever candidate wins readable.
    let muted_bytes = file
        .light_foreground
        .as_deref()
        .and_then(parse_hex)
        .or_else(|| file.muted.as_deref().and_then(parse_hex))
        .unwrap_or_else(|| shifted(text_bytes, if is_dark { -0.25 } else { 0.25 }));
    let muted = format_hex(ensure_text_contrast(muted_bytes, bg_bytes, text_bytes));

    Some(Palette {
        bg: format_hex(bg_bytes),
        surface: shift(bg_bytes, if is_dark { 0.03 } else { -0.03 }),
        text: format_hex(text_bytes),
        muted,
        accent,
        is_dark,
    })
}

/// Parses `#rrggbb`, `0xrrggbb`, `0Xrrggbb`, or bare `rrggbb` into RGB bytes.
/// Returns None for anything else — no slicing panics, no partial parses.
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let trimmed = if let Some(stripped) = s.strip_prefix('#') {
        stripped
    } else if let Some(stripped) = s.strip_prefix("0x") {
        stripped
    } else if let Some(stripped) = s.strip_prefix("0X") {
        stripped
    } else {
        s
    };

    // Require exactly 6 ASCII hex digits; validate before slicing.
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    // Safe to slice: we've verified 6 ASCII chars.
    let r = u8::from_str_radix(&trimmed[0..2], 16).ok()?;
    let g = u8::from_str_radix(&trimmed[2..4], 16).ok()?;
    let b = u8::from_str_radix(&trimmed[4..6], 16).ok()?;

    Some([r, g, b])
}

/// Normalize RGB bytes to lowercase `#rrggbb`.
fn format_hex(bytes: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2])
}

/// Relative luminance of RGB bytes, used only to classify dark vs light.
fn luminance(bytes: [u8; 3]) -> f32 {
    let r = bytes[0] as f32;
    let g = bytes[1] as f32;
    let b = bytes[2] as f32;
    (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
}

/// WCAG relative luminance, for contrast between text and its surface.
///
/// This is deliberately separate from [`luminance`]: that function preserves
/// the app's historical light/dark classification, while contrast ratios need
/// the standard's linear-light conversion rather than a weighted sRGB byte.
fn relative_luminance(bytes: [u8; 3]) -> f32 {
    let channel = |byte: u8| {
        let value = byte as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(bytes[0]) + 0.7152 * channel(bytes[1]) + 0.0722 * channel(bytes[2])
}

fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Moves a secondary-text candidate toward the theme's primary foreground
/// only as far as needed to reach WCAG AA contrast for ordinary text.
///
/// Theme authors still choose the hue and starting weight. The guard matters
/// for older and custom themes that do not carry Omarchy 4's readable
/// `light_foreground`; if even the primary foreground is low-contrast, it is
/// still the best theme-consistent answer available.
fn ensure_text_contrast(candidate: [u8; 3], background: [u8; 3], text: [u8; 3]) -> [u8; 3] {
    const MIN_RATIO: f32 = 4.5;
    if contrast_ratio(candidate, background) >= MIN_RATIO {
        return candidate;
    }

    for percent in 1..=100u16 {
        let mixed = std::array::from_fn(|i| {
            let candidate = candidate[i] as u16;
            let text = text[i] as u16;
            ((candidate * (100 - percent) + text * percent + 50) / 100) as u8
        });
        if contrast_ratio(mixed, background) >= MIN_RATIO {
            return mixed;
        }
    }

    text
}

fn shifted(bytes: [u8; 3], amount: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        let value = bytes[i] as f32;
        (value + 255.0 * amount).clamp(0.0, 255.0) as u8
    })
}

/// Lightens or darkens RGB by `amount` (-1.0..=1.0), used to derive a surface
/// colour one step away from the background.
fn shift(bytes: [u8; 3], amount: f32) -> String {
    format_hex(shifted(bytes, amount))
}

/// Parses an Alacritty theme into a palette. Returns `None` when the file is
/// not valid TOML, carries no background colour, or contains invalid hex.
pub fn parse_alacritty(toml_src: &str) -> Option<Palette> {
    let file: AlacrittyFile = toml::from_str(toml_src).ok()?;
    let colors = file.colors?;
    let primary = colors.primary?;
    let bg_str = primary.background?;
    let bg_bytes = parse_hex(&bg_str)?;
    let bg = format_hex(bg_bytes);

    let text_str = primary.foreground.unwrap_or_else(|| "#e8e8ea".into());
    let text_bytes = parse_hex(&text_str).unwrap_or([0xe8, 0xe8, 0xea]);
    let text = format_hex(text_bytes);

    let is_dark = luminance(bg_bytes) < 0.5;

    let accent = colors
        .normal
        .as_ref()
        .and_then(|n| n.blue.as_ref())
        .and_then(|blue_str| parse_hex(blue_str))
        .map(format_hex)
        .unwrap_or_else(|| text.clone());

    let muted_bytes = colors
        .normal
        .as_ref()
        .and_then(|n| n.white.as_ref())
        .and_then(|white_str| parse_hex(white_str))
        .unwrap_or_else(|| shifted(text_bytes, if is_dark { -0.25 } else { 0.25 }));
    let muted = format_hex(ensure_text_contrast(muted_bytes, bg_bytes, text_bytes));

    Some(Palette {
        surface: shift(bg_bytes, if is_dark { 0.03 } else { -0.03 }),
        bg,
        text,
        muted,
        accent,
        is_dark,
    })
}

/// Resolves the active palette, following the spec §10 fallback chain:
/// `colors.toml`'s full palette (Omarchy 4), then `alacritty.toml` in the
/// theme directory, then the built-in dark palette — and on the alacritty and
/// fallback bases, `colors.toml`'s explicit `accent` still outranks whatever
/// the base produced. Never fails.
///
/// A pinned [`Appearance`] short-circuits the whole chain, theme directory and
/// all: it is the user saying which of the two built-in palettes they want,
/// and a theme file cannot outrank that. `Auto` is the chain exactly as it was.
pub fn resolve(theme_dir: Option<&Path>, appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Light => return Palette::fallback_light(),
        Appearance::Dark => return Palette::fallback_dark(),
        Appearance::Auto => {}
        named => return Palette::named(named).expect("every other variant is a named theme"),
    }
    let Some(dir) = theme_dir else {
        // No Omarchy theme: follow the system's light-or-dark instead, which
        // is what "Follow the desktop theme" says it does.
        return Palette::for_system(system_scheme());
    };
    let colors_src = std::fs::read_to_string(dir.join("colors.toml")).ok();

    if let Some(palette) = colors_src.as_deref().and_then(parse_colors) {
        return palette;
    }

    let mut palette = match std::fs::read_to_string(dir.join("alacritty.toml")) {
        Ok(src) => parse_alacritty(&src).unwrap_or_else(|| {
            tracing::warn!(?dir, "theme found but could not be parsed; using fallback");
            Palette::fallback_dark()
        }),
        Err(e) => {
            tracing::warn!(?dir, %e, "no readable theme; using fallback");
            Palette::fallback_dark()
        }
    };

    // Applied to the fallback too, deliberately: a theme whose alacritty.toml
    // is broken but whose colors.toml names an accent gets the fallback's
    // legible base wearing the theme's own accent — closer to intent than
    // either file alone.
    if let Some(accent) = colors_src.as_deref().and_then(parse_colors_accent) {
        palette.accent = accent;
    }
    palette
}

/// The conventional Omarchy location. Returns `None` off Linux or when absent.
///
/// Omarchy 4 keeps the live theme under the state dir (a hardcoded
/// `~/.local/state` in Omarchy's own scripts, so no XDG lookup here); the
/// `~/.config` path is where every earlier Omarchy put it.
pub fn omarchy_theme_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let home = Path::new(&home);
    [
        ".local/state/omarchy/current/theme",
        ".config/omarchy/current/theme",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/alacritty.toml");

    #[test]
    fn a_tokyo_night_alacritty_theme_parses() {
        let p = parse_alacritty(FIXTURE).unwrap();
        assert_eq!(p.bg, "#1a1b26");
        assert_eq!(p.text, "#c0caf5");
        assert_eq!(p.accent, "#7aa2f7");
        assert!(p.is_dark);
    }

    #[test]
    fn a_light_background_is_detected_as_light() {
        let src = r##"
[colors.primary]
background = "#eff1f5"
foreground = "#4c4f69"
[colors.normal]
blue = "#1e66f5"
"##;
        let p = parse_alacritty(src).unwrap();
        assert!(!p.is_dark);
    }

    #[test]
    fn a_theme_without_a_background_is_rejected() {
        assert!(parse_alacritty("[colors.normal]\nblue = \"#1e66f5\"").is_none());
    }

    #[test]
    fn malformed_toml_is_rejected_without_panicking() {
        assert!(parse_alacritty("this is not toml {{{").is_none());
    }

    #[test]
    fn a_missing_accent_falls_back_to_the_foreground() {
        let src = "[colors.primary]\nbackground = \"#1a1b26\"\nforeground = \"#c0caf5\"";
        let p = parse_alacritty(src).unwrap();
        assert_eq!(p.accent, "#c0caf5");
    }

    #[test]
    fn resolve_falls_back_when_the_directory_is_missing() {
        let p = resolve(Some(std::path::Path::new("/nonexistent/omarchy/theme")), Appearance::Auto);
        assert_eq!(p, Palette::fallback_dark());
    }

    #[test]
    fn resolve_falls_back_when_given_nothing() {
        assert_eq!(resolve(None, Appearance::Auto), Palette::fallback_dark());
    }

    #[test]
    fn resolve_parses_a_real_theme_directory() {
        let temp_dir = std::env::temp_dir().join(format!(
            "omacal_test_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let theme_file = temp_dir.join("alacritty.toml");
        let toml_content = r##"
[colors.primary]
background = "#1a1b26"
foreground = "#c0caf5"
[colors.normal]
blue = "#7aa2f7"
white = "#a9b1d6"
"##;
        std::fs::write(&theme_file, toml_content).expect("write temp theme file");
        let p = resolve(Some(&temp_dir), Appearance::Auto);
        assert_eq!(p.bg, "#1a1b26");
        assert_eq!(p.text, "#c0caf5");
        assert_eq!(p.accent, "#7aa2f7");
        assert_eq!(p.muted, "#a9b1d6");
        assert!(p.is_dark);
        assert_ne!(p.surface, p.bg, "surface should differ from bg");
        let _ = std::fs::remove_file(&theme_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    /// The whole feature: the author's accent beats the blue we guessed.
    /// Distinct values on purpose — an orange accent over a blue guess is
    /// exactly the theme (e.g. anything warm) the guess got wrong.
    #[test]
    fn the_explicit_accent_outranks_alacrittys_blue() {
        let temp_dir = std::env::temp_dir().join(format!("omacal_accent_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        std::fs::write(
            temp_dir.join("alacritty.toml"),
            "[colors.primary]\nbackground = \"#1a1b26\"\nforeground = \"#c0caf5\"\n[colors.normal]\nblue = \"#7aa2f7\"",
        )
        .unwrap();
        std::fs::write(temp_dir.join("colors.toml"), "accent = \"#e0af68\"").unwrap();

        let p = resolve(Some(&temp_dir), Appearance::Auto);
        assert_eq!(p.accent, "#e0af68", "the theme said orange; blue was a guess");
        assert_eq!(p.bg, "#1a1b26", "everything else still comes from alacritty");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// The other half, without which the override could be `accent = file
    /// contents`: garbage in colors.toml keeps the guess rather than
    /// breaking the palette.
    #[test]
    fn a_bad_or_absent_colors_accent_keeps_the_guess() {
        assert_eq!(parse_colors_accent("accent = \"#e0af68\""), Some("#e0af68".into()));
        assert_eq!(parse_colors_accent("accent = \"0xE0AF68\""), Some("#e0af68".into()));
        assert_eq!(parse_colors_accent("accent = \"not-a-colour\""), None);
        assert_eq!(parse_colors_accent("cursor = \"#ffffff\""), None, "no accent key");
        assert_eq!(parse_colors_accent("this is not toml {{{"), None);

        // And through `resolve`: a colors.toml with a broken accent leaves
        // alacritty's answer standing.
        let temp_dir = std::env::temp_dir().join(format!("omacal_badacc_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        std::fs::write(
            temp_dir.join("alacritty.toml"),
            "[colors.primary]\nbackground = \"#1a1b26\"\nforeground = \"#c0caf5\"\n[colors.normal]\nblue = \"#7aa2f7\"",
        )
        .unwrap();
        std::fs::write(temp_dir.join("colors.toml"), "accent = \"nope\"").unwrap();
        assert_eq!(resolve(Some(&temp_dir), Appearance::Auto).accent, "#7aa2f7");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn zero_x_prefixed_colours_parse() {
        let src = r##"
[colors.primary]
background = "0x1a1b26"
foreground = "0xc0caf5"
[colors.normal]
blue = "0x7aa2f7"
"##;
        let p = parse_alacritty(src).unwrap();
        assert_eq!(p.bg, "#1a1b26", "0x prefix should normalize to #");
        assert_eq!(p.text, "#c0caf5", "0x prefix should normalize to #");
        assert_eq!(p.accent, "#7aa2f7", "0x prefix should normalize to #");
        assert!(p.is_dark);
        assert_ne!(p.surface, p.bg, "surface should differ from bg even with 0x format");
    }

    #[test]
    fn malformed_colours_do_not_panic() {
        // Table-driven: each variant should fail gracefully without panic.
        let test_cases = vec![
            (r##"[colors.primary]
background = ""
foreground = "#c0caf5""##, "empty string"),
            (r##"[colors.primary]
background = "#GGGGGG"
foreground = "#c0caf5""##, "invalid hex chars"),
            (r##"[colors.primary]
background = "#abc"
foreground = "#c0caf5""##, "3 digits instead of 6"),
            (r##"[colors.primary]
background = "#1a1b26ff"
foreground = "#c0caf5""##, "8 digits instead of 6"),
        ];

        for (src, label) in test_cases {
            let result = parse_alacritty(src);
            assert!(
                result.is_none(),
                "malformed colour {} should be rejected, not parsed",
                label
            );
        }
    }

    /// Omarchy 4's colors.toml carries the whole palette; nothing should be
    /// guessed from alacritty when the author wrote it all down.
    #[test]
    fn an_omarchy_4_colors_file_yields_a_full_palette() {
        let src = r##"
mode = "dark"
accent = "#7aa2f7"
muted = "#414868"
light_foreground = "#b4bee6"
background = "#1a1b26"
foreground = "#a9b1d6"
blue = "#7aa2f7"
"##;
        let p = parse_colors(src).unwrap();
        assert_eq!(p.bg, "#1a1b26");
        assert_eq!(p.text, "#a9b1d6");
        assert_eq!(p.accent, "#7aa2f7");
        assert_eq!(p.muted, "#b4bee6");
        assert!(p.is_dark);
        assert_ne!(p.surface, p.bg, "surface should differ from bg");
    }

    /// Omarchy's quiet `muted` swatch is often deliberately close to the
    /// background. It remains a valid palette colour, but it is not readable
    /// secondary text; omacal's `--muted` is text, so the foreground tier wins.
    #[test]
    fn light_foreground_outranks_the_quiet_muted_swatch_for_secondary_text() {
        let src = r##"
mode = "dark"
background = "#2e3440"
foreground = "#d8dee9"
muted = "#4c566a"
light_foreground = "#adb5c4"
"##;
        let p = parse_colors(src).unwrap();
        assert_eq!(p.muted, "#adb5c4");
        assert!(contrast_ratio(parse_hex(&p.muted).unwrap(), parse_hex(&p.bg).unwrap()) >= 4.5);
    }

    /// The built-in light palette is the one palette in the app nobody else
    /// checks: an Omarchy theme's colours are the author's business and
    /// `ensure_text_contrast` lifts what it must, but these five values are
    /// ours, and if they drift the only symptom is grey-on-grey that someone
    /// has to notice by eye. Both surfaces, because text lands on each.
    #[test]
    fn the_built_in_light_palette_is_readable_on_both_of_its_surfaces() {
        let p = Palette::fallback_light();
        assert!(!p.is_dark);
        let hex = |s: &str| parse_hex(s).unwrap();
        for (name, on) in [("bg", &p.bg), ("surface", &p.surface)] {
            for (label, fg) in [("text", &p.text), ("muted", &p.muted), ("accent", &p.accent)] {
                let ratio = contrast_ratio(hex(fg), hex(on));
                assert!(ratio >= 4.5, "{label} on {name} is {ratio:.2}:1, below AA");
            }
        }
        // Cards sit the way `parse_colors` makes them sit for a light Omarchy
        // theme — a shade *darker* than the page, not lighter. One convention.
        assert!(
            relative_luminance(hex(&p.surface)) < relative_luminance(hex(&p.bg)),
            "a light theme's surface is a shade below its background",
        );
    }

    /// The setting overrules the theme directory entirely — that is what
    /// choosing a palette means. Pinned to both directions because the bug it
    /// prevents is silent: a `Light` that still read `colors.toml` would look
    /// right on a light Omarchy theme and change nothing for the users this
    /// exists for.
    #[test]
    fn a_pinned_appearance_outranks_whatever_theme_is_installed() {
        let dir = std::env::temp_dir().join(format!("omacal-appearance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("colors.toml"),
            "mode = \"dark\"\nbackground = \"#2e3440\"\nforeground = \"#d8dee9\"\n",
        )
        .unwrap();

        assert_eq!(resolve(Some(&dir), Appearance::Light), Palette::fallback_light());
        assert_eq!(resolve(Some(&dir), Appearance::Dark), Palette::fallback_dark());
        // Auto still follows the theme, which is every existing install.
        let auto = resolve(Some(&dir), Appearance::Auto);
        assert_eq!(auto.bg, "#2e3440", "Auto is the behaviour that must not move");

        // And with no Omarchy at all — the case issue #30 is about.
        assert_eq!(resolve(None, Appearance::Light), Palette::fallback_light());
        assert_eq!(resolve(None, Appearance::Auto), Palette::fallback_dark());
        // A named theme outranks the installed one exactly as Light does.
        assert_eq!(resolve(Some(&dir), Appearance::RosePineDawn).bg, "#faf4ed");
        assert_eq!(resolve(None, Appearance::TokyoNight).bg, "#1a1b26");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// With no Omarchy theme, Auto is the system's choice; no preference is
    /// dark, as it always was.
    #[test]
    fn auto_without_omarchy_follows_the_system() {
        assert_eq!(Palette::for_system(SystemScheme::Light), Palette::fallback_light());
        assert_eq!(Palette::for_system(SystemScheme::Dark), Palette::fallback_dark());
        assert_eq!(Palette::for_system(SystemScheme::NoPreference), Palette::fallback_dark());
        assert_eq!(SystemScheme::from_portal(0), SystemScheme::NoPreference);
        assert_eq!(SystemScheme::from_portal(1), SystemScheme::Dark);
        assert_eq!(SystemScheme::from_portal(2), SystemScheme::Light);
        assert_eq!(SystemScheme::from_portal(7), SystemScheme::NoPreference);
    }

    const NAMED: [Appearance; 4] = [
        Appearance::TokyoNight, Appearance::CatppuccinMocha,
        Appearance::CatppuccinLatte, Appearance::RosePineDawn,
    ];

    /// WCAG contrast of two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let lum = |h: &str| {
            let c = |i: usize| {
                let x = f64::from(u8::from_str_radix(&h[i..i + 2], 16).unwrap()) / 255.0;
                if x <= 0.039_28 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Small text needs 4.5:1, and text lands on `bg` and on `surface` both.
    #[test]
    fn every_named_palette_is_legible() {
        for theme in NAMED {
            let p = Palette::named(theme).unwrap();
            for (role, c) in [("text", &p.text), ("muted", &p.muted), ("accent", &p.accent)] {
                assert!(contrast(c, &p.bg) >= 4.5, "{theme:?} {role} on bg: {:.2}", contrast(c, &p.bg));
                assert!(contrast(c, &p.surface) >= 4.5, "{theme:?} {role} on surface: {:.2}", contrast(c, &p.surface));
            }
            // Muted is the quieter of the two, which the theme file inverted.
            assert!(contrast(&p.muted, &p.bg) < contrast(&p.text, &p.bg), "{theme:?} muted outshouts text");
            assert_eq!(p.is_dark, contrast("#000000", &p.bg) < contrast("#ffffff", &p.bg), "{theme:?} is_dark");
        }
    }

    /// Stored as Omarchy spells the theme, sent as the same string, and read
    /// back through one table — so the settings row, the wire and the parse
    /// cannot drift apart.
    #[test]
    fn a_named_theme_round_trips_its_spelling() {
        for theme in NAMED.into_iter().chain([Appearance::Auto, Appearance::Light, Appearance::Dark]) {
            assert_eq!(Appearance::parse(theme.as_str()), Some(theme));
            assert_eq!(serde_json::to_value(theme).unwrap(), serde_json::json!(theme.as_str()));
            assert!(theme == Appearance::Auto || theme.is_pinned());
        }
        assert_eq!(Appearance::parse("solarized"), None);
    }

    /// Pre-Omarchy-4 and custom themes may have no `light_foreground`. Preserve
    /// their muted hue, but move it toward their own foreground just enough to
    /// make ordinary secondary labels readable.
    #[test]
    fn low_contrast_legacy_muted_text_is_lifted_to_wcag_aa() {
        let src = r##"
mode = "dark"
background = "#2e3440"
foreground = "#d8dee9"
muted = "#4c566a"
"##;
        let p = parse_colors(src).unwrap();
        assert_ne!(p.muted, "#4c566a");
        assert!(
            contrast_ratio(parse_hex(&p.muted).unwrap(), parse_hex(&p.bg).unwrap()) >= 4.5,
            "secondary text resolved to {} on {}",
            p.muted,
            p.bg,
        );
    }

    /// The author's `mode` outranks luminance — a deliberately dark-looking
    /// background declared light must classify as light.
    #[test]
    fn a_declared_mode_outranks_measured_luminance() {
        let dark_bg_declared_light = "mode = \"light\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"";
        assert!(!parse_colors(dark_bg_declared_light).unwrap().is_dark);

        let garbled_mode = "mode = \"mauve\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"";
        assert!(parse_colors(garbled_mode).unwrap().is_dark, "bad mode falls back to luminance");
    }

    /// A pre-4 colors.toml (accent only) must NOT parse as a full palette —
    /// that is what keeps the alacritty chain alive for old themes.
    #[test]
    fn an_accent_only_colors_file_is_not_a_full_palette() {
        assert!(parse_colors("accent = \"#e0af68\"").is_none());
        assert!(parse_colors("background = \"#1a1b26\"").is_none(), "foreground required");
        assert!(parse_colors("this is not toml {{{").is_none());
    }

    /// End to end: with both files present, colors.toml wins outright — the
    /// alacritty file deliberately disagrees on every value.
    #[test]
    fn a_full_colors_file_outranks_alacritty_entirely() {
        let temp_dir = std::env::temp_dir().join(format!("omacal_colors4_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        std::fs::write(
            temp_dir.join("alacritty.toml"),
            "[colors.primary]\nbackground = \"#000000\"\nforeground = \"#ffffff\"\n[colors.normal]\nblue = \"#0000ff\"",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("colors.toml"),
            "mode = \"dark\"\naccent = \"#e0af68\"\nmuted = \"#414868\"\nlight_foreground = \"#b4bee6\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"",
        )
        .unwrap();

        let p = resolve(Some(&temp_dir), Appearance::Auto);
        assert_eq!(p.bg, "#1a1b26");
        assert_eq!(p.text, "#a9b1d6");
        assert_eq!(p.accent, "#e0af68");
        assert_eq!(p.muted, "#b4bee6");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// An Omarchy 4 theme with no alacritty.toml at all still themes the app.
    #[test]
    fn a_colors_only_theme_directory_resolves() {
        let temp_dir = std::env::temp_dir().join(format!("omacal_noala_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        std::fs::write(
            temp_dir.join("colors.toml"),
            "mode = \"light\"\nbackground = \"#eff1f5\"\nforeground = \"#4c4f69\"\naccent = \"#1e66f5\"",
        )
        .unwrap();

        let p = resolve(Some(&temp_dir), Appearance::Auto);
        assert_eq!(p.bg, "#eff1f5");
        assert_eq!(p.accent, "#1e66f5");
        assert!(!p.is_dark);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn a_multibyte_background_does_not_panic() {
        let src = r##"
[colors.primary]
background = "€€"
foreground = "#c0caf5"
"##;
        // This should return None due to invalid hex, not panic on char boundary.
        let result = parse_alacritty(src);
        assert!(result.is_none(), "multibyte background should be rejected");
    }
}
