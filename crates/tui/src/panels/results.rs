// crates/tui/src/panels/results.rs
// Structured results report shown after agent completes.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};
use crate::theme;

pub fn render_results(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let block = theme::panel_block("Results", theme::GREEN, focused);

    let inner = block.inner(area);
    let viewport_h = inner.height;
    let text_width = area.width.saturating_sub(2).max(20) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Summary (including streaming preview)
    if let Some(ref summary) = state.summary {
        lines.push(Line::from(vec![
            Span::styled(
                " SUMMARY ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", summary),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
    } else if !state.summary_streaming_buffer.is_empty() {
        let preview = crate::state::truncate(&state.summary_streaming_buffer, text_width.saturating_sub(12));
        lines.push(Line::from(vec![
            Span::styled(
                " SUMMARY ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}…", preview),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
    } else {
        lines.push(theme::empty("暂无结果"));
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(text_width),
        Style::default().fg(theme::BORDER).bg(theme::PANEL),
    )));

    // Key metrics
    let total_duration = state
        .total_duration_ms
        .map(|d| format!("{:.1}s", d as f64 / 1000.0))
        .unwrap_or_else(|| "N/A".to_string());
    let completed = state.exec_completed_steps;
    let total = state.exec_total_steps;
    let success_count = state
        .executions
        .iter()
        .filter(|s| s.status == crate::state::StepStatus::Success)
        .count();

    let success_color = if success_count == total && total > 0 {
        theme::GREEN
    } else {
        theme::RED
    };
    let step_color = if completed == total && total > 0 {
        theme::GREEN
    } else {
        theme::YELLOW
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!(" 总耗时: {}  ", total_duration),
            Style::default().fg(theme::TEXT).bg(theme::PANEL),
        ),
        Span::styled(
            format!("步骤: {}/{}  ", completed, total),
            Style::default().fg(step_color).bg(theme::PANEL),
        ),
        Span::styled(
            format!("成功: {}/{}", success_count, total),
            Style::default().fg(success_color).bg(theme::PANEL),
        ),
    ]));

    lines.push(Line::from(Span::styled(
        "",
        Style::default().bg(theme::PANEL),
    )));

    // Step list
    if !state.executions.is_empty() {
        for step in &state.executions {
            let (icon, color) = match step.status {
                crate::state::StepStatus::Success => ("✓", theme::GREEN),
                crate::state::StepStatus::Failed => ("✗", theme::RED),
                crate::state::StepStatus::Running => ("◎", theme::YELLOW),
                crate::state::StepStatus::Pending => ("○", theme::SUBTLE),
            };
            let indent = "  ".repeat(step.layer.min(4));
            let duration = step
                .duration_ms
                .map(|d| format!(" ({:.1}s)", d as f64 / 1000.0))
                .unwrap_or_default();

            let tool_text = format!("{} {} {}{}", indent, icon, step.tool, duration);
            lines.push(Line::from(Span::styled(
                tool_text,
                Style::default().fg(color).bg(theme::PANEL),
            )));

            if let Some(ref preview) = step.content_preview {
                let preview_indent = format!("{}    ", indent);
                let available = text_width.saturating_sub(preview_indent.len());
                let preview_line = crate::state::truncate(preview, available);
                lines.push(Line::from(Span::styled(
                    format!("{}{}", preview_indent, preview_line),
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                )));
            }
        }
    }

    lines.push(Line::from(Span::styled(
        "",
        Style::default().bg(theme::PANEL),
    )));

    // Reflection (last reflection entry from log)
    if let Some(reflection) = state
        .log_entries
        .iter()
        .rev()
        .find(|e| e.message.starts_with("反思:"))
    {
        lines.push(Line::from(Span::styled(
            "─".repeat(text_width),
            Style::default().fg(theme::BORDER).bg(theme::PANEL),
        )));
        lines.push(Line::from(Span::styled(
            &reflection.message,
            Style::default().fg(theme::TEXT).bg(theme::PANEL),
        )));
    }

    lines.push(Line::from(Span::styled(
        "",
        Style::default().bg(theme::PANEL),
    )));
    lines.push(Line::from(Span::styled(
        " 按 q 退出    |    按 Tab 切换至 Log 面板",
        Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
    )));

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .scroll((state.log_scroll, 0));

    frame.render_widget(para, area);

    // Scrollbar
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.log_scroll, content_height, viewport_h);
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
