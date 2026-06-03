// crates/tui/src/panels/execution.rs
// Execution panel: per-step progress with layer indentation, progress bar, scrollbar.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, TuiAppState};
use crate::theme;

pub fn render_execution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    if area.width < 3 || area.height < 4 {
        return;
    }
    let completed = state.exec_completed_steps;
    let total = state.exec_total_steps;

    let block = theme::panel_block(
        format!("Execute  {completed}/{total}"),
        theme::YELLOW,
        focused,
    );

    if state.executions.is_empty() {
        let text = Paragraph::new(theme::empty("等待执行步骤..."))
            .block(block)
            .style(Style::default().fg(theme::SUBTLE).bg(theme::PANEL));
        frame.render_widget(text, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split: step list + progress bar
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .margin(0)
        .split(inner);

    let list_area = chunks[0];
    let bar_area = chunks[1];

    let viewport_h = list_area.height.max(1);

    // Step lines with layer indentation
    let lines: Vec<Line> = state
        .executions
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            let indent = "  ".repeat(step.layer.min(6));

            let (icon, color) = match step.status {
                crate::state::StepStatus::Pending => ("○", theme::SUBTLE),
                crate::state::StepStatus::Running => ("◉", theme::YELLOW),
                crate::state::StepStatus::Success => ("✓", theme::GREEN),
                crate::state::StepStatus::Failed => ("✗", theme::RED),
            };

            let selector = if state.exec_selected_index == Some(idx) {
                "› "
            } else {
                "  "
            };

            let tool = Span::styled(
                format!("{}{}{} {}", selector, indent, icon, step.tool),
                Style::default().fg(color).bg(theme::PANEL).add_modifier(
                    if step.status == crate::state::StepStatus::Running {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            );

            let duration = step.duration_ms.map(|d| {
                let formatted = if d < 1 {
                    "<1ms".to_string()
                } else if d < 1000 {
                    format!("{}ms", d)
                } else if d < 60_000 {
                    format!("{:.1}s", d as f64 / 1000.0)
                } else {
                    let minutes = d / 60_000;
                    let secs = (d % 60_000) / 1000;
                    format!("{}m{}s", minutes, secs)
                };
                Span::styled(
                    format!("  ({})", formatted),
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                )
            });

            let content = step.content_preview.as_deref().map_or_else(
                || Span::raw(""),
                |c| {
                    let clean = crate::state::strip_ansi(c);
                    let short = crate::state::truncate(&clean, 30);
                    Span::styled(
                        format!("  {}", short),
                        Style::default().fg(theme::MUTED).bg(theme::PANEL),
                    )
                },
            );

            let mut spans = vec![tool];
            if let Some(d) = duration {
                spans.push(d);
            }
            spans.push(content);

            let line_style = if state.exec_selected_index == Some(idx) {
                Style::default().bg(theme::PANEL_ALT)
            } else {
                Style::default().bg(theme::PANEL)
            };
            Line::from(spans).style(line_style)
        })
        .collect();

    let content_height = lines.len();

    // Render step list
    let para = Paragraph::new(lines)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .scroll((clamp_scroll(state.exec_scroll, content_height, viewport_h), 0));
    frame.render_widget(para, list_area);

    // Progress bar
    if total > 0 {
        let pct = if total > 0 {
            completed * 100 / total
        } else {
            0
        };
        let ratio = completed as f64 / total as f64;
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(theme::CYAN).bg(theme::PANEL_ALT))
            .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
            .label(format!(" {completed}/{total}  {pct}% "))
            .ratio(ratio.clamp(0.0, 1.0));
        frame.render_widget(gauge, bar_area);
    }

    // Scrollbar (single widget)
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(clamp_scroll(state.exec_scroll, content_height, viewport_h), content_height, viewport_h);
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
            y: list_area.y,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
