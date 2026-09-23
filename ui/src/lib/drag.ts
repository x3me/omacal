/**
 * The drag geometry: given where a pointer went, what span results.
 *
 * **No DOM, no pointer events, no component.** Everything here is a function of
 * its arguments, which is what lets the off-by-ones live somewhere they can be
 * tested exhaustively — spec §7. The gesture (threshold, escape,
 * drop-where-it-started) is the browser's half and is not in this file.
 *
 * Two conventions are inherited from `WeekGrid`, not invented here, because the
 * arithmetic has to agree with what is on screen:
 *
 * - **The vertical axis is a fraction of the day's own span.** A day is 23 or
 *   25 hours across a transition, and `hourFrac`/`slotAt` already lay a column
 *   out by its real length. Dividing by a fixed 24 puts every drop after the
 *   transition an hour out.
 * - **Snapping is in local wall-clock minutes**, never by rounding the instant
 *   to a multiple of the interval. `slotAt` documents why: a zone offset at :45
 *   (Kathmandu, Chatham) has no half hour on a whole-half-hour UTC boundary, so
 *   epoch-rounding would offer 09:15 for a drop on the 09:30 line.
 */

/**
 * How far the pointer must travel, with the button held, before a press
 * becomes a drag rather than a click (spec §4).
 *
 * Four pixels. Without it every click on an event is a potential fifteen-minute
 * move, because no hand holds a mouse perfectly still between pressing and
 * releasing — and a user who can no longer open an event by clicking it has
 * lost more than dragging gains.
 */
export const DRAG_THRESHOLD_PX = 4;

/** The snap interval (spec §5): fifteen minutes, which is how meetings are
 *  actually scheduled and lands on clean times without fighting. */
export const SNAP_MS = 15 * 60_000;

/**
 * Whether a press that has travelled `dx`, `dy` has become a drag.
 *
 * Straight-line distance rather than either axis alone: a diagonal wander of
 * three pixels each way is five pixels of travel, and treating it as a click
 * because neither component reached four would make the threshold mean
 * something different in every direction.
 *
 * Here rather than in the component because it is arithmetic, and arithmetic
 * in a Svelte file is arithmetic without a table.
 */
export function beganDrag(dx: number, dy: number, threshold: number = DRAG_THRESHOLD_PX): boolean {
  return Math.hypot(dx, dy) >= threshold;
}

/**
 * How thick the grab band at each end of a block is, in pixels.
 *
 * Six, which is a little larger than the 4px drag threshold on purpose: a
 * press that begins inside the band and wanders is a resize, and if the band
 * were the smaller of the two a hand could leave it before the drag even
 * started and silently become a move instead.
 */
export const RESIZE_EDGE_PX = 6;

/**
 * Which edge of a block a press at `offsetY` grabs, or `null` for the middle.
 *
 * **A short block has no edges at all.** Below three bands' worth of height
 * there is no middle left between them, and every press on a 15-minute event
 * would resize it — the block would become impossible to *move*, which is the
 * commoner gesture by far. `null` there means such a block can still be
 * dragged, and resized by making it longer from a taller sibling or the form.
 */
/** Whether a block is tall enough to have edges at all — `edgeAt`'s own
 *  first rule, exported because two components draw grips from it.
 *
 *  `EventBlock` decides whether to draw them on a block and `WeekGrid`
 *  whether to draw them on the form's draft; both restated `>= band * 3` by
 *  hand. The failure mode of a drift is visible grips that do nothing, or
 *  invisible edges that resize — the second of which `drag.ts` records having
 *  been reported once already. */
export function hasResizeEdges(height: number, band: number = RESIZE_EDGE_PX): boolean {
  return height >= band * 3;
}

export function edgeAt(
  offsetY: number,
  height: number,
  band: number = RESIZE_EDGE_PX,
): 'start' | 'end' | null {
  if (!hasResizeEdges(height, band)) return null;
  if (offsetY <= band) return 'start';
  if (offsetY >= height - band) return 'end';
  return null;
}

/**
 * How many day columns a horizontal travel of `dx` crosses.
 *
 * Rounded, so the block lands in the column the pointer is nearest rather than
 * the one it has fully entered: dragging a little over halfway into Tuesday
 * means Tuesday. Ties go **away from zero** — `Math.round` alone rounds −0.5 to
 * −0, which would make a drag exactly half a column to the *left* stay put
 * while the same drag to the right moved, and a gesture that behaves
 * differently in mirror image is a gesture nobody can predict.
 *
 * A column of zero width crosses nothing rather than dividing by it.
 */
export function colsMoved(dx: number, colWidth: number): number {
  if (colWidth <= 0) return 0;
  const cols = dx / colWidth;
  const n = cols < 0 ? -Math.round(-cols) : Math.round(cols);
  // `+ 0` turns `-0` into `0`. Not cosmetic: a drag a little to the left
  // produces `-0` here, and `Object.is(-0, 0)` is false — so a caller
  // comparing against zero, or a spec asserting it, disagrees with itself
  // depending on which way the pointer went.
  return n + 0;
}

/** A span of time. The shape a drag produces and a write consumes. */
export type Span = { startMs: number; endMs: number };

/**
 * `ms` moved to the nearest `intervalMs` boundary, **counted in local
 * wall-clock minutes past the hour**.
 *
 * Ties round **up** — 7:30 into a 15-minute slot becomes 15, not 0. That is
 * `Math.round`'s own rule and the one that is easiest to say out loud:
 * *nearest, and ties go forward*. It is a decision rather than a detail,
 * because a drag sits on the midpoint every time the pointer is halfway
 * between two lines.
 *
 * `intervalMs` must divide an hour. 15 minutes does, which is spec §5's answer
 * and the only value this app passes; the constraint is what lets the snap be
 * done by setting minutes on a `Date` and never touching the hour, which in
 * turn is what keeps it away from daylight-saving arithmetic entirely.
 * `slotAt` makes the same assumption for its 30.
 */
export function snapMs(ms: number, intervalMs: number): number {
  const intervalMin = intervalMs / 60_000;
  const at = new Date(ms);
  // Minutes past the hour, carrying the sub-minute part so a value just short
  // of a boundary rounds on to it rather than being truncated back.
  const within = at.getMinutes() + at.getSeconds() / 60 + at.getMilliseconds() / 60_000;
  // `setMinutes` handles a rounded 60 by rolling the hour itself, in local
  // time, which is exactly what is wanted and is why the hour is never
  // computed here.
  at.setMinutes(Math.round(within / intervalMin) * intervalMin, 0, 0);
  return at.getTime();
}

/**
 * The span an event lands on when it is dragged.
 *
 * `dyFrac` is how far the pointer travelled down the column as a fraction of
 * its height, `dayMs` the span of the day it is being dragged in, and `dxCols`
 * how many day columns it crossed. `dayCols` is the number of columns in the
 * view, and bounds the horizontal movement so a drag cannot leave the week it
 * is in.
 *
 * **The duration never changes.** The end is the new start plus the span that
 * went in, and is deliberately *not* recomputed from the pointer: a move that
 * derived both ends independently would resize the event by however much the
 * two roundings disagreed, silently, while still landing where the test looked.
 *
 * **A day is a civil day.** `setDate` keeps the wall-clock time and moves the
 * date, so dragging a 09:00 meeting across a spring-forward leaves it at 09:00
 * — twenty-three hours later, not twenty-four. Adding `dxCols * 86400000`
 * would move it to 10:00, which is the defect this project has closed twice
 * elsewhere and has no business reintroducing through a gesture.
 *
 * A move of nothing returns the very instants it was given, rather than a
 * value that merely reads the same: §4 says putting an event back where it came
 * from must be free, and a civil round trip through a repeated hour could
 * otherwise hand back the other pass of it.
 */
export function spanForMove(
  origin: Span,
  dyFrac: number,
  dayMs: number,
  dayCols: number,
  dxCols: number,
  snapInterval: number,
): Span {
  const duration = origin.endMs - origin.startMs;

  // Clamped rather than refused: a pointer dragged off the right-hand edge
  // should pin to the last column, not throw away the gesture.
  const cols = clamp(dxCols, -(dayCols - 1), dayCols - 1);

  const moved = dyFrac === 0 && cols === 0
    ? origin.startMs
    : addDays(snapMs(origin.startMs + dyFrac * dayMs, snapInterval), cols);

  return { startMs: moved, endMs: moved + duration };
}

/**
 * The span an event lands on when one of its edges is dragged.
 *
 * The other edge stays exactly where it was — spec §5 — so a resize is never
 * also a move.
 *
 * **A resize may not invert the event.** Dragging an edge past its opposite
 * clamps to a minimum span rather than producing a negative one:
 * `endAfterStart` already refuses an inverted span in the form, and a grid able
 * to construct one would produce a block the form then rejects. That
 * inconsistency is the defect, not the negative number.
 *
 * The minimum is the snap interval itself, deliberately rather than a second
 * constant. The smallest span the grid can express is the smallest one it
 * should be able to produce, and a separate number would be one more thing to
 * keep in step with §5.
 */
export function spanForResize(
  origin: Span,
  edge: 'start' | 'end',
  dyFrac: number,
  dayMs: number,
  snapInterval: number,
): Span {
  const minimum = snapInterval;

  if (edge === 'start') {
    const wanted = snapMs(origin.startMs + dyFrac * dayMs, snapInterval);
    return { startMs: Math.min(wanted, origin.endMs - minimum), endMs: origin.endMs };
  }

  const wanted = snapMs(origin.endMs + dyFrac * dayMs, snapInterval);
  return { startMs: origin.startMs, endMs: Math.max(wanted, origin.startMs + minimum) };
}

/**
 * The span swept out over empty grid, from `fromFrac` to `toFrac` down a
 * column whose day starts at `dayStartMs` and runs for `dayMs`.
 *
 * **Nothing is created by this.** It answers which two instants the event form
 * opens on; the form is what makes an event, because a new event needs a title
 * and that is where a title lives.
 *
 * Both fractions are of the column's own height and both ends snap
 * independently, so a sweep lands on the grid rather than under the finger at
 * either end. `dayMs` rather than a constant day for the reason `spanForMove`
 * gives: a day either side of a transition is 23 or 25 hours, and dividing by a
 * fixed 24 puts every sweep after it an hour out.
 *
 * **A sweep has no direction.** Dragging from 15:00 up to 14:00 is the same
 * gesture read backwards and produces 14:00–15:00 — never a negative span,
 * which `endAfterStart` would refuse and which would open a form that cannot be
 * saved. Same family as `spanForResize`'s inversion clamp, different code path.
 *
 * **And a minimum, which is not cosmetic.** A hand that twitches between press
 * and release sweeps a few pixels, both ends snap to the same slot, and the raw
 * span is zero — a form that opens already refusing to save with no field on it
 * visibly wrong (§7.2 of the form-time spec, arriving through a new door). The
 * minimum is the snap interval, the same value and for the same reason
 * `spanForResize` uses it: the smallest span the grid can express is the
 * smallest one it should be able to produce.
 *
 * It always extends the **end** forward from the earlier of the two points, so
 * a sweep of nothing gives the same answer whichever way the hand moved.
 *
 * A pointer dragged off the top or bottom of the column pins to the day's own
 * ends rather than sweeping into its neighbours, which is the clamp `slotAt`
 * already applies to a click — including its one odd corner, that a sweep
 * pinned to the very bottom names the following midnight. That is the honest
 * reading of "swept to the end of the day", and it is what a click there has
 * always done.
 */
export function spanForSweep(
  dayStartMs: number,
  dayMs: number,
  fromFrac: number,
  toFrac: number,
  snapInterval: number,
): Span {
  const instantAt = (frac: number) =>
    snapMs(dayStartMs + clamp(frac, 0, 1) * dayMs, snapInterval);

  const a = instantAt(fromFrac);
  const b = instantAt(toFrac);
  const startMs = Math.min(a, b);
  return { startMs, endMs: Math.max(Math.max(a, b), startMs + snapInterval) };
}

/**
 * What a sweep across the grid is asking for.
 *
 * One shape with two arms rather than a span plus a flag: the two ends of an
 * all-day span are *days*, not instants, and the form takes them as dates —
 * carrying them as a `Span` would invite somebody to read a wall clock off a
 * value that never had one.
 */
export type SweepAsk =
  | { kind: 'timed'; startMs: number; endMs: number }
  | { kind: 'allDay'; firstDayMs: number; lastDayMs: number };

/**
 * A sweep's ask, from how far the hand travelled in each direction.
 *
 * **Sideways wins.** Cross a column boundary and this stops being a meeting
 * and becomes an all-day span over the days the hand crossed — first to last
 * inclusive, in either direction, because a sweep that ends left of where it
 * began still names the same set of days (requested 2026-08-31). Staying in
 * one column keeps the old behaviour exactly: a timed event, and only the
 * vertical travel is read.
 *
 * This overturns an earlier deliberate rule — sideways travel used to count
 * toward the drag threshold and nothing else, on the grounds that a sweep
 * silently producing a three-day event would be a nasty surprise. The
 * surprise was the *silence*: `WeekGrid` now ghosts every covered column for
 * the whole of its height, so what will be created is on screen before the
 * button comes up.
 *
 * `days` is the visible day list; `from` indexes the column the press landed
 * in. The far column is clamped into the list, so sweeping off the edge of
 * the week stops at the week's edge rather than naming a day nobody can see.
 */
export function sweepAsk(
  days: readonly { start_ms: number; end_ms: number }[],
  from: number,
  cols: number,
  fromFrac: number,
  toFrac: number,
  snapInterval: number,
): SweepAsk | null {
  const origin = days[from];
  if (!origin) return null;
  if (cols === 0) {
    const { startMs, endMs } = spanForSweep(
      origin.start_ms,
      origin.end_ms - origin.start_ms,
      fromFrac,
      toFrac,
      snapInterval,
    );
    return { kind: 'timed', startMs, endMs };
  }
  const to = clamp(from + cols, 0, days.length - 1);
  const [a, b] = from <= to ? [from, to] : [to, from];
  return { kind: 'allDay', firstDayMs: days[a].start_ms, lastDayMs: days[b].start_ms };
}

/** `n` whole **civil** days on from `ms`, keeping the local wall-clock time. */
function addDays(ms: number, n: number): number {
  if (n === 0) return ms;
  const at = new Date(ms);
  at.setDate(at.getDate() + n);
  return at.getTime();
}

const clamp = (n: number, lo: number, hi: number): number => Math.min(Math.max(n, lo), hi);
