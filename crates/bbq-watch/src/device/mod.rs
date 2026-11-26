//! Device layer: events, commands and the task that produces readings.

pub mod ble;
pub mod demo;
pub mod ibbq;

use tokio::sync::mpsc;

use crate::clock::{AppClock, AppInstant};
use crate::config::Settings;
use crate::model::{PROBE_COUNT, Unit};

/// Failures raised inside the device task.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// No usable Bluetooth adapter.
    #[error("no Bluetooth adapter: {0}")]
    Adapter(String),
    /// A GATT write failed.
    #[error("bluetooth write failed: {0}")]
    Write(String),
}

/// Events produced by the device task. Index 0 of `Readings::temps_c` is P1.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// A scan has started.
    Scanning,
    /// The thermometer connected.
    Connected {
        /// Advertised (or default) name.
        name: String,
        /// CoreBluetooth peripheral UUID.
        id: String,
    },
    /// The connection dropped.
    Disconnected {
        /// Why the connection ended.
        reason: String,
    },
    /// A fresh set of probe temperatures, in °C (`None` = unplugged).
    Readings {
        /// Simulated instant the reading was taken.
        at: AppInstant,
        /// One entry per probe.
        temps_c: [Option<f32>; PROBE_COUNT],
    },
    /// Bluetooth is unusable (no adapter or permission denied).
    Unavailable,
    /// Battery percentage.
    Battery(u8),
    /// Signal strength in dBm.
    Rssi(i16),
    /// A non-fatal problem worth surfacing.
    Error(String),
}

/// Commands sent from the UI to the device task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCommand {
    /// Set the thermometer's own screen units.
    SetDeviceUnits(Unit),
    /// Forget the saved peripheral and scan again.
    Rescan,
    /// Clear the saved peripheral id and scan again.
    Forget,
    /// Stop the device task.
    Shutdown,
}

/// Configuration the device task needs.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    /// Saved CoreBluetooth peripheral UUID (empty = none).
    pub peripheral_id: String,
    /// Reconnect automatically after a drop.
    pub auto_reconnect: bool,
    /// Battery poll interval in minutes.
    pub battery_check_min: u32,
    /// Demo sampling interval in simulated seconds.
    pub sample_interval_s: u32,
    /// Units to show on the thermometer screen.
    pub device_units: Unit,
    /// Demo time multiplier.
    pub speed: f64,
}

impl DeviceConfig {
    /// Build a device config from the app settings.
    #[must_use]
    pub fn from_settings(settings: &Settings, speed: f64) -> Self {
        Self {
            peripheral_id: settings.device.peripheral_id.clone(),
            auto_reconnect: settings.device.auto_reconnect,
            battery_check_min: settings.device.battery_check_min,
            sample_interval_s: settings.data.sample_interval_s,
            device_units: settings
                .device
                .device_screen_units
                .resolve(settings.display.units),
            speed,
        }
    }
}

/// Spawn the device task, returning its event receiver and command sender.
#[must_use]
pub fn spawn_device(
    cfg: DeviceConfig,
    clock: AppClock,
    demo: bool,
) -> (mpsc::Receiver<DeviceEvent>, mpsc::Sender<DeviceCommand>) {
    let (event_tx, event_rx) = mpsc::channel(64);
    let (command_tx, command_rx) = mpsc::channel(8);
    tokio::spawn(async move {
        if demo {
            demo::run(cfg, clock, event_tx, command_rx).await;
        } else {
            ble::run(cfg, clock, event_tx, command_rx).await;
        }
    });
    (event_rx, command_tx)
}
