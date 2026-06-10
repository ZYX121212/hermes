// crates/tui/src/panels/results.rs
// Structured results report shown after agent completes.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{
    clamp_scroll, render_scrollbar, wrapped_line_count, StepExecState, TuiAppState,
};
use crate::theme;

const MAX_STEP_PREVIEW_LINES: usize = 10;

pub fn render_results(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let block = theme::panel_block("Result", theme::GREEN, focused);

    let inner = block.inner(area);
    let viewport_h = inner.height;
    let text_width = area.width.saturating_sub(2).max(20) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Summary (including streaming preview)
    let summary_style = Style::default().fg(theme::TEXT).bg(theme::PANEL);
    if let Some(ref summary) = state.summary {
        let mut spans = vec![Span::styled(
            " SUMMARY ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(crate::rich_text::render_markdown_line(summary, summary_style).spans);
        lines.push(Line::from(spans));
    } else if !state.summary_streaming_buffer.is_empty() {
        let preview = crate::state::truncate(
            &state.summary_streaming_buffer,
            text_width.saturating_sub(12),
        );
        let mut spans = vec![Span::styled(
            " SUMMARY ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(crate::rich_text::render_markdown_line(&preview, summary_style).spans);
        lines.push(Line::from(spans));
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

            if let Some(preview_lines) = step_preview_lines(step) {
                let preview_indent = format!("{}    ", indent);
                let indent_style = Style::default().fg(theme::SUBTLE).bg(theme::PANEL);
                for preview_line in preview_lines {
                    let mut spans = vec![ratatui::text::Span::styled(
                        preview_indent.clone(),
                        indent_style,
                    )];
                    spans.extend(
                        crate::rich_text::render_markdown_line(&preview_line, indent_style).spans,
                    );
                    lines.push(Line::from(spans));
                }
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
        " 按 q 退出    |    ↑↓ 滚动    |    Enter 查看完整步骤输出    |    Tab 切换至 Log 面板",
        Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
    )));

    let content_text: String = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content_height = wrapped_line_count(&content_text, inner.width.saturating_sub(1));

    let effective_scroll = clamp_scroll(state.log_scroll, content_height, viewport_h);

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .wrap(Wrap { trim: false })
        .scroll((effective_scroll, 0));

    frame.render_widget(para, area);

    // Scrollbar
    if content_height > viewport_h as usize {
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

fn step_preview_lines(step: &StepExecState) -> Option<Vec<String>> {
    let content = step
        .content_full
        .as_deref()
        .or(step.content_preview.as_deref())?
        .trim();
    if content.is_empty() {
        return None;
    }

    let formatted = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| content.to_string());

    let mut lines: Vec<String> = formatted
        .lines()
        .take(MAX_STEP_PREVIEW_LINES)
        .map(|line| line.to_string())
        .collect();
    if formatted.lines().count() > MAX_STEP_PREVIEW_LINES {
        lines.push("… 按 Enter 查看完整输出".to_string());
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{StepExecState, StepStatus};

    fn make_step(content_full: Option<&str>, content_preview: Option<&str>) -> StepExecState {
        StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: StepStatus::Success,
            content_preview: content_preview.map(|s| s.to_string()),
            content_full: content_full.map(|s| s.to_string()),
            duration_ms: Some(100),
            layer: 0,
        }
    }

    #[test]
    fn test_step_preview_from_full_content() {
        let step = make_step(Some("hello\nworld\n"), None);
        let lines = step_preview_lines(&step).unwrap();
        assert_eq!(lines, vec!["hello", "world"]);
    }

    #[test]
    fn test_step_preview_from_preview_fallback() {
        let step = make_step(None, Some("preview text"));
        let lines = step_preview_lines(&step).unwrap();
        assert_eq!(lines, vec!["preview text"]);
    }

    #[test]
    fn test_step_preview_empty_content() {
        let step = make_step(Some(""), None);
        assert!(step_preview_lines(&step).is_none());
    }

    #[test]
    fn test_step_preview_no_content() {
        let step = make_step(None, None);
        assert!(step_preview_lines(&step).is_none());
    }

    #[test]
    fn test_step_preview_whitespace_only() {
        let step = make_step(Some("   \n  \n"), None);
        // trim() on the joined content results in empty
        assert!(step_preview_lines(&step).is_none());
    }

    #[test]
    fn test_step_preview_json_pretty() {
        let step = make_step(Some(r#"{"key":"value","num":1}"#), None);
        let lines = step_preview_lines(&step).unwrap();
        // JSON should be pretty-printed
        let joined = lines.join("\n");
        assert!(joined.contains("key"), "should contain key: {joined}");
        assert!(joined.contains("value"), "should contain value: {joined}");
    }

    #[test]
    fn test_step_preview_long_output_truncated() {
        let long = (1..=15)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let step = make_step(Some(&long), None);
        let lines = step_preview_lines(&step).unwrap();
        assert_eq!(lines.len(), 11); // 10 + truncation hint
        assert!(lines.last().unwrap().contains("Enter"));
    }
}
