//! Application state, event loop and key handling.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Timelike as _};
use futures::StreamExt as _;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind};
use tokio::sync::{Mutex, mpsc};

use crate::alerts::{
    ActiveAlerts, ActiveInfo, AlertEngine, AlertKey, AlertKind, Priority, ProbeSnapshot, Snapshot,
};
use crate::analysis::{EtaResult, ProbeHistory, StallTracker, StallTransition, compute_eta};
use crate::caffeinate::Caffeinate;
use crate::cli::Cli;
use crate::clock::{AppClock, AppInstant, Clock};
use crate::config::{self, Settings, parse_hhmm};
use crate::device::{self, DeviceCommand, DeviceEvent};
use crate::logger::CsvLogger;
use crate::model::{PROBE_COUNT, ProbeId, Unit, celsius_to_fahrenheit};
use crate::notify::Notifier;
use crate::ui::{self, Tab};

/// How often the app redraws at minimum.
const MIN_REFRESH_MS: u64 = 250;
/// Simulated minutes a pit drop must persist to count as "lid open".
const LID_WINDOW_MIN: f64 = 3.0;
/// Minimum seconds between settings writes while keys repeat.
const SAVE_THROTTLE_SECS: f64 = 0.3;
/// Seconds the "Saved" / "Couldn't save" footer indicator stays visible.
const SAVE_STATUS_SECS: f64 = 2.0;

/// Connection state of the thermometer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceStatus {
    /// Scanning for the device.
    Scanning,
    /// Connected and streaming.
    Connected,
    /// Previously connected, now dropped.
    Disconnected,
    /// No usable Bluetooth adapter.
    Unavailable,
}

/// Icon class for an event-log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventIcon {
    /// Milestones (cook started, target reached, connected).
    Milestone,
    /// Warnings (stall, lid open, normal alerts).
    Warning,
    /// Urgent alerts.
    Urgent,
    /// Informational.
    Info,
}

/// One entry in the cook event log.
#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub(crate) wall: DateTime<Local>,
    pub(crate) icon: EventIcon,
    pub(crate) message: String,
}

/// Per-probe live state.
#[derive(Debug, Default)]
pub(crate) struct ProbeState {
    pub(crate) temp_c: Option<f32>,
    pub(crate) had_reading: bool,
    pub(crate) history: ProbeHistory,
    pub(crate) stall: StallTracker,
    pub(crate) stalled: bool,
    pub(crate) wrapped_at: Option<AppInstant>,
    pub(crate) start_temp_c: Option<f32>,
    pub(crate) rate_c_h: Option<f32>,
    pub(crate) deviation_c: Option<f32>,
    pub(crate) eta: Option<EtaResult>,
    pub(crate) last_milestone_f: i32,
    pub(crate) reached_range_min: bool,
}

/// Highlighted banner for the current alert.
#[derive(Clone, Debug)]
pub(crate) struct Banner {
    pub(crate) text: String,
    pub(crate) priority: Priority,
}

/// Result of the most recent settings write, shown briefly in the footer.
#[derive(Clone, Debug)]
pub(crate) struct SaveStatus {
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) at: AppInstant,
}

/// One row of the Alarms tab.
#[derive(Clone, Debug)]
pub(crate) struct AlertRow {
    pub(crate) key: AlertKey,
    pub(crate) priority: Priority,
    pub(crate) since: AppInstant,
    pub(crate) last_spoken: Option<AppInstant>,
    pub(crate) acked: bool,
    pub(crate) snoozed_until: Option<AppInstant>,
}

/// A read-only view of the active-alerts snapshot, refreshed every tick.
#[derive(Default)]
pub(crate) struct AlertsView {
    pub(crate) banner: Option<Banner>,
    pub(crate) rows: Vec<AlertRow>,
    pub(crate) cleared: Vec<(AlertKey, AppInstant)>,
}

/// All application state, owned by the event loop.
pub struct App {
    pub(crate) settings: Settings,
    pub(crate) cli: Cli,
    pub(crate) clock: AppClock,
    pub(crate) config_path: PathBuf,
    pub(crate) demo: bool,
    pub(crate) now: AppInstant,

    pub(crate) tab: Tab,
    pub(crate) show_help: bool,
    pub(crate) quit_prompt: bool,
    pub(crate) should_quit: bool,

    pub(crate) device_status: DeviceStatus,
    pub(crate) device_name: String,
    pub(crate) peripheral_id: String,
    pub(crate) rssi: Option<i16>,
    pub(crate) battery: Option<u8>,
    pub(crate) last_reading_at: Option<AppInstant>,
    pub(crate) demo_unplugged: [bool; PROBE_COUNT],

    pub(crate) probes: [ProbeState; PROBE_COUNT],

    pub(crate) engine: AlertEngine,
    pub(crate) shared: Arc<Mutex<ActiveAlerts>>,
    pub(crate) alerts_view: AlertsView,
    pub(crate) cleared_seen: usize,
    pub(crate) bell_pending: bool,

    pub(crate) events: Vec<Event>,
    pub(crate) cook_started: bool,

    pub(crate) selected_card: usize,
    pub(crate) table_scroll: usize,
    pub(crate) log_scroll: usize,
    pub(crate) alarms_selected: usize,
    pub(crate) settings_ui: ui::settings::SettingsUi,
    pub(crate) save_pending: bool,
    pub(crate) last_save_at: Option<AppInstant>,
    pub(crate) save_status: Option<SaveStatus>,

    pub(crate) all_done_seconds: Option<f64>,
    pub(crate) serve_wall: Option<DateTime<Local>>,
    pub(crate) serve_late: bool,
    pub(crate) frozen_etas: Option<[Option<f64>; PROBE_COUNT]>,
    pub(crate) lid_open: bool,
    pub(crate) pit_reference_c: Option<f32>,
    pub(crate) next_status_at: Option<AppInstant>,

    pub(crate) logger: CsvLogger,
    pub(crate) caffeinate: Caffeinate,
    pub(crate) notifier: Notifier,
    pub(crate) device_tx: mpsc::Sender<DeviceCommand>,
    pub(crate) voices: Vec<String>,
    pub(crate) audio_devices: Vec<String>,
}

/// Build the app and start the device and notifier tasks.
async fn build(
    cli: Cli,
    settings: Settings,
    clock: AppClock,
    config_path: PathBuf,
) -> (App, mpsc::Receiver<DeviceEvent>, mpsc::Receiver<String>) {
    let demo = cli.demo;
    let shared = Arc::new(Mutex::new(ActiveAlerts::default()));
    let device_config = device::DeviceConfig::from_settings(&settings, clock.speed());
    let (device_rx, device_tx) = device::spawn_device(device_config, clock.clone(), demo);
    let (error_tx, error_rx) = mpsc::channel(16);
    let notifier = crate::notify::spawn(
        Arc::new(clock.clone()),
        Arc::clone(&shared),
        &settings,
        cli.no_speech,
        error_tx,
    );
    let voices = crate::notify::speech::list_voices().await;
    let audio_devices = crate::notify::speech::list_audio_devices().await;

    let engine = AlertEngine::new(
        settings.alarms.repeat_s,
        settings.alarms.bt_lost_after_min,
        settings.alarms.unplugged_after_s,
    );
    let logger = CsvLogger::new(
        settings.data.csv_enabled,
        settings.csv_dir(),
        settings.data.sample_interval_s,
    );
    let mut caffeinate = Caffeinate::new();
    caffeinate.set(settings.data.keep_awake).await;

    let app = App {
        settings,
        cli,
        clock,
        config_path,
        demo,
        now: AppInstant::ZERO,
        tab: Tab::Dashboard,
        show_help: false,
        quit_prompt: false,
        should_quit: false,
        device_status: DeviceStatus::Scanning,
        device_name: String::new(),
        peripheral_id: String::new(),
        rssi: None,
        battery: None,
        last_reading_at: None,
        demo_unplugged: [false; PROBE_COUNT],
        probes: std::array::from_fn(|_| ProbeState::default()),
        engine,
        shared,
        alerts_view: AlertsView::default(),
        cleared_seen: 0,
        bell_pending: false,
        events: Vec::new(),
        cook_started: false,
        selected_card: 0,
        table_scroll: 0,
        log_scroll: 0,
        alarms_selected: 0,
        settings_ui: ui::settings::SettingsUi::default(),
        save_pending: false,
        last_save_at: None,
        save_status: None,
        all_done_seconds: None,
        serve_wall: None,
        serve_late: false,
        frozen_etas: None,
        lid_open: false,
        pit_reference_c: None,
        next_status_at: None,
        logger,
        caffeinate,
        notifier,
        device_tx,
        voices,
        audio_devices,
    };
    (app, device_rx, error_rx)
}

/// Parse the CLI, load settings and run the app.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let config_path = config::config_path(cli.config.as_deref())?;
    let settings = config::load(&config_path)?;
    let speed = if cli.demo { cli.speed } else { 1.0 };
    let clock = AppClock::new(speed);
    let (mut app, mut device_rx, mut error_rx) = build(cli, settings, clock, config_path).await;
    let mut terminal = ratatui::init();
    let result = app
        .event_loop(&mut terminal, &mut device_rx, &mut error_rx)
        .await;
    ratatui::restore();
    app.cleanup().await;
    result
}

impl App {
    /// The display unit.
    #[must_use]
    pub(crate) fn unit(&self) -> Unit {
        self.settings.display.units
    }

    /// Wall-clock time for a simulated instant.
    #[must_use]
    pub(crate) fn wall(&self, at: AppInstant) -> DateTime<Local> {
        self.clock.wall(at)
    }

    /// Probe label from settings.
    #[must_use]
    pub(crate) fn probe_label(&self, index: usize) -> String {
        self.settings.probe(index).label
    }

    /// Whether probe `index` is the pit probe.
    #[must_use]
    pub(crate) fn is_pit(&self, index: usize) -> bool {
        self.settings.is_pit(index)
    }

    /// Whether the thermometer is connected.
    #[must_use]
    pub(crate) fn connected(&self) -> bool {
        self.device_status == DeviceStatus::Connected
    }

    /// Name to show for the device.
    #[must_use]
    pub(crate) fn device_label(&self) -> String {
        if self.settings.device.display_name.trim().is_empty() {
            self.device_name.clone()
        } else {
            self.settings.device.display_name.clone()
        }
    }

    /// Current temperatures, in probe order.
    #[must_use]
    pub(crate) fn temps_c(&self) -> [Option<f32>; PROBE_COUNT] {
        std::array::from_fn(|index| self.probes[index].temp_c)
    }

    async fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        device_rx: &mut mpsc::Receiver<DeviceEvent>,
        error_rx: &mut mpsc::Receiver<String>,
    ) -> anyhow::Result<()> {
        let mut input = ratatui::crossterm::event::EventStream::new();
        let period = Duration::from_millis(self.settings.display.refresh_ms.max(MIN_REFRESH_MS));
        let mut ticker = tokio::time::interval(period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            terminal.draw(|frame| ui::draw(frame, self))?;
            self.emit_bell();

            tokio::select! {
                maybe = input.next() => {
                    if let Some(Ok(event)) = maybe {
                        self.on_input(event).await;
                    }
                }
                _ = ticker.tick() => self.on_tick().await,
                maybe = device_rx.recv() => {
                    if let Some(event) = maybe {
                        self.on_device_event(event).await;
                    }
                }
                maybe = error_rx.recv() => {
                    if let Some(message) = maybe {
                        self.on_command_error(message);
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Write the terminal bell if one is pending and enabled.
    fn emit_bell(&mut self) {
        if !self.bell_pending {
            return;
        }
        self.bell_pending = false;
        if self.settings.alarms.bell {
            let _ = ratatui::crossterm::execute!(
                std::io::stdout(),
                ratatui::crossterm::style::Print("\u{7}")
            );
        }
    }

    async fn cleanup(&mut self) {
        self.logger.flush();
        self.flush_settings(true);
        self.caffeinate.stop().await;
        let _ = self.device_tx.send(DeviceCommand::Shutdown).await;
        self.notifier.shutdown();
    }

    // ---- device -----------------------------------------------------------

    async fn on_device_event(&mut self, event: DeviceEvent) {
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
                self.log_event(EventIcon::Urgent, format!("Disconnected: {reason}"));
            }
            DeviceEvent::Readings { at, temps_c } => {
                self.last_reading_at = Some(at);
                self.now = at;
                let mut temps = temps_c;
                if self.demo {
                    for (index, unplugged) in self.demo_unplugged.iter().enumerate() {
                        if *unplugged && let Some(slot) = temps.get_mut(index) {
                            *slot = None;
                        }
                    }
                }
                for (index, probe) in self.probes.iter_mut().enumerate() {
                    let temp = temps.get(index).copied().flatten();
                    probe.temp_c = temp;
                    if temp.is_some() {
                        probe.had_reading = true;
                    }
                }
                if !self.cook_started {
                    self.cook_started = true;
                    self.log_event(EventIcon::Milestone, String::from("Cook started"));
                }
            }
            DeviceEvent::Battery(percent) => self.battery = Some(percent),
            DeviceEvent::Rssi(value) => self.rssi = Some(value),
            DeviceEvent::Error(message) => {
                if device::is_bluetooth_unavailable(&message) {
                    self.device_status = DeviceStatus::Unavailable;
                }
                self.push_error_event(&message);
            }
        }
    }

    fn on_command_error(&mut self, message: String) {
        self.push_error_event(&message);
    }

    fn push_error_event(&mut self, message: &str) {
        if self.events.iter().any(|event| event.message == message) {
            return;
        }
        tracing::warn!("{message}");
        self.log_event(EventIcon::Info, message.to_string());
    }

    // ---- tick -------------------------------------------------------------

    async fn on_tick(&mut self) {
        let now = self.clock.now();
        self.now = now;
        let cook = self.settings.cook.clone();
        let mut new_events: Vec<(EventIcon, String)> = Vec::new();

        for index in 0..PROBE_COUNT {
            let cfg = self.settings.probe(index);
            let probe = &mut self.probes[index];
            probe.history.push(now, probe.temp_c);
            if probe.start_temp_c.is_none() {
                probe.start_temp_c = probe.history.current();
            }
            let rate = probe.history.rate_c_per_h(now, cook.rate_window_min);
            probe.rate_c_h = rate;
            probe.deviation_c = probe.history.max_deviation_c(now, 10.0);
            let transition = probe.stall.update(now, probe.temp_c, rate, &cook);
            probe.stalled = probe.stall.is_stalled();
            match transition {
                StallTransition::Started => {
                    new_events.push((EventIcon::Warning, format!("{} stalled", cfg.label)));
                }
                StallTransition::Ended => {
                    new_events.push((EventIcon::Milestone, format!("{} stall ended", cfg.label)));
                }
                StallTransition::None => {}
            }
            if !self.settings.is_pit(index) {
                if let Some(temp) = probe.temp_c {
                    let fahrenheit = celsius_to_fahrenheit(temp);
                    let milestone = (fahrenheit / 20.0).floor() * 20.0;
                    if milestone >= 100.0 && milestone as i32 > probe.last_milestone_f {
                        probe.last_milestone_f = milestone as i32;
                        new_events.push((
                            EventIcon::Milestone,
                            format!("{} passed {milestone:.0}°F", cfg.label),
                        ));
                    }
                }
            } else if let Some(range) = cfg.range_c
                && probe.temp_c.is_some_and(|temp| temp >= range[0])
                && !probe.reached_range_min
            {
                probe.reached_range_min = true;
                new_events.push((EventIcon::Milestone, String::from("Pit reached range")));
            }
            probe.eta = Some(compute_eta(
                cfg.target_c.unwrap_or(0.0),
                probe.temp_c,
                rate,
                probe.stalled,
                probe.stall.started_at(),
                now,
                probe.wrapped_at,
                &probe.history,
                &cook,
            ));
        }

        self.detect_lid(now, &mut new_events);
        self.compute_summary(now);

        let snapshot = self.snapshot();
        let deliveries = self.engine.evaluate(&snapshot, now);
        for delivery in deliveries {
            if !matches!(
                delivery.key.1,
                AlertKind::StallStarted | AlertKind::StallEnded
            ) {
                let icon = match delivery.priority {
                    Priority::Urgent => EventIcon::Urgent,
                    Priority::Normal => EventIcon::Warning,
                };
                new_events.push((icon, alert_message(&delivery.key, &snapshot)));
            }
            if self.settings.alarms.bell {
                self.bell_pending = true;
            }
            self.notifier.deliver(delivery);
        }

        for (icon, message) in new_events {
            self.log_event(icon, message);
        }

        self.log_cleared(now);
        self.sync_alerts_view(now, &snapshot).await;

        if self.logger.due(now) {
            let temps = self.temps_c();
            let wall = self.clock.wall(now);
            let battery = self.battery;
            self.logger.write_row(now, wall, &temps, battery, "");
        }

        self.maybe_speak_status(now);
        self.caffeinate.set(self.settings.data.keep_awake).await;
        self.flush_settings(false);
    }

    fn detect_lid(&mut self, now: AppInstant, new_events: &mut Vec<(EventIcon, String)>) {
        let pit_index = (0..PROBE_COUNT).find(|index| self.is_pit(*index));
        let Some(index) = pit_index else {
            return;
        };
        let current = self.probes[index].temp_c;
        let past = self.probes[index]
            .history
            .value_near(AppInstant::from_secs_f64(
                now.secs() - LID_WINDOW_MIN * 60.0,
            ));
        if let (Some(now_temp), Some(past_temp)) = (current, past) {
            let drop = celsius_to_fahrenheit(past_temp - now_temp);
            if drop > 25.0 && !self.lid_open {
                self.lid_open = true;
                self.pit_reference_c = Some(past_temp);
                new_events.push((EventIcon::Warning, String::from("Lid open")));
                self.frozen_etas = Some(std::array::from_fn(|i| {
                    self.probes[i].eta.and_then(|eta| eta.seconds)
                }));
            } else if self.lid_open
                && let Some(reference) = self.pit_reference_c
                && celsius_to_fahrenheit(reference - now_temp).abs() <= 10.0
            {
                self.lid_open = false;
                self.pit_reference_c = None;
                self.frozen_etas = None;
                new_events.push((EventIcon::Milestone, String::from("Pit recovered")));
            }
        }
    }

    fn compute_summary(&mut self, now: AppInstant) {
        let mut all_done: Option<f64> = None;
        for index in 0..PROBE_COUNT {
            if self.is_pit(index) {
                continue;
            }
            let seconds = if self.settings.cook.pause_eta_on_lid_open && self.lid_open {
                self.frozen_etas
                    .and_then(|frozen| frozen[index])
                    .or_else(|| self.probes[index].eta.and_then(|eta| eta.seconds))
            } else {
                self.probes[index].eta.and_then(|eta| eta.seconds)
            };
            if let Some(seconds) = seconds {
                all_done = Some(all_done.map_or(seconds, |current: f64| current.max(seconds)));
            }
        }
        self.all_done_seconds = all_done;
        self.serve_wall = all_done.map(|seconds| {
            let serve_at = AppInstant::from_secs_f64(
                now.secs() + seconds + self.settings.cook.rest_min * 60.0,
            );
            self.clock.wall(serve_at)
        });
        self.serve_late = match (
            &self.serve_wall,
            parse_hhmm(&self.settings.cook.want_to_eat_at),
        ) {
            (Some(serve), Some((want_h, want_m))) => {
                let serve_minutes = serve.hour() * 60 + serve.minute();
                serve_minutes > want_h * 60 + want_m
            }
            _ => false,
        };
    }

    fn snapshot(&self) -> Snapshot {
        let probes = (0..PROBE_COUNT)
            .map(|index| {
                let cfg = self.settings.probe(index);
                let is_pit = self.settings.is_pit(index);
                ProbeSnapshot {
                    id: ProbeId::from_index(index),
                    label: cfg.label.clone(),
                    temp_c: self.probes[index].temp_c,
                    target_c: cfg.target_c,
                    alert_high_c: cfg.alert_high_c,
                    alert_low_c: cfg.alert_low_c,
                    pre_alert_c: cfg.pre_alert_c,
                    is_pit,
                    range_c: cfg.range_c,
                    stalled: self.probes[index].stalled,
                }
            })
            .collect();
        Snapshot {
            probes,
            battery: self.battery,
            last_reading_at: self.last_reading_at,
            serve_late: self.serve_late,
            alert_on_stall: self.settings.alarms.alert_on_stall,
        }
    }

    fn log_cleared(&mut self, now: AppInstant) {
        let cleared = self.engine.cleared_recent(now);
        if cleared.len() > self.cleared_seen {
            for (key, _) in &cleared[self.cleared_seen..] {
                let message = format!("{} cleared", kind_label(key.1));
                self.log_event(EventIcon::Info, message);
            }
            self.cleared_seen = cleared.len();
        }
        self.alerts_view.cleared = cleared;
    }

    async fn sync_alerts_view(&mut self, now: AppInstant, snapshot: &Snapshot) {
        let serve_at = self.serve_wall.map(|wall| wall.format("%H:%M").to_string());
        let mut shared = self.shared.lock().await;
        shared.sync(
            &self.engine,
            snapshot,
            self.unit(),
            self.battery,
            serve_at,
            now,
        );
        let mut rows: Vec<AlertRow> = shared
            .entries()
            .iter()
            .map(|(key, info): (&AlertKey, &ActiveInfo)| AlertRow {
                key: *key,
                priority: info.priority,
                since: info.since,
                last_spoken: info.last_spoken,
                acked: info.acked,
                snoozed_until: info.snoozed_until,
            })
            .collect();
        rows.sort_by(|a, b| {
            a.priority.cmp(&b.priority).then_with(|| {
                b.since
                    .partial_cmp(&a.since)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let banner = rows.iter().find(|row| !row.acked).map(|row| Banner {
            text: shared.build_text(&row.key),
            priority: row.priority,
        });
        drop(shared);
        self.alerts_view.banner = banner;
        self.alerts_view.rows = rows;
    }

    fn maybe_speak_status(&mut self, now: AppInstant) {
        let every = self.settings.speech.status_every_min;
        if every == 0 {
            self.next_status_at = None;
            return;
        }
        let due = self
            .next_status_at
            .is_none_or(|at| now.since(at) >= f64::from(every) * 60.0);
        if due && self.connected() {
            self.next_status_at = Some(now);
            self.notifier.status();
        }
    }

    fn log_event(&mut self, icon: EventIcon, message: String) {
        let now = self.now;
        let wall = self.clock.wall(now);
        self.events.push(Event {
            wall,
            icon,
            message: message.clone(),
        });
        let temps = self.temps_c();
        let battery = self.battery;
        self.logger.write_row(now, wall, &temps, battery, &message);
    }

    // ---- input ------------------------------------------------------------

    async fn on_input(&mut self, event: CrosstermEvent) {
        let CrosstermEvent::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                self.show_help = false;
            }
            return;
        }
        if self.quit_prompt {
            match key.code {
                KeyCode::Char('y' | 'Y') => self.should_quit = true,
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.quit_prompt = false,
                _ => {}
            }
            return;
        }
        if self.settings_ui.preset_popup || self.settings_ui.editing {
            ui::settings::handle_key(self, key);
            return;
        }

        // Keys that are global on every tab, including Settings.
        match key.code {
            KeyCode::Char('q') => {
                self.request_quit();
                return;
            }
            KeyCode::Char('x') => {
                self.ack_all();
                return;
            }
            KeyCode::Char('z') => {
                self.snooze_all();
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Dashboard;
                return;
            }
            KeyCode::Char('2') => {
                self.tab = Tab::Table;
                return;
            }
            KeyCode::Char('3') => {
                self.tab = Tab::Alarms;
                return;
            }
            KeyCode::Char('4') => {
                self.tab = Tab::Log;
                return;
            }
            KeyCode::Char('5') => {
                self.tab = Tab::Settings;
                return;
            }
            _ => {}
        }

        if self.tab == Tab::Settings {
            ui::settings::handle_key(self, key);
            return;
        }

        match key.code {
            KeyCode::Left => self.tab = self.tab.prev(),
            KeyCode::Right => self.tab = self.tab.next(),
            _ => {}
        }

        if self.demo {
            self.handle_demo_keys(key);
        }
        match self.tab {
            Tab::Dashboard => self.handle_dashboard_key(key),
            Tab::Table => self.handle_table_key(key),
            Tab::Alarms => self.handle_alarms_key(key),
            Tab::Log => self.handle_log_key(key),
            Tab::Settings => {}
        }
    }

    fn handle_demo_keys(&mut self, key: KeyEvent) {
        let slot = match key.code {
            KeyCode::Char('!') => 0,
            KeyCode::Char('@') => 1,
            KeyCode::Char('#') => 2,
            KeyCode::Char('$') => 3,
            _ => return,
        };
        self.demo_unplugged[slot] = !self.demo_unplugged[slot];
        let state = if self.demo_unplugged[slot] {
            "unplugged"
        } else {
            "plugged back in"
        };
        let label = self.probe_label(slot);
        self.log_event(EventIcon::Info, format!("{label} {state}"));
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.selected_card = self.selected_card.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected_card = (self.selected_card + 1).min(PROBE_COUNT - 1);
            }
            KeyCode::Char('w') if !self.is_pit(self.selected_card) => {
                self.probes[self.selected_card].wrapped_at = Some(self.now);
                let label = self.probe_label(self.selected_card);
                self.log_event(EventIcon::Info, format!("{label} wrapped"));
            }
            _ => {}
        }
    }

    fn handle_table_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.table_scroll = self.table_scroll.saturating_sub(1),
            KeyCode::Down => self.table_scroll += 1,
            KeyCode::PageUp => self.table_scroll = self.table_scroll.saturating_sub(10),
            KeyCode::PageDown => self.table_scroll += 10,
            KeyCode::Home => self.table_scroll = 0,
            KeyCode::End => self.table_scroll = usize::MAX,
            KeyCode::Char('i') => {
                self.settings.display.table_interval_min =
                    match self.settings.display.table_interval_min {
                        1 => 5,
                        5 => 15,
                        _ => 1,
                    };
                self.table_scroll = 0;
            }
            KeyCode::Char('e') => self.export_history(),
            _ => {}
        }
    }

    fn handle_alarms_key(&mut self, key: KeyEvent) {
        let count = self.alerts_view.rows.len();
        match key.code {
            KeyCode::Up => self.alarms_selected = self.alarms_selected.saturating_sub(1),
            KeyCode::Down => {
                if count > 0 {
                    self.alarms_selected = (self.alarms_selected + 1).min(count - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(row) = self.alerts_view.rows.get(self.alarms_selected) {
                    let key = row.key;
                    let snapshot = self.snapshot();
                    self.engine.ack(key, self.now, &snapshot);
                }
            }
            KeyCode::Char('t') => self.notifier.test(),
            _ => {}
        }
    }

    fn handle_log_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.log_scroll = self.log_scroll.saturating_sub(1),
            KeyCode::Down => self.log_scroll += 1,
            KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_sub(10),
            KeyCode::PageDown => self.log_scroll += 10,
            KeyCode::Home => self.log_scroll = 0,
            KeyCode::End => self.log_scroll = usize::MAX,
            _ => {}
        }
    }

    fn export_history(&mut self) {
        let dir = self.settings.csv_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            self.log_event(
                EventIcon::Warning,
                String::from("Could not create export directory"),
            );
            return;
        }
        let wall = self.clock.wall(self.now);
        let path = dir.join(format!("history-{}.csv", wall.format("%Y-%m-%d-%H%M")));
        let Ok(mut file) = std::fs::File::create(&path) else {
            self.log_event(
                EventIcon::Warning,
                String::from("Could not create export file"),
            );
            return;
        };
        use std::collections::BTreeMap;
        use std::io::Write as _;
        let mut table: BTreeMap<i64, [Option<f32>; PROBE_COUNT]> = BTreeMap::new();
        for index in 0..PROBE_COUNT {
            for (at, temp) in self.probes[index].history.samples() {
                let entry = table.entry(at.secs() as i64).or_insert([None; PROBE_COUNT]);
                entry[index] = *temp;
            }
        }
        let _ = writeln!(file, "elapsed_s,p1_c,p2_c,p3_c,p4_c");
        for (elapsed, temps) in table {
            let cells: Vec<String> = temps
                .iter()
                .map(|value| value.map_or_else(String::new, |temp| format!("{temp:.1}")))
                .collect();
            let _ = writeln!(file, "{elapsed},{}", cells.join(","));
        }
        let _ = file.flush();
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.log_event(EventIcon::Info, format!("Exported history to {name}"));
    }

    fn request_quit(&mut self) {
        if self.connected() {
            self.quit_prompt = true;
        } else {
            self.should_quit = true;
        }
    }

    fn ack_all(&mut self) {
        let snapshot = self.snapshot();
        self.engine.ack_all(self.now, &snapshot);
    }

    fn snooze_all(&mut self) {
        let minutes = f64::from(self.settings.alarms.snooze_min);
        self.engine.snooze(self.now, minutes);
    }

    // ---- settings ---------------------------------------------------------

    /// Apply and persist a settings change: propagate to the subsystems and
    /// write the config file (throttled so held keys do not thrash the disk).
    pub(crate) fn commit_settings(&mut self) {
        self.engine.set_limits(
            self.settings.alarms.repeat_s,
            self.settings.alarms.bt_lost_after_min,
            self.settings.alarms.unplugged_after_s,
        );
        self.notifier.update_settings(&self.settings);
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

fn kind_label(kind: AlertKind) -> &'static str {
    kind.label()
}

fn alert_message(key: &AlertKey, snapshot: &Snapshot) -> String {
    let probe = key
        .0
        .and_then(|id| snapshot.probes.iter().find(|probe| probe.id == id));
    let label = probe.map_or_else(String::new, |probe| probe.label.clone());
    match key.1 {
        AlertKind::TargetReached => format!("{label} reached target"),
        AlertKind::PreAlert => format!("{label} approaching target"),
        AlertKind::AboveHigh => format!("{label} above high"),
        AlertKind::BelowLow => format!("{label} below low"),
        AlertKind::ProbeUnplugged => format!("{label} unplugged"),
        AlertKind::BluetoothLost => String::from("Bluetooth lost"),
        AlertKind::StallStarted => format!("{label} stalled"),
        AlertKind::StallEnded => format!("{label} stall ended"),
        AlertKind::BatteryLow => String::from("Battery low"),
        AlertKind::ServeLate => String::from("Serve time is late"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Build a demo-mode app on a throwaway config path.
    pub(super) async fn harness() -> (App, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bbq-watch-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let settings = config::load(&path).unwrap();
        let cli = Cli {
            demo: true,
            speed: 1.0,
            config: Some(path.clone()),
            no_speech: true,
        };
        let clock = AppClock::new(1.0);
        let (app, _device_rx, _error_rx) = build(cli, settings, clock, path.clone()).await;
        (app, path)
    }

    pub(super) async fn press(app: &mut App, code: KeyCode) {
        app.on_input(CrosstermEvent::Key(KeyEvent::new(
            code,
            ratatui::crossterm::event::KeyModifiers::NONE,
        )))
        .await;
    }

    pub(super) async fn press_mod(
        app: &mut App,
        code: KeyCode,
        modifiers: ratatui::crossterm::event::KeyModifiers,
    ) {
        app.on_input(CrosstermEvent::Key(KeyEvent::new(code, modifiers)))
            .await;
    }

    pub(super) async fn type_str(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character)).await;
        }
    }

    #[allow(dead_code)]
    pub(super) fn render(app: &App) -> String {
        let backend = TestBackend::new(132, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn cleanup(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[tokio::test]
    async fn esc_never_discards_settings() {
        let (mut app, path) = harness().await;
        press(&mut app, KeyCode::Char('5')).await;
        press(&mut app, KeyCode::Enter).await;
        press_mod(
            &mut app,
            KeyCode::Char('u'),
            ratatui::crossterm::event::KeyModifiers::CONTROL,
        )
        .await;
        type_str(&mut app, "Pork Shoulder").await;
        press(&mut app, KeyCode::Enter).await;
        for _ in 0..3 {
            press(&mut app, KeyCode::Esc).await;
        }
        assert_eq!(app.settings.probes[0].label, "Pork Shoulder");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Pork Shoulder"), "config not saved:\n{text}");
        cleanup(&path);
    }
}
