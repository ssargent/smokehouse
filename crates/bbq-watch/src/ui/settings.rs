//! Settings tab: a sidebar plus one scrollable pane. Probes and presets are
//! edited through dedicated forms; the option sections are a flat field list.

use std::cell::RefCell;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Cell, Clear, List, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState,
};

use super::theme;
use crate::app::App;
use crate::config::{
    CUSTOM_PRESET_ID, CUSTOM_PRESET_NAME, ChartWindow, DeviceScreenUnits, EtaMethod, Preset,
    Settings, parse_hhmm,
};
use crate::device::DeviceCommand;
use crate::model::{PROBE_COUNT, Unit, celsius_to_fahrenheit, fahrenheit_to_celsius};

/// Sidebar sections, in fixed order.
const SECTIONS: usize = 7;
/// Sidebar labels, matched to [`SECTIONS`].
const SECTION_TITLES: [&str; SECTIONS] = [
    "Probes",
    "Presets",
    "Alarms",
    "Speech",
    "Cook & estimates",
    "Display",
    "Device & data",
];
/// Width of the sidebar.
const SIDEBAR_WIDTH: u16 = 22;
/// Width of the overlay dialogs.
const OVERLAY_WIDTH: u16 = 62;

/// Which field the text editor writes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum EditTarget {
    #[default]
    None,
    OptionField,
    ProbeLabel,
    PresetName,
}

/// Focus and overlay state for the Settings tab.
#[derive(Debug, Default)]
pub(crate) struct SettingsUi {
    /// Focused sidebar section.
    pub(crate) section: usize,
    /// Focused field within the pane (option sections).
    pub(crate) field: usize,
    /// Probe row of the read-only table, and the probe a form edits.
    pub(crate) probe_row: usize,
    /// Cursor over the preset list (the last row is "＋ New preset").
    pub(crate) preset_index: usize,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    edit_target: EditTarget,
    /// Open probe form, if any.
    probe_form: Option<ProbeForm>,
    /// Open preset form, if any.
    preset_form: Option<PresetForm>,
    /// Open picker, if any.
    picker: Option<Picker>,
    pub(crate) confirm_delete: Option<usize>,
    /// Scroll state for the option pane.
    list_state: RefCell<ListState>,
}

impl SettingsUi {
    /// Whether any overlay (popup or text edit) is open.
    #[must_use]
    pub(crate) fn overlay_active(&self) -> bool {
        self.editing
            || self.probe_form.is_some()
            || self.preset_form.is_some()
            || self.picker.is_some()
            || self.confirm_delete.is_some()
    }
}

/// The open probe form.
#[derive(Clone, Copy, Debug)]
struct ProbeForm {
    probe: usize,
    field: usize,
}

/// The open preset form.
#[derive(Clone, Copy, Debug)]
struct PresetForm {
    index: usize,
    field: usize,
    /// A freshly-created preset whose name is being typed.
    new: bool,
}

/// The open picker overlay.
#[derive(Clone, Debug)]
struct Picker {
    target: PickerTarget,
    index: usize,
    items: Vec<String>,
}

/// What a picker selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerTarget {
    ProbePreset,
    Voice,
    AudioDevice,
}

/// Kind of a settings field.
enum Kind {
    Toggle,
    Spinner { unit: &'static str },
    Segmented { options: &'static [&'static str] },
    Text,
    Time,
    Voice,
    AudioDevice,
    Button(Action),
}

/// A button action.
#[derive(Clone, Copy)]
enum Action {
    TestAlarm,
    PreviewVoice,
    Scan,
    Forget,
}

/// The displayed value of a field.
enum Value {
    Bool(bool),
    Text(String),
    Index(usize),
}

type GetFn = fn(&Settings) -> Value;
type StepFn = fn(&mut Settings, i32);
type ToggleFn = fn(&mut Settings);
type TextFn = fn(&mut Settings, &str);

struct Field {
    label: &'static str,
    help: &'static str,
    kind: Kind,
    get: GetFn,
    step: StepFn,
    toggle: ToggleFn,
    set_text: TextFn,
}

fn noop_step(_settings: &mut Settings, _dir: i32) {}
fn noop_toggle(_settings: &mut Settings) {}
fn noop_text(_settings: &mut Settings, _value: &str) {}

fn toggle(label: &'static str, help: &'static str, get: GetFn, toggle: ToggleFn) -> Field {
    Field {
        label,
        help,
        kind: Kind::Toggle,
        get,
        step: noop_step,
        toggle,
        set_text: noop_text,
    }
}

fn spinner(
    label: &'static str,
    help: &'static str,
    unit: &'static str,
    get: GetFn,
    step_fn: StepFn,
) -> Field {
    Field {
        label,
        help,
        kind: Kind::Spinner { unit },
        get,
        step: step_fn,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn segmented(
    label: &'static str,
    help: &'static str,
    options: &'static [&'static str],
    get: GetFn,
    step: StepFn,
) -> Field {
    Field {
        label,
        help,
        kind: Kind::Segmented { options },
        get,
        step,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn text(label: &'static str, help: &'static str, get: GetFn, set_text: TextFn) -> Field {
    Field {
        label,
        help,
        kind: Kind::Text,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text,
    }
}

fn time(label: &'static str, help: &'static str, get: GetFn, set_text: TextFn) -> Field {
    Field {
        label,
        help,
        kind: Kind::Time,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text,
    }
}

fn choice(label: &'static str, help: &'static str, kind: Kind, get: GetFn) -> Field {
    Field {
        label,
        help,
        kind,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn button(label: &'static str, help: &'static str, action: Action) -> Field {
    Field {
        label,
        help,
        kind: Kind::Button(action),
        get: |_| Value::Text(String::new()),
        step: noop_step,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn clamp_step(value: f64, dir: i32, min: f64, max: f64, step: f64) -> f64 {
    (value + f64::from(dir) * step).clamp(min, max)
}

fn step_index(index: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as i32;
    let next = (index as i32 + dir).rem_euclid(len);
    next as usize
}

/// Fields for an option section (0 is Probes, 1 is Presets).
fn fields(section: usize) -> Vec<Field> {
    match section {
        2 => alarms_fields(),
        3 => speech_fields(),
        4 => cook_fields(),
        5 => display_fields(),
        6 => device_fields(),
        _ => Vec::new(),
    }
}

fn alarms_fields() -> Vec<Field> {
    vec![
        toggle(
            "Sound on this Mac",
            "Play a sound when an alert fires.",
            |s| Value::Bool(s.alarms.sound),
            |s| s.alarms.sound = !s.alarms.sound,
        ),
        text(
            "Sound file",
            "Sound name in /System/Library/Sounds.",
            |s| Value::Text(s.alarms.sound_file.clone()),
            |s, v| s.alarms.sound_file = v.to_string(),
        ),
        toggle(
            "macOS banner",
            "Show a macOS notification banner.",
            |s| Value::Bool(s.alarms.banner),
            |s| s.alarms.banner = !s.alarms.banner,
        ),
        toggle(
            "Push via ntfy",
            "Send alerts to the ntfy topic below.",
            |s| Value::Bool(s.alarms.ntfy),
            |s| s.alarms.ntfy = !s.alarms.ntfy,
        ),
        text(
            "ntfy topic",
            "ntfy.sh topic to publish alerts to.",
            |s| Value::Text(s.alarms.ntfy_topic.clone()),
            |s, v| s.alarms.ntfy_topic = v.to_string(),
        ),
        toggle(
            "Terminal bell",
            "Ring the terminal bell on an alert.",
            |s| Value::Bool(s.alarms.bell),
            |s| s.alarms.bell = !s.alarms.bell,
        ),
        toggle(
            "Alert on stall",
            "Announce the start and end of a stall.",
            |s| Value::Bool(s.alarms.alert_on_stall),
            |s| s.alarms.alert_on_stall = !s.alarms.alert_on_stall,
        ),
        spinner(
            "Repeat every",
            "Seconds before an unacknowledged alert repeats.",
            "s",
            |s| Value::Text(s.alarms.repeat_s.to_string()),
            |s, d| {
                s.alarms.repeat_s =
                    clamp_step(f64::from(s.alarms.repeat_s), d, 15.0, 600.0, 15.0) as u32;
            },
        ),
        spinner(
            "Snooze",
            "Minutes a snooze silences new alerts.",
            "min",
            |s| Value::Text(s.alarms.snooze_min.to_string()),
            |s, d| {
                s.alarms.snooze_min =
                    clamp_step(f64::from(s.alarms.snooze_min), d, 1.0, 60.0, 1.0) as u32;
            },
        ),
        spinner(
            "Probe unplugged after",
            "Seconds a probe must read nothing before alerting.",
            "s",
            |s| Value::Text(s.alarms.unplugged_after_s.to_string()),
            |s, d| {
                s.alarms.unplugged_after_s =
                    clamp_step(f64::from(s.alarms.unplugged_after_s), d, 5.0, 300.0, 5.0) as u32;
            },
        ),
        spinner(
            "Bluetooth lost after",
            "Minutes without readings before a Bluetooth alert.",
            "min",
            |s| Value::Text(s.alarms.bt_lost_after_min.to_string()),
            |s, d| {
                s.alarms.bt_lost_after_min =
                    clamp_step(f64::from(s.alarms.bt_lost_after_min), d, 1.0, 30.0, 1.0) as u32;
            },
        ),
        button("Test alarm", "Fire a test alert now.", Action::TestAlarm),
    ]
}

fn speech_fields() -> Vec<Field> {
    vec![
        toggle(
            "Spoken alerts",
            "Speak alerts out loud.",
            |s| Value::Bool(s.speech.enabled),
            |s| s.speech.enabled = !s.speech.enabled,
        ),
        choice("Voice", "The say(1) voice to use.", Kind::Voice, |s| {
            if s.speech.voice.is_empty() {
                Value::Text(String::from("System default"))
            } else {
                Value::Text(s.speech.voice.clone())
            }
        }),
        spinner(
            "Rate",
            "Speaking rate in words per minute.",
            "wpm",
            |s| Value::Text(s.speech.rate_wpm.to_string()),
            |s, d| {
                s.speech.rate_wpm =
                    clamp_step(f64::from(s.speech.rate_wpm), d, 90.0, 360.0, 10.0) as u32;
            },
        ),
        choice(
            "Audio output",
            "Audio device for spoken alerts.",
            Kind::AudioDevice,
            |s| {
                if s.speech.audio_device.is_empty() {
                    Value::Text(String::from("Default"))
                } else {
                    Value::Text(s.speech.audio_device.clone())
                }
            },
        ),
        toggle(
            "Urgent only",
            "Skip normal-priority alerts.",
            |s| Value::Bool(s.speech.urgent_only),
            |s| s.speech.urgent_only = !s.speech.urgent_only,
        ),
        text(
            "Quiet hours",
            "Window with no speaking, e.g. 23:00-07:00.",
            |s| Value::Text(s.speech.quiet_hours.clone()),
            |s, v| s.speech.quiet_hours = v.to_string(),
        ),
        spinner(
            "Spoken status every",
            "Speak a status summary this often (0 = off).",
            "min",
            |s| Value::Text(s.speech.status_every_min.to_string()),
            |s, d| {
                s.speech.status_every_min =
                    clamp_step(f64::from(s.speech.status_every_min), d, 0.0, 60.0, 5.0) as u32;
            },
        ),
        button(
            "Preview voice",
            "Speak a short preview with the current settings.",
            Action::PreviewVoice,
        ),
    ]
}

fn cook_fields() -> Vec<Field> {
    vec![
        segmented(
            "ETA method",
            "How finish times are estimated.",
            &["Stall-aware", "Linear"],
            |s| Value::Index(usize::from(matches!(s.cook.eta_method, EtaMethod::Linear))),
            |s, d| {
                let linear = step_index(
                    usize::from(matches!(s.cook.eta_method, EtaMethod::Linear)),
                    2,
                    d,
                ) == 1;
                s.cook.eta_method = if linear {
                    EtaMethod::Linear
                } else {
                    EtaMethod::StallAware
                };
            },
        ),
        spinner(
            "Rate window",
            "Minutes of history used for the current rate.",
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.rate_window_min)),
            |s, d| s.cook.rate_window_min = clamp_step(s.cook.rate_window_min, d, 10.0, 120.0, 5.0),
        ),
        spinner(
            "Stall rate",
            "Rise slower than this counts as a stall.",
            "°F/h",
            |s| Value::Text(format!("{:.1}", s.cook.stall_rate_f_per_h)),
            |s, d| {
                s.cook.stall_rate_f_per_h =
                    clamp_step(f64::from(s.cook.stall_rate_f_per_h), d, 1.0, 6.0, 0.5) as f32;
            },
        ),
        spinner(
            "Stall minutes",
            "How long below the stall rate counts as a stall.",
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.stall_minutes)),
            |s, d| s.cook.stall_minutes = clamp_step(s.cook.stall_minutes, d, 5.0, 60.0, 5.0),
        ),
        spinner(
            "Stall min",
            "Lower temperature bound of the stall band.",
            "°F",
            |s| Value::Text(format!("{:.0}", celsius_to_fahrenheit(s.cook.stall_min_c))),
            |s, d| {
                let f = f64::from(celsius_to_fahrenheit(s.cook.stall_min_c));
                s.cook.stall_min_c =
                    fahrenheit_to_celsius(clamp_step(f, d, 100.0, 200.0, 5.0) as f32);
            },
        ),
        spinner(
            "Stall max",
            "Upper temperature bound of the stall band.",
            "°F",
            |s| Value::Text(format!("{:.0}", celsius_to_fahrenheit(s.cook.stall_max_c))),
            |s, d| {
                let f = f64::from(celsius_to_fahrenheit(s.cook.stall_max_c));
                s.cook.stall_max_c =
                    fahrenheit_to_celsius(clamp_step(f, d, 120.0, 220.0, 5.0) as f32);
            },
        ),
        spinner(
            "Rest time",
            "Minutes added after the cook before serving.",
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.rest_min)),
            |s, d| s.cook.rest_min = clamp_step(s.cook.rest_min, d, 0.0, 240.0, 15.0),
        ),
        time(
            "Want to eat at",
            "Target serve time, e.g. 19:30 (empty = off).",
            |s| Value::Text(s.cook.want_to_eat_at.clone()),
            |s, v| s.cook.want_to_eat_at = v.to_string(),
        ),
        toggle(
            "Wrapped resets stall",
            "Wrapping a probe clears an active stall.",
            |s| Value::Bool(s.cook.wrapped_resets_stall),
            |s| s.cook.wrapped_resets_stall = !s.cook.wrapped_resets_stall,
        ),
        toggle(
            "Pause ETA while lid open",
            "Freeze finish estimates while the lid is open.",
            |s| Value::Bool(s.cook.pause_eta_on_lid_open),
            |s| s.cook.pause_eta_on_lid_open = !s.cook.pause_eta_on_lid_open,
        ),
    ]
}

fn display_fields() -> Vec<Field> {
    vec![
        segmented(
            "Units",
            "Temperature unit for the whole app.",
            &["°F", "°C"],
            |s| Value::Index(usize::from(matches!(s.display.units, Unit::Celsius))),
            |s, d| {
                let celsius =
                    step_index(usize::from(matches!(s.display.units, Unit::Celsius)), 2, d) == 1;
                s.display.units = if celsius {
                    Unit::Celsius
                } else {
                    Unit::Fahrenheit
                };
            },
        ),
        segmented(
            "Chart window",
            "How much history the chart shows.",
            &["whole", "6h", "2h", "30m"],
            |s| Value::Index(s.display.chart_window as usize),
            |s, d| {
                let index = step_index(s.display.chart_window as usize, ChartWindow::ALL.len(), d);
                s.display.chart_window = ChartWindow::ALL[index];
            },
        ),
        segmented(
            "Table interval",
            "Minutes between rows in the table.",
            &["1", "5", "15"],
            |s| {
                Value::Index(match s.display.table_interval_min {
                    1 => 0,
                    5 => 1,
                    _ => 2,
                })
            },
            |s, d| {
                let options = [1_u32, 5, 15];
                let index = step_index(
                    options
                        .iter()
                        .position(|v| *v == s.display.table_interval_min)
                        .unwrap_or(1),
                    options.len(),
                    d,
                );
                s.display.table_interval_min = options[index];
            },
        ),
        spinner(
            "Refresh",
            "Redraw interval in milliseconds.",
            "ms",
            |s| Value::Text(s.display.refresh_ms.to_string()),
            |s, d| {
                s.display.refresh_ms =
                    clamp_step(s.display.refresh_ms as f64, d, 250.0, 5000.0, 250.0) as u64;
            },
        ),
        text(
            "Theme",
            "Colour theme name.",
            |s| Value::Text(s.display.theme.clone()),
            |s, v| s.display.theme = v.to_string(),
        ),
        toggle(
            "Show target lines",
            "Draw meat target lines on the chart.",
            |s| Value::Bool(s.display.show_target_lines),
            |s| s.display.show_target_lines = !s.display.show_target_lines,
        ),
        toggle(
            "Show pit range lines",
            "Draw the pit range band on the chart.",
            |s| Value::Bool(s.display.show_pit_range_lines),
            |s| s.display.show_pit_range_lines = !s.display.show_pit_range_lines,
        ),
        toggle(
            "24-hour clock",
            "Show times as 24-hour rather than AM/PM.",
            |s| Value::Bool(s.display.clock_24h),
            |s| s.display.clock_24h = !s.display.clock_24h,
        ),
        toggle(
            "Compact cards",
            "Draw compact probe cards.",
            |s| Value::Bool(s.display.compact_cards),
            |s| s.display.compact_cards = !s.display.compact_cards,
        ),
    ]
}

fn device_fields() -> Vec<Field> {
    vec![
        button("Scan", "Rescan for the thermometer.", Action::Scan),
        button("Forget", "Forget the saved thermometer.", Action::Forget),
        text(
            "Display name",
            "Name shown instead of the thermometer's own.",
            |s| Value::Text(s.device.display_name.clone()),
            |s, v| s.device.display_name = v.to_string(),
        ),
        toggle(
            "Auto reconnect",
            "Reconnect automatically after a drop.",
            |s| Value::Bool(s.device.auto_reconnect),
            |s| s.device.auto_reconnect = !s.device.auto_reconnect,
        ),
        spinner(
            "Battery check",
            "Minutes between battery polls.",
            "min",
            |s| Value::Text(s.device.battery_check_min.to_string()),
            |s, d| {
                s.device.battery_check_min =
                    clamp_step(f64::from(s.device.battery_check_min), d, 1.0, 30.0, 1.0) as u32;
            },
        ),
        segmented(
            "Device screen units",
            "Units on the thermometer's own screen.",
            &["Match app", "°F", "°C"],
            |s| {
                Value::Index(match s.device.device_screen_units {
                    DeviceScreenUnits::Match => 0,
                    DeviceScreenUnits::Fahrenheit => 1,
                    DeviceScreenUnits::Celsius => 2,
                })
            },
            |s, d| {
                let index = step_index(
                    match s.device.device_screen_units {
                        DeviceScreenUnits::Match => 0,
                        DeviceScreenUnits::Fahrenheit => 1,
                        DeviceScreenUnits::Celsius => 2,
                    },
                    DeviceScreenUnits::ALL.len(),
                    d,
                );
                s.device.device_screen_units = DeviceScreenUnits::ALL[index];
            },
        ),
        toggle(
            "Log to CSV",
            "Write readings to a CSV file.",
            |s| Value::Bool(s.data.csv_enabled),
            |s| s.data.csv_enabled = !s.data.csv_enabled,
        ),
        text(
            "CSV directory",
            "Directory for CSV files (empty = ~/bbq-watch).",
            |s| Value::Text(s.data.csv_dir.clone()),
            |s, v| s.data.csv_dir = v.to_string(),
        ),
        segmented(
            "Sample every",
            "CSV sampling interval.",
            &["1s", "10s", "60s"],
            |s| {
                Value::Index(match s.data.sample_interval_s {
                    1 => 0,
                    10 => 1,
                    _ => 2,
                })
            },
            |s, d| {
                let options = [1_u32, 10, 60];
                let index = step_index(
                    options
                        .iter()
                        .position(|v| *v == s.data.sample_interval_s)
                        .unwrap_or(1),
                    options.len(),
                    d,
                );
                s.data.sample_interval_s = options[index];
            },
        ),
        toggle(
            "Keep Mac awake",
            "Hold a caffeinate assertion while running.",
            |s| Value::Bool(s.data.keep_awake),
            |s| s.data.keep_awake = !s.data.keep_awake,
        ),
    ]
}

// ---- drawing --------------------------------------------------------------

/// Draw the settings tab.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
    let cols =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)]).split(rows[0]);
    draw_sidebar(frame, cols[0], app);
    draw_pane(frame, cols[1], app);
    draw_help(frame, rows[1], app);

    if app.settings_ui.confirm_delete.is_some() {
        draw_confirm(frame, area, app);
    } else if app.settings_ui.picker.is_some() {
        draw_picker(frame, area, app);
    } else if app.settings_ui.probe_form.is_some() {
        draw_probe_form(frame, area, app);
    } else if app.settings_ui.preset_form.is_some() {
        draw_preset_form(frame, area, app);
    }
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = SECTION_TITLES
        .iter()
        .map(|title| ListItem::new(Line::from(format!(" {title}"))))
        .collect();
    let list = List::new(items)
        .block(theme::block(" Settings "))
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶");
    let mut state = ListState::default().with_selected(Some(app.settings_ui.section));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_pane(frame: &mut Frame, area: Rect, app: &App) {
    match app.settings_ui.section {
        0 => draw_probes(frame, area, app),
        1 => draw_presets(frame, area, app),
        section => draw_fields(frame, area, app, section),
    }
}

fn pane_title(section: usize) -> &'static str {
    match section {
        0 => " Probes ",
        1 => " Presets ",
        2 => " Alarms & notifications ",
        3 => " Speech ",
        4 => " Cook & estimates ",
        5 => " Display ",
        _ => " Device & data ",
    }
}

/// Format a temperature stored in Celsius for the current unit.
fn fmt_temp(unit: Unit, celsius: Option<f32>) -> String {
    celsius.map_or_else(|| String::from("—"), |c| format!("{:.0}°", unit.from_c(c)))
}

fn fmt_delta(unit: Unit, celsius: Option<f32>) -> String {
    celsius.map_or_else(
        || String::from("—"),
        |c| format!("{:.0}°", unit.delta_from_c(c)),
    )
}

fn draw_probes(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let rows: Vec<Row> = (0..PROBE_COUNT)
        .map(|index| {
            let cfg = app.settings.probe(index);
            let pit = app.settings.is_pit(index);
            let target = if pit {
                cfg.range_c.map_or_else(
                    || String::from("—"),
                    |range| format!("{:.0}–{:.0}°", unit.from_c(range[0]), unit.from_c(range[1])),
                )
            } else {
                fmt_temp(unit, cfg.target_c)
            };
            let pre = if pit {
                String::from("—")
            } else {
                fmt_delta(unit, cfg.pre_alert_c)
            };
            Row::new(vec![
                Cell::from(format!(" {}", index + 1)),
                Cell::from(format!(" {}", cfg.label)),
                Cell::from(format!(" {} ", app.settings.preset_name(&cfg.preset))),
                Cell::from(format!(" {target}")),
                Cell::from(format!(" {pre}")),
                Cell::from(format!(" {}", fmt_temp(unit, cfg.alert_low_c))),
                Cell::from(format!(" {}", fmt_temp(unit, cfg.alert_high_c))),
                Cell::from(format!(" {} ", cfg.color.name()))
                    .style(Style::default().fg(theme::probe_color(cfg.color))),
                Cell::from(if cfg.show_on_chart { " [x] " } else { " [ ] " }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(14),
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(6),
    ];
    let header = Row::new(
        [
            " # ", " Label ", " Preset ", " Target ", " Pre ", " Low ", " High ", " Color ",
            " Chart ",
        ]
        .into_iter()
        .map(Cell::from),
    );
    let selected = if app.settings_ui.section == 0 {
        Some(app.settings_ui.probe_row)
    } else {
        None
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(pane_title(0)))
        .row_highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(
        table,
        area,
        &mut TableState::default().with_selected(selected),
    );
}

fn draw_presets(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let mut items: Vec<ListItem> = app
        .settings
        .presets
        .iter()
        .map(|preset| {
            ListItem::new(Line::from(format!(
                " {:<32}{:>14} ",
                preset.name,
                preset_target(unit, preset)
            )))
        })
        .collect();
    items.push(ListItem::new(Line::from(Span::styled(
        " ＋ New preset",
        Style::default().fg(theme::ACCENT),
    ))));
    let list = List::new(items)
        .block(theme::block(pane_title(1)))
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(app.settings_ui.preset_index));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_fields(frame: &mut Frame, area: Rect, app: &App, section: usize) {
    let list = fields(section);
    let block = theme::block(pane_title(section));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let selected = app.settings_ui.field == index;
            ListItem::new(render_field(field, &app.settings, app, selected))
        })
        .collect();
    let widget = List::new(items).highlight_style(
        Style::default()
            .bg(theme::SELECTED_BG)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = app.settings_ui.list_state.borrow_mut();
    state.select(Some(app.settings_ui.field));
    frame.render_stateful_widget(widget, inner, &mut state);

    if list.len() > inner.height as usize {
        let mut scrollbar =
            ScrollbarState::new(list.len().saturating_sub(1)).position(app.settings_ui.field);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut scrollbar,
        );
    }
}

fn render_field(field: &Field, settings: &Settings, app: &App, selected: bool) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG)
    };
    let mut spans = vec![Span::styled(format!(" {:<26}", field.label), label_style)];
    let value = (field.get)(settings);
    match &field.kind {
        Kind::Toggle => {
            let on = matches!(value, Value::Bool(true));
            spans.push(Span::styled(
                if on { "[x]" } else { "[ ]" },
                Style::default().fg(if on { theme::GREEN } else { theme::MUTED }),
            ));
        }
        Kind::Spinner { unit } => {
            let text = if let Value::Text(text) = value {
                text
            } else {
                String::new()
            };
            spans.push(Span::styled("◀ ", Style::default().fg(theme::MUTED)));
            spans.push(Span::styled(
                format!(" {text} "),
                Style::default()
                    .bg(theme::FIELD_BG)
                    .fg(theme::FG)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" ▶ {unit}"),
                Style::default().fg(theme::MUTED),
            ));
        }
        Kind::Segmented { options } => {
            let index = if let Value::Index(index) = value {
                index
            } else {
                0
            };
            for (position, option) in options.iter().enumerate() {
                let active = position == index;
                spans.push(Span::styled(
                    format!(" {option} "),
                    if active {
                        Style::default().bg(theme::ACCENT).fg(theme::BG)
                    } else {
                        Style::default().bg(theme::GAUGE_TRACK).fg(theme::FG)
                    },
                ));
            }
        }
        Kind::Text | Kind::Time | Kind::Voice | Kind::AudioDevice => {
            let text = if let Value::Text(text) = value {
                text
            } else {
                String::new()
            };
            let editing = selected && app.settings_ui.editing;
            let display = if editing {
                format!("{}▏", app.settings_ui.edit_buffer)
            } else {
                format!("{text} ")
            };
            spans.push(Span::styled(
                format!(" {display} "),
                Style::default().bg(theme::FIELD_BG).fg(theme::FG),
            ));
        }
        Kind::Button(_) => {
            spans.push(Span::styled(
                format!(" {} ", field.label),
                Style::default()
                    .bg(theme::MUTED)
                    .fg(theme::BG)
                    .add_modifier(Modifier::BOLD),
            ));
            // The chip is self-describing; drop the leading label.
            spans.remove(0);
        }
    }
    Line::from(spans)
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let text = if app.settings_ui.probe_form.is_some() {
        String::from(
            " ↑/↓ field · ←/→ change · Shift ±5 · Space toggle · Enter edit/choose · Esc back ",
        )
    } else if app.settings_ui.preset_form.is_some() {
        String::from(" ↑/↓ field · ←/→ change · Shift ±5 · Enter edit · Esc back ")
    } else if app.settings_ui.picker.is_some() {
        String::from(" ↑/↓ choose · Enter select · Esc cancel ")
    } else if app.settings_ui.section == 0 {
        String::from(" ↑/↓ probe · Enter edit probe ")
    } else if app.settings_ui.section == 1 {
        String::from(" ↑/↓ preset · Enter edit · the last row adds a new preset ")
    } else {
        let list = fields(app.settings_ui.section);
        list.get(app.settings_ui.field)
            .map_or_else(String::new, |field| format!(" {}", field.help))
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme::MUTED),
        )))
        .style(theme::base()),
        area,
    );
}

/// `203°F`, `225–275°F` for a pit preset, or `—` when the preset sets neither.
fn preset_target(unit: Unit, preset: &Preset) -> String {
    let display = |f: f32| preset_display(unit, f);
    if preset.pit {
        return preset.range_f.map_or_else(
            || String::from("—"),
            |[low, high]| format!("{:.0}–{:.0}°{}", display(low), display(high), unit.symbol()),
        );
    }
    preset.target_f.map_or_else(
        || String::from("—"),
        |target| format!("{:.0}°{}", display(target), unit.symbol()),
    )
}

/// A stored °F preset temperature shown in the current unit.
fn preset_display(unit: Unit, fahrenheit: f32) -> f32 {
    match unit {
        Unit::Fahrenheit => fahrenheit,
        Unit::Celsius => fahrenheit_to_celsius(fahrenheit),
    }
}

/// A value entered in the current unit, stored back as °F.
fn preset_store(unit: Unit, value: f32) -> f32 {
    match unit {
        Unit::Fahrenheit => value,
        Unit::Celsius => celsius_to_fahrenheit(value),
    }
}

/// A popup rectangle of `width`×`height`, centred in `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn draw_picker(frame: &mut Frame, area: Rect, app: &App) {
    let Some(picker) = &app.settings_ui.picker else {
        return;
    };
    let popup = centered(area, OVERLAY_WIDTH, picker.items.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    let items: Vec<ListItem> = picker
        .items
        .iter()
        .map(|item| ListItem::new(Line::from(format!(" {item}"))))
        .collect();
    let hint = Line::from(Span::styled(
        " ↑/↓ choose · Enter select · Esc cancel ",
        Style::default().fg(theme::MUTED),
    ));
    let list = List::new(items)
        .block(theme::block(" Choose ").title_bottom(hint))
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(picker.index));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// The fields of the probe form, depending on whether the probe is a pit probe.
fn probe_form_fields(app: &App, probe: usize) -> Vec<ProbeField> {
    if app.settings.is_pit(probe) {
        vec![
            ProbeField::Label,
            ProbeField::Preset,
            ProbeField::RangeLow,
            ProbeField::RangeHigh,
            ProbeField::LowAlert,
            ProbeField::HighAlert,
            ProbeField::Color,
            ProbeField::Chart,
        ]
    } else {
        vec![
            ProbeField::Label,
            ProbeField::Preset,
            ProbeField::Target,
            ProbeField::PreAlert,
            ProbeField::LowAlert,
            ProbeField::HighAlert,
            ProbeField::Color,
            ProbeField::Chart,
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeField {
    Label,
    Preset,
    Target,
    PreAlert,
    RangeLow,
    RangeHigh,
    LowAlert,
    HighAlert,
    Color,
    Chart,
}

fn draw_probe_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = app.settings_ui.probe_form else {
        return;
    };
    let unit = app.unit();
    let probe = form.probe;
    let cfg = app.settings.probe(probe);
    let fields = probe_form_fields(app, probe);
    let height = fields.len() as u16 + 3;
    let popup = centered(area, OVERLAY_WIDTH, height);
    frame.render_widget(Clear, popup);
    let title = format!(" Edit probe P{} ", probe + 1);
    let block = theme::block(&title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let selected = form.field == index;
            probe_form_line(app, &cfg, *field, selected, unit)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

fn probe_form_line(
    app: &App,
    cfg: &crate::config::ProbeConfig,
    field: ProbeField,
    selected: bool,
    unit: Unit,
) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG)
    };
    let (label, value) = match field {
        ProbeField::Label => ("Label", cfg.label.clone()),
        ProbeField::Preset => ("Preset", app.settings.preset_name(&cfg.preset)),
        ProbeField::Target => (
            "Target",
            spinner_text_unit(fmt_temp(unit, cfg.target_c), unit),
        ),
        ProbeField::PreAlert => (
            "Pre-alert",
            spinner_text(fmt_delta(unit, cfg.pre_alert_c), "°"),
        ),
        ProbeField::RangeLow => (
            "Range low",
            spinner_text_unit(
                cfg.range_c.map_or_else(
                    || String::from("—"),
                    |r| format!("{:.0}", unit.from_c(r[0])),
                ),
                unit,
            ),
        ),
        ProbeField::RangeHigh => (
            "Range high",
            spinner_text_unit(
                cfg.range_c.map_or_else(
                    || String::from("—"),
                    |r| format!("{:.0}", unit.from_c(r[1])),
                ),
                unit,
            ),
        ),
        ProbeField::LowAlert => (
            "Low alert",
            spinner_text_unit(fmt_temp_value(unit, cfg.alert_low_c), unit),
        ),
        ProbeField::HighAlert => (
            "High alert",
            spinner_text_unit(fmt_temp_value(unit, cfg.alert_high_c), unit),
        ),
        ProbeField::Color => ("Color", format!("● {}", cfg.color.name())),
        ProbeField::Chart => (
            "Show on chart",
            if cfg.show_on_chart {
                String::from("[x]")
            } else {
                String::from("[ ]")
            },
        ),
    };
    let editing = selected && app.settings_ui.editing && field == ProbeField::Label;
    let value = if editing {
        format!("{}▏", app.settings_ui.edit_buffer)
    } else {
        value
    };
    Line::from(vec![
        Span::styled(format!(" {label:<16}"), label_style),
        Span::styled(value, Style::default().fg(theme::FG)),
    ])
}

fn fmt_temp_value(unit: Unit, celsius: Option<f32>) -> String {
    celsius.map_or_else(|| String::from("—"), |c| format!("{:.0}", unit.from_c(c)))
}

fn spinner_text(value: String, unit: &str) -> String {
    format!("◀ {value} ▶ {unit}")
}

fn spinner_text_unit(value: String, unit: Unit) -> String {
    format!("◀ {value} ▶ °{}", unit.symbol())
}

fn draw_preset_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = app.settings_ui.preset_form else {
        return;
    };
    let Some(preset) = app.settings.presets.get(form.index) else {
        return;
    };
    let unit = app.unit();
    let fields = preset_form_fields(preset.pit);
    let users = app.settings.preset_users(&preset.id);
    let height = fields.len() as u16 + 3;
    let popup = centered(area, OVERLAY_WIDTH, height);
    frame.render_widget(Clear, popup);
    let title = if form.new {
        String::from(" New preset ")
    } else {
        String::from(" Edit preset ")
    };
    let block = theme::block(&title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let selected = app.settings_ui.preset_form.map(|f| f.field) == Some(index);
            preset_form_line(app, preset, *field, selected, unit)
        })
        .collect();
    let used = if users.is_empty() {
        String::from("Used by: —")
    } else {
        let names: Vec<String> = users.iter().map(|i| format!("P{}", i + 1)).collect();
        format!("Used by: {}", names.join(", "))
    };
    lines.push(Line::from(Span::styled(
        format!(" {used}"),
        Style::default().fg(theme::MUTED),
    )));
    frame.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PresetField {
    Name,
    Type,
    Target,
    RangeLow,
    RangeHigh,
    LowAlert,
    HighAlert,
    Delete,
}

fn preset_form_fields(pit: bool) -> Vec<PresetField> {
    if pit {
        vec![
            PresetField::Name,
            PresetField::Type,
            PresetField::RangeLow,
            PresetField::RangeHigh,
            PresetField::LowAlert,
            PresetField::HighAlert,
            PresetField::Delete,
        ]
    } else {
        vec![
            PresetField::Name,
            PresetField::Type,
            PresetField::Target,
            PresetField::Delete,
        ]
    }
}

fn preset_form_line(
    app: &App,
    preset: &Preset,
    field: PresetField,
    selected: bool,
    unit: Unit,
) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG)
    };
    let display = |f: Option<f32>| {
        f.map_or_else(
            || String::from("—"),
            |value| format!("{:.0}", preset_display(unit, value)),
        )
    };
    let (label, value) = match field {
        PresetField::Name => {
            let editing = selected && app.settings_ui.editing;
            let text = if editing {
                format!("{}▏", app.settings_ui.edit_buffer)
            } else {
                preset.name.clone()
            };
            ("Name", text)
        }
        PresetField::Type => (
            "Type",
            if preset.pit {
                String::from(" Meat │[Pit] ")
            } else {
                String::from("[Meat]│ Pit ")
            },
        ),
        PresetField::Target => ("Target", spinner_text_unit(display(preset.target_f), unit)),
        PresetField::RangeLow => (
            "Range low",
            spinner_text_unit(display(preset.range_f.map(|r| r[0])), unit),
        ),
        PresetField::RangeHigh => (
            "Range high",
            spinner_text_unit(display(preset.range_f.map(|r| r[1])), unit),
        ),
        PresetField::LowAlert => (
            "Low alert",
            spinner_text_unit(display(preset.alert_low_f), unit),
        ),
        PresetField::HighAlert => (
            "High alert",
            spinner_text_unit(display(preset.alert_high_f), unit),
        ),
        PresetField::Delete => ("Delete preset", String::from(" [ Delete ] ")),
    };
    Line::from(vec![
        Span::styled(format!(" {label:<16}"), label_style),
        Span::styled(value, Style::default().fg(theme::FG)),
    ])
}

fn draw_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let Some(index) = app.settings_ui.confirm_delete else {
        return;
    };
    let text = delete_confirm_text(app, index);
    let popup = centered(area, OVERLAY_WIDTH, 3);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme::FG),
        )))
        .block(theme::block(" Confirm "))
        .style(theme::base()),
        popup,
    );
}

/// The confirmation prompt shown before deleting a preset.
fn delete_confirm_text(app: &App, index: usize) -> String {
    let Some(preset) = app.settings.presets.get(index) else {
        return String::new();
    };
    let users: Vec<String> = app
        .settings
        .preset_users(&preset.id)
        .iter()
        .map(|slot| format!("P{}", slot + 1))
        .collect();
    let warning = if users.is_empty() {
        String::new()
    } else {
        format!(" {} uses it and will become Custom.", users.join(", "))
    };
    format!(" Delete \"{}\"?{warning} (y/n) ", preset.name)
}

// ---- input ----------------------------------------------------------------

/// Handle a key press on the Settings tab.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) {
    if app.settings_ui.confirm_delete.is_some() {
        handle_confirm(app, key);
    } else if app.settings_ui.editing {
        handle_edit(app, key);
    } else if app.settings_ui.picker.is_some() {
        handle_picker(app, key);
    } else if app.settings_ui.probe_form.is_some() {
        handle_probe_form(app, key);
    } else if app.settings_ui.preset_form.is_some() {
        handle_preset_form(app, key);
    } else {
        handle_browse(app, key);
    }
}

fn change_section(app: &mut App, delta: i32) {
    let next = (app.settings_ui.section as i32 + delta).rem_euclid(SECTIONS as i32);
    app.settings_ui.section = next as usize;
    app.settings_ui.field = 0;
}

fn handle_browse(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {}
        KeyCode::Tab => change_section(app, 1),
        KeyCode::BackTab => change_section(app, -1),
        KeyCode::Up => move_field(app, -1),
        KeyCode::Down => move_field(app, 1),
        KeyCode::Left if app.settings_ui.section > 1 => step_field(app, -1, false),
        KeyCode::Right if app.settings_ui.section > 1 => step_field(app, 1, false),
        KeyCode::Char('+') | KeyCode::Char('=') if app.settings_ui.section > 1 => {
            step_field(app, 1, true);
        }
        KeyCode::Char('-') | KeyCode::Char('_') if app.settings_ui.section > 1 => {
            step_field(app, -1, true);
        }
        KeyCode::Char(' ') => handle_toggle(app),
        KeyCode::Enter => activate(app),
        _ => {}
    }
}

fn move_field(app: &mut App, direction: i32) {
    match app.settings_ui.section {
        0 => {
            let row = app.settings_ui.probe_row as i32 + direction;
            app.settings_ui.probe_row = row.clamp(0, PROBE_COUNT as i32 - 1) as usize;
        }
        1 => {
            let last = app.settings.presets.len();
            let row = app.settings_ui.preset_index as i32 + direction;
            app.settings_ui.preset_index = row.clamp(0, last as i32) as usize;
        }
        section => {
            let len = fields(section).len().max(1);
            let next = app.settings_ui.field as i32 + direction;
            app.settings_ui.field = next.clamp(0, len as i32 - 1) as usize;
        }
    }
}

/// Step the focused option field (option sections only).
fn step_field(app: &mut App, direction: i32, shift: bool) {
    let list = fields(app.settings_ui.section);
    let Some(field) = list.get(app.settings_ui.field) else {
        return;
    };
    let direction = if shift { direction * 5 } else { direction };
    match field.kind {
        Kind::Segmented { .. } => {
            (field.step)(&mut app.settings, direction.signum());
            app.commit_settings();
        }
        Kind::Spinner { .. } => {
            (field.step)(&mut app.settings, direction);
            app.commit_settings();
        }
        Kind::Voice => open_picker(app, PickerTarget::Voice),
        Kind::AudioDevice => open_picker(app, PickerTarget::AudioDevice),
        _ => {}
    }
}

fn handle_toggle(app: &mut App) {
    if app.settings_ui.section <= 1 {
        return;
    }
    let list = fields(app.settings_ui.section);
    if let Some(field) = list.get(app.settings_ui.field)
        && matches!(field.kind, Kind::Toggle)
    {
        (field.toggle)(&mut app.settings);
        app.commit_settings();
    }
}

fn activate(app: &mut App) {
    match app.settings_ui.section {
        0 => {
            let probe = app.settings_ui.probe_row;
            app.settings_ui.probe_form = Some(ProbeForm { probe, field: 0 });
        }
        1 => {
            let index = app.settings_ui.preset_index;
            if index >= app.settings.presets.len() {
                new_preset(app);
            } else {
                app.settings_ui.preset_form = Some(PresetForm {
                    index,
                    field: 0,
                    new: false,
                });
            }
        }
        section => {
            let list = fields(section);
            let Some(field) = list.get(app.settings_ui.field) else {
                return;
            };
            match field.kind {
                Kind::Text | Kind::Time => begin_edit(app, EditTarget::OptionField),
                Kind::Voice => open_picker(app, PickerTarget::Voice),
                Kind::AudioDevice => open_picker(app, PickerTarget::AudioDevice),
                Kind::Toggle => handle_toggle(app),
                Kind::Button(action) => run_action(app, action),
                _ => {}
            }
        }
    }
}

fn run_action(app: &mut App, action: Action) {
    match action {
        Action::TestAlarm => app.notifier.test(),
        Action::PreviewVoice => app.notifier.preview_voice(),
        Action::Scan => {
            let _ = app.device_tx.try_send(DeviceCommand::Rescan);
        }
        Action::Forget => {
            let _ = app.device_tx.try_send(DeviceCommand::Forget);
        }
    }
}

// ---- text editing ---------------------------------------------------------

fn begin_edit(app: &mut App, target: EditTarget) {
    let buffer = match target {
        EditTarget::OptionField => {
            let list = fields(app.settings_ui.section);
            list.get(app.settings_ui.field)
                .map_or_else(String::new, |field| match (field.get)(&app.settings) {
                    Value::Text(text) => text,
                    _ => String::new(),
                })
        }
        EditTarget::ProbeLabel => app
            .settings_ui
            .probe_form
            .and_then(|form| app.settings.probes.get(form.probe))
            .map_or_else(String::new, |probe| probe.label.clone()),
        EditTarget::PresetName => app
            .settings_ui
            .preset_form
            .and_then(|form| app.settings.presets.get(form.index))
            .map_or_else(String::new, |preset| preset.name.clone()),
        EditTarget::None => String::new(),
    };
    app.settings_ui.editing = true;
    app.settings_ui.edit_buffer = buffer;
    app.settings_ui.edit_target = target;
}

fn handle_edit(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.settings_ui.editing = false;
            app.settings_ui.edit_buffer.clear();
            app.settings_ui.edit_target = EditTarget::None;
        }
        KeyCode::Enter => commit_edit(app),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.settings_ui.edit_buffer.clear();
        }
        KeyCode::Backspace => {
            app.settings_ui.edit_buffer.pop();
        }
        KeyCode::Char(character) => {
            let limit = match app.settings_ui.edit_target {
                EditTarget::ProbeLabel => 18,
                EditTarget::PresetName => 40,
                _ => 64,
            };
            if app.settings_ui.edit_buffer.chars().count() < limit {
                app.settings_ui.edit_buffer.push(character);
            }
        }
        _ => {}
    }
}

fn commit_edit(app: &mut App) {
    let text = app.settings_ui.edit_buffer.clone();
    let target = app.settings_ui.edit_target;
    app.settings_ui.editing = false;
    app.settings_ui.edit_target = EditTarget::None;
    match target {
        EditTarget::ProbeLabel => {
            if let Some(form) = app.settings_ui.probe_form
                && let Some(probe) = app.settings.probes.get_mut(form.probe)
            {
                probe.label = text;
                app.commit_settings();
            }
        }
        EditTarget::PresetName => rename_preset(app, text),
        EditTarget::OptionField => {
            let list = fields(app.settings_ui.section);
            if let Some(field) = list.get(app.settings_ui.field) {
                if let Kind::Time = field.kind
                    && !text.trim().is_empty()
                    && parse_hhmm(&text).is_none()
                {
                    return;
                }
                (field.set_text)(&mut app.settings, &text);
                app.commit_settings();
            }
        }
        EditTarget::None => {}
    }
}

// ---- probe form -----------------------------------------------------------

fn handle_probe_form(app: &mut App, key: KeyEvent) {
    let Some(form) = app.settings_ui.probe_form else {
        return;
    };
    let probe = form.probe;
    let list = probe_form_fields(app, probe);
    let len = list.len();
    match key.code {
        KeyCode::Esc => app.settings_ui.probe_form = None,
        KeyCode::Up => {
            if let Some(form) = app.settings_ui.probe_form.as_mut() {
                form.field = form.field.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(form) = app.settings_ui.probe_form.as_mut() {
                form.field = (form.field + 1).min(len - 1);
            }
        }
        KeyCode::Left => adjust_probe_field(app, probe, list[form.field], -1, false),
        KeyCode::Right => adjust_probe_field(app, probe, list[form.field], 1, false),
        KeyCode::Char('+') | KeyCode::Char('=') => {
            adjust_probe_field(app, probe, list[form.field], 1, true);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            adjust_probe_field(app, probe, list[form.field], -1, true);
        }
        KeyCode::Char(' ') | KeyCode::Enter => match list[form.field] {
            ProbeField::Label => begin_edit(app, EditTarget::ProbeLabel),
            ProbeField::Preset => open_picker(app, PickerTarget::ProbePreset),
            ProbeField::Chart => {
                if let Some(probe) = app.settings.probes.get_mut(probe) {
                    probe.show_on_chart = !probe.show_on_chart;
                    app.commit_settings();
                }
            }
            ProbeField::Color => {
                if let Some(probe) = app.settings.probes.get_mut(probe) {
                    probe.color = probe.color.next();
                    app.commit_settings();
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn adjust_probe_field(app: &mut App, probe: usize, field: ProbeField, direction: i32, shift: bool) {
    let unit = app.unit();
    let step = if shift { 5.0 } else { 1.0 };
    let delta = f64::from(direction) * step;
    let Some(cfg) = app.settings.probes.get_mut(probe) else {
        return;
    };
    let mut custom = false;
    match field {
        ProbeField::Target => {
            let current = cfg.target_c.map_or(0.0, |c| f64::from(unit.from_c(c)));
            cfg.target_c = Some(unit.to_c((current + delta) as f32));
            custom = true;
        }
        ProbeField::PreAlert => {
            let current = cfg
                .pre_alert_c
                .map_or(0.0, |c| f64::from(unit.delta_from_c(c)));
            cfg.pre_alert_c = Some(unit.delta_to_c((current + delta) as f32));
            custom = true;
        }
        ProbeField::LowAlert => {
            let current = cfg.alert_low_c.map_or(0.0, |c| f64::from(unit.from_c(c)));
            cfg.alert_low_c = Some(unit.to_c((current + delta) as f32));
            custom = true;
        }
        ProbeField::HighAlert => {
            let current = cfg.alert_high_c.map_or(0.0, |c| f64::from(unit.from_c(c)));
            cfg.alert_high_c = Some(unit.to_c((current + delta) as f32));
            custom = true;
        }
        ProbeField::RangeLow | ProbeField::RangeHigh => {
            let mut range = cfg.range_c.unwrap_or([100.0, 130.0]);
            let end = usize::from(field == ProbeField::RangeHigh);
            let value = f64::from(unit.from_c(range[end])) + delta;
            range[end] = unit.to_c(value as f32);
            if range[0] > range[1] {
                range.swap(0, 1);
            }
            cfg.range_c = Some(range);
            custom = true;
        }
        ProbeField::Color => cfg.color = cfg.color.next(),
        ProbeField::Chart => cfg.show_on_chart = !cfg.show_on_chart,
        _ => {}
    }
    if custom {
        app.settings.mark_custom(probe);
    }
    app.commit_settings();
}

// ---- preset form ----------------------------------------------------------

fn new_preset(app: &mut App) {
    let base = app
        .settings
        .presets
        .get(app.settings_ui.preset_index)
        .cloned()
        .unwrap_or_default();
    let id = app
        .settings
        .unique_preset_id(&Preset::slugify(&base.name), None);
    let index = app.settings.presets.len();
    app.settings.presets.push(Preset {
        id,
        name: base.name.clone(),
        ..base
    });
    app.settings_ui.preset_index = index;
    app.settings_ui.preset_form = Some(PresetForm {
        index,
        field: 0,
        new: true,
    });
    app.settings_ui.edit_buffer = app.settings.presets[index].name.clone();
    app.settings_ui.editing = true;
    app.settings_ui.edit_target = EditTarget::PresetName;
    app.commit_settings();
}

fn handle_preset_form(app: &mut App, key: KeyEvent) {
    let Some(form) = app.settings_ui.preset_form else {
        return;
    };
    let index = form.index;
    let Some(preset) = app.settings.presets.get(index) else {
        app.settings_ui.preset_form = None;
        return;
    };
    let pit = preset.pit;
    let list = preset_form_fields(pit);
    let len = list.len();
    match key.code {
        KeyCode::Esc => app.settings_ui.preset_form = None,
        KeyCode::Up => {
            if let Some(form) = app.settings_ui.preset_form.as_mut() {
                form.field = form.field.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(form) = app.settings_ui.preset_form.as_mut() {
                form.field = (form.field + 1).min(len - 1);
            }
        }
        KeyCode::Left => adjust_preset_field(app, index, list[form.field], -1, false),
        KeyCode::Right => adjust_preset_field(app, index, list[form.field], 1, false),
        KeyCode::Char('+') | KeyCode::Char('=') => {
            adjust_preset_field(app, index, list[form.field], 1, true);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            adjust_preset_field(app, index, list[form.field], -1, true);
        }
        KeyCode::Char(' ') | KeyCode::Enter => match list[form.field] {
            PresetField::Name => begin_edit(app, EditTarget::PresetName),
            PresetField::Type => toggle_preset_type(app, index),
            PresetField::Delete => app.settings_ui.confirm_delete = Some(index),
            _ => {}
        },
        _ => {}
    }
}

fn toggle_preset_type(app: &mut App, index: usize) {
    if let Some(preset) = app.settings.presets.get_mut(index) {
        preset.pit = !preset.pit;
    }
    app.settings.sync_preset_followers(index);
    app.commit_settings();
}

fn adjust_preset_field(
    app: &mut App,
    index: usize,
    field: PresetField,
    direction: i32,
    shift: bool,
) {
    let unit = app.unit();
    let step = if shift { 5.0 } else { 1.0 };
    let delta = f64::from(direction) * step;
    let Some(preset) = app.settings.presets.get_mut(index) else {
        return;
    };
    let step_f = |value: Option<f32>, start: f32| -> f32 {
        let current = preset_display(unit, value.unwrap_or(start));
        preset_store(unit, (current as f64 + delta).clamp(5.0, 320.0) as f32)
    };
    match field {
        PresetField::Target => preset.target_f = Some(step_f(preset.target_f, 200.0)),
        PresetField::RangeLow => {
            let mut range = preset.range_f.unwrap_or([225.0, 275.0]);
            range[0] = step_f(Some(range[0]), 225.0);
            if range[0] > range[1] {
                range.swap(0, 1);
            }
            preset.range_f = Some(range);
        }
        PresetField::RangeHigh => {
            let mut range = preset.range_f.unwrap_or([225.0, 275.0]);
            range[1] = step_f(Some(range[1]), 275.0);
            if range[0] > range[1] {
                range.swap(0, 1);
            }
            preset.range_f = Some(range);
        }
        PresetField::LowAlert => preset.alert_low_f = Some(step_f(preset.alert_low_f, 215.0)),
        PresetField::HighAlert => preset.alert_high_f = Some(step_f(preset.alert_high_f, 290.0)),
        _ => return,
    }
    app.settings.sync_preset_followers(index);
    app.commit_settings();
}

/// Rename the preset at `index`, keeping its id and followers in step.
fn rename_preset(app: &mut App, text: String) {
    let name = text.trim().to_string();
    if name.is_empty() {
        return;
    }
    let Some(index) = app.settings_ui.preset_form.map(|form| form.index) else {
        return;
    };
    let Some(preset) = app.settings.presets.get(index) else {
        return;
    };
    let old_id = preset.id.clone();
    let new_id = app
        .settings
        .unique_preset_id(&Preset::slugify(&name), Some(index));
    let Some(preset) = app.settings.presets.get_mut(index) else {
        return;
    };
    preset.name = name;
    if new_id != old_id {
        preset.id.clone_from(&new_id);
        for probe in &mut app.settings.probes {
            if probe.preset == old_id {
                probe.preset.clone_from(&new_id);
            }
        }
    }
    if let Some(form) = app.settings_ui.preset_form.as_mut() {
        form.new = false;
    }
    app.commit_settings();
}

// ---- picker ---------------------------------------------------------------

fn open_picker(app: &mut App, target: PickerTarget) {
    let (items, current) = match target {
        PickerTarget::ProbePreset => {
            let mut items: Vec<String> = app
                .settings
                .presets
                .iter()
                .map(|preset| preset.name.clone())
                .collect();
            items.push(String::from(CUSTOM_PRESET_NAME));
            let current = app.settings_ui.probe_form.map_or(0, |form| {
                let id = app
                    .settings
                    .probes
                    .get(form.probe)
                    .map_or_else(String::new, |probe| probe.preset.clone());
                app.settings
                    .presets
                    .iter()
                    .position(|preset| preset.id == id)
                    .unwrap_or(items.len() - 1)
            });
            (items, current)
        }
        PickerTarget::Voice => {
            let items = app.voices.clone();
            let current = items
                .iter()
                .position(|voice| voice == &app.settings.speech.voice)
                .unwrap_or(0);
            (items, current)
        }
        PickerTarget::AudioDevice => {
            let items = app.audio_devices.clone();
            let current = items
                .iter()
                .position(|device| device == &app.settings.speech.audio_device)
                .unwrap_or(0);
            (items, current)
        }
    };
    app.settings_ui.picker = Some(Picker {
        target,
        index: current.min(items.len().saturating_sub(1)),
        items,
    });
}

fn handle_picker(app: &mut App, key: KeyEvent) {
    let Some(picker) = app.settings_ui.picker.as_ref() else {
        return;
    };
    let len = picker.items.len();
    let target = picker.target;
    match key.code {
        KeyCode::Esc => app.settings_ui.picker = None,
        KeyCode::Up => {
            if let Some(picker) = app.settings_ui.picker.as_mut() {
                picker.index = picker.index.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(picker) = app.settings_ui.picker.as_mut() {
                picker.index = (picker.index + 1).min(len.saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            let Some(picker) = app.settings_ui.picker.take() else {
                return;
            };
            apply_picker(app, target, &picker.items, picker.index);
        }
        _ => {}
    }
}

fn apply_picker(app: &mut App, target: PickerTarget, items: &[String], index: usize) {
    match target {
        PickerTarget::ProbePreset => {
            let Some(form) = app.settings_ui.probe_form else {
                return;
            };
            let id = if index < app.settings.presets.len() {
                app.settings.presets[index].id.clone()
            } else {
                String::from(CUSTOM_PRESET_ID)
            };
            app.settings.apply_preset(form.probe, &id);
        }
        PickerTarget::Voice => {
            app.settings.speech.voice = if index == 0 {
                String::new()
            } else {
                items.get(index).cloned().unwrap_or_default()
            };
        }
        PickerTarget::AudioDevice => {
            app.settings.speech.audio_device = if index == 0 {
                String::new()
            } else {
                items.get(index).cloned().unwrap_or_default()
            };
        }
    }
    app.commit_settings();
}

fn handle_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y' | 'Y') => {
            if let Some(index) = app.settings_ui.confirm_delete.take() {
                app.settings.remove_preset(index);
                let len = app.settings.presets.len();
                app.settings_ui.preset_index = index.min(len.saturating_sub(1));
                app.settings_ui.preset_form = None;
                app.commit_settings();
            }
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => app.settings_ui.confirm_delete = None,
        _ => {}
    }
}

/// Labels of an option section's fields, for the visibility test.
#[cfg(test)]
pub(crate) fn field_labels(section: usize) -> Vec<&'static str> {
    fields(section).iter().map(|field| field.label).collect()
}
