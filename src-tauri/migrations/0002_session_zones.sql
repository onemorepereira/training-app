-- Per-session zone definitions snapshot (nullable JSON), used by Area A's
-- recompute of time-in-zone. SHIPPED STEP once released: never edit.
ALTER TABLE sessions ADD COLUMN zones TEXT;
