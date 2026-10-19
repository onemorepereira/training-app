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
