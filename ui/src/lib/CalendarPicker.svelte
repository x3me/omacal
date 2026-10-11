<!-- ui/src/lib/CalendarPicker.svelte -->
<script lang="ts">
  import { byAccount, calendarInk, type Calendar } from './calendars';

  let {
    calendars,
    value,
    disabled = false,
    disabledReason,
    open = $bindable(false),
    onpick,
  }: {
    /** Already filtered to what a write could land on — the caller owns
     *  `writableCalendars`, the same way the old `<select>`'s caller did. */
    calendars: Calendar[];
    value: number | null;
    disabled?: boolean;
    disabledReason?: string;
    /** Bindable so the owning form can subordinate its own Escape guard to
     *  this popover — see `EventForm`'s `escapeCloses` call. */
    open?: boolean;
    onpick: (id: number) => void;
  } = $props();

  const chosen = $derived(calendars.find((c) => c.id === value) ?? null);

  const groups = $derived(byAccount(calendars));

  /** The muted account headings earn their place only when there is more
   *  than one account to tell apart — the single-account rule the old
   *  select's `summary · email` suffix followed. */
  const showHeadings = $derived(groups.length > 1);

  // Escape deliberately lives with the OWNER, not here: the form's one
  // window listener checks `open` (bound) and peels this layer first. A
  // second listener in this component is the double-close `dismiss.svelte`
  // documents — the child's registers first and closes the picker before
  // the form's guard reads it.

  function pick(id: number) {
    open = false;
    onpick(id);
  }
</script>

<span class="picker">
  <button
    type="button"
    class="dot"
    aria-label="Calendar"
    aria-haspopup="listbox"
    aria-expanded={open}
    {disabled}
    title={disabled ? disabledReason : (chosen?.summary ?? 'Calendar')}
    onclick={() => (open = !open)}
  >
    <i style="background:{calendarInk(chosen?.color_hex)}"></i>
  </button>

  {#if open}
    <!-- A sibling of the list, never a wrapper — the same shape every other
         scrim in this app has, and for the same reason. -->
    <button class="scrim" aria-label="Close calendar list" onclick={() => (open = false)}></button>
    <div class="list" role="listbox" aria-label="Calendar">
      {#each groups as group (group.id)}
        {#if showHeadings}<span class="acct" title={group.label}>{group.label}</span>{/if}
        {#each group.calendars as c (c.id)}
          <button
            type="button"
            role="option"
            aria-selected={c.id === value}
            class:current={c.id === value}
            onclick={() => pick(c.id)}
          >
            <i style="background:{calendarInk(c.color_hex)}"></i>
            <span class="name">{c.summary}</span>
            {#if c.id === value}<span class="check" aria-hidden="true">✓</span>{/if}
          </button>
        {/each}
      {/each}
    </div>
  {/if}
</span>

<style>
  .picker { position: relative; display: inline-flex; flex: none; }

  .dot { display: inline-flex; align-items: center; justify-content: center;
         width: 24px; height: 24px; padding: 0; cursor: pointer;
         background: color-mix(in srgb, var(--text) 5%, transparent);
         border: 1px solid var(--hairline); border-radius: 6px; }
  .dot i { width: 11px; height: 11px; border-radius: 50%; }
  .dot:hover:not(:disabled) { border-color: var(--muted); }
  .dot:disabled { cursor: default; opacity: .6; }
  .dot:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }

  .scrim { position: fixed; inset: 0; background: none; border: 0;
           cursor: default; z-index: 44; }

  /* Right-aligned under the dot, like the reference: the dot sits at the
     panel's right edge, so the list grows leftward into the form. */
  .list { position: absolute; top: calc(100% + 4px); right: 0; z-index: 45;
          min-width: 200px; max-width: 260px; max-height: 46vh; overflow-y: auto;
          display: flex; flex-direction: column; gap: 1px; padding: 5px;
          background: var(--surface); border: 1px solid var(--hairline);
          border-radius: 8px; box-shadow: 0 10px 32px rgba(0, 0, 0, .45); }

  .acct { font-size: 9.5px; color: var(--muted); letter-spacing: .04em;
          padding: 5px 8px 2px; overflow: hidden; text-overflow: ellipsis;
          white-space: nowrap; }
  .acct:not(:first-child) { border-top: 1px solid var(--hairline);
                            margin-top: 3px; padding-top: 7px; }

  .list [role='option'] { display: flex; align-items: center; gap: 7px;
        font: inherit; font-size: 12px; color: var(--text); cursor: pointer;
        background: none; border: 0; border-radius: 5px; padding: 4px 8px;
        text-align: left; min-width: 0; }
  .list [role='option'] i { width: 10px; height: 10px; border-radius: 50%; flex: none; }
  .list [role='option'] .name { flex: 1; min-width: 0; overflow: hidden;
                                text-overflow: ellipsis; white-space: nowrap; }
  .list [role='option']:hover { background: color-mix(in srgb, var(--text) 7%, transparent); }
  .list [role='option'].current {
    background: color-mix(in srgb, var(--accent) 18%, transparent); }
  .check { flex: none; font-size: 11px; color: var(--accent); }
</style>
