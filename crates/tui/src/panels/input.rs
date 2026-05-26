// crates/tui/src/panels/input.rs
// Input bar rendered at the bottom: task input, search, or idle prompt.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_input(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        theme::BORDER_FOCUSED
    } else {
        theme::BORDER
    };

    if state.awaiting_input {
        // Active task input mode: show cursor and text
        render_text_input(
            frame, area, state, focused, border_color,
            "TASK", &state.input_text, state.input_cursor,
            if focused {
                "   Enter 确认  |  Esc 取消  |  Ctrl+W 删词  |  Ctrl+U 清行"
            } else {
                "   Tab 切换至此以输入"
            },
        );
    } else if state.search_active {
        // Search mode
        let match_info = if !state.search_match_lines.is_empty() {
            format!(
                "  {}/{} 匹配",
                state.search_current_match.map(|i| i + 1).unwrap_or(0),
                state.search_match_lines.len()
            )
        } else {
            String::new()
        };
        render_text_input(
            frame, area, state, focused, border_color,
            "SEARCH", &state.search_query, state.input_cursor,
            &format!(
                "   Enter 搜索  |  Esc 取消  |  n/N 导航{}",
                match_info
            ),
        );
    } else {
        // Idle / agent done: show prompt
        let hint = if state.agent_done {
            " 完成 — 输入新任务或按 q 退出"
        } else {
            " 等待中 — 按 Tab 切换至此输入新任务"
        };
        let spans: Vec<Span> = vec![
            Span::styled(
                " TASK ",
                Style::default()
                    .fg(theme::BG)
                    .bg(if focused { theme::CYAN } else { theme::MUTED })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " │ ",
                Style::default().fg(border_color).bg(theme::PANEL_ALT),
            ),
            Span::styled(
                hint,
                Style::default().fg(if focused { theme::MUTED } else { theme::SUBTLE }).bg(theme::PANEL_ALT),
            ),
            Span::styled(
                if focused { "   Enter 开始输入  |  / 搜索" } else { "   Tab 切换至此以输入" },
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL_ALT),
            ),
        ];

        let line = Line::from(spans);
        let para = Paragraph::new(line).style(Style::default().bg(theme::PANEL_ALT));
        frame.render_widget(para, area);
    }
}

fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    state: &TuiAppState,
    focused: bool,
    border_color: ratatui::style::Color,
    label: &str,
    text: &str,
    cursor: usize,
    hints: &str,
) {
    let cursor_char = if state.frame_count % 16 < 8 {
        "▌"
    } else {
        " "
    };
    let cursor = cursor.min(text.chars().count());
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let before: String = char_indices[..cursor].iter().map(|(_, c)| c).collect();
    let after: String = char_indices[cursor..].iter().map(|(_, c)| c).collect();

    let label_bg = if focused { theme::CYAN } else { theme::MUTED };

    let mut spans: Vec<Span> = vec![
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme::BG)
                .bg(label_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " │ ",
            Style::default().fg(border_color).bg(theme::PANEL_ALT),
        ),
    ];

    if !before.is_empty() {
        spans.push(Span::styled(
            before,
            Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT),
        ));
    }
    spans.push(Span::styled(
        cursor_char,
        Style::default().fg(theme::CYAN).bg(theme::PANEL_ALT),
    ));
    if !after.is_empty() {
        spans.push(Span::styled(
            after,
            Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT),
        ));
    }
    spans.push(Span::styled(
        hints,
        Style::default().fg(theme::SUBTLE).bg(theme::PANEL_ALT),
    ));

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(theme::PANEL_ALT));
    frame.render_widget(para, area);
}
