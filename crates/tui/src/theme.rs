// crates/tui/src/theme.rs
// Shared visual language for the terminal UI.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

pub const BG: Color = Color::Rgb(11, 18, 32);
pub const PANEL: Color = Color::Rgb(15, 23, 42);
pub const PANEL_ALT: Color = Color::Rgb(17, 31, 48);
pub const BORDER: Color = Color::Rgb(51, 65, 85);
pub const BORDER_FOCUSED: Color = Color::Rgb(56, 189, 248);
pub const TEXT: Color = Color::Rgb(226, 232, 240);
pub const MUTED: Color = Color::Rgb(148, 163, 184);
pub const SUBTLE: Color = Color::Rgb(100, 116, 139);
pub const CYAN: Color = Color::Rgb(34, 211, 238);
pub const BLUE: Color = Color::Rgb(96, 165, 250);
pub const GREEN: Color = Color::Rgb(52, 211, 153);
pub const YELLOW: Color = Color::Rgb(251, 191, 36);
pub const RED: Color = Color::Rgb(248, 113, 113);
pub const MAGENTA: Color = Color::Rgb(216, 180, 254);

pub fn panel_block<'a>(title: impl Into<String>, accent: Color, focused: bool) -> Block<'a> {
    let border = if focused { BORDER_FOCUSED } else { BORDER };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(PANEL))
        .title(title_line(title, accent))
}

pub fn title_line(title: impl Into<String>, accent: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::default().bg(PANEL)),
        Span::styled("▌", Style::default().fg(accent).bg(PANEL)),
        Span::styled(
            format!(" {} ", title.into()),
            Style::default()
                .fg(TEXT)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub fn empty(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text.into()),
        Style::default().fg(SUBTLE).bg(PANEL),
    ))
}

pub fn key(text: impl Into<String>) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default()
            .fg(TEXT)
            .bg(Color::Rgb(30, 41, 59))
            .add_modifier(Modifier::BOLD),
    )
}
