"""Build a legacy training.db fixture by replaying a tagged Storage::new.

usage: genfx.py build <storage_src.rs> <001_init.sql> <out.db>
       genfx.py dump <db> <out.sql>

`build` extracts, in source order, every SQL string literal Storage::new ran
(ALTER TABLE ... and CREATE TABLE IF NOT EXISTS ...), runs 001_init.sql then
each statement (ignoring 'duplicate column name' exactly like the Rust code),
then seeds rows using only columns that exist in that schema.
"""
import re
import sqlite3
import sys


def extract_statements(src):
    body = src[src.index("pub async fn new"):]
    body = body[: body.index("Ok(Self")]
    stmts = []
    for m in re.finditer(r'"((?:ALTER TABLE|CREATE TABLE IF NOT EXISTS)[^"]*)"', body, re.S):
        stmts.append(m.group(1))
    return stmts


def cols(c, table):
    return [r[1] for r in c.execute(f"pragma table_info({table})")]


def insert(c, table, row):
    have = set(cols(c, table))
    row = {k: v for k, v in row.items() if k in have}
    keys = ",".join(row)
    q = ",".join("?" for _ in row)
    c.execute(f"INSERT OR REPLACE INTO {table} ({keys}) VALUES ({q})", list(row.values()))


def build(src_path, init_path, out):
    src = open(src_path).read()
    c = sqlite3.connect(out, isolation_level=None)
    c.executescript(open(init_path).read())
    for s in extract_statements(src):
        try:
            c.execute(s)
        except sqlite3.OperationalError as e:
            if "duplicate column name" not in str(e):
                raise
    ids = ["11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"]
    for i, sid in enumerate(ids):
        insert(c, "sessions", dict(
            id=sid, start_time=f"2026-0{i+1}-15T07:30:00+00:00", duration_secs=3600 + i,
            ftp=250, avg_power=180 + i, max_power=650, normalized_power=195, tss=60.8,
            intensity_factor=0.78, avg_hr=142, max_hr=171, avg_cadence=88.5, avg_speed=31.25,
            raw_file_path=f"/legacy/sessions/{sid}.bin", title=f"Ride {i+1}",
            activity_type="endurance", rpe=6, notes="it's \"quoted\"; with semicolon",
            zone_config='{"mode":"power","zone":2}', work_kj=648.0, variability_index=1.08,
            distance_km=31.25,
        ))
        if "session_power_curves" in [r[0] for r in c.execute("select name from sqlite_master")]:
            insert(c, "session_power_curves", dict(session_id=sid, duration_secs=1, watts=650))
            insert(c, "session_power_curves", dict(session_id=sid, duration_secs=60, watts=320))
    c.execute("DELETE FROM user_config")
    insert(c, "user_config", dict(
        id=1, ftp=250, weight_kg=72.5, hr_zone_1=125, hr_zone_2=145, hr_zone_3=160,
        hr_zone_4=172, hr_zone_5=185, units="imperial", power_zone_1=56, power_zone_2=76,
        power_zone_3=91, power_zone_4=106, power_zone_5=121, power_zone_6=151,
        date_of_birth="1985-04-02", sex="male", resting_hr=48, max_hr=186,
    ))
    insert(c, "known_devices", dict(
        id="ble-AA:BB", name="KICKR", device_type="Power", transport="Ble", rssi=-61,
        battery_level=90, last_seen="2026-02-15T08:30:00Z", manufacturer="Wahoo",
        model_number="KICKR5", serial_number="SN1", device_group="grp-1",
    ))
    c.close()


def dump(db, out):
    c = sqlite3.connect(db)
    with open(out, "w", newline="\n") as f:
        for line in c.iterdump():
            f.write(line + "\n")


if __name__ == "__main__":
    if sys.argv[1] == "build":
        build(*sys.argv[2:5])
    else:
        dump(*sys.argv[2:4])
