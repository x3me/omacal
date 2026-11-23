<!-- ui/src/lib/DeleteConfirm.svelte -->
<script lang="ts">
  import ConfirmPanel from './ConfirmPanel.svelte';
  import type { Rect } from './position';
  import type { EventDetail } from './eventdetail';
  import type { Scope } from './eventform';

  let {
    detail,
    anchor,
    onconfirm,
    oncancel,
  }: {
    /** The event as it was loaded. Only ever read here — this panel writes
     *  nothing itself; the caller owns the command and the reload after it,
     *  the same split `EventForm` uses for saving. */
    detail: EventDetail;
    /** The rect to sit beside: the popover the Delete button was clicked in. */
    anchor: Rect;
    /** The scope the user actually chose. Always `'this'` for a one-off, where
     *  the chooser is not offered because the three scopes would name the same
     *  single deletion three times. */
    onconfirm: (scope: Scope) => void;
    oncancel: () => void;
  } = $props();

  let scope = $state<Scope>('this');

  /** Everyone else on the event, which on Google is who the delete emails —
   *  the same exclusion `mailableGuests` makes, and for the same reason:
   *  telling somebody they are about to notify themselves is just wrong. */
  const guests = $derived(detail.attendees.filter((a) => !a.is_self).length);
  const title = $derived(detail.title ?? '(no title)');
  /** A guest cannot delete somebody else's event, only their own copy of
   *  it — and Google's own help is explicit about what that is: the event
   *  comes off this calendar alone, and Calendar tells the organizer you
   *  declined. Unlike a move, `guests_can_modify` changes nothing here:
   *  modifying is one permission, deleting for everyone is the organizer's
   *  alone. So the guest count would be a lie about who loses the event,
   *  and the panel says what actually happens instead.
   *
   *  **Google's rule, so Google only.** On CalDAV what deleting an invitation
   *  does to anyone else is the server's business (a shared calendar has no
   *  "your copy" at all), so the panel claims no reach there and the button
   *  says Delete. */
  const ownCopy = $derived(detail.mails_guests && !detail.is_organizer);
  const organizer = $derived(detail.organizer_email ?? 'the organizer');
</script>

<ConfirmPanel {anchor} label="Delete event" title={`Delete “${title}”?`} {oncancel}>
  {#snippet body()}
    {#if detail.is_recurring}
      <!-- Three operations, not three sizes of one, so each says what it does
           rather than how much of it. The middle one in particular deletes
           nothing at all: it patches the series' rule so it stops earlier, which
           is the only way to lose the tail of a series without also losing the
           occurrences before the clicked one — they are all the same Google
           event. See `deleteEvent`'s doc comment in `eventdetail.ts`.

           Deliberately no occurrence count anywhere here: an open-ended rule has
           no last occurrence to count to, and a number that is only right for the
           rules that happen to end is worse than no number in a dialog with no
           undo. -->
      <div class="scope" role="radiogroup" aria-label="Delete" data-choice-group>
        <label>
          <input
            type="radio"
            name="delete-scope"
            aria-label="This event"
            data-choice
            data-initial-choice
            checked={scope === 'this'}
            onchange={() => (scope = 'this')}
          />
          <span><b>This event</b> — removes this one occurrence. The rest of the series stays.</span>
        </label>
        <!-- Not for a guest: it changes the organizer's rule, which Google
             ignores on a guest's copy (the series came back on the next sync,
             2026-11-23) and applies to everyone where guests may modify it. The
             backend refuses it too (`GUEST_CANNOT_END_SERIES`). -->
        {#if !ownCopy}
        <label>
          <input
            type="radio"
            name="delete-scope"
            aria-label="This and following"
            data-choice
            checked={scope === 'following'}
            onchange={() => (scope = 'following')}
          />
          <span
            ><b>This and following</b> — deletes nothing. It shortens the series to end just before
            this occurrence.</span
          >
        </label>
        {/if}
        <label>
          <input
            type="radio"
            name="delete-scope"
            aria-label="All events"
            data-choice
            checked={scope === 'all'}
            onchange={() => (scope = 'all')}
          />
          <span
            ><b>All events</b> — removes the whole series, including the occurrences that have
            already happened.</span
          >
        </label>
      </div>
    {/if}

    {#if !detail.mails_guests}
      <!-- CalDAV: the DELETE goes to the server and OmaCal sends nobody mail,
           whatever the scope. A server that does CalDAV scheduling (RFC 6638)
           may tell the others itself — the same two halves the form's
           attendee hint names. Nothing at all for an event nobody else is on. -->
      {#if guests > 0}
        <p class="notice" data-testid="delete-no-mail-notice">
          OmaCal emails nobody about this; a server that handles scheduling may.
        </p>
      {/if}
    {:else if ownCopy}
      <!-- What Google does with it, said as it is: a guest cannot take somebody
           else's meeting off their calendar, only decline it, and the meeting
           stays there as declined. OmaCal hides declined meetings unless
           "Show meetings I've declined" is on, which is what makes it go. -->
      <p class="notice" data-testid="delete-own-copy-notice">
        You are a guest on this event, so removing it declines it: Google tells {organizer}, and
        {organizer} and the other guests keep it. OmaCal then hides it, unless "Show meetings I've
        declined" is on in Settings → Appearance.
      </p>
    {:else if guests > 0}
      <!-- `sendUpdates=all` is unconditional on every one of the three paths — the
           DELETE and the "this and following" PATCH alike (see
           `omacal-google`'s `delete_event` and `patch_event`) — so this is not
           hedged on the scope, only worded to say that out loud when there is a
           scope to choose. -->
      <p class="notice" data-testid="delete-guest-notice">
        {detail.is_recurring ? 'Whichever you choose, ' : ''}{guests}
        {guests === 1 ? 'guest is' : 'guests are'} told by email.
      </p>
    {/if}

    <!-- Last, immediately above the button that does it. Google keeps no copy
         OmaCal can reach, and for "All events" the past occurrences go with the
         series. -->
    <p class="undo" data-testid="delete-no-undo">This cannot be undone.</p>
  {/snippet}

  {#snippet actions()}
    <button type="button" class="ghost" data-cancel onclick={oncancel}>Cancel</button>
    <button
      type="button"
      class="primary"
      data-choice
      data-default-choice-action
      onclick={() => onconfirm(scope)}
    >{ownCopy ? 'Remove from my calendar' : 'Delete'}</button>
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

  .undo { font-size: 10.5px; color: var(--muted); line-height: 1.4; margin: 0; }
</style>
