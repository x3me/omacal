<!-- ui/src/lib/DateField.svelte
     A date field with a calendar we own.

     Replaces `<input type="date">`, whose calendar is the platform's and
     **cannot be closed from the page**: WebKitGTK's popup holds an input grab,
     so an outside click never reaches us at all (#78, measured on the real
     build — clicking another field while it is open does nothing whatever).
     Escape was the only way out, because Escape is the one key the popup
     itself handles. No listener could fix that; only owning the calendar can.

     The text half keeps `yyyy-mm-dd` as its value, which is what
     `EventFormValue` carries and what every caller already reads and writes.
     Typing still works, and works unambiguously — no reader has to know
     whether 06/09 means June or September. -->
<script lang="ts">
  import { weekStartDay } from './weekstartstore.svelte';
  import { dropdown } from './dropdown';
  import {
    addDays, addMonths, columnOf, formatYmd, monthGrid, parseYmd, weekdayNames, type Ymd,
  } from './datepicker';

  let { id, label, value = $bindable(''), disabled = false, open = $bindable(false), onchange }: {
    id?: string;
    label: string;
    value?: string;
    disabled?: boolean;
    /** Bindable so the **owner** can take Escape for this layer too.
     *
     *  Not a private `$state`, and not an `escapeCloses` of its own: a child
     *  listener runs before its parent's, closes itself, and the parent's
     *  guard — reading state rather than history — then closes the panel
     *  behind it off the same keystroke. `EventForm` says so where it takes
     *  Escape, having been bitten by exactly this before; `CalendarPicker`
     *  is bindable for the same reason. */
    open?: boolean;
    onchange?: (v: string) => void;
  } = $props();
  let input: HTMLInputElement | undefined = $state();
  let field: HTMLSpanElement | undefined = $state();

  /** The month on screen, and the day the keyboard is on. Seeded from the
   *  value when there is one and from today when there is not — opening an
   *  empty field on January 1970 would be a joke at the user's expense. */
  let cursor = $state<Ymd>(today());

  function today(): Ymd {
    const n = new Date();
    return { y: n.getFullYear(), m: n.getMonth() + 1, d: n.getDate() };
  }

  const grid = $derived(monthGrid(cursor, weekStartDay()));
  const headings = $derived(weekdayNames(weekStartDay()));
  const monthLabel = $derived(new Intl.DateTimeFormat(undefined, {
    month: 'long', year: 'numeric', timeZone: 'UTC',
  }).format(new Date(Date.UTC(cursor.y, cursor.m - 1, 1))));

  function show() {
    if (disabled) return;
    cursor = parseYmd(value) ?? today();
    open = true;
  }

  function commit(d: Ymd) {
    value = formatYmd(d);
    onchange?.(value);
    open = false;
    input?.focus();
  }

  function onGridKey(e: KeyboardEvent) {
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    if (step !== undefined) {
      e.preventDefault();
      cursor = addDays(cursor, step);
      return;
    }
    if (e.key === 'PageUp' || e.key === 'PageDown') {
      e.preventDefault();
      cursor = addMonths(cursor, e.key === 'PageUp' ? -1 : 1);
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      commit(cursor);
    }
  }
</script>

<span class="datefield" bind:this={field}>
  <input
    bind:this={input}
    {id}
    type="text"
    inputmode="numeric"
    autocomplete="off"
    spellcheck="false"
    placeholder="yyyy-mm-dd"
    aria-label={label}
    {disabled}
    {value}
    oninput={(e) => { value = e.currentTarget.value; onchange?.(value); }}
    onpointerdown={() => { if (!open) show(); }}
    
  />
  <!-- **`pointerdown`, not `click`.** A press that lands on the calendar —
       a day, or the scrim — closes it on mouseup, and the popover unmounts
       while the click is still resolving. The browser then targets whatever
       is underneath, which is this input, and a `click` handler here would
       reopen the calendar the press had just dismissed. WebKit retargets
       where Chromium does not, so it failed on CI alone (2026-09-09).
       A press that began elsewhere is not a press on the field. -->
  <button
    type="button"
    class="open"
    {disabled}
    aria-label="Pick {label.toLowerCase()}"
    aria-haspopup="dialog"
    aria-expanded={open}
    onclick={() => (open ? (open = false) : show())}
  >
    <!-- Drawn rather than a glyph: `▦` was a filled square at this size and
         read as decoration, and the font that has a calendar character is
         not one we ship. `currentColor` so it follows the theme like every
         other icon here. -->
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
      <rect x="1.5" y="3" width="13" height="11.5" rx="1.5" fill="none"
            stroke="currentColor" stroke-width="1.4" />
      <path d="M1.5 6.5h13" stroke="currentColor" stroke-width="1.4" />
      <path d="M5 1.5v3M11 1.5v3" stroke="currentColor" stroke-width="1.4"
            stroke-linecap="round" />
    </svg>
  </button>

  {#if open}
    <!-- A sibling of the panel, never a wrapper — `CalendarPicker`'s shape,
         and the thing the platform's popup would never let us have. -->
    <button class="scrim" aria-label="Close {label.toLowerCase()} chooser" onclick={() => (open = false)}></button>
    <div class="cal" role="dialog" aria-label="{label} chooser" use:dropdown={field}>
      <div class="head">
        <button type="button" aria-label="Previous month" onclick={() => (cursor = addMonths(cursor, -1))}>‹</button>
        <span aria-live="polite">{monthLabel}</span>
        <button type="button" aria-label="Next month" onclick={() => (cursor = addMonths(cursor, 1))}>›</button>
      </div>
      <div class="names" aria-hidden="true">
        {#each headings as h (h)}<span>{h}</span>{/each}
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="days"
        role="grid"
        tabindex="0"
        aria-label={monthLabel}
        onkeydown={onGridKey}
      >
        {#each grid as cell (formatYmd(cell.date))}
          {@const iso = formatYmd(cell.date)}
          <button
            type="button"
            role="gridcell"
            class:outside={!cell.inMonth}
            class:on={iso === value}
            class:cursor={iso === formatYmd(cursor)}
            aria-selected={iso === value}
            aria-label={iso}
            onclick={() => commit(cell.date)}
          >{cell.date.d}</button>
        {/each}
      </div>
    </div>
  {/if}
</span>

<style>
  .datefield { position: relative; display: inline-flex; align-items: center; gap: 4px; }
  input { font: inherit; font-size: 12.5px; color: var(--text); width: 108px;
          background-color: color-mix(in srgb, var(--text) 5%, transparent);
          border: 1px solid var(--hairline); border-radius: 5px; padding: 4px 6px; }
  input:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  input:disabled, .open:disabled { opacity: .5; cursor: default; }
  /* `--text` at rest, not `--muted`: at 14px this is the only sign the field
     has a calendar behind it, and muted read as disabled against the field's
     own border (reported 2026-09-09, "not very visible"). */
  .open { display: inline-flex; align-items: center; color: var(--text);
          cursor: pointer; background: none; border: 0; padding: 2px 3px;
          border-radius: 4px; opacity: .8; }
  .open:hover:not(:disabled) { opacity: 1;
          background: color-mix(in srgb, var(--text) 10%, transparent); }
  .open:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }

  /* Covers the window beneath, so a press anywhere closes the calendar —
     the whole point of owning it. */
  .scrim { position: fixed; inset: 0; z-index: 40; background: none; border: 0;
           padding: 0; cursor: default; }
  /* Fixed and placed by `dropdown`, not absolute under the field: inside the
     Tasks pane's scrolling rows an absolute calendar wider than the pane was
     cut off at its edge (reported 2026-09-17). */
  .cal { position: fixed; top: 0; left: 0; z-index: 41;
         background: var(--surface); border: 1px solid var(--hairline);
         border-radius: 8px; padding: 8px; box-shadow: 0 6px 20px #0005; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 8px;
          font-size: 12.5px; color: var(--text); margin-bottom: 6px; }
  .head button { font: inherit; color: var(--muted); cursor: pointer; background: none;
                 border: 0; padding: 2px 6px; border-radius: 4px; }
  .head button:hover { color: var(--text); }
  .names, .days { display: grid; grid-template-columns: repeat(7, 28px); gap: 2px; }
  .names span { font-size: 10px; color: var(--muted); text-align: center; }
  .days:focus-visible { outline: 1px solid var(--accent); outline-offset: 2px; }
  .days button { font: inherit; font-size: 12px; color: var(--text); cursor: pointer;
                 background: none; border: 0; border-radius: 4px; height: 26px; }
  .days button:hover { background: color-mix(in srgb, var(--text) 8%, transparent); }
  .days button.outside { color: var(--muted); opacity: .55; }
  .days button.cursor { outline: 1px solid var(--accent); outline-offset: -1px; }
  .days button.on { background: var(--accent-fill); color: var(--on-accent, #fff); }
</style>
