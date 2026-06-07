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
            frame,
            area,
            state,
            focused,
            border_color,
            InputContent {
                label: "TASK",
                text: &state.input_text,
                cursor: state.input_cursor,
                hints: if focused {
                    "   Enter 确认  |  Esc 取消  |  Ctrl+W 删词  |  Ctrl+U 清行"
                } else {
                    "   Tab 切换至此以输入"
                },
            },
        );
    } else if state.slash_command_active {
        render_text_input(
            frame,
            area,
            state,
            focused,
            border_color,
            InputContent {
                label: "CMD",
                text: &state.slash_command_buffer,
                cursor: state.slash_command_cursor,
                hints: if focused {
                    "   Enter 执行  |  Esc 取消  |  : /help 查看全部命令"
                } else {
                    "   Tab 切换至此以输入"
                },
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
            frame,
            area,
            state,
            focused,
            border_color,
            InputContent {
                label: "SEARCH",
                text: &state.search_query,
                cursor: state.input_cursor,
                hints: &format!("   Enter 搜索  |  Esc 取消  |  n/N 导航{}", match_info),
            },
        );
    } else {
        // Idle / agent done: show prompt
        let hint = if state.agent_done {
            " 完成 — Tab 到此或按 i 继续输入"
        } else {
            " 运行中 — 输入将在 agent 请求时启用"
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
                Style::default()
                    .fg(if focused { theme::MUTED } else { theme::SUBTLE })
                    .bg(theme::PANEL_ALT),
            ),
            Span::styled(
                if focused {
                    "   i 输入  |  / 搜索  |  : 命令"
                } else {
                    "   Tab 切换焦点  |  i 输入"
                },
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL_ALT),
            ),
        ];

        let line = Line::from(spans);
        let para = Paragraph::new(line).style(Style::default().bg(theme::PANEL_ALT));
        frame.render_widget(para, area);
    }
}

struct InputContent<'a> {
    label: &'a str,
    text: &'a str,
    cursor: usize,
    hints: &'a str,
}

fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    state: &TuiAppState,
    focused: bool,
    border_color: ratatui::style::Color,
    content: InputContent<'_>,
) {
    let cursor_char = if state.frame_count % 16 < 8 {
        "▌"
    } else {
        " "
    };
    let text = content.text;
    let cursor = content.cursor.min(text.chars().count());
    let label_bg = if focused { theme::CYAN } else { theme::MUTED };

    // Split into lines
    let input_lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split('\n').collect()
    };

    // Find cursor position (which line, which column)
    let mut char_count = 0usize;
    let mut cursor_line = 0usize;
    let mut cursor_col = 0usize;
    for (i, line) in input_lines.iter().enumerate() {
        let line_len = line.chars().count();
        if char_count + line_len >= cursor || i == input_lines.len() - 1 {
            cursor_line = i;
            cursor_col = cursor.saturating_sub(char_count);
            break;
        }
        char_count += line_len + 1; // +1 for newline char
    }

    let visible_lines = state.input_line_count.max(1) as usize;
    let start_line = cursor_line.saturating_sub(visible_lines.saturating_sub(1));

    let mut all_lines: Vec<Line> = Vec::new();
    let label_width = content.label.chars().count() + 4; // " LABEL │ "

    for i in start_line..(start_line + visible_lines).min(input_lines.len()) {
        let line = input_lines.get(i).copied().unwrap_or("");
        let mut spans: Vec<Span> = Vec::new();

        if i == start_line {
            spans.push(Span::styled(
                format!(" {} ", content.label),
                Style::default()
                    .fg(theme::BG)
                    .bg(label_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                " │ ",
                Style::default().fg(border_color).bg(theme::PANEL_ALT),
            ));
        } else {
            spans.push(Span::styled(
                " ".repeat(label_width),
                Style::default().bg(theme::PANEL_ALT),
            ));
        }

        let line_chars: Vec<char> = line.chars().collect();
        if i == cursor_line {
            let col = cursor_col.min(line_chars.len());
            let before: String = line_chars[..col].iter().collect();
            let after: String = line_chars[col..].iter().collect();
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
        } else {
            spans.push(Span::styled(
                line,
                Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT),
            ));
        }

        all_lines.push(Line::from(spans));
    }

    // Add hint on a separate line
    all_lines.push(Line::from(Span::styled(
        content.hints,
        Style::default().fg(theme::SUBTLE).bg(theme::PANEL_ALT),
    )));

    let para = Paragraph::new(all_lines).style(Style::default().bg(theme::PANEL_ALT));
    frame.render_widget(para, area);
}
