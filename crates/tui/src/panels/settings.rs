// crates/tui/src/panels/settings.rs
// Settings overlay showing gateway configuration and routing options.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_settings(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    // Overlay: center box, height adapts to content
    let overlay_w = 64.min(area.width.saturating_sub(4));
    let overlay_h = 22.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
    let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
    let overlay_area = Rect::new(x, y, overlay_w, overlay_h);

    // Dim background
    let bg = Paragraph::new("")
        .style(Style::default().bg(theme::BG))
        .block(Block::default().style(Style::default().bg(theme::BG)));
    frame.render_widget(bg, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line("Settings", theme::CYAN));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    let content_area = chunks[0];

    let route_mode_idx = match state.gateway_mode.as_str() {
        "quality-first" => 1,
        "latency-first" => 2,
        _ => 0, // cost-first (default)
    };

    let modes = ["cost-first", "quality-first", "latency-first"];
    let mode_lines: Vec<String> = modes
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let marker = if i == route_mode_idx { " ● " } else { " ○ " };
            let desc = match *m {
                "cost-first" => "成本优先 — 选择最便宜的可用模型",
                "quality-first" => "质量优先 — 选择推理能力最强的模型",
                "latency-first" => "延迟优先 — 选择响应最快的模型",
                _ => "",
            };
            format!("{marker}{desc}")
        })
        .collect();

    // ── Gateway status section ──
    let gateway_section = if state.gateway_enabled {
        let model_count = state.gateway_models.len();
        let model_list = if state.gateway_models.is_empty() {
            "  (未发现模型)".to_string()
        } else {
            state
                .gateway_models
                .iter()
                .map(|m| format!("  • {m}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "  Gateway: 已连接 · {model_count} 个模型\n  URL: {url}\n\n  ── 模型列表 ──\n{model_list}",
            url = state.gateway_url,
        )
    } else {
        "  Gateway: 未检测到\n\n  启动方式:\n    hermes --tui --model auto\n    hermes --tui --base-url http://localhost:9090/v1".to_string()
    };

    let shg_status = if state.shg_triggered {
        "SHG: 上次请求触发 (已自动路由到强模型)"
    } else {
        "SHG: 待命中"
    };

    let route_decision = state
        .last_route_decision
        .as_deref()
        .unwrap_or("暂无路由决策");

    let text = format!(
        "{gateway_section}\n\n  ── 路由模式 ──\n  {mode_0}\n  {mode_1}\n  {mode_2}\n\n  ── 状态 ──\n  {shg_status}\n  最近决策: {route_decision}\n\n  按 1/2/3 切换路由模式",
        mode_0 = mode_lines[0],
        mode_1 = mode_lines[1],
        mode_2 = mode_lines[2],
    );

    let para = Paragraph::new(text)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, content_area);

    // Footer hint
    let hint = Line::from(vec![
        Span::styled(
            " 1/2/3 切换模式 ",
            Style::default().fg(theme::BG).bg(theme::CYAN),
        ),
        Span::styled(
            " Esc / s 关闭 ",
            Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().bg(theme::PANEL)),
        chunks[1],
    );
}
