//! BLE transport: scan, connect, login, subscribe and reconnect.
//!
//! Runs on macOS via CoreBluetooth. Linux builds compile too (btleplug's
//! BlueZ backend) but the app targets macOS.

use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::clock::{AppClock, Clock};
use crate::device::ibbq;
use crate::device::{DeviceCommand, DeviceConfig, DeviceError, DeviceEvent};

/// How long a single scan runs before giving up.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// Wait after a fruitless scan before scanning again.
const NOT_FOUND_WAIT: Duration = Duration::from_secs(10);
/// Reconnect backoff schedule; the last entry repeats.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(30),
];

/// Result of waiting on the command channel.
enum Wait {
    /// The wait completed.
    Elapsed,
    /// A rescan or forget arrived; restart the outer loop.
    Abort,
    /// Shut down.
    Shutdown,
}

/// Result of a scan.
enum ScanOutcome {
    /// A matching peripheral was found.
    Found(Peripheral),
    /// Nothing matched before the timeout.
    NotFound,
    /// Shut down.
    Shutdown,
}

/// Why a connected session ended.
enum SessionOutcome {
    /// Shut down.
    Shutdown,
    /// Rescan requested.
    Rescan,
    /// Forget requested.
    Forget,
    /// The connection dropped. `connected` records whether it had reached
    /// `Connected` first, so a healthy session resets the backoff.
    Disconnected { reason: String, connected: bool },
}

/// Run the BLE device task until told to shut down.
pub async fn run(
    cfg: DeviceConfig,
    clock: AppClock,
    tx: mpsc::Sender<DeviceEvent>,
    mut cmd: mpsc::Receiver<DeviceCommand>,
) {
    let mut saved_id = cfg.peripheral_id.clone();
    let mut backoff_idx = 0_usize;

    loop {
        let adapter = match get_adapter().await {
            Ok(adapter) => adapter,
            Err(err) => {
                tracing::warn!("{err}");
                let _ = tx.send(DeviceEvent::Unavailable).await;
                if matches!(
                    wait_cmd(Duration::from_secs(10), &mut cmd, &mut saved_id).await,
                    Wait::Shutdown
                ) {
                    return;
                }
                continue;
            }
        };

        let peripheral = match scan_for(&adapter, &mut saved_id, &tx, &mut cmd).await {
            ScanOutcome::Found(peripheral) => peripheral,
            ScanOutcome::NotFound => {
                let _ = tx.send(DeviceEvent::Error(String::from("not found"))).await;
                if matches!(
                    wait_cmd(NOT_FOUND_WAIT, &mut cmd, &mut saved_id).await,
                    Wait::Shutdown
                ) {
                    return;
                }
                continue;
            }
            ScanOutcome::Shutdown => return,
        };

        match session(
            &cfg,
            &adapter,
            &peripheral,
            &mut saved_id,
            &clock,
            &tx,
            &mut cmd,
        )
        .await
        {
            SessionOutcome::Shutdown => return,
            SessionOutcome::Rescan => {
                backoff_idx = 0;
            }
            SessionOutcome::Forget => {
                saved_id.clear();
                backoff_idx = 0;
            }
            SessionOutcome::Disconnected { reason, connected } => {
                let _ = tx.send(DeviceEvent::Disconnected { reason }).await;
                match reconnect_delay(cfg.auto_reconnect, connected, &mut backoff_idx) {
                    Some(delay) => {
                        if matches!(
                            wait_cmd(delay, &mut cmd, &mut saved_id).await,
                            Wait::Shutdown
                        ) {
                            return;
                        }
                    }
                    None => {
                        if matches!(wait_forever(&mut cmd, &mut saved_id).await, Wait::Shutdown) {
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Delay before the next reconnect, or `None` to wait for a command. Reaching
/// `Connected` resets the backoff so a drop after hours does not wait long.
fn reconnect_delay(
    auto_reconnect: bool,
    connected: bool,
    backoff_idx: &mut usize,
) -> Option<Duration> {
    if connected {
        *backoff_idx = 0;
    }
    if !auto_reconnect {
        return None;
    }
    let delay = BACKOFF[(*backoff_idx).min(BACKOFF.len() - 1)];
    *backoff_idx += 1;
    Some(delay)
}

/// Wait for `Rescan` or `Shutdown` indefinitely (used when auto-reconnect is
/// off), without reconnecting on its own.
async fn wait_forever(cmd: &mut mpsc::Receiver<DeviceCommand>, saved_id: &mut String) -> Wait {
    loop {
        match cmd.recv().await {
            None | Some(DeviceCommand::Shutdown) => return Wait::Shutdown,
            Some(DeviceCommand::Rescan) => return Wait::Abort,
            Some(DeviceCommand::Forget) => {
                saved_id.clear();
                return Wait::Abort;
            }
            Some(DeviceCommand::SetDeviceUnits(_)) => {}
        }
    }
}

/// Wait for `dur`, still servicing commands.
async fn wait_cmd(
    dur: Duration,
    cmd: &mut mpsc::Receiver<DeviceCommand>,
    saved_id: &mut String,
) -> Wait {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Wait::Elapsed;
        }
        match tokio::time::timeout(remaining, cmd.recv()).await {
            Err(_) => return Wait::Elapsed,
            Ok(None) => return Wait::Shutdown,
            Ok(Some(DeviceCommand::Shutdown)) => return Wait::Shutdown,
            Ok(Some(DeviceCommand::Rescan)) => return Wait::Abort,
            Ok(Some(DeviceCommand::Forget)) => {
                saved_id.clear();
                return Wait::Abort;
            }
            Ok(Some(DeviceCommand::SetDeviceUnits(_))) => {}
        }
    }
}

/// First available Bluetooth adapter.
async fn get_adapter() -> Result<Adapter, DeviceError> {
    let manager = Manager::new()
        .await
        .map_err(|err| DeviceError::Adapter(err.to_string()))?;
    let adapters = manager
        .adapters()
        .await
        .map_err(|err| DeviceError::Adapter(err.to_string()))?;
    adapters
        .into_iter()
        .next()
        .ok_or_else(|| DeviceError::Adapter(String::from("none found")))
}

/// Scan for a matching thermometer, honouring commands while scanning.
async fn scan_for(
    adapter: &Adapter,
    saved_id: &mut String,
    tx: &mpsc::Sender<DeviceEvent>,
    cmd: &mut mpsc::Receiver<DeviceCommand>,
) -> ScanOutcome {
    let _ = tx.send(DeviceEvent::Scanning).await;
    if adapter.start_scan(ScanFilter::default()).await.is_err() {
        return ScanOutcome::NotFound;
    }

    let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let outcome = loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Ok(peripherals) = adapter.peripherals().await
                    && let Some(found) = pick(&peripherals, saved_id).await {
                        break ScanOutcome::Found(found);
                    }
                if tokio::time::Instant::now() >= deadline {
                    break ScanOutcome::NotFound;
                }
            }
            incoming = cmd.recv() => {
                match incoming {
                    None | Some(DeviceCommand::Shutdown) => break ScanOutcome::Shutdown,
                    Some(DeviceCommand::Forget) => saved_id.clear(),
                    Some(DeviceCommand::Rescan | DeviceCommand::SetDeviceUnits(_)) => {}
                }
            }
        }
    };
    let _ = adapter.stop_scan().await;
    outcome
}

/// Choose a peripheral: the saved id first, else a name match.
async fn pick(peripherals: &[Peripheral], saved_id: &str) -> Option<Peripheral> {
    if !saved_id.is_empty()
        && let Some(found) = peripherals
            .iter()
            .find(|peripheral| peripheral.id().to_string() == saved_id)
    {
        return Some(found.clone());
    }
    for peripheral in peripherals {
        if let Ok(Some(props)) = peripheral.properties().await
            && let Some(name) = props.local_name
            && (name == "iBBQ" || name == "BBQ")
        {
            return Some(peripheral.clone());
        }
    }
    None
}

/// Connect, handshake and stream notifications until something breaks.
async fn session(
    cfg: &DeviceConfig,
    adapter: &Adapter,
    peripheral: &Peripheral,
    saved_id: &mut String,
    clock: &AppClock,
    tx: &mpsc::Sender<DeviceEvent>,
    cmd: &mut mpsc::Receiver<DeviceCommand>,
) -> SessionOutcome {
    let _ = adapter.stop_scan().await;

    if let Err(err) = peripheral.connect().await {
        return SessionOutcome::Disconnected {
            reason: format!("connect failed: {err}"),
            connected: false,
        };
    }
    if let Err(err) = peripheral.discover_services().await {
        let _ = peripheral.disconnect().await;
        return SessionOutcome::Disconnected {
            reason: format!("service discovery failed: {err}"),
            connected: false,
        };
    }

    // The notifications stream must be taken before subscribing.
    let mut notifications = match peripheral.notifications().await {
        Ok(stream) => stream,
        Err(err) => {
            let _ = peripheral.disconnect().await;
            return SessionOutcome::Disconnected {
                reason: format!("notifications failed: {err}"),
                connected: false,
            };
        }
    };

    let characteristics = peripheral.characteristics();
    let find = |short: u16| {
        let uuid = ibbq::uuid(short);
        characteristics.iter().find(|c| c.uuid == uuid).cloned()
    };
    let (Some(account), Some(settings), Some(realtime), Some(result)) = (
        find(ibbq::CHAR_ACCOUNT_AND_VERIFY),
        find(ibbq::CHAR_SETTINGS_DATA),
        find(ibbq::CHAR_REALTIME_DATA),
        find(ibbq::CHAR_SETTINGS_RESULT),
    ) else {
        let _ = peripheral.disconnect().await;
        return SessionOutcome::Disconnected {
            reason: String::from("required characteristics missing"),
            connected: false,
        };
    };

    if let Err(err) = write_char(peripheral, &account, ibbq::LOGIN).await {
        let _ = peripheral.disconnect().await;
        return SessionOutcome::Disconnected {
            reason: format!("login failed: {err}"),
            connected: false,
        };
    }
    if let Err(err) = peripheral.subscribe(&realtime).await {
        let _ = peripheral.disconnect().await;
        return SessionOutcome::Disconnected {
            reason: format!("subscribe failed: {err}"),
            connected: false,
        };
    }
    if let Err(err) = peripheral.subscribe(&result).await {
        let _ = peripheral.disconnect().await;
        return SessionOutcome::Disconnected {
            reason: format!("subscribe failed: {err}"),
            connected: false,
        };
    }
    let _ = write_char(peripheral, &settings, ibbq::ENABLE_REALTIME).await;
    let _ = write_char(peripheral, &settings, ibbq::encode_units(cfg.device_units)).await;
    let _ = write_char(peripheral, &settings, ibbq::REQUEST_BATTERY).await;

    let name = peripheral
        .properties()
        .await
        .ok()
        .flatten()
        .and_then(|props| props.local_name)
        .unwrap_or_else(|| String::from("iBBQ"));
    let id = peripheral.id().to_string();
    if saved_id.is_empty() {
        *saved_id = id.clone();
    }
    let _ = tx.send(DeviceEvent::Connected { name, id }).await;

    let battery_period = Duration::from_secs(u64::from(cfg.battery_check_min.max(1)) * 60);
    let mut battery_timer =
        tokio::time::interval_at(tokio::time::Instant::now() + battery_period, battery_period);
    let mut rssi_timer = tokio::time::interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            note = notifications.next() => {
                match note {
                    None => {
                        let _ = peripheral.disconnect().await;
                        return SessionOutcome::Disconnected {
                            reason: String::from("connection lost"),
                            connected: true,
                        };
                    }
                    Some(note) => handle_notification(&note, clock, tx).await,
                }
            }
            _ = battery_timer.tick() => {
                let _ = write_char(peripheral, &settings, ibbq::REQUEST_BATTERY).await;
            }
            _ = rssi_timer.tick() => {
                if let Ok(Some(props)) = peripheral.properties().await
                    && let Some(rssi) = props.rssi {
                        let _ = tx.send(DeviceEvent::Rssi(rssi)).await;
                    }
            }
            incoming = cmd.recv() => {
                match incoming {
                    None | Some(DeviceCommand::Shutdown) => {
                        let _ = peripheral.disconnect().await;
                        return SessionOutcome::Shutdown;
                    }
                    Some(DeviceCommand::SetDeviceUnits(unit)) => {
                        let _ = write_char(peripheral, &settings, ibbq::encode_units(unit)).await;
                    }
                    Some(DeviceCommand::Rescan) => {
                        let _ = peripheral.disconnect().await;
                        return SessionOutcome::Rescan;
                    }
                    Some(DeviceCommand::Forget) => {
                        saved_id.clear();
                        let _ = peripheral.disconnect().await;
                        return SessionOutcome::Forget;
                    }
                }
            }
        }
    }
}

/// Translate a notification into a device event.
async fn handle_notification(
    note: &ValueNotification,
    clock: &AppClock,
    tx: &mpsc::Sender<DeviceEvent>,
) {
    if note.uuid == ibbq::uuid(ibbq::CHAR_REALTIME_DATA) {
        let temps_c = ibbq::parse_realtime(&note.value);
        let _ = tx
            .send(DeviceEvent::Readings {
                at: clock.now(),
                temps_c,
            })
            .await;
    } else if note.uuid == ibbq::uuid(ibbq::CHAR_SETTINGS_RESULT)
        && let Some(percent) = ibbq::parse_battery(&note.value)
    {
        let _ = tx.send(DeviceEvent::Battery(percent)).await;
    }
}

/// Write to a characteristic, choosing the response mode from its flags.
async fn write_char(
    peripheral: &Peripheral,
    characteristic: &Characteristic,
    data: &[u8],
) -> Result<(), DeviceError> {
    let write_type = if characteristic.properties.contains(CharPropFlags::WRITE) {
        WriteType::WithResponse
    } else {
        WriteType::WithoutResponse
    };
    peripheral
        .write(characteristic, data, write_type)
        .await
        .map_err(|err| DeviceError::Write(err.to_string()))
}

/// Compact a device id for display.
#[must_use]
pub fn short_id(id: &str) -> String {
    let mut chars = id.chars();
    let short: String = chars.by_ref().take(8).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn backoff_resets_after_a_healthy_connection() {
        // Five failures of backoff would mean a 30s wait; a connection resets it.
        let mut idx = 5;
        assert_eq!(
            reconnect_delay(true, true, &mut idx),
            Some(Duration::from_secs(5))
        );
        assert_eq!(idx, 1);
    }

    #[test]
    fn backoff_grows_without_a_connection() {
        let mut idx = 0;
        assert_eq!(
            reconnect_delay(true, false, &mut idx),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            reconnect_delay(true, false, &mut idx),
            Some(Duration::from_secs(10))
        );
        assert_eq!(idx, 2);
    }

    #[test]
    fn auto_reconnect_off_waits_for_a_command() {
        let mut idx = 3;
        assert_eq!(reconnect_delay(false, false, &mut idx), None);
    }
}
