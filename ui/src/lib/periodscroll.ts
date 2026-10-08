// Month/Year period paging — the decisions behind the wheel and swipe
// listeners, with no DOM and no clock of its own. Spec:
// `docs/superpowers/specs/2026-10-06-omacal-month-year-paging-design.md`.
//
// `App` owns the listeners and the animated snap; this module owns only *when*
// a gesture has earned a page, and how long the tail is muted afterwards, so
// each rule can be tested directly (the reason `zoom.ts` and `weekwindow.ts`
// are shaped the same way). A gesture moves exactly one period: a wheel's
// momentum is not a second gesture, and a slow drag is not one at all.

import { STRONG_SWIPE_MIN_PX, STRONG_SWIPE_PX_PER_MS, velocityOf, type PanSample } from './weekwindow';

/** Vertical wheel travel, in px, gathered before a page is earned. One mouse
 *  notch (~100px) clears it in a single event; a trackpad's many small events
 *  need a few, which is what stops a stray two-finger wobble from paging. */
export const PAGE_THRESHOLD_PX = 50;

/** How long after a page the tail is ignored before it would count again. The
 *  lock is **not fixed**: every tail event re-arms it by `PAGE_IDLE_MS`, so it
 *  ends only once the momentum goes quiet. A touchpad keeps emitting for a
 *  second or more after the hand leaves, and a fixed lock lapses mid-tail and
 *  lets a second page through (review 2026-10-08). Week ends its pan the same
 *  way (`panLull`/`PAN_LULL_MS`). */
export const PAGE_LOCK_MS = 350;

/** A gap this long ends the gesture. It is both the idle reset — the
 *  accumulator starts over, so a slow drag cannot quietly bank travel across
 *  separate gestures — and the window each tail event re-arms the lock to. */
export const PAGE_IDLE_MS = 180;

/** The snap's travel time and easing, shared with `App`'s animated page. */
export const PAGE_SLIDE_MS = 260;
export const PAGE_EASE = 'cubic-bezier(.22,.61,.36,1)';

/** One gesture's gathered travel and its lock. Sign is the direction: positive
 *  is down/forward. */
export type PageAccumulator = {
  acc: number;
  /** Wheel events before this instant are the previous gesture's tail. */
  lockedUntil: number;
};

export function newAccumulator(): PageAccumulator {
  return { acc: 0, lockedUntil: 0 };
}

/** Feed one vertical wheel event. Returns the accumulator to store and a page
 *  intent: `1` forward (next period), `-1` back, `0` nothing. Down is forward,
 *  the mainstream convention (`docs/superpowers/specs/2026-08-28-...:236` and
 *  ticket 06). */
export function wheelPage(
  a: PageAccumulator, dy: number, now: number,
): { acc: PageAccumulator; page: -1 | 0 | 1 } {
  // This is the last gesture's tail. Dropped whole, not accumulated (or the
  // tail would merely page a little later), and the lock is re-armed so it ends
  // only after `PAGE_IDLE_MS` of quiet however long the momentum runs.
  if (now < a.lockedUntil) return { acc: { ...a, lockedUntil: now + PAGE_IDLE_MS }, page: 0 };
  const acc = a.acc + dy;
  if (Math.abs(acc) < PAGE_THRESHOLD_PX) return { acc: { ...a, acc }, page: 0 };
  // The one page. The lock starts now, not at lift, because a wheel has no
  // lift; `PAGE_IDLE_MS` is what ends the gesture.
  return { acc: { acc: 0, lockedUntil: now + PAGE_LOCK_MS }, page: acc > 0 ? 1 : -1 };
}

/** End the gesture: drop sub-threshold travel so it cannot be resumed by the
 *  next one. Idempotent, and returns the same object when there is nothing to
 *  clear. */
export function resetAccumulator(a: PageAccumulator): PageAccumulator {
  return a.acc === 0 ? a : { ...a, acc: 0 };
}

/** Whether `user` prefers reduced motion — the snap becomes an instant swap,
 *  still exactly one period. Reads the media query only; a caller that cannot
 *  (`matchMedia` missing in a test) gets `false`, i.e. the animated default. */
export function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** A vertical swipe's page intent, from its net finger travel (px, positive
 *  down) and the samples gathered along the way. Reuses `#118`'s measured
 *  strong-swipe rule — a minimum distance *and* a speed of the hand — but none
 *  of its track or spring: Month/Year have no day-column track to slide, so a
 *  gentle swipe deliberately does nothing rather than following the finger
 *  (ticket 08). */
export function swipePage(travelPx: number, samples: PanSample[]): -1 | 0 | 1 {
  if (Math.abs(travelPx) < STRONG_SWIPE_MIN_PX) return 0;
  if (Math.abs(velocityOf(samples)) < STRONG_SWIPE_PX_PER_MS) return 0;
  // Content-drag direction, so the sheet follows the finger: finger up
  // (negative travel) is forward. Thunderbird's month view does the same, and
  // it is the opposite of the wheel's down-as-forward only because a drag moves
  // content while a wheel moves the viewport (ticket 06).
  return travelPx < 0 ? 1 : -1;
}
