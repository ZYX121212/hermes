// crates/tui/src/panels/log.rs
// Scrollable log panel showing agent log entries with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Log ", Style::default().fg(Color::Magenta)));

    let inner = block.inner(area);
    let viewport_h = inner.height;

    if state.log_entries.is_empty() && state.summary.is_none() {
        let text = Paragraph::new("暂无日志")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let lines: Vec<Line> = state
        .log_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let color = if entry.is_error {
                Color::Red
            } else {
                // Dim older entries (beyond last 3)
                let threshold = state.log_entries.len().saturating_sub(3);
                if i < threshold {
                    Color::DarkGray
                } else {
                    Color::Gray
                }
            };
            Line::from(Span::styled(&entry.message, Style::default().fg(color)))
        })
        .collect();

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.log_scroll, 0));

    frame.render_widget(para, area);

    // Render scrollbar on the right edge (single widget for performance)
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.log_scroll, content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| Line::from(Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray))))
            .collect();
        let bar_rect = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}

/// Render a compact mini-log (3-line version used during Planning/Executing phases).
pub fn render_mini_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Log ", Style::default().fg(Color::Magenta)));

    let count = state.log_entries.len();

    if count == 0 {
        let text = Paragraph::new("暂无日志")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let start = if count > 3 { count - 3 } else { 0 };

    // inner width accounts for border columns (left + right = 2)
    let text_width = area.width.saturating_sub(2) as usize;

    let lines: Vec<Line> = state
        .log_entries
        .iter()
        .skip(start)
        .map(|entry| {
            let color = if entry.is_error {
                Color::Red
            } else {
                Color::Gray
            };
            Line::from(Span::styled(
                crate::state::truncate(&entry.message, text_width),
                Style::default().fg(color),
            ))
        })
        .collect();

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
