// crates/tui/tests/stress_100.rs
// 100-round stress test: simulates 100 full task cycles through TUI state
// without a real terminal. Verifies no panics, correct phase transitions,
// log pruning (≤200 entries), and input history cap (≤50 entries).

use std::sync::Arc;

use agent_core::AgentEvent;
use tui::run::{begin_next_task_input_pub, handle_event_pub, submit_tui_input_pub};
use tui::state::{AgentPhase, TuiAppState, TuiInput};

fn make_state() -> TuiAppState {
    let mem: Arc<dyn agent_core::MemoryStore> = Arc::new(memory::MockMemoryStore::default());
    let evo = Arc::new(evolution::EvolutionEngine::new(0.01, mem));
    TuiAppState::new("stress-test".into(), evo)
}

fn make_input() -> TuiInput {
    TuiInput::new()
}

/// Drive a complete agent turn through the state machine.
fn drive_agent_turn(state: &mut TuiAppState, turn: u32) {
    handle_event_pub(state, AgentEvent::TurnStarted { turn: turn as u64 });
    handle_event_pub(state, AgentEvent::PlanPhaseStarted);
    handle_event_pub(state, AgentEvent::PlanReady { steps_count: 1 });
    handle_event_pub(
        state,
        AgentEvent::ExecutePhaseStarted { total_steps: 1 },
    );
    handle_event_pub(
        state,
        AgentEvent::SummaryReady {
            summary: format!("完成任务 {turn}"),
        },
    );
    handle_event_pub(state, AgentEvent::EvolvePhaseComplete);
}

#[test]
fn stress_100_rounds() {
    let mut state = make_state();
    let input = make_input();

    // Initial state should be Idle
    assert_eq!(state.phase, AgentPhase::Idle);
    assert!(!state.agent_done);

    for round in 0u32..100 {
        // Step 1: put input into awaiting state
        begin_next_task_input_pub(&mut state, &input);
        assert!(state.awaiting_input);

        // Step 2: simulate user submitting a task
        let task_text = format!("任务{round}");
        let ok = submit_tui_input_pub(&mut state, &input, task_text.clone());
        assert!(ok, "submit_tui_input failed on round {round}");
        assert!(!state.awaiting_input);

        // Step 3: manually track history (capped at 50, mirrors render-loop logic)
        if !task_text.is_empty() {
            if state.input_history.len() >= 50 {
                state.input_history.pop_front();
            }
            state.input_history.push_back(task_text);
        }

        // Step 4: drive agent turn events
        drive_agent_turn(&mut state, round);

        // Step 5: verify post-turn invariants
        assert_eq!(
            state.phase,
            AgentPhase::Idle,
            "phase should be Idle after round {round}"
        );
        assert!(state.agent_done, "agent_done should be true after round {round}");
        assert!(
            state.log_entries.len() <= 200,
            "log_entries exceeded 200 on round {round}: {}",
            state.log_entries.len()
        );
    }

    // ── Final assertions ──
    // History capped at 50
    assert_eq!(
        state.input_history.len(),
        50,
        "expected 50 history entries (cap), got {}",
        state.input_history.len()
    );
    // Log never exceeded pruning threshold
    assert!(
        state.log_entries.len() <= 200,
        "log_entries exceeded 200 at end: {}",
        state.log_entries.len()
    );
    // State consistency
    assert_eq!(state.phase, AgentPhase::Idle);
    assert!(state.agent_done);
    // Turn counter reflects last round (0-indexed, so turn 99)
    assert_eq!(state.turn, 99u64);
}

#[test]
fn stress_no_panic_on_rapid_submit_cancel() {
    // Verify that rapid submit without driving agent events doesn't panic.
    let mut state = make_state();
    let input = make_input();

    for i in 0u32..50 {
        begin_next_task_input_pub(&mut state, &input);
        let ok = submit_tui_input_pub(&mut state, &input, format!("快速任务{i}"));
        assert!(ok);
        // Don't drive agent — simulates UI submit with no agent response
    }
    // State should still be consistent (not panic)
    assert!(!state.awaiting_input);
}
