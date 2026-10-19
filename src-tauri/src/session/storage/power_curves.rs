use super::Storage;
use crate::error::AppError;
use crate::session::analysis::PowerCurvePoint;

/// Replace a session's cached curve inside an open transaction: delete every
/// old row first so durations absent from the new curve can't keep stale bests.
pub(super) async fn replace_power_curve_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: &str,
    curve: &[PowerCurvePoint],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM session_power_curves WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
    for point in curve {
        sqlx::query(
            "INSERT INTO session_power_curves (session_id, duration_secs, watts) VALUES (?, ?, ?)",
        )
        .bind(session_id)
        .bind(point.duration_secs as i32)
        .bind(point.watts as i32)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
    }
    Ok(())
}

impl Storage {
    /// Atomically replace the session's power-curve cache (delete + insert in
    /// one transaction, so concurrent writers can't interleave rows).
    pub async fn save_power_curve(
        &self,
        session_id: &str,
        curve: &[PowerCurvePoint],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        replace_power_curve_in_tx(&mut tx, session_id, curve).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn get_best_power_curve(
        &self,
        after_date: Option<&str>,
    ) -> Result<Vec<PowerCurvePoint>, AppError> {
        let rows: Vec<(i32, i32)> = if let Some(date) = after_date {
            sqlx::query_as(
                "SELECT pc.duration_secs, MAX(pc.watts) as watts \
                 FROM session_power_curves pc \
                 JOIN sessions s ON s.id = pc.session_id \
                 WHERE s.start_time >= ? \
                 GROUP BY pc.duration_secs \
                 ORDER BY pc.duration_secs",
            )
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?
        } else {
            sqlx::query_as(
                "SELECT duration_secs, MAX(watts) as watts \
                 FROM session_power_curves \
                 GROUP BY duration_secs \
                 ORDER BY duration_secs",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?
        };
        Ok(rows
            .into_iter()
            .map(|(d, w)| PowerCurvePoint {
                duration_secs: d as u32,
                watts: w as u16,
            })
            .collect())
    }

    pub async fn has_power_curve(&self, session_id: &str) -> Result<bool, AppError> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM session_power_curves WHERE session_id = ? LIMIT 1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(row.is_some())
    }
}
