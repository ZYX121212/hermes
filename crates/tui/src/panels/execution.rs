// crates/tui/src/panels/execution.rs
// Execution panel: per-step progress with layer indentation, progress bar, scrollbar.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_execution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let completed = state.exec_completed_steps;
    let total = state.exec_total_steps;

    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Execution ({}/{}) ", completed, total),
            Style::default().fg(Color::Yellow),
        ));

    if state.executions.is_empty() {
        let text = Paragraph::new("等待执行...")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let inner = block.inner(area);

    // Split: step list + progress bar
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let list_area = chunks[0];
    let bar_area = chunks[1];

    let viewport_h = list_area.height.max(1);

    // Step lines with layer indentation
    let lines: Vec<Line> = state
        .executions
        .iter()
        .map(|step| {
            let indent = "  ".repeat(step.layer.min(6));

            let (icon, color) = match step.status {
                crate::state::StepStatus::Pending => ("○", Color::DarkGray),
                crate::state::StepStatus::Running => {
                    let blink = if state.frame_count % 16 < 8 { "◉" } else { "◎" };
                    (blink, Color::Yellow)
                }
                crate::state::StepStatus::Success => ("✓", Color::Green),
                crate::state::StepStatus::Failed => ("✗", Color::Red),
            };

            let tool = Span::styled(
                format!("{}{} {}", indent, icon, step.tool),
                Style::default().fg(color),
            );

            let duration = step.duration_ms.map(|d| {
                Span::styled(
                    format!("  ({:.1}s)", d as f64 / 1000.0),
                    Style::default().fg(Color::DarkGray),
                )
            });

            let content = step.content_preview.as_deref().map_or_else(
                || Span::raw(""),
                |c| {
                    let clean = crate::state::strip_ansi(c);
                    let short = crate::state::truncate(&clean, 50);
                    Span::styled(format!("  {}", short), Style::default().fg(Color::Gray))
                },
            );

            let mut spans = vec![tool];
            if let Some(d) = duration {
                spans.push(d);
            }
            spans.push(content);
            Line::from(spans)
        })
        .collect();

    let content_height = lines.len();

    // Render step list
    let para = Paragraph::new(lines)
        .scroll((state.exec_scroll, 0));
    frame.render_widget(para, list_area);

    // Render block border
    frame.render_widget(
        Paragraph::new("").block(block),
        area,
    );

    // Progress bar
    if total > 0 {
        let bar_width = bar_area.width.saturating_sub(2).max(1) as usize;
        let filled = (completed as f64 / total as f64 * bar_width as f64).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        let pct = if total > 0 { completed * 100 / total } else { 0 };
        let bar_text = format!(
            " {}{}  {}/{}  {}%",
            "█".repeat(filled),
            "░".repeat(empty),
            completed,
            total,
            pct
        );

        let bar_span = Span::styled(bar_text, Style::default().fg(Color::Cyan));
        let bar_para = Paragraph::new(bar_span);
        frame.render_widget(bar_para, bar_area);
    }

    // Scrollbar (single widget)
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.exec_scroll, content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| Line::from(Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray))))
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
