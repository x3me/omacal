import { test, expect, type Page } from '@playwright/test';
import { APP_NOW } from './fixtures';
import { cssFamily } from '../src/lib/font';

/**
 * "Use the Omarchy font" (#179): the monospace family `omarchy font set`
 * chose, at the head of the app's font stack while the switch is on, and
 * nothing at all while it is off.
 *
 * The backend reads the family from fontconfig (`font.rs`, whose own tests
 * pin the parsing and the spawn); the stub plays it as `Fake Mono`, a name no
 * machine has, so finding it in a computed `font-family` means it came from
 * the setting.
 */

test.describe('cssFamily', () => {
  /** Quoted always: unquoted, `3270 Nerd Font` is not a family at all, and
   *  one bad family voids the whole list. */
  test('a family is quoted, whatever it looks like', () => {
    expect(cssFamily('JetBrainsMono Nerd Font')).toBe('"JetBrainsMono Nerd Font"');
    expect(cssFamily('3270 Nerd Font')).toBe('"3270 Nerd Font"');
    expect(cssFamily('  Iosevka Term ')).toBe('"Iosevka Term"');
  });

  /** It came from a command's output. Anything that could end the quotes it
   *  is put in, or is no name at all, is refused rather than escaped. */
  test('nothing, or a name that could end its quotes, is no family', () => {
    const refused = [
      null, undefined, '', '   ',
      'Evil", fantasy, "x', 'Evil\\Mono', 'Evil\nMono', 'Evil\u001b[31mMono', 'Evil\u0085Mono',
      'M'.repeat(129),
    ];
    for (const name of refused) expect(cssFamily(name), JSON.stringify(name)).toBeNull();
    expect(cssFamily('M'.repeat(128))).toBe(`"${'M'.repeat(128)}"`);
  });
});

test.describe('App: the Omarchy font', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  /** The stack the body has always been drawn in. Compared as the engine
   *  serialises it, by computing it in the same page, so the two engines'
   *  different quoting cannot make a match or a miss. */
  const STACK = "-apple-system, 'SF Pro Text', Inter, system-ui, sans-serif";
  const resolved = (page: Page, family: string) => page.evaluate((f) => {
    const probe = document.createElement('span');
    probe.style.fontFamily = f;
    document.body.append(probe);
    const value = getComputedStyle(probe).fontFamily;
    probe.remove();
    return value;
  }, family);
  const font = (page: Page, selector = 'body') =>
    page.evaluate((s) => getComputedStyle(document.querySelector(s)!).fontFamily, selector);

  const seed = (page: Page, s: Record<string, unknown>) =>
    page.addInitScript(([k, v]) => sessionStorage.setItem(k, v), ['omacal-stub-settings', JSON.stringify(s)] as const);
  const calls = (page: Page, cmd: string) =>
    page.evaluate((c) => window.__harness.calls.filter((x) => x.cmd === c).length, cmd);

  /** The app, mounted, with its settings read: the first week is fetched
   *  only once they have been (`settingsSeeded`), and by then the startup
   *  read has done whatever it does about the font. */
  async function open(page: Page) {
    await page.goto('/tests/harness/index.html?c=App&f=writable');
    await expect(page.locator('.ev').first()).toBeVisible();
    await expect.poll(() => calls(page, 'get_week')).toBeGreaterThan(0);
  }
  async function appearance(page: Page, loaded: string) {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    // The settings have arrived and named the desktop: until they do, the
    // row is absent on every desktop and an absence proves nothing.
    await expect(modal.getByText(loaded)).toBeVisible();
    return modal;
  }

  /** Off is the default, and then the stack is exactly what it was before
   *  the setting existed — nor is the font so much as asked for. */
  test('off, the app draws the stack it always has and asks for nothing', async ({ page }) => {
    await seed(page, { desktop: 'omarchy' });
    await open(page);
    expect(await font(page)).toBe(await resolved(page, STACK));
    expect(await calls(page, 'get_omarchy_font')).toBe(0);
  });

  test('on, the Omarchy font leads the stack, and the event titles wear it', async ({ page }) => {
    await seed(page, { desktop: 'omarchy', useOmarchyFont: true });
    await open(page);
    const worn = await resolved(page, `"Fake Mono", ${STACK}`);
    await expect.poll(() => font(page)).toBe(worn);
    expect(await font(page, '.ev')).toBe(worn);
  });

  test('the switch is offered on Omarchy, off, and applies at once both ways', async ({ page }) => {
    await seed(page, { desktop: 'omarchy' });
    await open(page);
    const modal = await appearance(page, 'wears your Omarchy theme');
    const toggle = modal.getByRole('checkbox', { name: 'Use the Omarchy font' });
    await expect(toggle).not.toBeChecked();

    await toggle.check();
    await expect.poll(() => font(page)).toBe(await resolved(page, `"Fake Mono", ${STACK}`));
    await toggle.uncheck();
    await expect.poll(() => font(page)).toBe(await resolved(page, STACK));
    expect(await page.evaluate(() => window.__harness.settingValues('useOmarchyFont'))).toEqual([true, false]);
  });

  test('off Omarchy there is no Omarchy font to offer', async ({ page }) => {
    await open(page);
    const modal = await appearance(page, 'wears your light or dark theme');
    await expect(modal.getByRole('checkbox', { name: 'Use the Omarchy font' })).toHaveCount(0);
  });

  /** `omarchy font set` while the app is open: nothing moves until the
   *  backend's watcher says the desktop changed, and then the new family is
   *  worn without a restart. */
  test('a font set while the app is open is worn when the desktop says so', async ({ page }) => {
    await seed(page, { desktop: 'omarchy', useOmarchyFont: true });
    await open(page);
    await expect.poll(() => font(page)).toBe(await resolved(page, `"Fake Mono", ${STACK}`));

    await page.evaluate(() => window.__harness.setOmarchyFont('Other Mono'));
    await page.evaluate(() => window.__harness.emit('font-changed', null));
    await expect.poll(() => font(page)).toBe(await resolved(page, `"Other Mono", ${STACK}`));
  });

  /** A name that could end its quotes is no font: the stack goes back to
   *  the one it always was, rather than taking whatever the name injects. */
  test('a name that could break out of its quotes leaves the stack as it always was', async ({ page }) => {
    await seed(page, { desktop: 'omarchy', useOmarchyFont: true });
    await open(page);
    await expect.poll(() => font(page)).toBe(await resolved(page, `"Fake Mono", ${STACK}`));

    await page.evaluate(() => window.__harness.setOmarchyFont('Evil", fantasy, "x'));
    await page.evaluate(() => window.__harness.emit('font-changed', null));
    await expect.poll(() => font(page)).toBe(await resolved(page, STACK));
  });

  /** Fontconfig can take its time. Switched on and straight back off, the
   *  "on" answer may land after the "off" one; it answers a question nobody
   *  is asking any more and must not put the font back. */
  test('an answer that lands after the switch went back off does not put the font back', async ({ page }) => {
    await seed(page, { desktop: 'omarchy' });
    await open(page);
    // Hold the first answer — given as the stub would have given it when
    // asked, while the switch was on — until the spec lets it go.
    await page.evaluate(() => {
      const ipc = (window as any).__TAURI_INTERNALS__;
      const invoke = ipc.invoke;
      let first = true;
      ipc.invoke = async (cmd: string, ...rest: unknown[]) => {
        const answer = await invoke(cmd, ...rest);
        if (cmd !== 'get_omarchy_font' || !first) return answer;
        first = false;
        await new Promise((release) => { (window as any).__releaseFont = release; });
        return answer;
      };
    });
    const modal = await appearance(page, 'wears your Omarchy theme');
    const toggle = modal.getByRole('checkbox', { name: 'Use the Omarchy font' });
    await toggle.check();
    await expect.poll(() => page.evaluate(() => typeof (window as any).__releaseFont)).toBe('function');
    await toggle.uncheck();
    await expect.poll(() => calls(page, 'get_omarchy_font')).toBe(2);

    await page.evaluate(() => (window as any).__releaseFont());
    // The late answer's chance to land, then the absence it must leave.
    await page.evaluate(() => new Promise((r) => setTimeout(r, 50)));
    expect(await font(page)).toBe(await resolved(page, STACK));
  });
});
