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

/// Focus and edit state for the Settings tab.
#[derive(Debug, Default)]
pub(crate) struct SettingsUi {
    pub(crate) section: usize,
    pub(crate) field: usize,
    pub(crate) probe_row: usize,
    pub(crate) probe_col: usize,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    pub(crate) preset_popup: bool,
    pub(crate) preset_index: usize,
    pub(crate) dirty: bool,
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
            Row::new(vec![
                Cell::from(format!(" {}", index + 1)),
                Cell::from(format!(" {}", cfg.label)),
                Cell::from(format!(" {} ", cfg.preset.label())),
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

    let hint = Line::from(vec![Span::styled(
        " Enter edit label   p choose preset   ←/→ column   +/- adjust (1°, Shift 5°)   Space chart   Labels are saved per-probe ",
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

fn draw_preset_popup(frame: &mut Frame, area: Rect, app: &App) {
    let width = 40.min(area.width);
    let height = (Preset::ALL.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let items: Vec<Line> = Preset::ALL
        .iter()
        .map(|preset| {
            let target = preset.target_c().map_or_else(
                || String::from("—"),
                |c| format!("{:.0}°F", crate::model::celsius_to_fahrenheit(c)),
            );
            Line::from(format!(" {}  {}", preset.label(), target))
        })
        .collect();
    let list = List::new(items)
        .block(theme::block(" Choose preset "))
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(app.settings_ui.preset_index));
    frame.render_stateful_widget(list, popup, &mut state);
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
    let mut changed = true;
    {
        let Some(probe) = app.settings.probes.get_mut(row) else {
            return;
        };
        match column {
            2 => {
                let index = step_index(
                    Preset::ALL
                        .iter()
                        .position(|preset| *preset == probe.preset)
                        .unwrap_or(0),
                    Preset::ALL.len(),
                    direction,
                );
                Preset::ALL[index].apply(probe);
            }
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
            1 => begin_edit(app, true),
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
        Kind::Text | Kind::Time | Kind::Voice | Kind::AudioDevice => begin_edit(app, false),
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

fn begin_edit(app: &mut App, label_edit: bool) {
    let buffer = if app.settings_ui.section == 0 {
        let index = app.settings_ui.probe_row;
        app.settings
            .probes
            .get(index)
            .map(|probe| probe.label.clone())
            .unwrap_or_default()
    } else {
        let list = fields(app.settings_ui.section);
        list.get(app.settings_ui.field)
            .map_or_else(String::new, |field| match (field.get)(&app.settings) {
                Value::Text(text) => text,
                _ => String::new(),
            })
    };
    let _ = label_edit;
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
        KeyCode::Backspace => {
            app.settings_ui.edit_buffer.pop();
        }
        KeyCode::Char(character) => {
            let limit = if app.settings_ui.section == 0 { 18 } else { 64 };
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
    if app.settings_ui.section == 0 {
        if app.settings_ui.probe_col == 1
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
    let current = app
        .settings
        .probes
        .get(app.settings_ui.probe_row)
        .map(|probe| probe.preset);
    app.settings_ui.preset_index = current
        .and_then(|preset| {
            Preset::ALL
                .iter()
                .position(|candidate| *candidate == preset)
        })
        .unwrap_or(0);
}

fn handle_preset(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.settings_ui.preset_popup = false,
        KeyCode::Up => {
            app.settings_ui.preset_index = app.settings_ui.preset_index.saturating_sub(1);
        }
        KeyCode::Down => {
            app.settings_ui.preset_index =
                (app.settings_ui.preset_index + 1).min(Preset::ALL.len() - 1);
        }
        KeyCode::Enter => {
            let preset = Preset::ALL[app.settings_ui.preset_index];
            if let Some(probe) = app.settings.probes.get_mut(app.settings_ui.probe_row) {
                preset.apply(probe);
                app.settings_ui.dirty = true;
            }
            app.settings_ui.preset_popup = false;
        }
        _ => {}
    }
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
