// crates/tui/src/panels/plan.rs
// Plan panel: shows streaming LLM output during planning with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, wrapped_line_count, AgentPhase, TuiAppState};
use crate::theme;

pub fn render_plan(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let block = theme::panel_block(
        format!("Plan  {} steps", state.plan_steps_count),
        theme::CYAN,
        focused,
    );

    let inner = block.inner(area);

    if state.streaming_buffer.is_empty() && !state.plan_ready {
        let text = Paragraph::new(theme::empty("等待规划输出..."))
            .block(block)
            .style(Style::default().fg(theme::SUBTLE).bg(theme::PANEL));
        frame.render_widget(text, area);
        return;
    }

    let cursor = if state.phase == AgentPhase::Planning {
        "▌"
    } else {
        ""
    };
    let content = format!("{}{}", state.streaming_buffer, cursor);

    let vh = inner.height.max(1);
    let line_count = wrapped_line_count(&content, inner.width.saturating_sub(1));

    let lines = crate::rich_text::render_markdown_lines(
        &content,
        Style::default().fg(theme::TEXT).bg(theme::PANEL),
    );

    let text = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .wrap(Wrap { trim: false })
        .scroll((clamp_scroll(state.plan_scroll, line_count, vh), 0));

    frame.render_widget(text, area);

    // Scrollbar (single widget for performance)
    if line_count > vh as usize {
        let bar = render_scrollbar(
            clamp_scroll(state.plan_scroll, line_count, vh),
            line_count,
            vh,
        );
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
            height: vh,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
