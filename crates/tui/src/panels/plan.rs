// crates/tui/src/panels/plan.rs
// Plan panel: shows streaming LLM output during planning with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{render_scrollbar, AgentPhase, TuiAppState};

pub fn render_plan(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Plan ({}) ", state.plan_steps_count),
            Style::default().fg(Color::Cyan),
        ));

    let inner = block.inner(area);

    if state.streaming_buffer.is_empty() && !state.plan_ready {
        let text = Paragraph::new("等待规划...")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let cursor = if state.phase == AgentPhase::Planning && state.frame_count % 16 < 8 { "▌" } else { " " };
    let content = format!("{}{}", state.streaming_buffer, cursor);

    let line_count = content.lines().count();

    let text = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((state.plan_scroll, 0));

    frame.render_widget(text, area);

    // Scrollbar (single widget for performance)
    let vh = inner.height.max(1);
    if line_count > vh as usize {
        let bar = render_scrollbar(state.plan_scroll, line_count, vh);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| Line::from(Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray))))
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
