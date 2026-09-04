<!-- ui/src/lib/AllDayBand.svelte -->
<script lang="ts">
  import type { Lane, UiEvent } from './api';
  import type { Rect } from './position';
  import { gutterWidth } from './secondzone.svelte';
  import { cursorNamesEvent, type KeyboardCursor } from './keyboardnav';

  let { lanes, events, overflow, columns = 7, visible = columns, vis = 0, pan = 0, sliding = false, expanded = false, onexpand = null, dayStarts = [], keyboardCursor = null, onopen }:
    { lanes: Lane[]; events: UiEvent[]; overflow: number[]; columns?: number;
      /** The chips ride `WeekGrid`'s track (2026-09-03): `columns` drawn,
       *  `visible` of them on screen, `vis` columns of padding before the
       *  window and `pan` columns of finger travel — the grid's own numbers,
       *  handed down so the band's columns slide with the day columns or
       *  the chips shear off their days. The defaults are a band that does
       *  not slide, which is what every existing mount asks for. */
      visible?: number; vis?: number; pan?: number;
      /** While the grid's track is sliding — the only time the offset is
       *  applied, and by transform; see `WeekGrid`'s `.cols.sliding`. */
      sliding?: boolean;
      /** Whether every row is drawn, or four with the rest behind "+N more".
       *  The band does not decide it: the rows it is handed are packed to
       *  the same answer, so the two cannot disagree. */
      expanded?: boolean;
      /** "+N more" was clicked. Optional: a mount that passes nothing gets
       *  the label it always had, which is a label and not a control —
       *  nothing here should invite a click it cannot answer. */
      onexpand?: (() => void) | null;
      dayStarts?: number[];
      keyboardCursor?: KeyboardCursor | null;
      /** Same contract as `EventBlock`'s, and wired to the same
       *  `WeekGrid.openPopover`. Required rather than optional: every
       *  `is_all_day` event is routed here by `commands::assemble_week`, so a
       *  chip is the *only* representation one ever gets — a caller that
       *  omitted this would leave an all-day off-site with a guest list
       *  unopenable, which is the state this prop exists to end. */
      onopen: (event: UiEvent, rect: Rect) => void } = $props();

  // Only to place the "+N more" row on the track just past the last occupied
  // lane. Not a reserved height, unlike `BigYearRibbon`'s
  // `RESERVED_PILL_LANES`: `pack_lanes` fills lanes from 0 up, so every lane
  // below this one carries a chip and `.rows` is already exactly this many
  // chips tall. There is nothing for a height derived from it to hold open.
  const laneCount = $derived(lanes.length ? Math.max(...lanes.map((l) => l.lane)) + 1 : 0);

  // `getBoundingClientRect()` for the same reason `EventBlock` uses it: the
  // popover places itself against the viewport, and a chip's own geometry is
  // grid-line coordinates, which say nothing about where it landed on screen.
  function open(event: UiEvent, e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopen(event, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  function isKeyboardSelected(lane: Lane, event: UiEvent): boolean {
    if (!keyboardCursor) return false;
    const column = dayStarts.indexOf(keyboardCursor.dayStartMs);
    return column >= lane.start_col && column <= lane.end_col
      && cursorNamesEvent(keyboardCursor, keyboardCursor.dayStartMs, event);
  }
</script>

{#if lanes.length || overflow.length}
  <div class="band" class:expanded style="--gutter:{gutterWidth()}">
    <div class="label">ALL-DAY</div>
    <div class="track">
    <div class="rows" class:sliding style="--cols:{columns}; --visible:{visible}; --vis:{vis}; --pan:{pan}">
      <!-- Keyed by the event's index, so a re-pack moves a chip's node rather
           than tearing it down and building another where it lands. -->
      {#each lanes as lane (lane.idx)}
        {@const ev = events[lane.idx]}
        {@const keyboardSelected = isKeyboardSelected(lane, ev)}
        <button
          class="chip"
          class:cl={lane.cont_left}
          class:cr={lane.cont_right}
          class:keyboard={keyboardSelected}
          data-kbd-selected-event={keyboardSelected ? '' : undefined}
          style="
            grid-row:{lane.lane + 1};
            grid-column:{lane.start_col + 1} / {lane.end_col + 2};
            --start:{lane.start_col};
            --span:{lane.end_col - lane.start_col + 1};
            --cal:{ev.color};
          "
          title={ev.title}
          onclick={(e) => open(ev, e)}
        >
          {lane.cont_left ? '‹ ' : ''}{ev.title}
        </button>
      {/each}
      <!-- The fold, and since 2026-09-04 a real control: a week with more
           all-day events than the band draws used to say "+70 more" and do
           nothing when clicked (Michael Brennan, by email, on 1.2.0). Still
           a plain label without `onexpand`, for the reason `MonthGrid`'s
           row-level one is one: a label that cannot answer a click must not
           invite it. -->
      {#if overflow.length || expanded}
        {#if onexpand}
          <button class="more" type="button" aria-expanded={expanded}
                  style="grid-row:{laneCount + 1}; grid-column:1 / -1"
                  onclick={onexpand}>
            {overflow.length ? `+${overflow.length} more` : 'Show fewer'}
          </button>
        {:else if overflow.length}
          <div class="more" style="grid-row:{laneCount + 1}; grid-column:1 / -1">
            +{overflow.length} more
          </div>
        {/if}
      {/if}
    </div>
    </div>
  </div>
{/if}

<style>
  /* `--gutter` is WeekGrid's own first column (see `secondzone.svelte`'s
     one exported width): the band's label sits over the hour ruler, and the
     two must widen together when the second clock takes a lane, or the
     chips shear off their days. */
  .band { display: grid; grid-template-columns: var(--gutter, 44px) 1fr;
          border-bottom: 1px solid var(--hairline); padding: 3px 0 6px; margin-bottom: 2px; }
  .label { font-size: 9.5px; color: var(--muted); opacity: .8; text-align: right;
           padding-right: 7px; letter-spacing: .05em; align-self: center; }
  /* No gap: a gap here is subtracted from every column, so the band's columns
     drift out of step with the grid below it — by Sunday the chips sit a chip's
     width off their days. The separation lives inside the chip instead. */
  .rows { display: grid; grid-template-columns: repeat(var(--cols), 1fr);
          width: calc(100% * var(--cols) / var(--visible)); }
  /* `WeekGrid`'s `.cols.sliding` and `.track`, for `WeekGrid`'s reasons: a
     transform, and only while sliding; `clip`, not `hidden`. */
  .rows.sliding { transform: translateX(calc((var(--pan) - var(--vis)) / var(--cols) * 100%));
                  will-change: transform; }
  /* While sliding a chip keeps its real extent, so one that begins in the
     padding has its left edge — and its label — off the pane. The label is
     pinned to the pane's edge instead: as much padding as the chip's start
     is to the left of the window's, in the chip's own columns, and none
     once the start slides into view. Pure CSS off the same `--pan` the
     track moves by, so it rides every frame with no script. `‹` marks the
     pinned state the way `cont_left` marks a chip cut by the payload. */
  .rows.sliding .chip {
    --offscreen: calc((var(--vis) - var(--pan) - var(--start)) / var(--span));
    padding-left: calc(7px + max(0px, var(--offscreen) * 100%)); }
  .rows.sliding .chip:not(.cl)::before {
    content: '‹ '; display: inline-block; overflow: hidden; vertical-align: bottom;
    width: clamp(0px, calc(var(--offscreen) * 100000px), 1.2ch); }
  .track { min-width: 0; overflow-x: clip; }

  /* A <button>, like EventBlock, rather than a <div> with a click handler
     bolted on: the role, the tab stop and Enter/Space all come for free and
     stay correct. The first three declarations exist only to undo the UA
     button styles the <div> never had — without them the chip picks up
     native chrome, a centred label and the button font. `border: 0` restores
     what a <div> starts with, so the colour spine below is unchanged rather
     than added on top of a default button border.

     EventBlock replaced its own one-sided border with an inset shadow, over
     a WKWebView artifact where corners away from the border rendered square.
     Not copied here: that spine has to go dashed for a continuing span
     (`.cl` below), and no shadow can be dashed. So this keeps the shape that
     caused the artifact, and the `AllDayBand chip corners` specs guard it —
     one zero-tolerance snapshot per corner state, at `threshold: 0`.
     Deliberately not the band's own `allday-populated.png`: that frame is
     1280x42 under `maxDiffPixelRatio: 0.01`, ~537 pixels of slack against an
     artifact worth about 3-4 per corner, so it would not notice. */
  .chip { appearance: none; -webkit-appearance: none;
          font: inherit; text-align: left; cursor: pointer;
          border: 0; border-left: 2px solid var(--cal);
          /* The same size and near the same weight as a timed block's title
             (EventBlock 11.5px/600): at 10.5px these were the faintest text
             on the grid, on the row least likely to be looked at directly
             (2026-08-17, by request). */
          font-size: 11.5px; font-weight: 500;
          border-radius: 4px; padding: 2px 7px; white-space: nowrap;
          overflow: hidden; text-overflow: ellipsis;
          margin: 0 2px 2px 0;
          background: color-mix(in srgb, var(--cal) 16%, transparent);
          color: color-mix(in srgb, var(--cal) 60%, var(--text)); }
  /* Flat edges mark a span continuing beyond this week. */
  .chip.cl { border-top-left-radius: 0; border-bottom-left-radius: 0; border-left-style: dashed; }
  .chip.cr { border-top-right-radius: 0; border-bottom-right-radius: 0; }
  .chip.keyboard { outline: 2px solid var(--accent); outline-offset: 1px; }

  .more { font: inherit; font-size: 10px; color: var(--muted); opacity: .7; padding: 2px 4px;
          background: none; border: 0; text-align: left; }
  button.more { cursor: pointer; }
  button.more:hover { color: var(--text); opacity: 1; }
  /* Expanded, the band takes what it needs and scrolls rather than pushing
     the hours off the screen: seventy all-day events is eighteen rows, and
     a calendar whose grid has been shouldered out of the window has traded
     one problem for a worse one. The label stays put beside it. */
  .band.expanded .track { max-height: 38vh; overflow-y: auto; }
</style>
