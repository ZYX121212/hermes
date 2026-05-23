// crates/tui/src/render.rs
// Main render function: phase-adaptive layout with focus-aware borders.

use ratatui::layout::{Constraint, Layout};
use ratatui::Frame;

use crate::panels;
use crate::state::{AgentPhase, FocusedPanel, TuiAppState};

pub fn render_app(frame: &mut Frame, state: &TuiAppState) {
    let area = frame.area();

    // ── Vertical split: header (1), main (fill), mini-log (opt), footer (1) ──
    let needs_mini_log = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let v_chunks = if needs_mini_log {
        Layout::vertical([
            Constraint::Length(1),  // header
            Constraint::Min(1),     // main
            Constraint::Length(3),  // mini-log
            Constraint::Length(1),  // footer
        ])
        .split(area)
    } else {
        Layout::vertical([
            Constraint::Length(1),  // header
            Constraint::Min(1),     // main
            Constraint::Length(1),  // footer
        ])
        .split(area)
    };

    let header_area = v_chunks[0];
    let main_area = v_chunks[1];
    let footer_area = if needs_mini_log { v_chunks[3] } else { v_chunks[2] };

    // ── Main area horizontal split based on phase ──
    let has_weights = !state.evolution.all_weights().is_empty();
    let (left_pct, right_pct) = state.phase.main_split_ratio(has_weights);

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
            use ratatui::style::{Color, Modifier, Style};
            use ratatui::text::{Line, Span};
            use ratatui::widgets::Paragraph;

            use crate::state::LeftTab;

            // Split left area: tab bar (1) + content (fill)
            let left_chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
            ]).split(left_area);

            let tab_area = left_chunks[0];
            let content_area = left_chunks[1];

            // Render tab bar
            let plan_style = if state.left_tab == LeftTab::Plan {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Color::Gray)
            };
            let exec_style = if state.left_tab == LeftTab::Execution {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Color::Gray)
            };
            let tabs_text = Line::from(vec![
                Span::styled("[Plan]", plan_style),
                Span::raw(" "),
                Span::styled("[Exec]", exec_style),
            ]);
            frame.render_widget(Paragraph::new(tabs_text), tab_area);

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
            panels::log::render_log(
                frame,
                left_area,
                state,
                state.focused_panel == FocusedPanel::MainLeft,
            );
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

    // ── Render footer (or input bar when awaiting input) ──
    if state.awaiting_input {
        panels::input::render_input(frame, footer_area, state);
    } else {
        panels::footer::render_footer(frame, footer_area, state);
    }

    // ── Render help overlay (on top of everything) ──
    if state.help_visible {
        panels::help::render_help(frame, area, state);
    }
}
