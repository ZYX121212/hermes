// crates/tui/src/panels/log.rs
// Scrollable log panel showing agent log entries with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, wrapped_line_count, LogEntry, LogFilter, TuiAppState};
use crate::theme;

pub fn render_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let title = format!("Log [{}]", state.log_filter.label());
    let block = theme::panel_block(&title, theme::MUTED, focused);

    let inner = block.inner(area);
    let viewport_h = inner.height;

    if state.log_entries.is_empty() && state.summary.is_none() {
        let text = Paragraph::new(theme::empty("暂无日志"))
            .block(block)
            .style(Style::default().fg(theme::SUBTLE).bg(theme::PANEL));
        frame.render_widget(text, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // ── Summary banner (prominently displayed at top) ──
    if let Some(ref summary) = state.summary {
        lines.push(Line::from(vec![
            Span::styled(
                " RESULT ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", summary),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2).max(20) as usize),
            Style::default().fg(theme::BORDER).bg(theme::PANEL),
        )));
        lines.push(Line::from(Span::styled(
            "",
            Style::default().bg(theme::PANEL),
        )));
    }

    // ── Log entries (with optional filter) ──
    let filtered: Vec<(usize, &LogEntry)> = state
        .log_entries
        .iter()
        .enumerate()
        .filter(|(_, e)| match state.log_filter {
            LogFilter::All => true,
            LogFilter::ErrorsOnly => e.is_error,
        })
        .collect();
    for (_orig_idx, entry) in filtered.iter() {
        let color = if entry.is_error {
            theme::RED
        } else {
            theme::MUTED
        };
        let marker = if entry.is_error { "!" } else { "•" };
        // Truncate message to 80 chars, append repeat count if >0
        let max_msg = 80usize.saturating_sub(if entry.repeat_count > 0 { 6 } else { 0 });
        let truncated = crate::state::truncate(&entry.message, max_msg);
        let display_msg = if entry.repeat_count > 0 {
            format!("{}  (x{})", truncated, entry.repeat_count + 1)
        } else {
            truncated
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {marker} "),
                Style::default().fg(color).bg(theme::PANEL),
            ),
            Span::styled(
                display_msg,
                Style::default().fg(color).bg(theme::PANEL),
            ),
        ]));
    }

    // Calculate wrapped content height for scrollbar
    let _text_width = inner.width.saturating_sub(1).max(1) as usize;
    let content_text: String = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<Vec<&str>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content_height = wrapped_line_count(&content_text, inner.width.saturating_sub(1));

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .wrap(Wrap { trim: false })
        .scroll((clamp_scroll(state.log_scroll, content_height, viewport_h), 0));

    frame.render_widget(para, area);

    // Render scrollbar on the right edge (single widget for performance)
    if content_height > viewport_h as usize {
        let effective_scroll = clamp_scroll(state.log_scroll, content_height, viewport_h);
        let bar = render_scrollbar(effective_scroll, content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| {
                Line::from(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                ))
            })
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
    if area.width < 2 || area.height < 2 {
        return;
    }
    let block = theme::panel_block("Activity", theme::MUTED, focused);

    let count = state.log_entries.len();

    // inner width for truncation
    let text_width = area.width.saturating_sub(2) as usize;

    if count == 0 {
        let text = Paragraph::new(theme::empty("暂无日志"))
            .block(block)
            .style(Style::default().fg(theme::SUBTLE).bg(theme::PANEL));
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
                theme::RED
            } else {
                theme::MUTED
            };
            let max_msg = text_width.saturating_sub(if entry.repeat_count > 0 { 6 } else { 0 });
            let truncated = crate::state::truncate(&entry.message, max_msg);
            let display_msg = if entry.repeat_count > 0 {
                format!("{}  (x{})", truncated, entry.repeat_count + 1)
            } else {
                truncated
            };
            Line::from(Span::styled(display_msg, Style::default().fg(color).bg(theme::PANEL)))
        })
        .collect();

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().bg(theme::PANEL));
    frame.render_widget(para, area);
}
