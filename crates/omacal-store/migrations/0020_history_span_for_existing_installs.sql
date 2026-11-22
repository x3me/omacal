-- Keep history becomes a choice (2026-11-01). From here an absent
-- `history_span_months` reads as 2 years (src-tauri/src/settings.rs), which
-- covers all of last year in the Year view. An install already in use keeps
-- the 6 months sync has always fetched, so an update does not refetch
-- everyone's calendars at once.
--
-- The same split as 0018: the old value is written only where a calendar
-- exists. A fresh database gets no row, so it reads as the new default.

INSERT OR IGNORE INTO settings (key, value)
SELECT 'history_span_months', '6'
WHERE EXISTS (SELECT 1 FROM calendars);
