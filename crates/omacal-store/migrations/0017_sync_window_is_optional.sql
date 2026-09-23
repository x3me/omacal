-- `window_start` / `window_end` become nullable, because one provider no
-- longer has a window at all.
--
-- A subscribed feed (`provider = 'webcal'`) arrives whole on every fetch, so
-- there is no slice to record and nothing for a later sync to compare against.
-- It used to be windowed like the others, and that was the bug: an event past
-- the horizon was parsed, discarded, and then never seen again, because the
-- next sync answered 304 and never re-parsed the file. Feeds now store what
-- the file says, and these two columns are meaningless for them.
--
-- NULL rather than a sentinel, and rather than leaving the feed rows writing
-- a window they do not honour: the Google and CalDAV paths are about to
-- *read* these columns to decide when a sliding window has moved past what
-- was last fetched, and "no window applies here" has to be distinguishable
-- from "a window starting at 0". A lie stored to satisfy NOT NULL would read
-- as the second.
--
-- SQLite cannot drop NOT NULL in place, so the table is rebuilt. It is small
-- (one row per calendar) and carries no indexes of its own.

CREATE TABLE sync_state_new (
  calendar_id        INTEGER PRIMARY KEY REFERENCES calendars(id) ON DELETE CASCADE,
  sync_token         TEXT,
  last_full_sync_at  INTEGER,
  window_start       INTEGER,
  window_end         INTEGER
);

INSERT INTO sync_state_new (calendar_id, sync_token, last_full_sync_at, window_start, window_end)
SELECT calendar_id, sync_token, last_full_sync_at, window_start, window_end FROM sync_state;

DROP TABLE sync_state;

ALTER TABLE sync_state_new RENAME TO sync_state;

-- Feeds keep their cursor and lose their window, in one statement rather than
-- waiting for each to sync once.
UPDATE sync_state
   SET window_start = NULL, window_end = NULL
 WHERE calendar_id IN (
       SELECT c.id FROM calendars c
         JOIN accounts a ON a.id = c.account_id
        WHERE a.provider = 'webcal');
