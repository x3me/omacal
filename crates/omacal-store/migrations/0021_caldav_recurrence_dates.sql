-- A DATE recurrence ID has no UTC offset. Preserve the wire date alongside
-- the existing event key, even after deleting a detached event creates a
-- notice. The organizer may reinstate that date in a later REPORT.
CREATE TABLE caldav_recurrence_dates (
  calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
  gid TEXT NOT NULL,
  recurrence_date TEXT,
  PRIMARY KEY (calendar_id, gid)
) WITHOUT ROWID;

-- NULL identifies legacy rows whose original wire date was not recorded.
INSERT INTO caldav_recurrence_dates (calendar_id, gid)
SELECT e.calendar_id, e.google_id FROM events e
JOIN calendars c ON c.id = e.calendar_id JOIN accounts a ON a.id = c.account_id
JOIN events master ON master.calendar_id=e.calendar_id AND master.google_id=e.recurring_event_id
WHERE a.provider = 'caldav' AND master.is_all_day
UNION
SELECT ec.calendar_id, ec.gid FROM event_changes ec
JOIN calendars c ON c.id = ec.calendar_id JOIN accounts a ON a.id = c.account_id
LEFT JOIN events master ON master.calendar_id=ec.calendar_id AND master.google_id=ec.series_gid
WHERE a.provider = 'caldav' AND ec.series_gid IS NOT NULL AND (ec.is_all_day OR master.is_all_day);

UPDATE sync_state SET sync_token = NULL
WHERE calendar_id IN (SELECT calendar_id FROM caldav_recurrence_dates);
