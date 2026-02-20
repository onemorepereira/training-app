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
