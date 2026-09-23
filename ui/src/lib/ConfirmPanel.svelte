<!-- ui/src/lib/ConfirmPanel.svelte -->
<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { escapeCloses } from './dismiss.svelte';
  import { placePopover, type Rect } from './position';
  import { focusInitialChoice, handleChoiceKey } from './choicefocus';

  let {
    anchor,
    label,
    title,
    oncancel,
    body,
    actions,
  }: {
    /** The rect to sit beside — the popover, block or chip that opened this. */
    anchor: Rect;
    /** The dialog's accessible name. */
    label: string;
    /** The heading. Always names the event: "Delete this?" is the same sentence
     *  whichever block was clicked, and whatever opened this panel is no longer
     *  on screen to say which one it was. */
    title: string;
    oncancel: () => void;
    /** Everything between the heading and the buttons. */
    body: Snippet;
    /** The buttons, Cancel included — a panel that offers one confirmation and
     *  a panel that offers two are different enough to be worth writing out at
     *  each call site rather than parameterising into a list. */
    actions: Snippet;
  } = $props();

  let panelEl: HTMLDivElement | undefined = $state();
  // A neutral default so the panel renders — and is measurable — before
  // `onMount` places it, exactly as `EventPopover` and `EventForm` do. `anchor`
  // never changes for this component's lifetime: a fresh panel is mounted per
  // open rather than reused, so one placement is all this needs.
  let pos = $state<{ top: number; left: number }>({ top: 0, left: 0 });

  function handlePanelKey(event: KeyboardEvent) {
    if (panelEl) handleChoiceKey(panelEl, event);
  }

  onMount(() => {
    if (panelEl) {
      const viewport = { width: window.innerWidth, height: window.innerHeight };
      pos = placePopover(
        anchor,
        { width: panelEl.offsetWidth, height: panelEl.offsetHeight },
        viewport,
      );
    }
    // A panel can nominate a meaningful scope or notification choice. Without
    // one, focus falls back to Cancel: native Enter then activates exactly the
    // button the user can see is focused, including the safe choice.
    focusInitialChoice(panelEl);
  });

  // Escape closes this panel. `escapeCloses` carries the whole reason it is a
  // `window` listener; `() => true` is this panel saying it is the topmost
  // thing whenever it is open, which it is — nothing opens over a confirmation.
  escapeCloses(() => true, () => oncancel());
</script>

<!-- A sibling of `.pop`, not a wrapper, so a click inside the panel never
     reaches this button. -->
<button class="scrim" aria-label="Cancel" onclick={oncancel}></button>

<div
  class="pop"
  bind:this={panelEl}
  role="dialog"
  aria-modal="true"
  tabindex="-1"
  aria-label={label}
  style="top:{pos.top}px; left:{pos.left}px"
  onkeydown={handlePanelKey}
>
  <h2>{title}</h2>
  {@render body()}
  <div class="actions" data-choice-group>{@render actions()}</div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default; z-index: 40; }

  .pop { position: fixed; z-index: 41; width: 320px; max-height: 80vh; overflow-y: auto;
         background: var(--surface); border: 1px solid var(--hairline);
         border-radius: 8px; padding: 12px 14px; box-shadow: 0 8px 28px rgba(0, 0, 0, .45);
         font-size: 12px; color: var(--text);
         display: flex; flex-direction: column; gap: 8px; }
  /* Focused on mount only to contain the tab order, exactly as `EventPopover`'s
     panel is; the controls inside keep their own rings. */
  .pop:focus { outline: none; }

  h2 { font-size: 14px; font-weight: 600; margin: 0; letter-spacing: -.01em;
       overflow-wrap: anywhere; }

  .actions { display: flex; gap: 6px; justify-content: flex-end; margin-top: 2px; }
  /* `:global`, because the buttons are the caller's own markup rendered through
     a snippet: the two panels offer different numbers of them and different
     words on them, and only the row they sit in is shared. */
  .actions :global(button) { font: inherit; font-size: 11.5px; cursor: pointer;
                             border-radius: 6px; padding: 5px 12px;
                             border: 1px solid var(--hairline); }
  .actions :global(.ghost) { background: none; color: var(--muted); }
  /* Accent, not red. This app takes its palette from the Omarchy theme, which
     offers no semantic red — the same reason `EventPopover`'s guest list
     carries its status in a glyph rather than a hue. The weight of a
     destructive action comes from the dialog, the wording and where focus
     starts, not from a colour the theme cannot promise. */
  .actions :global(.primary) { background: var(--accent); border-color: var(--accent);
                               color: var(--on-accent); }
</style>
