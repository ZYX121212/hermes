// crates/tui/src/panels/help.rs
// Full-screen help overlay listing all keyboard shortcuts.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {title}"),
        Style::default()
            .fg(theme::CYAN)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    ))
}

fn entry(keys: &[&str], desc: &str) -> Line<'static> {
    let mut spans: Vec<Span> = keys.iter().map(|k| theme::key(*k)).collect();
    spans.push(Span::styled(
        format!("  {desc}"),
        Style::default().fg(theme::MUTED).bg(theme::PANEL),
    ));
    Line::from(spans)
}

fn empty() -> Line<'static> {
    Line::from(Span::styled("", Style::default().bg(theme::PANEL)))
}

pub fn render_help(frame: &mut Frame, area: Rect, _state: &TuiAppState) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let w = 58.min(area.width.saturating_sub(4));
    let h = 26.min(area.height.saturating_sub(2));
    let h_margin = (area.width.saturating_sub(w)) / 2;
    let v_margin = (area.height.saturating_sub(h)) / 2;

    let popup_area = Rect {
        x: area.x + h_margin,
        y: area.y + v_margin,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER_FOCUSED))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line("Help / 快捷键", theme::CYAN));

    let lines = vec![
        section("全局"),
        entry(&["q", "Esc", "Ctrl+C"], "退出"),
        entry(&["h", "F1"], "帮助"),
        entry(&["s", "F2"], "设置面板 (LLM / 搜索 / 金融)"),
        entry(&["p"], "取消当前 agent 操作"),
        entry(&["Ctrl+Y"], "复制聚焦内容到剪贴板"),
        entry(&["Ctrl+S"], "导出对话到文件"),
        empty(),
        section("导航与焦点"),
        entry(&["Tab", "Shift+Tab"], "顺时针/逆时针切换焦点面板"),
        entry(&["↑↓", "j k"], "逐行滚动"),
        entry(&["PgUp", "PgDn"], "翻页"),
        entry(&["Home", "End"], "跳到顶部/底部"),
        entry(&["[", "]"], "调整左右分栏比例"),
        empty(),
        section("输入模式"),
        entry(&["Enter"], "提交任务"),
        entry(&["Shift+Enter"], "换行"),
        entry(&["Esc"], "取消输入"),
        entry(&["←→"], "移动光标"),
        entry(&["Ctrl+W"], "向前删除一个词"),
        entry(&["Ctrl+U"], "删除到行首"),
        entry(&["↑↓"], "浏览输入历史"),
        empty(),
        section("搜索"),
        entry(&["/"], "进入搜索模式"),
        entry(&["n", "N"], "下一个/上一个匹配"),
        entry(&["Esc"], "退出搜索"),
        empty(),
        section("日志面板"),
        entry(&["f"], "切换 All / Errors 过滤"),
        empty(),
        section("执行步骤"),
        entry(&["Enter"], "全屏查看步骤完整输出"),
        entry(&["n", "p"], "上一个/下一个步骤 (全屏时)"),
        entry(&["Ctrl+Tab"], "切换 Plan / Execution 视图"),
        empty(),
        section("标签页"),
        entry(&["Ctrl+T"], "新建标签"),
        entry(&["Ctrl+W"], "关闭标签"),
        entry(&["Ctrl+←/→"], "切换标签"),
        empty(),
        section("工具"),
        entry(&["Ctrl+K"], "切换看板"),
        entry(&["@"], "上下文引用"),
        entry(&[":"], "斜杠命令"),
        empty(),
        section("设置面板"),
        entry(&["Ctrl+Tab"], "切换 LLM / 搜索 / 金融 / 飞书 / 主题页签"),
        entry(&["↑↓"], "选择字段"),
        entry(&["Enter"], "编辑文本 / 切换下拉选项"),
        entry(&["Space"], "开关 boolean 选项"),
        entry(&["Ctrl+S"], "保存设置到磁盘"),
        entry(&["Esc", "s"], "关闭设置面板"),
    ];

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().bg(theme::PANEL));
    frame.render_widget(para, popup_area);
}
