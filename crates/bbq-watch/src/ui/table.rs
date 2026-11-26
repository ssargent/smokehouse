//! Table tab: interval-sampled readings with deltas, plus per-probe sparklines.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{
    Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Sparkline, Table, TableState,
};

use super::theme;
use crate::app::App;
use crate::clock::AppInstant;
use crate::model::PROBE_COUNT;

/// Draw the table tab.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(8)]).split(area);
    draw_table(frame, rows[0], app);
    draw_sparklines(frame, rows[1], app);
}

fn draw_table(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let interval = f64::from(app.settings.display.table_interval_min.max(1));
    let step = interval * 60.0;
    let count = (app.now.secs() / step).floor() as i64;

    let mut table_rows: Vec<Row> = Vec::new();
    let mut last = count;
    while last >= 0 {
        let time = AppInstant::from_secs_f64(last as f64 * step);
        let previous = AppInstant::from_secs_f64((last as f64 * step - step).max(0.0));
        let mut cells: Vec<Cell> = Vec::new();
        cells.push(Cell::from(format!(" {} ", app.fmt_time(time))));
        let mut deltas: Vec<Cell> = Vec::new();
        for probe in 0..PROBE_COUNT {
            let cfg = app.settings.probe(probe);
            let color = theme::probe_color(cfg.color);
            let current = app.probes[probe].history.value_near(time);
            cells.push(
                Cell::from(current.map_or_else(
                    || String::from("      —"),
                    |c| format!("{:>7.1}", unit.from_c(c)),
                ))
                .style(Style::default().fg(color)),
            );
            let change = current
                .zip(app.probes[probe].history.value_near(previous))
                .map(|(a, b)| unit.delta_from_c(a - b));
            deltas.push(delta_cell(change, unit));
        }
        cells.extend(deltas);
        let stalled = (0..PROBE_COUNT).any(|probe| {
            !app.is_pit(probe)
                && app.probes[probe]
                    .stall
                    .started_at()
                    .is_some_and(|started| started <= time && time.secs() - started.secs() < 3600.0)
        });
        cells.push(Cell::from(if stalled {
            Span::styled("stall", Style::default().fg(theme::MUTED))
        } else {
            Span::raw("")
        }));
        table_rows.push(Row::new(cells));
        last -= 1;
    }

    let total = table_rows.len().max(1);
    let scroll = app.table_scroll.min(total.saturating_sub(1));
    let mut state = TableState::default().with_selected(Some(scroll));

    let mut header = vec![" Time ".to_string()];
    for probe in 0..PROBE_COUNT {
        header.push(format!(
            " {} {} ",
            super::tab_index_label(probe),
            app.probe_label(probe)
        ));
    }
    for probe in 0..PROBE_COUNT {
        header.push(format!(" {} Δ ", super::tab_index_label(probe)));
    }
    header.push(String::from(" Status "));

    let widths: Vec<Constraint> = std::iter::once(Constraint::Length(7))
        .chain(std::iter::repeat_n(Constraint::Length(12), PROBE_COUNT))
        .chain(std::iter::repeat_n(Constraint::Length(7), PROBE_COUNT))
        .chain(std::iter::once(Constraint::Length(8)))
        .collect();

    let title = format!(
        " Readings · {}-min intervals · newest first · {total} rows ",
        app.settings.display.table_interval_min
    );
    let table = Table::new(table_rows, widths)
        .header(Row::new(header.into_iter().map(Cell::from)))
        .block(theme::block(&title))
        .row_highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut state);

    let mut scrollbar_state = ScrollbarState::new(total).position(scroll);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area,
        &mut scrollbar_state,
    );
}

fn delta_cell(change: Option<f32>, unit: crate::model::Unit) -> Cell<'static> {
    let _ = unit;
    match change {
        Some(value) if value > 0.3 => {
            Cell::from(format!("{:+.1}", value)).style(Style::default().fg(theme::GREEN))
        }
        Some(value) if value < -0.3 => {
            Cell::from(format!("{:+.1}", value)).style(Style::default().fg(theme::RED))
        }
        Some(value) => {
            Cell::from(format!("{:+.1}", value)).style(Style::default().fg(theme::MUTED))
        }
        None => Cell::from("  —"),
    }
}

fn draw_sparklines(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let columns = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(area);
    let low = unit.from_c(0.0);
    let high = unit.from_c(crate::model::fahrenheit_to_celsius(300.0));

    for probe in 0..PROBE_COUNT {
        let cfg = app.settings.probe(probe);
        let color = theme::probe_color(cfg.color);
        let values: Vec<u64> = app.probes[probe]
            .history
            .samples()
            .iter()
            .filter_map(|(_, temp)| *temp)
            .map(|c| {
                let value = unit.from_c(c);
                (((value - low) / (high - low)).clamp(0.0, 1.0) * 100.0) as u64
            })
            .collect();
        let (min, max, now) =
            app.probes[probe]
                .history
                .min_max()
                .map_or((0.0, 0.0, 0.0), |(min, max)| {
                    (
                        unit.from_c(min),
                        unit.from_c(max),
                        app.probes[probe]
                            .temp_c
                            .map_or(f64::from(unit.from_c(max)), |c| f64::from(unit.from_c(c))),
                    )
                });
        let title = format!(
            " {} · min {min:.0} · max {max:.0} · now {now:.0} ",
            cfg.label
        );
        let sparkline = Sparkline::default()
            .block(theme::block(&title))
            .data(&values)
            .style(Style::default().fg(color));
        frame.render_widget(sparkline, columns[probe]);
    }
}
