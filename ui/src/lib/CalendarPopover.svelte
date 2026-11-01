<!-- ui/src/lib/CalendarPopover.svelte -->
<script lang="ts">
  import { escapeCloses } from './dismiss.svelte';
  import CalendarGroupsPanel from './CalendarGroupsPanel.svelte';
  import type { Calendar } from './calendars';
  import type { CalendarGroup } from './calendargroups';

  let {
    calendars,
    onchange,
    open = $bindable(false),
    groups = [],
    onapplygroup = () => {},
    onsavegroups = async () => {},
  }: {
    calendars: Calendar[];
    onchange: () => void;
    open?: boolean;
    /** The saved calendar groups. This is where the first one is made: the
     *  header's switcher only appears once one exists. */
    groups?: CalendarGroup[];
    /** A group's row was clicked. The panel stays open, so the boxes below
     *  are seen to follow it. */
    onapplygroup?: (i: number) => void;
    /** Asked to store the whole new list. Rejects with the sentence to show. */
    onsavegroups?: (next: CalendarGroup[]) => Promise<void>;
  } = $props();

  const shown = $derived(calendars.filter((c) => c.sync_enabled && c.selected).length);

  function toggle(e: MouseEvent) {
    open = !open;
    if (open) {
      // WebKit, unlike Chromium, does not focus a <button> on click — only on
      // Tab. Without an explicit focus() here, Escape has nothing local to
      // bubble from until the user tabs somewhere, and the popover would open
      // to mouse clicks but not close to the keyboard until it was touched.
      (e.currentTarget as HTMLElement).focus();
    }
  }

  function close() {
    open = false;
  }

  // Only while open, which is this panel's whole guard — see `escapeCloses`
  // for why the listener is on `window` and why the guard is the caller's.
  escapeCloses(() => open, close);
</script>

<!--
  **A host, not a list.** Everything about a calendar *row* — the two switches,
  the busy set, the messages, the focus reclaim — is `CalendarList`'s, so the
  same rows can appear here and in the settings modal's Calendars tab without a
  second implementation to keep in step. What is left here is the popover: a
  trigger, a count, a scrim, and a panel to float them in. The panel's body is
  `CalendarGroupsPanel` — the list plus saved groups — the same body the
  header's group switcher floats, so a group is made and managed in one piece
  of code whichever door it is reached by.

  The `$effect` that used to clear a stale note whenever `open` became true is
  **gone, and its behaviour is not.** The list lives inside `{#if open}`, so
  each open mounts a fresh one whose `message` starts `null` — the reopening
  spec passes on that alone. Keeping the effect would have been keeping a line
  that could no longer fire.
-->
<div class="wrap">
  <button class="trigger" onclick={toggle} aria-expanded={open}>
    Calendars <span class="count">{shown}</span>
  </button>

  {#if open}
    <!-- Click-away. Deliberately a sibling rather than a document listener:
         no global state to leak if the component unmounts while open. -->
    <button class="scrim" aria-label="Close" onclick={close}></button>

    <div class="panel" role="group" aria-label="Calendars">
      <CalendarGroupsPanel {calendars} {groups} onapply={onapplygroup} onsave={onsavegroups} {onchange} />
    </div>
  {/if}
</div>

<style>
  .wrap { position: relative; }
  .trigger { font: inherit; font-size: 11px; color: var(--muted); cursor: pointer;
             background: color-mix(in srgb, var(--text) 6%, transparent);
             border: 0; border-radius: 6px; padding: 4px 10px; }
  .count { opacity: .7; margin-left: 4px; }

  .scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default; z-index: 40; }

  .panel { position: absolute; right: 0; top: calc(100% + 6px); z-index: 41;
           min-width: 300px; max-height: 60vh; overflow-y: auto;
           display: flex; flex-direction: column; gap: 2px;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 8px; padding: 8px; box-shadow: 0 8px 28px rgba(0,0,0,.45); }
</style>
