use btleplug::api::{Characteristic, Peripheral as _};
use btleplug::platform::Peripheral;
use futures::StreamExt;
use log::{error, info, warn};
use tokio::sync::broadcast;

use super::protocol::*;
use super::types::{DeviceType, SensorReading};

pub async fn listen_to_device(
    peripheral: Peripheral,
    device_type: DeviceType,
    tx: broadcast::Sender<SensorReading>,
    device_id: String,
) {
    let characteristics = peripheral.characteristics();
    let target_chars: Vec<&Characteristic> = characteristics
        .iter()
        .filter(|c| match device_type {
            DeviceType::HeartRate => c.uuid == HEART_RATE_MEASUREMENT,
            DeviceType::Power => c.uuid == CYCLING_POWER_MEASUREMENT,
            DeviceType::CadenceSpeed => c.uuid == CSC_MEASUREMENT,
            DeviceType::FitnessTrainer => c.uuid == INDOOR_BIKE_DATA,
        })
        .collect();

    let mut subscribed_count = 0;
    for char in &target_chars {
        if let Err(e) = peripheral.subscribe(char).await {
            warn!("[{}] Failed to subscribe to {:?}: {}", device_id, char.uuid, e);
        } else {
            subscribed_count += 1;
        }
    }
    if subscribed_count == 0 {
        error!(
            "[{}] No characteristics subscribed for {:?} device — nothing to listen to",
            device_id, device_type
        );
        return;
    }
    info!(
        "[{}] Listening to {:?} device, {}/{} characteristics subscribed",
        device_id, device_type, subscribed_count, target_chars.len()
    );

    let mut notification_stream = match peripheral.notifications().await {
        Ok(stream) => stream,
        Err(e) => {
            error!("[{}] Failed to get notification stream: {}", device_id, e);
            return;
        }
    };

    let mut prev_wheel_revs: u32 = 0;
    let mut prev_wheel_time: u16 = 0;
    let mut prev_crank_revs: u16 = 0;
    let mut prev_crank_time: u16 = 0;

    while let Some(notification) = notification_stream.next().await {
        let readings: Vec<SensorReading> = if notification.uuid == HEART_RATE_MEASUREMENT {
            decode_heart_rate(&notification.value, &device_id)
                .into_iter()
                .collect()
        } else if notification.uuid == CYCLING_POWER_MEASUREMENT {
            decode_cycling_power(&notification.value, &device_id)
                .into_iter()
                .collect()
        } else if notification.uuid == CSC_MEASUREMENT {
            decode_csc(
                &notification.value,
                &mut prev_wheel_revs,
                &mut prev_wheel_time,
                &mut prev_crank_revs,
                &mut prev_crank_time,
                &device_id,
            )
        } else if notification.uuid == INDOOR_BIKE_DATA {
            decode_indoor_bike_data(&notification.value, &device_id)
        } else {
            continue;
        };

        for reading in readings {
            if tx.send(reading).is_err() {
                warn!("[{}] No receivers for sensor readings, stopping listener", device_id);
                return;
            }
        }
    }
    info!("[{}] Notification stream ended for {:?} device", device_id, device_type);
}
