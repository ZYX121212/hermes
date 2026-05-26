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
    AgentPhase, FocusedPanel, LeftTab, LogEntry, StepExecState, StepStatus, TuiAppState, TuiInput,
};

/// Main entry point for TUI mode.
pub async fn run_tui<A>(
    mut agent: A,
    ctx: agent_core::context::Context,
    event_rx: UnboundedReceiver<AgentEvent>,
    evolution: Arc<EvolutionEngine>,
    agent_name: String,
    tui_input: Arc<TuiInput>,
) -> anyhow::Result<A>
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
                // Lock buffer first to close TOCTOU window with agent thread
                let buffer = tui_input.buffer.lock();
                state.awaiting_input = tui_input
                    .awaiting
                    .load(std::sync::atomic::Ordering::Relaxed);
                state.input_text = if state.awaiting_input {
                    buffer.clone()
                } else {
                    String::new()
                };
                drop(buffer);
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

                        // ── Overlay mode (absorbs all keys) ──
                        if state.output_overlay.is_some() {
                            match key.code {
                                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                                    state.output_overlay = None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = o.scroll.saturating_sub(1);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = o.scroll.saturating_add(1);
                                    }
                                }
                                KeyCode::PageUp => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = o.scroll.saturating_sub(10);
                                    }
                                }
                                KeyCode::PageDown => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = o.scroll.saturating_add(10);
                                    }
                                }
                                KeyCode::Home => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = 0;
                                    }
                                }
                                KeyCode::End => {
                                    if let Some(ref mut o) = state.output_overlay {
                                        o.scroll = 10_000;
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }

                        // ── Help overlay absorbs keys ──
                        if state.help_visible {
                            match key.code {
                                KeyCode::Char('h') | KeyCode::Esc | KeyCode::F(1) => {
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
                                        let entry = state
                                            .input_history
                                            .get(pos)
                                            .cloned()
                                            .unwrap_or_default();
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
                                KeyCode::BackTab => {
                                    state.focused_panel = state.focused_panel.prev();
                                }
                                KeyCode::Tab => {
                                    if matches!(
                                        state.phase,
                                        AgentPhase::Planning | AgentPhase::Executing
                                    ) && state.focused_panel == FocusedPanel::MainLeft
                                        && !key.modifiers.contains(KeyModifiers::SHIFT)
                                    {
                                        state.left_tab = state.left_tab.next();
                                    } else if state.phase == AgentPhase::Idle
                                        && state.agent_done
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && !key.modifiers.contains(KeyModifiers::SHIFT)
                                    {
                                        state.results_visible = !state.results_visible;
                                    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.focused_panel = state.focused_panel.prev();
                                    } else {
                                        state.focused_panel = state.focused_panel.next();
                                    }
                                }
                                // Scroll focused panel
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                    {
                                        let len = state.executions.len();
                                        if len > 0 {
                                            let idx = state.exec_selected_index.unwrap_or(0);
                                            let new_idx = if idx > 0 { idx - 1 } else { 0 };
                                            state.exec_selected_index = Some(new_idx);
                                            if (new_idx as u16) < state.exec_scroll {
                                                state.exec_scroll = new_idx as u16;
                                            }
                                        }
                                    } else {
                                        scroll_focused(&mut state, -1);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                    {
                                        let len = state.executions.len();
                                        if len > 0 {
                                            let idx = state.exec_selected_index.unwrap_or(0);
                                            let new_idx =
                                                if idx + 1 < len { idx + 1 } else { len - 1 };
                                            state.exec_selected_index = Some(new_idx);
                                            if (new_idx as u16) >= state.exec_scroll + 8 {
                                                state.exec_scroll =
                                                    (new_idx as u16).saturating_sub(7);
                                            }
                                        }
                                    } else {
                                        scroll_focused(&mut state, 1);
                                    }
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
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                    {
                                        if let Some(idx) = state.exec_selected_index {
                                            if let Some(step) = state.executions.get(idx) {
                                                state.output_overlay =
                                                    Some(crate::state::StepOutputOverlay {
                                                        step_id: step.step_id,
                                                        tool: step.tool.clone(),
                                                        status: step.status.clone(),
                                                        duration_ms: step.duration_ms,
                                                        full_content: step
                                                            .content_full
                                                            .clone()
                                                            .unwrap_or_else(|| {
                                                                step.content_preview
                                                                    .clone()
                                                                    .unwrap_or_default()
                                                            }),
                                                        scroll: 0,
                                                    });
                                            }
                                        }
                                    } else if state.focused_panel == FocusedPanel::Evolution {
                                        toggle_evolution_section(&mut state);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(Event::Mouse(mouse)) => match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            scroll_mouse(&app_state, 1, mouse.column, mouse.row);
                        }
                        MouseEventKind::ScrollUp => {
                            scroll_mouse(&app_state, -1, mouse.column, mouse.row);
                        }
                        _ => {}
                    },
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
        state.results_visible = true;
        state.phase = AgentPhase::Idle;
        if let Err(ref e) = result {
            let message = format!("执行失败: {e}");
            state.log_entries.push_back(LogEntry {
                message: message.clone(),
                is_error: true,
            });
            state.summary = Some(message);
        } else if state.summary.is_none() {
            // Don't overwrite real summary with boilerplate
            state.summary = Some("完成 — 按 q 或 Esc 退出".into());
        }
    }

    match tui_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "TUI render task returned error"),
        Err(join_err) => tracing::error!(error = %join_err, "TUI render task panicked"),
    }

    result.map(|()| agent)
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
            AgentPhase::Planning | AgentPhase::Executing => {
                // Use left_tab to determine which content is visible
                match state.left_tab {
                    crate::state::LeftTab::Plan => {
                        state.plan_scroll = apply_delta(state.plan_scroll, delta);
                    }
                    crate::state::LeftTab::Execution => {
                        state.exec_scroll = apply_delta(state.exec_scroll, delta);
                    }
                }
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
            AgentPhase::Planning | AgentPhase::Executing => match state.left_tab {
                crate::state::LeftTab::Plan => state.plan_scroll = 0,
                crate::state::LeftTab::Execution => state.exec_scroll = 0,
            },
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
fn scroll_mouse(app_state: &Arc<parking_lot::RwLock<TuiAppState>>, delta: i16, col: u16, row: u16) {
    let mut state = app_state.write();
    let term_size = crossterm::terminal::size().unwrap_or((80, 24));

    let needs_mini_log = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let footer_h = 1;
    let mini_log_h = if needs_mini_log { 3 } else { 0 };
    let main_h = term_size.1.saturating_sub(header_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = state
        .phase
        .main_split_ratio(!state.evolution.all_weights().is_empty());

    let left_w = (term_size.0 as f64 * left_pct as f64 / 100.0) as u16;

    // Determine panel under cursor
    if row < header_h {
        return; // header, not scrollable
    }

    let in_main = row < header_h + main_h;
    let in_left = col < left_w;

    // Tab bar row during Planning/Executing is not scrollable
    if in_main && in_left && needs_mini_log && row == header_h {
        return;
    }

    if in_main && in_left {
        // Main left panel
        match state.phase {
            AgentPhase::Planning | AgentPhase::Executing => match state.left_tab {
                crate::state::LeftTab::Plan => {
                    state.plan_scroll = apply_delta(state.plan_scroll, delta);
                }
                crate::state::LeftTab::Execution => {
                    state.exec_scroll = apply_delta(state.exec_scroll, delta);
                }
            },
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
            state.summary_streaming_buffer.clear();
            state.plan_steps_count = 0;
            state.plan_ready = false;
            state.executions.clear();
            state.exec_total_steps = 0;
            state.exec_completed_steps = 0;
            state.summary = None;
            state.plan_scroll = 0;
            state.exec_scroll = 0;
            state.left_tab = LeftTab::Execution;
            state.exec_selected_index = None;
            state.results_visible = true;
        }
        AgentEvent::PlanPhaseStarted => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.plan_ready = false;
            state.left_tab = LeftTab::Plan;
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
            state.exec_selected_index = None;
            state.left_tab = LeftTab::Execution;
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
            if state.exec_selected_index.is_none() {
                state.exec_selected_index = Some(state.executions.len().saturating_sub(1));
            }
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
                let clean = crate::state::strip_ansi(&output.content);
                step.content_full = Some({
                    let limit = 10_000; // 10KB upper bound
                    if clean.len() > limit {
                        // Find safe char boundary to avoid panicking on multi-byte UTF-8
                        let mut end = limit;
                        while end > 0 && !clean.is_char_boundary(end) {
                            end -= 1;
                        }
                        let mut s = clean[..end].to_string();
                        s.push_str("…[truncated]");
                        s
                    } else {
                        clean.clone()
                    }
                });
                step.content_preview = Some(crate::state::truncate(&clean, 100));
            }
            // Recompute completed count
            state.exec_completed_steps = state
                .executions
                .iter()
                .filter(|s| s.status != StepStatus::Pending && s.status != StepStatus::Running)
                .count();
            // Add step result to log for persistent visibility
            let status_icon = if output.success { "✓" } else { "✗" };
            let preview = crate::state::truncate(&crate::state::strip_ansi(&output.content), 80);
            state.log_entries.push_back(LogEntry {
                message: format!(
                    "{} {} ({:.1}s): {}",
                    status_icon,
                    output
                        .step_id
                        .to_string()
                        .chars()
                        .take(8)
                        .collect::<String>(),
                    output.duration_ms as f64 / 1000.0,
                    preview,
                ),
                is_error: !output.success,
            });
        }
        AgentEvent::ExecutePhaseComplete { duration_ms, .. } => {
            state.phase = AgentPhase::Reflecting;
            state.total_duration_ms = Some(duration_ms);
            state.log_entries.push_back(LogEntry {
                message: format!("执行完成 ({:.1}s)", duration_ms as f64 / 1000.0),
                is_error: false,
            });
        }
        AgentEvent::SubAgentStarted { task } => {
            state.log_entries.push_back(LogEntry {
                message: format!("子任务开始: {}", task),
                is_error: false,
            });
        }
        AgentEvent::SubAgentCompleted { task, summary } => {
            state.log_entries.push_back(LogEntry {
                message: format!("子任务完成: {} → {}", task, summary),
                is_error: false,
            });
        }
        AgentEvent::ReplanNeeded { reason, attempt } => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.plan_ready = false;
            state.left_tab = LeftTab::Plan;
            state.log_entries.push_back(LogEntry {
                message: format!("重规划 #{attempt}: {reason}"),
                is_error: false,
            });
        }
        AgentEvent::ReplanComplete { new_steps_count } => {
            state.plan_steps_count = new_steps_count;
            state.plan_ready = true;
            state.log_entries.push_back(LogEntry {
                message: format!("重规划完成: {new_steps_count} 个步骤"),
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
        AgentEvent::SummaryStreamingToken { token } => {
            state.summary_streaming_buffer.push_str(&token);
        }
        AgentEvent::SummaryReady { summary } => {
            state.summary_streaming_buffer.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TuiAppState;
    use agent_core::AgentEvent;
    use std::sync::Arc;

    fn make_state() -> TuiAppState {
        let mem: Arc<dyn agent_core::MemoryStore> = Arc::new(memory::MockMemoryStore::default());
        let evo = Arc::new(evolution::EvolutionEngine::new(0.01, mem));
        TuiAppState::new("test".into(), evo)
    }

    // ── apply_delta ──

    #[test]
    fn test_apply_delta_positive() {
        assert_eq!(apply_delta(5, 3), 8);
    }

    #[test]
    fn test_apply_delta_negative() {
        assert_eq!(apply_delta(5, -3), 2);
    }

    #[test]
    fn test_apply_delta_saturating_zero() {
        assert_eq!(apply_delta(2, -5), 0);
    }

    #[test]
    fn test_apply_delta_saturating_max() {
        assert_eq!(apply_delta(u16::MAX, 5), u16::MAX);
    }

    #[test]
    fn test_apply_delta_zero() {
        assert_eq!(apply_delta(10, 0), 10);
    }

    // ── handle_event: TurnStarted resets state ──

    #[test]
    fn test_turn_started_resets_execution() {
        let mut state = make_state();
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            content_preview: Some("output".into()),
            content_full: Some("full output".into()),
            duration_ms: Some(100),
            layer: 0,
        });
        state.exec_total_steps = 1;
        state.exec_completed_steps = 1;
        state.streaming_buffer = "plan content".into();
        state.plan_ready = true;
        state.plan_steps_count = 3;

        handle_event(&mut state, AgentEvent::TurnStarted { turn: 2 });

        assert_eq!(state.turn, 2);
        assert_eq!(state.phase, AgentPhase::Observing);
        assert!(state.executions.is_empty());
        assert_eq!(state.exec_total_steps, 0);
        assert_eq!(state.exec_completed_steps, 0);
        assert!(state.streaming_buffer.is_empty());
        assert!(!state.plan_ready);
        assert_eq!(state.plan_steps_count, 0);
        assert_eq!(state.left_tab, LeftTab::Execution);
        assert!(state.exec_selected_index.is_none());
        assert!(state.results_visible);
    }

    // ── handle_event: PlanPhaseStarted switches tab ──

    #[test]
    fn test_plan_phase_started_switches_tab() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Planning);
        assert_eq!(state.left_tab, LeftTab::Plan);
    }

    // ── handle_event: ExecutePhaseStarted switches tab back ──

    #[test]
    fn test_execute_phase_started_switches_tab() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 5 },
        );
        assert_eq!(state.phase, AgentPhase::Executing);
        assert_eq!(state.left_tab, LeftTab::Execution);
        assert_eq!(state.exec_total_steps, 5);
        assert!(state.executions.is_empty());
        assert!(state.exec_selected_index.is_none());
    }

    // ── handle_event: StepStarted + StepCompleted ──

    #[test]
    fn test_step_lifecycle() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 1 },
        );

        let step_id = uuid::Uuid::new_v4();
        handle_event(
            &mut state,
            AgentEvent::StepStarted {
                step_id,
                tool: "bash".into(),
                layer: 0,
            },
        );

        assert_eq!(state.executions.len(), 1);
        assert_eq!(
            state.executions[0].status,
            crate::state::StepStatus::Running
        );
        assert_eq!(state.exec_selected_index, Some(0));

        handle_event(
            &mut state,
            AgentEvent::StepCompleted {
                output: agent_core::StepOutput {
                    step_id,
                    tool: "bash".into(),
                    success: true,
                    content: "done".into(),
                    duration_ms: 42,
                },
            },
        );

        assert_eq!(
            state.executions[0].status,
            crate::state::StepStatus::Success
        );
        assert_eq!(state.executions[0].duration_ms, Some(42));
        assert!(state.executions[0].content_full.is_some());
        assert!(state.executions[0].content_preview.is_some());
        assert_eq!(state.exec_completed_steps, 1);
        // Should also add a log entry
        assert!(!state.log_entries.is_empty());
    }

    // ── handle_event: ExecutePhaseComplete sets total_duration ──

    #[test]
    fn test_execute_phase_complete_sets_duration() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 1 },
        );
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseComplete {
                all_success: true,
                duration_ms: 500,
            },
        );
        assert_eq!(state.phase, AgentPhase::Reflecting);
        assert_eq!(state.total_duration_ms, Some(500));
    }

    // ── handle_event: log buffer pruning ──

    #[test]
    fn test_log_buffer_pruning() {
        let mut state = make_state();
        // Fill log beyond 200 entries
        for i in 0..250 {
            handle_event(
                &mut state,
                AgentEvent::AgentError {
                    message: format!("error {i}"),
                },
            );
        }
        assert!(state.log_entries.len() <= 200);
        // Oldest entries should be gone
        assert!(!state
            .log_entries
            .front()
            .unwrap()
            .message
            .contains("error 0"));
    }

    // ── scroll helpers ──

    #[test]
    fn test_scroll_focused_main_left_planning_plan_tab() {
        let mut state = make_state();
        state.phase = AgentPhase::Planning;
        state.focused_panel = FocusedPanel::MainLeft;
        state.left_tab = LeftTab::Plan;
        state.plan_scroll = 10;
        scroll_focused(&mut state, 5);
        assert_eq!(state.plan_scroll, 15);
        scroll_focused(&mut state, -3);
        assert_eq!(state.plan_scroll, 12);
    }

    #[test]
    fn test_scroll_focused_main_left_executing_exec_tab() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        state.focused_panel = FocusedPanel::MainLeft;
        state.left_tab = LeftTab::Execution;
        state.exec_scroll = 5;
        scroll_focused(&mut state, 3);
        assert_eq!(state.exec_scroll, 8);
    }

    #[test]
    fn test_scroll_focused_evolution() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 3;
        scroll_focused(&mut state, 2);
        assert_eq!(state.evo_scroll, 5);
    }

    // ── AgentError adds to log ──

    #[test]
    fn test_agent_error_adds_log_entry() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::AgentError {
                message: "test error".into(),
            },
        );
        assert_eq!(state.log_entries.len(), 1);
        assert!(state.log_entries[0].is_error);
        assert_eq!(state.log_entries[0].message, "test error");
    }

    // ── SummaryReady ──

    #[test]
    fn test_summary_ready_sets_summary() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SummaryReady {
                summary: "all done".into(),
            },
        );
        assert_eq!(state.summary.as_deref(), Some("all done"));
    }
}
