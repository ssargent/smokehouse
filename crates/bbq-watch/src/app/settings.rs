//! Settings persistence: propagate to subsystems and write the config file.

use super::*;

impl App {
    /// Apply and persist a settings change: propagate to the subsystems and
    /// write the config file (throttled so held keys do not thrash the disk).
    pub(crate) fn commit_settings(&mut self) {
        self.engine.set_limits(
            self.settings.alarms.repeat_s,
            self.settings.alarms.bt_lost_after_min,
            self.settings.alarms.unplugged_after_s,
        );
        let _ = self.settings_tx.send(self.settings.clone());
        self.logger.set_config(
            self.settings.data.csv_enabled,
            self.settings.csv_dir(),
            self.settings.data.sample_interval_s,
        );
        let units = self
            .settings
            .device
            .device_screen_units
            .resolve(self.unit());
        let _ = self
            .device_tx
            .try_send(DeviceCommand::SetDeviceUnits(units));
        self.caffeinate.set(self.settings.data.keep_awake);
        self.save_pending = true;
        self.flush_settings(false);
    }

    /// Write the config file when a change is pending and the throttle window
    /// has elapsed (or `force` is set).
    pub(crate) fn flush_settings(&mut self, force: bool) {
        if !self.save_pending {
            return;
        }
        let now = self.clock.now();
        let throttle_elapsed = self
            .last_save_at
            .is_none_or(|prev| now.since(prev) >= SAVE_THROTTLE_SECS);
        if !force && !throttle_elapsed {
            return;
        }
        self.last_save_at = Some(now);
        self.save_pending = false;
        match config::save(&self.settings, &self.config_path) {
            Ok(()) => {
                self.save_status = Some(SaveStatus {
                    ok: true,
                    message: String::from("Saved"),
                    at: now,
                });
            }
            Err(err) => {
                let message = format!("Couldn't save: {err}");
                tracing::warn!("{message}");
                self.save_status = Some(SaveStatus {
                    ok: false,
                    message: message.clone(),
                    at: now,
                });
                self.push_error_event(&message);
            }
        }
    }

    /// The saved / failed indicator while it is still fresh.
    #[must_use]
    pub(crate) fn save_status(&self) -> Option<&SaveStatus> {
        self.save_status
            .as_ref()
            .filter(|status| self.now.since(status.at) <= SAVE_STATUS_SECS)
    }
}
