/// Connection watchdog interval — how often we check for stale/disconnected devices.
pub const CONNECTION_CHECK_INTERVAL_SECS: u64 = 5;

/// Autosave interval — how often we snapshot the active session to disk.
pub const AUTOSAVE_INTERVAL_SECS: u64 = 30;

/// Live metrics push interval — how often we emit metrics to the frontend.
pub const LIVE_METRICS_PUSH_MS: u64 = 250;

/// BLE scan duration — how long a BLE scan runs before returning results.
pub const BLE_SCAN_DURATION_SECS: u64 = 3;

/// ANT+ staleness threshold — device considered disconnected after this many seconds without data.
pub const ANT_STALE_SECS: u64 = 10;

/// Reconnect initial backoff — delay before first reconnect attempt.
pub const RECONNECT_INITIAL_BACKOFF_MS: u64 = 2000;

/// Reconnect max backoff — ceiling for exponential backoff.
pub const RECONNECT_MAX_BACKOFF_MS: u64 = 30000;

/// Reconnect backoff multiplier.
pub const RECONNECT_BACKOFF_MULTIPLIER: u64 = 2;
