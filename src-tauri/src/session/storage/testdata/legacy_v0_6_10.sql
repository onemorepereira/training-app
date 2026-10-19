BEGIN TRANSACTION;
CREATE TABLE known_devices (
    id TEXT PRIMARY KEY,
    name TEXT,
    device_type TEXT NOT NULL,
    transport TEXT NOT NULL,
    rssi INTEGER,
    battery_level INTEGER,
    last_seen TEXT NOT NULL
, device_group TEXT, manufacturer TEXT, model_number TEXT, serial_number TEXT);
INSERT INTO "known_devices" VALUES('ble-AA:BB','KICKR','Power','Ble',-61,90,'2026-02-15T08:30:00Z','grp-1','Wahoo','KICKR5','SN1');
CREATE TABLE session_power_curves (
                session_id TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                watts INTEGER NOT NULL,
                PRIMARY KEY (session_id, duration_secs)
            );
INSERT INTO "session_power_curves" VALUES('11111111-1111-4111-8111-111111111111',1,650);
INSERT INTO "session_power_curves" VALUES('11111111-1111-4111-8111-111111111111',60,320);
INSERT INTO "session_power_curves" VALUES('22222222-2222-4222-8222-222222222222',1,650);
INSERT INTO "session_power_curves" VALUES('22222222-2222-4222-8222-222222222222',60,320);
CREATE TABLE sessions (
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
, ftp INTEGER, title TEXT, activity_type TEXT, rpe INTEGER, notes TEXT, work_kj REAL, variability_index REAL, distance_km REAL);
INSERT INTO "sessions" VALUES('11111111-1111-4111-8111-111111111111','2026-01-15T07:30:00+00:00',3600,180,650,195,60.8,0.78,142,171,88.5,31.25,'/legacy/sessions/11111111-1111-4111-8111-111111111111.bin',250,'Ride 1','endurance',6,'it''s "quoted"; with semicolon',648.0,1.08,31.25);
INSERT INTO "sessions" VALUES('22222222-2222-4222-8222-222222222222','2026-02-15T07:30:00+00:00',3601,181,650,195,60.8,0.78,142,171,88.5,31.25,'/legacy/sessions/22222222-2222-4222-8222-222222222222.bin',250,'Ride 2','endurance',6,'it''s "quoted"; with semicolon',648.0,1.08,31.25);
CREATE TABLE user_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ftp INTEGER NOT NULL DEFAULT 200,
    weight_kg REAL NOT NULL DEFAULT 75.0,
    hr_zone_1 INTEGER NOT NULL DEFAULT 120,
    hr_zone_2 INTEGER NOT NULL DEFAULT 140,
    hr_zone_3 INTEGER NOT NULL DEFAULT 160,
    hr_zone_4 INTEGER NOT NULL DEFAULT 175,
    hr_zone_5 INTEGER NOT NULL DEFAULT 190
, units TEXT NOT NULL DEFAULT 'metric', power_zone_1 INTEGER NOT NULL DEFAULT 55, power_zone_2 INTEGER NOT NULL DEFAULT 75, power_zone_3 INTEGER NOT NULL DEFAULT 90, power_zone_4 INTEGER NOT NULL DEFAULT 105, power_zone_5 INTEGER NOT NULL DEFAULT 120, power_zone_6 INTEGER NOT NULL DEFAULT 150, date_of_birth TEXT, sex TEXT, resting_hr INTEGER, max_hr INTEGER);
INSERT INTO "user_config" VALUES(1,250,72.5,125,145,160,172,185,'imperial',56,76,91,106,121,151,'1985-04-02','male',48,186);
COMMIT;
