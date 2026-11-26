//! Key handling for the settings tab: browsing, text editing, the probe and
//! preset forms, the picker and the delete confirmation.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::form::*;
use super::{EditTarget, Picker, PickerTarget, PresetForm, ProbeForm, SECTIONS};
use crate::app::App;
use crate::config::{CUSTOM_PRESET_ID, CUSTOM_PRESET_NAME, Preset, parse_hhmm};
use crate::device::DeviceCommand;
use crate::model::PROBE_COUNT;

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
