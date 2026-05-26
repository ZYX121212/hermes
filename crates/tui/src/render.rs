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

pub fn render_app(frame: &mut Frame, state: &TuiAppState) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);

    // ── Vertical split: header (1), main (fill), mini-log (opt), footer (1) ──
    let needs_mini_log = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let v_chunks = if needs_mini_log {
        Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // main
            Constraint::Length(3), // mini-log
            Constraint::Length(1), // footer
        ])
        .split(area)
    } else {
        Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // main
            Constraint::Length(1), // footer
        ])
        .split(area)
    };

    let header_area = v_chunks[0];
    let main_area = v_chunks[1];
    let footer_area = if needs_mini_log {
        v_chunks[3]
    } else {
        v_chunks[2]
    };

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
            if state.agent_done && state.results_visible {
                panels::results::render_results(
                    frame,
                    left_area,
                    state,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
            } else {
                panels::log::render_log(
                    frame,
                    left_area,
                    state,
                    state.focused_panel == FocusedPanel::MainLeft,
                );
            }
        }
    }

    // ── Render evolution panel (always on the right) ──
    panels::evolution::render_evolution(
        frame,
        right_area,
        state,
        state.focused_panel == FocusedPanel::Evolution,
    );

    // ── Render mini-log during Planning/Executing ──
    if needs_mini_log {
        panels::log::render_mini_log(
            frame,
            v_chunks[2],
            state,
            state.focused_panel == FocusedPanel::MiniLog,
        );
    }

    // ── Render header ──
    panels::header::render_header(frame, header_area, state);

    // ── Render footer / input bar ──
    // Always show input bar in TUI mode (footer hints are inline in the input bar).
    let input_focused = state.focused_panel == FocusedPanel::Input;
    panels::input::render_input(frame, footer_area, state, input_focused);

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
}
