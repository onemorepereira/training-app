use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid as BtUuid;

use super::types::{
    CharacteristicInfo, ConnectionStatus, DeviceDetails, DeviceInfo, DeviceType, ServiceInfo,
    Transport,
};
use crate::error::AppError;

const HEART_RATE_SERVICE: BtUuid = BtUuid::from_u128(0x0000180D_0000_1000_8000_00805f9b34fb);
const CYCLING_POWER_SERVICE: BtUuid = BtUuid::from_u128(0x00001818_0000_1000_8000_00805f9b34fb);
const CSC_SERVICE: BtUuid = BtUuid::from_u128(0x00001816_0000_1000_8000_00805f9b34fb);
const FTMS_SERVICE: BtUuid = BtUuid::from_u128(0x00001826_0000_1000_8000_00805f9b34fb);
const BATTERY_LEVEL_CHAR: BtUuid = BtUuid::from_u128(0x00002A19_0000_1000_8000_00805f9b34fb);

// Device Information Service characteristics
const DIS_MANUFACTURER: BtUuid = BtUuid::from_u128(0x00002A29_0000_1000_8000_00805f9b34fb);
const DIS_MODEL_NUMBER: BtUuid = BtUuid::from_u128(0x00002A24_0000_1000_8000_00805f9b34fb);
const DIS_SERIAL_NUMBER: BtUuid = BtUuid::from_u128(0x00002A25_0000_1000_8000_00805f9b34fb);
const DIS_FIRMWARE_REV: BtUuid = BtUuid::from_u128(0x00002A26_0000_1000_8000_00805f9b34fb);
const DIS_HARDWARE_REV: BtUuid = BtUuid::from_u128(0x00002A27_0000_1000_8000_00805f9b34fb);
const DIS_SOFTWARE_REV: BtUuid = BtUuid::from_u128(0x00002A28_0000_1000_8000_00805f9b34fb);

pub struct BleManager {
    adapter: Adapter,
    discovered: Arc<Mutex<HashMap<String, (Peripheral, DeviceInfo)>>>,
    connected: Arc<Mutex<HashMap<String, Peripheral>>>,
}

impl BleManager {
    pub async fn new() -> Result<Self, AppError> {
        let manager = Manager::new()
            .await
            .map_err(|e| AppError::Ble(format!("Failed to create BLE manager: {}", e)))?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(format!("Failed to get adapters: {}", e)))?;
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Ble("No BLE adapter found".into()))?;
        Ok(Self {
            adapter,
            discovered: Arc::new(Mutex::new(HashMap::new())),
            connected: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn start_scan(&self) -> Result<(), AppError> {
        self.adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| AppError::Ble(format!("Failed to start scan: {}", e)))
    }

    pub async fn stop_scan(&self) -> Result<(), AppError> {
        self.adapter
            .stop_scan()
            .await
            .map_err(|e| AppError::Ble(format!("Failed to stop scan: {}", e)))
    }

    pub async fn get_discovered_devices(&self) -> Result<Vec<DeviceInfo>, AppError> {
        let peripherals = self
            .adapter
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(format!("Failed to get peripherals: {}", e)))?;

        let mut devices = Vec::new();
        let mut discovered = self.discovered.lock().await;
        let connected = self.connected.lock().await;

        // Rebuild discovered map from current adapter peripherals.
        // Retain connected devices even if they're not in the current adapter list.
        let connected_ids: std::collections::HashSet<&String> = connected.keys().collect();
        discovered.retain(|id, _| connected_ids.contains(id));

        for peripheral in peripherals {
            let properties = peripheral
                .properties()
                .await
                .map_err(|e| AppError::Ble(format!("Failed to get properties: {}", e)))?;
            let Some(properties) = properties else {
                continue;
            };
            let id = peripheral.id().to_string();
            let device_type = classify_device(&properties.services);
            let Some(device_type) = device_type else {
                continue;
            };
            let info = DeviceInfo {
                id: id.clone(),
                name: properties.local_name.clone(),
                device_type,
                status: ConnectionStatus::Disconnected,
                transport: Transport::Ble,
                rssi: properties.rssi,
                battery_level: None,
                last_seen: Some(chrono::Utc::now().to_rfc3339()),
                manufacturer: None,
                model_number: None,
                serial_number: None,
                device_group: None,
            };
            discovered.insert(id, (peripheral, info.clone()));
            devices.push(info);
        }
        Ok(devices)
    }

    pub async fn connect_device(&self, device_id: &str) -> Result<DeviceInfo, AppError> {
        // Try to get the peripheral + info from the discovered map first
        let entry = self.discovered.lock().await.get(device_id).cloned();

        let (peripheral, mut info) = if let Some(entry) = entry {
            entry
        } else {
            // Device not in discovered map — find the peripheral directly.
            // btleplug tracks all peripherals the adapter has seen, so check there
            // before resorting to a full scan.
            let peripheral = self.find_peripheral(device_id).await?;

            // We don't have a classified DeviceInfo yet.  Connect + discover services
            // first, then classify from the actual GATT services (reliable, not ad-dependent).
            peripheral
                .connect()
                .await
                .map_err(|e| AppError::Ble(format!("Failed to connect: {}", e)))?;
            peripheral
                .discover_services()
                .await
                .map_err(|e| AppError::Ble(format!("Failed to discover services: {}", e)))?;

            let properties = peripheral.properties().await
                .map_err(|e| AppError::Ble(format!("Failed to get properties: {}", e)))?;
            let props = properties.unwrap_or_default();

            // Classify from actual GATT services (post-connection), not advertisement data
            let gatt_services: Vec<BtUuid> = peripheral.services().iter().map(|s| s.uuid).collect();
            let device_type = classify_device(&gatt_services)
                .or_else(|| classify_device(&props.services))
                .ok_or_else(|| AppError::Ble(format!(
                    "Device {} has no recognized services", device_id
                )))?;

            let battery_level = {
                let chars = peripheral.characteristics();
                if let Some(battery_char) = chars.iter().find(|c| c.uuid == BATTERY_LEVEL_CHAR) {
                    match peripheral.read(battery_char).await {
                        Ok(data) if !data.is_empty() => Some(data[0]),
                        _ => None,
                    }
                } else {
                    None
                }
            };

            let info = DeviceInfo {
                id: device_id.to_string(),
                name: props.local_name.clone(),
                device_type,
                status: ConnectionStatus::Connected,
                battery_level,
                transport: Transport::Ble,
                rssi: props.rssi,
                last_seen: Some(chrono::Utc::now().to_rfc3339()),
                manufacturer: None,
                model_number: None,
                serial_number: None,
                device_group: None,
            };

            // Cache in discovered for future use
            self.discovered.lock().await.insert(
                device_id.to_string(),
                (peripheral.clone(), info.clone()),
            );
            self.connected
                .lock()
                .await
                .insert(device_id.to_string(), peripheral);
            return Ok(info);
        };

        // Normal path: device was in discovered map, connect now.
        // If BlueZ evicted the D-Bus object (stale cache), retry with a fresh scan.
        if let Err(e) = peripheral.connect().await {
            let err_str = e.to_string();
            if err_str.contains("doesn't exist") || err_str.contains("does not exist") {
                log::warn!("[{}] Stale BlueZ handle, rescanning...", device_id);
                self.discovered.lock().await.remove(device_id);
                let fresh = self.find_peripheral(device_id).await?;
                fresh
                    .connect()
                    .await
                    .map_err(|e2| AppError::Ble(format!("Failed to connect after rescan: {}", e2)))?;
                fresh
                    .discover_services()
                    .await
                    .map_err(|e2| AppError::Ble(format!("Failed to discover services: {}", e2)))?;

                let battery_level = {
                    let chars = fresh.characteristics();
                    if let Some(battery_char) = chars.iter().find(|c| c.uuid == BATTERY_LEVEL_CHAR) {
                        match fresh.read(battery_char).await {
                            Ok(data) if !data.is_empty() => Some(data[0]),
                            _ => None,
                        }
                    } else {
                        None
                    }
                };

                info.status = ConnectionStatus::Connected;
                info.battery_level = battery_level;
                info.last_seen = Some(chrono::Utc::now().to_rfc3339());
                self.discovered.lock().await.insert(
                    device_id.to_string(),
                    (fresh.clone(), info.clone()),
                );
                self.connected
                    .lock()
                    .await
                    .insert(device_id.to_string(), fresh);
                return Ok(info);
            }
            return Err(AppError::Ble(format!("Failed to connect: {}", e)));
        }
        peripheral
            .discover_services()
            .await
            .map_err(|e| AppError::Ble(format!("Failed to discover services: {}", e)))?;

        let battery_level = {
            let chars = peripheral.characteristics();
            if let Some(battery_char) = chars.iter().find(|c| c.uuid == BATTERY_LEVEL_CHAR) {
                match peripheral.read(battery_char).await {
                    Ok(data) if !data.is_empty() => Some(data[0]),
                    _ => None,
                }
            } else {
                None
            }
        };

        info.status = ConnectionStatus::Connected;
        info.battery_level = battery_level;
        info.last_seen = Some(chrono::Utc::now().to_rfc3339());
        self.connected
            .lock()
            .await
            .insert(device_id.to_string(), peripheral);
        Ok(info)
    }

    /// Find a peripheral by ID. First checks adapter's cached peripherals,
    /// then does a scan if needed. This avoids depending on advertisement
    /// packets containing service UUIDs (which BLE devices don't always include).
    async fn find_peripheral(&self, device_id: &str) -> Result<Peripheral, AppError> {
        // Check if the adapter already knows about this peripheral
        if let Ok(peripherals) = self.adapter.peripherals().await {
            for p in &peripherals {
                if p.id().to_string() == device_id {
                    log::info!("[{}] Found peripheral in adapter cache", device_id);
                    return Ok(p.clone());
                }
            }
        }

        // Not cached — scan to find it
        log::info!("[{}] Peripheral not cached, scanning...", device_id);
        self.adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| AppError::Ble(format!("Failed to start scan: {}", e)))?;
        tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
        let _ = self.adapter.stop_scan().await;

        if let Ok(peripherals) = self.adapter.peripherals().await {
            for p in &peripherals {
                if p.id().to_string() == device_id {
                    log::info!("[{}] Found peripheral after scan", device_id);
                    return Ok(p.clone());
                }
            }
        }

        Err(AppError::DeviceNotFound(device_id.to_string()))
    }

    pub async fn disconnect_device(&self, device_id: &str) -> Result<(), AppError> {
        let mut connected = self.connected.lock().await;
        if let Some(peripheral) = connected.remove(device_id) {
            peripheral
                .disconnect()
                .await
                .map_err(|e| AppError::Ble(format!("Failed to disconnect: {}", e)))?;
        }
        Ok(())
    }

    pub fn get_connected(&self) -> Arc<Mutex<HashMap<String, Peripheral>>> {
        self.connected.clone()
    }

    /// Read detailed information from a connected BLE peripheral including
    /// GATT services, characteristics, and Device Information Service fields.
    pub async fn get_device_details(&self, device_id: &str) -> Result<DeviceDetails, AppError> {
        let connected = self.connected.lock().await;
        let peripheral = connected
            .get(device_id)
            .ok_or_else(|| AppError::DeviceNotFound(device_id.to_string()))?;

        let properties = peripheral.properties().await
            .map_err(|e| AppError::Ble(format!("Failed to get properties: {}", e)))?;
        let props = properties.unwrap_or_default();
        let characteristics = peripheral.characteristics();

        // Read Device Information Service string fields
        async fn read_dis_string(peripheral: &Peripheral, characteristics: &std::collections::BTreeSet<btleplug::api::Characteristic>, uuid: BtUuid) -> Option<String> {
            if let Some(c) = characteristics.iter().find(|c| c.uuid == uuid) {
                match peripheral.read(c).await {
                    Ok(data) => {
                        let s = String::from_utf8_lossy(&data).trim().to_string();
                        if s.is_empty() { None } else { Some(s) }
                    }
                    Err(_) => None,
                }
            } else {
                None
            }
        }

        let manufacturer = read_dis_string(peripheral, &characteristics, DIS_MANUFACTURER).await;
        let model_number = read_dis_string(peripheral, &characteristics, DIS_MODEL_NUMBER).await;
        let serial_number = read_dis_string(peripheral, &characteristics, DIS_SERIAL_NUMBER).await;
        let firmware_revision = read_dis_string(peripheral, &characteristics, DIS_FIRMWARE_REV).await;
        let hardware_revision = read_dis_string(peripheral, &characteristics, DIS_HARDWARE_REV).await;
        let software_revision = read_dis_string(peripheral, &characteristics, DIS_SOFTWARE_REV).await;

        let battery_level = if let Some(c) = characteristics.iter().find(|c| c.uuid == BATTERY_LEVEL_CHAR) {
            match peripheral.read(c).await {
                Ok(data) if !data.is_empty() => Some(data[0]),
                _ => None,
            }
        } else {
            None
        };

        // Build service/characteristic tree
        let gatt_services = peripheral.services();
        let mut services: Vec<ServiceInfo> = Vec::new();
        for service in &gatt_services {
            let mut chars_info: Vec<CharacteristicInfo> = Vec::new();
            for c in &service.characteristics {
                let mut char_props: Vec<String> = Vec::new();
                let p = c.properties;
                if p.contains(btleplug::api::CharPropFlags::READ) { char_props.push("Read".into()); }
                if p.contains(btleplug::api::CharPropFlags::WRITE) { char_props.push("Write".into()); }
                if p.contains(btleplug::api::CharPropFlags::WRITE_WITHOUT_RESPONSE) { char_props.push("WriteNoResp".into()); }
                if p.contains(btleplug::api::CharPropFlags::NOTIFY) { char_props.push("Notify".into()); }
                if p.contains(btleplug::api::CharPropFlags::INDICATE) { char_props.push("Indicate".into()); }
                if p.contains(btleplug::api::CharPropFlags::BROADCAST) { char_props.push("Broadcast".into()); }

                chars_info.push(CharacteristicInfo {
                    uuid: c.uuid.to_string(),
                    name: well_known_char_name(c.uuid),
                    properties: char_props,
                });
            }
            services.push(ServiceInfo {
                uuid: service.uuid.to_string(),
                name: well_known_service_name(service.uuid),
                characteristics: chars_info,
            });
        }

        // Classify device type from GATT services
        let gatt_uuids: Vec<BtUuid> = gatt_services.iter().map(|s| s.uuid).collect();
        let device_type = classify_device(&gatt_uuids)
            .or_else(|| classify_device(&props.services))
            .unwrap_or(DeviceType::HeartRate);

        Ok(DeviceDetails {
            id: device_id.to_string(),
            name: props.local_name,
            device_type,
            transport: Transport::Ble,
            rssi: props.rssi,
            battery_level,
            manufacturer,
            model_number,
            serial_number,
            firmware_revision,
            hardware_revision,
            software_revision,
            services,
        })
    }
}

fn classify_device(services: &[BtUuid]) -> Option<DeviceType> {
    if services.contains(&FTMS_SERVICE) {
        Some(DeviceType::FitnessTrainer)
    } else if services.contains(&CYCLING_POWER_SERVICE) {
        Some(DeviceType::Power)
    } else if services.contains(&HEART_RATE_SERVICE) {
        Some(DeviceType::HeartRate)
    } else if services.contains(&CSC_SERVICE) {
        Some(DeviceType::CadenceSpeed)
    } else {
        None
    }
}

fn well_known_service_name(uuid: BtUuid) -> Option<String> {
    // Extract the 16-bit short UUID from the standard Bluetooth base
    let val = (uuid.as_u128() >> 96) as u16;
    let name = match val {
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x180A => "Device Information",
        0x180D => "Heart Rate",
        0x180F => "Battery Service",
        0x1816 => "Cycling Speed and Cadence",
        0x1818 => "Cycling Power",
        0x1826 => "Fitness Machine",
        _ => return None,
    };
    Some(name.to_string())
}

fn well_known_char_name(uuid: BtUuid) -> Option<String> {
    let val = (uuid.as_u128() >> 96) as u16;
    let name = match val {
        0x2A00 => "Device Name",
        0x2A01 => "Appearance",
        0x2A04 => "Peripheral Preferred Connection Parameters",
        0x2A05 => "Service Changed",
        0x2A19 => "Battery Level",
        0x2A24 => "Model Number String",
        0x2A25 => "Serial Number String",
        0x2A26 => "Firmware Revision String",
        0x2A27 => "Hardware Revision String",
        0x2A28 => "Software Revision String",
        0x2A29 => "Manufacturer Name String",
        0x2A37 => "Heart Rate Measurement",
        0x2A38 => "Body Sensor Location",
        0x2A39 => "Heart Rate Control Point",
        0x2A5B => "CSC Measurement",
        0x2A5C => "CSC Feature",
        0x2A5D => "Sensor Location",
        0x2A63 => "Cycling Power Measurement",
        0x2A64 => "Cycling Power Vector",
        0x2A65 => "Cycling Power Feature",
        0x2A66 => "Cycling Power Control Point",
        0x2AD2 => "Indoor Bike Data",
        0x2AD3 => "Training Status",
        0x2AD6 => "Supported Resistance Level Range",
        0x2AD8 => "Supported Power Range",
        0x2AD9 => "Fitness Machine Control Point",
        0x2ADA => "Fitness Machine Status",
        0x2ACC => "Fitness Machine Feature",
        _ => return None,
    };
    Some(name.to_string())
}
