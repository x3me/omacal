import type { Placed } from './api';

/** Original packed positions keep overlap markers aligned with the hover
 * targets, even while the active event fills the day column. */
export type ColorSegment = { color: string; left: number; width: number };

/** Placement fractions can differ by rounding at a shared hour boundary. */
export function containsPlacement(outer: Placed, inner: Placed): boolean {
  return inner.top >= outer.top - 1e-9
    && inner.top + inner.height <= outer.top + outer.height + 1e-9;
}
