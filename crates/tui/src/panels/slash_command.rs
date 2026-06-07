// crates/tui/src/panels/slash_command.rs
// Popup overlay for multi-line slash-command results.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, SlashResult};
use crate::theme;

pub fn render_slash_result(frame: &mut Frame, area: Rect, result: &SlashResult) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    let w = 64.min(area.width.saturating_sub(4));
    let h = 20.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER_FOCUSED))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line(&result.title, theme::CYAN));

    let inner = block.inner(popup);
    let viewport_h = inner.height;

    let lines: Vec<Line> = result
        .lines
        .iter()
        .map(|s| {
            Line::from(Span::styled(
                s.clone(),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            ))
        })
        .collect();

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .scroll((clamp_scroll(result.scroll, content_height, viewport_h), 0));

    frame.render_widget(para, popup);

    if content_height > viewport_h as usize {
        let bar = render_scrollbar(
            clamp_scroll(result.scroll, content_height, viewport_h),
            content_height,
            viewport_h,
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
            x: popup.x + popup.width.saturating_sub(1),
            y: popup.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }

    // Footer hint
    let hint_area = Rect {
        x: popup.x + 2,
        y: popup.y + popup.height.saturating_sub(1),
        width: popup.width.saturating_sub(4),
        height: 1,
    };
    if hint_area.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " j/k 滚动  |  Esc 关闭 ",
                Style::default()
                    .fg(theme::SUBTLE)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::BOLD),
            ))),
            hint_area,
        );
    }
}
