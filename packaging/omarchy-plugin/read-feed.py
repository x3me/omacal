#!/usr/bin/python3
"""Read one bounded snapshot for the shell; never wait on a FIFO or follow a link."""
import json
import os
import signal
import stat
import sys

INPUT_LIMIT = 2 * 1024 * 1024
OUTPUT_LIMIT = 1024 * 1024 - 1


def clean(value, depth=0):
    if depth > 10:
        raise ValueError('nested feed')
    if isinstance(value, str):
        return ''.join(c for c in value[:2048] if ord(c) >= 32
                       and not 127 <= ord(c) <= 159
                       and ord(c) not in {0x61c, 0x200e, 0x200f, *range(0x202a, 0x202f), *range(0x2066, 0x206a)})
    if isinstance(value, list):
        return [clean(v, depth + 1) for v in value[:200]]
    if isinstance(value, dict):
        return {k: clean(v, depth + 1) for k, v in list(value.items())[:600] if len(k) <= 64}
    if value is None or isinstance(value, (bool, int, float)):
        return value
    raise ValueError('invalid field')


def events_valid(events):
    # The app can publish point-in-time events. Rejecting an equal start/end
    # here blanks the entire widget, including every other upcoming meeting.
    return isinstance(events, list) and len(events) <= 200 and all(isinstance(e, dict)
        and all(type(e.get(k)) in (int, float) and abs(e[k]) < 8640000000000000 for k in ('start_ms', 'end_ms'))
        and e['end_ms'] >= e['start_ms'] and type(e.get('all_day')) is bool
        and all(e.get(k) is None or isinstance(e[k], str) for k in ('title', 'color', 'conference', 'location', 'calendar', 'response'))
        for e in events)


def read_feed(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_size > INPUT_LIMIT:
            raise ValueError('unexpected feed file')
        snapshot = source.read(INPUT_LIMIT + 1)
    if len(snapshot) > INPUT_LIMIT:
        raise ValueError('feed too large')
    feed = json.loads(snapshot)
    if not isinstance(feed, dict) or not events_valid(feed.get('events')):
        raise ValueError('invalid events')
    panel = feed.get('panel')
    if panel is not None:
        if not isinstance(panel, dict) or not events_valid(panel.get('events')):
            raise ValueError('invalid day')
        if 'agenda_days' in panel:
            days = panel['agenda_days']
            if not isinstance(days, list) or not 1 <= len(days) <= 8 or not all(
                isinstance(d, dict) and isinstance(d.get('date_label'), str) and events_valid(d.get('events')) for d in days
            ) or sum(len(d['events']) for d in days) > 200:
                raise ValueError('invalid agenda days')
        if not all(type(panel.get(k)) is int for k in ('day_start_ms', 'day_end_ms', 'join_minutes')):
            raise ValueError('invalid day clock')
        if not 0 < panel['day_end_ms'] - panel['day_start_ms'] <= 26 * 3600000 or not 0 <= panel['join_minutes'] <= 60:
            raise ValueError('invalid day bounds')
        if not isinstance(panel.get('clocks'), dict) or not isinstance(panel.get('hours'), list):
            raise ValueError('invalid clocks')
        if any(abs(panel[k]) >= 8640000000000000 for k in ('day_start_ms', 'day_end_ms')):
            raise ValueError('day out of range')
        if not all(type(panel.get(k)) is bool for k in ('label',)):
            raise ValueError('invalid preferences')
        if len(panel['hours']) > 26 or not all(type(h) is int and panel['day_start_ms'] <= h < panel['day_end_ms'] for h in panel['hours']):
            raise ValueError('invalid hours')
        if len(panel['clocks']) > 600 or not all(isinstance(v, str) and len(v) <= 32 for v in panel['clocks'].values()):
            raise ValueError('invalid labels')
    result = json.dumps(clean(feed), ensure_ascii=True, allow_nan=False, separators=(',', ':'))
    if len(result) > OUTPUT_LIMIT:
        raise ValueError('result too large')
    return result


if __name__ == '__main__':
    signal.alarm(2)
    try:
        print(read_feed(sys.argv[1]))
    except (OSError, ValueError, TypeError, IndexError, RecursionError):
        sys.exit(1)
