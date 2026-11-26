//! Top-level frame: header, tabs, alert banner, body and footer.

pub mod alarms;
pub mod dashboard;
pub mod log;
pub mod settings;
pub mod table;
pub mod theme;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Tabs};

use crate::alerts::Priority;
use crate::analysis::format_duration;
use crate::app::{App, Banner, DeviceStatus};

/// Minimum usable terminal width.
pub const MIN_WIDTH: u16 = 100;
/// Minimum usable terminal height.
pub const MIN_HEIGHT: u16 = 30;

/// The five top-level tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// Dashboard.
    Dashboard,
    /// Readings table.
    Table,
    /// Active alarms.
    Alarms,
    /// Event log.
    Log,
    /// Settings.
    Settings,
}

impl Tab {
    /// All tabs, in order.
    pub const ALL: [Self; 5] = [
        Self::Dashboard,
        Self::Table,
        Self::Alarms,
        Self::Log,
        Self::Settings,
    ];

    /// Zero-based position.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Dashboard => 0,
            Self::Table => 1,
            Self::Alarms => 2,
            Self::Log => 3,
            Self::Settings => 4,
        }
    }

    /// Tab title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Table => "Table",
            Self::Alarms => "Alarms",
            Self::Log => "Log",
            Self::Settings => "Settings",
        }
    }

    /// The tab to the right.
    #[must_use]
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// The tab to the left.
    #[must_use]
    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Draw one frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme::base()), area);

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small(frame, area);
        return;
    }

    let banner_height = u16::from(app.alerts_view.banner.is_some());
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(banner_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(frame, chunks[0], app);
    if let Some(banner) = &app.alerts_view.banner {
        draw_banner(frame, chunks[1], banner);
    }
    match app.tab {
        Tab::Dashboard => dashboard::draw(frame, chunks[2], app),
        Tab::Table => table::draw(frame, chunks[2], app),
        Tab::Alarms => alarms::draw(frame, chunks[2], app),
        Tab::Log => log::draw(frame, chunks[2], app),
        Tab::Settings => settings::draw(frame, chunks[2], app),
    }
    draw_footer(frame, chunks[3], app);

    if app.show_help {
        draw_help(frame, area);
    }

    if app.quit_prompt {
        draw_quit_prompt(frame, area);
    }
}

fn too_small(frame: &mut Frame, area: Rect) {
    let message = Paragraph::new("Enlarge terminal to at least 100×30")
        .style(theme::base())
        .alignment(Alignment::Center);
    let middle = Layout::vertical([
        Constraint::Percentage(45),
        Constraint::Length(1),
        Constraint::Percentage(45),
    ])
    .split(area);
    frame.render_widget(message, middle[1]);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = theme::block(" bbq-watch ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let columns =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);

    let count = app.alerts_view.rows.len();
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|tab| {
            let text = if *tab == Tab::Alarms && count > 0 {
                format!(" {} Alarms ({count}) ", tab.index() + 1)
            } else {
                format!(" {} {} ", tab.index() + 1, tab.title())
            };
            Line::from(text)
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.tab.index())
        .divider(Span::styled("│", Style::default().fg(theme::BORDER)))
        .highlight_style(
            Style::default()
                .bg(theme::ACCENT)
                .fg(theme::BG)
                .add_modifier(Modifier::BOLD),
        )
        .style(theme::base());
    frame.render_widget(tabs, columns[0]);

    frame.render_widget(
        Paragraph::new(header_status(app))
            .style(theme::base())
            .alignment(Alignment::Right),
        columns[1],
    );
}

fn header_status(app: &App) -> Line<'static> {
    let (dot, dot_color) = match app.device_status {
        DeviceStatus::Connected => ("●", theme::GREEN),
        DeviceStatus::Scanning => ("●", theme::YELLOW),
        DeviceStatus::Disconnected => ("●", theme::RED),
        DeviceStatus::Unavailable => ("●", theme::RED),
    };
    let battery_color = match app.battery {
        Some(pct) if pct < 20 => theme::RED,
        _ => theme::FG,
    };
    let rssi = app
        .rssi
        .map_or_else(|| String::from("— dBm"), |value| format!("{value} dBm"));
    let battery = app
        .battery
        .map_or_else(|| String::from("batt —%"), |pct| format!("batt {pct}%"));
    let wall = app.wall(app.now);
    let clock = if app.settings.display.clock_24h {
        wall.format("%H:%M:%S").to_string()
    } else {
        wall.format("%I:%M:%S %p").to_string()
    };
    let mut spans = vec![
        Span::styled(format!("{dot} "), Style::default().fg(dot_color)),
        Span::styled(app.device_label(), Style::default().fg(theme::FG)),
        Span::styled(" │ ", Style::default().fg(theme::BORDER)),
        Span::styled(rssi, Style::default().fg(theme::MUTED)),
        Span::styled(" │ ", Style::default().fg(theme::BORDER)),
        Span::styled(battery, Style::default().fg(battery_color)),
        Span::styled(" │ ", Style::default().fg(theme::BORDER)),
        Span::styled(
            format!("cook {}", format_duration(Some(app.now.secs()))),
            Style::default().fg(theme::MUTED),
        ),
        Span::styled(" │ ", Style::default().fg(theme::BORDER)),
        Span::styled(clock, Style::default().fg(theme::FG)),
        Span::raw(" "),
    ];
    spans.shrink_to_fit();
    Line::from(spans)
}

fn draw_banner(frame: &mut Frame, area: Rect, banner: &Banner) {
    let bg = match banner.priority {
        Priority::Urgent => theme::RED,
        Priority::Normal => theme::YELLOW,
    };
    let text = format!("⚠ {}  ·  x ack  z snooze ", banner.text);
    let line = Line::from(Span::styled(
        text,
        Style::default()
            .bg(bg)
            .fg(theme::BG)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let columns = Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(footer_hints(app))).style(theme::base()),
        columns[0],
    );
    let (text, style) = footer_right(app);
    frame.render_widget(
        Paragraph::new(Span::styled(text, style))
            .style(theme::base())
            .alignment(Alignment::Right),
        columns[1],
    );
}

/// Right-hand footer text: the settings-save indicator, else demo/log info.
fn footer_right(app: &App) -> (String, Style) {
    if let Some(status) = app.save_status() {
        let (mark, color) = if status.ok {
            ("✓", theme::GREEN)
        } else {
            ("✗", theme::RED)
        };
        return (
            format!("{mark} {} ", status.message),
            Style::default().fg(color),
        );
    }
    if app.demo {
        return (
            format!("demo ×{} ", app.cli.speed),
            Style::default().fg(theme::MUTED),
        );
    }
    let text = app.logger.file_name().map_or_else(
        || String::from("logging off "),
        |name| format!("logging → {name} "),
    );
    (text, Style::default().fg(theme::MUTED))
}

fn chip(key: &str) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::default()
            .bg(theme::MUTED)
            .fg(theme::BG)
            .add_modifier(Modifier::BOLD),
    )
}

fn hint(key: &str, description: &str) -> Vec<Span<'static>> {
    vec![
        chip(key),
        Span::styled(
            format!(" {description}  "),
            Style::default().fg(theme::MUTED),
        ),
    ]
}

fn footer_hints(app: &App) -> Vec<Span<'static>> {
    let mut spans = match app.tab {
        Tab::Dashboard => {
            let mut spans = hint("←/→", "tabs");
            spans.extend(hint("↑/↓", "select card"));
            spans.extend(hint("w", "wrapped"));
            spans
        }
        Tab::Table => {
            let mut spans = hint("↑/↓", "scroll");
            spans.extend(hint("i", "interval"));
            spans.extend(hint("e", "export"));
            spans
        }
        Tab::Alarms => {
            let mut spans = hint("↑/↓", "select");
            spans.extend(hint("Enter", "ack"));
            spans.extend(hint("t", "test"));
            spans
        }
        Tab::Log => hint("↑/↓", "scroll"),
        Tab::Settings => {
            let mut spans = hint("Tab", "section");
            spans.push(Span::styled(
                format!(" {}  ", settings::footer_hint(app)),
                Style::default().fg(theme::MUTED),
            ));
            spans
        }
    };
    spans.extend(hint("q", "quit"));
    spans.extend(hint("?", "help"));
    spans
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = 46.min(area.width);
    let height = 18.min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(" Help ")
        .style(theme::base());
    let entries = [
        ("q", "quit (confirm if connected)"),
        ("←/→ or 1–5", "switch tabs"),
        ("x", "acknowledge all alerts"),
        ("z", "snooze all alerts"),
        ("?", "toggle this help"),
        ("↑/↓", "select / scroll"),
        ("w", "wrapped (dashboard)"),
        ("i", "table interval"),
        ("e", "export history"),
        ("t", "test alarm (alarms)"),
        ("s / Esc", "save / discard settings"),
        ("! @ # $", "demo: unplug P1–P4"),
    ];
    let lines: Vec<Line> = entries
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled(format!(" {key:<12}"), Style::default().fg(theme::ACCENT)),
                Span::styled(format!("{description} "), Style::default().fg(theme::FG)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// Confirmation shown when `q` is pressed while a probe is connected.
fn draw_quit_prompt(frame: &mut Frame, area: Rect) {
    let width = 44.min(area.width);
    let height = 6.min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::YELLOW))
        .title(" Quit and end cook? ")
        .style(theme::base());
    let key = Style::default()
        .fg(theme::ACCENT)
        .add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("    y", key),
            Span::styled("  quit      ", Style::default().fg(theme::FG)),
            Span::styled("n", key),
            Span::styled("  cancel", Style::default().fg(theme::FG)),
        ]),
        Line::from(""),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// Label for a chart window length in minutes.
pub(crate) fn settings_window_label(_app: &App, minutes: f64) -> String {
    if (minutes - 360.0).abs() < f64::EPSILON {
        String::from("6h")
    } else if (minutes - 120.0).abs() < f64::EPSILON {
        String::from("2h")
    } else {
        String::from("30m")
    }
}

/// Label like `P1` for a zero-based probe index.
#[must_use]
pub(crate) fn tab_index_label(index: usize) -> String {
    format!("P{}", index + 1)
}
