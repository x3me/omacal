<!-- ui/src/lib/MonthGrid.svelte -->
<script lang="ts">
  import { rotate } from './weekstart';
  import { weekStartDay } from './weekstartstore.svelte';
  import type { MonthPayload, UiEvent } from './api';
  import type { Rect } from './position';
  import { cursorNamesEvent, type KeyboardCursor } from './keyboardnav';
  import { MONTH_GRID_TIMED_LIMIT } from './filmstrip';

  let { month, keyboardCursor = null, onopen, ondaypick, oncreate }: {
    month: MonthPayload;
    keyboardCursor?: KeyboardCursor | null;
    /** Same contract as `WeekGrid`'s: an anchor rect plus the clicked event,
     *  handed straight to `EventPopover` via `placePopover`. */
    onopen: (event: UiEvent, rect: Rect) => void;
    /** Asks the parent to switch to Day view for this day's `start_ms`. */
    ondaypick: (startMs: number) => void;
    /** A click on empty space in a day cell. A month cell has no time in it,
     *  only a date, so this hands the parent the day's own `start_ms` and lets
     *  it apply the app's default hour — unlike `WeekGrid`'s `oncreate`, which
     *  reads a real time off where in the column the click landed. */
    oncreate: (dayStartMs: number, rect: Rect) => void;
  } = $props();

  // `pack_lanes`'s own cap, mirrored here so the row-level "+N more" can be
  // placed on the track just past the last lane a bar can occupy.
  //
  // It does *not* fix the bar strip's height: `.bars` is content-sized, so a
  // row with no bars gets a 4px strip and a two-lane row gets 36px. The cells
  // now keep their own reading budget and a busier row makes the grid scroll
  // sooner rather than taking that space from its timed lines. Unlike
  // `BigYearRibbon`, Month view therefore pays only for lanes a week uses.
  const MAX_BAR_LANES = 3;
  // How many timed lines a cell shows before folding the rest into "+N more".
  // Matches `pack_lanes`'s own lane cap for bars — three is what a narrow
  // cell has room for before a title stops being legible. The value lives in
  // `filmstrip.ts` because App's keyboard cursor must share this exact limit.

  // Written Monday-first and rotated, never rewritten per setting: one
  // spelling of the seven names, and `rotate` is tested on its own.
  const MONDAY_FIRST = ['MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT', 'SUN'];
  const DOW = $derived(rotate(MONDAY_FIRST, weekStartDay()));

  // Derived from a ticking clock, never computed once — WeekGrid records
  // why: an app left running overnight kept yesterday ringed as today
  // (2026-08-19, live). The focus snap makes a wake-from-suspend right the
  // moment the user looks.
  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => { nowMs = Date.now(); }, 60_000);
    const snap = () => { nowMs = Date.now(); };
    window.addEventListener('focus', snap);
    return () => { clearInterval(id); window.removeEventListener('focus', snap); };
  });
  const todayStart = $derived.by(() => {
    const d = new Date(nowMs);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  });

  // Shared by `.bar` and `.timed`: both hand `onopen` the same
  // `{ event, rect }` shape an `EventBlock`/`AllDayBand` chip does.
  // `stopPropagation` stays belt-and-braces: `.mcell` still owns no click
  // handler of its own, and the empty-space target added below it is a
  // *sibling* button rather than an ancestor, precisely so that no click on a
  // day number, a chip or a "+N more" has to be stopped from also creating an
  // event. Handlers on the cell itself would have made every one of those a
  // propagation question.
  function openEvent(event: UiEvent, e: MouseEvent) {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopen(event, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  function pickDay(startMs: number) {
    ondaypick(startMs);
  }

  function createOn(dayStartMs: number, e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    oncreate(dayStartMs, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  function isBarSelected(lane: { start_col: number; end_col: number }, event: UiEvent,
                         cells: { start_ms: number }[]): boolean {
    if (!keyboardCursor) return false;
    const column = cells.findIndex((cell) => cell.start_ms === keyboardCursor.dayStartMs);
    return column >= lane.start_col && column <= lane.end_col
      && cursorNamesEvent(keyboardCursor, keyboardCursor.dayStartMs, event);
  }
</script>

<div class="head">
  {#each DOW as d}<span>{d}</span>{/each}
</div>

<div class="grid">
  {#each month.rows as row}
    <div class="mrow">
      <div class="bars">
        {#each row.bars as lane (`${lane.idx}:${lane.lane}`)}
          {@const ev = row.bar_events[lane.idx]}
          {@const keyboardSelected = isBarSelected(lane, ev, row.cells)}
          <button
            class="bar"
            class:cl={lane.cont_left}
            class:cr={lane.cont_right}
            class:keyboard={keyboardSelected}
            data-kbd-selected-event={keyboardSelected ? '' : undefined}
            style="
              grid-row:{lane.lane + 1};
              grid-column:{lane.start_col + 1} / {lane.end_col + 2};
              --cal:{ev.color};
            "
            onclick={(e) => openEvent(ev, e)}
          >{lane.cont_left ? '‹ ' : ''}{ev.title}</button>
        {/each}
        {#if row.bar_overflow.length}
          <!-- A span, not a button: unlike a cell's own overflow, these
               events cover several days, so there is no single day to ask
               the parent for. -->
          <div class="more" style="grid-row:{MAX_BAR_LANES + 1}; grid-column:1 / -1">
            +{row.bar_overflow.length} more
          </div>
        {/if}
      </div>

      <div class="cells">
        {#each row.cells as cell}
          <!-- `data-start-ms` on every view's day element, uniformly: it is
               how App's paste finds the day under the mouse. -->
          <div class="mcell" class:out={!cell.in_month} class:today={cell.start_ms === todayStart}
               class:keyboard={keyboardCursor?.dayStartMs === cell.start_ms}
               data-start-ms={cell.start_ms}
               data-kbd-selected-day={keyboardCursor?.dayStartMs === cell.start_ms ? '' : undefined}>
            <!-- Empty cell space, as a real control — same reasoning as
                 `WeekGrid`'s own `.newhere`, including the `tabindex="-1"`:
                 42 invisible tab stops per month would be noise, and `n`
                 reaches the same form with no target at all. -->
            <button
              class="newhere"
              aria-label="New event"
              tabindex="-1"
              onclick={(e) => createOn(cell.start_ms, e)}
            ></button>
            <button class="num" onclick={() => pickDay(cell.start_ms)}>
              {new Date(cell.start_ms).getDate()}
            </button>
            {#each cell.timed.slice(0, MONTH_GRID_TIMED_LIMIT) as ev}
              <button
                class="timed"
                class:keyboard={keyboardCursor
                  ? cursorNamesEvent(keyboardCursor, cell.start_ms, ev)
                  : false}
                data-kbd-selected-event={keyboardCursor
                  && cursorNamesEvent(keyboardCursor, cell.start_ms, ev) ? '' : undefined}
                style="--cal:{ev.color}"
                onclick={(e) => openEvent(ev, e)}
              ><i class="dot" style="background:{ev.color}"></i>{ev.title}</button>
            {/each}
            {#if cell.timed.length > MONTH_GRID_TIMED_LIMIT}
              <button class="more" onclick={() => pickDay(cell.start_ms)}>
                +{cell.timed.length - MONTH_GRID_TIMED_LIMIT} more
              </button>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {/each}
</div>

<style>
  .head { display: grid; grid-template-columns: repeat(7, 1fr); padding-bottom: 6px; }
  .head span { text-align: center; font-size: 11px; color: var(--muted);
               letter-spacing: .05em; }

  /* `flex: 1` against App's `main`, not a guess at what surrounds it — the
     day-name row above is content-sized, so this takes the rest. The grid is
     the scrollport: rows may grow to share a tall window, but never shrink
     below the reading budget carried by `.cells`. */
  .grid { display: flex; flex-direction: column; flex: 1; min-height: 0;
          overflow-y: auto; }
  .mrow { flex: 1 0 auto; display: flex; flex-direction: column;
          border-top: 1px solid var(--hairline); }
  .mrow:first-child { border-top: 0; }

  .bars { display: grid; grid-template-columns: repeat(7, 1fr);
          grid-auto-rows: 15px; gap: 2px 0; padding: 2px 0; }

  .bar { appearance: none; -webkit-appearance: none; font: inherit;
         text-align: left; cursor: pointer; border: 0; border-left: 2px solid var(--cal);
         font-size: 10.5px; border-radius: var(--event-chip-radius, 4px); padding: 1px 6px; white-space: nowrap;
         overflow: hidden; text-overflow: ellipsis; margin: 0 2px;
         /* Keep the former 16% visual tint in an opaque base. The shared
            preference supplies all actual alpha once startup applies it. */
         --event-fill: color-mix(in srgb, var(--cal) 16%, var(--bg));
         background: color-mix(in srgb, var(--cal) 16%, transparent);
         color: color-mix(in srgb, var(--cal) 60%, var(--text)); }
  :global(:root[data-event-transparency]) .bar {
    background: color-mix(
      in srgb,
      var(--event-fill) var(--event-fill-opacity),
      transparent
    );
  }
  .bar.cl { border-top-left-radius: 0; border-bottom-left-radius: 0; border-left-style: dashed; }
  .bar.cr { border-top-right-radius: 0; border-bottom-right-radius: 0; }
  .bar.keyboard { outline: 2px solid var(--accent); outline-offset: 1px; }

  /* Date + three timed lines + the cell's `+N more`, including their gaps and
     padding. A row with all-day lanes adds those above this budget instead of
     stealing height from it; once six rows no longer fit, `.grid` scrolls. */
  .cells { flex: 1 0 76px; display: grid; grid-template-columns: repeat(7, 1fr);
           min-height: 76px; }

  .mcell { display: flex; flex-direction: column; gap: 1px; padding: 3px 4px;
           border-left: 1px solid var(--hairline); min-width: 0;
           overflow: hidden; position: relative; }

  /* Covers the whole cell, paints nothing, and is held *under* the cell's own
     controls by the `z-index` pair below — explicitly, not incidentally.
     Measured, with the `z-index`es taken out of all four rules: both WebKit
     and Chromium already put `.num`, `.timed` and `.more` on top, because a
     flex item paints as an atomic inline block and both engines lift that
     above an absolutely positioned sibling. That is far too fine a point to be
     resting "is the day number clickable" on, so the order is stated instead
     of inherited — 0 here, 1 on each of the three — and `MonthGrid`'s own
     "the day number still does not" spec fails the moment they invert. */
  .newhere { appearance: none; -webkit-appearance: none; position: absolute; inset: 0;
             background: none; border: 0; padding: 0; margin: 0; font: inherit;
             cursor: cell; z-index: 0; }

  .mcell:first-child { border-left: 0; }
  .mcell.today { background: var(--today-tint); border-radius: 6px; }
  .mcell.keyboard { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
                    border-radius: 6px; }
  .mcell.out .num { color: var(--muted); opacity: .6; }
  .mcell.out .timed { opacity: .55; }

  /* `position: relative` + `z-index: 1` on all three of the cell's own
     controls, so each stays above `.newhere` — see its comment. */
  .num { appearance: none; -webkit-appearance: none; font: inherit; cursor: pointer;
         border: 0; background: transparent; padding: 0; margin: 0; align-self: flex-start;
         font-size: 12px; color: var(--text); font-variant-numeric: tabular-nums;
         position: relative; z-index: 1; flex: none; }
  .mcell.today .num { color: var(--accent); font-weight: 600; }

  .timed { appearance: none; -webkit-appearance: none; font: inherit;
           display: flex; align-items: center; gap: 4px; text-align: left; cursor: pointer;
           border: 0; background: transparent; padding: 0; margin: 0;
           font-size: 10.5px; color: var(--text); white-space: nowrap; overflow: hidden;
           text-overflow: ellipsis; position: relative; z-index: 1; flex: none; }
  .dot { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .timed.keyboard { border-radius: 3px;
                    outline: 2px solid var(--accent); outline-offset: 1px; }

  .more { font: inherit; font-size: 10px; line-height: 1.2; color: var(--muted); opacity: .8;
          padding: 0; background: transparent; border: 0; text-align: left;
          position: relative; z-index: 1; flex: none; }
  /* Only the cell-level `+N more` is a button. The row-level one is a `div`
     covering several days at once, with no single day to hand the parent, so
     it does nothing when clicked — and must not invite the click. */
  button.more { cursor: pointer; }
</style>
