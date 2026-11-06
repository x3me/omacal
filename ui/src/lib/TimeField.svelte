<!-- ui/src/lib/TimeField.svelte
     A time field with a list we own.

     **The one time field** (Plamen, 2026-09-17, #111): the task editor and
     the event form both use it, and any change to how a time is picked is
     made here so the two cannot drift. Replaced `<input type="time">` in the
     task editor first (by request with a screenshot). The native field renders the *system
     locale's* clock rather than the app's (`EventForm` left it for that
     reason on 2026-08-25), draws its empty state as a greyed "12:30 PM" that
     reads as a chosen time, and is edited a segment at a time with no list
     to pick from. This is the event form's typed field — any spelling
     `parseClock` accepts, shown in the app's own clock — with a list of the
     day's half hours under it, opened on the likely answer.

     The value is `HH:MM`, or empty for no time, which is what every caller
     already stores. -->
<script lang="ts">
  import { tick } from 'svelte';
  import { clockFormat } from './clock.svelte';
  import { displayClock, parseClock } from './timefmt';
  import { openingSlot, slotAtOrBefore, stepSlot, timeSlots } from './timepicker';
  import { dropdown } from './dropdown';

  let {
    label,
    value = $bindable(''),
    disabled = false,
    open = $bindable(false),
    invalid = $bindable(false),
    required = false,
    live = false,
    flagged = false,
    note = null,
    isToday = false,
    placeholder = 'Add time',
    onchange,
  }: {
    label: string;
    value?: string;
    disabled?: boolean;
    /** Bindable for `DateField`'s reason: an owner that takes Escape for
     *  its own layer must be able to see this one is open first. */
    open?: boolean;
    /** Something that is not a time was left in the field. It stays there,
     *  marked, rather than being thrown away — the hand that typed "3pmm"
     *  wants to fix one letter, not start again — and bindable, so the owner
     *  can refuse to save around it. */
    invalid?: boolean;
    /** A time must be given: no clear button, and an emptied field is
     *  marked like any other entry that is not a time. An event's start and
     *  end are required; a task's due time is not. */
    required?: boolean;
    /** Take a finished-looking typed time as the value on each keystroke —
     *  two-digit minutes after a separator, or a meridiem said — so what the
     *  form draws on the grid follows the typing. Committing "1" as 01:00 on
     *  the way to "12:30" would walk it through nonsense, hence "finished". */
    live?: boolean;
    /** Marked by the owner for a reason of its own: the event form's refused
     *  save, which names the field it is about. */
    flagged?: boolean;
    /** A few words beside a row of the list, or `null` — the End field's
     *  duration from the start. */
    note?: ((slot: string) => string | null) | null;
    /** Whether the day this time belongs to is today, so an empty field
     *  opens its list on the next half hour rather than on 09:00. */
    isToday?: boolean;
    placeholder?: string;
    /** Every committed change — a pick, a typed time, a clear. */
    onchange?: (v: string) => void;
  } = $props();

  const slots = timeSlots(30);
  const uid = `tf${Math.random().toString(36).slice(2, 8)}`;
  let input: HTMLInputElement | undefined = $state();
  let list: HTMLDivElement | undefined = $state();
  let field: HTMLSpanElement | undefined = $state();

  /** What the field shows: the value in the app's clock, or what is being
   *  typed over it. */
  let text = $state('');
  let typing = $state(false);
  /** The row the keyboard is on. */
  let active = $state<string | null>(null);

  $effect(() => {
    const shown = value ? displayClock(value, clockFormat()) : '';
    if (!typing) text = shown;
  });

  async function reveal(block: ScrollLogicalPosition) {
    await tick();
    list?.querySelector('[data-active]')?.scrollIntoView({ block });
  }

  function show() {
    if (disabled) return;
    active = openingSlot(value, slots, { nowMs: Date.now(), isToday });
    open = true;
    void reveal('center');
  }

  /** What `onchange` last reported, so a value a live keystroke already put
   *  in place is still reported once, when the entry is committed. */
  let reported = value;
  $effect(() => {
    if (!typing) reported = value;
  });

  function set(v: string) {
    typing = false;
    invalid = false;
    text = v ? displayClock(v, clockFormat()) : '';
    value = v;
    if (v !== reported) {
      reported = v;
      onchange?.(v);
    }
  }

  function pick(slot: string) {
    set(slot);
    open = false;
    input?.focus();
  }

  /** The typed text, taken as the answer. Empty is "no time". */
  function commitTyped(): boolean {
    if (!typing) return true;
    const t = text.trim();
    if (t === '' && !required) {
      set('');
      return true;
    }
    const parsed = parseClock(t);
    if (parsed === null) {
      invalid = true;
      return false;
    }
    set(parsed);
    return true;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!open) {
        show();
        return;
      }
      active = stepSlot(slots, active, e.key === 'ArrowDown' ? 1 : -1);
      void reveal('nearest');
      return;
    }
    if (e.key === 'Enter') {
      // Never the form's submit: Enter here answers this field.
      e.preventDefault();
      if (open && !typing && active) {
        pick(active);
        return;
      }
      if (commitTyped()) open = false;
      return;
    }
    if (e.key === 'Escape' && open) {
      e.preventDefault();
      e.stopPropagation();
      open = false;
      return;
    }
    if (e.key === 'Tab') open = false;
  }
</script>

<span class="timefield" class:open bind:this={field}>
  <input
    bind:this={input}
    type="text"
    role="combobox"
    autocomplete="off"
    spellcheck="false"
    aria-label={label}
    aria-expanded={open}
    aria-controls="{uid}-list"
    aria-autocomplete="list"
    aria-activedescendant={open && active ? `${uid}-${active}` : undefined}
    aria-invalid={invalid || flagged ? 'true' : undefined}
    class:empty={!value && !typing}
    {placeholder}
    {disabled}
    value={text}
    oninput={(e) => {
      typing = true;
      invalid = false;
      text = e.currentTarget.value;
      const parsed = parseClock(text);
      if (parsed) {
        active = slotAtOrBefore(parsed, slots);
        if (open) void reveal('center');
        if (live && /[:.][0-5]\d|[ap]/i.test(text)) value = parsed;
      }
    }}
    onchange={() => commitTyped()}
    onkeydown={onKey}
    onpointerdown={() => { if (!open) show(); }}
  />
  {#if value && !disabled && !required}
    <button type="button" class="clear" aria-label="Clear {label.toLowerCase()}"
            onclick={() => { set(''); open = false; }}>×</button>
  {/if}
  <button
    type="button"
    class="toggle"
    {disabled}
    aria-label="Pick {label.toLowerCase()}"
    aria-haspopup="listbox"
    aria-expanded={open}
    tabindex="-1"
    onclick={() => (open ? (open = false) : show())}
  >
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
      <circle cx="8" cy="8" r="6.3" fill="none" stroke="currentColor" stroke-width="1.4" />
      <path d="M8 4.6V8l2.4 1.6" fill="none" stroke="currentColor" stroke-width="1.4"
            stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if open}
    <!-- `DateField`'s scrim: a press anywhere else closes the list. The
         field itself sits above it (`.timefield.open`), so the hand can go
         on typing, or clear, without the first press being eaten. -->
    <button class="scrim" tabindex="-1" aria-label="Close {label.toLowerCase()} list"
            onclick={() => (open = false)}></button>
    <div class="list quiet-scroll" id="{uid}-list" role="listbox" aria-label="{label} options"
         bind:this={list} use:dropdown={field}>
      {#each slots as slot (slot)}
        <!-- `pointerdown` kept from the input: the field keeps the focus,
             so the caret and the keyboard stay where the hand left them. -->
        <button
          type="button"
          role="option"
          id="{uid}-{slot}"
          tabindex="-1"
          class:on={slot === value}
          data-active={slot === active ? '' : undefined}
          aria-selected={slot === value}
          onpointerdown={(e) => e.preventDefault()}
          onclick={() => pick(slot)}
        >{displayClock(slot, clockFormat())}{#if note?.(slot)}<span class="note">{note(slot)}</span>{/if}</button>
      {/each}
    </div>
  {/if}
</span>

<style>
  .timefield { position: relative; display: inline-flex; align-items: center; gap: 2px; }
  .timefield.open { z-index: 41; }
  input { font: inherit; font-size: 12.5px; color: var(--text); width: 76px;
          font-variant-numeric: tabular-nums;
          background-color: color-mix(in srgb, var(--text) 5%, transparent);
          border: 1px solid var(--hairline); border-radius: 5px; padding: 4px 6px; }
  /* The empty state has to read as empty. The native field drew a greyed
     "12:30 PM" that looked chosen (reported 2026-09-17); a word in the muted
     voice, and a dashed edge, say "nothing here yet" instead. */
  input.empty { border-style: dashed; background-color: transparent; }
  input::placeholder { color: var(--muted); opacity: .75; }
  input:focus { outline: 1px solid var(--accent); outline-offset: -1px; border-style: solid; }
  input[aria-invalid='true'] { outline: 1px solid var(--error); outline-offset: -1px; }
  input:disabled, .toggle:disabled { opacity: .5; cursor: default; }
  .clear, .toggle { display: inline-flex; align-items: center; justify-content: center;
                    color: var(--text); cursor: pointer; background: none; border: 0;
                    padding: 2px 3px; border-radius: 4px; opacity: .8; font: inherit; }
  .clear { font-size: 14px; line-height: 1; color: var(--muted); }
  .clear:hover, .toggle:hover:not(:disabled) { opacity: 1; color: var(--text);
          background: color-mix(in srgb, var(--text) 10%, transparent); }
  .toggle:focus-visible, .clear:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }

  .scrim { position: fixed; inset: 0; z-index: -1; background: none; border: 0;
           padding: 0; cursor: default; }
  /* Fixed and placed by `dropdown`, for `DateField`'s calendar's reason. */
  .list { position: fixed; top: 0; left: 0; z-index: 1;
          display: flex; flex-direction: column; min-width: 100px; max-height: 216px;
          overflow-y: auto; background: var(--surface); border: 1px solid var(--hairline);
          border-radius: 8px; padding: 4px; box-shadow: 0 6px 20px #0005; }
  .list button { font: inherit; font-size: 12px; text-align: left; color: var(--text);
                 cursor: pointer; background: none; border: 0; border-radius: 4px;
                 padding: 5px 8px; font-variant-numeric: tabular-nums; flex: 0 0 auto; }
  .list button:hover { background: color-mix(in srgb, var(--text) 8%, transparent); }
  .list button[data-active] { outline: 1px solid var(--accent); outline-offset: -1px; }
  .list button.on { background: var(--accent-fill); color: var(--on-accent, #fff); }
  .note { margin-left: 10px; color: var(--muted); font-size: 11px; }
  .list button.on .note { color: inherit; opacity: .75; }
  /* A finger, not a pointer (#111 asks for tablets): rows a fingertip can
     hit without taking the one beside it. */
  @media (pointer: coarse) {
    .list { max-height: 280px; }
    .list button { padding: 11px 12px; font-size: 14px; }
    .toggle { padding: 6px; }
  }
</style>
