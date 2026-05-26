// crates/tui/src/panels/footer.rs
// Single-line footer with context-sensitive keybinding hints.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, FocusedPanel, LeftTab, TuiAppState};
use crate::theme;

pub fn render_footer(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let hint = if state.help_visible {
        "[Esc] 关闭帮助".to_string()
    } else if state.settings_visible {
        "[Esc] 关闭  [1/2/3] 切换模式".to_string()
    } else if state.output_overlay.is_some() {
        "[Esc] 关闭  [↑↓] 滚动  [←→/n/p] 切步骤  [Ctrl+Y] 复制".to_string()
    } else if state.awaiting_input {
        "[Enter] 提交  [Esc] 取消  [Ctrl+W] 删词  [Ctrl+U] 清行  [←→] 移光标".to_string()
    } else {
        let ctx_hint = if !state.agent_done {
            " [p] 取消"
        } else {
            ""
        };
        let global = " [q] 退出 [h] 帮助 [Ctrl+Y] 复制 [Ctrl+S] 导出";

        match (state.focused_panel, state.phase, state.left_tab, state.agent_done) {
            (FocusedPanel::MainLeft, AgentPhase::Planning, LeftTab::Plan, _) => {
                format!("[Tab] Exec  [↑↓] 滚动{ctx_hint}{global}")
            }
            (FocusedPanel::MainLeft, AgentPhase::Executing, LeftTab::Execution, _) => {
                format!("[Tab] Plan  [↑↓] 选择  [Enter] 详情{ctx_hint}{global}")
            }
            (FocusedPanel::MainLeft, AgentPhase::Planning | AgentPhase::Executing, _, _) => {
                format!("[Tab] 切换  [↑↓] 滚动{ctx_hint}{global}")
            }
            (FocusedPanel::MainLeft, _, _, true) => {
                format!("[Tab] Results/Log  [↑↓] 滚动  [f] 过滤{global}")
            }
            (FocusedPanel::MainLeft, _, _, _) => {
                format!("[↑↓] 滚动  [f] 过滤{ctx_hint}{global}")
            }
            (FocusedPanel::Evolution, _, _, _) => {
                format!("[↑↓] 滚动  [Enter] 折叠全部  [w]权重 [s]统计 [m]元信息{global}")
            }
            (FocusedPanel::MiniLog, _, _, _) => {
                format!("[↑↓] 滚动  [f] 过滤{ctx_hint}{global}")
            }
            (FocusedPanel::Input, _, _, _) => {
                "[Enter] 提交  [↑↓] 历史  [Tab] 切换  [h] 帮助".to_string()
            }
        }
    };

    let line = Line::from(vec![
        Span::styled(" ", Style::default().bg(theme::BG)),
        Span::styled("⌘ ", Style::default().fg(theme::CYAN).bg(theme::BG)),
        Span::styled(hint, Style::default().fg(theme::MUTED).bg(theme::BG)),
    ]);
    let para = Paragraph::new(line).style(Style::default().bg(theme::BG));
    frame.render_widget(para, area);
}
