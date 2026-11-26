//! Dashboard tab: probe cards, temperature chart, ETA panel and events panel.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, BorderType, Borders, Chart, Dataset, Gauge, GraphType, LegendPosition, Paragraph,
};

use super::{Tab, theme};
use crate::analysis::{ProbeHistory, format_duration};
use crate::app::{App, Event, EventIcon, ProbeState};
use crate::clock::AppInstant;
use crate::config::ProbeConfig;
use crate::model::{PROBE_COUNT, ProbeId, Unit};

/// One chart series: label, colour and downsampled points.
type Series = (String, Color, Vec<(f64, f64)>);

/// Draw the dashboard.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    draw_cards(frame, rows[0], app);
    let columns =
        Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)]).split(rows[1]);
    draw_chart(frame, columns[0], app);
    let right = Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(columns[1]);
    draw_eta(frame, right[0], app);
    draw_events(frame, right[1], app);
}

fn draw_cards(frame: &mut Frame, area: Rect, app: &App) {
    let columns = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(area);
    for index in 0..PROBE_COUNT {
        draw_card(frame, columns[index], app, index);
    }
}

fn draw_card(frame: &mut Frame, area: Rect, app: &App, index: usize) {
    let cfg = app.settings.probe(index);
    let color = theme::probe_color(cfg.color);
    let selected = app.selected_card == index && app.tab == Tab::Dashboard;
    let border_style = Style::default().fg(if selected {
        theme::ACCENT
    } else {
        theme::BORDER
    });
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(
            format!(" {} · {} ", ProbeId::from_index(index), cfg.label),
            Style::default().fg(color),
        ))
        .style(theme::base());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let state = &app.probes[index];
    if app.is_pit(index) {
        draw_pit_card(frame, inner, app, &cfg, state);
    } else {
        draw_meat_card(frame, inner, app, &cfg, state);
    }
}

fn draw_meat_card(frame: &mut Frame, area: Rect, app: &App, cfg: &ProbeConfig, state: &ProbeState) {
    let unit = app.unit();
    let color = theme::probe_color(cfg.color);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let (temp_text, temp_style) = match state.temp_c {
        Some(celsius) => (
            unit.format_c(celsius),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        None => state.last_seen_c.map_or_else(
            || (String::from("--"), Style::default().fg(theme::MUTED)),
            |celsius| (unit.format_c(celsius), Style::default().fg(theme::MUTED)),
        ),
    };
    lines.push(spread(
        vec![Span::raw(" "), Span::styled(temp_text, temp_style)],
        rate_spans(unit, state),
        area.width,
    ));

    if state.temp_c.is_none() {
        lines.push(Line::from(Span::styled(
            " probe not connected",
            Style::default().fg(theme::MUTED),
        )));
    } else {
        let target = cfg
            .target_c
            .map_or_else(|| String::from("—"), |value| unit.format_c(value));
        let mut badge = Vec::new();
        if state.stalled {
            badge.push(badge_span(" STALL ", theme::YELLOW));
        }
        if state.wrapped_at.is_some() {
            badge.push(badge_span(" WRAPPED ", theme::BLUE));
        }
        if state.eta.is_some_and(|eta| eta.done) {
            badge.push(badge_span(" DONE ", theme::GREEN));
        }
        lines.push(spread(
            vec![Span::styled(
                format!(" target {target}"),
                Style::default().fg(theme::MUTED),
            )],
            badge,
            area.width,
        ));
    }

    lines.push(Line::raw(""));

    let eta = state.eta;
    let eta_text = eta.map_or_else(
        || String::from("ETA —"),
        |eta| format!("ETA {}", format_duration(eta.seconds)),
    );
    let eta_clock = eta
        .and_then(|eta| eta.seconds.map(|seconds| (seconds, eta.stall_adjusted)))
        .map(|(seconds, adjusted)| {
            let at = AppInstant::from_secs_f64(app.now.secs() + seconds);
            let star = if adjusted { " *" } else { "" };
            format!("≈ {}{star} ", app.wall(at).format("%H:%M"))
        })
        .unwrap_or_default();
    lines.push(spread(
        vec![Span::styled(
            format!(" {eta_text}"),
            Style::default().fg(theme::FG),
        )],
        vec![Span::styled(eta_clock, Style::default().fg(color))],
        area.width,
    ));

    frame.render_widget(Paragraph::new(lines).style(theme::base()), area);

    if let (Some(current), Some(start)) = (state.temp_c, state.start_temp_c)
        && let Some(target) = cfg.target_c
        && area.y + 2 < area.y + area.height
        && target > start
    {
        let ratio = (f64::from(current - start) / f64::from(target - start)).clamp(0.0, 1.0);
        let percent = (ratio * 100.0).round() as u16;
        let gauge_area = Rect {
            x: area.x + 1,
            y: area.y + 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(color).bg(theme::GAUGE_TRACK))
            .ratio(ratio)
            .label(format!("{percent}%"));
        frame.render_widget(gauge, gauge_area);
    }
}

fn draw_pit_card(frame: &mut Frame, area: Rect, app: &App, cfg: &ProbeConfig, state: &ProbeState) {
    let unit = app.unit();
    let color = theme::probe_color(cfg.color);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let (temp_text, temp_style) = match state.temp_c {
        Some(celsius) => (
            unit.format_c(celsius),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        None => state.last_seen_c.map_or_else(
            || (String::from("--"), Style::default().fg(theme::MUTED)),
            |celsius| (unit.format_c(celsius), Style::default().fg(theme::MUTED)),
        ),
    };
    let steady = state.temp_c.map(|_| {
        let deviation = state
            .deviation_c
            .map_or(0.0, |value| unit.delta_from_c(value).abs());
        format!("◆ steady ±{deviation:.1}°{} ", unit.symbol())
    });
    lines.push(spread(
        vec![Span::raw(" "), Span::styled(temp_text, temp_style)],
        steady
            .map(|text| vec![Span::styled(text, Style::default().fg(theme::GREEN))])
            .unwrap_or_default(),
        area.width,
    ));

    if let Some(range) = cfg.range_c {
        let min = unit.from_c(range[0]);
        let max = unit.from_c(range[1]);
        let badge = match state.temp_c.map(|c| unit.from_c(c)) {
            None => Vec::new(),
            Some(value) if value < min => vec![badge_span(" LOW ", theme::RED)],
            Some(value) if value > max => vec![badge_span(" HIGH ", theme::RED)],
            Some(_) => vec![badge_span(" IN RANGE ", theme::GREEN)],
        };
        lines.push(spread(
            vec![Span::styled(
                format!(" range {min:.0}–{max:.0}°{}", unit.symbol()),
                Style::default().fg(theme::MUTED),
            )],
            badge,
            area.width,
        ));

        let width = area.width.saturating_sub(2) as usize;
        if width > 3 {
            let track = state.temp_c.map(|temp| pit_track(unit, range, temp, width));
            lines.push(Line::from(Span::styled(
                track.map_or_else(|| "─".repeat(width), |text| format!(" {text}")),
                Style::default().fg(theme::BLUE),
            )));
            let lo = unit.from_c(range[0]) - 50.0;
            let hi = unit.from_c(range[1]) + 50.0;
            lines.push(spread(
                vec![Span::styled(
                    format!(" {lo:.0}"),
                    Style::default().fg(theme::MUTED),
                )],
                vec![
                    Span::styled(
                        format!("{:.0}", (lo + hi) / 2.0),
                        Style::default().fg(theme::MUTED),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("{hi:.0} "), Style::default().fg(theme::MUTED)),
                ],
                area.width,
            ));
        }
    }

    frame.render_widget(Paragraph::new(lines).style(theme::base()), area);
}

fn rate_spans(unit: Unit, state: &ProbeState) -> Vec<Span<'static>> {
    let Some(rate) = state.rate_c_h else {
        return vec![Span::styled(" — ", Style::default().fg(theme::MUTED))];
    };
    let value = unit.delta_from_c(rate);
    let (arrow, style) = if state.stalled {
        (
            "",
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )
    } else if value > 5.0 {
        ("▲ ", Style::default().fg(theme::GREEN))
    } else if value < 0.0 {
        ("▼ ", Style::default().fg(theme::RED))
    } else {
        ("▲ ", Style::default().fg(theme::MUTED))
    };
    vec![Span::styled(
        format!("{arrow}{value:+.1}°{}/h ", unit.symbol()),
        style,
    )]
}

fn badge_span(text: &str, background: Color) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .bg(background)
            .fg(theme::BG)
            .add_modifier(Modifier::BOLD),
    )
}

fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let left_width: usize = left.iter().map(|span| span.content.chars().count()).sum();
    let right_width: usize = right.iter().map(|span| span.content.chars().count()).sum();
    let mut spans = left;
    if !right.is_empty() {
        let pad = (width as usize).saturating_sub(left_width + right_width);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right);
    }
    Line::from(spans)
}

fn pit_track(unit: Unit, range: [f32; 2], temp_c: f32, width: usize) -> String {
    let low = unit.from_c(range[0]) - 50.0;
    let high = unit.from_c(range[1]) + 50.0;
    let span = (high - low).max(1.0);
    let min = unit.from_c(range[0]);
    let max = unit.from_c(range[1]);
    let current = unit.from_c(temp_c);
    let last = width.saturating_sub(1).max(1);
    let mut chars: Vec<char> = (0..width)
        .map(|x| {
            let value = low + span * (x as f32 / last as f32);
            if value >= min && value <= max {
                '━'
            } else {
                '─'
            }
        })
        .collect();
    let position = (((current - low) / span) * last as f32).round() as isize;
    let position = position.clamp(0, width as isize - 1) as usize;
    if let Some(slot) = chars.get_mut(position) {
        *slot = '█';
    }
    chars.into_iter().collect()
}

fn draw_chart(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let window = app.settings.display.chart_window.minutes();
    let start = window.map_or(AppInstant::ZERO, |minutes| {
        AppInstant::from_secs_f64((app.now.secs() - minutes * 60.0).max(0.0))
    });
    let end = app.now;
    let columns = area.width.saturating_sub(2).max(1) as usize;

    let mut series: Vec<Series> = Vec::new();
    let mut extra: Vec<(Color, Vec<(f64, f64)>)> = Vec::new();
    let mut max_value = 0.0_f64;
    let mut max_target = 0.0_f64;

    for index in 0..PROBE_COUNT {
        let cfg = app.settings.probe(index);
        if !cfg.show_on_chart {
            continue;
        }
        let data = downsample(&app.probes[index].history, start, end, columns, unit);
        if data.is_empty() {
            continue;
        }
        for (_, value) in &data {
            max_value = max_value.max(*value);
        }
        if let Some(target) = cfg.target_c {
            max_target = max_target.max(f64::from(unit.from_c(target)));
        }
        series.push((cfg.label.clone(), theme::probe_color(cfg.color), data));
    }

    if app.settings.display.show_target_lines {
        for index in 0..PROBE_COUNT {
            let cfg = app.settings.probe(index);
            if let Some(target) = cfg.target_c {
                let value = f64::from(unit.from_c(target));
                extra.push((
                    theme::dim(theme::probe_color(cfg.color)),
                    vec![(0.0, value), (x_end(app, window), value)],
                ));
            }
        }
    }
    if app.settings.display.show_pit_range_lines {
        for index in 0..PROBE_COUNT {
            let cfg = app.settings.probe(index);
            if let Some(range) = cfg.range_c {
                for bound in range {
                    let value = f64::from(unit.from_c(bound));
                    extra.push((
                        theme::dim(theme::BLUE),
                        vec![(0.0, value), (x_end(app, window), value)],
                    ));
                }
            }
        }
    }

    let mut datasets: Vec<Dataset> = series
        .iter()
        .map(|(name, color, data)| {
            Dataset::default()
                .name(name.clone())
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data.as_slice())
        })
        .collect();
    for (color, data) in &extra {
        datasets.push(
            Dataset::default()
                .marker(Marker::Dot)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data.as_slice()),
        );
    }

    let x_max = x_end(app, window).max(1.0);
    let y_max = ((max_value.max(max_target) + 25.0) / 50.0).ceil() * 50.0;
    let y_max = y_max.max(50.0);

    let x_labels: Vec<Line> = (0..5)
        .map(|step| {
            let minutes = x_max * f64::from(step) / 4.0;
            let at = AppInstant::from_secs_f64(start.secs() + minutes * 60.0);
            Line::from(app.wall(at).format("%H:%M").to_string())
        })
        .collect();
    let mut y_labels: Vec<Line> = Vec::new();
    let mut value = 0.0;
    while value <= y_max {
        y_labels.push(Line::from(format!("{value:.0}")));
        value += 50.0;
    }

    let title = match window {
        Some(minutes) => format!(
            " Temperature °{} · last {} ",
            unit.symbol(),
            super::settings_window_label(app, minutes)
        ),
        None => format!(
            " Temperature °{} · since {} ",
            unit.symbol(),
            app.wall(start).format("%H:%M")
        ),
    };

    let chart = Chart::new(datasets)
        .block(theme::block(&title))
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .labels(x_labels)
                .style(Style::default().fg(theme::MUTED)),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max])
                .labels(y_labels)
                .style(Style::default().fg(theme::MUTED)),
        )
        .legend_position(Some(LegendPosition::BottomRight));
    frame.render_widget(chart, area);
}

fn x_end(app: &App, window: Option<f64>) -> f64 {
    window.unwrap_or_else(|| app.now.minutes())
}

fn downsample(
    history: &ProbeHistory,
    start: AppInstant,
    end: AppInstant,
    columns: usize,
    unit: Unit,
) -> Vec<(f64, f64)> {
    let span = (end.secs() - start.secs()).max(1.0);
    let mut buckets: Vec<Vec<f64>> = vec![Vec::new(); columns];
    for (at, temp) in history.samples() {
        if *at < start || *at > end {
            continue;
        }
        let Some(celsius) = temp else {
            continue;
        };
        let ratio = (at.secs() - start.secs()) / span;
        let column = ((ratio * columns as f64) as usize).min(columns - 1);
        if let Some(bucket) = buckets.get_mut(column) {
            bucket.push(f64::from(unit.from_c(*celsius)));
        }
    }
    let mut out = Vec::new();
    for (column, values) in buckets.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let x = ((column as f64 + 0.5) / columns as f64) * span / 60.0;
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        out.push((x, min));
        if (max - min).abs() > f64::EPSILON {
            out.push((x, max));
        }
    }
    out
}

fn draw_eta(frame: &mut Frame, area: Rect, app: &App) {
    let unit = app.unit();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut meat: Vec<(usize, f64)> = Vec::new();
    for index in 0..PROBE_COUNT {
        if app.is_pit(index) {
            continue;
        }
        if let Some(seconds) = app.probes[index].eta.and_then(|eta| eta.seconds) {
            meat.push((index, seconds));
        }
    }
    meat.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut stalled_probes: Vec<String> = Vec::new();
    for (index, seconds) in meat {
        let cfg = app.settings.probe(index);
        let color = theme::probe_color(cfg.color);
        let eta = app.probes[index].eta;
        let adjusted = eta.is_some_and(|eta| eta.stall_adjusted);
        if adjusted {
            stalled_probes.push(cfg.label.clone());
        }
        let at = AppInstant::from_secs_f64(app.now.secs() + seconds);
        let clock = format!(
            "{}{}",
            app.wall(at).format("%H:%M"),
            if adjusted { "*" } else { "" }
        );
        lines.push(spread(
            vec![
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(
                    format!("{} {}", ProbeId::from_index(index), cfg.label),
                    Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
                ),
            ],
            vec![Span::styled(
                format!("{clock} "),
                Style::default().fg(color),
            )],
            area.width.saturating_sub(2),
        ));
        let current = app.probes[index]
            .temp_c
            .map_or_else(|| String::from("—"), |c| format!("{:.1}", unit.from_c(c)));
        let target = cfg
            .target_c
            .map_or_else(|| String::from("—"), |c| format!("{:.0}", unit.from_c(c)));
        let rate = app.probes[index].rate_c_h.map_or_else(
            || String::from("—"),
            |r| format!("{:+.1}", unit.delta_from_c(r)),
        );
        lines.push(spread(
            vec![Span::styled(
                format!(
                    "  {current} → {target}°{}  {rate}°{}/h",
                    unit.symbol(),
                    unit.symbol()
                ),
                Style::default().fg(theme::MUTED),
            )],
            vec![Span::styled(
                format!("{} ", format_duration(Some(seconds))),
                Style::default().fg(theme::FG),
            )],
            area.width.saturating_sub(2),
        ));
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(theme::BORDER),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            " All probes done {}",
            app.all_done_seconds.map_or_else(
                || String::from("—"),
                |seconds| {
                    let at = AppInstant::from_secs_f64(app.now.secs() + seconds);
                    format!("≈ {}", app.wall(at).format("%H:%M"))
                }
            )
        ),
        Style::default().fg(theme::FG),
    )));
    let serve_style = if app.serve_late {
        Style::default().fg(theme::RED)
    } else {
        Style::default().fg(theme::FG)
    };
    lines.push(Line::from(Span::styled(
        format!(
            " + {} rest → serve {}",
            format_duration(Some(app.settings.cook.rest_min * 60.0)),
            app.serve_wall.map_or_else(
                || String::from("—"),
                |wall| format!("≈ {}", wall.format("%H:%M"))
            )
        ),
        serve_style,
    )));

    if !stalled_probes.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "* stall-adjusted. Straight-line fit gives {} for {}.",
                meat_linear(app),
                stalled_probes.join(", ")
            ),
            Style::default().fg(theme::MUTED),
        )));
    }
    if let Some(index) = wrap_suggestion(app) {
        let label = app.probe_label(index);
        if let Some(delta) = wrap_delta(app, index) {
            lines.push(Line::from(Span::styled(
                format!("Wrap {label} now → est. −{}", format_duration(Some(delta))),
                Style::default().fg(theme::YELLOW),
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::block(" Estimated completion "))
            .style(theme::base()),
        area,
    );
}

fn meat_linear(app: &App) -> String {
    let values: Vec<String> = (0..PROBE_COUNT)
        .filter(|index| !app.is_pit(*index))
        .filter_map(|index| {
            app.probes[index]
                .eta
                .and_then(|eta| eta.linear_seconds)
                .map(|seconds| format_duration(Some(seconds)))
        })
        .collect();
    if values.is_empty() {
        String::from("—")
    } else {
        values.join(", ")
    }
}

fn wrap_suggestion(app: &App) -> Option<usize> {
    (0..PROBE_COUNT).find(|index| {
        !app.is_pit(*index) && app.probes[*index].stalled && app.probes[*index].wrapped_at.is_none()
    })
}

fn wrap_delta(app: &App, index: usize) -> Option<f64> {
    app.settings.probe(index).target_c?;
    if !app.settings.cook.wrapped_resets_stall {
        return None;
    }
    let started = app.probes[index].stall.started_at()?;
    let elapsed_min = app.now.since(started) / 60.0;
    let remaining_plain = (180.0 - elapsed_min).max(15.0);
    let remaining_wrapped = (90.0 - elapsed_min).max(15.0);
    Some((remaining_plain - remaining_wrapped) * 60.0)
}

fn draw_events(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let height = area.height.saturating_sub(2) as usize;
    for event in app.events.iter().rev().take(height) {
        lines.push(event_line(event));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::block(" Events "))
            .style(theme::base()),
        area,
    );
}

fn event_line(event: &Event) -> Line<'static> {
    let (icon, color) = match event.icon {
        EventIcon::Milestone => ("✓", theme::GREEN),
        EventIcon::Warning => ("⚠", theme::YELLOW),
        EventIcon::Urgent => ("✖", theme::RED),
        EventIcon::Info => ("•", theme::MUTED),
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", event.wall.format("%H:%M")),
            Style::default().fg(theme::MUTED),
        ),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(
            format!("{} ", event.message),
            Style::default().fg(theme::FG),
        ),
    ])
}
