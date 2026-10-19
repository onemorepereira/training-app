use log::warn;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::ConnectOptions;
use std::path::Path;

use super::Storage;
use crate::error::AppError;

/// True when `path` is a readable SQLite database that passes
/// `PRAGMA quick_check`. Opens it read-only; any error counts as a failure.
pub async fn verify_sqlite_file(path: &Path) -> bool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .disable_statement_logging();
    let pool = match SqlitePoolOptions::new().max_connections(1).connect_with(options).await {
        Ok(p) => p,
        Err(e) => {
            warn!("Backup check: cannot open {}: {}", path.display(), e);
            return false;
        }
    };
    let result: Result<Vec<(String,)>, _> =
        sqlx::query_as("PRAGMA quick_check").fetch_all(&pool).await;
    pool.close().await;
    match result {
        Ok(rows) if rows.len() == 1 && rows[0].0 == "ok" => true,
        Ok(rows) => {
            warn!("Backup check: quick_check failed for {}: {:?}", path.display(), rows);
            false
        }
        Err(e) => {
            warn!("Backup check: quick_check error for {}: {}", path.display(), e);
            false
        }
    }
}

impl Storage {
    /// Read an `app_meta` value; `None` when the key is absent.
    pub async fn get_meta(&self, key: &str) -> Result<Option<String>, AppError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT value FROM app_meta WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(row.and_then(|(v,)| v))
    }

    /// Write a consistent single-file copy of the database to `path` with
    /// `VACUUM INTO` (SQLite ≥ 3.27; bundled 3.46). Unlike a file copy this is
    /// safe while other pool connections write. Fails if `path` exists.
    pub async fn backup_db_to(&self, path: &std::path::Path) -> Result<(), AppError> {
        sqlx::query("VACUUM INTO ?")
            .bind(path.to_string_lossy().to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    /// Insert or overwrite an `app_meta` value.
    pub async fn set_meta(&self, key: &str, value: &str) -> Result<(), AppError> {
        sqlx::query("INSERT OR REPLACE INTO app_meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }
}
