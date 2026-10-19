-- Baseline: the exact schema every release up to main@3b10a73 created at startup
-- (former 001_init.sql + the inline ALTERs from Storage::new + session_power_curves).
-- It must converge every shipped database state to the same column set, so each
-- ADD COLUMN is skipped by the runner when the column already exists.
-- SHIPPED STEP: never edit this file (its SHA-256 is pinned by a test).

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    start_time TEXT NOT NULL,
    duration_secs INTEGER NOT NULL,
    avg_power INTEGER,
    max_power INTEGER,
    normalized_power INTEGER,
    tss REAL,
    intensity_factor REAL,
    avg_hr INTEGER,
    max_hr INTEGER,
    avg_cadence REAL,
    avg_speed REAL,
    raw_file_path TEXT
);

CREATE TABLE IF NOT EXISTS user_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ftp INTEGER NOT NULL DEFAULT 200,
    weight_kg REAL NOT NULL DEFAULT 75.0,
    hr_zone_1 INTEGER NOT NULL DEFAULT 120,
    hr_zone_2 INTEGER NOT NULL DEFAULT 140,
    hr_zone_3 INTEGER NOT NULL DEFAULT 160,
    hr_zone_4 INTEGER NOT NULL DEFAULT 175,
    hr_zone_5 INTEGER NOT NULL DEFAULT 190
);

INSERT OR IGNORE INTO user_config (id) VALUES (1);

CREATE TABLE IF NOT EXISTS known_devices (
    id TEXT PRIMARY KEY,
    name TEXT,
    device_type TEXT NOT NULL,
    transport TEXT NOT NULL,
    rssi INTEGER,
    battery_level INTEGER,
    last_seen TEXT NOT NULL
);

-- Unit system preference and power zones as % of FTP (Coggan: upper bounds Z1-Z6)
ALTER TABLE user_config ADD COLUMN units TEXT NOT NULL DEFAULT 'metric';
ALTER TABLE user_config ADD COLUMN power_zone_1 INTEGER NOT NULL DEFAULT 55;
ALTER TABLE user_config ADD COLUMN power_zone_2 INTEGER NOT NULL DEFAULT 75;
ALTER TABLE user_config ADD COLUMN power_zone_3 INTEGER NOT NULL DEFAULT 90;
ALTER TABLE user_config ADD COLUMN power_zone_4 INTEGER NOT NULL DEFAULT 105;
ALTER TABLE user_config ADD COLUMN power_zone_5 INTEGER NOT NULL DEFAULT 120;
ALTER TABLE user_config ADD COLUMN power_zone_6 INTEGER NOT NULL DEFAULT 150;

-- Athlete profile
ALTER TABLE user_config ADD COLUMN date_of_birth TEXT;
ALTER TABLE user_config ADD COLUMN sex TEXT;
ALTER TABLE user_config ADD COLUMN resting_hr INTEGER;
ALTER TABLE user_config ADD COLUMN max_hr INTEGER;

-- FTP used in each session (audit trail)
ALTER TABLE sessions ADD COLUMN ftp INTEGER;

-- Device metadata for cross-transport deduplication
ALTER TABLE known_devices ADD COLUMN device_group TEXT;
ALTER TABLE known_devices ADD COLUMN manufacturer TEXT;
ALTER TABLE known_devices ADD COLUMN model_number TEXT;
ALTER TABLE known_devices ADD COLUMN serial_number TEXT;

-- Activity metadata
ALTER TABLE sessions ADD COLUMN title TEXT;
ALTER TABLE sessions ADD COLUMN activity_type TEXT;
ALTER TABLE sessions ADD COLUMN rpe INTEGER;
ALTER TABLE sessions ADD COLUMN notes TEXT;

-- Zone ride config snapshot
ALTER TABLE sessions ADD COLUMN zone_config TEXT;

-- Work (kJ), variability index, distance
ALTER TABLE sessions ADD COLUMN work_kj REAL;
ALTER TABLE sessions ADD COLUMN variability_index REAL;
ALTER TABLE sessions ADD COLUMN distance_km REAL;

-- Power curve cache
CREATE TABLE IF NOT EXISTS session_power_curves (
    session_id TEXT NOT NULL,
    duration_secs INTEGER NOT NULL,
    watts INTEGER NOT NULL,
    PRIMARY KEY (session_id, duration_secs)
);
