// crates/tui/src/panels/tab_bar.rs
// Session tab bar rendered between header and main area.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_tab_bar(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if state.session_tabs.len() <= 1 {
        return;
    }

    let tabs: Vec<Span> = state
        .session_tabs
        .iter()
        .enumerate()
        .flat_map(|(i, tab)| {
            let is_active = i == state.active_tab_index;
            let style = if is_active {
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::CYAN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED).bg(theme::PANEL)
            };
            let close = Span::styled(
                "x",
                Style::default().fg(if is_active { theme::BG } else { theme::SUBTLE }),
            );
            let name = Span::styled(format!(" {} ", tab.name), style);
            vec![
                name,
                close,
                Span::styled(" ", Style::default().bg(theme::BG)),
            ]
        })
        .collect();

    let plus = Span::styled(" + ", Style::default().fg(theme::SUBTLE).bg(theme::BG));
    let all_spans: Vec<Span> = tabs.into_iter().chain(std::iter::once(plus)).collect();

    frame.render_widget(
        Paragraph::new(Line::from(all_spans)).style(Style::default().bg(theme::BG)),
        area,
    );
}
