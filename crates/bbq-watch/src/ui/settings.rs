//! Settings tab: an editable probes table plus a grid of option sections.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListState, Paragraph, Row, Table, TableState,
};

use super::theme;
use crate::app::App;
use crate::config::{ChartWindow, DeviceScreenUnits, EtaMethod, Preset, Settings, parse_hhmm};
use crate::device::DeviceCommand;
use crate::model::{PROBE_COUNT, Unit};

/// Number of focusable sections: probes + five option groups.
const SECTIONS: usize = 6;

/// Columns of the probes table.
const PROBE_COLUMNS: usize = 9;

/// Column of the probes table that holds the label (also the initial column).
const LABEL_COLUMN: usize = 1;

/// Rows of the preset editor popup.
const PRESET_ROWS: usize = 7;

/// Width of the preset popups.
const PRESET_POPUP_WIDTH: u16 = 76;

/// Focus and edit state for the Settings tab.
#[derive(Debug)]
pub(crate) struct SettingsUi {
    pub(crate) section: usize,
    pub(crate) field: usize,
    pub(crate) probe_row: usize,
    pub(crate) probe_col: usize,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    pub(crate) preset_popup: bool,
    pub(crate) preset_index: usize,
    /// Whether the preset popup shows the editor instead of the list.
    preset_edit: bool,
    /// Focused row of the preset editor.
    preset_row: usize,
    pub(crate) dirty: bool,
}

impl Default for SettingsUi {
    fn default() -> Self {
        Self {
            section: 0,
            field: 0,
            probe_row: 0,
            probe_col: LABEL_COLUMN,
            editing: false,
            edit_buffer: String::new(),
            preset_popup: false,
            preset_index: 0,
            preset_edit: false,
            preset_row: 0,
            dirty: false,
        }
    }
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
    kind: Kind,
    get: GetFn,
    step: StepFn,
    toggle: ToggleFn,
    set_text: TextFn,
}

fn noop_step(_settings: &mut Settings, _dir: i32) {}
fn noop_toggle(_settings: &mut Settings) {}
fn noop_text(_settings: &mut Settings, _value: &str) {}

fn toggle(label: &'static str, get: GetFn, toggle: ToggleFn) -> Field {
    Field {
        label,
        kind: Kind::Toggle,
        get,
        step: noop_step,
        toggle,
        set_text: noop_text,
    }
}

fn spinner(label: &'static str, unit: &'static str, get: GetFn, step_fn: StepFn) -> Field {
    Field {
        label,
        kind: Kind::Spinner { unit },
        get,
        step: step_fn,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn segmented(
    label: &'static str,
    options: &'static [&'static str],
    get: GetFn,
    step: StepFn,
) -> Field {
    Field {
        label,
        kind: Kind::Segmented { options },
        get,
        step,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn text(label: &'static str, get: GetFn, set_text: TextFn) -> Field {
    Field {
        label,
        kind: Kind::Text,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text,
    }
}

fn time(label: &'static str, get: GetFn, set_text: TextFn) -> Field {
    Field {
        label,
        kind: Kind::Time,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text,
    }
}

fn choice(label: &'static str, kind: Kind, get: GetFn) -> Field {
    Field {
        label,
        kind,
        get,
        step: noop_step,
        toggle: noop_toggle,
        set_text: noop_text,
    }
}

fn button(label: &'static str, action: Action) -> Field {
    Field {
        label,
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

fn fields(section: usize) -> Vec<Field> {
    match section {
        1 => alarms_fields(),
        2 => speech_fields(),
        3 => cook_fields(),
        4 => display_fields(),
        5 => device_fields(),
        _ => Vec::new(),
    }
}

fn alarms_fields() -> Vec<Field> {
    vec![
        toggle(
            "Sound on this Mac",
            |s| Value::Bool(s.alarms.sound),
            |s| s.alarms.sound = !s.alarms.sound,
        ),
        text(
            "Sound file",
            |s| Value::Text(s.alarms.sound_file.clone()),
            |s, v| s.alarms.sound_file = v.to_string(),
        ),
        toggle(
            "macOS banner",
            |s| Value::Bool(s.alarms.banner),
            |s| s.alarms.banner = !s.alarms.banner,
        ),
        toggle(
            "Push via ntfy",
            |s| Value::Bool(s.alarms.ntfy),
            |s| s.alarms.ntfy = !s.alarms.ntfy,
        ),
        text(
            "ntfy topic",
            |s| Value::Text(s.alarms.ntfy_topic.clone()),
            |s, v| s.alarms.ntfy_topic = v.to_string(),
        ),
        toggle(
            "Terminal bell",
            |s| Value::Bool(s.alarms.bell),
            |s| s.alarms.bell = !s.alarms.bell,
        ),
        toggle(
            "Alert on stall",
            |s| Value::Bool(s.alarms.alert_on_stall),
            |s| s.alarms.alert_on_stall = !s.alarms.alert_on_stall,
        ),
        spinner(
            "Repeat every",
            "s",
            |s| Value::Text(s.alarms.repeat_s.to_string()),
            |s, d| {
                s.alarms.repeat_s =
                    clamp_step(f64::from(s.alarms.repeat_s), d, 15.0, 600.0, 15.0) as u32
            },
        ),
        spinner(
            "Snooze",
            "min",
            |s| Value::Text(s.alarms.snooze_min.to_string()),
            |s, d| {
                s.alarms.snooze_min =
                    clamp_step(f64::from(s.alarms.snooze_min), d, 1.0, 60.0, 1.0) as u32
            },
        ),
        spinner(
            "Probe unplugged after",
            "s",
            |s| Value::Text(s.alarms.unplugged_after_s.to_string()),
            |s, d| {
                s.alarms.unplugged_after_s =
                    clamp_step(f64::from(s.alarms.unplugged_after_s), d, 5.0, 300.0, 5.0) as u32;
            },
        ),
        spinner(
            "Bluetooth lost after",
            "min",
            |s| Value::Text(s.alarms.bt_lost_after_min.to_string()),
            |s, d| {
                s.alarms.bt_lost_after_min =
                    clamp_step(f64::from(s.alarms.bt_lost_after_min), d, 1.0, 30.0, 1.0) as u32;
            },
        ),
        button("Test alarm", Action::TestAlarm),
    ]
}

fn speech_fields() -> Vec<Field> {
    vec![
        toggle(
            "Spoken alerts",
            |s| Value::Bool(s.speech.enabled),
            |s| s.speech.enabled = !s.speech.enabled,
        ),
        choice("Voice", Kind::Voice, |s| {
            if s.speech.voice.is_empty() {
                Value::Text(String::from("System default"))
            } else {
                Value::Text(s.speech.voice.clone())
            }
        }),
        spinner(
            "Rate",
            "wpm",
            |s| Value::Text(s.speech.rate_wpm.to_string()),
            |s, d| {
                s.speech.rate_wpm =
                    clamp_step(f64::from(s.speech.rate_wpm), d, 90.0, 360.0, 10.0) as u32
            },
        ),
        choice("Audio output", Kind::AudioDevice, |s| {
            if s.speech.audio_device.is_empty() {
                Value::Text(String::from("Default"))
            } else {
                Value::Text(s.speech.audio_device.clone())
            }
        }),
        toggle(
            "Urgent only",
            |s| Value::Bool(s.speech.urgent_only),
            |s| s.speech.urgent_only = !s.speech.urgent_only,
        ),
        text(
            "Quiet hours",
            |s| Value::Text(s.speech.quiet_hours.clone()),
            |s, v| s.speech.quiet_hours = v.to_string(),
        ),
        spinner(
            "Spoken status every",
            "min",
            |s| Value::Text(s.speech.status_every_min.to_string()),
            |s, d| {
                s.speech.status_every_min =
                    clamp_step(f64::from(s.speech.status_every_min), d, 0.0, 60.0, 5.0) as u32;
            },
        ),
        button("Preview voice", Action::PreviewVoice),
    ]
}

fn cook_fields() -> Vec<Field> {
    vec![
        segmented(
            "ETA method",
            &["stall_aware", "linear"],
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
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.rate_window_min)),
            |s, d| s.cook.rate_window_min = clamp_step(s.cook.rate_window_min, d, 10.0, 120.0, 5.0),
        ),
        spinner(
            "Stall rate",
            "°F/h",
            |s| Value::Text(format!("{:.1}", s.cook.stall_rate_f_per_h)),
            |s, d| {
                s.cook.stall_rate_f_per_h =
                    clamp_step(f64::from(s.cook.stall_rate_f_per_h), d, 1.0, 6.0, 0.5) as f32;
            },
        ),
        spinner(
            "Stall minutes",
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.stall_minutes)),
            |s, d| s.cook.stall_minutes = clamp_step(s.cook.stall_minutes, d, 5.0, 60.0, 5.0),
        ),
        spinner(
            "Stall min",
            "°F",
            |s| {
                Value::Text(format!(
                    "{:.0}",
                    crate::model::celsius_to_fahrenheit(s.cook.stall_min_c)
                ))
            },
            |s, d| {
                let f = f64::from(crate::model::celsius_to_fahrenheit(s.cook.stall_min_c));
                s.cook.stall_min_c =
                    crate::model::fahrenheit_to_celsius(clamp_step(f, d, 100.0, 200.0, 5.0) as f32);
            },
        ),
        spinner(
            "Stall max",
            "°F",
            |s| {
                Value::Text(format!(
                    "{:.0}",
                    crate::model::celsius_to_fahrenheit(s.cook.stall_max_c)
                ))
            },
            |s, d| {
                let f = f64::from(crate::model::celsius_to_fahrenheit(s.cook.stall_max_c));
                s.cook.stall_max_c =
                    crate::model::fahrenheit_to_celsius(clamp_step(f, d, 120.0, 220.0, 5.0) as f32);
            },
        ),
        spinner(
            "Rest time",
            "min",
            |s| Value::Text(format!("{:.0}", s.cook.rest_min)),
            |s, d| s.cook.rest_min = clamp_step(s.cook.rest_min, d, 0.0, 240.0, 15.0),
        ),
        time(
            "Want to eat at",
            |s| Value::Text(s.cook.want_to_eat_at.clone()),
            |s, v| s.cook.want_to_eat_at = v.to_string(),
        ),
        toggle(
            "Wrapped resets stall",
            |s| Value::Bool(s.cook.wrapped_resets_stall),
            |s| s.cook.wrapped_resets_stall = !s.cook.wrapped_resets_stall,
        ),
        toggle(
            "Pause ETA while lid open",
            |s| Value::Bool(s.cook.pause_eta_on_lid_open),
            |s| s.cook.pause_eta_on_lid_open = !s.cook.pause_eta_on_lid_open,
        ),
    ]
}

fn display_fields() -> Vec<Field> {
    vec![
        segmented(
            "Units",
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
            &["whole", "6h", "2h", "30m"],
            |s| Value::Index(s.display.chart_window as usize),
            |s, d| {
                let index = step_index(s.display.chart_window as usize, ChartWindow::ALL.len(), d);
                s.display.chart_window = ChartWindow::ALL[index];
            },
        ),
        segmented(
            "Table interval",
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
            "ms",
            |s| Value::Text(s.display.refresh_ms.to_string()),
            |s, d| {
                s.display.refresh_ms =
                    clamp_step(s.display.refresh_ms as f64, d, 250.0, 5000.0, 250.0) as u64;
            },
        ),
        text(
            "Theme",
            |s| Value::Text(s.display.theme.clone()),
            |s, v| s.display.theme = v.to_string(),
        ),
        toggle(
            "Show target lines",
            |s| Value::Bool(s.display.show_target_lines),
            |s| s.display.show_target_lines = !s.display.show_target_lines,
        ),
        toggle(
            "Show pit range lines",
            |s| Value::Bool(s.display.show_pit_range_lines),
            |s| s.display.show_pit_range_lines = !s.display.show_pit_range_lines,
        ),
        toggle(
            "24-hour clock",
            |s| Value::Bool(s.display.clock_24h),
            |s| s.display.clock_24h = !s.display.clock_24h,
        ),
        toggle(
            "Compact cards",
            |s| Value::Bool(s.display.compact_cards),
            |s| s.display.compact_cards = !s.display.compact_cards,
        ),
    ]
}

fn device_fields() -> Vec<Field> {
    let mut fields = vec![
        button("Scan", Action::Scan),
        button("Forget", Action::Forget),
        text(
            "Display name",
            |s| Value::Text(s.device.display_name.clone()),
            |s, v| s.device.display_name = v.to_string(),
        ),
        toggle(
            "Auto reconnect",
            |s| Value::Bool(s.device.auto_reconnect),
            |s| s.device.auto_reconnect = !s.device.auto_reconnect,
        ),
        spinner(
            "Battery check",
            "min",
            |s| Value::Text(s.device.battery_check_min.to_string()),
            |s, d| {
                s.device.battery_check_min =
                    clamp_step(f64::from(s.device.battery_check_min), d, 1.0, 30.0, 1.0) as u32;
            },
        ),
        segmented(
            "Device screen units",
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
            |s| Value::Bool(s.data.csv_enabled),
            |s| s.data.csv_enabled = !s.data.csv_enabled,
        ),
        text(
            "CSV directory",
            |s| Value::Text(s.data.csv_dir.clone()),
            |s, v| s.data.csv_dir = v.to_string(),
        ),
        segmented(
            "Sample every",
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
            |s| Value::Bool(s.data.keep_awake),
            |s| s.data.keep_awake = !s.data.keep_awake,
        ),
    ];
    fields.shrink_to_fit();
    fields
}

/// Draw the settings tab.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(11), Constraint::Min(0)]).split(area);
    draw_probes(frame, rows[0], app);
    draw_grid(frame, rows[1], app);
    if app.settings_ui.preset_popup {
        draw_preset_popup(frame, area, app);
    }
}

fn draw_probes(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let rows: Vec<Row> = (0..PROBE_COUNT)
        .map(|index| {
            let cfg = app.settings.probe(index);
            let target = cfg
                .target_c
                .map_or_else(|| String::from("—"), |c| format!("{:.0}°", unit.from_c(c)));
            let low = cfg
                .alert_low_c
                .map_or_else(|| String::from("—"), |c| format!("{:.0}°", unit.from_c(c)));
            let high = cfg
                .alert_high_c
                .map_or_else(|| String::from("—"), |c| format!("{:.0}°", unit.from_c(c)));
            let pre = cfg.pre_alert_c.map_or_else(
                || String::from("—"),
                |c| format!("{:.0}°", unit.delta_from_c(c)),
            );
            let label_editing = app.settings_ui.section == 0
                && app.settings_ui.probe_row == index
                && app.settings_ui.editing;
            let label = if label_editing {
                Cell::from(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        app.settings_ui.edit_buffer.clone(),
                        Style::default().bg(theme::FIELD_BG).fg(theme::FG),
                    ),
                    Span::styled("▏", Style::default().fg(theme::ACCENT)),
                ]))
            } else {
                Cell::from(format!(" {}", cfg.label))
            };
            Row::new(vec![
                Cell::from(format!(" {}", index + 1)),
                label,
                Cell::from(format!(" {} ", app.settings.preset_name(&cfg.preset))),
                Cell::from(format!(" {target}")),
                Cell::from(format!(" {low}")),
                Cell::from(format!(" {high}")),
                Cell::from(format!(" {pre}")),
                Cell::from(format!(" {} ", cfg.color.name()))
                    .style(Style::default().fg(theme::probe_color(cfg.color))),
                Cell::from(if cfg.show_on_chart { " [x] " } else { " [ ] " }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(20),
        Constraint::Length(22),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(6),
    ];
    let header = Row::new(
        [
            " # ", " Label ", " Preset ", " Target ", " Low ", " High ", " Pre ", " Color ",
            " Chart ",
        ]
        .into_iter()
        .map(Cell::from),
    );
    let mut block = theme::block(" Probes ");
    if app.settings_ui.dirty {
        block = block.title(Span::styled(
            " ● unsaved changes ",
            Style::default().fg(theme::YELLOW),
        ));
    }
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        );

    let highlight = if app.settings_ui.section == 0 {
        Some(app.settings_ui.probe_row)
    } else {
        None
    };
    let table_area = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    frame.render_stateful_widget(
        table,
        table_area,
        &mut TableState::default().with_selected(highlight),
    );

    let hint_text = if app.settings_ui.editing {
        " Enter save   Esc cancel   Ctrl-U clear   Backspace delete "
    } else {
        " Enter edit   p presets (n new · e edit · d delete)   ←/→ column   +/- adjust   Space chart "
    };
    let hint = Line::from(vec![Span::styled(
        hint_text,
        Style::default().fg(theme::MUTED),
    )]);
    let hint_area = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };
    frame.render_widget(Paragraph::new(hint), hint_area);
}

fn draw_grid(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Min(6),
    ])
    .split(area);
    let top =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    let middle =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);
    draw_section(frame, top[0], app, 1, " Alarms & notifications ");
    draw_section(frame, top[1], app, 3, " Cook & estimates ");
    draw_section(frame, middle[0], app, 4, " Display ");
    draw_section(frame, middle[1], app, 2, " Speech ");
    draw_section(frame, rows[2], app, 5, " Device & data ");
}

fn draw_section(frame: &mut Frame, area: Rect, app: &App, section: usize, title: &str) {
    let focused = app.settings_ui.section == section;
    let border_style = Style::default().fg(if focused {
        theme::ACCENT
    } else {
        theme::BORDER
    });
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title)
        .style(theme::base());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let list = fields(section);
    let mut lines: Vec<Line> = Vec::new();
    for (index, field) in list.iter().enumerate() {
        let selected = focused && app.settings_ui.field == index;
        lines.push(render_field(field, &app.settings, app, selected));
        if lines.len() >= inner.height as usize {
            break;
        }
    }
    if section == 5 {
        lines.push(Line::from(Span::styled(
            format!(" Config: {}", app.config_path.display()),
            Style::default().fg(theme::MUTED),
        )));
    }
    frame.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

fn render_field(field: &Field, settings: &Settings, app: &App, selected: bool) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG)
    };
    let mut spans = vec![Span::styled(format!(" {:<24}", field.label), label_style)];
    let value = (field.get)(settings);
    match &field.kind {
        Kind::Toggle => {
            let on = matches!(value, Value::Bool(true));
            spans.push(Span::styled(
                if on { "[x]" } else { "[ ]" },
                Style::default().fg(if on { theme::GREEN } else { theme::MUTED }),
            ));
        }
        Kind::Spinner { unit, .. } => {
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
            // Remove the leading label for buttons; the chip is self-describing.
            spans.remove(0);
        }
    }
    Line::from(spans)
}

/// Draw the preset manager: the list, or the editor for the selected preset.
fn draw_preset_popup(frame: &mut Frame, area: Rect, app: &App) {
    if app.settings_ui.preset_edit {
        draw_preset_editor(frame, area, app);
    } else {
        draw_preset_list(frame, area, app);
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

fn draw_preset_list(frame: &mut Frame, area: Rect, app: &App) {
    let presets = &app.settings.presets;
    let popup = centered(area, PRESET_POPUP_WIDTH, presets.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    let items: Vec<Line> = presets
        .iter()
        .map(|preset| {
            Line::from(format!(
                " {:<46}{:>12} ",
                preset.name,
                preset_target(preset)
            ))
        })
        .collect();
    let hint = Line::from(Span::styled(
        " ↑/↓ choose · Enter apply · e edit · n new · d delete · Esc close ",
        Style::default().fg(theme::MUTED),
    ));
    let list = List::new(items)
        .block(theme::block(" Presets ").title_bottom(hint))
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(app.settings_ui.preset_index));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// `203°F`, `225–275°F` for a pit preset, or `—` when the preset sets neither.
fn preset_target(preset: &Preset) -> String {
    if preset.pit {
        return preset.range_f.map_or_else(
            || String::from("—"),
            |[low, high]| format!("{low:.0}–{high:.0}°F"),
        );
    }
    preset
        .target_f
        .map_or_else(|| String::from("—"), |target| format!("{target:.0}°F"))
}

fn draw_preset_editor(frame: &mut Frame, area: Rect, app: &App) {
    let Some(preset) = app.settings.presets.get(app.settings_ui.preset_index) else {
        return;
    };
    let popup = centered(area, PRESET_POPUP_WIDTH, PRESET_ROWS as u16 + 2);
    frame.render_widget(Clear, popup);
    let hint = Line::from(Span::styled(
        " ↑/↓ field · ←/→ 1°F · +/- 5°F · Space toggle · Enter rename · Esc back ",
        Style::default().fg(theme::MUTED),
    ));
    let block = theme::block(" Edit preset ").title_bottom(hint);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let lines: Vec<Line> = (0..PRESET_ROWS)
        .map(|row| preset_editor_line(app, preset, row))
        .collect();
    frame.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

/// One row of the preset editor.
fn preset_editor_line(app: &App, preset: &Preset, row: usize) -> Line<'static> {
    let selected = app.settings_ui.preset_row == row;
    // The target applies to meat presets; the bounds below only to the pit one.
    let inactive = if row == 1 {
        preset.pit
    } else {
        row >= 3 && !preset.pit
    };
    let label_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else if inactive {
        Style::default().fg(theme::MUTED)
    } else {
        Style::default().fg(theme::FG)
    };
    let (label, spans) = match row {
        0 => ("Name", preset_name_spans(app, preset)),
        1 => ("Target", spinner_spans(preset.target_f, "°F")),
        2 => ("Pit preset", toggle_spans(preset.pit)),
        3 => (
            "Pit range low",
            spinner_spans(preset.range_f.map(|range| range[0]), "°F"),
        ),
        4 => (
            "Pit range high",
            spinner_spans(preset.range_f.map(|range| range[1]), "°F"),
        ),
        5 => ("Pit low alert", spinner_spans(preset.alert_low_f, "°F")),
        _ => ("Pit high alert", spinner_spans(preset.alert_high_f, "°F")),
    };
    let mut line = vec![Span::styled(format!(" {label:<15}"), label_style)];
    line.extend(spans);
    Line::from(line)
}

fn preset_name_spans(app: &App, preset: &Preset) -> Vec<Span<'static>> {
    let style = Style::default().bg(theme::FIELD_BG).fg(theme::FG);
    if app.settings_ui.editing {
        vec![
            Span::styled(" ", style),
            Span::styled(app.settings_ui.edit_buffer.clone(), style),
            Span::styled("▏", Style::default().fg(theme::ACCENT)),
        ]
    } else {
        vec![Span::styled(format!(" {} ", preset.name), style)]
    }
}

fn spinner_spans(value: Option<f32>, unit: &'static str) -> Vec<Span<'static>> {
    let text = value.map_or_else(|| String::from("—"), |number| format!("{number:.0}"));
    vec![
        Span::styled("◀ ", Style::default().fg(theme::MUTED)),
        Span::styled(
            format!(" {text} "),
            Style::default()
                .bg(theme::FIELD_BG)
                .fg(theme::FG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" ▶ {unit}"), Style::default().fg(theme::MUTED)),
    ]
}

fn toggle_spans(on: bool) -> Vec<Span<'static>> {
    vec![Span::styled(
        if on { " [x] " } else { " [ ] " },
        Style::default().fg(if on { theme::GREEN } else { theme::MUTED }),
    )]
}

/// Handle a key press on the Settings tab.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) {
    if app.settings_ui.preset_popup {
        handle_preset(app, key);
        return;
    }
    if app.settings_ui.editing {
        handle_edit(app, key);
        return;
    }
    match key.code {
        KeyCode::Esc => app.reload_settings(),
        KeyCode::Char('s') => app.apply_settings(),
        KeyCode::Tab => {
            app.settings_ui.section = (app.settings_ui.section + 1) % SECTIONS;
            app.settings_ui.field = 0;
        }
        KeyCode::BackTab => {
            app.settings_ui.section = (app.settings_ui.section + SECTIONS - 1) % SECTIONS;
            app.settings_ui.field = 0;
        }
        KeyCode::Up => move_field(app, -1),
        KeyCode::Down => move_field(app, 1),
        KeyCode::Left => move_or_adjust(app, -1),
        KeyCode::Right => move_or_adjust(app, 1),
        KeyCode::Char('+') | KeyCode::Char('=') => adjust_value(app, 1, KeyModifiers::SHIFT),
        KeyCode::Char('-') | KeyCode::Char('_') => adjust_value(app, -1, KeyModifiers::SHIFT),
        KeyCode::Char(' ') => handle_toggle(app),
        KeyCode::Enter => activate(app),
        KeyCode::Char('p') if app.settings_ui.section == 0 => open_preset(app),
        KeyCode::Delete if app.settings_ui.section == 0 => clear_alert_column(app),
        _ => {}
    }
}

fn move_field(app: &mut App, direction: i32) {
    if app.settings_ui.section == 0 {
        let row = app.settings_ui.probe_row as i32 + direction;
        app.settings_ui.probe_row = row.clamp(0, PROBE_COUNT as i32 - 1) as usize;
        return;
    }
    let len = fields(app.settings_ui.section).len().max(1);
    let next = app.settings_ui.field as i32 + direction;
    app.settings_ui.field = next.clamp(0, len as i32 - 1) as usize;
}

/// Left/right: choose a probe column in the probes section, else adjust a field.
fn move_or_adjust(app: &mut App, direction: i32) {
    if app.settings_ui.section == 0 {
        let column = app.settings_ui.probe_col as i32 + direction;
        app.settings_ui.probe_col = column.clamp(0, PROBE_COLUMNS as i32 - 1) as usize;
        return;
    }
    step_field(app, direction);
}

/// Adjust the focused field value by one step.
fn adjust_value(app: &mut App, direction: i32, modifiers: KeyModifiers) {
    if app.settings_ui.section == 0 {
        adjust_probe(app, direction, modifiers);
        return;
    }
    step_field(app, direction);
}

fn step_field(app: &mut App, direction: i32) {
    let list = fields(app.settings_ui.section);
    let Some(field) = list.get(app.settings_ui.field) else {
        return;
    };
    match &field.kind {
        Kind::Segmented { .. } | Kind::Spinner { .. } => {
            (field.step)(&mut app.settings, direction);
            app.settings_ui.dirty = true;
        }
        Kind::Voice => step_voice(app, direction),
        Kind::AudioDevice => step_audio_device(app, direction),
        _ => {}
    }
}

fn step_voice(app: &mut App, direction: i32) {
    let options = app.voices.clone();
    if options.is_empty() {
        return;
    }
    let index = options
        .iter()
        .position(|voice| *voice == app.settings.speech.voice)
        .unwrap_or(0);
    let next = step_index(index, options.len(), direction);
    app.settings.speech.voice = if next == 0 {
        String::new()
    } else {
        options[next].clone()
    };
    app.settings_ui.dirty = true;
}

fn step_audio_device(app: &mut App, direction: i32) {
    let options = app.audio_devices.clone();
    if options.is_empty() {
        return;
    }
    let index = options
        .iter()
        .position(|device| *device == app.settings.speech.audio_device)
        .unwrap_or(0);
    let next = step_index(index, options.len(), direction);
    app.settings.speech.audio_device = if next == 0 {
        String::new()
    } else {
        options[next].clone()
    };
    app.settings_ui.dirty = true;
}

fn adjust_probe(app: &mut App, direction: i32, modifiers: KeyModifiers) {
    let row = app.settings_ui.probe_row;
    let column = app.settings_ui.probe_col;
    let unit = app.unit();
    let step = if modifiers.contains(KeyModifiers::SHIFT) {
        5.0
    } else {
        1.0
    };
    let delta = f64::from(direction) * step;

    if column == 2 {
        let mut applied = false;
        {
            let Settings {
                probes, presets, ..
            } = &mut app.settings;
            if let Some(probe) = probes.get_mut(row) {
                let len = presets.len();
                if len > 0 {
                    let current = presets
                        .iter()
                        .position(|preset| preset.id == probe.preset)
                        .unwrap_or(0);
                    let index = step_index(current, len, direction);
                    if let Some(preset) = presets.get(index) {
                        preset.apply(probe);
                        applied = true;
                    }
                }
            }
        }
        if applied {
            app.settings_ui.dirty = true;
        }
        return;
    }

    let Some(probe) = app.settings.probes.get_mut(row) else {
        return;
    };
    let mut changed = true;
    match column {
        3..=6 => {
            let adjust = |value: &mut Option<f32>| {
                let current = value.map_or(0.0, |c| f64::from(unit.from_c(c)));
                let next = current + delta;
                *value = Some(unit.to_c(next as f32));
            };
            match column {
                3 => adjust(&mut probe.target_c),
                4 => adjust(&mut probe.alert_low_c),
                5 => adjust(&mut probe.alert_high_c),
                _ => {
                    let mut pre = probe.pre_alert_c;
                    adjust(&mut pre);
                    probe.pre_alert_c = pre;
                }
            }
        }
        7 => probe.color = probe.color.next(),
        8 => probe.show_on_chart = !probe.show_on_chart,
        _ => changed = false,
    }
    if changed {
        app.settings_ui.dirty = true;
    }
}

fn handle_toggle(app: &mut App) {
    if app.settings_ui.section == 0 {
        if app.settings_ui.probe_col == 8
            && let Some(probe) = app.settings.probes.get_mut(app.settings_ui.probe_row)
        {
            probe.show_on_chart = !probe.show_on_chart;
            app.settings_ui.dirty = true;
        }
        return;
    }
    let list = fields(app.settings_ui.section);
    if let Some(field) = list.get(app.settings_ui.field)
        && matches!(field.kind, Kind::Toggle)
    {
        (field.toggle)(&mut app.settings);
        app.settings_ui.dirty = true;
    }
}

fn activate(app: &mut App) {
    if app.settings_ui.section == 0 {
        match app.settings_ui.probe_col {
            0 | LABEL_COLUMN => begin_edit(app),
            2 => open_preset(app),
            7 => {
                if let Some(probe) = app.settings.probes.get_mut(app.settings_ui.probe_row) {
                    probe.color = probe.color.next();
                    app.settings_ui.dirty = true;
                }
            }
            8 => handle_toggle(app),
            _ => {}
        }
        return;
    }
    let list = fields(app.settings_ui.section);
    let Some(field) = list.get(app.settings_ui.field) else {
        return;
    };
    match field.kind {
        Kind::Text | Kind::Time | Kind::Voice | Kind::AudioDevice => begin_edit(app),
        Kind::Toggle => handle_toggle(app),
        Kind::Button(action) => run_action(app, action),
        _ => {}
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

fn begin_edit(app: &mut App) {
    let buffer = if app.settings_ui.section == 0 {
        let index = app.settings_ui.probe_row;
        app.settings
            .probes
            .get(index)
            .map_or_else(String::new, |probe| probe.label.clone())
    } else {
        let list = fields(app.settings_ui.section);
        list.get(app.settings_ui.field)
            .map_or_else(String::new, |field| match (field.get)(&app.settings) {
                Value::Text(text) => text,
                _ => String::new(),
            })
    };
    app.settings_ui.editing = true;
    app.settings_ui.edit_buffer = buffer;
}

fn handle_edit(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.settings_ui.editing = false;
            app.settings_ui.edit_buffer.clear();
        }
        KeyCode::Enter => commit_edit(app),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.settings_ui.edit_buffer.clear();
        }
        KeyCode::Backspace => {
            app.settings_ui.edit_buffer.pop();
        }
        KeyCode::Char(character) => {
            let limit = if app.settings_ui.preset_popup {
                40
            } else if app.settings_ui.section == 0 {
                18
            } else {
                64
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
    app.settings_ui.editing = false;
    if app.settings_ui.preset_popup {
        rename_preset(app, text);
        return;
    }
    if app.settings_ui.section == 0 {
        if matches!(app.settings_ui.probe_col, 0 | LABEL_COLUMN)
            && let Some(probe) = app.settings.probes.get_mut(app.settings_ui.probe_row)
        {
            probe.label = text;
            app.settings_ui.dirty = true;
        }
        return;
    }
    let list = fields(app.settings_ui.section);
    if let Some(field) = list.get(app.settings_ui.field) {
        if let Kind::Time = field.kind
            && !text.trim().is_empty()
            && parse_hhmm(&text).is_none()
        {
            return;
        }
        (field.set_text)(&mut app.settings, &text);
        app.settings_ui.dirty = true;
    }
}

fn open_preset(app: &mut App) {
    app.settings_ui.preset_popup = true;
    app.settings_ui.preset_edit = false;
    let current = app.settings.probe(app.settings_ui.probe_row).preset;
    app.settings_ui.preset_index = app
        .settings
        .presets
        .iter()
        .position(|preset| preset.id == current)
        .unwrap_or(0);
}

/// A key press while the preset popup is open.
fn handle_preset(app: &mut App, key: KeyEvent) {
    if app.settings_ui.editing {
        handle_edit(app, key);
    } else if app.settings_ui.preset_edit {
        handle_preset_editor(app, key);
    } else {
        handle_preset_list(app, key);
    }
}

fn handle_preset_list(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.settings_ui.preset_popup = false,
        KeyCode::Up => {
            app.settings_ui.preset_index = app.settings_ui.preset_index.saturating_sub(1);
        }
        KeyCode::Down => {
            let last = app.settings.presets.len().saturating_sub(1);
            app.settings_ui.preset_index = (app.settings_ui.preset_index + 1).min(last);
        }
        KeyCode::Enter => apply_preset(app),
        KeyCode::Char('e') => {
            app.settings_ui.preset_edit = true;
            app.settings_ui.preset_row = 0;
        }
        KeyCode::Char('n') => new_preset(app),
        KeyCode::Char('d') => delete_preset(app),
        KeyCode::Char(' ') => toggle_preset_pit(app),
        _ => {}
    }
}

/// Apply the highlighted preset to the probe the popup was opened from.
fn apply_preset(app: &mut App) {
    let row = app.settings_ui.probe_row;
    let index = app.settings_ui.preset_index;
    let applied = {
        let Settings {
            probes, presets, ..
        } = &mut app.settings;
        match (probes.get_mut(row), presets.get(index)) {
            (Some(probe), Some(preset)) => {
                preset.apply(probe);
                true
            }
            _ => false,
        }
    };
    if applied {
        app.settings_ui.dirty = true;
    }
    app.settings_ui.preset_popup = false;
}

fn handle_preset_editor(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.settings_ui.preset_edit = false,
        KeyCode::Up => app.settings_ui.preset_row = app.settings_ui.preset_row.saturating_sub(1),
        KeyCode::Down => {
            app.settings_ui.preset_row = (app.settings_ui.preset_row + 1).min(PRESET_ROWS - 1);
        }
        KeyCode::Left => adjust_preset(app, -1, key.modifiers),
        KeyCode::Right => adjust_preset(app, 1, key.modifiers),
        KeyCode::Char('+') | KeyCode::Char('=') => adjust_preset(app, 1, KeyModifiers::SHIFT),
        KeyCode::Char('-') | KeyCode::Char('_') => adjust_preset(app, -1, KeyModifiers::SHIFT),
        KeyCode::Char(' ') if app.settings_ui.preset_row == 2 => toggle_preset_pit(app),
        KeyCode::Enter if app.settings_ui.preset_row == 0 => begin_preset_name(app),
        _ => {}
    }
}

fn begin_preset_name(app: &mut App) {
    let index = app.settings_ui.preset_index;
    let Some(preset) = app.settings.presets.get(index) else {
        return;
    };
    app.settings_ui.edit_buffer = preset.name.clone();
    app.settings_ui.editing = true;
}

fn adjust_preset(app: &mut App, direction: i32, modifiers: KeyModifiers) {
    let step = if modifiers.contains(KeyModifiers::SHIFT) {
        5.0
    } else {
        1.0
    };
    let delta = f64::from(direction) * step;
    let row = app.settings_ui.preset_row;
    let index = app.settings_ui.preset_index;
    let Some(preset) = app.settings.presets.get_mut(index) else {
        return;
    };
    match row {
        1 => preset.target_f = Some(step_temperature(preset.target_f, delta, 80.0, 300.0, 200.0)),
        3 => preset.range_f = Some(step_range(preset.range_f, delta, 0)),
        4 => preset.range_f = Some(step_range(preset.range_f, delta, 1)),
        5 => {
            preset.alert_low_f = Some(step_temperature(
                preset.alert_low_f,
                delta,
                80.0,
                400.0,
                200.0,
            ));
        }
        6 => {
            preset.alert_high_f = Some(step_temperature(
                preset.alert_high_f,
                delta,
                80.0,
                500.0,
                275.0,
            ));
        }
        _ => return,
    }
    app.settings_ui.dirty = true;
}

/// Move a °F setting by `delta`, starting from `start` when it is unset.
fn step_temperature(value: Option<f32>, delta: f64, min: f32, max: f32, start: f32) -> f32 {
    (f64::from(value.unwrap_or(start)) + delta).clamp(f64::from(min), f64::from(max)) as f32
}

/// Move one end of the pit range, keeping the low end below the high end.
fn step_range(range: Option<[f32; 2]>, delta: f64, end: usize) -> [f32; 2] {
    let mut value = range.unwrap_or([225.0, 275.0]);
    let current = f64::from(value[end]);
    let (min, max) = if end == 0 {
        (80.0, f64::from(value[1]))
    } else {
        (f64::from(value[0]), 500.0)
    };
    value[end] = (current + delta).clamp(min, max) as f32;
    value
}

fn toggle_preset_pit(app: &mut App) {
    let index = app.settings_ui.preset_index;
    if let Some(preset) = app.settings.presets.get_mut(index) {
        preset.pit = !preset.pit;
        app.settings_ui.dirty = true;
    }
}

fn new_preset(app: &mut App) {
    let index = app.settings_ui.preset_index;
    let target_f = app
        .settings
        .presets
        .get(index)
        .and_then(|preset| preset.target_f);
    let id = app.settings.unique_preset_id("new_preset", None);
    let insert_at = if app.settings.presets.is_empty() {
        0
    } else {
        index + 1
    };
    app.settings.presets.insert(
        insert_at,
        Preset {
            id,
            name: String::from("New preset"),
            target_f,
            ..Preset::default()
        },
    );
    app.settings_ui.preset_index = insert_at;
    app.settings_ui.preset_edit = true;
    app.settings_ui.preset_row = 0;
    app.settings_ui.edit_buffer.clear();
    app.settings_ui.editing = true;
    app.settings_ui.dirty = true;
}

fn delete_preset(app: &mut App) {
    let index = app.settings_ui.preset_index;
    if app.settings.presets.len() <= 1 {
        return;
    }
    let removed = app.settings.presets.remove(index);
    let fallback = index.min(app.settings.presets.len() - 1);
    let replacement = app
        .settings
        .presets
        .get(fallback)
        .map(|preset| preset.id.clone());
    if let Some(replacement) = replacement {
        for probe in &mut app.settings.probes {
            if probe.preset == removed.id {
                probe.preset.clone_from(&replacement);
            }
        }
    }
    app.settings_ui.preset_index = fallback;
    app.settings_ui.preset_edit = false;
    app.settings_ui.dirty = true;
}

/// Rename the highlighted preset, keeping its id and the probe references in step.
fn rename_preset(app: &mut App, text: String) {
    let name = text.trim().to_string();
    if name.is_empty() {
        return;
    }
    let index = app.settings_ui.preset_index;
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
    app.settings_ui.dirty = true;
}

fn clear_alert_column(app: &mut App) {
    let column = app.settings_ui.probe_col;
    if !matches!(column, 3..=6) {
        return;
    }
    if let Some(probe) = app.settings.probes.get_mut(app.settings_ui.probe_row) {
        match column {
            3 => probe.target_c = None,
            4 => probe.alert_low_c = None,
            5 => probe.alert_high_c = None,
            _ => probe.pre_alert_c = None,
        }
        app.settings_ui.dirty = true;
    }
}
