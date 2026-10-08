<!-- ui/src/lib/LoginPrompt.svelte -->
<script lang="ts">
  import { setSetting, type AppSettings, type StartOnLogin } from './settings';

  /** The one-time question of how OmaCal starts at login (2026-10-08).
   *
   *  Users objected to a login entry nobody had agreed to, and on Omarchy to
   *  the window it opened, which Hyprland tiles across the whole workspace.
   *  The backend registers nothing until this card or Settings is answered.
   *  Not a modal: the calendar stays usable underneath, and any of the three
   *  answers is final (Settings → General changes it later). */
  let { desktop, onanswered }: {
    desktop: AppSettings['desktop'];
    /** The settings the answer produced, for App to take in. */
    onanswered: (settings: AppSettings) => void;
  } = $props();

  let busy = $state(false);
  let note = $state<string | null>(null);

  /** What stops working while OmaCal is not running, in the desktop's terms. */
  const why = $derived(
    desktop === 'omarchy' ? 'Reminders and the bar widget only work while it runs.'
    : desktop === 'macos' ? 'Reminders and the menu bar only work while it runs.'
    : 'Reminders only reach you while it runs.',
  );

  async function answer(mode: StartOnLogin) {
    busy = true;
    note = null;
    try {
      onanswered(await setSetting('startOnLogin', mode));
    } catch (e) {
      note = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="login-prompt" aria-label="Start OmaCal when you log in?">
  <p class="q">Start OmaCal when you log in?</p>
  <p class="why">{why}</p>
  <div class="answers">
    <button class="primary" disabled={busy} onclick={() => answer('background')}>In the background</button>
    <button disabled={busy} onclick={() => answer('open')}>With its window</button>
    <button disabled={busy} onclick={() => answer('off')}>Don't start it</button>
  </div>
  {#if desktop === 'omarchy'}
    <p class="hint">With its window, Omarchy tiles it like any other.</p>
  {/if}
  {#if note}<p class="note" role="alert">{note}</p>{/if}
</section>

<style>
  /* Bottom right, over the calendar but under every popover and modal (41
     and up): a question that waits, not one that blocks. */
  .login-prompt { position: fixed; right: 16px; bottom: 16px; z-index: 40;
                  width: min(340px, calc(100vw - 32px)); padding: 12px 14px;
                  background: var(--surface); border: 1px solid var(--hairline); border-radius: 8px;
                  box-shadow: 0 4px 16px #0004; font-size: 12px; }
  .q { margin: 0; font-weight: 600; color: var(--text); font-size: 13px; }
  .why { margin: 3px 0 10px; color: var(--muted); }
  .answers { display: flex; flex-wrap: wrap; gap: 6px; }
  button { font: inherit; font-size: 12px; color: var(--text); cursor: pointer; white-space: nowrap;
           background: color-mix(in srgb, var(--text) 6%, transparent);
           border: 0; border-radius: 6px; padding: 5px 10px; }
  button:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 10%, transparent); }
  button:disabled { opacity: .5; cursor: default; }
  .primary { background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .primary:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 88%, var(--text)); }
  .hint { margin: 8px 0 0; color: var(--muted); font-size: 11px; }
  .note { margin: 8px 0 0; color: var(--error); }
</style>
