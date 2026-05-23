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

    let mut lines: Vec<Line> = Vec::new();

    // ── Summary banner (prominently displayed at top) ──
    if let Some(ref summary) = state.summary {
        lines.push(Line::from(Span::styled(
            format!(" 结果: {}", summary),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2).max(20) as usize),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::raw("")));
    }

    // ── Log entries ──
    for (i, entry) in state.log_entries.iter().enumerate() {
        let color = if entry.is_error {
            Color::Red
        } else {
            let threshold = state.log_entries.len().saturating_sub(3);
            if i < threshold {
                Color::DarkGray
            } else {
                Color::Gray
            }
        };
        lines.push(Line::from(Span::styled(
            &entry.message,
            Style::default().fg(color),
        )));
    }

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

    // inner width for truncation
    let text_width = area.width.saturating_sub(2) as usize;

    // Show summary in mini-log if present
    if let Some(ref summary) = state.summary {
        let line = Line::from(Span::styled(
            format!(" 结果: {}", crate::state::truncate(summary, text_width.saturating_sub(4))),
            Style::default().fg(Color::Yellow),
        ));
        let para = Paragraph::new(line).block(block);
        frame.render_widget(para, area);
        return;
    }

    if count == 0 {
        let text = Paragraph::new("暂无日志")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let start = count.saturating_sub(3);

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
