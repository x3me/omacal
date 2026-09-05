<!-- ui/src/lib/EventPopover.svelte -->
<script lang="ts">
  import { meetingUrl } from './location';
  import { openConference } from './api';
  import { clockFormat } from './clock.svelte';
  import { formatClock } from './timefmt';
  import { onMount, tick } from 'svelte';
  import { escapeCloses } from './dismiss.svelte';
  import { placePopover, type Rect } from './position';
  import { descriptionSegments } from './sanitize';
  import { occurrenceDate, ruleInWords } from './eventform';
  import { isMachineAddress } from './organizer';
  import { respondToEvent, type Attendee, type EventDetail } from './eventdetail';
  import { focusInitialChoice, handleChoiceKey } from './choicefocus';
  import { EVENT_SHORTCUT_LIST, type EventShortcutId, shortcutKeyFor } from './shortcuts';

  let {
    detail,
    anchor,
    occurrenceStartMs,
    occurrenceEndMs,
    onclose,
    onresponded,
    onedit,
    ondelete,
    oncopy,
  }: {
    detail: EventDetail;
    anchor: Rect;
    /** The clicked block's own `start_ms` — see `eventdetail.ts`'s
     *  `respondToEvent` doc comment for why this can never be
     *  `detail.start_ms`. Threaded through from `WeekGrid` alongside
     *  `anchor`, both sourced from the same `UiEvent`. */
    occurrenceStartMs: number;
    /** The clicked block's own `end_ms`, and here for the same reason its
     *  start is: `detail.end_ms` is the **master's** for a series, so a
     *  standup shown from `detail` reads the DTSTART's clock — an hour out
     *  from the block on the grid beside it for every occurrence on the far
     *  side of a daylight-saving transition, and on the master's date for
     *  every occurrence at all. Both call sites already hold this value; they
     *  build an `{ detail, startMs, endMs }` occurrence out of it for Edit and
     *  Delete. Required, not optional, so a third call site cannot quietly
     *  reintroduce the master's times by omission. */
    occurrenceEndMs: number;
    onclose: () => void;
    /** Told the response that just landed, so the caller can restyle the
     *  block that was clicked without waiting for the next sync — the
     *  backend deliberately leaves `detail` itself unchanged after a "this
     *  one" RSVP against a bare master (see `respond_to_event`'s own
     *  comment), so nothing here can be read back off `detail`. Required,
     *  not optional: it is the only channel the clicked block ever gets
     *  told a response landed, and a caller that silently omits it gets a
     *  grid that never restyles — the exact staleness this task exists to
     *  close. */
    onresponded: (response: 'accepted' | 'tentative' | 'declined') => void;
    /** Asked to open the event form on the block this popover was opened for.
     *  This component deliberately neither owns the form nor calls
     *  `updateEvent` itself: both need the *clicked block's* own `start_ms`,
     *  which lives one layer out with the `UiEvent` it came from, and
     *  `detail.start_ms` — the only start this component has — is precisely
     *  the value that must never reach a write. See `eventdetail.ts`.
     *
     *  Required, not optional, for the same reason `onresponded` is: a caller
     *  that quietly omitted it would render an Edit button that does nothing. */
    onedit: () => void;
    /** Asked to confirm and perform a delete, for the same block and for the
     *  same reason `onedit` is asked rather than done here. Nothing is deleted
     *  by clicking this: the caller owns the confirmation, which has three
     *  scopes, a guest count and no undo behind it. */
    ondelete: () => void;
    /** Ctrl+C (⌘C) landed on the popover itself rather than one of its
     *  focusable copy fields: the caller should remember this occurrence as
     *  what Ctrl+V will paste. Living here rather
     *  than in `App`'s key handler because *here* is the only place that
     *  knows a popover is open at all for Day and Week — `WeekGrid` owns that
     *  popover end-to-end, and `App` never sees its occurrence. One listener
     *  in the shared component covers every view. Required, like `onedit`: a
     *  caller that omitted it would swallow the chord and copy nothing. */
    oncopy: () => void;
  } = $props();

  const segments = $derived(descriptionSegments(detail.description));

  const hhmm = (ms: number) => formatClock(ms, clockFormat());

  const DAY_FORMAT: Intl.DateTimeFormatOptions =
    { weekday: 'short', month: 'short', day: 'numeric' };

  /** The day an **instant** falls on, read in the browser's zone — right for a
   *  timed event, whose start genuinely is an instant and whose reader is
   *  whoever is looking at the screen. */
  const dayOfInstant = (ms: number) => new Date(ms).toLocaleDateString([], DAY_FORMAT);

  /** The day a `yyyy-mm-dd` names, read in **no** zone.
   *
   *  Built through `Date.UTC` and rendered with `timeZone: 'UTC'` — both
   *  halves, and the second is the one that is easy to forget: a UTC instant
   *  formatted without it is put straight back through the browser's zone and
   *  lands a day early for every user west of it. The same pair, for the same
   *  reason, as `untilInWords` in `eventform.ts`.
   *
   *  An all-day event has no instant to read. The one the store holds is
   *  midnight in the **calendar's** zone, and this browser does not know what
   *  that zone is; reading it here is how this line used to show "Sun, Aug 9"
   *  for a trip the form beside it opened on `2026-08-10`. */
  const dayOfDate = (date: string) => {
    const [y, m, d] = date.split('-').map(Number);
    return new Date(Date.UTC(y, m - 1, d)).toLocaleDateString([], { ...DAY_FORMAT, timeZone: 'UTC' });
  };

  /**
   * The day this popover is about — **the clicked block's**, never the row's.
   *
   * Both arms take the occurrence, and the all-day arm takes it as a *date*:
   * `occurrenceDate` moves the detail's own `start_date` onto the clicked block
   * by the whole-day distance between two instants on the same side of the same
   * event. That is the form's answer to the same question, from the same
   * function, so the two cannot drift apart on screen again — see its comment in
   * `eventform.ts` for why the subtraction has no zone in it.
   *
   * Only the first day, for a multi-day all-day event, which is what this line
   * has always shown. Widening it to a range is a display question, not a
   * boundary one, and is not this task's.
   *
   * One consequence worth stating, since `detail` is a live prop and these two
   * are not: the after-paint `refresh_event` no longer moves this line. It
   * never legitimately could — for a series `detail` carries the *master's*
   * instants, so a refresh moved the popover onto a day the block beneath it
   * was not on — and a genuine time change reaches the screen the way every
   * other one does, when the grid reloads.
   */
  const day = $derived(
    detail.is_all_day
      ? dayOfDate(occurrenceDate(detail.start_date, detail.start_ms, occurrenceStartMs))
      : dayOfInstant(occurrenceStartMs),
  );
  const whenLine = $derived(
    detail.is_all_day ? day : `${day} · ${hhmm(occurrenceStartMs)}–${hhmm(occurrenceEndMs)}`,
  );

  // A neutral default so `.pop` renders (and so `offsetWidth`/`offsetHeight`
  // are measurable at all) before `onMount` below can place it for real.
  // `anchor` never changes for this component's lifetime — WeekGrid mounts a
  // fresh EventPopover per open rather than reusing one across clicks — so a
  // one-time placement in `onMount` is all this ever needs; there's nothing
  // to keep tracking after that.
  let pos = $state<{ top: number; left: number }>({ top: 0, left: 0 });
  let panelEl: HTMLDivElement | undefined = $state();

  onMount(() => {
    if (!panelEl) return;
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    pos = placePopover(anchor, { width: panelEl.offsetWidth, height: panelEl.offsetHeight }, viewport);
    // `role="dialog"` + `aria-modal` says "you are in here now"; without
    // moving focus in, Tab continues from whatever the click left focused and
    // walks straight out into the grid behind the scrim — a screen reader
    // then reads a week of blocks the scrim has already made unclickable.
    // `tabindex="-1"` on the panel is what makes it a legal focus target
    // without adding it to the tab order itself.
    panelEl.focus();
  });

  // Optimistic RSVP. `chosen` is `null` until the user picks something in
  // this session, in which case it — not `detail.self_response` — is what
  // the three buttons render against; see `onresponded` above for why
  // `detail` cannot be trusted to catch up on its own.
  let chosen = $state<'accepted' | 'tentative' | 'declined' | null>(null);
  /** A response clicked on a recurring event, waiting for its scope. The
   *  question is only relevant once the user touches something (2026-08-11,
   *  by request) — at rest the popover says the cadence instead, and a
   *  one-off answers in one click with no question at all. */
  let pending = $state<'accepted' | 'tentative' | 'declined' | null>(null);

  /** What a reader learns at a glance instead of the old always-on radio:
   *  that this is one occurrence of a series, and on what cadence. */
  const REPEAT_WORDS: Record<string, string> = {
    daily: 'daily', weekdays: 'every weekday (Mon–Fri)', weekly: 'weekly',
    monthly: 'monthly', yearly: 'yearly',
  };
  const cadence = $derived.by(() => {
    if (!detail.is_recurring) return null;
    const word = REPEAT_WORDS[detail.repeat];
    if (word) return `Repeats ${word}`;
    // A custom rule in its own words; an exception row (recurring by
    // parentage, carrying no rule of its own) still says what it is.
    if (detail.recurrence) return ruleInWords(detail.recurrence) ?? 'Repeats on a custom schedule';
    return 'Part of a repeating series';
  });

  /**
   * The location line, or `null` when it would only repeat itself: Google
   * routinely mints events whose location *is* the meeting URL while the
   * description spells the same URL out, and rendered verbatim the popover
   * said it twice (seen live, 2026-08-11). Only a URL echo is dropped — a
   * room stays, and so does a URL the description does not carry; deciding
   * anything cleverer about locations is `location.ts`'s documented
   * non-goal, not something to smuggle in here.
   */
  const locationShown = $derived.by(() => {
    const loc = detail.location?.trim();
    if (!loc) return null;
    if (/^https?:\/\//i.test(loc) && (detail.description ?? '').includes(loc)) return null;
    // The same echo rule, one source further along: when the location *is*
    // the link the Join button now offers, printing it again underneath that
    // button says the same thing twice — and the raw form is the one that
    // reads as `https://us02we…`, which is what `location.ts` exists to stop.
    if (joinUrl && loc === joinUrl) return null;
    return loc;
  });

  /**
   * The meeting to join: Google's structured conference data if it is there,
   * else a recognised link in `location`, else one in the description text.
   *
   * One control, not two: an event carries at most one meeting, and a second
   * Join button for the second place a link can hide would make the popover
   * argue with itself about which is the real one. Google's own field wins
   * because it is the only one that cannot be a coincidence; `location`
   * before `description` matches `open_conference`'s own order
   * (`conference_join_url` in `upcoming.rs`), which is what actually runs
   * when this is clicked — this derivation only has to agree with it, not
   * decide anything, since it drives the displayed `href` and the
   * `locationShown` echo check below, not the click itself.
   */
  const joinUrl = $derived(
    detail.conference_uri ?? meetingUrl(detail.location) ?? meetingUrl(detail.description),
  );

  async function ask(response: 'accepted' | 'tentative' | 'declined', e: MouseEvent) {
    if (!detail.is_recurring) {
      respond(response, 'this', e);
      return;
    }
    pending = response;
    await tick();
    focusInitialChoice(panelEl);
  }
  /** A `Set`, mirroring `CalendarPopover`: today there is only ever one RSVP
   *  target, so it never holds more than one entry, but a plain boolean
   *  would re-invent the same "which action is this even about" ambiguity
   *  a single id caused there — a Set stays correct if that ever changes. */
  let busy = $state<Set<'accepted' | 'tentative' | 'declined'>>(new Set());
  let note = $state<{ text: string; kind: 'info' | 'error' } | null>(null);

  const shown = $derived(chosen ?? detail.self_response);

  // For every non-recurring event, and for `scope: 'all'`, the backend
  // *does* write back and returns an `EventDetail` whose `attendees` carry
  // the new response — only a "this one" RSVP against a bare master skips
  // it (see `respond_to_event`'s own comment, and `onresponded` above). When
  // it doesn't skip, adopting the fresh list is what keeps the guest list's
  // own "you" row from reading `needsAction` while the buttons above it
  // already say otherwise. `chosen` still drives the buttons regardless —
  // this only ever affects the guest list.
  let freshAttendees = $state<Attendee[] | null>(null);
  const shownAttendees = $derived(freshAttendees ?? detail.attendees);

  // `?` means MAYBE — the letter Google and Outlook both use for it, and the
  // reading everyone brought to it anyway (2026-08-10, by request; it
  // previously meant no-reply and the tilde it displaced read as nothing at
  // all). "No reply yet" is the empty ring: nothing there, honestly. Google
  // can send a `responseStatus` we do not model, so both tables are read with
  // a `needsAction` fallback rather than indexed blindly — an unknown status
  // reads as "hasn't answered", which the empty ring now *is*; the hover word
  // beside it says so in words.
  const MARK: Record<string, string> = {
    accepted: '✓',
    declined: '✕',
    tentative: '?',
    needsAction: '',
  };
  const STATUS_WORD: Record<string, string> = {
    accepted: 'accepted',
    declined: 'declined',
    tentative: 'maybe',
    needsAction: 'no reply yet',
  };

  async function respond(
    response: 'accepted' | 'tentative' | 'declined',
    scope: 'this' | 'all',
    e: MouseEvent,
  ) {
    const btn = e.currentTarget as HTMLButtonElement;
    pending = null;
    const previous = chosen;
    // `detail` is a live prop, not a snapshot: WeekGrid sets its own
    // `detail` to `null` the moment this popover closes (a scrim click,
    // Escape, or another block opening), and that closure keeps running
    // after the `await` below regardless. Reading `detail.attendees` again
    // afterward would dereference null exactly when the popover has closed
    // mid-flight — precisely the case `onresponded` below exists to still
    // handle correctly. Capture what's needed from `detail` now, before
    // anything async, and never touch the prop again in this function.
    const attendeesBaseline = JSON.stringify(detail.attendees);
    const id = detail.id;
    chosen = response;
    busy = new Set([response]);
    note = null;
    try {
      const fresh = await respondToEvent(id, response, scope, occurrenceStartMs);
      if (JSON.stringify(fresh.attendees) !== attendeesBaseline) {
        freshAttendees = fresh.attendees;
      }
      onresponded(response);
    } catch (err) {
      chosen = previous;
      note = { text: String(err), kind: 'error' };
    } finally {
      busy = new Set();
      // Disabling a focused button mid-submit (just above) drops focus to
      // <body> the instant the attribute lands, before this handler even
      // gets to `finally` — the same failure `CalendarPopover`'s own
      // `toggleShown`/`toggleSync` guard against, with the same fix: reclaim
      // it once `disabled` is actually gone from the DOM.
      await tick();
      // An ask-row button (see `pending`) unmounts the moment the answer is
      // sent; focus then falls back to the panel so a keyboard user is not
      // stranded on <body>.
      if (btn.isConnected) btn.focus();
      else panelEl?.focus();
    }
  }

  // A window-level listener, not one on the panel: a disabled RSVP button
  // mid-submit (see `busy` above) can drop focus to `<body>`, exactly the
  // failure `CalendarPopover`'s own comment documents — nothing short of
  // `window` hears Escape from there.
  escapeCloses(() => true, () => onclose());

  /** What Ctrl+C puts on the OS clipboard, so a copy is also a copy in the
   *  ordinary sense — pasteable into a chat as text. The same formatters the
   *  panel renders with, so the text says what the screen says. */
  const clipboardLine = () => {
    return [detail.title ?? '(no title)', whenLine, detail.location ?? ''].filter(Boolean).join('\n');
  };

  function copyField(value: string, label: string) {
    try { navigator.clipboard?.writeText(value).catch(() => {}); } catch { /* no clipboard here */ }
    note = { text: `Copied ${label}`, kind: 'info' };
  }

  // Ctrl+C — ⌘C on a Mac — copies the focused field when it has one, otherwise
  // the event this popover is about. On `window` for the same reason Escape
  // is, and with one yield: a real text selection in the panel keeps native
  // copy, because taking the chord away from selected text would break the
  // older meaning of the key to serve the newer one. Shift and Alt variants
  // pass through untouched — Ctrl+Shift+C is a devtools chord in the webview.
  function onCopyKey(e: KeyboardEvent) {
    if (e.key.toLowerCase() !== 'c' || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
    if (document.getSelection()?.toString()) return;
    e.preventDefault();
    const field = (e.target as HTMLElement | null)?.closest<HTMLElement>('[data-copy-value]');
    if (field) {
      copyField(field.dataset.copyValue ?? '', field.dataset.copyLabel ?? 'field');
      return;
    }
    // Fire-and-forget: the buffer `oncopy` fills is what paste reads, and a
    // webview denying clipboard access must not turn the copy into an error.
    try { navigator.clipboard?.writeText(clipboardLine()).catch(() => {}); } catch { /* no clipboard here */ }
    note = { text: 'Copied — Ctrl+V pastes it as a new event', kind: 'info' };
    oncopy();
  }

  function clickResponse(response: 'accepted' | 'tentative' | 'declined') {
    const button = panelEl?.querySelector<HTMLButtonElement>(
      `[data-event-response="${response}"]`,
    );
    if (!button || button.disabled) return false;
    // Use the real control so one-off responses, recurring scope questions,
    // optimistic state and focus restoration remain the click path's job.
    button.click();
    return true;
  }

  const EVENT_SHORTCUT_ACTIONS: Record<EventShortcutId, () => boolean> = {
    edit: () => {
      if (!detail.can_edit) return false;
      onedit();
      return true;
    },
    delete: () => {
      if (!detail.can_edit) return false;
      ondelete();
      return true;
    },
    yes: () => clickResponse('accepted'),
    maybe: () => clickResponse('tentative'),
    no: () => clickResponse('declined'),
    join: () => {
      if (!joinUrl) return false;
      void openConference(detail.id);
      return true;
    },
  };

  /** An open event owns these bare keys. Focused controls keep Enter's native
   * meaning — a focused Join link joins once and an RSVP button answers —
   * while Enter on the panel itself joins immediately. The choice handler
   * (arrows and Enter inside a scope question) gets the key first: a key it
   * handled must not also dispatch an event action behind the question. */
  function onPopoverKey(e: KeyboardEvent) {
    onCopyKey(e);
    if (e.defaultPrevented) return;
    if (panelEl && handleChoiceKey(panelEl, e)) return;
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;

    // Read by the physical key on a layout that writes its own script, as
    // App's own dispatcher is — see `shortcutKeyFor` (#38).
    const key = shortcutKeyFor(e.key, e.code);
    const hit = EVENT_SHORTCUT_LIST.find((s) => s.key === key || s.aliases?.includes(key));
    if (!hit) return;

    if (hit.id === 'join') {
      const target = e.target as Element | null;
      if (target && target !== panelEl
          && target.closest('a[href], button, input, select, textarea, [contenteditable="true"]')) {
        return;
      }
    }

    if (EVENT_SHORTCUT_ACTIONS[hit.id]()) e.preventDefault();
  }
</script>

<svelte:window onkeydown={onPopoverKey} />

<!-- A sibling of `.pop`, not a wrapper around it, so a click inside the
     panel — the guest list included — never reaches this button. -->
<button class="scrim" aria-label="Close" onclick={onclose}></button>

<!-- `role="dialog"` rather than `CalendarPopover`'s `role="group"`: this panel
     has a guest list, external links and three buttons, and the scrim behind
     it makes everything else unclickable — that is a dialog, and claiming it
     obliges `aria-modal` and the focus move in `onMount`. -->
<div
  class="pop"
  bind:this={panelEl}
  role="dialog"
  aria-modal="true"
  tabindex="-1"
  aria-label={detail.title ?? '(no title)'}
  style="top:{pos.top}px; left:{pos.left}px"
>
  <h2 aria-label={detail.title ?? '(no title)'}><button type="button" class="copyfield"
      aria-label="Copy title" data-copy-label="title"
      data-copy-value={detail.title ?? '(no title)'}
      onclick={() => copyField(detail.title ?? '(no title)', 'title')}
      >{detail.title ?? '(no title)'}</button></h2>
  <p class="when"><button type="button" class="copyfield" aria-label="Copy date and time"
     data-copy-label="date and time"
     data-copy-value={whenLine} onclick={() => copyField(whenLine, 'date and time')}
     >{whenLine}</button></p>
  {#if cadence}<p class="cadence"><button type="button" class="copyfield" aria-label="Copy repeat schedule"
                       data-copy-label="repeat schedule" data-copy-value={cadence}
                       onclick={() => copyField(cadence!, 'repeat schedule')}
                       >{cadence}</button></p>{/if}

  {#if segments.length}
    <div class="desc">
      <button type="button" class="copydesc" aria-label="Copy description"
              data-copy-label="description"
              data-copy-value={detail.description ?? ''}
              onclick={() => copyField(detail.description ?? '', 'description')}>Copy</button>
      <!-- `href` and the copied value are the destination; the text between
           the tags is the label. For a bare URL those are the same string,
           which is every link this popover drew before descriptions could
           carry an anchor with words in it. -->
      <p>{#each segments as s}{#if s.kind === 'link'}<a
              href={s.href} target="_blank" rel="noopener noreferrer"
              data-copy-label="link" data-copy-value={s.href}>{s.value}</a
          >{:else}{s.value}{/if}{/each}</p>
    </div>
  {/if}

  {#if locationShown}<p class="loc"><button type="button" class="copyfield" aria-label="Copy location"
                            data-copy-label="location" data-copy-value={locationShown}
                            onclick={() => copyField(locationShown!, 'location')}
                            >{locationShown}</button></p>{/if}
  {#if joinUrl}
    <!-- The `href` stays for what an anchor gives away free — copy-link,
         middle-click, the status-bar preview — but a plain left click is
         taken over and sent backend-side: the webview's own `target="_blank"`
         spawn hands the browser this process's AppImage environment and
         crashes it (issue #1, `browser::open_external`). The backend
         re-derives the URL from its store rather than trusting this one. -->
    <a class="conf" href={joinUrl} target="_blank" rel="noopener noreferrer"
       data-copy-label="meeting link" data-copy-value={joinUrl}
       onclick={(e) => { e.preventDefault(); void openConference(detail.id); }}>Join video call</a>
  {/if}
  <!-- Suppressed for the addresses Google mints for shared calendars and
       meeting rooms: "Organized by" followed by forty hex characters is worse
       than nothing, and there is no name to fall back to — `EventDetail`
       carries no `organizer.displayName`. See `organizer.ts`. -->
  {#if detail.organizer_email && !isMachineAddress(detail.organizer_email)}
    <p class="organizer"><button type="button" class="copyfield" aria-label="Copy organizer email"
       data-copy-label="organizer email" data-copy-value={detail.organizer_email}
       onclick={() => copyField(detail.organizer_email!, 'organizer email')}
       >Organized by {detail.organizer_email}</button></p>
  {/if}

  {#if shownAttendees.length}
    {@const declined = shownAttendees.filter((a) => a.response_status === 'declined' && !a.is_self)}
    {@const others = shownAttendees.filter((a) => !a.is_self)}
    <!-- The tally, above the list, and only when somebody has said no: the
         list itself already tells you who, one line each, but on a long
         guest list "how many are out" is a counting exercise (2026-09-04).
         Silent when nobody has declined — a row saying "0 declined" would
         be furniture on every event in the app. -->
    {#if declined.length}
      <p class="tally" class:none={declined.length === others.length}>
        {declined.length === others.length
          ? (others.length === 1 ? 'The other guest declined' : 'Every guest declined')
          : `${declined.length} of ${others.length} guests declined`}
      </p>
    {/if}
    <div class="guests">
      {#each shownAttendees as a}
        <!-- `title` says the word on hover: a tilde in a 13px ring was read as
             "no idea what that is" (Omarchy, 2026-08-10), and the sighted had
             no equivalent of `.sr` to fall back on. -->
        <button
          type="button"
          class="guest copyfield {a.response_status}"
          data-copy-label="guest email"
          data-copy-value={a.email}
          onclick={() => copyField(a.email, 'guest email')}
          aria-label="Copy guest email for {a.display_name ?? a.email}. {a.response_status === 'needsAction'
            ? 'Awaiting response'
            : STATUS_WORD[a.response_status] ?? 'Awaiting response'}"
          title={STATUS_WORD[a.response_status] ?? STATUS_WORD.needsAction}
        >
          <!-- The glyph carries the status, not the colour. This app takes its
               palette from the Omarchy theme, which offers no semantic green or
               red — and at 11px a tick reads faster than a hue anyway. Colour
               only reinforces: answered is full strength, everything else is
               muted. `aria-hidden` because the word follows it in `.sr`. -->
          <i class="mark" aria-hidden="true">{MARK[a.response_status] ?? MARK.needsAction}</i>
          <span class="who">{a.display_name ?? a.email}{a.is_self ? ' (you)' : ''}</span>
          <span class="sr">{STATUS_WORD[a.response_status] ?? STATUS_WORD.needsAction}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if detail.can_respond}
    {#if pending}
      <!-- The scope, asked exactly when it means something: a response has
           been chosen and this event repeats. `.scope` keeps its class so the
           one-off specs ("no scope controls at all") keep their meaning. -->
      <div class="scope ask" role="group" aria-label="Apply to" data-choice-group>
        <span>{STATUS_WORD[pending]} —</span>
        <button type="button" data-choice data-initial-choice onclick={(e) => respond(pending!, 'this', e)}>This one</button>
        <button type="button" data-choice onclick={(e) => respond(pending!, 'all', e)}>All of them</button>
        <button type="button" class="back" aria-label="Cancel" onclick={() => (pending = null)}>✕</button>
      </div>
    {/if}
    <div class="rsvp">
      <button data-event-response="accepted" class:chosen={shown === 'accepted'} disabled={busy.size > 0 || pending !== null} onclick={(e) => ask('accepted', e)}
        >Yes</button
      >
      <button data-event-response="tentative" class:chosen={shown === 'tentative'} disabled={busy.size > 0 || pending !== null} onclick={(e) => ask('tentative', e)}
        >Maybe</button
      >
      <button data-event-response="declined" class:chosen={shown === 'declined'} disabled={busy.size > 0 || pending !== null} onclick={(e) => ask('declined', e)}
        >No</button
      >
    </div>
  {/if}

  <!-- Only when the backend says this account may write to the calendar the
       event is on (`can_edit`, from the same `access_role` column
       `create_impl`/`update_impl` check server-side). Offering either control
       on a subscribed holiday calendar would produce a Save — or worse, a
       Delete confirmation — the server could only refuse, after the user had
       already decided to go through with it. -->
  {#if detail.can_edit}
    <div class="own">
      <button onclick={onedit}>Edit</button>
      <button onclick={ondelete}>Delete</button>
    </div>
  {/if}

  {#if note}<p class="note" class:err={note.kind === 'error'}>{note.text}</p>{/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default; z-index: 40; }

  /* `overflow-wrap: anywhere` is on the *panel*, not on the field that
     reported the bug, and that is the point of it.

     What was seen: a full-width horizontal scrollbar along the bottom of the
     popover, from an organizer address of the form
     `c_<40 hex>@group.calendar.google.com`. `.organizer` had colour and size
     and no wrap handling, so the unbreakable token pushed past 320px — and
     because `overflow-y: auto` makes the other axis `auto` too rather than
     `visible`, the panel answered with a scroller.

     Suppressing that address (see the markup) fixes the case that was reported
     and none of the others. Measured against the `unbreakable` fixture with
     this declaration removed: the panel's 2008px of scroll width comes from
     the *title* (1994px) and would come from the location (1589px) and the
     organizer (1658px) on their own — three block-level fields that had no
     wrap handling of their own. `.desc` was never a contributor; it carries
     its own `word-break: break-word`. One declaration here covers all of them
     and every field the panel gains later.

     Two fields that look like they belong on that list and do not, both
     checked rather than assumed: `.conf`'s *text* is the fixed label "Join
     video call" — the long URI is only ever in its `href` — and `.who` clips
     itself with `overflow: hidden` and an ellipsis, so its own 1684px never
     reaches the panel's scroll width.

     `anywhere` rather than `break-word` is the stronger of the two: only
     `anywhere` also counts the break when working out a box's *minimum* width,
     which is what a shrink-to-fit box would size itself against. Worth being
     honest about, though — **this panel has no such box today, and no test
     here distinguishes the two.** Swapping `anywhere` for `break-word` keeps
     every spec green (measured). It is chosen as the safer default, not
     because something currently needs it. */
  .pop { position: fixed; z-index: 41; width: 320px; max-height: 70vh; overflow-y: auto;
         background: var(--surface); border: 1px solid var(--hairline);
         border-radius: 8px; padding: 12px 14px; box-shadow: 0 8px 28px rgba(0, 0, 0, .45);
         font-size: 12px; overflow-wrap: anywhere; }
  /* The panel is focused on mount to contain the tab order, not because it is
     itself operable — a ring around the whole popover would only be noise.
     The controls inside keep theirs. */
  .pop:focus { outline: none; }
  .copyfield { appearance: none; -webkit-appearance: none; font: inherit; color: inherit;
               background: none; border: 0; padding: 0; margin: 0; text-align: left;
               cursor: copy; }
  .copyfield:focus-visible, .copydesc:focus-visible {
    outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 3px;
  }

  h2 { font-size: 14px; font-weight: 600; margin: 0 0 4px; letter-spacing: -.01em; }
  .when { color: var(--muted); font-size: 11px; margin: 0 0 8px; }

  .desc { position: relative; white-space: pre-wrap; word-break: break-word; line-height: 1.5;
          margin: 0 0 10px; padding-right: 38px; }
  .desc p { margin: 0; }
  .copydesc { appearance: none; -webkit-appearance: none; position: absolute; top: 0; right: 0;
              font: inherit; font-size: 9.5px; color: var(--muted); cursor: copy;
              border: 0; border-radius: 3px; background: none; padding: 1px 3px; }
  .copydesc:hover { color: var(--text); }
  .desc a { color: var(--accent); }

  .loc, .organizer { color: var(--muted); font-size: 11px; margin: 0 0 4px; }
  .conf { display: inline-block; color: var(--accent); font-size: 11px;
          text-decoration: none; margin: 0 0 8px; }
  .conf:hover { text-decoration: underline; }

  /* Sits with the guest list rather than with the detail lines above it,
     because it is a reading of that list and not another fact about the
     event. Emphasised only when every guest is out, which is the state
     worth a second glance. */
  .tally { margin: 8px 0 -2px; font-size: 11px; color: var(--muted); }
  .tally.none { color: var(--text); font-weight: 600; }
  .guests { margin: 8px 0; display: flex; flex-direction: column; gap: 3px; }
  .guest { font-size: 11px; padding: 1px 0; display: flex; align-items: center; gap: 6px; }
  .who { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* `currentColor` so the ring always matches the name beside it — one rule
     sets both, and a status can never end up with a ring in one strength and
     a name in another. */
  .mark {
    flex: none; width: 13px; height: 13px; border-radius: 50%;
    border: 1px solid currentColor; color: inherit;
    display: grid; place-items: center;
    font-size: 8px; font-style: normal; line-height: 1;
  }

  /* Answered-yes is the only row at full strength; everything else recedes.
     That inverts the old styling, where four states shared two greys and the
     difference between "coming" and "hasn't replied" was an opacity of .8. */
  .guest.accepted { color: var(--text); }
  .guest.tentative,
  .guest.needsAction { color: var(--muted); }
  .guest.declined { color: var(--muted); }
  /* Strike the name, never the ring — a struck-through ✕ is unreadable. */
  .guest.declined .who { text-decoration: line-through; }

  /* Visually hidden, still announced: the ring is decorative to a screen
     reader, so the status has to exist as text somewhere. */
  .sr {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip-path: inset(50%); white-space: nowrap;
  }

  .cadence { color: var(--muted); font-size: 11px; margin: -4px 0 8px; }
  .scope { display: flex; gap: 8px; align-items: center; font-size: 11px;
           margin: 8px 0 6px; color: var(--muted); }
  .scope button { font: inherit; font-size: 11px; color: var(--text); cursor: pointer;
                  background: color-mix(in srgb, var(--text) 6%, transparent);
                  border: 1px solid var(--hairline); border-radius: 5px; padding: 3px 8px; }
  .scope .back { background: none; border: 0; color: var(--muted); padding: 0 2px; }
  .scope .back:hover { color: var(--text); }

  .rsvp { display: flex; gap: 6px; margin-top: 6px; }
  .rsvp button { flex: 1; font: inherit; font-size: 11.5px; cursor: pointer;
                 background: color-mix(in srgb, var(--text) 6%, transparent);
                 color: var(--text); border: 1px solid var(--hairline);
                 border-radius: 6px; padding: 5px 0; }
  .rsvp button.chosen { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .rsvp button:disabled { opacity: .6; cursor: default; }

  /* Quieter than the RSVP row above it, and separated from it by a hairline:
     answering an invitation is what this panel is mostly for, and these two
     are the pair that change the event for everybody. */
  .own { display: flex; gap: 6px; margin-top: 8px; padding-top: 8px;
         border-top: 1px solid var(--hairline); }
  .own button { flex: 1; font: inherit; font-size: 11.5px; cursor: pointer;
                background: none; color: var(--muted);
                border: 1px solid var(--hairline); border-radius: 6px; padding: 5px 0; }
  .own button:hover { color: var(--text); }

  .note { font-size: 10.5px; color: var(--muted); line-height: 1.4;
          margin: 8px 0 0; padding: 6px 8px; border-radius: 5px;
          background: color-mix(in srgb, var(--text) 6%, transparent); }
  .note.err { color: var(--error); background: color-mix(in srgb, var(--error) 9%, transparent); }
</style>
