//! Ordered, idempotent schema migrations keyed by `PRAGMA user_version`.
//!
//! This is the ONLY place the schema is defined. Each entry in [`MIGRATIONS`] is a
//! `.sql` file under `src-tauri/migrations/`, applied once, in order, inside a
//! `BEGIN IMMEDIATE` transaction that also bumps `user_version`.
//!
//! * `user_version = 0` means "legacy or new": every release before this runner
//!   left it at 0. Step 1 (`0001_baseline.sql`) replays what those releases ran
//!   at startup, so it converges any shipped DB to the same column set.
//! * Every `ALTER TABLE <t> ADD COLUMN <c> ...` is skipped when the column already
//!   exists (`pragma_table_info`), so the baseline is safe on any prior state.
//! * Before the first pending step >= 2 on a non-empty DB, the DB is copied with
//!   `VACUUM INTO <db>.pre-v<N>` (never overwriting an existing backup).
//!
//! SQL file rules (enforced by tests): statements end in `;`, no `;` inside string
//! literals, `--` comments on their own lines, `ADD COLUMN` spelled out with
//! unquoted identifiers. Shipped files are immutable (SHA-256 pinned in tests).
//! Migrations are additive only: no DROP, RENAME, or NOT NULL without DEFAULT.

use log::{info, warn};
use sqlx::sqlite::SqlitePool;
use sqlx::SqliteConnection;
use std::path::{Path, PathBuf};

use crate::error::AppError;

pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_baseline.sql",
        sql: include_str!("../../../migrations/0001_baseline.sql"),
    },
    Migration {
        version: 2,
        name: "0002_session_zones.sql",
        sql: include_str!("../../../migrations/0002_session_zones.sql"),
    },
    Migration {
        version: 3,
        name: "0003_app_meta.sql",
        sql: include_str!("../../../migrations/0003_app_meta.sql"),
    },
];

/// Split a migration file into statements: drop `--` comment lines, split on `;`.
pub(crate) fn split_statements(sql: &str) -> Vec<String> {
    let without_comments: String = sql
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// If `stmt` is `ALTER TABLE <table> ADD COLUMN <column> ...`, return (table, column).
pub(crate) fn parse_add_column(stmt: &str) -> Option<(String, String)> {
    let t: Vec<&str> = stmt.split_whitespace().collect();
    if t.len() >= 6
        && t[0].eq_ignore_ascii_case("ALTER")
        && t[1].eq_ignore_ascii_case("TABLE")
        && t[3].eq_ignore_ascii_case("ADD")
        && t[4].eq_ignore_ascii_case("COLUMN")
    {
        Some((t[2].to_string(), t[5].to_string()))
    } else {
        None
    }
}

async fn user_version(conn: &mut SqliteConnection) -> Result<i64, AppError> {
    let (v,): (i64,) = sqlx::query_as("PRAGMA user_version")
        .fetch_one(conn)
        .await
        .map_err(AppError::Database)?;
    Ok(v)
}

async fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, AppError> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
            .bind(table)
            .bind(column)
            .fetch_one(conn)
            .await
            .map_err(AppError::Database)?;
    Ok(n > 0)
}

fn backup_path(db_path: &Path, version: i64) -> PathBuf {
    let mut name = db_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "training.db".into());
    name.push(format!(".pre-v{}", version));
    db_path.with_file_name(name)
}

/// Apply all pending [`MIGRATIONS`]. Returns the resulting `user_version`.
pub(crate) async fn migrate(pool: &SqlitePool, db_path: &Path) -> Result<i64, AppError> {
    migrate_with(pool, db_path, MIGRATIONS).await
}

pub(crate) async fn migrate_with(
    pool: &SqlitePool,
    db_path: &Path,
    migrations: &[Migration],
) -> Result<i64, AppError> {
    for (i, m) in migrations.iter().enumerate() {
        if m.version != i as i64 + 1 {
            return Err(AppError::Session(format!(
                "Migration list out of order: {} has version {}",
                m.name, m.version
            )));
        }
    }
    let latest = migrations.len() as i64;

    let mut conn = pool.acquire().await.map_err(AppError::Database)?;
    let current = user_version(&mut conn).await?;
    let (user_tables,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    drop(conn);

    if current > latest {
        warn!(
            "Database schema version {} is newer than this build ({}); continuing without migrating",
            current, latest
        );
        return Ok(current);
    }

    let mut backed_up = user_tables == 0;
    for m in migrations.iter().filter(|m| m.version > current) {
        if m.version >= 2 && !backed_up {
            let backup = backup_path(db_path, m.version);
            if backup.exists() {
                info!("Pre-migration backup already exists: {}", backup.display());
            } else {
                sqlx::query("VACUUM INTO ?")
                    .bind(backup.to_string_lossy().to_string())
                    .execute(pool)
                    .await
                    .map_err(AppError::Database)?;
                info!("Pre-migration backup written: {}", backup.display());
            }
            backed_up = true;
        }

        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AppError::Database)?;
        // Another process may have applied this step while we waited for the lock.
        if user_version(&mut tx).await? >= m.version {
            tx.rollback().await.map_err(AppError::Database)?;
            continue;
        }
        for stmt in split_statements(m.sql) {
            if let Some((table, column)) = parse_add_column(&stmt) {
                if column_exists(&mut tx, &table, &column).await? {
                    continue;
                }
            }
            sqlx::raw_sql(&stmt)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
        }
        // `version` is an integer constant from MIGRATIONS, not user input.
        sqlx::raw_sql(&format!("PRAGMA user_version = {}", m.version))
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        info!("Applied migration {}", m.name);
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::storage::Storage;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::str::FromStr;

    /// Every database state the app has ever shipped (plus latest main).
    /// RULE: every new migration step must keep passing against all of these.
    const FIXTURES: &[(&str, &str)] = &[
        ("legacy_initial_e0d85f4", include_str!("testdata/legacy_initial_e0d85f4.sql")),
        ("legacy_v0_6_10", include_str!("testdata/legacy_v0_6_10.sql")),
        ("main_3b10a73", include_str!("testdata/main_3b10a73.sql")),
    ];

    /// SHA-256 (LF-normalized) of every shipped migration. Editing a shipped step
    /// must fail CI; adding a step means adding its hash here.
    const PINNED_SHA256: &[(&str, &str)] = &[
        ("0001_baseline.sql", "769b855e56e288f385b6cd859545e7e051b626271f63e3cfeb741d6798f85622"),
        ("0002_session_zones.sql", "660fde829995dbfaef556bad80398555caa04f0e32618cdcc7dbb0781d40bfb9"),
        ("0003_app_meta.sql", "e19885e292e06f1c5e4c71e03d44d3a3179ed0c00d5071d2da5c71410ff4cbad"),
    ];

    async fn open_pool(path: &Path) -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
            "sqlite:{}?mode=rwc",
            path.display()
        ))
        .unwrap();
        SqlitePool::connect_with(opts).await.unwrap()
    }

    /// Write a fixture dump to `<dir>/training.db`.
    async fn load_fixture(dir: &Path, dump: &str) {
        let pool = open_pool(&dir.join("training.db")).await;
        sqlx::raw_sql(dump).execute(&pool).await.unwrap();
        pool.close().await;
    }

    type ColumnSets = BTreeMap<String, BTreeSet<(String, String, i64, Option<String>)>>;

    async fn column_sets(pool: &SqlitePool) -> ColumnSets {
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        let mut out = BTreeMap::new();
        for (t,) in tables {
            let cols: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
                "SELECT name, type, \"notnull\", dflt_value FROM pragma_table_info(?)",
            )
            .bind(&t)
            .fetch_all(pool)
            .await
            .unwrap();
            out.insert(t, cols.into_iter().collect());
        }
        out
    }

    /// Every (table, column) → all values in rowid order, SQL-quoted.
    async fn all_values(pool: &SqlitePool) -> BTreeMap<(String, String), String> {
        let mut out = BTreeMap::new();
        for (table, cols) in column_sets(pool).await {
            for (col, ..) in cols {
                let q = format!(
                    "SELECT COALESCE(group_concat(v, ','), '') FROM \
                     (SELECT quote(\"{col}\") AS v FROM \"{table}\" ORDER BY rowid)"
                );
                let (v,): (String,) = sqlx::query_as(&q).fetch_one(pool).await.unwrap();
                out.insert((table.clone(), col), v);
            }
        }
        out
    }

    async fn version_of(pool: &SqlitePool) -> i64 {
        let (v,): (i64,) = sqlx::query_as("PRAGMA user_version")
            .fetch_one(pool)
            .await
            .unwrap();
        v
    }

    async fn fresh_column_sets() -> ColumnSets {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        column_sets(&storage.pool).await
    }

    #[tokio::test]
    async fn fresh_db_reaches_latest_user_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        assert_eq!(version_of(&storage.pool).await, MIGRATIONS.len() as i64);
        let cols = column_sets(&storage.pool).await;
        assert!(cols["sessions"].iter().any(|c| c.0 == "zones"));
        assert!(cols.contains_key("app_meta"));
    }

    #[tokio::test]
    async fn fresh_db_gets_no_backup_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        let backups: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".pre-v"))
            .collect();
        assert!(backups.is_empty(), "fresh install must not create backups");
    }

    #[tokio::test]
    async fn reopening_migrated_db_changes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_string_lossy().to_string();
        let first = Storage::new(&dir).await.unwrap();
        let before = (column_sets(&first.pool).await, all_values(&first.pool).await);
        first.pool.close().await;
        let second = Storage::new(&dir).await.unwrap();
        let after = (column_sets(&second.pool).await, all_values(&second.pool).await);
        assert_eq!(before, after);
        assert_eq!(version_of(&second.pool).await, MIGRATIONS.len() as i64);
    }

    #[tokio::test]
    async fn every_fixture_upgrades_preserving_all_values() {
        for (name, dump) in FIXTURES {
            let tmp = tempfile::TempDir::new().unwrap();
            load_fixture(tmp.path(), dump).await;
            let pool = open_pool(&tmp.path().join("training.db")).await;
            let before = all_values(&pool).await;
            assert_eq!(version_of(&pool).await, 0, "{name}: fixture must be pre-runner");
            pool.close().await;

            let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
            let after = all_values(&storage.pool).await;
            for (key, value) in &before {
                assert_eq!(after.get(key), Some(value), "{name}: {key:?} changed");
            }
            assert_eq!(
                version_of(&storage.pool).await,
                MIGRATIONS.len() as i64,
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn every_fixture_upgrades_to_fresh_column_sets() {
        let fresh = fresh_column_sets().await;
        for (name, dump) in FIXTURES {
            let tmp = tempfile::TempDir::new().unwrap();
            load_fixture(tmp.path(), dump).await;
            let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
            assert_eq!(column_sets(&storage.pool).await, fresh, "{name}");
        }
    }

    #[tokio::test]
    async fn upgraded_main_fixture_is_readable_through_storage_api() {
        let tmp = tempfile::TempDir::new().unwrap();
        load_fixture(tmp.path(), FIXTURES[2].1).await;
        let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        let s = storage
            .get_session("22222222-2222-4222-8222-222222222222")
            .await
            .unwrap();
        assert_eq!(s.duration_secs, 3601);
        assert_eq!(s.avg_power, Some(181));
        assert_eq!(s.notes.as_deref(), Some("it's \"quoted\"; with semicolon"));
        let cfg = storage.get_user_config().await.unwrap();
        assert_eq!(cfg.ftp, 250);
        assert_eq!(cfg.power_zones, [56, 76, 91, 106, 121, 151]);
        let best = storage.get_best_power_curve(None).await.unwrap();
        assert_eq!(best.len(), 2);
        assert_eq!(best[0].watts, 650);
    }

    #[tokio::test]
    async fn legacy_upgrade_writes_one_backup_before_step_two() {
        let tmp = tempfile::TempDir::new().unwrap();
        load_fixture(tmp.path(), FIXTURES[1].1).await;
        let _storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        let backup = tmp.path().join("training.db.pre-v2");
        assert!(backup.exists());
        assert!(!tmp.path().join("training.db.pre-v3").exists());
        // Backup holds the user's sessions
        let pool = open_pool(&backup).await;
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn existing_backup_is_not_overwritten() {
        let tmp = tempfile::TempDir::new().unwrap();
        load_fixture(tmp.path(), FIXTURES[1].1).await;
        let backup = tmp.path().join("training.db.pre-v2");
        std::fs::write(&backup, b"older backup").unwrap();
        let _storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), b"older backup");
    }

    #[tokio::test]
    async fn failed_step_rolls_back_and_keeps_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("training.db");
        let pool = open_pool(&db).await;
        let steps = [
            Migration { version: 1, name: "one", sql: "CREATE TABLE a (x INTEGER);" },
            Migration {
                version: 2,
                name: "two",
                sql: "CREATE TABLE b (y INTEGER);\nINSERT INTO missing_table VALUES (1);",
            },
        ];
        assert!(migrate_with(&pool, &db, &steps).await.is_err());
        assert_eq!(version_of(&pool).await, 1);
        let (b_exists,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE name = 'b'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(b_exists, 0, "partial step 2 must be rolled back");
    }

    #[tokio::test]
    async fn add_column_is_skipped_when_column_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("training.db");
        let pool = open_pool(&db).await;
        sqlx::raw_sql("CREATE TABLE t (a INTEGER, b TEXT)").execute(&pool).await.unwrap();
        let steps = [Migration {
            version: 1,
            name: "one",
            sql: "ALTER TABLE t ADD COLUMN b TEXT;\nALTER TABLE t ADD COLUMN c REAL;",
        }];
        assert_eq!(migrate_with(&pool, &db, &steps).await.unwrap(), 1);
        let cols = column_sets(&pool).await;
        let names: Vec<_> = cols["t"].iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn newer_user_version_is_left_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_string_lossy().to_string();
        let s = Storage::new(&dir).await.unwrap();
        sqlx::raw_sql("PRAGMA user_version = 99").execute(&s.pool).await.unwrap();
        s.pool.close().await;
        let s = Storage::new(&dir).await.unwrap();
        assert_eq!(version_of(&s.pool).await, 99);
    }

    #[test]
    fn split_statements_drops_comment_lines_and_empty_chunks() {
        let sql = "-- header; with semicolon\nCREATE TABLE a (x);\n\n  -- note\nALTER TABLE a ADD COLUMN y TEXT;\n";
        assert_eq!(
            split_statements(sql),
            vec!["CREATE TABLE a (x)", "ALTER TABLE a ADD COLUMN y TEXT"]
        );
    }

    #[test]
    fn parse_add_column_extracts_table_and_column() {
        assert_eq!(
            parse_add_column("ALTER TABLE sessions ADD COLUMN zones TEXT"),
            Some(("sessions".into(), "zones".into()))
        );
        assert_eq!(
            parse_add_column("alter  table t\n add column c INTEGER NOT NULL DEFAULT 5"),
            Some(("t".into(), "c".into()))
        );
        assert_eq!(parse_add_column("ALTER TABLE t RENAME TO u"), None);
        assert_eq!(parse_add_column("CREATE TABLE t (a)"), None);
    }

    #[test]
    fn baseline_statement_count_is_pinned() {
        // 3 CREATE + 1 INSERT + 24 ALTER + 1 CREATE (power curves)
        assert_eq!(split_statements(MIGRATIONS[0].sql).len(), 29);
    }

    #[test]
    fn every_add_column_spells_out_column_with_plain_identifiers() {
        for m in MIGRATIONS {
            for stmt in split_statements(m.sql) {
                let upper = stmt.to_ascii_uppercase();
                if upper.starts_with("ALTER TABLE") && upper.contains(" ADD ") {
                    let (t, c) = parse_add_column(&stmt)
                        .unwrap_or_else(|| panic!("{}: unparseable ALTER: {stmt}", m.name));
                    for ident in [t, c] {
                        assert!(
                            ident.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
                            "{}: quoted/odd identifier in {stmt}",
                            m.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn migration_files_match_registry() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        on_disk.sort();
        let registered: Vec<String> = MIGRATIONS.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(on_disk, registered, "every migrations/ file must be registered, in order");
        for m in MIGRATIONS {
            assert!(
                m.name.starts_with(&format!("{:04}_", m.version)),
                "{} must be prefixed with its version",
                m.name
            );
        }
    }

    #[test]
    fn shipped_migrations_match_pinned_sha256() {
        assert_eq!(PINNED_SHA256.len(), MIGRATIONS.len(), "pin every migration");
        for (m, (name, pinned)) in MIGRATIONS.iter().zip(PINNED_SHA256) {
            assert_eq!(m.name, *name);
            let normalized = m.sql.replace("\r\n", "\n");
            let digest = Sha256::digest(normalized.as_bytes());
            let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
            assert_eq!(&hex, pinned, "{} was edited after shipping", m.name);
        }
    }

    #[test]
    fn backup_path_appends_version_suffix() {
        assert_eq!(
            backup_path(Path::new("/data/training.db"), 4),
            PathBuf::from("/data/training.db.pre-v4")
        );
    }
}
