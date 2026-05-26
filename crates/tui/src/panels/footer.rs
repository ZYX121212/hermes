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
    } else if state.output_overlay.is_some() {
        "[Esc] 关闭  [↑↓] 滚动  [PgUp/PgDn] 翻页".to_string()
    } else if state.awaiting_input {
        "[Enter] 提交  [Backspace] 删除  [↑↓] 历史".to_string()
    } else {
        match (state.focused_panel, state.phase, state.left_tab, state.agent_done) {
            (FocusedPanel::MainLeft, AgentPhase::Planning, LeftTab::Plan, _) => {
                "[Tab] Exec  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::MainLeft, AgentPhase::Executing, LeftTab::Execution, _) => {
                "[Tab] Plan  [↑↓] 选择  [Enter] 详情  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::MainLeft, AgentPhase::Planning | AgentPhase::Executing, _, _) =>
            {
                "[Tab] 切换  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::MainLeft, _, _, true) => {
                "[Tab] 切换  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::MainLeft, _, _, _) => {
                "[↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::Evolution, _, _, _) => {
                "[↑↓] 滚动  [Enter] 折叠  [q] 退出  [h] 帮助".to_string()
            }
            (FocusedPanel::MiniLog, _, _, _) => {
                "[↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
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
