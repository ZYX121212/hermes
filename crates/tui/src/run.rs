// crates/tui/src/run.rs
// TUI orchestrator: terminal lifecycle, event drain+dispatch, render loop, keyboard + mouse input.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use agent_core::AgentEvent;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind};
use crossterm::ExecutableCommand;
use evolution::EvolutionEngine;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::state::{
    AgentPhase, FocusedPanel, LogEntry, StepExecState, StepStatus, TuiAppState, TuiInput,
};

/// Main entry point for TUI mode.
pub async fn run_tui<A>(
    mut agent: A,
    ctx: agent_core::context::Context,
    event_rx: UnboundedReceiver<AgentEvent>,
    evolution: Arc<EvolutionEngine>,
    agent_name: String,
    tui_input: Arc<TuiInput>,
) -> anyhow::Result<()>
where
    A: agent_core::agent::HermesAgent + Send + 'static,
{
    // ── Terminal setup ──
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = io::stdout().execute(crossterm::terminal::LeaveAlternateScreen);
        original_hook(info);
    }));

    crossterm::terminal::enable_raw_mode()?;
    io::stdout().execute(crossterm::terminal::EnterAlternateScreen)?;
    io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // ── Shared state ──
    let app_state = Arc::new(parking_lot::RwLock::new(TuiAppState::new(
        agent_name.clone(),
        Arc::clone(&evolution),
    )));

    let rx = Arc::new(parking_lot::Mutex::new(event_rx));
    let rx_clone = Arc::clone(&rx);
    let app_state_clone = Arc::clone(&app_state);
    let tui_input_clone = Arc::clone(&tui_input);

    // ── Spawn render+input loop ──
    let tui_task = tokio::task::spawn_blocking(move || {
        let rx = rx_clone;
        let app_state = app_state_clone;
        let tui_input = tui_input_clone;

        loop {
            // 1. Drain pending agent events
            {
                let mut rx_lock = rx.lock();
                let mut state = app_state.write();
                while let Ok(event) = rx_lock.try_recv() {
                    handle_event(&mut state, event);
                }
                state.awaiting_input =
                    tui_input.awaiting.load(std::sync::atomic::Ordering::Relaxed);
                if state.awaiting_input {
                    state.input_text = tui_input.buffer.lock().clone();
                }
                state.frame_count += 1;
            }

            // 2. Render frame
            {
                let state = app_state.read();
                if let Err(e) = terminal.draw(|f| crate::render::render_app(f, &state)) {
                    tracing::error!(error = %e, "Terminal draw failed");
                    drop(state);
                    let mut state = app_state.write();
                    state.should_quit = true;
                    state.log_entries.push_back(LogEntry {
                        message: format!("Terminal render error: {e}"),
                        is_error: true,
                    });
                    break;
                }
                if state.should_quit {
                    break;
                }
            }

            // 3. Check for input (~30fps)
            if crossterm::event::poll(Duration::from_millis(33)).unwrap_or(false) {
                match crossterm::event::read() {
                    Ok(Event::Key(key)) => {
                        let mut state = app_state.write();

                        // ── Help overlay absorbs keys ──
                        if state.help_visible {
                            match key.code {
                                KeyCode::Char('h')
                                | KeyCode::Esc
                                | KeyCode::F(1) => {
                                    state.help_visible = false;
                                }
                                _ => {} // ignore other keys while help visible
                            }
                            continue;
                        }

                        // ── Input mode ──
                        if state.awaiting_input {
                            match key.code {
                                KeyCode::Enter => {
                                    let mut buffer = tui_input.buffer.lock();
                                    let text = buffer.clone();
                                    // Push to history
                                    if !text.is_empty() {
                                        if state.input_history.len() >= 50 {
                                            state.input_history.pop_front();
                                        }
                                        state.input_history.push_back(text.clone());
                                    }
                                    state.input_history_pos = None;
                                    buffer.clear();
                                    *tui_input.submitted.lock() = Some(text);
                                }
                                KeyCode::Backspace => {
                                    tui_input.buffer.lock().pop();
                                }
                                KeyCode::Up => {
                                    // Navigate input history backward
                                    let hist_len = state.input_history.len();
                                    if hist_len > 0 {
                                        let pos = state
                                            .input_history_pos
                                            .map(|p| if p > 0 { p - 1 } else { 0 })
                                            .unwrap_or(hist_len - 1);
                                        let entry =
                                            state.input_history.get(pos).cloned().unwrap_or_default();
                                        *tui_input.buffer.lock() = entry;
                                        state.input_history_pos = Some(pos);
                                    }
                                }
                                KeyCode::Down => {
                                    let hist_len = state.input_history.len();
                                    if let Some(pos) = state.input_history_pos {
                                        if pos + 1 < hist_len {
                                            let new_pos = pos + 1;
                                            let entry = state
                                                .input_history
                                                .get(new_pos)
                                                .cloned()
                                                .unwrap_or_default();
                                            *tui_input.buffer.lock() = entry;
                                            state.input_history_pos = Some(new_pos);
                                        } else {
                                            state.input_history_pos = None;
                                            tui_input.buffer.lock().clear();
                                        }
                                    }
                                }
                                KeyCode::Char(c) => {
                                    tui_input.buffer.lock().push(c);
                                }
                                _ => {}
                            }
                        }
                        // ── Normal mode ──
                        else {
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                                    state.should_quit = true;
                                }
                                KeyCode::Char('c') | KeyCode::Char('C')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    state.should_quit = true;
                                }
                                KeyCode::Char('h') | KeyCode::F(1) => {
                                    state.help_visible = !state.help_visible;
                                }
                                KeyCode::Tab => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.focused_panel = state.focused_panel.prev();
                                    } else {
                                        state.focused_panel = state.focused_panel.next();
                                    }
                                }
                                // Scroll focused panel
                                KeyCode::Up | KeyCode::Char('k') => {
                                    scroll_focused(&mut state, -1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    scroll_focused(&mut state, 1);
                                }
                                KeyCode::PageUp => {
                                    page_scroll_focused(&mut state, -10);
                                }
                                KeyCode::PageDown => {
                                    page_scroll_focused(&mut state, 10);
                                }
                                KeyCode::Home => {
                                    scroll_to_top(&mut state);
                                }
                                KeyCode::End => {
                                    scroll_to_bottom(&mut state);
                                }
                                // Evolution panel: Enter toggles section collapse
                                KeyCode::Enter => {
                                    if state.focused_panel == FocusedPanel::Evolution {
                                        toggle_evolution_section(&mut state);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(Event::Mouse(mouse)) => {
                        match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                scroll_mouse(&app_state, 1, mouse.column, mouse.row);
                            }
                            MouseEventKind::ScrollUp => {
                                scroll_mouse(&app_state, -1, mouse.column, mouse.row);
                            }
                            _ => {}
                        }
                    }
                    Ok(Event::Resize(..)) => {
                        // Redraw on next frame with new dimensions
                    }
                    _ => {}
                }
            }
        }

        // ── Terminal cleanup ──
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = io::stdout().execute(crossterm::event::DisableMouseCapture);
        let _ = io::stdout().execute(crossterm::terminal::LeaveAlternateScreen);
        Ok::<_, anyhow::Error>(())
    });

    // ── Run agent ──
    let result = agent.run_loop(ctx).await;

    // Signal completion — preserve real summary from agent output
    {
        let mut state = app_state.write();
        state.agent_done = true;
        state.phase = AgentPhase::Idle;
        // Don't overwrite real summary with boilerplate
        if state.summary.is_none() {
            state.summary = Some("完成 — 按 q 或 Esc 退出".into());
        }
    }

    match tui_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "TUI render task returned error"),
        Err(join_err) => tracing::error!(error = %join_err, "TUI render task panicked"),
    }

    result
}

// ── Scroll helpers ──

fn apply_delta(current: u16, delta: i16) -> u16 {
    if delta > 0 {
        current.saturating_add(delta as u16)
    } else {
        current.saturating_sub((-delta) as u16)
    }
}

fn scroll_focused(state: &mut TuiAppState, delta: i16) {
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning => {
                state.plan_scroll = apply_delta(state.plan_scroll, delta);
            }
            AgentPhase::Executing => {
                state.exec_scroll = apply_delta(state.exec_scroll, delta);
            }
            _ => {
                state.log_scroll = apply_delta(state.log_scroll, delta);
            }
        },
        FocusedPanel::Evolution => {
            state.evo_scroll = apply_delta(state.evo_scroll, delta);
        }
        FocusedPanel::MiniLog => {
            state.log_scroll = apply_delta(state.log_scroll, delta);
        }
    }
}

fn page_scroll_focused(state: &mut TuiAppState, delta: i16) {
    scroll_focused(state, delta);
}

fn scroll_to_top(state: &mut TuiAppState) {
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning => state.plan_scroll = 0,
            AgentPhase::Executing => state.exec_scroll = 0,
            _ => state.log_scroll = 0,
        },
        FocusedPanel::Evolution => state.evo_scroll = 0,
        FocusedPanel::MiniLog => state.log_scroll = 0,
    }
}

fn scroll_to_bottom(state: &mut TuiAppState) {
    // Use a large delta instead of u16::MAX to avoid breaking
    // ratatui's Paragraph::scroll (which skips ALL content lines
    // when the offset exceeds the line count).
    let delta = 10_000_i16;
    scroll_focused(state, delta);
}

fn toggle_evolution_section(state: &mut TuiAppState) {
    // Cycle: hide weights → hide stats → hide meta → unhide all
    if !state.evo_weights_hidden {
        state.evo_weights_hidden = true;
    } else if !state.evo_stats_hidden {
        state.evo_stats_hidden = true;
    } else if !state.evo_meta_hidden {
        state.evo_meta_hidden = true;
    } else {
        // All hidden — unhide all
        state.evo_stats_hidden = false;
        state.evo_weights_hidden = false;
        state.evo_meta_hidden = false;
    }
}

/// Simple mouse scroll: determine which panel the cursor is over and scroll it.
/// Uses a single write lock to avoid TOCTOU between layout computation and scroll update.
fn scroll_mouse(
    app_state: &Arc<parking_lot::RwLock<TuiAppState>>,
    delta: i16,
    col: u16,
    row: u16,
) {
    let mut state = app_state.write();
    let term_size = crossterm::terminal::size().unwrap_or((80, 24));

    let needs_mini_log =
        matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let footer_h = 1;
    let mini_log_h = if needs_mini_log { 3 } else { 0 };
    let main_h = term_size.1.saturating_sub(header_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = {
        let has_weights = !state.evolution.all_weights().is_empty();
        match (state.phase, has_weights) {
            (AgentPhase::Planning, false) => (85, 15),
            (AgentPhase::Planning, true) => (75, 25),
            (AgentPhase::Executing, false) => (80, 20),
            (AgentPhase::Executing, true) => (70, 30),
            (_, false) => (75, 25),
            (_, true) => (60, 40),
        }
    };

    let left_w = (term_size.0 as f64 * left_pct as f64 / 100.0) as u16;

    // Determine panel under cursor
    if row < header_h {
        return; // header, not scrollable
    }

    let in_main = row < header_h + main_h;
    let in_left = col < left_w;

    if in_main && in_left {
        // Main left panel
        match state.phase {
            AgentPhase::Planning => {
                state.plan_scroll = apply_delta(state.plan_scroll, delta);
            }
            AgentPhase::Executing => {
                state.exec_scroll = apply_delta(state.exec_scroll, delta);
            }
            _ => {
                state.log_scroll = apply_delta(state.log_scroll, delta);
            }
        }
    } else if in_main && !in_left {
        // Evolution panel
        state.evo_scroll = apply_delta(state.evo_scroll, delta);
    } else if needs_mini_log {
        // Mini-log area
        state.log_scroll = apply_delta(state.log_scroll, delta);
    }
}

// ── Agent event handler ──

fn handle_event(state: &mut TuiAppState, event: AgentEvent) {
    match event {
        AgentEvent::AgentStarted { name } => {
            state.agent_name = name;
        }
        AgentEvent::AgentStopped => {
            state.phase = AgentPhase::Idle;
        }
        AgentEvent::TurnStarted { turn } => {
            state.turn = turn;
            state.phase = AgentPhase::Observing;
            state.streaming_buffer.clear();
            state.plan_steps_count = 0;
            state.plan_ready = false;
            state.executions.clear();
            state.exec_total_steps = 0;
            state.exec_completed_steps = 0;
            state.summary = None;
            state.plan_scroll = 0;
            state.exec_scroll = 0;
        }
        AgentEvent::PlanPhaseStarted => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.plan_ready = false;
        }
        AgentEvent::PlanStreamingToken { token } => {
            state.streaming_buffer.push_str(&token);
        }
        AgentEvent::PlanReady { steps_count } => {
            state.plan_steps_count = steps_count;
            state.plan_ready = true;
        }
        AgentEvent::PlanRetry => {
            state.streaming_buffer.push_str("\n[重试规划...]\n");
        }
        AgentEvent::ExecutePhaseStarted { total_steps } => {
            state.phase = AgentPhase::Executing;
            state.executions.clear();
            state.exec_total_steps = total_steps;
            state.exec_completed_steps = 0;
            state.exec_scroll = 0;
        }
        AgentEvent::StepStarted {
            step_id,
            tool,
            layer,
        } => {
            state.executions.push(StepExecState {
                step_id,
                tool,
                status: StepStatus::Running,
                content_preview: None,
                content_full: None,
                duration_ms: None,
                layer,
            });
        }
        AgentEvent::StepCompleted { output } => {
            if let Some(step) = state
                .executions
                .iter_mut()
                .find(|s| s.step_id == output.step_id)
            {
                step.status = if output.success {
                    StepStatus::Success
                } else {
                    StepStatus::Failed
                };
                step.duration_ms = Some(output.duration_ms);
                step.content_preview = Some(crate::state::truncate(
                    &crate::state::strip_ansi(&output.content),
                    100,
                ));
            }
            // Recompute completed count
            state.exec_completed_steps = state
                .executions
                .iter()
                .filter(|s| s.status != StepStatus::Pending && s.status != StepStatus::Running)
                .count();
            // Add step result to log for persistent visibility
            let status_icon = if output.success { "✓" } else { "✗" };
            let preview = crate::state::truncate(
                &crate::state::strip_ansi(&output.content),
                80,
            );
            state.log_entries.push_back(LogEntry {
                message: format!(
                    "{} {} ({:.1}s): {}",
                    status_icon,
                    output.step_id.to_string().chars().take(8).collect::<String>(),
                    output.duration_ms as f64 / 1000.0,
                    preview,
                ),
                is_error: !output.success,
            });
        }
        AgentEvent::ExecutePhaseComplete { duration_ms, .. } => {
            state.phase = AgentPhase::Reflecting;
            state.log_entries.push_back(LogEntry {
                message: format!("执行完成 ({:.1}s)", duration_ms as f64 / 1000.0),
                is_error: false,
            });
        }
        AgentEvent::ReflectPhaseStarted => {
            state.phase = AgentPhase::Reflecting;
        }
        AgentEvent::ReflectPhaseComplete { score, lesson } => {
            state.phase = AgentPhase::Evolving;
            state.log_entries.push_back(LogEntry {
                message: format!("反思: score={:.2} | {}", score, lesson),
                is_error: score < 0.0,
            });
        }
        AgentEvent::EvolvePhaseStarted => {
            state.phase = AgentPhase::Evolving;
        }
        AgentEvent::EvolvePhaseComplete => {
            state.phase = AgentPhase::Idle;
        }
        AgentEvent::SummaryReady { summary } => {
            state.log_entries.push_back(LogEntry {
                message: format!("结果: {}", summary),
                is_error: false,
            });
            state.summary = Some(summary);
        }
        AgentEvent::AgentError { message } => {
            state.log_entries.push_back(LogEntry {
                message,
                is_error: true,
            });
        }
    }

    // Prune log buffer
    while state.log_entries.len() > 200 {
        state.log_entries.pop_front();
    }
}
