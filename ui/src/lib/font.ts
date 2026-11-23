import { invoke } from '@tauri-apps/api/core';

/** Longer than any family a font ships with; the backend's `font.rs` holds
 *  the same bound. */
const MAX_FAMILY_CHARS = 128;

/**
 * `name` as a CSS string, ready to lead a `font-family` list, or `null` when
 * it cannot be one safely.
 *
 * The name comes from a command's output (`fc-match`, by way of
 * `get_omarchy_font`), so it is refused rather than escaped if it carries a
 * double quote, a backslash or a control character: those are what could end
 * the string, and no font is named with them. Refused here even though the
 * backend refuses the same set, because this is where a string becomes CSS.
 *
 * Quoted whatever it looks like: an unquoted family must parse as
 * identifiers, and Nerd Fonts' `3270 Nerd Font` does not — one such name
 * would void the whole `font-family` and drop the window to the default
 * serif.
 */
export function cssFamily(name: string | null | undefined): string | null {
  const family = name?.trim();
  if (!family || [...family].length > MAX_FAMILY_CHARS || /["\\\p{Cc}]/u.test(family)) return null;
  return `"${family}"`;
}

/**
 * Puts `family` at the head of the app's font stack, or takes it off again.
 *
 * `App`'s body reads `var(--omarchy-font,)` in front of its own stack. The
 * fallback is empty, so while this is unset the declaration is exactly the
 * stack it always was; set, the value carries its own comma. Removed rather
 * than set to something neutral, the way `setThemeStyle` removes its
 * variables, so there is one copy of the stack, in the stylesheet.
 */
export function setOmarchyFont(family: string | null, root: HTMLElement = document.documentElement): void {
  const css = cssFamily(family);
  if (css) root.style.setProperty('--omarchy-font', `${css},`);
  else root.style.removeProperty('--omarchy-font');
}

/** Which ask is the latest. An answer to an older one is stale: switched on
 *  and straight back off, the "on" answer (fontconfig, up to two seconds)
 *  can land after the "off" one, and must not put the font back. */
let asks = 0;

/**
 * Asks the backend for the Omarchy font and wears its answer: the family
 * while "Use the Omarchy font" is on and fontconfig names one, nothing
 * otherwise. The backend decides; this never reads the setting itself, so
 * turning it off is the same call as turning it on.
 */
export async function applyOmarchyFont(root: HTMLElement = document.documentElement): Promise<void> {
  const ask = ++asks;
  const family = await invoke<string | null>('get_omarchy_font').catch(() => null);
  if (ask === asks) setOmarchyFont(family, root);
}
