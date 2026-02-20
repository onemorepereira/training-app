# Training App

A cycling training application with real-time sensor support over BLE and ANT+.

## Tech Stack

- **Backend**: Rust, Tauri v2
- **Frontend**: Svelte 5, TypeScript, ECharts
- **Sensors**: BLE (btleplug), ANT+ (custom USB driver via rusb)
- **Storage**: SQLite (sqlx)

## Features

- Real-time heart rate, power, and cadence from BLE and ANT+ sensors
- Session recording with lap markers
- Training metrics (Normalized Power, TSS, Intensity Factor)
- Session history and activity metadata
- Device management with auto-reconnect for known devices

## Prerequisites

- Linux (other platforms not yet tested)
- Rust toolchain (1.77.2+)
- Node.js and npm
- System dependencies for Tauri v2: see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
- For ANT+ USB sticks, udev rules to allow non-root access:

```
# /etc/udev/rules.d/99-ant-usb.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="0fcf", MODE="0666"
```

Reload with `sudo udevadm control --reload-rules && sudo udevadm trigger`.

## Build

```bash
npm install
cd src-tauri && cargo build
npm run tauri dev      # development
npm run build:release  # production build
```

## Test

```bash
cd src-tauri && cargo test   # Rust tests
npm run check                # Frontend type checking
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the contributor workflow.

## License

GPL-3.0. See [LICENSE](LICENSE).
