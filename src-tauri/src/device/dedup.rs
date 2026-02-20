use std::collections::HashMap;
use uuid::Uuid;

use super::types::{DeviceInfo, Transport};

/// Compute device groups for cross-transport deduplication.
///
/// Returns a map of device_id → group_id for devices that share the same
/// physical device across BLE and ANT+. Devices that don't match anything
/// are not included in the map.
///
/// Two-tier matching between BLE and ANT+ devices of the same device_type:
/// 1. Serial match: both have serial numbers, manufacturer matches, serials equal
/// 2. Name-number match: BLE device name contains the ANT+ device number,
///    and manufacturer matches if both are available
pub fn compute_device_groups(devices: &[DeviceInfo]) -> HashMap<String, String> {
    let mut groups: HashMap<String, String> = HashMap::new();

    let ble_devices: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.transport == Transport::Ble)
        .collect();
    let ant_devices: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.transport == Transport::AntPlus)
        .collect();

    for ble in &ble_devices {
        for ant in &ant_devices {
            // Must be same device type
            if ble.device_type != ant.device_type {
                continue;
            }

            // Skip if either is already grouped
            if groups.contains_key(&ble.id) || groups.contains_key(&ant.id) {
                continue;
            }

            if serial_match(ble, ant) || name_number_match(ble, ant) {
                let group_id = deterministic_group_id(&ble.id, &ant.id);
                groups.insert(ble.id.clone(), group_id.clone());
                groups.insert(ant.id.clone(), group_id);
            }
        }
    }

    groups
}

/// High-confidence match: both have non-sentinel serial numbers, and
/// manufacturers match (if both available).
fn serial_match(ble: &DeviceInfo, ant: &DeviceInfo) -> bool {
    let ble_serial = match &ble.serial_number {
        Some(s) if !s.is_empty() && s != "0" => s.as_str(),
        _ => return false,
    };
    let ant_serial = match &ant.serial_number {
        Some(s) if !s.is_empty() && s != "0" => s.as_str(),
        _ => return false,
    };

    if ble_serial != ant_serial {
        return false;
    }

    // If both have manufacturer info, they must match
    if let (Some(ble_mfr), Some(ant_mfr)) = (&ble.manufacturer, &ant.manufacturer) {
        if !manufacturers_match(ble_mfr, ant_mfr) {
            return false;
        }
    }

    true
}

/// Medium-confidence match: BLE device name contains the ANT+ device number
/// (extracted from the ant:{type}:{number} ID format).
fn name_number_match(ble: &DeviceInfo, ant: &DeviceInfo) -> bool {
    let ble_name = match &ble.name {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };

    let ant_number = match extract_ant_device_number(&ant.id) {
        Some(n) => n,
        None => return false,
    };

    if !ble_name.contains(&ant_number) {
        return false;
    }

    // If both have manufacturer info, they must match
    if let (Some(ble_mfr), Some(ant_mfr)) = (&ble.manufacturer, &ant.manufacturer) {
        if !manufacturers_match(ble_mfr, ant_mfr) {
            return false;
        }
    }

    true
}

/// Extract the device number from an ANT+ device ID like "ant:power:12345".
fn extract_ant_device_number(ant_id: &str) -> Option<String> {
    ant_id.split(':').nth(2).map(|s| s.to_string())
}

/// Case-insensitive manufacturer comparison, also handling common variations
/// (e.g. "Wahoo Fitness" vs "Wahoo").
fn manufacturers_match(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    a_lower == b_lower || a_lower.starts_with(&b_lower) || b_lower.starts_with(&a_lower)
}

/// Generate a deterministic UUID v5 group ID from sorted constituent device IDs.
fn deterministic_group_id(id_a: &str, id_b: &str) -> String {
    let mut ids = [id_a, id_b];
    ids.sort();
    let combined = format!("{}:{}", ids[0], ids[1]);
    let namespace = Uuid::NAMESPACE_OID;
    Uuid::new_v5(&namespace, combined.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::{ConnectionStatus, DeviceType, Transport};

    fn ble_device(id: &str, name: Option<&str>, dt: DeviceType) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
            device_type: dt,
            status: ConnectionStatus::Disconnected,
            transport: Transport::Ble,
            rssi: None,
            battery_level: None,
            last_seen: None,
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
        }
    }

    fn ant_device(id: &str, name: Option<&str>, dt: DeviceType) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
            device_type: dt,
            status: ConnectionStatus::Disconnected,
            transport: Transport::AntPlus,
            rssi: None,
            battery_level: None,
            last_seen: None,
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
        }
    }

    #[test]
    fn serial_match_same_manufacturer() {
        let mut ble = ble_device("ble-abc", Some("KICKR 1234"), DeviceType::FitnessTrainer);
        ble.manufacturer = Some("Wahoo Fitness".to_string());
        ble.serial_number = Some("12345".to_string());

        let mut ant = ant_device("ant:fec:1234", Some("ANT+ FitnessTrainer 1234"), DeviceType::FitnessTrainer);
        ant.manufacturer = Some("Wahoo Fitness".to_string());
        ant.serial_number = Some("12345".to_string());

        let groups = compute_device_groups(&[ble.clone(), ant.clone()]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(&ble.id), groups.get(&ant.id));
    }

    #[test]
    fn serial_mismatch_different_serial() {
        let mut ble = ble_device("ble-abc", Some("KICKR"), DeviceType::FitnessTrainer);
        ble.manufacturer = Some("Wahoo Fitness".to_string());
        ble.serial_number = Some("12345".to_string());

        let mut ant = ant_device("ant:fec:1234", Some("ANT+ FitnessTrainer 1234"), DeviceType::FitnessTrainer);
        ant.manufacturer = Some("Wahoo Fitness".to_string());
        ant.serial_number = Some("99999".to_string());

        let groups = compute_device_groups(&[ble, ant]);
        assert!(groups.is_empty());
    }

    #[test]
    fn serial_sentinel_skipped() {
        let mut ble = ble_device("ble-abc", Some("KICKR"), DeviceType::FitnessTrainer);
        ble.serial_number = Some("12345".to_string());

        let mut ant = ant_device("ant:fec:1234", Some("ANT+ FitnessTrainer 1234"), DeviceType::FitnessTrainer);
        ant.serial_number = Some("0".to_string()); // ANT+ sentinel

        let groups = compute_device_groups(&[ble, ant]);
        assert!(groups.is_empty());
    }

    #[test]
    fn name_number_match() {
        let mut ble = ble_device("ble-abc", Some("KICKR 1234"), DeviceType::FitnessTrainer);
        ble.manufacturer = Some("Wahoo".to_string());

        let mut ant = ant_device("ant:fec:1234", Some("ANT+ FitnessTrainer 1234"), DeviceType::FitnessTrainer);
        ant.manufacturer = Some("Wahoo Fitness".to_string());

        let groups = compute_device_groups(&[ble.clone(), ant.clone()]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(&ble.id), groups.get(&ant.id));
    }

    #[test]
    fn different_device_type_no_match() {
        let mut ble = ble_device("ble-abc", Some("Device"), DeviceType::Power);
        ble.serial_number = Some("12345".to_string());

        let mut ant = ant_device("ant:hr:1234", Some("Device"), DeviceType::HeartRate);
        ant.serial_number = Some("12345".to_string());

        let groups = compute_device_groups(&[ble, ant]);
        assert!(groups.is_empty());
    }

    #[test]
    fn no_metadata_no_match() {
        let ble = ble_device("ble-abc", None, DeviceType::Power);
        let ant = ant_device("ant:power:1234", None, DeviceType::Power);

        let groups = compute_device_groups(&[ble, ant]);
        assert!(groups.is_empty());
    }
}
