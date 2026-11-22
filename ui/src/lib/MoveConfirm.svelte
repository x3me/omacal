<!-- ui/src/lib/MoveConfirm.svelte -->
<script lang="ts">
  import ConfirmPanel from './ConfirmPanel.svelte';
  import type { Rect } from './position';
  import { editReach, type EventDetail, type SendUpdates } from './eventdetail';
  import type { Scope } from './eventform';

  let {
    detail,
    anchor,
    onconfirm,
    oncancel,
  }: {
    /** The event as it was loaded. Read only — this panel writes nothing; the
     *  caller owns the command and the reload, the same split `DeleteConfirm`
     *  and `EventForm` use. */
    detail: EventDetail;
    /** The rect to sit beside: the block that was dropped. */
    anchor: Rect;
    /**
     * The choice the user made. **Both halves come back together**, because
     * this is one dialog and not two: spec §3's rule is at most one per drop,
     * so when a recurring event also has guests the scope prompt carries the
     * notify choice rather than a second panel appearing behind it.
     */
    onconfirm: (choice: { scope: Scope; sendUpdates: SendUpdates }) => void;
    /** §4: cancelling returns the block and writes nothing at all. */
    oncancel: () => void;
  } = $props();

  let scope = $state<Scope>('this');

  /** Everyone the move would email, which is everyone but the person doing it
   *  — the same exclusion `DeleteConfirm` and `mailableGuests` make, and for
   *  the same reason: telling somebody they are about to notify themselves is
   *  just wrong. */
  const guests = $derived(
    // CalDAV mails nobody, so there is nobody the move *could* email and the
    // notify choice is not offered — the rule the form's Save already keeps.
    detail.mails_guests ? detail.attendees.filter((a) => !a.is_self).length : 0,
  );
  const title = $derived(detail.title ?? '(no title)');
  /** Who the move reaches — see `editReach`. A guest's move of their own copy
   *  has nobody to notify, however many people are on the event, so the
   *  notify choice is not offered and the panel says why instead. */
  const reach = $derived(editReach(detail));
  /** The own-copy notice is Google's documented behaviour, so it is said on
   *  Google only; on CalDAV whether anyone else's copy moves is the server's
   *  business, and a series there gets the plain Move. */
  const ownCopy = $derived(detail.mails_guests && reach === 'own-copy');
  const organizer = $derived(detail.organizer_email ?? 'the organizer');
</script>

<ConfirmPanel {anchor} label="Move event" title={`Move “${title}”?`} {oncancel}>
  {#snippet body()}
    {#if detail.is_recurring}
      <!-- Three operations, not three sizes of one — the same reasoning
           `DeleteConfirm` spells out, with the copy a *move* needs rather than
           the copy a delete needs. "This and following" in particular is two
           writes: it ends the old series before this occurrence and creates a
           new one at the new time, which is why it cannot be described as
           "moves some of them". -->
      <div class="scope" role="radiogroup" aria-label="Move" data-choice-group>
        <label>
          <input
            type="radio"
            name="move-scope"
            aria-label="This event"
            data-choice
            data-initial-choice
            checked={scope === 'this'}
            onchange={() => (scope = 'this')}
          />
          <span><b>This event</b> — moves this one occurrence. The rest of the series stays.</span>
        </label>
        <label>
          <input
            type="radio"
            name="move-scope"
            aria-label="This and following"
            data-choice
            checked={scope === 'following'}
            onchange={() => (scope = 'following')}
          />
          <span
            ><b>This and following</b> — ends the series before this occurrence and starts a new
            one at the new time.</span
          >
        </label>
        <label>
          <input
            type="radio"
            name="move-scope"
            aria-label="All events"
            data-choice
            checked={scope === 'all'}
            onchange={() => (scope = 'all')}
          />
          <span
            ><b>All events</b> — shifts every occurrence by the same amount, earlier ones
            included. It does not move them all onto this day.</span
          >
        </label>
      </div>
    {/if}

    {#if ownCopy}
      <!-- Google keeps a guest's copy apart from the organizer's: the moved
           time lands on this calendar alone, and nobody else's changes. Said
           here, before the write, because the drag itself looks exactly like
           moving your own meeting — and Google's own UI warns at this point
           too. -->
      <p class="notice" data-testid="move-own-copy-notice">
        You are a guest on this event. Moving it changes only your copy — {organizer} and
        the other guests keep the original time, and nobody is told.
      </p>
    {:else if guests > 0}
      <p class="notice" data-testid="move-guest-notice">
        {#if reach === 'shared'}The organizer lets guests change this event, so the move
          reaches everyone. {/if}{guests}
        {guests === 1 ? 'guest' : 'guests'} can be told by email, or not — the two buttons below
        are the choice.
      </p>
    {/if}
  {/snippet}

  {#snippet actions()}
    <button type="button" class="ghost" data-cancel onclick={oncancel}>Cancel</button>
    {#if ownCopy}
      <!-- Nobody to tell: the write is to this calendar's copy alone, so the
           only honest button names that. `none` because there is no mail to
           send, not because the user declined to send it. -->
      <button
        type="button"
        class="primary"
        data-choice
        data-default-choice-action
        data-initial-choice={detail.is_recurring ? undefined : ''}
        onclick={() => onconfirm({ scope, sendUpdates: 'none' })}
      >Move my copy</button>
    {:else if guests > 0}
      <!-- **Notifying is the primary action** (Plamen, 2026-11-20: "when
           rescheduling a meeting the default should be to Notify"): a meeting
           moved is a meeting its guests need to hear about. It was the other
           way round, so that an accidental drag could not mail the guest
           list; the dialog itself still stands between a drag and any mail,
           and not notifying is one click away. -->
      <button
        type="button"
        class="ghost"
        data-choice
        onclick={() => onconfirm({ scope, sendUpdates: 'none' })}
      >Move without notifying</button>
      <button
        type="button"
        class="primary"
        data-choice
        data-default-choice-action
        data-initial-choice={detail.is_recurring ? undefined : ''}
        onclick={() => onconfirm({ scope, sendUpdates: 'all' })}
      >Move and notify guests</button>
    {:else}
      <!-- Nobody to tell, so there is nothing to choose between. This panel is
           only open at all because the event repeats. -->
      <button
        type="button"
        class="primary"
        data-choice
        data-default-choice-action
        onclick={() => onconfirm({ scope, sendUpdates: 'none' })}
      >Move</button>
    {/if}
  {/snippet}
</ConfirmPanel>

<style>
  .scope { display: flex; flex-direction: column; gap: 6px; font-size: 11px; color: var(--muted); }
  .scope label { display: flex; align-items: flex-start; gap: 6px; cursor: pointer; line-height: 1.4; }
  .scope input { margin: 2px 0 0; flex: none; }
  .scope b { color: var(--text); font-weight: 600; }

  .notice { font-size: 10.5px; color: var(--text); line-height: 1.4; margin: 0;
            padding: 6px 8px; border-radius: 5px;
            background: color-mix(in srgb, var(--text) 6%, transparent); }
</style>
