// crates/tui/src/panels/footer.rs
// Single-line footer with context-sensitive keybinding hints.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, FocusedPanel, LeftTab, TuiAppState};
use crate::theme;

pub fn render_footer(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let error_count = state.log_entries.iter().filter(|e| e.is_error).count();
    let error_indicator = if error_count > 0 && !state.log_visible {
        format!(" ⚠ {} errors (l: toggle) |", error_count)
    } else {
        String::new()
    };

    let bg = if focused { theme::PANEL_ALT } else { theme::BG };

    let hint = if state.help_visible {
        if state.awaiting_input {
            "[Esc/h] 关闭帮助  [任意键] 返回输入".to_string()
        } else {
            "[Esc/h] 关闭帮助".to_string()
        }
    } else if state.settings_visible {
        if state.awaiting_input {
            "[Esc/q] 关闭并返回输入".to_string()
        } else {
            let dirty_hint = if state.settings_dirty {
                " [Ctrl+S] 保存"
            } else {
                ""
            };
            format!(
                "[Esc/q] 关闭  [Tab] 切换标签  [Space] 切换值  [↑↓] 导航  [Enter] 编辑{dirty_hint}"
            )
        }
    } else if state.output_overlay.is_some() {
        "[Esc] 关闭  [↑↓] 滚动  [←→/n/p] 切步骤  [Ctrl+Y] 复制".to_string()
    } else if !state.search_match_lines.is_empty() {
        let cur = state.search_current_match.map(|i| i + 1).unwrap_or(0);
        let total = state.search_match_lines.len();
        format!("[n/N] 上一个/下一个匹配 ({cur}/{total})  [Esc] 清除匹配  [/] 继续搜索")
    } else if state.search_active {
        "[Enter] 执行搜索  [Esc] 取消  [←→] 移光标".to_string()
    } else if state.slash_command_active {
        "[Enter] 执行  [Esc] 取消  [←→] 移光标  : /help 查看全部命令".to_string()
    } else if state.awaiting_input {
        "[Enter] 提交  [Esc] 取消  [Ctrl+W] 删词  [Ctrl+U] 清行  [←→] 移光标".to_string()
    } else {
        let ctx_hint = if !state.agent_done { " [p] 取消" } else { "" };
        let global = " [q] 退出 [h] 帮助 [Ctrl+Y] 复制 [Ctrl+S] 导出";

        match (
            state.focused_panel,
            state.phase,
            state.left_tab,
            state.agent_done,
        ) {
            (FocusedPanel::MainLeft, AgentPhase::Planning, LeftTab::Plan, _) => {
                format!("[Ctrl+Tab] Exec  [Tab] 下一面板  [↑↓] 滚动{ctx_hint}{global}")
            }
            (FocusedPanel::MainLeft, AgentPhase::Executing, LeftTab::Execution, _) => {
                format!(
                    "[Ctrl+Tab] Plan  [Tab] 下一面板  [↑↓] 选择  [Enter] 详情{ctx_hint}{global}"
                )
            }
            (FocusedPanel::MainLeft, AgentPhase::Planning | AgentPhase::Executing, _, _) => {
                format!("[Tab] 下一面板  [↑↓] 滚动{ctx_hint}{global}")
            }
            (FocusedPanel::MainLeft, _, _, true) => {
                format!("[Tab] 下一面板  [↑↓] 滚动  [f] 过滤{global}")
            }
            (FocusedPanel::MainLeft, _, _, _) => {
                format!("[Tab] 下一面板  [↑↓] 滚动  [f] 过滤{ctx_hint}{global}")
            }
            (FocusedPanel::Evolution, _, _, _) => {
                format!("[Tab] 下一面板  [↑↓] 滚动  [Enter] 折叠全部  [w]权重 [t]统计 [m]元信息{global}")
            }
            (FocusedPanel::MiniLog, _, _, _) => {
                format!("[Tab] 下一面板  [↑↓] 滚动  [f] 过滤{ctx_hint}{global}")
            }
            (FocusedPanel::Input, _, _, _) => {
                "输入字符开始新任务  [Tab] 下一面板  [h] 帮助".to_string()
            }
        }
    };

    let label_bg = if focused { theme::CYAN } else { theme::MUTED };
    let hint_color = if focused { theme::MUTED } else { theme::SUBTLE };

    let mut spans = vec![Span::styled(" ", Style::default().bg(bg))];
    if !error_indicator.is_empty() {
        spans.push(Span::styled(
            &error_indicator,
            Style::default().fg(theme::RED).bg(bg),
        ));
    }
    spans.push(Span::styled(
        " ⌨ ",
        Style::default()
            .fg(theme::BG)
            .bg(label_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ", Style::default().bg(bg)));
    spans.push(Span::styled(hint, Style::default().fg(hint_color).bg(bg)));

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(bg));
    frame.render_widget(para, area);
}
