// crates/tui/src/panels/results.rs
// Structured results report shown after agent completes.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_results(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Results ", Style::default().fg(Color::Green)));

    let inner = block.inner(area);
    let viewport_h = inner.height;
    let text_width = area.width.saturating_sub(2).max(20) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Summary
    if let Some(ref summary) = state.summary {
        lines.push(Line::from(Span::styled(
            format!(" 结果: {}", summary),
            Style::default().fg(Color::Yellow),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " 结果: (无)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(text_width),
        Style::default().fg(Color::DarkGray),
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
        Color::Green
    } else {
        Color::Red
    };
    let step_color = if completed == total && total > 0 {
        Color::Green
    } else {
        Color::Yellow
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!(" 总耗时: {}  ", total_duration),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("步骤: {}/{}  ", completed, total),
            Style::default().fg(step_color),
        ),
        Span::styled(
            format!("成功: {}/{}", success_count, total),
            Style::default().fg(success_color),
        ),
    ]));

    lines.push(Line::from(Span::raw("")));

    // Step list
    if !state.executions.is_empty() {
        for step in &state.executions {
            let (icon, color) = match step.status {
                crate::state::StepStatus::Success => ("✓", Color::Green),
                crate::state::StepStatus::Failed => ("✗", Color::Red),
                crate::state::StepStatus::Running => ("◎", Color::Yellow),
                crate::state::StepStatus::Pending => ("○", Color::DarkGray),
            };
            let indent = "  ".repeat(step.layer.min(4));
            let duration = step
                .duration_ms
                .map(|d| format!(" ({:.1}s)", d as f64 / 1000.0))
                .unwrap_or_default();

            let tool_text = format!("{} {} {}{}", indent, icon, step.tool, duration);
            lines.push(Line::from(Span::styled(tool_text, Style::default().fg(color))));

            if let Some(ref preview) = step.content_preview {
                let preview_indent = format!("{}    ", indent);
                let available = text_width.saturating_sub(preview_indent.len());
                let preview_line = crate::state::truncate(preview, available);
                lines.push(Line::from(Span::styled(
                    format!("{}{}", preview_indent, preview_line),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    lines.push(Line::from(Span::raw("")));

    // Reflection (last reflection entry from log)
    if let Some(reflection) = state
        .log_entries
        .iter()
        .rev()
        .find(|e| e.message.starts_with("反思:"))
    {
        lines.push(Line::from(Span::styled(
            "─".repeat(text_width),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            &reflection.message,
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        " 按 q 退出    |    按 Tab 切换至 Log 面板",
        Style::default().fg(Color::DarkGray),
    )));

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.log_scroll, 0));

    frame.render_widget(para, area);

    // Scrollbar
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
