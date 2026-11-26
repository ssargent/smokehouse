//! Log tab: the complete event log for this cook.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use super::theme;
use crate::app::{App, EventIcon};

/// Draw the log tab.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let capacity = area.height.saturating_sub(2) as usize;
    let total = app.events.len();
    let max_scroll = total.saturating_sub(capacity);
    let scroll = app.log_scroll.min(max_scroll);

    let lines: Vec<Line> = app
        .events
        .iter()
        .rev()
        .skip(scroll)
        .take(capacity)
        .map(event_line)
        .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::block(" Event log "))
            .style(theme::base()),
        area,
    );

    let mut state = ScrollbarState::new(total).position(scroll);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area,
        &mut state,
    );
}

fn event_line(event: &crate::app::Event) -> Line<'static> {
    let (icon, color) = match event.icon {
        EventIcon::Milestone => ("✓", theme::GREEN),
        EventIcon::Warning => ("⚠", theme::YELLOW),
        EventIcon::Urgent => ("✖", theme::RED),
        EventIcon::Info => ("•", theme::MUTED),
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", event.wall.format("%Y-%m-%d %H:%M:%S")),
            Style::default().fg(theme::MUTED),
        ),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(event.message.clone(), Style::default().fg(theme::FG)),
    ])
}
