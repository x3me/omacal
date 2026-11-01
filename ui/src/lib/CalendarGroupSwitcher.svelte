<!-- ui/src/lib/CalendarGroupSwitcher.svelte -->
<script lang="ts">
  import { escapeCloses } from './dismiss.svelte';
  import CalendarGroupsPanel from './CalendarGroupsPanel.svelte';
  import type { Calendar } from './calendars';
  import { activeGroupIndex, groupChordLabel, type CalendarGroup } from './calendargroups';

  let {
    calendars,
    groups,
    onapply,
    onsave,
    onchange,
    open = $bindable(false),
  }: {
    calendars: Calendar[];
    groups: CalendarGroup[];
    /** Asked to show group `i`'s calendars. `App` owns it, because the chord
     *  does the same thing without this panel open. */
    onapply: (i: number) => void;
    /** Asked to store the whole new list. Rejects with the sentence to show. */
    onsave: (next: CalendarGroup[]) => Promise<void>;
    /** A calendar was ticked, relabelled or recoloured in the list below.
     *  The same refresh `CalendarPopover` is given, so `calendars` — and with
     *  it the label on the button — follows the box as it is ticked. */
    onchange: () => void;
    /** Bound by `Header`, which keeps this mounted while it is open: deleting
     *  the last group from here leaves the panel up rather than pulling the
     *  button out from under the pointer. */
    open?: boolean;
  } = $props();

  const active = $derived(activeGroupIndex(groups, calendars));
  const label = $derived(
    active >= 0
      ? groups[active].name
      : calendars.every((c) => !c.sync_enabled || c.selected) ? 'All calendars' : 'Custom',
  );

  function toggle(e: MouseEvent) {
    open = !open;
    // WebKit does not focus a <button> on click, only on Tab — see
    // `CalendarPopover.toggle`. Without it Escape has nothing to bubble from.
    if (open) (e.currentTarget as HTMLElement).focus();
  }

  function close() {
    open = false;
  }

  // The innermost thing first: Escape in a name field backs out of the field,
  // and only a second one closes the panel. The field's own handler stops the
  // first from also reaching this listener.
  escapeCloses(() => open, close);

  function apply(i: number) {
    onapply(i);
    close();
  }
</script>

<!--
  **A saved choice of what is shown, one click or ⌘1–⌘9 away.** The button
  names the group whose calendars are exactly the ones on screen, and says
  "All calendars" or "Custom" when none is — derived, never stored, so a
  calendar ticked by hand in the picker moves the label with it.

  **Only there once a group is.** The header was emptied on purpose (settings
  spec §1), so `Header` mounts this only when a group exists; the first one is
  made in Menu → Calendars. The panel body is `CalendarGroupsPanel`, the same
  one that menu hosts, so there is one save field, not two.

  Everything here is about *shown*. Applying a group never touches
  `sync_enabled`; a calendar a group hides is still syncing.
-->
<div class="cg-wrap">
  <button class="cg-trigger" onclick={toggle} aria-expanded={open} aria-haspopup="true"
          title="Calendar groups ({groupChordLabel(0) ?? ''}–9)">
    <span class="cg-label">{label}</span> <span class="cg-caret" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <button class="cg-scrim" aria-label="Close" onclick={close}></button>

    <div class="cg-panel" role="group" aria-label="Calendar groups">
      <CalendarGroupsPanel {calendars} {groups} onapply={apply} {onsave} {onchange} />
    </div>
  {/if}
</div>

<style>
  .cg-wrap { position: relative; }
  .cg-trigger { font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
           background: color-mix(in srgb, var(--text) 6%, transparent);
           border: 0; border-radius: 6px; padding: 4px 10px;
           display: flex; align-items: center; gap: 4px; max-width: 170px; }
  .cg-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cg-caret { font-size: 9px; opacity: .7; }

  .cg-scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default;
           z-index: 40; padding: 0; border-radius: 0; }
  .cg-panel { position: absolute; left: 0; top: calc(100% + 6px); z-index: 41;
           min-width: 300px; max-height: 70vh; overflow-y: auto;
           display: flex; flex-direction: column; gap: 2px;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 8px; padding: 6px; box-shadow: 0 8px 28px rgba(0,0,0,.45); }
</style>
