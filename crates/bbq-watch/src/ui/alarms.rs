//! Alarms tab: active alerts and recently cleared alerts.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::theme;
use crate::alerts::Priority;
use crate::app::{AlertRow, App};

/// Draw the alarms tab.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(7)]).split(area);
    draw_active(frame, rows[0], app);
    draw_cleared(frame, rows[1], app);
}

fn draw_active(frame: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .alerts_view
        .rows
        .iter()
        .map(|row| alert_row(row, app))
        .collect();
    let selected = if rows.is_empty() {
        None
    } else {
        Some(app.alarms_selected.min(rows.len() - 1))
    };
    let mut state = TableState::default().with_selected(selected);
    let widths = [
        Constraint::Length(8),
        Constraint::Length(16),
        Constraint::Length(18),
        Constraint::Length(9),
        Constraint::Length(14),
        Constraint::Min(10),
    ];
    let header = Row::new(
        [
            " Priority ",
            " Probe ",
            " Alert ",
            " Since ",
            " Last spoken ",
            " Status ",
        ]
        .into_iter()
        .map(Cell::from),
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(" Active alerts "))
        .row_highlight_style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut state);
}

fn alert_row(row: &AlertRow, app: &App) -> Row<'static> {
    let (priority_text, priority_color) = match row.priority {
        Priority::Urgent => (" urgent ", theme::RED),
        Priority::Normal => (" normal ", theme::YELLOW),
    };
    let probe = row.key.0.map_or_else(
        || String::from("—"),
        |id| format!(" {} ", app.probe_label(id.index())),
    );
    let since = format!(" {} ", app.wall(row.since).format("%H:%M:%S"));
    let spoken = row.last_spoken.map_or_else(
        || String::from(" —"),
        |at| format!(" {}", app.wall(at).format("%H:%M:%S")),
    );
    let status = if row.acked {
        String::from("acked")
    } else if let Some(until) = row.snoozed_until {
        format!("snoozed until {}", app.wall(until).format("%H:%M"))
    } else {
        String::from("active")
    };
    Row::new(vec![
        Cell::from(priority_text).style(Style::default().fg(priority_color)),
        Cell::from(probe),
        Cell::from(format!(" {} ", row.key.1.label())),
        Cell::from(since),
        Cell::from(spoken),
        Cell::from(format!(" {status} ")),
    ])
}

fn draw_cleared(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = app
        .alerts_view
        .cleared
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|(key, at)| {
            let probe = key.0.map_or_else(String::new, |id| {
                format!("{} ", app.probe_label(id.index()))
            });
            Line::from(Span::styled(
                format!(
                    " {} {}{} cleared",
                    app.wall(*at).format("%H:%M"),
                    probe,
                    key.1.label()
                ),
                Style::default().fg(theme::MUTED),
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::block(" Cleared in the last hour "))
            .style(theme::base()),
        area,
    );
}
