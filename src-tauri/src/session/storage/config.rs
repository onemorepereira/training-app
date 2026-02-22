use super::Storage;
use crate::error::AppError;
use crate::session::types::SessionConfig;

#[derive(sqlx::FromRow)]
struct ConfigRow {
    ftp: i32,
    weight_kg: f64,
    hr_zone_1: i32,
    hr_zone_2: i32,
    hr_zone_3: i32,
    hr_zone_4: i32,
    hr_zone_5: i32,
    units: String,
    power_zone_1: i32,
    power_zone_2: i32,
    power_zone_3: i32,
    power_zone_4: i32,
    power_zone_5: i32,
    power_zone_6: i32,
    date_of_birth: Option<String>,
    sex: Option<String>,
    resting_hr: Option<i32>,
    max_hr: Option<i32>,
}

impl Storage {
    pub async fn get_user_config(&self) -> Result<SessionConfig, AppError> {
        let row = sqlx::query_as::<_, ConfigRow>(
            "SELECT ftp, weight_kg, hr_zone_1, hr_zone_2, hr_zone_3, hr_zone_4, hr_zone_5, \
             units, power_zone_1, power_zone_2, power_zone_3, power_zone_4, power_zone_5, \
             power_zone_6, date_of_birth, sex, resting_hr, max_hr \
             FROM user_config WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(SessionConfig {
            ftp: row.ftp as u16,
            weight_kg: row.weight_kg as f32,
            hr_zones: [
                row.hr_zone_1 as u8,
                row.hr_zone_2 as u8,
                row.hr_zone_3 as u8,
                row.hr_zone_4 as u8,
                row.hr_zone_5 as u8,
            ],
            units: row.units,
            power_zones: [
                row.power_zone_1 as u16,
                row.power_zone_2 as u16,
                row.power_zone_3 as u16,
                row.power_zone_4 as u16,
                row.power_zone_5 as u16,
                row.power_zone_6 as u16,
            ],
            date_of_birth: row.date_of_birth,
            sex: row.sex,
            resting_hr: row.resting_hr.map(|v| v as u8),
            max_hr: row.max_hr.map(|v| v as u8),
        })
    }

    pub async fn save_user_config(&self, config: &SessionConfig) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_config (id, ftp, weight_kg, hr_zone_1, hr_zone_2, hr_zone_3, \
             hr_zone_4, hr_zone_5, units, power_zone_1, power_zone_2, power_zone_3, \
             power_zone_4, power_zone_5, power_zone_6, date_of_birth, sex, resting_hr, max_hr) \
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
             ftp = excluded.ftp, weight_kg = excluded.weight_kg, \
             hr_zone_1 = excluded.hr_zone_1, hr_zone_2 = excluded.hr_zone_2, \
             hr_zone_3 = excluded.hr_zone_3, hr_zone_4 = excluded.hr_zone_4, \
             hr_zone_5 = excluded.hr_zone_5, units = excluded.units, \
             power_zone_1 = excluded.power_zone_1, power_zone_2 = excluded.power_zone_2, \
             power_zone_3 = excluded.power_zone_3, power_zone_4 = excluded.power_zone_4, \
             power_zone_5 = excluded.power_zone_5, power_zone_6 = excluded.power_zone_6, \
             date_of_birth = excluded.date_of_birth, sex = excluded.sex, \
             resting_hr = excluded.resting_hr, max_hr = excluded.max_hr",
        )
        .bind(config.ftp as i32)
        .bind(config.weight_kg as f64)
        .bind(config.hr_zones[0] as i32)
        .bind(config.hr_zones[1] as i32)
        .bind(config.hr_zones[2] as i32)
        .bind(config.hr_zones[3] as i32)
        .bind(config.hr_zones[4] as i32)
        .bind(&config.units)
        .bind(config.power_zones[0] as i32)
        .bind(config.power_zones[1] as i32)
        .bind(config.power_zones[2] as i32)
        .bind(config.power_zones[3] as i32)
        .bind(config.power_zones[4] as i32)
        .bind(config.power_zones[5] as i32)
        .bind(&config.date_of_birth)
        .bind(&config.sex)
        .bind(config.resting_hr.map(|v| v as i32))
        .bind(config.max_hr.map(|v| v as i32))
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}
