// crates/tui/src/panels/input.rs
// Input bar rendered at the bottom when awaiting user input in TUI interactive mode.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::TuiAppState;

pub fn render_input(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let cursor = if state.frame_count % 16 < 8 { "▌" } else { " " };
    let text = format!("▸ {}{}", state.input_text, cursor);

    let para = Paragraph::new(text)
        .style(Style::default().fg(Color::White));

    frame.render_widget(para, area);
}
