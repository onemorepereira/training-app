use log::warn;

use super::Storage;
use crate::device::types::{ConnectionStatus, DeviceInfo, DeviceType, Transport};
use crate::error::AppError;

#[derive(sqlx::FromRow)]
struct KnownDeviceRow {
    id: String,
    name: Option<String>,
    device_type: String,
    transport: String,
    rssi: Option<i32>,
    battery_level: Option<i32>,
    last_seen: String,
    manufacturer: Option<String>,
    model_number: Option<String>,
    serial_number: Option<String>,
    device_group: Option<String>,
}

impl From<KnownDeviceRow> for DeviceInfo {
    fn from(row: KnownDeviceRow) -> Self {
        let device_type = match row.device_type.as_str() {
            "HeartRate" => DeviceType::HeartRate,
            "Power" => DeviceType::Power,
            "CadenceSpeed" => DeviceType::CadenceSpeed,
            "FitnessTrainer" => DeviceType::FitnessTrainer,
            other => {
                warn!("Unknown device_type '{}' for device '{}', defaulting to HeartRate", other, row.id);
                DeviceType::HeartRate
            }
        };
        let transport = match row.transport.as_str() {
            "AntPlus" => Transport::AntPlus,
            _ => Transport::Ble,
        };
        Self {
            id: row.id,
            name: row.name,
            device_type,
            status: ConnectionStatus::Disconnected,
            transport,
            rssi: row.rssi.map(|v| v as i16),
            battery_level: row.battery_level.map(|v| v as u8),
            last_seen: Some(row.last_seen),
            manufacturer: row.manufacturer,
            model_number: row.model_number,
            serial_number: row.serial_number,
            device_group: row.device_group,
            in_range: true,
        }
    }
}

impl Storage {
    #[cfg(test)]
    pub async fn upsert_known_device(&self, device: &DeviceInfo) -> Result<(), AppError> {
        let device_type = device.device_type.as_str();
        let transport = device.transport.as_str();
        let last_seen = device
            .last_seen
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        sqlx::query(
            "INSERT INTO known_devices (id, name, device_type, transport, rssi, battery_level, \
             last_seen, manufacturer, model_number, serial_number, device_group) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = COALESCE(excluded.name, known_devices.name), \
               rssi = COALESCE(excluded.rssi, known_devices.rssi), \
               battery_level = COALESCE(excluded.battery_level, known_devices.battery_level), \
               last_seen = excluded.last_seen, \
               manufacturer = COALESCE(excluded.manufacturer, known_devices.manufacturer), \
               model_number = COALESCE(excluded.model_number, known_devices.model_number), \
               serial_number = COALESCE(excluded.serial_number, known_devices.serial_number), \
               device_group = COALESCE(excluded.device_group, known_devices.device_group)",
        )
        .bind(&device.id)
        .bind(&device.name)
        .bind(&device_type)
        .bind(&transport)
        .bind(device.rssi.map(|v| v as i32))
        .bind(device.battery_level.map(|v| v as i32))
        .bind(&last_seen)
        .bind(&device.manufacturer)
        .bind(&device.model_number)
        .bind(&device.serial_number)
        .bind(&device.device_group)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn upsert_known_devices_batch(&self, devices: &[DeviceInfo]) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        for device in devices {
            let device_type = device.device_type.as_str();
            let transport = device.transport.as_str();
            let last_seen = device
                .last_seen
                .clone()
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            sqlx::query(
                "INSERT INTO known_devices (id, name, device_type, transport, rssi, battery_level, \
                 last_seen, manufacturer, model_number, serial_number, device_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                   name = COALESCE(excluded.name, known_devices.name), \
                   rssi = COALESCE(excluded.rssi, known_devices.rssi), \
                   battery_level = COALESCE(excluded.battery_level, known_devices.battery_level), \
                   last_seen = excluded.last_seen, \
                   manufacturer = COALESCE(excluded.manufacturer, known_devices.manufacturer), \
                   model_number = COALESCE(excluded.model_number, known_devices.model_number), \
                   serial_number = COALESCE(excluded.serial_number, known_devices.serial_number), \
                   device_group = COALESCE(excluded.device_group, known_devices.device_group)",
            )
            .bind(&device.id)
            .bind(&device.name)
            .bind(&device_type)
            .bind(&transport)
            .bind(device.rssi.map(|v| v as i32))
            .bind(device.battery_level.map(|v| v as i32))
            .bind(&last_seen)
            .bind(&device.manufacturer)
            .bind(&device.model_number)
            .bind(&device.serial_number)
            .bind(&device.device_group)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        }
        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn clear_device_group(&self, device_id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE known_devices SET device_group = NULL WHERE id = ?")
            .bind(device_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn list_known_devices(&self) -> Result<Vec<DeviceInfo>, AppError> {
        let rows = sqlx::query_as::<_, KnownDeviceRow>(
            "SELECT id, name, device_type, transport, rssi, battery_level, last_seen, \
             manufacturer, model_number, serial_number, device_group \
             FROM known_devices ORDER BY last_seen DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
}
