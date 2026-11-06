-- "Show as" and "Visibility", per event, on both providers' vocabularies.
--
-- `transparency` is Google's word and holds Google's two values: `opaque`
-- (busy, the default) or `transparent` (free). CalDAV's `TRANSP:OPAQUE` and
-- `TRANSP:TRANSPARENT` map onto the same two.
--
-- `visibility` is `default` | `public` | `private` | `confidential`. Google
-- uses all four; CalDAV's `CLASS` has the last three, and an event without a
-- `CLASS` line is `default`. `confidential` is stored and shown but never set
-- by this app: it has no agreed meaning across servers, so a save only keeps
-- it.
--
-- Both default to the value a server assumes when the field is absent, so a
-- row that never learns either reads as what it actually is.
ALTER TABLE events ADD COLUMN transparency TEXT NOT NULL DEFAULT 'opaque';
ALTER TABLE events ADD COLUMN visibility TEXT NOT NULL DEFAULT 'default';

-- The backfill, the same trade 0004 made. Every event already stored would
-- read as busy and default forever, since an unchanged event is never
-- re-delivered by an incremental sync; dropping every cursor makes the next
-- sync a full window fetch that rewrites each row with what the server
-- actually holds. Costs one slow sync on first launch after the update.
DELETE FROM sync_state;
