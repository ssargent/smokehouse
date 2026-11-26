//! Form model for the settings tab: the field list, its kinds and per-field
//! help text, plus the probe and preset form field sets.

use crate::app::App;
use crate::config::{ChartWindow, DeviceScreenUnits, EtaMethod, Settings};
use crate::model::{Unit, celsius_to_fahrenheit, fahrenheit_to_celsius};

/// Kind of a settings field.
pub(super) enum Kind {
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
pub(super) enum Action {
    TestAlarm,
    PreviewVoice,
    Scan,
    Forget,
}

/// The displayed value of a field.
pub(super) enum Value {
    Bool(bool),
    Text(String),
    Index(usize),
}

pub(super) type GetFn = fn(&Settings) -> Value;
pub(super) type StepFn = fn(&mut Settings, i32);
pub(super) type ToggleFn = fn(&mut Settings);
pub(super) type TextFn = fn(&mut Settings, &str);

pub(super) struct Field {
    pub(super) label: &'static str,
    pub(super) help: &'static str,
    pub(super) kind: Kind,
    pub(super) get: GetFn,
    pub(super) step: StepFn,
    pub(super) toggle: ToggleFn,
    pub(super) set_text: TextFn,
}

fn noop_step(_settings: &mut Settings, _dir: i32) {}
fn noop_toggle(_settings: &mut Settings) {}
fn noop_text(_settings: &mut Settings, _value: &str) {}

pub(super) fn toggle(
    label: &'static str,
    help: &'static str,
    get: GetFn,
    toggle: ToggleFn,
) -> Field {
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

pub(super) fn spinner(
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

pub(super) fn segmented(
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

pub(super) fn text(label: &'static str, help: &'static str, get: GetFn, set_text: TextFn) -> Field {
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

pub(super) fn time(label: &'static str, help: &'static str, get: GetFn, set_text: TextFn) -> Field {
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

pub(super) fn choice(label: &'static str, help: &'static str, kind: Kind, get: GetFn) -> Field {
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

pub(super) fn button(label: &'static str, help: &'static str, action: Action) -> Field {
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

pub(super) fn clamp_step(value: f64, dir: i32, min: f64, max: f64, step: f64) -> f64 {
    (value + f64::from(dir) * step).clamp(min, max)
}

pub(super) fn step_index(index: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as i32;
    let next = (index as i32 + dir).rem_euclid(len);
    next as usize
}

/// Fields for an option section (0 is Probes, 1 is Presets).
pub(super) fn fields(section: usize) -> Vec<Field> {
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
            |s| {
                if s.speech.status_every_min == 0 {
                    Value::Text(String::from("off"))
                } else {
                    Value::Text(s.speech.status_every_min.to_string())
                }
            },
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
            &["Whole cook", "6h", "2h", "30m"],
            |s| Value::Index(s.display.chart_window as usize),
            |s, d| {
                let index = step_index(s.display.chart_window as usize, ChartWindow::ALL.len(), d);
                s.display.chart_window = ChartWindow::ALL[index];
            },
        ),
        segmented(
            "Table interval",
            "Minutes between rows in the table.",
            &["1m", "5m", "15m"],
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
/// A stored °F preset temperature shown in the current unit.
pub(super) fn preset_display(unit: Unit, fahrenheit: f32) -> f32 {
    match unit {
        Unit::Fahrenheit => fahrenheit,
        Unit::Celsius => fahrenheit_to_celsius(fahrenheit),
    }
}

/// A value entered in the current unit, stored back as °F.
pub(super) fn preset_store(unit: Unit, value: f32) -> f32 {
    match unit {
        Unit::Fahrenheit => value,
        Unit::Celsius => celsius_to_fahrenheit(value),
    }
}
/// The fields of the probe form, depending on whether the probe is a pit probe.
pub(super) fn probe_form_fields(app: &App, probe: usize) -> Vec<ProbeField> {
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
pub(super) enum ProbeField {
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PresetField {
    Name,
    Type,
    Target,
    RangeLow,
    RangeHigh,
    LowAlert,
    HighAlert,
    Delete,
}

pub(super) fn preset_form_fields(pit: bool) -> Vec<PresetField> {
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

/// Labels of an option section's fields, for the visibility test.
#[cfg(test)]
pub(crate) fn field_labels(section: usize) -> Vec<&'static str> {
    fields(section).iter().map(|field| field.label).collect()
}
