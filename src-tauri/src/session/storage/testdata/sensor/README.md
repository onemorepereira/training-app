# Sensor log golden files

These files were written by **bincode 1.3.3** (`bincode::serialize`) before the dependency was
removed. They pin the on-disk layout that `sensor_codec` must keep reading, and
`current.bin` pins what it must keep writing byte for byte. **Do not regenerate or edit them.**
To cover a new layout, add a new file.

| File | Layout | Written from |
|---|---|---|
| `current.bin` | v0.4.0 onwards | `Vec<SensorReading>`: all five variants, `pedal_balance` `Some` and `None`, empty and non-ASCII `device_id`, `u16::MAX`/`u64::MAX`, `-0.0` |
| `empty.bin` | any | `Vec::<SensorReading>::new()` |
| `legacy_no_pedal_balance.bin` | v0.1.0–v0.3.x, no balance readings | an enum like `SensorReading` without the `pedal_balance` field |
| `v0_3_mixed_pedal_balance.bin` | v0.1.0–v0.3.x | Power with `#[serde(skip_serializing_if = "Option::is_none")] pedal_balance`, mixing `Some` and `None`; includes `Some(1)` followed by a HeartRate, whose tag also starts with byte `1` |
| `pre_device_id.bin` | pre-release dev builds | variants without `device_id` or `pedal_balance` |

The exact readings in each file are listed in the tests in `../../sensor_codec.rs`.
