// crates/tui/src/panels/input.rs
// Input bar rendered at the bottom when awaiting user input in TUI interactive mode.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_input(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let cursor = if state.frame_count % 16 < 8 {
        "▌"
    } else {
        " "
    };
    let line = Line::from(vec![
        Span::styled(
            " TASK ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().bg(theme::PANEL_ALT)),
        Span::styled(
            &state.input_text,
            Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT),
        ),
        Span::styled(
            cursor,
            Style::default().fg(theme::CYAN).bg(theme::PANEL_ALT),
        ),
    ]);

    let para = Paragraph::new(line).style(Style::default().bg(theme::PANEL_ALT));

    frame.render_widget(para, area);
}
