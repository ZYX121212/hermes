// crates/tui/src/panels/help.rs
// Full-screen help overlay listing all keyboard shortcuts.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;

pub fn render_help(frame: &mut Frame, area: Rect, _state: &TuiAppState) {
    // Center a 50x16 help box
    let h_margin = (area.width.saturating_sub(50)) / 2;
    let v_margin = (area.height.saturating_sub(16)) / 2;

    let popup_area = Rect {
        x: area.x + h_margin,
        y: area.y + v_margin,
        width: 50.min(area.width),
        height: 16.min(area.height),
    };

    // Clear background behind popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" Help — 快捷键 ", Style::default().fg(Color::Cyan)));

    let lines = vec![
        Line::from(Span::styled("── 全局 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  q / Esc / Ctrl+C    ", Style::default().fg(Color::White)),
            Span::styled("退出程序", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  h / F1              ", Style::default().fg(Color::White)),
            Span::styled("显示/关闭此帮助", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled("── 焦点与滚动 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  Tab / Shift+Tab     ", Style::default().fg(Color::White)),
            Span::styled("顺时针/逆时针切换焦点面板", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  ↑↓ / j k            ", Style::default().fg(Color::White)),
            Span::styled("聚焦面板逐行滚动", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  PgUp / PgDn         ", Style::default().fg(Color::White)),
            Span::styled("聚焦面板翻页", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  Home / End          ", Style::default().fg(Color::White)),
            Span::styled("跳到顶部/底部", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled("── Evolution 面板 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  Enter               ", Style::default().fg(Color::White)),
            Span::styled("展开/折叠当前 section", Style::default().fg(Color::Gray)),
        ]),
    ];

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, popup_area);
}
