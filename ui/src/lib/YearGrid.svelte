<!-- ui/src/lib/YearGrid.svelte -->
<script lang="ts">
  import { rotate } from './weekstart';
  import { weekStartDay } from './weekstartstore.svelte';
  import type { YearPayload } from './api';

  let { year, ondaypick }: {
    year: YearPayload;
    /** Asks the parent to switch to Day view for this day's `start_ms` — the
     *  same contract `MonthGrid` already established. */
    ondaypick: (startMs: number) => void;
  } = $props();

  const MONDAY_FIRST = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];
  const DOW = $derived(rotate(MONDAY_FIRST, weekStartDay()));
  const MONTH_NAMES = [
    'January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December',
  ];

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
</script>

<div class="ygrid quiet-scroll" data-year={year.year}>
  {#each year.months as month (month.month)}
    <div class="ymonth">
      <div class="mname">{MONTH_NAMES[month.month - 1]}</div>
      <div class="dow">
        {#each DOW as d, i (i)}<span>{d}</span>{/each}
      </div>
      <div class="days">
        {#each Array.from({ length: month.lead_blanks }) as _, i (i)}
          <div class="blank"></div>
        {/each}
        {#each month.days as d (d.start_ms)}
          <button
            class="yday"
            class:dotted={d.has_all_day}
            class:today={d.start_ms === todayStart}
            class:unsynced={d.unsynced}
            data-start-ms={d.start_ms}
            onclick={() => ondaypick(d.start_ms)}
          >{d.day}</button>
        {/each}
      </div>
    </div>
  {/each}
</div>

<style>
  /* This component's only root, so `flex: 1` claims everything App's `main`
     has left below the header — replacing a `calc(100vh - 150px)` that left
     79px of a 1080-tall window unused. Twelve months scroll inside it rather
     than pushing the window, and `overflow-y` is what buys that: a flex item
     whose overflow is not `visible` has no automatic minimum size, so no
     `min-height: 0` is needed beside it. Measured at 400px and at 720p. */
  .ygrid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 20px;
           padding: 4px; overflow-y: auto; flex: 1; }

  .ymonth { display: flex; flex-direction: column; gap: 4px; }
  .mname { font-size: 12px; font-weight: 600; color: var(--text);
           letter-spacing: .02em; }

  .dow { display: grid; grid-template-columns: repeat(7, 1fr); }
  .dow span { text-align: center; font-size: 9px; color: var(--muted); }

  .days { display: grid; grid-template-columns: repeat(7, 1fr); gap: 1px; }

  .blank { aspect-ratio: 1; }

  .yday { appearance: none; -webkit-appearance: none; font: inherit; cursor: pointer;
          border: 0; background: transparent; margin: 0; padding: 0;
          aspect-ratio: 1; display: flex; align-items: center; justify-content: center;
          position: relative; font-size: 10.5px; color: var(--text);
          font-variant-numeric: tabular-nums; border-radius: 50%; }
  .yday:hover { background: var(--surface); }

  /* A dot below the number for a day carrying at least one all-day event —
     a timed meeting never earns this, so the dot means "blocked out". */
  .yday.dotted::after {
    content: ''; position: absolute; bottom: 1px; left: 50%;
    width: 3px; height: 3px; border-radius: 50%;
    background: var(--accent); transform: translateX(-50%);
  }

  .yday.today { background: var(--accent-fill); color: var(--on-accent); font-weight: 600; }
  .yday.today.dotted::after { background: var(--bg); }

  /* Distinct from a plain empty day on the *cell* itself, not just the
     absence of a dot — a hatch, since absence is exactly what this must not
     be confused with (an unsynced day is "not fetched", not "free"). */
  .yday.unsynced {
    color: var(--muted);
    background-image: repeating-linear-gradient(
      45deg, var(--hairline), var(--hairline) 1px, transparent 1px, transparent 4px
    );
  }
  .yday.unsynced.today {
    background-image: repeating-linear-gradient(
      45deg, var(--hour-rule), var(--hour-rule) 1px, transparent 1px, transparent 4px
    ), linear-gradient(var(--accent-fill), var(--accent-fill));
    color: var(--on-accent);
  }
</style>
