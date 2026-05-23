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
    let (left_pct, right_pct) = match (state.phase, has_weights) {
        (AgentPhase::Planning, false) => (85, 15),
        (AgentPhase::Planning, true) => (75, 25),
        (AgentPhase::Executing, false) => (80, 20),
        (AgentPhase::Executing, true) => (70, 30),
        (_, false) => (75, 25),
        (_, true) => (60, 40),
    };

    let h_chunks = Layout::horizontal([
        Constraint::Percentage(left_pct),
        Constraint::Percentage(right_pct),
    ])
    .split(main_area);

    let left_area = h_chunks[0];
    let right_area = h_chunks[1];

    // ── Render left panel based on phase ──
    match state.phase {
        AgentPhase::Planning => {
            panels::plan::render_plan(
                frame,
                left_area,
                state,
                state.focused_panel == FocusedPanel::MainLeft,
            );
        }
        AgentPhase::Executing => {
            panels::execution::render_execution(
                frame,
                left_area,
                state,
                state.focused_panel == FocusedPanel::MainLeft,
            );
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
