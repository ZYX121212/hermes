// crates/tui/src/panels/kanban.rs
// Kanban board panel: displays tasks in Pending / In Progress / Completed columns.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{KanbanItem, KanbanStatus, TuiAppState};
use crate::theme;

pub fn render_kanban(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if area.width < 30 || area.height < 6 {
        return;
    }

    let columns = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);

    let configs = [
        ("Pending", KanbanStatus::Pending, theme::YELLOW),
        ("In Progress", KanbanStatus::InProgress, theme::BLUE),
        ("Completed", KanbanStatus::Completed, theme::GREEN),
    ];

    for (i, (title, status, color)) in configs.iter().enumerate() {
        let col_area = columns[i];
        let items: Vec<&KanbanItem> = state
            .kanban_items
            .iter()
            .filter(|item| item.status == *status)
            .collect();
        let count = items.len();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(*color))
            .style(Style::default().bg(theme::PANEL))
            .title(format!(" {} ({}) ", title, count));

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if items.is_empty() {
            frame.render_widget(
                Paragraph::new(theme::empty("(空)"))
                    .style(Style::default().bg(theme::PANEL)),
                inner,
            );
        } else {
            let lines: Vec<Line> = items
                .iter()
                .map(|item| {
                    let icon = match status {
                        KanbanStatus::Pending => "□",
                        KanbanStatus::InProgress => "⏳",
                        KanbanStatus::Completed => "✓",
                    };
                    Line::from(Span::styled(
                        format!(" {} {}", icon, item.title),
                        Style::default().fg(theme::TEXT).bg(theme::PANEL),
                    ))
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(theme::PANEL)),
                inner,
            );
        }
    }
}
