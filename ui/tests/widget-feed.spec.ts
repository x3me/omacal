import { test, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const helper = fileURLToPath(new URL('../../packaging/omarchy-plugin/read-feed.py', import.meta.url));

test('feed reader rejects oversized, malformed, non-object, symlink and FIFO inputs', () => {
  const dir = mkdtempSync(join(tmpdir(), 'omacal-feed-'));
  const path = join(dir, 'feed');
  const read = (p = path) => spawnSync('/usr/bin/python3', [helper, p], { timeout: 3000, encoding: 'utf8' });
  try {
    for (const content of ['null', '[]', 'true', '2', '"hello"', '{', '{"events":[null]}', '{"events":[]}' + ' '.repeat(2 * 1024 * 1024 - 12)]) {
      writeFileSync(path, content);
      expect(read().status).not.toBe(0);
    }
    writeFileSync(path, '{"events":[]}');
    symlinkSync(path, join(dir, 'link'));
    expect(read(join(dir, 'link')).status).not.toBe(0);
    expect(spawnSync('mkfifo', [join(dir, 'fifo')]).status).toBe(0);
    const fifo = read(join(dir, 'fifo'));
    expect(fifo.status).toBe(1);
    expect(fifo.error).toBeUndefined();
    writeFileSync(path, '{"events":[]}' + ' '.repeat(2 * 1024 * 1024 - 13));
    expect(read().status).toBe(0);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('feed reader bounds and sanitizes text before the shell receives it', () => {
  const dir = mkdtempSync(join(tmpdir(), 'omacal-feed-'));
  try {
    const path = join(dir, 'feed');
    writeFileSync(path, JSON.stringify({ events: [{ title: '<b>Design</b>\u202e\n' + 'x'.repeat(4000), start_ms: 1, end_ms: 2, all_day: false }] }));
    const result = spawnSync('/usr/bin/python3', [helper, path], { timeout: 3000, encoding: 'utf8' });
    expect(result.status).toBe(0);
    const title = JSON.parse(result.stdout).events[0].title;
    expect(title).toContain('<b>Design</b>');
    expect(title).not.toMatch(/[\u202e\n]/);
    expect(title.length).toBeLessThanOrEqual(2048);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('zero-duration events keep the feed available in every agenda list', () => {
  const dir = mkdtempSync(join(tmpdir(), 'omacal-instant-feed-'));
  try {
    const path = join(dir, 'feed');
    const meeting = {title: 'Design review', start_ms: 1000, end_ms: 2000, all_day: false};
    const instant = {title: 'Deadline', start_ms: 3000, end_ms: 3000, all_day: false};
    const events = [meeting, instant];
    const panel = {events, agenda_days: [{date_label: 'Sep 9', events}], day_view: false,
      label: true, day_start_ms: 0, day_end_ms: 86400000, join_minutes: 5, clocks: {}, hours: []};
    const read = (feed: unknown) => {
      writeFileSync(path, JSON.stringify(feed));
      return spawnSync('/usr/bin/python3', [helper, path], {timeout: 3000, encoding: 'utf8'});
    };
    const result = read({events, panel});
    expect(result.status).toBe(0);
    const feed = JSON.parse(result.stdout);
    expect(feed.events).toEqual(events);
    expect(feed.panel.events).toEqual(events);
    expect(feed.panel.agenda_days[0].events).toEqual(events);

    // Equal timestamps are allowed; reversed intervals still fail validation.
    const reversed = {...instant, end_ms: instant.start_ms - 1};
    for (const invalid of [
      {events: [meeting, reversed], panel},
      {events, panel: {...panel, events: [reversed]}},
      {events, panel: {...panel, agenda_days: [{date_label: 'Sep 9', events: [reversed]}]}},
    ]) expect(read(invalid).status).not.toBe(0);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});

test('agenda feed accepts five days and rejects invalid or oversized day groups', () => {
  const dir = mkdtempSync(join(tmpdir(), 'omacal-agenda-feed-'));
  try {
    const path = join(dir, 'feed');
    const event = {title: '<b>Name</b>\u202e', start_ms: 1, end_ms: 2, all_day: false};
    const day = {date_label: 'Sep 7', events: [event]};
    const panel = {events: [], day_view: false, label: true, day_start_ms: 0, day_end_ms: 86400000, join_minutes: 5, clocks: {}, hours: []};
    for (const days of [null, [null], Array(8).fill(day), [{...day, events: [null]}],
      [{...day, events: Array(101).fill(event)}, {...day, events: Array(100).fill(event)}]]) {
      writeFileSync(path, JSON.stringify({events: [], panel: {...panel, agenda_days: days}}));
      expect(spawnSync('/usr/bin/python3', [helper, path]).status).not.toBe(0);
    }
    writeFileSync(path, JSON.stringify({events: [], panel: {...panel, agenda_days: Array(5).fill(day)}}));
    const result = spawnSync('/usr/bin/python3', [helper, path], {encoding:'utf8'});
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout).panel.agenda_days[4].events[0].title).toBe('<b>Name</b>');
  } finally { rmSync(dir, {recursive:true, force:true}); }
});


test('combined calendar colors are bounded hex values before they reach QML', () => {
  const dir = mkdtempSync(join(tmpdir(), 'omacal-colors-'));
  const path = join(dir, 'feed');
  const read = (colors: unknown) => {
    writeFileSync(path, JSON.stringify({events: [{title: 'Design sync', start_ms: 1, end_ms: 2, all_day: false, colors}]}));
    return spawnSync('/usr/bin/python3', [helper, path], {timeout: 3000, encoding: 'utf8'});
  };
  try {
    for (const colors of [['#112233', '#ABCDEF'], [], Array(200).fill('#123456')]) {
      const result = read(colors);
      expect(result.status).toBe(0);
      expect(JSON.parse(result.stdout).events[0].colors).toEqual(colors);
    }
    for (const invalid of [null, {}, 'red', ['red'], ['#fff'], ['#123456\u202e'], [12], Array(201).fill('#123456')])
      expect(read(invalid).status).not.toBe(0);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});
