-- The time an event actually happened (as opposed to created_at, when it was
-- ingested). Set explicitly by the ingest API, or auto-extracted from the
-- document's content. Drives temporal correlation: entities whose events fall
-- close in time are correlated even when they never co-occur.
ALTER TABLE documents ADD COLUMN event_time timestamptz;
CREATE INDEX documents_event_time_idx ON documents (event_time) WHERE event_time IS NOT NULL;
