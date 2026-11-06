import type { UiEvent } from './api';

/**
 * The two looks Show as and visibility give an event in the views, copied
 * from Fantastical (2026-11-05, by request): a free event draws its colour
 * bar as an outline instead of a solid stripe, and a private one carries a
 * small lock in its corner. Busy and default visibility are the look every
 * event had before, so neither predicate is true for them.
 */

/** Shown as free: time the user is still available in. */
export const isFree = (ev: Pick<UiEvent, 'transparency'>): boolean => ev.transparency === 'transparent';

/** Private or confidential. The views never tell the two apart: both mean
 *  "others see only that you are busy", and confidential is a CalDAV value
 *  the dialog can show but not set. */
export const isPrivate = (ev: Pick<UiEvent, 'visibility'>): boolean =>
  ev.visibility === 'private' || ev.visibility === 'confidential';
