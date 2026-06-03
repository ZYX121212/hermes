// crates/tui/src/panels/overlay.rs
// Full-screen overlay for inspecting complete step output.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, wrapped_line_count, StepOutputOverlay};
use crate::theme;

pub fn render_overlay(frame: &mut Frame, area: Rect, overlay: &StepOutputOverlay) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    // Clear entire area
    frame.render_widget(Clear, area);

    // Overlay takes 80% of screen
    let ow = (area.width as f64 * 0.8) as u16;
    let oh = (area.height as f64 * 0.8) as u16;
    let ox = area.x + (area.width.saturating_sub(ow)) / 2;
    let oy = area.y + (area.height.saturating_sub(oh)) / 2;
    let overlay_rect = Rect::new(ox, oy, ow, oh);

    let status_icon = match overlay.status {
        crate::state::StepStatus::Success => "✓",
        crate::state::StepStatus::Failed => "✗",
        crate::state::StepStatus::Running => "◎",
        crate::state::StepStatus::Pending => "○",
    };

    let short_id: String = overlay.step_id.to_string().chars().take(8).collect();
    let duration_str = overlay
        .duration_ms
        .map(|d| format!("{:.1}s", d as f64 / 1000.0))
        .unwrap_or_else(|| "N/A".to_string());

    let title = format!(
        "{} {}  {}  {}  Esc/Enter/q 关闭",
        status_icon, overlay.tool, short_id, duration_str,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER_FOCUSED))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line(title, theme::CYAN));

    let inner = block.inner(overlay_rect);
    let viewport_h = inner.height;

    let mut lines: Vec<Line> = Vec::new();

    // Info line
    lines.push(Line::from(Span::styled(
        format!(
            " Tool {}   Duration {}   Status {}",
            overlay.tool, duration_str, status_icon
        ),
        Style::default()
            .fg(theme::MUTED)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(inner.width.saturating_sub(2).max(20) as usize),
        Style::default().fg(theme::BORDER).bg(theme::PANEL),
    )));

    // Content lines (preserving newlines from original output)
    for line in overlay.full_content.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(theme::TEXT).bg(theme::PANEL),
        )));
    }

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
        .scroll((clamp_scroll(overlay.scroll, content_height, viewport_h), 0));

    frame.render_widget(para, overlay_rect);

    // Scrollbar
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(clamp_scroll(overlay.scroll, content_height, viewport_h), content_height, viewport_h);
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
            x: overlay_rect.x + overlay_rect.width.saturating_sub(1),
            y: overlay_rect.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
