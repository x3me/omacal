<!-- ui/src/lib/InviteTray.svelte -->
<script lang="ts">
  import { formatDate } from './datefmt';
  import { dateFormat } from './date.svelte';
  import { respondToEvent } from './eventdetail';
  import { pendingResponse, responseFailure, dismissResponseFailure, showResponseFailuresHere } from './responses.svelte';
  import { clockFormat } from './clock.svelte';
  import { formatClock } from './timefmt';
  import { escapeCloses } from './dismiss.svelte';
  import {
    dismissAllChangeNotices, dismissAllDeclineNotices,
    dismissChangeNotice, dismissDeclineNotice,
    type ChangeNotice, type DeclineNotice, type PendingInvite,
  } from './invites';

  let { invites, declines = [], changes = [], onanswered, ondismissed = () => {} }: {
    /** `App`'s list, with queued/saved answers hidden until the refetch
     *  confirms them. Failed answers return without changing this prop. */
    invites: PendingInvite[];
    /** Guests who declined the user's own meetings, unacknowledged — the
     *  organizer's side of the same tray (2026-08-18, by request: in the
     *  app only, no toast). */
    declines?: DeclineNotice[];
    /** Meetings the user attends that moved or were cancelled under them —
     *  the attendee's side, same request, same lifecycle. */
    changes?: ChangeNotice[];
    /** A queued RSVP reached the provider. App schedules a background sync. */
    onanswered: () => void;
    /** A notice was dismissed locally. App only refetches lists and the grid. */
    ondismissed?: () => void;
  } = $props();

  let open = $state(false);
  /** Whether the panel hangs from the badge's left edge instead of its
   *  right — decided from real geometry at each open; see the onclick. */
  let alignLeft = $state(false);
  /** In-flight row actions lock only their own controls.
   *  Reassigned, never mutated: a `$state` array notifies on assignment. */
  let busyIds = $state<number[]>([]);
  let answeredIds = $state<number[]>([]);
  const shownInvites = $derived(invites.filter(inv =>
    !pendingResponse(inv.id, inv.start_ms) && !answeredIds.includes(inv.id)));
  $effect(() => {
    const remaining = answeredIds.filter(id => invites.some(inv => inv.id === id));
    if (remaining.length !== answeredIds.length) answeredIds = remaining;
  });
  // Only local acknowledgment failures live here; RSVP failures have one
  // shared record, also readable after this tray closes.
  let errors = $state<Record<number, string>>({});

  const errorFor = (id: number, startMs?: number) => responseFailure(id, startMs)?.message ?? errors[id];
  const changeStart = (c: ChangeNotice) => c.respond_scope === 'this' ? c.respond_start_ms ?? undefined : undefined;
  function dismissError(id: number, startMs?: number) {
    const failure = responseFailure(id, startMs);
    if (failure) dismissResponseFailure(failure.key);
    const {[id]: _gone, ...rest} = errors; errors = rest;
  }
  $effect(() => {
    if (open) return showResponseFailuresHere([
      ...shownInvites.flatMap(inv => { const f = responseFailure(inv.id); return f ? [f.key] : []; }),
      ...shownMoved.flatMap(c => { const f = c.event_id === null ? undefined : responseFailure(c.event_id, changeStart(c)); return f ? [f.key] : []; }),
    ]);
  });

  const hhmm = (ms: number) => formatClock(ms, clockFormat());

  const day = (ms: number) =>
    formatDate(new Date(ms).getTime(), dateFormat(), { weekday: 'short', month: 'short', day: 'numeric' });

  /** `yyyy-mm-dd` (a calendar-zone day) rendered as "Mon, Aug 17". Built
   *  from parts, never from `Date.parse` — a bare ISO date parses as UTC
   *  midnight and shifts a day for any browser east of Greenwich. */
  function dateWords(d: string): string {
    const [y, m, dd] = d.split('-').map(Number);
    return formatDate(new Date(y, m - 1, dd).getTime(), dateFormat(), {
      weekday: 'short', month: 'short', day: 'numeric',
    });
  }

  type Dated = Pick<PendingInvite, 'is_all_day' | 'start_date' | 'end_date' | 'start_ms' | 'end_ms'>;
  function when(inv: Dated): string {
    if (inv.is_all_day && inv.start_date && inv.end_date) {
      return inv.start_date === inv.end_date
        ? `${dateWords(inv.start_date)} · All day`
        : `${dateWords(inv.start_date)} – ${dateWords(inv.end_date)} · All day`;
    }
    return `${day(inv.start_ms)} · ${hhmm(inv.start_ms)} – ${hhmm(inv.end_ms)}`;
  }

  /** Declines already ×-ed this session, hidden immediately — the write is
   *  idempotent and `App`'s refetch confirms, but the row must not wait for
   *  the round trip under the finger that dismissed it. */
  let acked = $state<string[]>([]);
  const ackKey = (d: DeclineNotice) => `${d.calendar_id}:${d.gid}:${d.email}`;
  const shownDeclines = $derived(declines.filter((d) => !acked.includes(ackKey(d))));

  async function acknowledge(d: DeclineNotice) {
    acked = [...acked, ackKey(d)];
    try {
      await dismissDeclineNotice(d);
      ondismissed();
    } catch {
      // The write failed; the row comes back rather than lying about it.
      acked = acked.filter((k) => k !== ackKey(d));
    }
  }

  /** All the ×s in one stroke — same optimistic hide, same honesty on
   *  failure: rows return rather than pretending they were acknowledged. */
  async function acknowledgeAll() {
    const before = acked;
    acked = [...acked, ...shownDeclines.map(ackKey)];
    try {
      await dismissAllDeclineNotices();
      ondismissed();
    } catch {
      acked = before;
    }
  }

  /** Same pair of moves for the change sections, keyed and stroked per kind. */
  let ackedChanges = $state<string[]>([]);
  const changeKey = (c: ChangeNotice) => `${c.calendar_id}:${c.gid}`;
  const shownMoved = $derived(
    changes.filter((c) => c.kind === 'moved' && !ackedChanges.includes(changeKey(c))));
  const shownCancelled = $derived(
    changes.filter((c) => c.kind === 'cancelled' && !ackedChanges.includes(changeKey(c))));

  // Emptying the tray closes it. A later failed reply restores the badge,
  // not a scrim over whatever the user has moved on to.
  $effect(() => {
    if (shownInvites.length + shownDeclines.length + shownMoved.length + shownCancelled.length === 0) open = false;
  });

  async function acknowledgeChange(c: ChangeNotice) {
    ackedChanges = [...ackedChanges, changeKey(c)];
    try {
      await dismissChangeNotice(c);
      ondismissed();
    } catch {
      ackedChanges = ackedChanges.filter((k) => k !== changeKey(c));
    }
  }

  async function acknowledgeAllChanges(kind: 'moved' | 'cancelled') {
    const before = ackedChanges;
    const batch = kind === 'moved' ? shownMoved : shownCancelled;
    ackedChanges = [...ackedChanges, ...batch.map(changeKey)];
    try {
      await dismissAllChangeNotices(kind);
      ondismissed();
    } catch {
      ackedChanges = before;
    }
  }

  /** "Wed, Jan 3 · 15:30" — one endpoint of a move, or a cancellation's
   *  vacated slot. All-day meetings speak in their calendar-zone day. */
  function slot(dateStr: string | null, ms: number, allDay: boolean): string {
    if (allDay && dateStr) return dateWords(dateStr);
    return `${day(ms)} · ${hhmm(ms)}`;
  }

  async function answer(inv: PendingInvite, response: 'accepted' | 'tentative' | 'declined') {
    busyIds = [...busyIds, inv.id];
    const { [inv.id]: _gone, ...rest } = errors;
    errors = rest;
    try {
      // Scope `all`, the invitation's own semantics: answering an invite
      // answers the series, exactly as the emailed Yes would. The anchor is
      // the master's own start — with scope `all` no instance is ever
      // resolved, so `detail.start_ms`'s trap has no purchase here.
      await respondToEvent(inv.id, response, 'all', inv.start_ms, inv.title ?? '(no title)');
      answeredIds = [...answeredIds, inv.id];
      onanswered();
    } catch {
      // The queue restores the row and owns the error, even after closing.
    } finally {
      busyIds = busyIds.filter((id) => id !== inv.id);
    }
  }

  /**
   * "Can you still make the new time?" — a Rescheduled row's Yes/Maybe/No
   * (2026-08-21, by request). The answer is the popover's own write, at the
   * scope the backend already decided (a moved exception answers this
   * occurrence, a moved master the series), and answering *is* dealing with
   * the notice — the acknowledgment rides along, so one click does both.
   * Errors key on the event id, same slot the invite rows use.
   */
  async function answerChange(c: ChangeNotice, response: 'accepted' | 'tentative' | 'declined') {
    if (c.event_id === null || c.respond_start_ms === null) return;
    const id = c.event_id;
    ackedChanges = [...ackedChanges, changeKey(c)];
    busyIds = [...busyIds, id];
    const { [id]: _gone, ...rest } = errors;
    errors = rest;
    try {
      await respondToEvent(id, response, c.respond_scope, c.respond_start_ms, c.title ?? '(no title)');
      await dismissChangeNotice(c);
      onanswered();
    } catch (e) {
      ackedChanges = ackedChanges.filter(key => key !== changeKey(c));
      if (!responseFailure(id, changeStart(c))) errors = { ...errors, [id]: String(e) };
    } finally {
      busyIds = busyIds.filter((b) => b !== id);
    }
  }

  escapeCloses(() => open, () => (open = false));
</script>

{#if shownInvites.length + shownDeclines.length + shownMoved.length + shownCancelled.length > 0}
  <div class="wrap">
    <!-- The badge: present exactly while something awaits attention, so its
         absence means inbox-zero rather than "feature off". A count, not a
         dot — one item and four ask for different amounts of your attention.
         The label says what kinds, because an invitation asks for an answer
         and a decline only asks to be seen. -->
    <button
      class="badge"
      aria-label={[
        shownInvites.length > 0
          ? `${shownInvites.length} pending ${shownInvites.length === 1 ? 'invitation' : 'invitations'}` : '',
        shownDeclines.length > 0
          ? `${shownDeclines.length} ${shownDeclines.length === 1 ? 'decline' : 'declines'}` : '',
        shownMoved.length > 0 ? `${shownMoved.length} rescheduled` : '',
        shownCancelled.length > 0 ? `${shownCancelled.length} cancelled` : '',
      ].filter(Boolean).join(', ')}
      aria-expanded={open}
      title="Invitations and replies"
      onclick={(e) => {
        open = !open;
        // WebKit does not focus a <button> on click — the same line
        // Header's burger carries, for the same Escape-needs-a-focus reason.
        if (open) {
          const el = e.currentTarget as HTMLElement;
          // The panel hangs from whichever side of the badge has room. It
          // used to hang right unconditionally, which assumed the badge
          // lives near the window's right edge — in a tiled window the
          // header wraps, the badge lands left, and the panel walked off
          // the screen (seen live, 2026-08-19). 428 = the panel's max
          // width plus its margin.
          alignLeft = el.getBoundingClientRect().right < 428;
          el.focus();
        }
      }}
    >✉ {shownInvites.length + shownDeclines.length + shownMoved.length + shownCancelled.length}</button>

    {#if open}
      <button class="scrim" aria-label="Close invitations" onclick={() => (open = false)}></button>
      <div class="panel" class:alignleft={alignLeft} role="group" aria-label="Pending invitations">
        {#each shownInvites as inv (inv.id)}
          <div class="row" data-testid="invite-row">
            <span class="tick" style:background={inv.color ?? 'var(--muted)'}></span>
            <div class="text">
              <span class="title">{inv.title ?? '(no title)'}</span>
              <span class="meta">{when(inv)}</span>
              {#if inv.organizer_email}
                <span class="meta">from {inv.organizer_email}</span>
              {/if}
              {#if errorFor(inv.id)}
                <span class="rowerr" role="alert">{errorFor(inv.id)} <button aria-label="Dismiss response error" onclick={() => dismissError(inv.id)}>×</button></span>
              {/if}
            </div>
            {#if inv.can_respond}
              <div class="rsvp">
                <button disabled={busyIds.includes(inv.id)} onclick={() => answer(inv, 'accepted')}>Yes</button>
                <button disabled={busyIds.includes(inv.id)} onclick={() => answer(inv, 'tentative')}>Maybe</button>
                <button disabled={busyIds.includes(inv.id)} onclick={() => answer(inv, 'declined')}>No</button>
              </div>
            {:else}
              <!-- A CalDAV (or read-only) invitation is real and listed; the
                   answer just lives with the provider. Saying so beats three
                   buttons that could only fail. -->
              <span class="meta">answer at your provider</span>
            {/if}
          </div>
        {/each}

        {#if shownDeclines.length > 0}
          <!-- The section row earns its keep beyond labelling: it carries
               Dismiss all, offered once there is an "all" to speak of — a
               single decline's × is already under the finger. -->
          <div class="sect" class:joined={shownInvites.length > 0}>
            <span>Declined your meeting</span>
            {#if shownDeclines.length > 1}
              <button class="ackall" onclick={acknowledgeAll}>Dismiss all</button>
            {/if}
          </div>
        {/if}
        {#each shownDeclines as d (ackKey(d))}
          <div class="row" data-testid="decline-row">
            <span class="tick" style:background={d.color ?? 'var(--muted)'}></span>
            <div class="text">
              <span class="title">{d.display_name ?? d.email} declined</span>
              <span class="meta">{d.title ?? '(no title)'}</span>
              <span class="meta">{when(d)}</span>
            </div>
            <button
              class="ack"
              aria-label="Dismiss decline by {d.display_name ?? d.email}"
              title="Got it"
              onclick={() => acknowledge(d)}
            >×</button>
          </div>
        {/each}

        {#if shownMoved.length > 0}
          <div class="sect" class:joined={shownInvites.length + shownDeclines.length > 0}>
            <span>Rescheduled</span>
            {#if shownMoved.length > 1}
              <button class="ackall" onclick={() => acknowledgeAllChanges('moved')}>Dismiss all</button>
            {/if}
          </div>
        {/if}
        {#each shownMoved as c (changeKey(c))}
          <div class="row" data-testid="moved-row">
            <span class="tick" style:background={c.color ?? 'var(--muted)'}></span>
            <div class="text">
              <span class="title">{c.title ?? '(no title)'}</span>
              <span class="meta">
                {slot(c.old_start_date, c.old_start_ms, c.is_all_day)}
                &nbsp;→&nbsp;
                {#if c.new_start_ms !== null}
                  {slot(c.new_start_date, c.new_start_ms, c.is_all_day)}{#if !c.is_all_day && c.new_end_ms !== null}&nbsp;– {hhmm(c.new_end_ms)}{/if}
                {/if}
              </span>
              {#if c.event_id !== null && errorFor(c.event_id, changeStart(c))}
                <span class="rowerr" role="alert">{errorFor(c.event_id, changeStart(c))} <button aria-label="Dismiss response error" onclick={() => dismissError(c.event_id!, changeStart(c))}>×</button></span>
              {/if}
            </div>
            {#if c.can_respond && c.event_id !== null}
              <!-- The same three answers the invitation rows offer, because a
                   reschedule is a new proposal your old yes should not cover
                   silently. Answering also dismisses — one click, dealt with.
                   The × stays for "seen, saying nothing". -->
              <div class="rsvp">
                <button disabled={busyIds.includes(c.event_id)} onclick={() => answerChange(c, 'accepted')}>Yes</button>
                <button disabled={busyIds.includes(c.event_id)} onclick={() => answerChange(c, 'tentative')}>Maybe</button>
                <button disabled={busyIds.includes(c.event_id)} onclick={() => answerChange(c, 'declined')}>No</button>
              </div>
            {/if}
            <button
              class="ack"
              aria-label="Dismiss reschedule of {c.title ?? '(no title)'}"
              title="Got it"
              onclick={() => acknowledgeChange(c)}
            >×</button>
          </div>
        {/each}

        {#if shownCancelled.length > 0}
          <div class="sect"
               class:joined={shownInvites.length + shownDeclines.length + shownMoved.length > 0}>
            <span>Cancelled</span>
            {#if shownCancelled.length > 1}
              <button class="ackall" onclick={() => acknowledgeAllChanges('cancelled')}>Dismiss all</button>
            {/if}
          </div>
        {/if}
        {#each shownCancelled as c (changeKey(c))}
          <div class="row" data-testid="cancelled-row">
            <span class="tick" style:background={c.color ?? 'var(--muted)'}></span>
            <div class="text">
              <span class="title">{c.title ?? '(no title)'}</span>
              <span class="meta">was {slot(c.old_start_date, c.old_start_ms, c.is_all_day)}</span>
            </div>
            <button
              class="ack"
              aria-label="Dismiss cancellation of {c.title ?? '(no title)'}"
              title="Got it"
              onclick={() => acknowledgeChange(c)}
            >×</button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .wrap { position: relative; }
  /* The update notice's accent language, not the error's red: invitations
     are options, and a red badge would teach users to ignore red. */
  .badge { font: inherit; font-size: 12px; cursor: pointer; border: 0;
           border-radius: 6px; padding: 4px 10px; font-weight: 600;
           background: color-mix(in srgb, var(--accent) 18%, transparent);
           color: var(--text); }
  .badge:hover { background: color-mix(in srgb, var(--accent) 28%, transparent); }
  .scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default; z-index: 40; }
  .panel { position: absolute; right: 0; top: calc(100% + 6px); z-index: 41;
           min-width: min(340px, calc(100vw - 16px));
           max-width: min(420px, calc(100vw - 16px));
           max-height: 60vh; overflow-y: auto;
           display: flex; flex-direction: column; gap: 2px;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 8px; padding: 6px;
           box-shadow: 0 8px 28px rgba(0, 0, 0, .45); }
  .panel.alignleft { right: auto; left: 0; }
  .row { display: flex; align-items: center; gap: 10px; padding: 7px 8px;
         border-radius: 6px; }
  .row:hover { background: color-mix(in srgb, var(--text) 4%, transparent); }
  .tick { width: 3px; align-self: stretch; border-radius: 1.5px; flex: none; }
  .text { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .title { font-size: 12.5px; font-weight: 600; overflow: hidden;
           text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 11px; color: var(--muted); overflow: hidden;
          text-overflow: ellipsis; white-space: nowrap; }
  .rowerr { font-size: 11px; color: var(--error); white-space: normal; }
  .rsvp { display: flex; gap: 4px; flex: none; }
  .rsvp button { font: inherit; font-size: 11.5px; cursor: pointer; border: 0;
                 border-radius: 6px; padding: 4px 9px; color: var(--text);
                 background: color-mix(in srgb, var(--text) 6%, transparent); }
  .rsvp button:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 22%, transparent); }
  .rsvp button:disabled { opacity: .5; cursor: default; }
  /* The declines section row: label left, Dismiss all right. The hairline
     only when invitations sit above it — a divider with nothing above is a
     stray line. */
  .sect { display: flex; align-items: center; justify-content: space-between;
          font-size: 10.5px; color: var(--muted); letter-spacing: .05em;
          margin: 0; padding: 2px 8px 0; }
  .sect.joined { margin-top: 4px; border-top: 1px solid var(--hairline); }
  .ackall { font: inherit; font-size: 10.5px; letter-spacing: .05em;
            cursor: pointer; border: 0; border-radius: 5px; padding: 2px 7px;
            color: var(--muted); background: none; }
  .ackall:hover { color: var(--text);
                  background: color-mix(in srgb, var(--text) 8%, transparent); }
  /* The ×: acknowledgement, not deletion — quiet until hovered. */
  .ack { font: inherit; font-size: 14px; line-height: 1; cursor: pointer;
         border: 0; border-radius: 6px; padding: 4px 8px; flex: none;
         color: var(--muted); background: none; }
  .ack:hover { color: var(--text);
               background: color-mix(in srgb, var(--text) 8%, transparent); }
</style>
