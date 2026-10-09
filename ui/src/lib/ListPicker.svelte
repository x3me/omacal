<!-- ui/src/lib/ListPicker.svelte -->
<script lang="ts" module>
  /** One row of the picker: a task list, or `id: null` for no list in
   *  particular. */
  export type ListChoice = { id: number | null; name: string; color: string | null };
</script>

<script lang="ts">
  import { dropdown } from './dropdown';
  import { calendarInk } from './calendars';

  /** A task list, chosen the way the rest of the app chooses things.
   *
   *  It replaces a native `<select>`, whose list is drawn by GTK rather than
   *  by the page: bigger type, the system's blue, none of the theme's
   *  colours, and no list colour beside a name (Plamen, 2026-09-18: "lists
   *  selection is not styled"). This is `CalendarPicker`'s list with a field
   *  that shows its answer, placed by `dropdown` so a scrolling pane cannot
   *  clip it, and driven from the keyboard as a select-only combobox (ARIA's
   *  pattern): the focus stays on the field and the arrows move through the
   *  rows. */
  let { label, choices, value, disabled = false, compact = false, dotOnly = false, noDot = false, onpick }: {
    /** The control's name, which a screen reader says before its answer. */
    label: string;
    choices: ListChoice[];
    value: number | null;
    disabled?: boolean;
    /** The task editor's size, a step down from the add row's. */
    compact?: boolean;
    /** Only the chosen list's colour until the list is opened (Plamen,
     *  2026-09-18: a dot and a name took the add row's room). The name stays
     *  in the field for a screen reader, and in its tooltip. */
    dotOnly?: boolean;
    /** Leave the colour dot out altogether, for a picker whose choices carry
     *  no colour (a task's priority). */
    noDot?: boolean;
    onpick: (id: number | null) => void;
  } = $props();

  const uid = `lp${Math.random().toString(36).slice(2, 8)}`;
  let open = $state(false);
  /** The row the arrows are on, by index into `choices`. */
  let active = $state(0);
  let field: HTMLElement | undefined = $state();
  let list: HTMLElement | undefined = $state();

  const chosen = $derived(choices.find((c) => c.id === value) ?? null);

  function show() {
    if (disabled || choices.length === 0) return;
    active = Math.max(0, choices.findIndex((c) => c.id === value));
    open = true;
  }

  function pick(c: ListChoice) {
    open = false;
    if (c.id !== value) onpick(c.id);
  }

  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    const last = choices.length - 1;
    if (e.key === 'ArrowDown') active = Math.min(last, active + 1);
    else if (e.key === 'ArrowUp') active = Math.max(0, active - 1);
    else if (e.key === 'Home') active = 0;
    else if (e.key === 'End') active = last;
    else if (e.key === 'Enter' || e.key === ' ') pick(choices[active]);
    else if (e.key === 'Escape') {
      // Only this layer: the editor or pane around it stays open.
      e.stopPropagation();
      open = false;
    } else {
      if (e.key === 'Tab') open = false;
      return;
    }
    e.preventDefault();
  }

  // The row the arrows reach stays in sight in a long list.
  $effect(() => {
    if (open) list?.querySelector(`#${uid}-${active}`)?.scrollIntoView({ block: 'nearest' });
  });
</script>

{#snippet dot(c: ListChoice | null)}
  <!-- "No list in particular" is a ring rather than a colour: it is every
       list, not one of them. -->
  <i class="dot" class:any={c?.id === null} style:background={c && c.id !== null ? calendarInk(c.color) : null}></i>
{/snippet}

<span class="picker" class:open class:compact class:dotonly={dotOnly} bind:this={field}>
  <div
    class="field"
    role="combobox"
    tabindex={disabled ? -1 : 0}
    aria-label={label}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls="{uid}-list"
    aria-disabled={disabled ? 'true' : undefined}
    aria-activedescendant={open ? `${uid}-${active}` : undefined}
    title={chosen?.name}
    onclick={() => (open ? (open = false) : show())}
    onkeydown={onKey}
  >
    {#if !noDot}{@render dot(chosen)}{/if}
    <span class="name" class:sr={dotOnly}>{chosen?.name ?? ''}</span>
    {#if !dotOnly}
      <svg class="chev" viewBox="0 0 10 10" width="9" height="9" aria-hidden="true" focusable="false">
        <path d="M2 3.5l3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.3"
              stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {/if}
  </div>

  {#if open}
    <!-- `TimeField`'s scrim: a press anywhere else closes the list, and the
         field sits above it, so pressing the field again closes it too. -->
    <button class="scrim" tabindex="-1" aria-label="Close {label.toLowerCase()} list"
            onclick={() => (open = false)}></button>
    <div class="list quiet-scroll" id="{uid}-list" role="listbox" aria-label={label}
         bind:this={list} use:dropdown={field}>
      {#each choices as c, i (c.id ?? 'any')}
        <!-- `pointerdown` kept from the field, so the keyboard stays where
             it was when the list closes. -->
        <button
          type="button"
          role="option"
          id="{uid}-{i}"
          tabindex="-1"
          aria-selected={c.id === value}
          class:current={c.id === value}
          data-active={i === active ? '' : undefined}
          onpointerdown={(e) => e.preventDefault()}
          onpointermove={() => (active = i)}
          onclick={() => pick(c)}
        >
          {#if !noDot}{@render dot(c)}{/if}
          <span class="name">{c.name}</span>
          {#if c.id === value}<span class="check" aria-hidden="true">✓</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</span>

<style>
  /* In the add row it keeps its answer readable and the title field gives
     way; a long list name still stops at under half the row. */
  .picker { position: relative; display: inline-flex; flex: none; min-width: 0; max-width: 45%; }
  .picker.compact { flex: 0 1 auto; max-width: 100%; }
  .picker.dotonly { max-width: none; }
  .picker.open { z-index: 41; }

  /* The add row's field: the height and the edge of the input beside it. */
  .field { display: inline-flex; align-items: center; gap: 6px; box-sizing: border-box;
           min-width: 0; max-width: 132px; font: inherit; font-size: 12px; color: var(--text);
           cursor: pointer; padding: 0 7px 0 9px; border-radius: 7px;
           border: 1px solid var(--hairline); background: var(--surface); }
  .field:hover:not([aria-disabled='true']) { border-color: var(--muted); }
  .field:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .field[aria-disabled='true'] { cursor: default; opacity: .6; }
  .compact .field { max-width: 100%; font-size: 11.5px; padding: 3px 7px 3px 8px;
                    border-radius: 6px; background: none; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* The add row's neighbour, `newlist`: a square the height of the input. */
  .dotonly .field { width: 30px; padding: 0; justify-content: center; }
  .dotonly .field .dot { width: 11px; height: 11px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden;
        clip-path: inset(50%); white-space: nowrap; }
  .chev { flex: none; color: var(--muted); }

  .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; }
  .dot.any { background: none; box-shadow: inset 0 0 0 1.5px var(--muted); }

  .scrim { position: fixed; inset: 0; z-index: -1; background: none; border: 0;
           padding: 0; cursor: default; }
  /* Fixed and placed by `dropdown`, for `DateField`'s calendar's reason;
     `CalendarPicker`'s rows. */
  .list { position: fixed; top: 0; left: 0; z-index: 1;
          display: flex; flex-direction: column; gap: 1px; min-width: 170px; max-width: 260px;
          max-height: 46vh; overflow-y: auto; padding: 5px; background: var(--surface);
          border: 1px solid var(--hairline); border-radius: 8px;
          box-shadow: 0 10px 32px rgba(0, 0, 0, .45); }
  .list [role='option'] { display: flex; align-items: center; gap: 7px; flex: 0 0 auto;
        font: inherit; font-size: 12px; color: var(--text); cursor: pointer;
        background: none; border: 0; border-radius: 5px; padding: 5px 8px;
        text-align: left; min-width: 0; }
  .list [role='option'][data-active] { background: color-mix(in srgb, var(--text) 8%, transparent); }
  .list [role='option'].current { background: color-mix(in srgb, var(--accent) 18%, transparent); }
  .list [role='option'].current[data-active] {
    background: color-mix(in srgb, var(--accent) 26%, transparent); }
  .check { flex: none; font-size: 11px; color: var(--accent); }
  /* A finger, not a pointer: `TimeField`'s rows' reason. */
  @media (pointer: coarse) {
    .list [role='option'] { padding: 11px 12px; font-size: 14px; }
  }
</style>
