// crates/tui/src/panels/footer.rs
// Single-line footer with context-sensitive keybinding hints.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, FocusedPanel, TuiAppState};

pub fn render_footer(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let hint = if state.help_visible {
        "[Esc/h/F1]关闭帮助"
    } else if state.awaiting_input {
        "[Enter]提交  [Backspace]删除  [↑↓]历史"
    } else {
        match (state.focused_panel, state.phase) {
            (FocusedPanel::MainLeft, AgentPhase::Planning) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助"
            }
            (FocusedPanel::MainLeft, AgentPhase::Executing) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助"
            }
            (FocusedPanel::MainLeft, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Home/End]首尾  [q]退出  [h]帮助"
            }
            (FocusedPanel::Evolution, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Enter]展开/折叠  [q]退出  [h]帮助"
            }
            (FocusedPanel::MiniLog, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [q]退出  [h]帮助"
            }
        }
    };

    let span = Span::styled(hint, Style::default().fg(Color::DarkGray));
    let para = Paragraph::new(span);
    frame.render_widget(para, area);
}
