use super::Storage;
use crate::error::AppError;

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
