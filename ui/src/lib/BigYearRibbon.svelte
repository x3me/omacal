<!-- ui/src/lib/BigYearRibbon.svelte -->
<script lang="ts">
  import { isWeekendColumn } from './weekstart';
  import { weekStartDay } from './weekstartstore.svelte';
  import type { BigYearPayload, UiEvent } from './api';
  import type { Calendar } from './calendars';
  import { foregroundFor } from './ink';
  import type { Rect } from './position';

  let { ribbon, onopen, oncreate, openId = null, openStart = null, calendars = [], ontoggle = null }: {
    ribbon: BigYearPayload;
    /** The app's calendar list, which the legend under the rows is drawn
     *  from: every syncing calendar, dimmed when hidden. Not the payload's
     *  pills, and deliberately so — a calendar hidden from this legend has
     *  no pills left to be listed by, and it has to stay listed to be shown
     *  again from here. Optional so a standalone mount without one simply
     *  has no legend. */
    calendars?: Calendar[];
    /** A legend item was clicked: flip that calendar's show/hide switch.
     *  Handed up rather than written here, the way every write in the grids
     *  is — the same `selected` the sidebar's checkbox edits, owned by App. */
    ontoggle?: ((calendar: Calendar) => void) | null;
    /** Same contract as `WeekGrid`/`MonthGrid`: an anchor rect plus the
     *  clicked event, handed straight to `EventPopover` via `placePopover`. */
    onopen: (event: UiEvent, rect: Rect) => void;
    /** A click on a day in the strip. Same contract as `MonthGrid`'s: a ribbon
     *  day carries a date and no time, so the parent applies the app's default
     *  hour to it. */
    oncreate: (dayStartMs: number, rect: Rect) => void;
    /** The occurrence whose popover is open, so its segments stay lit while it
     *  is. Two primitives rather than the `UiEvent`, mirroring `App.svelte`'s
     *  own `gridSelId`/`gridSelStart` — which is exactly where these come from,
     *  since App is what opened the popover — and for the reason its comment
     *  gives: the proxy identity of an object reassigned into `$state` is not
     *  something to rest a later `===` on.
     *
     *  Optional, both of them, so a caller that does not run a popover (the
     *  standalone mounts in `tests/harness`) need not pass anything. */
    openId?: number | null;
    openStart?: number | null;
  } = $props();

  // A calendar that does not sync has nothing to show or hide, so it is not
  // offered — the sidebar disables its checkbox for the same reason.
  const legendCalendars = $derived(calendars.filter((c) => c.sync_enabled));

  // Which occurrence the cursor is on. Local, because nothing outside this
  // component has any use for it — unlike `openId`/`openStart`, which App owns
  // because App owns the popover.
  let hoverId = $state<number | null>(null);
  let hoverStart = $state<number | null>(null);

  /**
   * Whether this segment belongs to the occurrence being pointed at, or the one
   * whose popover is open.
   *
   * **The key is the occurrence — `id` *and* `start_ms` — and neither half is
   * optional.**
   *
   * Not `lane.idx`: an event running past day 28 produces one `Lane` per row it
   * touches, each landing in a *different* row's `pill_events` at a different
   * index, so segments of one occurrence do not share one.
   *
   * And not `id` alone, which is the repair that looks right and is worse:
   * every occurrence of a recurring series carries its master row's id
   * (`to_ui(src, ...)` builds each expanded occurrence from the same
   * `StoredEvent`), so hovering January's standup would light up all
   * fifty-two.
   *
   * `start_ms` is what separates them, and that it does is a property of the
   * assembler rather than a hope: `occurrences()` hands back each occurrence's
   * own interval, and `expand()` (`recur.rs`) filters by the row window without
   * ever clamping to it — so the row-segments of one occurrence carry one
   * identical start, and two occurrences of a series never do. `App.svelte`'s
   * `isGridSelected` already names an occurrence the same way, for the same
   * reason.
   */
  const isLit = (ev: UiEvent) =>
    (hoverId === ev.id && hoverStart === ev.start_ms)
    || (openId === ev.id && openStart === ev.start_ms);

  // The data cap: `pack_lanes(&segments, 28, 3)` (commands.rs) never packs a
  // fourth overlapping span into a lane, folding it into `overflow` instead.
  // This is a limit on what the assembler *packs*, not on what the strip
  // *reserves* — the reservation is `--lanes` in the stylesheet below, and it
  // is a different number on purpose — and it's what the "+N more" row's own
  // position (`grid-row:{cap + 1}`) reads off: the row just past the highest
  // lane a pill can occupy. Mirrors `MonthGrid`'s `MAX_BAR_LANES`.
  const PILL_LANE_CAP = 3;

  const MONTH_NAMES = [
    'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
  ];

  // Every row is 28 days starting on the chosen first day of the week
  // (`assemble_big_year`'s own invariant), so which weekday a column holds is
  // fixed by its index alone
  // — never by the date it happens to carry. That constancy is the entire
  // point of the 28-day row (see `every_row_puts_its_weekends_in_the_same_columns`
  // in commands.rs): reading it off the column index rather than the date
  // keeps the stripes straight even if a caller ever fed in dates that
  // disagreed with the assumption.
  const isWeekend = (col: number) => isWeekendColumn(col, weekStartDay());

  function openPill(event: UiEvent, e: MouseEvent) {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopen(event, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  function createOn(dayStartMs: number, e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    oncreate(dayStartMs, { top: r.top, left: r.left, width: r.width, height: r.height });
  }
</script>

<div class="ribbon" data-year={ribbon.year}>
  <div class="rows">
    {#each ribbon.rows as row, r (r)}
      <div class="rrow">
        <!-- The day cells come first and the pills second, and the order is
             load-bearing rather than editorial. Both occupy the same grid cell
             (`.rrow` is a one-cell grid), and grid items paint in DOM order, so
             this is what puts an event *on* its days rather than under them.
             Swap them and the weekend shading paints over every pill. -->
        <div class="rdays">
          {#each row.days as d, c (c)}
            {@const date = new Date(d.start_ms)}
            <div
              class="rday"
              class:wknd={isWeekend(c)}
              class:out={!d.in_year}
              class:unsynced={d.unsynced}
              data-start-ms={d.start_ms}
            >
              <!-- Empty grid space, same contract and same `tabindex="-1"` as
                   `WeekGrid`'s and `MonthGrid`'s: 392 invisible tab stops is
                   not a keyboard route to anything, and `n` already is one. -->
              <button
                class="newhere"
                aria-label="New event"
                tabindex="-1"
                onclick={(e) => createOn(d.start_ms, e)}
              ></button>
              <!-- The date sits at the top of its box now, not in the middle of
                   it, because the box is the whole row and the rest of it is
                   where the events go. The month chip is inline beside the
                   number rather than absolutely positioned above it: it used to
                   float over a 15px strip that nothing else occupied, and that
                   space now belongs to lane 0. -->
              <span class="dlabel">
                {#if date.getDate() === 1}
                  <span class="mchip">{MONTH_NAMES[date.getMonth()]}</span>
                {/if}
                <span class="dnum">{date.getDate()}</span>
              </span>
            </div>
          {/each}
        </div>

        <!-- No `--lanes` here. The reservation is a question about how much
             height the window has, which the stylesheet can answer and this
             cannot; an inline custom property would also beat the media query
             that answers it. See `.pills` below. -->
        <div class="pills">
          {#each row.pills as lane (`${lane.idx}:${lane.lane}`)}
            {@const ev = row.pill_events[lane.idx]}
            <!-- The title goes on the head segment only. An event running past
                 day 28 produces one `Lane` per row it touches, and each used to
                 carry the full title: a three-row conference printed its name
                 three times, once per row, which is noise rather than
                 information. Both references print it once at the start and
                 leave the continuation bars bare — the colour and the unbroken
                 run are what say it is the same event.

                 So `aria-label`, explicitly, rather than leaning on `title`.
                 A continuation's only content was the title, and a `<button>`
                 with no content is a button with no name. `title` is *usually*
                 promoted to the accessible name, but it is a fallback and the
                 two engines do not have to agree on it; a control's name is
                 not a thing to leave to "usually".

                 The `‹` marker went with the title. It said "this started
                 earlier", which was worth saying beside a title and is not
                 worth saying alone: a lone chevron on a solid fill reads as a
                 glyph, not a hint. `.pill.cl`'s squared corner and contrasting
                 left edge were built for exactly this and now carry it. -->
            <button
              class="pill"
              class:cont={lane.cont_left || lane.cont_right}
              class:cl={lane.cont_left}
              class:cr={lane.cont_right}
              class:lit={isLit(ev)}
              onmouseenter={() => { hoverId = ev.id; hoverStart = ev.start_ms; }}
              onmouseleave={() => { hoverId = null; hoverStart = null; }}
              style="
                grid-row:{lane.lane + 1};
                grid-column:{lane.start_col + 1} / {lane.end_col + 2};
                --cal:{ev.color};
                --ink:{foregroundFor(ev.color)};
              "
              title={ev.title}
              aria-label={ev.title}
              onclick={(e) => openPill(ev, e)}
            >{lane.cont_left ? '' : ev.title}</button>
          {/each}
          {#if row.overflow.length}
            <!-- A span, not a button: like `MonthRow.bar_overflow`, these
                 cover several days, so there is no single day to hand the
                 parent. -->
            <div class="more" style="grid-row:{PILL_LANE_CAP + 1}; grid-column:1 / -1">
              +{row.overflow.length} more
            </div>
          {/if}
        </div>
      </div>
    {/each}
  </div>

  {#if legendCalendars.length}
    <!-- The key, and the filter bar (2026-09-03): each entry is the same
         show/hide switch as the sidebar's checkbox, so the strip you are
         looking at is where you thin the year down. Keyed by `id`, never by
         `summary`: two accounts subscribed to the same public calendar
         ("Holidays in Bulgaria") both report that identical summary, and a
         duplicate key is not a cosmetic problem in Svelte 5 —
         `each_key_duplicate` throws, and the whole ribbon fails to render,
         not just the legend. -->
    <div class="legend" role="group" aria-label="Calendars">
      {#each legendCalendars as c (c.id)}
        <button type="button" class="item" class:off={!c.selected}
                aria-pressed={c.selected}
                title={c.selected ? `Hide ${c.summary}` : `Show ${c.summary}`}
                onclick={() => ontoggle?.(c)}>
          <i class="dot" style="--c:{c.color_hex ?? 'var(--muted)'}"></i>
          <span>{c.summary}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* `min-height: 0` is load-bearing here and nowhere else in this file.
     Unlike `.rows` below, this box has no `overflow` of its own, so it keeps
     the automatic minimum size a flex item has by default and refuses to
     shrink below its fourteen rows. Measured on a 400px-tall window with it
     removed: `.ribbon` comes out 539px against the 337px available, overflows
     `main`, and carries the last rows off the bottom of the screen with no
     scroller to reach them — which is precisely the short-viewport case Plan 4
     settled. */
  .ribbon { display: flex; flex-direction: column; gap: 8px; padding: 4px;
            flex: 1; min-height: 0; }

  /* `flex: 1` inside `.ribbon`, which is itself `flex: 1` inside App's `main`:
     the legend below takes the height it needs and the rows take the rest.
     What this replaced was `calc(100vh - 190px)`, and the 190 was the reported
     defect — the real chrome is nothing like that, so a 1080-tall window was
     left with 123px of nothing under the fourteenth row.

     No `min-height: 0` on this one: `overflow-y: auto` already means it has no
     automatic minimum size, so it shrinks and scrolls without one — measured,
     adding it changes no geometry at 400px or at 720p. `.ribbon` above does
     need its own, for the reason stated there. And note this is the *scroll
     container*: `.rrow` below keeps its automatic minimum deliberately, which
     is the opposite arrangement and the reason a short window scrolls the
     rows rather than flattening them. */
  .rows { display: flex; flex-direction: column; flex: 1; overflow-y: auto; }
  /* One cell, two layers. `.rdays` and `.pills` are both placed at `1 / 1`, so
     the day boxes and the events drawn in them are the same box rather than two
     stacked bands — which is the whole of this change. The ribbon used to be a
     flex column of a pill strip above a day strip, and read as two separate
     things: a row of floating bars, and under it a thin row of numbers.

     A grid overlay rather than `position: absolute` on `.pills`, and the
     difference is not cosmetic. An absolutely positioned strip contributes
     nothing to its parent's height, which would silently drop the property that
     a row needing more lanes than the reservation *grows* to fit them — three
     packed lanes against a reservation of two would simply be drawn over the
     next row's dates. As grid items both layers still size the row, so that
     behaviour survives with no explicit minimum to compute and no lane count to
     thread through the markup. `a row that needs a third lane grows to fit it`
     is the spec that holds this.

     Still no `min-height: 0`, and now for a slightly different reason than
     before: the automatic minimum is what makes the paragraph above true. It
     resolves to the taller layer's own minimum — the date band plus however
     many lanes the row actually uses — so a short viewport scrolls `.rows`
     (which is what its `overflow-y` is for) rather than crushing a row into its
     neighbour.

     The three geometry numbers live here, on the row, because both layers need
     them: `.dlabel` sizes itself by `--date-band` and `.pills` clears it by
     exactly the same value, so the date and lane 0 cannot drift apart. */
  .rrow { flex: 1; display: grid; border-top: 1px solid var(--hairline);
          --date-band: 14px; --lane-h: 14px; --lane-gap: 1px; }
  .rrow:first-child { border-top: 0; }
  .rdays, .pills { grid-area: 1 / 1; }

  /* `grid-template-rows`, not `grid-auto-rows`, for the reserved lanes
     themselves: explicit tracks exist whether or not a pill occupies them, so
     the strip is `--lanes` tall in every row that stays within budget, and a
     quiet row — the common case — does not collapse to a different height than
     its neighbours. `grid-auto-rows` is the same 10px for whatever goes beyond
     that: a lane past the reservation, and the one implicit track the "+N more"
     row adds when a row overflows. Both grow the row rather than being clipped;
     same mechanism, same size.

     `--lanes` is the *reservation*, and it is deliberately not `PILL_LANE_CAP`.
     The cap is what `pack_lanes` will pack (three); the reservation is what the
     layout pays for up front. Where they differ, a row that genuinely needs its
     third lane still gets it and simply grows past the budget — nothing is
     dropped.

     How many to reserve is a question about the window, so it is asked here and
     not in the script: an inline `style="--lanes:…"` is what this used to be,
     and a declaration in a `style` attribute beats any stylesheet rule, media
     query included. Nothing needs measuring at runtime for it either. A view's
     box is the viewport minus `App`'s chrome, a fixed 63px
     (`tests/harness/viewbox.ts`), so a viewport-height query *is* a query about
     the height the ribbon has — which makes a `ResizeObserver`, with a live
     subscription and a teardown to get right, the more expensive way to learn
     the same fact.

     The threshold is measured, in both engines, and they agree to the pixel.
     Re-measured for the overlay: a row is no longer a day strip stacked on a
     pill strip, it is one box holding a date band and the lanes, so every
     number below moved.

     Fourteen rows reserving three lanes need 825px of `.rows`. A row is 58px —
     14px of date band plus a 44px lane strip (3 × 14px and two 1px gaps) — with
     a 1px top border on all but the first: 13 × 59 + 58 = 825. Around that sit
     the legend at 19px, this element's 8px of gap and padding, and App's 64px:
     64 + 4 + 825 + 8 + 19 + 4 = 924. Measured by stepping the viewport a pixel
     at a time against `.rows`'s own overflow: 924 is the first height that
     fits, in both engines, and the arithmetic lands on the same number. Two
     lanes fit from 714px, and at 720p three lanes overflow by 206px — which is
     why the reservation below 924 is still two. (Every number in this chain
     moved up by one on 2026-08-20, when a font update grew the header from
     63px to 64px — the chrome term is the only term that changed.)

     The 14px lanes are the 2026-08-14 readability pass (11px before it, and
     8px pill text read as decoration). What that spent: §4's one-screen
     promise at 720p now survives on 4px — two-lane rows need 615px of the
     619 the default viewport leaves `.rows` — so any future lane growth must
     re-check "all fourteen rows fit on one screen" before anything else.

     Do not read the 19px legend as a constant. It is one line of legend, which
     is what a handful of calendars comes to; a wrapped second line costs about
     as much again, and at 924 exactly that scrolls. No fixed threshold can
     prevent that — a third line would cost the same again — so the measured
     one-line height is what this uses, and a taller legend does what it
     already does today: `.rows` scrolls, which is what its `overflow-y` is
     for. */
  .pills { display: grid; grid-template-columns: repeat(28, 1fr);
           --lanes: 2;
           grid-template-rows: repeat(var(--lanes), var(--lane-h));
           grid-auto-rows: var(--lane-h); gap: var(--lane-gap) 0;
           /* Clears the date at the top of the cell, by exactly the height the
              date is given. Lane 0 starts where the label stops. */
           margin-top: var(--date-band);
           /* Not `stretch`. The box is then the lane strip and nothing else,
              which keeps `.pills`'s measured height meaningful — it is the
              reservation, or the lanes actually used, and a spec can read it.
              Stretched, it would always be the row height and any spec that
              measured it would have stopped distinguishing anything. */
           align-self: start;
           /* Paints over the day cells and their shading. `z-index` applies to
              a grid item whether or not it is positioned, which is what makes
              this work without `position: relative` — and it is needed, because
              `.newhere` below *is* positioned, and a positioned descendant
              otherwise paints above a non-positioned sibling regardless of DOM
              order. Without it the click targets sit on top of every pill. */
           z-index: 2;
           /* The trap. This strip covers the middle of all 28 day boxes, and a
              grid container receives pointer events across its whole box — not
              only where a pill happens to be drawn. Left alone it swallows
              every click meant for `.newhere` underneath it, including in the
              empty tracks of an otherwise busy row, so "click an empty day"
              silently stops working on exactly the rows that have events on
              them. `a day under the pill strip is still clickable` is the spec,
              and it clicks a day that genuinely has the strip over it — a day
              in a row with no pills at all would pass either way. */
           pointer-events: none; }
  .pill, .more { pointer-events: auto; }
  @media (min-height: 924px) { .pills { --lanes: 3; } }

  /* Solid, not a 16% wash. Fourteen rows of pastel smudge is what the ribbon
     read as; a bar of the calendar's actual colour is what makes the year
     legible at a glance, and it is what both references do.

     `--ink` is the pill's own foreground, decided per event from `--cal`'s
     relative luminance in the script above — a fixed `color:` cannot serve a
     dark blue and a pale yellow at once, and after this change both are solid
     rather than washed towards the theme.

     The `border-left: 2px solid var(--cal)` accent that used to be here is
     gone. Against a 16% wash it was the one place the full-strength colour
     appeared, so it carried the calendar's identity; against a fill that *is*
     that colour it is the same colour on the same colour, and paints nothing
     at all. Keeping it would be keeping the box-model cost of a rule nobody
     can see. */
  /* Fully rounded ends, not the old 3px. Both references use a capsule, and on
     a bar this short the radius is most of what says "this is one object" — at
     3px on an 11px lane the corners read as a rectangle with the edges knocked
     off. `999px` rather than `50%`, which on a wide bar would give an ellipse.
     The squared continuation corners below still win, since they come after.

     `line-height: var(--lane-h)` centres the label without a flex context,
     which would fight `text-overflow: ellipsis`. The pill stretches to the
     track, so its content box is the lane height and one line box fills it. */
  .pill { appearance: none; -webkit-appearance: none; font: inherit;
          text-align: left; cursor: pointer; border: 0;
          font-size: 10.5px; line-height: var(--lane-h);
          border-radius: var(--event-pill-radius, 999px); padding: 0 5px; white-space: nowrap;
          overflow: hidden; text-overflow: ellipsis; margin: 0 1px;
          --event-fill: var(--cal); background: var(--cal); color: var(--ink); }
  :global(:root[data-event-transparency]) .pill {
    background: color-mix(
      in srgb,
      var(--event-fill) var(--event-fill-opacity),
      transparent
    );
    /* `--ink` is chosen for a solid calendar-colour fill. Once that fill is
       translucent, theme text mixed toward the calendar colour is the stable
       contrast target instead. */
    color: color-mix(in srgb, var(--cal) 60%, var(--text));
  }
  :global(:root[data-event-transparency]) .pill.cl { border-left-color: currentColor; }
  :global(:root[data-event-transparency]) .pill.lit {
    box-shadow: inset 0 0 0 1px currentColor;
  }
  /* The continuation edge, in `--ink` rather than `--cal` for the reason the
     plain accent above was dropped: `border-left-style: dashed` in the fill's
     own colour is invisible on a solid fill — the dashes and the gaps are the
     same colour, so "this started earlier" stops being said at all. `--ink` is
     the one colour on the pill guaranteed to contrast with the fill, since
     that is the entire property `foregroundFor` picks it for. */
  .pill.cl { border-top-left-radius: 0; border-bottom-left-radius: 0;
             border-left: 2px dashed var(--ink); }
  .pill.cr { border-top-right-radius: 0; border-bottom-right-radius: 0; }

  /* Lit: hovered, or the one whose popover is open. A trip runs across several
     28-day rows and, with a dozen teal bars stacked, finding where one ends is
     genuinely hard — so the whole occurrence lights at once, every row of it.

     **Nothing here may move anything.** These pills sit under the cursor by
     definition, and a highlight that changed a box would make the ribbon jump
     out from under it — and, worse, could re-enter the pill and flicker.
     `filter` and `box-shadow` are the two that paint outside the box model
     without taking part in layout; neither reserves space, and `a highlight
     changes nothing about where a pill sits` pins that.

     `box-shadow` rather than `outline`: it follows `border-radius` on every
     engine, which matters on a capsule, and being inset means the ring never
     bleeds onto the day either side. `--ink` is the pill's own foreground, so
     the ring is guaranteed to contrast with the fill whatever Google sent —
     the same property `foregroundFor` picks it for. */
  .pill.lit { filter: brightness(1.35);
              box-shadow: inset 0 0 0 1px var(--ink); }

  .more { font-size: 10px; color: var(--muted); opacity: .8; }

  /* The full-height box now, not a strip under the pills: it stretches to the
     grid cell it shares with `.pills`, so a day cell is as tall as its row and
     the events are drawn inside it. The weekend shading and the `.unsynced`
     hatch are still painted here, which is what puts them *behind* the pills
     rather than beside them. */
  .rdays { display: grid; grid-template-columns: repeat(28, 1fr); }

  /* `flex-start`, not `center`: the date belongs at the top of its box because
     the rest of the box is where the events go. */
  .rday { display: flex; flex-direction: column; align-items: center;
          justify-content: flex-start;
          min-width: 0; position: relative; font-size: 10.5px; color: var(--text);
          font-variant-numeric: tabular-nums; border-left: 1px solid var(--hairline); }
  .rday:first-child { border-left: 0; }
  .rday.wknd { background: color-mix(in srgb, var(--muted) 8%, transparent); }
  .rday.out { color: var(--muted); opacity: .55; }
  .rday.unsynced {
    background-image: repeating-linear-gradient(
      45deg, var(--hairline), var(--hairline) 1px, transparent 1px, transparent 4px
    );
  }

  /* Covers the day, paints nothing, and is held under the day's own two labels
     by the `z-index` pair below — stated rather than inherited, for the reason
     `MonthGrid`'s own `.newhere` comment sets out at length: both engines
     already paint a flex item above an absolutely positioned sibling, and that
     is not a fact to rest a hit target on. */
  .newhere { appearance: none; -webkit-appearance: none; position: absolute; inset: 0;
             background: none; border: 0; padding: 0; margin: 0; font: inherit;
             cursor: cell; z-index: 0; }

  /* The date band: the one strip of the day box that pills do not cover, and
     `.pills`'s `margin-top` is the same variable, so the two cannot drift.
     `z-index: 1` keeps it above `.newhere` (0) and below `.pills` (2) — which
     costs nothing, since the band is exactly the height the pills clear. */
  .dlabel { display: flex; align-items: center; gap: 2px;
            height: var(--date-band); line-height: var(--date-band);
            position: relative; z-index: 1; }
  .mchip { font-size: 8px; font-weight: 600;
           color: var(--accent); letter-spacing: .02em; }

  .legend { display: flex; flex-wrap: wrap; gap: 10px 16px; padding: 4px; }
  /* A button now, drawn to the box the plain item had: the 2px/4px padding
     is taken back by the negative margin, so the legend's one-line height —
     which the ribbon's row arithmetic above measures, not assumes — is what
     it was before the items could be pressed. */
  .legend .item { display: flex; align-items: center; gap: 5px; font: inherit; font-size: 10px;
                  color: var(--muted); background: none; border: 0; cursor: pointer;
                  padding: 2px 4px; margin: -2px -4px; border-radius: 4px; }
  .legend .item:hover { color: var(--text); background: color-mix(in srgb, var(--text) 8%, transparent); }
  .legend .dot { width: 7px; height: 7px; border-radius: 50%; flex: none;
                 background: var(--c); box-shadow: inset 0 0 0 1.5px var(--c); }
  /* Hidden: hollow dot, struck name, dimmed — three cues, because the dot
     alone is 7px and the colour alone may be the calendar's own grey. */
  .legend .item.off { opacity: .55; }
  .legend .item.off .dot { background: transparent; }
  .legend .item.off span { text-decoration: line-through; }
</style>
