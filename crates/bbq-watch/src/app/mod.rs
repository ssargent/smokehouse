//! Application state, event loop and key handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Timelike as _};
use futures::StreamExt as _;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind};
use std::sync::Mutex;
use tokio::sync::mpsc;

use crate::alerts::{
    ActiveAlerts, ActiveInfo, AlertEngine, AlertKey, AlertKind, Priority, ProbeSnapshot, Snapshot,
};
use crate::analysis::{EtaResult, ProbeHistory, StallTracker};
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
    /// Last temperature seen before a disconnect, shown dimmed.
    pub(crate) last_seen_c: Option<f32>,
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

    pub(crate) probes: [ProbeState; PROBE_COUNT],

    pub(crate) engine: AlertEngine,
    pub(crate) shared: Arc<Mutex<ActiveAlerts>>,
    pub(crate) error_events: HashMap<String, AppInstant>,
    pub(crate) alerts_view: AlertsView,
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
        clock.clone(),
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
    caffeinate.set(settings.data.keep_awake);

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
        probes: std::array::from_fn(|_| ProbeState::default()),
        engine,
        shared,
        alerts_view: AlertsView::default(),
        error_events: HashMap::new(),
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

    /// Format a simulated instant as a wall-clock time honouring the
    /// 24-hour setting.
    #[must_use]
    pub(crate) fn fmt_time(&self, at: AppInstant) -> String {
        self.fmt_wall(self.clock.wall(at))
    }

    /// Format a simulated instant as a wall-clock time with seconds.
    #[must_use]
    pub(crate) fn fmt_time_hms(&self, at: AppInstant) -> String {
        let wall = self.clock.wall(at);
        let format = if self.settings.display.clock_24h {
            "%H:%M:%S"
        } else {
            "%I:%M:%S %p"
        };
        wall.format(format).to_string()
    }

    /// Format an already-resolved wall-clock time.
    #[must_use]
    pub(crate) fn fmt_wall(&self, wall: DateTime<Local>) -> String {
        let format = if self.settings.display.clock_24h {
            "%H:%M"
        } else {
            "%I:%M %p"
        };
        wall.format(format).to_string()
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
                        self.on_input(event);
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
        self.caffeinate.stop();
        let _ = self.device_tx.send(DeviceCommand::Shutdown).await;
        self.notifier.shutdown();
    }

    // ---- device -----------------------------------------------------------

    // ---- tick -------------------------------------------------------------

    async fn on_tick(&mut self) {
        let now = self.clock.now();
        self.now = now;
        let mut new_events: Vec<(EventIcon, String)> = Vec::new();

        self.detect_lid(now, &mut new_events);
        self.compute_summary(now);
        self.check_reading_watchdog(now);

        let snapshot = self.snapshot();
        let evaluation = self.engine.evaluate(&snapshot, now);
        for (key, _) in &evaluation.cleared {
            new_events.push((EventIcon::Info, format!("{} cleared", kind_label(key.kind))));
        }
        for delivery in evaluation.deliveries {
            if !matches!(
                delivery.key.kind,
                AlertKind::StallStarted | AlertKind::StallEnded
            ) {
                let icon = match delivery.priority {
                    Priority::Urgent => EventIcon::Urgent,
                    Priority::Normal => EventIcon::Warning,
                };
                new_events.push((icon, crate::alerts::short_text(&delivery.key, &snapshot)));
            }
            if self.settings.alarms.bell {
                self.bell_pending = true;
            }
            self.notifier.deliver(delivery);
        }

        for (icon, message) in new_events {
            self.log_event(icon, message);
        }

        self.alerts_view.cleared = self.engine.cleared_recent(now);
        self.sync_alerts_view(now, &snapshot);

        if self.logger.due(now) {
            let temps = self.temps_c();
            let wall = self.clock.wall(now);
            let battery = self.battery;
            self.logger.write_row(now, wall, &temps, battery, "");
        }

        self.maybe_speak_status(now);
        self.caffeinate.poll();
        self.flush_settings(false);
    }
}

fn kind_label(kind: AlertKind) -> &'static str {
    kind.label()
}

/// The next local `hour:minute` at or after `start`, rolling past a day if that
/// time has already passed — so a serve time after midnight still compares.
fn serve_deadline(start: DateTime<Local>, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let base = start
        .with_hour(hour)?
        .with_minute(minute)?
        .with_second(0)?
        .with_nanosecond(0)?;
    Some(if base < start {
        base + chrono::Duration::days(1)
    } else {
        base
    })
}

mod alerts;
mod cook;
mod device_events;
mod export;
mod input;
mod settings;
#[cfg(test)]
mod tests;
