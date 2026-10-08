-- Starting at login becomes something OmaCal asks about instead of assuming
-- (2026-10-08). Until now an unset `autostart` read as "open": every install
-- registered a login entry that opened the window, and nobody had agreed to
-- it. From here an unset value reads as off, and `autostart_asked` records
-- that the user has answered (src-tauri/src/settings.rs).
--
-- Order matters: the first statement must see only choices made before this
-- migration, not the one the second statement writes.

-- A choice already stored was made in Settings, so it is an answer.
INSERT OR IGNORE INTO settings (key, value)
SELECT 'autostart_asked', '1'
WHERE EXISTS (SELECT 1 FROM settings WHERE key = 'autostart');

-- An install in use that never chose keeps starting at login, so nobody
-- loses reminders to an update, but in the background: no window appears at
-- login until they answer. A fresh install (no calendars yet) gets nothing
-- written, so it registers nothing until they say yes.
INSERT OR IGNORE INTO settings (key, value)
SELECT 'autostart', 'background'
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'autostart')
  AND EXISTS (SELECT 1 FROM calendars);
