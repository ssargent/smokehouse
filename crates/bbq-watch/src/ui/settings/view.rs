//! Rendering for the settings tab: sidebar, pane, forms and overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Cell, Clear, List, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState,
};

use super::form::*;
use super::{OVERLAY_WIDTH, SECTION_TITLES, SIDEBAR_WIDTH};
use crate::app::App;
use crate::config::{Preset, Settings};
use crate::model::{PROBE_COUNT, Unit};
use crate::ui::theme;

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
