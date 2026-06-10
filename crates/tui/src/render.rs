// crates/tui/src/render.rs
// Main render function: phase-adaptive layout with focus-aware borders.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::panels;
use crate::state::{AgentPhase, FocusedPanel, TuiAppState};
use crate::theme;

pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 10;
pub const RENDER_POLL_MS: u64 = 16;

pub fn render_app(frame: &mut Frame, state: &TuiAppState) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            " 终端太小 ({w}×{h})  请调整至 {mw}×{mh} 或更大",
            w = area.width,
            h = area.height,
            mw = MIN_WIDTH,
            mh = MIN_HEIGHT,
        );
        let para =
            Paragraph::new(msg.as_str()).style(Style::default().fg(theme::RED).bg(theme::BG));
        frame.render_widget(para, area);
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);

    // ── Vertical split: header (1), session tabs (opt), main (fill), footer (dyn) ──
    let session_tabs_h = if state.session_tabs.len() > 1 { 1 } else { 0 };

    // Footer height: 1 line for hint + input_line_count lines for multiline input
    let footer_h = if state.awaiting_input || state.search_active || state.slash_command_active {
        (state.input_line_count as u16).max(1) + 1 // +1 for hints line
    } else {
        1
    };

    let v_chunks = Layout::vertical([
        Constraint::Length(1),              // header
        Constraint::Length(session_tabs_h), // session tabs
        Constraint::Min(1),                 // main
        Constraint::Length(footer_h),       // footer
    ])
    .split(area);

    let header_area = v_chunks[0];
    let session_tabs_area = v_chunks[1];
    let main_area = v_chunks[2];

    // ── Render session tab bar between header and main area (only when multiple tabs) ──
    if state.session_tabs.len() > 1 {
        panels::tab_bar::render_tab_bar(frame, session_tabs_area, state);
    }
    let footer_area = v_chunks[3];

    // ── Main area horizontal split ──
    let (left_pct, right_pct) = state.split_pct();

    let h_chunks = Layout::horizontal([
        Constraint::Percentage(left_pct),
        Constraint::Percentage(right_pct),
    ])
    .split(main_area);

    let left_area = h_chunks[0];
    let right_area = h_chunks[1];

    // ── Render left panel based on phase ──
    match state.phase {
        AgentPhase::Planning | AgentPhase::Executing => {
            use crate::state::LeftTab;

            // Split left area: tab bar (1) + content (fill)
            let left_chunks =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(left_area);

            let tab_area = left_chunks[0];
            let content_area = left_chunks[1];

            // Render tab bar
            let active_style = Style::default()
                .fg(theme::BG)
                .bg(theme::CYAN)
                .add_modifier(Modifier::BOLD);
            let idle_style = Style::default().fg(theme::MUTED).bg(theme::BG);
            let bg_style = Style::default().bg(theme::BG);
            let plan_style = if state.left_tab == LeftTab::Plan {
                active_style
            } else {
                idle_style
            };
            let exec_style = if state.left_tab == LeftTab::Execution {
                active_style
            } else {
                idle_style
            };
            let tabs_text = Line::from(vec![
                Span::styled(" ", bg_style),
                Span::styled(" PLAN ", plan_style),
                Span::styled(" ", bg_style),
                Span::styled(" EXEC ", exec_style),
                Span::styled(
                    "  Tab 切换标签  Shift+Tab 切换焦点",
                    Style::default().fg(Color::DarkGray).bg(theme::BG),
                ),
            ]);
            frame.render_widget(Paragraph::new(tabs_text).style(bg_style), tab_area);

            // Render content based on selected tab
            let main_focused = state.focused_panel == FocusedPanel::MainLeft;
            match state.left_tab {
                LeftTab::Plan => {
                    panels::plan::render_plan(frame, content_area, state, main_focused);
                }
                LeftTab::Execution => {
                    panels::execution::render_execution(frame, content_area, state, main_focused);
                }
            }
        }
        _ => {
            if state.turn == 0 && !state.agent_done && state.phase == AgentPhase::Idle {
                render_welcome(frame, left_area, state);
            } else if state.results_visible
                && (state.agent_done
                    || state.summary.is_some()
                    || !state.summary_streaming_buffer.is_empty())
            {
                panels::results::render_results(
                    frame,
                    left_area,
                    state,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
            } else if state.log_visible {
                panels::log::render_log(
                    frame,
                    left_area,
                    state,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
            } else if state.agent_done && state.summary.is_some() {
                panels::results::render_results(
                    frame,
                    left_area,
                    state,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
            } else {
                // Log hidden: show minimal panel
                let block = theme::panel_block(
                    "Hermess",
                    theme::MUTED,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
                let hint = if state.agent_done {
                    "按 l 显示日志"
                } else {
                    "运行中... (l: 日志)"
                };
                let text = Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                )))
                .block(block)
                .style(Style::default().bg(theme::PANEL));
                frame.render_widget(text, left_area);
            }
        }
    }

    // ── Render evolution panel (always on the right) ──
    if state.kanban_visible {
        panels::kanban::render_kanban(frame, right_area, state);
    } else {
        panels::evolution::render_evolution(
            frame,
            right_area,
            state,
            state.focused_panel == FocusedPanel::Evolution,
        );
    }

    // ── Render header ──
    panels::header::render_header(frame, header_area, state);

    // ── Render footer / input bar ──
    let input_focused = state.focused_panel == FocusedPanel::Input;
    if state.awaiting_input || state.search_active || state.slash_command_active || input_focused {
        // Show the input bar (with text field, search box, or idle prompt)
        panels::input::render_input(frame, footer_area, state, input_focused);
    } else {
        // Show context-sensitive keyboard hints when focus is elsewhere
        panels::footer::render_footer(frame, footer_area, state, false);
    }

    // ── Render @-mention context reference popup above input area ──
    if state.context_ref_active {
        panels::context_ref::render_context_ref(frame, footer_area, state);
    }

    // ── Render help overlay (on top of everything) ──
    if state.help_visible {
        panels::help::render_help(frame, area, state);
    }

    // ── Render settings overlay ──
    if state.settings_visible {
        panels::settings::render_settings(frame, area, state);
    }

    // ── Render output overlay (on top of everything, including help) ──
    if let Some(ref overlay) = state.output_overlay {
        panels::overlay::render_overlay(frame, area, overlay);
    }

    // ── Render slash command result popup (on top of output overlay) ──
    if let Some(ref result) = state.slash_command_popup {
        panels::slash_command::render_slash_result(frame, area, result);
    }
}

/// Welcome screen shown when TUI starts before the first task.
fn render_welcome(frame: &mut Frame, area: ratatui::layout::Rect, state: &TuiAppState) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::widgets::Wrap;

    if area.width < 2 || area.height < 4 {
        return;
    }

    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL))
        .title(theme::title_line("Welcome", theme::CYAN));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Full welcome when there's enough space
    if area.height >= 13 {
        let chunks = Layout::vertical([
            Constraint::Length(1), // spacer
            Constraint::Length(5), // welcome text
            Constraint::Length(1), // spacer
            Constraint::Length(4), // settings info
            Constraint::Min(0),
        ])
        .split(inner);

        let welcome_lines = vec![
            Line::from(Span::styled(
                "  Hermess AI Agent — 智能任务编排与路由",
                Style::default()
                    .fg(theme::CYAN)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  基于 LLM 的多步骤推理与工具调用框架",
                Style::default().fg(theme::MUTED).bg(theme::PANEL),
            )),
            Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
            Line::from(Span::styled(
                "  按 Enter 开始输入任务，或输入 / 进入搜索",
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            )),
        ];
        frame.render_widget(
            Paragraph::new(welcome_lines).style(Style::default().bg(theme::PANEL)),
            chunks[1],
        );

        let settings_hint = if state.user_settings.llm_api_key.is_empty() {
            vec![
                Line::from(Span::styled(
                    "  ⚡ 快速开始:",
                    Style::default()
                        .fg(theme::YELLOW)
                        .bg(theme::PANEL)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "  1. 按 s 打开设置面板",
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    "  2. 填入 LLM API Key 和 Provider",
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    "  3. 按 Ctrl+S 保存，下次启动生效",
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                )),
            ]
        } else {
            let provider_label = match state.user_settings.llm_provider.as_str() {
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                "deepseek" => "DeepSeek",
                _ => "已配置",
            };
            vec![
                Line::from(Span::styled(
                    format!("  ✓ LLM: {provider_label} 已配置"),
                    Style::default()
                        .fg(theme::GREEN)
                        .bg(theme::PANEL)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "  模型: {}",
                        if state.user_settings.llm_model.is_empty() {
                            "(默认)"
                        } else {
                            &state.user_settings.llm_model
                        }
                    ),
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                )),
                Line::from(Span::styled("", Style::default().bg(theme::PANEL))),
                Line::from(Span::styled(
                    "  按 s 打开设置面板修改配置",
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                )),
            ]
        };

        frame.render_widget(
            Paragraph::new(settings_hint)
                .style(Style::default().bg(theme::PANEL))
                .wrap(Wrap { trim: false }),
            chunks[3],
        );
    } else {
        // Slim welcome for small terminals
        let lines = vec![
            Span::styled(
                "Hermess AI Agent",
                Style::default()
                    .fg(theme::CYAN)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " — 按 Enter 输入任务",
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ];
        let hint = Paragraph::new(Line::from(lines)).style(Style::default().bg(theme::PANEL));
        frame.render_widget(hint, inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;

    fn make_state() -> TuiAppState {
        let mem: Arc<dyn agent_core::MemoryStore> = Arc::new(memory::MockMemoryStore::default());
        let evo = Arc::new(evolution::EvolutionEngine::new(0.01, mem));
        TuiAppState::new("render-test".into(), evo)
    }

    fn render_at_size(w: u16, h: u16) -> ratatui::Terminal<TestBackend> {
        let backend = TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = make_state();
        terminal.draw(|f| render_app(f, &state)).unwrap();
        terminal
    }

    // ── Small-window guard ──

    #[test]
    fn test_small_window_zero_width_does_not_panic() {
        // width=0 should return early without panic
        render_at_size(0, 20);
    }

    #[test]
    fn test_small_window_zero_height_does_not_panic() {
        // height=0 should return early without panic
        render_at_size(50, 0);
    }

    #[test]
    fn test_small_window_below_min_shows_warning() {
        // 39×9 is below MIN_WIDTH/MIN_HEIGHT — should render the warning message
        let terminal = render_at_size(39, 9);
        let buf = terminal.backend().buffer().clone();
        // Wide Chinese chars each occupy 2 columns so symbols have padding; strip spaces.
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            content.contains("终端太小"),
            "Expected 终端太小 in buffer (spaces stripped), got: {content}"
        );
    }

    #[test]
    fn test_normal_size_does_not_show_warning() {
        // 80×24 is well above minimum — should NOT show the size warning
        let terminal = render_at_size(80, 24);
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            !content.contains("终端太小"),
            "Should not show size warning at 80×24"
        );
    }
}
