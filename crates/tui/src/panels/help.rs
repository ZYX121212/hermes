// crates/tui/src/panels/help.rs
// Full-screen help overlay listing all keyboard shortcuts.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_help(frame: &mut Frame, area: Rect, _state: &TuiAppState) {
    // Center a 55x22 help box
    let h_margin = (area.width.saturating_sub(55)) / 2;
    let v_margin = (area.height.saturating_sub(22)) / 2;

    let popup_area = Rect {
        x: area.x + h_margin,
        y: area.y + v_margin,
        width: 55.min(area.width),
        height: 22.min(area.height),
    };

    // Clear background behind popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER_FOCUSED))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line("Help / 快捷键", theme::CYAN));

    let lines = vec![
        Line::from(Span::styled(
            "  全局",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            theme::key("q / Esc / Ctrl+C"),
            Span::styled(
                "  退出程序",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("h / F1"),
            Span::styled(
                "  显示/关闭此帮助",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
        Line::from(Span::styled(
            "  焦点与滚动",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            theme::key("Tab / Shift+Tab"),
            Span::styled(
                "  顺时针/逆时针切换焦点面板",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("↑↓ / j k"),
            Span::styled(
                "  聚焦面板逐行滚动",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("PgUp / PgDn"),
            Span::styled(
                "  聚焦面板翻页",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("Home / End"),
            Span::styled(
                "  跳到顶部/底部",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
        Line::from(Span::styled(
            "  规划/执行阶段",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            theme::key("Tab"),
            Span::styled(
                "  切换 Plan / Exec 标签页",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("↑↓"),
            Span::styled(
                "  选择执行步骤",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(vec![
            theme::key("Enter"),
            Span::styled(
                "  全屏查看步骤完整输出",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
        Line::from(Span::styled(
            "  Evolution 面板",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            theme::key("Enter"),
            Span::styled(
                "  展开/折叠当前 section",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
        Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
        Line::from(Span::styled(
            "  完成阶段",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            theme::key("Tab"),
            Span::styled(
                "  切换 Results / Log 面板",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]),
    ];

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().bg(theme::PANEL));
    frame.render_widget(para, popup_area);
}
