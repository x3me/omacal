<!-- ui/src/lib/SaveConfirm.svelte -->
<script lang="ts">
  import ConfirmPanel from './ConfirmPanel.svelte';
  import type { Rect } from './position';
  import type { EditReach, SendUpdates } from './eventdetail';

  let {
    guests,
    reach,
    verb,
    title,
    anchor,
    onconfirm,
    oncancel,
  }: {
    /** How many people a save would email — **everyone but the person doing
     *  it**, which is the count `mailableGuests` already answers and
     *  the same exclusion `MoveConfirm` and `DeleteConfirm` make. */
    guests: number;
    /** Who the save reaches — `editReach` in `eventdetail.ts`. `'own-copy'`
     *  is a guest saving their copy of somebody else's event: Google changes
     *  this calendar alone and tells nobody, so the notify choice is not
     *  offered and the panel says why instead. A create is always
     *  `'organizer'`. */
    reach: EditReach;
    /** What the button under this panel says — `'Save'` on an edit, `'Create'`
     *  on a create. The panel has to name the action it is confirming: "Save"
     *  over a form whose own action reads "Create" is a small lie in the one
     *  dialog whose whole job is to be unambiguous about mailing other people. */
    verb: 'Save' | 'Create';
    /** The event's title, so the heading names what is being saved. Whatever
     *  opened this panel is behind it and cannot be read. */
    title: string;
    /** The rect to sit beside: the form's own anchor. */
    anchor: Rect;
    /** The choice. This panel writes nothing and saves nothing — it hands an
     *  answer back and the form does the saving, the same split every other
     *  panel in this app uses. */
    onconfirm: (sendUpdates: SendUpdates) => void;
    /** Back to the form, with everything typed into it still there. */
    oncancel: () => void;
  } = $props();
</script>

<!--
  Guest-list spec §3, and it is `MoveConfirm`'s panel with a save's words on it
  rather than a third dialog: `ConfirmPanel` exists because a fourth
  near-identical one would drift, and the drag work built it for this.

  The form used to warn "Saving will notify N guests" and pass `all`
  unconditionally. That reasoning was sound while a save was the only way to
  change an event — a time typed on purpose is exactly what guests need to hear
  about. It stopped being sound the moment the form could edit the guest list
  itself: correcting a typo in an address, or marking somebody optional, would
  mail the whole room about a change that concerns one person.
-->
<ConfirmPanel {anchor} label="{verb} event" title={`${verb} “${title}”?`} {oncancel}>
  {#snippet body()}
    {#if reach === 'own-copy'}
      <p class="notice" data-testid="save-own-copy-notice">
        You are a guest on this event. Saving changes only your copy — the organizer and the
        other guests keep theirs, and nobody is told.
      </p>
    {:else}
      <p class="notice" data-testid="save-guest-notice">
        {#if reach === 'shared'}The organizer lets guests change this event, so the save
          reaches everyone. {/if}{guests}
        {guests === 1 ? 'guest' : 'guests'} can be told by email, or not — the two buttons below are
        the choice.
      </p>
    {/if}
  {/snippet}

  {#snippet actions()}
    <button type="button" class="ghost" data-cancel onclick={oncancel}>Cancel</button>
    {#if reach === 'own-copy'}
      <!-- Nobody to tell: the write is to this calendar's copy alone, so the
           only honest button names that. -->
      <button type="button" class="primary" data-choice onclick={() => onconfirm('none')}>
        {verb} my copy
      </button>
    {:else if verb === 'Create'}
    <!-- A new event with guests is an invitation: telling them is what
         creating it is for, so notifying is primary here (Plamen, 2026-11-19),
         as Quick Add's direct create already does. Focus still starts on
         Cancel, so Enter alone sends nothing. -->
    <button type="button" class="ghost" data-choice onclick={() => onconfirm('none')}>
      {verb} without notifying
    </button>
    <button type="button" class="primary" data-choice onclick={() => onconfirm('all')}>
      {verb} and notify guests
    </button>
    {:else}
    <!-- **On an edit, not notifying is the primary action**, and the order
         says so — the same ruling as the drag's, for the same reason: mailing
         everyone about a typo fix is the outcome this choice exists to
         prevent. This is also the only path from this form to
         `sendUpdates=all`. -->
    <button type="button" class="ghost" data-choice onclick={() => onconfirm('all')}>
      {verb} and notify guests
    </button>
    <button type="button" class="primary" data-choice onclick={() => onconfirm('none')}>
      {verb} without notifying
    </button>
    {/if}
  {/snippet}
</ConfirmPanel>

<style>
  .notice { font-size: 10.5px; color: var(--text); line-height: 1.4; margin: 0;
            padding: 6px 8px; border-radius: 5px;
            background: color-mix(in srgb, var(--text) 6%, transparent); }
</style>
