<!-- ui/src/lib/Filmstrip.svelte -->
<script lang="ts">
  import { clockFormat } from './clock.svelte';
  import { formatClock } from './timefmt';
  import { openConference, type UiEvent } from './api';
  import type { Rect } from './position';
  import { locationLabel, meetingUrl } from './location';
  import { temperatureUnit } from './tempunit.svelte';
  import { formatTemp } from './temperature';
  import WeatherGlyph from './WeatherGlyph.svelte';
  import { dateKey, type DayWeather } from './weather';
  import type { ListDay } from './filmstrip';
  import { cursorNamesEvent, type KeyboardCursor } from './keyboardnav';

  let { days, weather = null, revealNowRequest = 0, keyboardCursor = null, onopen }: {
    /** Already grouped, ordered and emptied of blank days by `filmstrip.ts`.
     *  This component draws a list; it does not decide what is in one. */
    days: ListDay[];
    /** The forecast by ISO date, or null for none — same contract as
     *  `WeekGrid`'s: a heading with no sky is just a heading. */
    weather?: Map<string, DayWeather> | null;
    /** The explicit Today request counter shared with the clock grid. */
    revealNowRequest?: number;
    keyboardCursor?: KeyboardCursor | null;
    /** Same contract as `MonthGrid`'s and `BigYearRibbon`'s: the clicked event
     *  plus an anchor rect, handed straight up to `App.openGridEvent` and so to
     *  `openOccurrence`.
     *
     *  **The one way to reach an event's detail** (spec §6). Deliberately not a
     *  popover of this component's own, the way `WeekGrid` owns one: a second
     *  path would be a second set of guards to keep in step, and the popover
     *  already owns every one it has. */
    onopen: (event: UiEvent, rect: Rect) => void;
  } = $props();

  const hhmm = (ms: number) => formatClock(ms, clockFormat());

  /** The day a section is about — the same fields, in the same order, as
   *  `EventPopover`'s own `DAY_FORMAT`, so a row and the popover it opens name
   *  the day the same way. No year: a list is one period long, and a year
   *  repeated down forty rows is noise. */
  const dateLabel = (ms: number) =>
    new Date(ms).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });

  /** `getBoundingClientRect()` for the reason `EventBlock` and `AllDayBand` both
   *  use it: the popover places itself against the viewport, and a row's own
   *  position in a scrolling list says nothing about where it landed on screen. */
  function open(event: UiEvent, e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopen(event, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  /** The joinable link, from the same two places in the same order as the
   *  popover's own derivation — so a row offering Join and the popover it
   *  opens can never disagree about whether there is a meeting. */
  const joinable = (ev: UiEvent) => ev.conference ?? meetingUrl(ev.location);

  // The ticking clock behind the now marker — `WeekGrid`'s exact pattern,
  // interval and focus-snap included, and for its reasons: a list left open
  // overnight or across a suspend must not keep drawing "now" where
  // yesterday's afternoon was.
  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => { nowMs = Date.now(); }, 60_000);
    const snap = () => { nowMs = Date.now(); };
    window.addEventListener('focus', snap);
    return () => { clearInterval(id); window.removeEventListener('focus', snap); };
  });

  /** Where today's now marker sits in a day's rows: the index of the first
   *  timed event still ahead of now — all-day rows sort first and are days,
   *  not instants, so the marker never lands among them — or one past the
   *  end when everything has started. `-1` for any day that is not today,
   *  which the template reads as "no marker in this section".
   *
   *  A `ListDay` carries only its midnight, so "today" is `startMs`'s
   *  24-hour window; DST puts the boundary an hour out twice a year, which
   *  for a marker between rows is a miss of nothing. */
  function markerIndex(d: ListDay, now: number): number {
    if (now < d.startMs || now >= d.startMs + 24 * 3_600_000) return -1;
    const i = d.events.findIndex((ev) => !ev.is_all_day && ev.start_ms > now);
    return i === -1 ? d.events.length : i;
  }

  let stripEl: HTMLDivElement | undefined = $state();
  let handledRevealNowRequest: number | null = null;
  const NOW_VIEWPORT_FRACTION = 0.45;

  // A list has no hour geometry to calculate against, so centre the rendered
  // NOW row itself. As in WeekGrid, a Today request waits for the newly-loaded
  // period rather than being consumed by the old one, and repeated requests
  // remain meaningful while the anchor already names today.
  $effect(() => {
    if (!stripEl) return;
    const request = revealNowRequest;
    if (handledRevealNowRequest === null) {
      handledRevealNowRequest = request;
      return;
    }
    const revealRequested = request !== handledRevealNowRequest;
    if (!revealRequested) return;
    const today = days.find(
      (d) => nowMs >= d.startMs && nowMs < d.startMs + 24 * 3_600_000,
    );
    if (!today) return;
    const el = stripEl;
    requestAnimationFrame(() => {
      const marker = el.querySelector<HTMLElement>('.nowrow');
      if (!marker) return;
      const viewport = el.getBoundingClientRect();
      const markerTop = marker.getBoundingClientRect().top - viewport.top + el.scrollTop;
      el.scrollTop = Math.max(
        0,
        markerTop - el.clientHeight * NOW_VIEWPORT_FRACTION,
      );
      if (revealRequested) handledRevealNowRequest = request;
    });
  });
</script>

<!-- **No drag handlers anywhere below, and that is the feature** (spec §6): a
     list has no geometry to drop onto, so they are absent rather than disabled.
     Creating still works through `n` and through the form. -->
<div class="strip" bind:this={stripEl}>
  {#if days.length === 0}
    <!-- Spec §3: a period with nothing in it says so, plainly, rather than
         rendering as blank. Empty days being skipped is exactly what makes an
         empty period indistinguishable from a broken view without this. -->
    <p class="none">Nothing scheduled.</p>
  {:else}
    {#each days as d (d.startMs)}
      {@const marker = markerIndex(d, nowMs)}
      <section class="sday" class:keyboard={keyboardCursor?.dayStartMs === d.startMs}
               data-start-ms={d.startMs}
               data-kbd-selected-day={keyboardCursor?.dayStartMs === d.startMs ? '' : undefined}>
        <h2 class="sdate">
          {dateLabel(d.startMs)}
          {#if weather?.get(dateKey(d.startMs))}
            {@const wx = weather.get(dateKey(d.startMs))!}
            <span class="wx">
              <WeatherGlyph bucket={wx.bucket} size={12} />{formatTemp(wx.tmax, temperatureUnit())}°
            </span>
          {/if}
        </h2>
        <ul>
          {#each d.events as ev, i}
            {#if i === marker}
              <!-- The current instant, drawn between the row that has started
                   and the row still to come — the one orientation a grid gives
                   for free and a list otherwise loses. Time in the `.when`
                   column so it lines up with every other time on screen. -->
              <li class="nowrow" aria-hidden="true">
                <em class="when">{hhmm(nowMs)}</em>
                <span class="nowline"></span>
              </li>
            {/if}
            <li class="srow-li" style="--cal:{ev.color}">
              <!-- `--cal` and nothing else, exactly as the grid's own chips
                   declare it (spec §5). A calendar recoloured in settings is
                   recoloured here for free, because the override has already
                   landed in `ev.color` server-side rather than being applied at
                   render. No fill is used — a 2px spine and the theme's own
                   text — so `ink.ts` has nothing to decide here; give a row a
                   filled background and it does. -->
              <!-- No `title=` anywhere in this list, per `EventBlock`'s
                   doctrine: the attribute renders as the engine's native
                   tooltip, which no stylesheet can reach — and on a
                   translucent compositor it also ghosts, leaving faint
                   seams where tooltips popped as the pointer crossed the
                   rows. The row's own text already says everything the
                   tooltip did; the popover is one click away for the rest. -->
              <button
                class="srow"
                class:allday={ev.is_all_day}
                class:nobodycoming={ev.all_guests_declined}
                title={ev.all_guests_declined ? 'Everyone declined' : undefined}
                class:keyboard={keyboardCursor
                  ? cursorNamesEvent(keyboardCursor, d.startMs, ev)
                  : false}
                data-kbd-selected-event={keyboardCursor
                  && cursorNamesEvent(keyboardCursor, d.startMs, ev) ? '' : undefined}
                onclick={(e) => open(ev, e)}
              >
                <em class="when">
                  {ev.is_all_day ? 'All day' : `${hhmm(ev.start_ms)}–${hhmm(ev.end_ms)}`}
                </em>
                <b>{ev.title}</b>
                {#if locationLabel(ev.location)}
                  <!-- Right beside the title, not across the row: the second
                       thing anyone looks for should not be a screen-width
                       away from the first. -->
                  <em class="where">{locationLabel(ev.location)}</em>
                {/if}
                {#if ev.recurring}
                  <!-- One glyph, muted: "this row repeats" is orientation,
                       not news. -->
                  <span class="meta rep">
                    <svg viewBox="0 0 16 16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"
                      d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 1.8v2.7h-2.7"/></svg>
                  </span>
                {/if}
                {#if ev.attendees > 1}
                  <!-- Only when there is anybody besides the user to count:
                       "1" on every solo event is a column of noise. The count
                       includes the organizer, matching the widget's feed. -->
                  <span class="meta who">
                    <svg viewBox="0 0 16 16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"
                      d="M5.5 7.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Zm-4 6c0-2.2 1.8-4 4-4s4 1.8 4 4M11 7.3a2.3 2.3 0 1 0-1.6-4M11.6 9.6c1.7.5 2.9 2 2.9 3.9"/></svg>{ev.attendees}
                  </span>
                {/if}
              </button>
              {#if joinable(ev)}
                <!-- A sibling of the row button, never inside it — nested
                     interactive elements are where keyboards and readers go
                     to die. It still *reads* as part of the left cluster:
                     the row button is content-sized (see `.srow`), so this
                     sits directly after the meta rather than a screen-width
                     away. The camera is the widget's own glyph for the same
                     fact. Backend-side by id, like the popover's Join and
                     for its reasons (`api.openConference`). -->
                <button class="join" aria-label="Join video call"
                        onclick={() => void openConference(ev.id)}>
                  <svg viewBox="0 0 16 16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"
                    d="M2 4.75h7.5a.75.75 0 0 1 .75.75v5a.75.75 0 0 1-.75.75H2a.75.75 0 0 1-.75-.75v-5A.75.75 0 0 1 2 4.75Zm8.25 2.4 4.5-2.4v6.5l-4.5-2.4"/></svg>
                </button>
              {/if}
            </li>
          {/each}
          {#if marker === d.events.length}
            <li class="nowrow" aria-hidden="true">
              <em class="when">{hhmm(nowMs)}</em>
              <span class="nowline"></span>
            </li>
          {/if}
        </ul>
      </section>
    {/each}
  {/if}
</div>

<style>
  /* `flex: 1` against App's `main`, the same contract every other view has —
     see `WeekGrid`'s `.body` for the whole of this app's opinion about height.
     `overflow-y: auto` also removes the need for a `min-height: 0` beside it: a
     flex item whose overflow is not `visible` has no automatic minimum size. */
  .strip { flex: 1; overflow-y: auto; }

  .sday { border-top: 1px solid var(--hairline); padding: 6px 0 8px; }
  .sday:first-child { border-top: 0; }
  .sday.keyboard { box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent) 55%, transparent);
                   padding-left: 6px; }

  /* Sticky, so the day a row belongs to is still named after scrolling past its
     heading — the one thing a list loses that a grid gives for free. */
  .sdate { position: sticky; top: 0; z-index: 1; margin: 0 0 4px;
           font-size: 11px; font-weight: 600; color: var(--muted);
           letter-spacing: .05em; text-transform: uppercase;
           background: var(--calendar-canvas, var(--bg)); padding: 2px 0;
           display: flex; align-items: center; gap: 8px; }
  /* The sky beside the day it belongs to, in the heading's own voice. */
  .sdate .wx { display: inline-flex; align-items: center; gap: 3px;
               font-weight: 500; letter-spacing: 0;
               font-variant-numeric: tabular-nums; }

  ul { list-style: none; margin: 0; padding: 0; }

  /* The row is a flex pair — the opening button, then Join when there is one —
     so Join can be a real control without nesting inside another one. The
     hover wash lives on the li so it still sweeps the full row width; the
     button itself is content-sized, which is what keeps Join in the left
     cluster instead of flushed to the far edge by a stretching sibling. */
  .srow-li { display: flex; align-items: baseline;
             border-radius: var(--event-chip-radius, 4px); }
  .srow-li:hover, .srow-li:focus-within {
    background: color-mix(in srgb, var(--text) 6%, transparent); }

  .srow { appearance: none; -webkit-appearance: none; font: inherit;
          display: flex; align-items: baseline; gap: 10px;
          flex: 0 1 auto; min-width: 0;
          text-align: left; cursor: pointer; border: 0;
          /* The colour spine is an inset shadow, not a `border-left` —
             EventBlock's fix (89bcb2b), which this row needed too. Give a
             rounded element a border on one side only and WebKit derives the
             whole corner geometry from it: WebKitGTK strokes the *entire*
             rounded path in that colour at hairline width, and on a row with
             no fill that stroke sits naked on the background — the
             "semi-visible border" wrapping every list row, which outlived
             both the tooltip purge (06c84bb) and the user-select fix
             (8b85db8) because neither was the painter. An inset shadow
             follows border-radius exactly and paints only the 2px it names.
             Left padding grows by the 2px the border occupied, so nothing
             on the row moves. */
          box-shadow: inset 2px 0 0 0 var(--cal); background: none;
          color: var(--text); border-radius: var(--event-chip-radius, 4px);
          padding: 4px 8px 4px 10px; }
  .srow.keyboard { outline: 2px solid var(--accent); outline-offset: 1px; }
  /* The grid's mark, in the list: same dotted strike, same meaning (see
     `EventBlock`'s `.nobodycoming`). */
  .srow.nobodycoming b { text-decoration: line-through; text-decoration-style: dotted; }

  /* Tabular figures so the times form a column the eye can run down, and a
     fixed width so a title never starts at a different x from the row above
     it. Wide enough for `09:00–09:30`; `All day` sits in the same box. */
  .when { font-style: normal; flex: none; width: 92px;
          font-size: 11.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .srow.allday .when { color: color-mix(in srgb, var(--cal) 60%, var(--muted)); }

  /* `flex: 0 1 auto`, deliberately not `1 1`: a stretching title is what used
     to shove the location to the far edge of the window, a screen-width away
     from the name it belongs to. Now the title takes what it needs and
     everything after it sits beside it. */
  .srow b { flex: 0 1 auto; min-width: 0; font-size: 12.5px; font-weight: 500;
            letter-spacing: -.01em; white-space: nowrap; overflow: hidden;
            text-overflow: ellipsis; }

  .where { font-style: normal; flex: 0 1 auto; min-width: 0; font-size: 11px;
           color: var(--muted); white-space: nowrap; overflow: hidden;
           text-overflow: ellipsis; }

  .meta { flex: none; display: inline-flex; align-items: center; gap: 3px;
          font-size: 10.5px; color: var(--muted);
          font-variant-numeric: tabular-nums; }
  .meta svg { width: 12px; height: 12px; display: block;
              /* Optical: the icons sit on the baseline row beside 11px text
                 and read high without a nudge. */
              transform: translateY(2px); }

  /* The widget's camera, not a labelled chip: one glyph in the row's own
     meta register, accent-inked so "there is a call to join" still reads at
     a scan. No background — the row's hover wash is the affordance. */
  .join { appearance: none; -webkit-appearance: none; font: inherit;
          flex: none; cursor: pointer; border: 0; border-radius: 4px;
          padding: 2px 5px; background: none; color: var(--accent);
          display: inline-flex; align-items: center; align-self: center; }
  .join svg { width: 14px; height: 14px; display: block; }
  .join:hover, .join:focus-visible {
    background: color-mix(in srgb, var(--accent) 22%, transparent); }

  /* The marker: today's current minute, as a rule the accent colour owns.
     `aria-hidden` in the template — a decoration between rows, not a stop on
     anybody's reading order. */
  .nowrow { display: flex; align-items: center; padding: 1px 8px 1px 10px; }
  .nowrow .when { color: var(--accent); font-weight: 600; font-size: 10.5px; }
  .nowline { flex: 1; height: 0; border-top: 1.5px solid var(--accent); }

  .none { font-size: 12.5px; color: var(--muted); margin: 0; padding: 10px 2px; }
</style>
