import { test, expect } from '@playwright/test';

for (const combined of [false, true]) {
  test(`agenda sections preserve calendar copies when combination is ${combined ? 'on' : 'off'}`, async ({ page }) => {
    const start = Date.UTC(2026, 8, 7);
    await page.clock.install({ time: start + 12 * 3600000 });
    await page.addInitScript(({ start, combined }) => {
      const copies = [
        { title: 'Holiday', start_ms: start, end_ms: start + 86400000, all_day: true, calendar: 'Work', color: '#336699' },
        { title: 'Holiday', start_ms: start, end_ms: start + 86400000, all_day: true, calendar: 'Home', color: '#66aa33' },
      ];
      const colors = copies.map(e => e.color);
      const today = combined ? [{ ...copies[0], colors }] : copies;
      const tomorrow = [{ title: 'Review', start_ms: start + 33 * 3600000, end_ms: start + 34 * 3600000,
        all_day: false, color: colors[0], colors: combined ? colors : [] }];
      const feed = { combine_identical_events: combined, events: tomorrow, tasks: [], panel: {
        day_start_ms: start, day_end_ms: start + 86400000, events: today, timezone: 'UTC',
        time_format: '24h', label: true, join_minutes: 5,
        agenda_days: [{ date_label: 'Today', events: today }, { date_label: 'Tomorrow', events: tomorrow }],
      } };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: async (cmd: string) => {
          if (cmd === 'menubar_feed') return feed;
          if (cmd === 'get_palette') return { bg: '#20232b', surface: '#303540', text: '#eeeeee', muted: '#aaaaaa', accent: '#87b7ff', is_dark: true };
          return 1;
        },
      };
    }, { start, combined });
    await page.goto('/?menubar');
    const holiday = page.getByRole('button', { name: 'Holiday', exact: true });
    await expect(holiday).toHaveCount(combined ? 1 : 2);
    const review = page.getByRole('button', { name: /Review/ });
    await expect(review).toBeVisible();
    for (const row of [holiday.first(), review]) {
      await expect(row.locator('.calendar-colors > span')).toHaveCount(combined ? 2 : 0);
    }
  });
}
