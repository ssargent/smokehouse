//! Device events and error reporting.

use super::*;

impl App {
    pub(crate) async fn on_device_event(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::Scanning => {
                self.device_status = DeviceStatus::Scanning;
            }
            DeviceEvent::Connected { name, id } => {
                self.device_status = DeviceStatus::Connected;
                self.device_name = name;
                self.peripheral_id.clone_from(&id);
                if self.settings.device.peripheral_id != id {
                    self.settings.device.peripheral_id.clone_from(&id);
                    self.commit_settings();
                }
                self.log_event(
                    EventIcon::Milestone,
                    format!("Connected to {}", self.device_label()),
                );
            }
            DeviceEvent::Disconnected { reason } => {
                self.device_status = DeviceStatus::Disconnected;
                for probe in &mut self.probes {
                    if let Some(temp) = probe.temp_c.take() {
                        probe.last_seen_c = Some(temp);
                    }
                }
                self.log_event(EventIcon::Urgent, format!("Disconnected: {reason}"));
            }
            DeviceEvent::Readings { at, temps_c } => {
                self.last_reading_at = Some(at);
                self.now = at;
                let temps = temps_c;
                for (index, probe) in self.probes.iter_mut().enumerate() {
                    let temp = temps.get(index).copied().flatten();
                    probe.temp_c = temp;
                    if let Some(temp) = temp {
                        probe.had_reading = true;
                        probe.last_seen_c = Some(temp);
                    }
                }
                if !self.cook_started {
                    self.cook_started = true;
                    self.log_event(EventIcon::Milestone, String::from("Cook started"));
                }
                let mut events = Vec::new();
                self.update_probes(at, &self.settings.cook.clone(), &mut events);
                for (icon, message) in events {
                    self.log_event(icon, message);
                }
            }
            DeviceEvent::Battery(percent) => self.battery = Some(percent),
            DeviceEvent::Rssi(value) => self.rssi = Some(value),
            DeviceEvent::Unavailable => {
                self.device_status = DeviceStatus::Unavailable;
                self.push_error_event("Bluetooth unavailable");
            }
            DeviceEvent::Error(message) => self.push_error_event(&message),
        }
    }

    pub(crate) fn on_command_error(&mut self, message: String) {
        self.push_error_event(&message);
    }

    /// Log an error at most once per five-minute window per message.
    pub(crate) fn push_error_event(&mut self, message: &str) {
        const WINDOW_S: f64 = 300.0;
        let now = self.clock.now();
        if self
            .error_events
            .get(message)
            .is_some_and(|at| now.since(*at) < WINDOW_S)
        {
            return;
        }
        self.error_events.retain(|_, at| now.since(*at) < WINDOW_S);
        self.error_events.insert(message.to_string(), now);
        tracing::warn!("{message}");
        self.log_event(EventIcon::Info, message.to_string());
    }

    // ---- tick -------------------------------------------------------------
}
