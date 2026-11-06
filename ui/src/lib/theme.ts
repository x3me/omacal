import { invoke } from '@tauri-apps/api/core';
import { foregroundFor } from './ink';

export type Palette = {
  bg: string; surface: string; text: string;
  muted: string; accent: string; is_dark: boolean;
  /** Present only on a theme whose look is more than colours (Daylight);
   *  absent means the stock look. Mirrors `theme::ThemeStyle`. */
  style?: ThemeStyle;
};

/** The knobs a theme may turn beyond its colours. The stock values — 7 / 2 /
 *  600, ink 65 / 60, second line 62 — are the literals the stylesheets fall
 *  back to when none of these variables is set, which is what keeps every
 *  other theme pixel-identical. */
export type ThemeStyle = {
  /** Percent of the calendar's colour in a timed block's fill. */
  tint: number;
  /** Width in px of the calendar-colour edge down a block's left side. */
  spine: number;
  /** `font-weight` of a block's title. */
  title_weight: number;
  /** Percent of the calendar's colour an event's text keeps (rest is the
   *  theme's ink): lower is a darker shade of the same hue. */
  ink: number;
  /** Percent opacity of a block's second line (time, location). */
  sub: number;
  /** Fill of today's column. */
  today: string;
  /** Size (px) and weight of the header's month-and-year title. */
  heading_size: number;
  heading_weight: number;
};

/** Pushes a resolved palette onto :root so all styling flows from CSS vars.
 *  The one place that knows the variable names — both the startup fetch
 *  (`applyPalette`) and the live `theme-changed` listener go through this. */
export function setPalette(p: Palette): void {
  const r = document.documentElement.style;
  // Not a variable: the one property here the engine reads itself. Scrollbars
  // are the piece of the window omacal never paints — WKWebView and WebKitGTK
  // draw them from `color-scheme`, and with it unset the page counts as light,
  // which put a white scrollbar on every dark theme. Branching on `is_dark`
  // like `--hairline` below keeps a light Omarchy theme's chrome light, and
  // routing it through here is what makes `theme-changed` restyle it live.
  r.setProperty('color-scheme', p.is_dark ? 'dark' : 'light');
  r.setProperty('--bg', p.bg);
  r.setProperty('--surface', p.surface);
  r.setProperty('--text', p.text);
  r.setProperty('--muted', p.muted);
  r.setProperty('--accent', p.accent);
  // The ink for text ON an accent fill — the active view chip, the Connect
  // button, the today badge. It used to be `var(--bg)`, which worked only as
  // long as the background happened to contrast with the accent; a light
  // theme with a light accent (Rose Pine Dawn's rose) drew light-on-light.
  // `foregroundFor` is the same luminance rule the Big Year pills already
  // live by, so an accent from colors.toml — which can be anything the theme
  // author liked — always gets a legible ink chosen against the fill itself.
  r.setProperty('--on-accent', foregroundFor(p.accent));
  r.setProperty('--hairline', p.is_dark ? 'rgba(255,255,255,.055)' : 'rgba(0,0,0,.07)');
  r.setProperty('--hour-rule', p.is_dark ? 'rgba(255,255,255,.035)' : 'rgba(0,0,0,.05)');
  r.setProperty('--today-tint', p.style?.today ?? (p.is_dark ? 'rgba(255,255,255,.028)' : 'rgba(0,0,0,.025)'));
  // The two endpoints for text drawn on a *calendar's own colour* rather than
  // on the theme's background — Big Year's solid pills. Which one a given
  // pill takes is a per-event decision made from that colour's relative
  // luminance (`foregroundFor`, `BigYearRibbon.svelte`), because `ev.color`
  // arrives from Google and can be anything: omacal shows dark blues and pale
  // yellows side by side, and a single `color:` fails one end or the other.
  //
  // Unlike the three above, these do **not** branch on `is_dark`, and that is
  // deliberate rather than an omission. The fill they sit on is the same in
  // either theme — Google's hex does not know what omacal's background is —
  // so the only thing they need contrast against is that fill. Dimming the
  // light ink for a dark theme would be choosing contrast against the wrong
  // surface, and would take it away from exactly the pills that need it.
  //
  // Not quite opaque: at 8px on a saturated chip, a flat #000/#fff reads as
  // printed-on rather than part of the pill. .88/.96 over the fill keeps the
  // contrast (~15:1 against white, ~17:1 against a mid-dark fill) and loses
  // the hard edge. `rgba()` rather than hex for the reason the three above
  // use it: this file states colours, and hex literals are spent elsewhere.
  r.setProperty('--ink-on-light', 'rgba(0,0,0,.88)');
  r.setProperty('--ink-on-dark', 'rgba(255,255,255,.96)');

  // Three variables, not one, and the reason is the whole point of publishing
  // them at all.
  //
  // `--error` and `--now` are the same hex today, and they were the same hex
  // literal in six places before this. That coincidence is exactly what made a
  // single `--error` covering both the wrong answer: a theme that wanted a
  // calmer "now" indicator would have silently restyled every error message in
  // the app, and a redder error would have repainted the current-time line.
  // They are different meanings that happen to have agreed on a value, and
  // naming them apart keeps the option of breaking that agreement without
  // anybody having to notice the coupling first.
  //
  // Each is stated once here rather than derived from another: `--now` is not
  // `var(--error)`, because writing it that way would re-create the coupling
  // in the one file that exists to prevent it.
  //
  // The values are byte-identical to the literals they replace. Whether any of
  // the three *should* vary with `is_dark` — as `--hairline` and its two
  // neighbours above do — is a live question and deliberately not answered
  // here: this change is a refactor, and its witness is that every screenshot
  // golden is unchanged.

  /** Something failed and the user has to read about it: the error banner in
   *  `Header`, the form's own `.err`, and the two popovers' `.note.err`. Each
   *  tints its background from this with `color-mix(… 9%, transparent)`
   *  rather than carrying a second variable for the wash. */
  r.setProperty('--error', '#e2564a');
  /** `WeekGrid`'s current-time line and its dot — "the loudest thing on
   *  screen, deliberately", and nothing to do with anything being wrong. */
  r.setProperty('--now', '#e2564a');
  /** The DEMO DATA badge: a standing warning that the data is synthetic, which
   *  is neither an error nor a clock. */
  r.setProperty('--demo', '#e2a03f');

  // The two quiet states of the header's status light. Its two loud ones
  // already have variables — `--accent` while a sync is running, `--error`
  // when one failed — and adding hues for these would be adding colour to say
  // "nothing is wrong", which is the opposite of what they mean.
  //
  // Neutral, and branching on `is_dark` like `--hairline` and its neighbours
  // three blocks up, because unlike the ink pair these sit on the app's own
  // background rather than on a calendar's colour.
  /** Synced, and recently. Quiet enough to ignore, but **unmistakably
   *  alive**: this was a neutral at 30%/28%, and in the field that proved
   *  indistinguishable from `--sync-idle` at a glance — "is it actually
   *  working?" asked about a working sync (2026-08-19). A muted green says
   *  "fine" the way every status light users already know does, without
   *  the saturation that would make a healthy state something the eye has
   *  to look past all day. */
  r.setProperty('--sync-ok', p.is_dark ? '#79b98c' : '#35855b');
  /** Nothing to be stale — signed out, or signed in and nothing fetched yet.
   *  Dimmer than `--sync-ok` rather than a different hue: the difference
   *  between "fine" and "nothing yet" is not worth a colour, and both are
   *  states nobody needs to act on. */
  r.setProperty('--sync-idle', p.is_dark ? 'rgba(255,255,255,.14)' : 'rgba(0,0,0,.13)');

  setThemeStyle(r, p.style);
}

/** The style overrides: set for a theme that carries a `ThemeStyle`,
 *  *removed* for one that does not, so switching Daylight -> anything puts the
 *  stock look back (the stylesheets' `var(--x, <stock>)` fallbacks). Removing
 *  rather than writing the stock values is deliberate: there is then one copy
 *  of each stock number, in the CSS that uses it. Same set-or-remove shape as
 *  the corner-style variables in `appearance.ts`.
 *
 *  Three are derived rather than carried, so a theme does not restate them: a hovered block deepens 9 points past its rest tint (7 -> 16
 *  today), and the all-day and month chips, which already sit at 16%, take
 *  their title one step lighter than a block's (600 -> 500 today), and the
 *  title's year takes the accent. Today's column is the one knob that is not
 *  a variable of its own: it replaces `--today-tint` in `setPalette`. */
function setThemeStyle(r: CSSStyleDeclaration, s: ThemeStyle | undefined): void {
  const vars: Record<string, string | undefined> = {
    '--event-tint': s && `${s.tint}%`,
    '--event-tint-hover': s && `${s.tint + 9}%`,
    '--event-spine-width': s && `${s.spine}px`,
    '--event-title-weight': s && String(s.title_weight),
    '--event-chip-weight': s && String(s.title_weight - 100),
    '--event-ink': s && `${s.ink}%`,
    '--event-sub-opacity': s && String(s.sub / 100),
    '--heading-size': s && `${s.heading_size}px`,
    '--heading-weight': s && String(s.heading_weight),
    // Only the year takes the accent, and only a themed title does the
    // colouring: `Header` reads this as `var(--title-year, inherit)`.
    '--title-year': s && 'var(--accent)',
  };
  for (const [name, value] of Object.entries(vars)) {
    if (value === undefined) r.removeProperty(name);
    else r.setProperty(name, value);
  }
}

/**
 * **The colours a user may give a calendar**, and the only place they are
 * written down.
 *
 * A curated set rather than a free colour picker, for one reason that decides
 * it: omacal draws on both a light and a dark Omarchy theme, and a colour a
 * user picks at 2am on a dark background can be unreadable on a light one.
 * Every entry here was chosen to hold up on both — `ink.ts` then guarantees the
 * *text* on top reads correctly for any of them, by relative luminance, so
 * nothing here has to also carry a foreground.
 *
 * Hex rather than `rgba()`, unlike the variables above, and deliberately:
 * these are not theme colours. They travel to Google's own colour field's
 * neighbour in the database, are stored per calendar, and are composited by
 * `color-mix` against whatever the theme's background happens to be. A value
 * that changed with the theme would mean an event silently changing colour
 * when the desktop did.
 *
 * Named because the names are what a control announces: a swatch a screen
 * reader calls "button" is a swatch nobody can choose deliberately.
 */
export const CALENDAR_COLOURS: Array<[name: string, hex: string]> = [
  ['Blue', '#5b8def'],
  ['Teal', '#2aa198'],
  ['Green', '#5aa84f'],
  ['Olive', '#8a9a3b'],
  ['Amber', '#e2a03f'],
  ['Orange', '#e07b39'],
  ['Red', '#e2564a'],
  ['Pink', '#e06c9f'],
  ['Purple', '#9a7bd0'],
  ['Slate', '#7b8a9a'],
];

/** Fetches the resolved palette and applies it. Used once at startup. */
export async function applyPalette(): Promise<Palette> {
  const p = await invoke<Palette>('get_palette');
  setPalette(p);
  return p;
}
