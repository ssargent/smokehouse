//! Tokyo Night palette and shared styles.

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::model::ProbeColor;

/// Window background.
pub const BG: Color = Color::Rgb(0x1a, 0x1b, 0x26);
/// Default foreground.
pub const FG: Color = Color::Rgb(0xc0, 0xca, 0xf5);
/// Borders and dividers.
pub const BORDER: Color = Color::Rgb(0x56, 0x5f, 0x89);
/// Dimmed text.
pub const MUTED: Color = Color::Rgb(0x7a, 0x83, 0xa8);
/// Accent.
pub const ACCENT: Color = Color::Rgb(0xff, 0x9e, 0x64);
/// Yellow.
pub const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
/// Green.
pub const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
/// Red.
pub const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
/// Blue.
pub const BLUE: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
/// Selected-row background.
pub const SELECTED_BG: Color = Color::Rgb(0x2f, 0x35, 0x49);
/// Gauge track.
pub const GAUGE_TRACK: Color = Color::Rgb(0x29, 0x2e, 0x42);
/// Field background in Settings.
pub const FIELD_BG: Color = Color::Rgb(0x3b, 0x42, 0x61);

/// The base style applied to the whole frame.
#[must_use]
pub fn base() -> Style {
    Style::default().fg(FG).bg(BG)
}

/// A rounded bordered block with a border-coloured frame.
#[must_use]
pub fn block<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(title)
        .style(base())
}

/// Map a configured probe colour to its RGB value.
#[must_use]
pub fn probe_color(color: ProbeColor) -> Color {
    match color {
        ProbeColor::Red => RED,
        ProbeColor::Orange => ACCENT,
        ProbeColor::Green => GREEN,
        ProbeColor::Blue => BLUE,
        ProbeColor::Purple => Color::Rgb(0xbb, 0x9a, 0xf7),
        ProbeColor::Cyan => Color::Rgb(0x7d, 0xcf, 0xff),
        ProbeColor::Yellow => YELLOW,
        ProbeColor::White => FG,
    }
}

/// Dim a colour towards the border colour, for target lines.
#[must_use]
pub fn dim(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(r / 2 + 0x2b, g / 2 + 0x2f, b / 2 + 0x44),
        other => other,
    }
}
