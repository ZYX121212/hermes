// crates/tui/src/run.rs
// TUI orchestrator: terminal lifecycle, event drain+dispatch, render loop, keyboard + mouse input.

use std::io::{self, Write};
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
                let cursor = tui_input.cursor.lock();
                let was_awaiting = state.awaiting_input;
                state.awaiting_input = tui_input
                    .awaiting
                    .load(std::sync::atomic::Ordering::Relaxed);
                // Auto-focus input when agent starts waiting for user input
                if state.awaiting_input && !was_awaiting {
                    state.focused_panel = FocusedPanel::Input;
                }
                state.input_text = if state.awaiting_input {
                    buffer.clone()
                } else {
                    String::new()
                };
                state.input_cursor = *cursor;
                drop(cursor);
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
                    push_log(&mut *state, format!("Terminal render error: {e}"), true);
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
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    state.output_overlay = None;
                                }
                                KeyCode::Enter => {
                                    // Close overlay but keep selection
                                    state.output_overlay = None;
                                }
                                KeyCode::Left
                                | KeyCode::Char('h')
                                | KeyCode::Char('p') => {
                                    // Previous step
                                    let len = state.executions.len();
                                    if len > 0 {
                                        let idx = state
                                            .exec_selected_index
                                            .unwrap_or(0);
                                        if idx > 0 {
                                            let new_idx = idx - 1;
                                            open_step_overlay(&mut *state, new_idx);
                                        }
                                    }
                                }
                                KeyCode::Right
                                | KeyCode::Char('l')
                                | KeyCode::Char('n') => {
                                    // Next step
                                    let len = state.executions.len();
                                    if len > 0 {
                                        let idx = state
                                            .exec_selected_index
                                            .unwrap_or(0);
                                        if idx + 1 < len {
                                            let new_idx = idx + 1;
                                            open_step_overlay(&mut *state, new_idx);
                                        }
                                    }
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

                        // ── Settings overlay absorbs keys ──
                        if state.settings_visible {
                            match key.code {
                                KeyCode::Char('s')
                                | KeyCode::F(2)
                                | KeyCode::Esc
                                | KeyCode::Char('q') => {
                                    state.settings_visible = false;
                                }
                                KeyCode::Char('1') => {
                                    state.gateway_mode = "cost-first".to_string();
                                    tui_input.set_gateway_mode("cost-first");
                                }
                                KeyCode::Char('2') => {
                                    state.gateway_mode = "quality-first".to_string();
                                    tui_input.set_gateway_mode("quality-first");
                                }
                                KeyCode::Char('3') => {
                                    state.gateway_mode = "latency-first".to_string();
                                    tui_input.set_gateway_mode("latency-first");
                                }
                                _ => {}
                            }
                            continue;
                        }

                        // ── Input mode ──
                        if state.awaiting_input {
                            match key.code {
                                KeyCode::Esc => {
                                    tui_input.buffer.lock().clear();
                                    *tui_input.cursor.lock() = 0;
                                    *tui_input.submitted.lock() = Some(String::new());
                                    state.input_history_pos = None;
                                }
                                KeyCode::Enter => {
                                    let mut buffer = tui_input.buffer.lock();
                                    let text = buffer.clone();
                                    if !text.is_empty() {
                                        if state.input_history.len() >= 50 {
                                            state.input_history.pop_front();
                                        }
                                        state.input_history.push_back(text.clone());
                                    }
                                    state.input_history_pos = None;
                                    buffer.clear();
                                    drop(buffer);
                                    *tui_input.cursor.lock() = 0;
                                    *tui_input.submitted.lock() = Some(text);
                                }
                                KeyCode::Backspace => {
                                    let mut buffer = tui_input.buffer.lock();
                                    let mut cursor = tui_input.cursor.lock();
                                    if *cursor > 0 {
                                        let idx = (*cursor).saturating_sub(1);
                                        if idx < buffer.chars().count() {
                                            let char_idx = buffer
                                                .char_indices()
                                                .nth(idx)
                                                .map(|(i, _)| i)
                                                .unwrap_or(0);
                                            buffer.remove(char_idx);
                                            *cursor = idx;
                                        }
                                    }
                                }
                                KeyCode::Delete => {
                                    let mut buffer = tui_input.buffer.lock();
                                    let cursor = tui_input.cursor.lock();
                                    let char_count = buffer.chars().count();
                                    if *cursor < char_count {
                                        let char_idx = buffer
                                            .char_indices()
                                            .nth(*cursor)
                                            .map(|(i, _)| i)
                                            .unwrap_or(0);
                                        buffer.remove(char_idx);
                                    }
                                }
                                KeyCode::Left => {
                                    let mut cursor = tui_input.cursor.lock();
                                    *cursor = cursor.saturating_sub(1);
                                }
                                KeyCode::Right => {
                                    let buffer = tui_input.buffer.lock();
                                    let mut cursor = tui_input.cursor.lock();
                                    *cursor = (*cursor + 1).min(buffer.chars().count());
                                }
                                KeyCode::Home => {
                                    *tui_input.cursor.lock() = 0;
                                }
                                KeyCode::End => {
                                    let buffer = tui_input.buffer.lock();
                                    let len = buffer.chars().count();
                                    *tui_input.cursor.lock() = len;
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'w' || c == 'W') =>
                                {
                                    // Delete word backward
                                    let mut buffer = tui_input.buffer.lock();
                                    let mut cursor_pos = *tui_input.cursor.lock();
                                    let chars: Vec<char> = buffer.chars().collect();
                                    if cursor_pos > chars.len() {
                                        cursor_pos = chars.len();
                                    }
                                    // Find start of current/last word
                                    let mut del_start = cursor_pos;
                                    // Skip trailing spaces
                                    while del_start > 0
                                        && chars[del_start - 1].is_whitespace()
                                    {
                                        del_start -= 1;
                                    }
                                    while del_start > 0 && !chars[del_start - 1].is_whitespace() {
                                        del_start -= 1;
                                    }
                                    let byte_idx = chars[..del_start]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum::<usize>();
                                    let end_byte: usize = chars[..cursor_pos]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    buffer.replace_range(byte_idx..end_byte, "");
                                    *tui_input.cursor.lock() = del_start;
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'u' || c == 'U') =>
                                {
                                    // Delete from cursor to start
                                    let mut buffer = tui_input.buffer.lock();
                                    let cursor_pos = *tui_input.cursor.lock();
                                    let chars: Vec<char> = buffer.chars().collect();
                                    let byte_idx: usize = chars[..cursor_pos]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    buffer.replace_range(..byte_idx, "");
                                    *tui_input.cursor.lock() = 0;
                                }
                                KeyCode::Up => {
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
                                        let len = entry.chars().count();
                                        *tui_input.buffer.lock() = entry;
                                        *tui_input.cursor.lock() = len;
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
                                            let len = entry.chars().count();
                                            *tui_input.buffer.lock() = entry;
                                            *tui_input.cursor.lock() = len;
                                            state.input_history_pos = Some(new_pos);
                                        } else {
                                            state.input_history_pos = None;
                                            tui_input.buffer.lock().clear();
                                            *tui_input.cursor.lock() = 0;
                                        }
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        // Ctrl+other keys already handled above, ignore rest
                                    } else {
                                        state.focused_panel = FocusedPanel::Input;
                                        let mut buffer = tui_input.buffer.lock();
                                        let mut cursor = tui_input.cursor.lock();
                                        let char_count = buffer.chars().count();
                                        if *cursor > char_count {
                                            *cursor = char_count;
                                        }
                                        let byte_idx = buffer
                                            .char_indices()
                                            .nth(*cursor)
                                            .map(|(i, _)| i)
                                            .unwrap_or(buffer.len());
                                        buffer.insert(byte_idx, c);
                                        *cursor += 1;
                                    }
                                }
                                KeyCode::Tab => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.focused_panel = state.focused_panel.prev();
                                    } else {
                                        state.focused_panel = state.focused_panel.next();
                                    }
                                }
                                KeyCode::BackTab => {
                                    state.focused_panel = state.focused_panel.prev();
                                }
                                _ => {}
                            }
                        }
                        // ── Search mode (active but not awaiting agent input) ──
                        else if state.search_active {
                            match key.code {
                                KeyCode::Esc => {
                                    state.search_active = false;
                                    state.search_query.clear();
                                    state.search_match_lines.clear();
                                    state.search_current_match = None;
                                }
                                KeyCode::Enter => {
                                    // Perform search on focused panel content
                                    let query = state.search_query.clone();
                                    if !query.is_empty() {
                                        let lines = get_focused_panel_lines(&state);
                                        let matches: Vec<usize> = lines
                                            .iter()
                                            .enumerate()
                                            .filter(|(_, line)| {
                                                line.to_lowercase().contains(&query.to_lowercase())
                                            })
                                            .map(|(i, _)| i)
                                            .collect();
                                        if !matches.is_empty() {
                                            let match_count = matches.len();
                                            state.search_match_lines = matches;
                                            state.search_current_match = Some(0);
                                            // Scroll to first match
                                            navigate_to_search_match(&mut state);
                                            push_log(
                                                &mut *state,
                                                format!("找到 {match_count} 处匹配"),
                                                false,
                                            );
                                        } else {
                                            state.search_match_lines.clear();
                                            state.search_current_match = None;
                                            push_log(&mut *state, "未找到匹配".into(), false);
                                        }
                                    }
                                }
                                KeyCode::Backspace => {
                                    let mut cursor = state.input_cursor;
                                    let query = &mut state.search_query;
                                    if cursor > 0 {
                                        let idx = cursor.saturating_sub(1);
                                        if idx < query.chars().count() {
                                            let char_idx = query
                                                .char_indices()
                                                .nth(idx)
                                                .map(|(i, _)| i)
                                                .unwrap_or(0);
                                            query.remove(char_idx);
                                            cursor = idx;
                                        }
                                    }
                                    state.input_cursor = cursor;
                                }
                                KeyCode::Delete => {
                                    let cursor = state.input_cursor;
                                    let query = &mut state.search_query;
                                    let char_count = query.chars().count();
                                    if cursor < char_count {
                                        let char_idx = query
                                            .char_indices()
                                            .nth(cursor)
                                            .map(|(i, _)| i)
                                            .unwrap_or(0);
                                        query.remove(char_idx);
                                    }
                                }
                                KeyCode::Left => {
                                    state.input_cursor =
                                        state.input_cursor.saturating_sub(1);
                                }
                                KeyCode::Right => {
                                    let len = state.search_query.chars().count();
                                    state.input_cursor =
                                        (state.input_cursor + 1).min(len);
                                }
                                KeyCode::Home => {
                                    state.input_cursor = 0;
                                }
                                KeyCode::End => {
                                    state.input_cursor =
                                        state.search_query.chars().count();
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'w' || c == 'W') =>
                                {
                                    let cursor_pos = state.input_cursor;
                                    let query = &mut state.search_query;
                                    let chars: Vec<char> = query.chars().collect();
                                    let mut del_start = cursor_pos.min(chars.len());
                                    while del_start > 0
                                        && chars[del_start - 1].is_whitespace()
                                    {
                                        del_start -= 1;
                                    }
                                    while del_start > 0
                                        && !chars[del_start - 1].is_whitespace()
                                    {
                                        del_start -= 1;
                                    }
                                    let byte_idx: usize = chars[..del_start]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    let end_byte: usize = chars
                                        [..cursor_pos.min(chars.len())]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    query.replace_range(byte_idx..end_byte, "");
                                    state.input_cursor = del_start;
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'u' || c == 'U') =>
                                {
                                    let cursor_pos = state.input_cursor;
                                    let query = &mut state.search_query;
                                    let chars: Vec<char> = query.chars().collect();
                                    let byte_idx: usize = chars
                                        [..cursor_pos.min(chars.len())]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    query.replace_range(..byte_idx, "");
                                    state.input_cursor = 0;
                                }
                                KeyCode::Char(c) => {
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                                        let cursor = state.input_cursor;
                                        let query = &mut state.search_query;
                                        let char_count = query.chars().count();
                                        let cursor = cursor.min(char_count);
                                        let byte_idx = query
                                            .char_indices()
                                            .nth(cursor)
                                            .map(|(i, _)| i)
                                            .unwrap_or(query.len());
                                        query.insert(byte_idx, c);
                                        state.input_cursor = cursor + 1;
                                    }
                                }
                                KeyCode::Tab => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.focused_panel =
                                            state.focused_panel.prev();
                                    } else {
                                        state.focused_panel =
                                            state.focused_panel.next();
                                    }
                                }
                                KeyCode::BackTab => {
                                    state.focused_panel = state.focused_panel.prev();
                                }
                                _ => {}
                            }
                        }
                        // ── Normal mode ──
                        else {
                            match key.code {
                                KeyCode::Esc => {
                                    if !state.search_match_lines.is_empty() {
                                        state.search_match_lines.clear();
                                        state.search_current_match = None;
                                        state.search_query.clear();
                                    } else {
                                        state.should_quit = true;
                                    }
                                }
                                KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    state.should_quit = true;
                                }
                                KeyCode::Char('/') => {
                                    state.search_active = true;
                                    state.search_query.clear();
                                    state.input_cursor = 0;
                                    state.search_match_lines.clear();
                                    state.search_current_match = None;
                                    state.focused_panel = FocusedPanel::Input;
                                }
                                KeyCode::Char('n') => {
                                    if !state.search_match_lines.is_empty() {
                                        if let Some(cur) = state.search_current_match {
                                            let next = if cur + 1
                                                < state.search_match_lines.len()
                                            {
                                                cur + 1
                                            } else {
                                                0
                                            };
                                            state.search_current_match = Some(next);
                                            navigate_to_search_match(&mut state);
                                        }
                                    }
                                }
                                KeyCode::Char('N') => {
                                    if !state.search_match_lines.is_empty() {
                                        if let Some(cur) = state.search_current_match {
                                            let prev = if cur > 0 {
                                                cur - 1
                                            } else {
                                                state.search_match_lines.len()
                                                    .saturating_sub(1)
                                            };
                                            state.search_current_match = Some(prev);
                                            navigate_to_search_match(&mut state);
                                        }
                                    }
                                }
                                KeyCode::Char('c') | KeyCode::Char('C')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    state.should_quit = true;
                                }
                                KeyCode::Char('y') | KeyCode::Char('Y')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    if let Some(text) = copy_focused_content(&state) {
                                        osc52_copy(&text);
                                        push_log(&mut *state, "已复制到剪贴板".into(), false);
                                    }
                                }
                                // Evolution per-section toggles (only when evolution focused)
                                KeyCode::Char('w') if state.focused_panel == FocusedPanel::Evolution => {
                                    state.evo_weights_hidden = !state.evo_weights_hidden;
                                }
                                KeyCode::Char('s') if state.focused_panel == FocusedPanel::Evolution => {
                                    state.evo_stats_hidden = !state.evo_stats_hidden;
                                }
                                KeyCode::Char('m') if state.focused_panel == FocusedPanel::Evolution => {
                                    state.evo_meta_hidden = !state.evo_meta_hidden;
                                }
                                KeyCode::Char('h') | KeyCode::F(1) => {
                                    state.help_visible = !state.help_visible;
                                }
                                KeyCode::Char('[') => {
                                    let pct = state.left_split_pct.unwrap_or_else(|| {
                                        state.phase.main_split_ratio(!state.evolution.all_weights().is_empty()).0
                                    });
                                    state.left_split_pct = Some(pct.saturating_sub(5).max(30));
                                }
                                KeyCode::Char(']') => {
                                    let pct = state.left_split_pct.unwrap_or_else(|| {
                                        state.phase.main_split_ratio(!state.evolution.all_weights().is_empty()).0
                                    });
                                    state.left_split_pct = Some((pct + 5).min(85));
                                }
                                KeyCode::Char('p') if !state.agent_done => {
                                    // Cancel current agent operation
                                    tui_input.stop_flag.as_ref().map(|f| f.store(true, std::sync::atomic::Ordering::Relaxed));
                                    push_log(&mut *state, "取消当前操作...".into(), false);
                                }
                                KeyCode::Char('f') if state.focused_panel == FocusedPanel::MiniLog || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing) && state.focused_panel == FocusedPanel::MainLeft) => {
                                    state.log_filter = state.log_filter.next();
                                }
                                KeyCode::Char('s')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    match export_to_file(&state) {
                                        Some((filename, _)) => {
                                            push_log(&mut *state, format!("已导出: {filename}"), false);
                                        }
                                        None => {
                                            push_log(&mut *state, "导出失败".into(), true);
                                        }
                                    }
                                }
                                KeyCode::Char('s') | KeyCode::F(2) => {
                                    state.settings_visible = !state.settings_visible;
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
                                            open_step_overlay(&mut *state, idx);
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
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            click_focus(&app_state, mouse.column, mouse.row);
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
            push_log(&mut *state, message.clone(), true);
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

/// Open the full-content overlay for a specific execution step.
fn open_step_overlay(state: &mut TuiAppState, idx: usize) {
    if let Some(step) = state.executions.get(idx) {
        state.exec_selected_index = Some(idx);
        state.output_overlay = Some(crate::state::StepOutputOverlay {
            step_id: step.step_id,
            tool: step.tool.clone(),
            status: step.status.clone(),
            duration_ms: step.duration_ms,
            full_content: step
                .content_full
                .clone()
                .unwrap_or_else(|| {
                    step.content_preview.clone().unwrap_or_default()
                }),
            scroll: 0,
        });
    }
}

/// Encode bytes to base64 string (no external crate needed).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
        out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Copy text to system clipboard via OSC 52 escape sequence.
fn osc52_copy(text: &str) {
    let b64 = base64_encode(text.as_bytes());
    let seq = format!("\x1b]52;c;{}\x1b\\", b64);
    let _ = io::stdout().write_all(seq.as_bytes());
    let _ = io::stdout().flush();
}

/// Export current results/plan/log to a file. Returns (filename, content).
fn export_to_file(state: &TuiAppState) -> Option<(String, String)> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (filename, content) = match state.phase {
        AgentPhase::Planning | AgentPhase::Executing => {
            let text = if !state.streaming_buffer.is_empty() {
                state.streaming_buffer.clone()
            } else {
                state.executions.iter().map(|s| {
                    format!("[{}] {} ({:?}ms)\n{}",
                        if matches!(s.status, crate::state::StepStatus::Success) { "OK" }
                        else if matches!(s.status, crate::state::StepStatus::Failed) { "FAIL" }
                        else { "..." },
                        s.tool,
                        s.duration_ms.unwrap_or(0),
                        s.content_full.as_deref().unwrap_or(""))
                }).collect::<Vec<_>>().join("\n\n")
            };
            (format!("hermess_plan_{ts}.txt"), text)
        }
        _ => {
            let mut text = String::new();
            if let Some(ref summary) = state.summary {
                text.push_str(&format!("Result: {}\n\n", summary));
            }
            text.push_str(&format!("Steps: {}/{}\n", state.exec_completed_steps, state.exec_total_steps));
            if let Some(dur) = state.total_duration_ms {
                text.push_str(&format!("Duration: {:.1}s\n\n", dur as f64 / 1000.0));
            }
            for entry in state.log_entries.iter() {
                let marker = if entry.is_error { "[!]" } else { "[*]" };
                text.push_str(&format!("{} {}\n", marker, entry.message));
            }
            (format!("hermess_results_{ts}.txt"), text)
        }
    };
    match std::fs::write(&filename, &content) {
        Ok(()) => Some((filename, content)),
        Err(_) => None,
    }
}

/// Copy content from the currently focused panel/selection.
fn copy_focused_content(state: &TuiAppState) -> Option<String> {
    // Overlay: copy full content
    if let Some(ref overlay) = state.output_overlay {
        return Some(overlay.full_content.clone());
    }
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning | AgentPhase::Executing => match state.left_tab {
                crate::state::LeftTab::Plan => {
                    if !state.streaming_buffer.is_empty() {
                        Some(state.streaming_buffer.clone())
                    } else {
                        None
                    }
                }
                crate::state::LeftTab::Execution => {
                    if let Some(idx) = state.exec_selected_index {
                        state.executions.get(idx).and_then(|s| {
                            s.content_full.clone().or_else(|| s.content_preview.clone())
                        })
                    } else {
                        None
                    }
                }
            },
            _ => {
                // Idle/Reflecting: copy summary
                state.summary.clone()
            }
        },
        FocusedPanel::MiniLog | FocusedPanel::Input => {
            // Copy last few log entries
            if state.log_entries.is_empty() {
                None
            } else {
                let text: String = state
                    .log_entries
                    .iter()
                    .rev()
                    .take(5)
                    .map(|e| {
                        let marker = if e.is_error { "[!]" } else { "[*]" };
                        format!("{} {}", marker, e.message)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(text)
            }
        }
        FocusedPanel::Evolution => None,
    }
}

/// Push a log entry and auto-scroll if the user hasn't scrolled away.
fn push_log(state: &mut TuiAppState, message: String, is_error: bool) {
    state.log_entries.push_back(LogEntry { message, is_error });
    if state.log_auto_scroll {
        state.log_scroll = state.log_entries.len().saturating_sub(1) as u16;
    }
}

fn scroll_focused(state: &mut TuiAppState, delta: i16) {
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning | AgentPhase::Executing => {
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
                // Disable auto-scroll when user scrolls up
                if delta < 0 {
                    state.log_auto_scroll = false;
                }
            }
        },
        FocusedPanel::Evolution => {
            state.evo_scroll = apply_delta(state.evo_scroll, delta);
        }
        FocusedPanel::MiniLog | FocusedPanel::Input => {
            state.log_scroll = apply_delta(state.log_scroll, delta);
            if delta < 0 {
                state.log_auto_scroll = false;
            }
        }
    }
}

fn page_scroll_focused(state: &mut TuiAppState, delta: i16) {
    // Scroll by roughly a viewport worth of lines (12 lines)
    let page_delta = if delta > 0 { 12 } else { -12 };
    scroll_focused(state, page_delta);
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
        FocusedPanel::MiniLog | FocusedPanel::Input => state.log_scroll = 0,
    }
}

fn scroll_to_bottom(state: &mut TuiAppState) {
    // Use a large delta to scroll to end
    let delta = 10_000_i16;
    scroll_focused(state, delta);
    // Re-enable auto-scroll when user explicitly goes to bottom
    match state.focused_panel {
        FocusedPanel::MainLeft | FocusedPanel::MiniLog | FocusedPanel::Input => {
            state.log_auto_scroll = true;
        }
        _ => {}
    }
}

fn toggle_evolution_section(state: &mut TuiAppState) {
    // Toggle all sections: if any are visible, hide all; if all hidden, show all
    let all_hidden = state.evo_weights_hidden && state.evo_stats_hidden && state.evo_meta_hidden;
    if all_hidden {
        state.evo_stats_hidden = false;
        state.evo_weights_hidden = false;
        state.evo_meta_hidden = false;
    } else {
        state.evo_weights_hidden = true;
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
    }
}

/// Collect text lines from the currently focused panel for search.
fn get_focused_panel_lines(state: &TuiAppState) -> Vec<String> {
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning | AgentPhase::Executing => match state.left_tab {
                crate::state::LeftTab::Plan => {
                    state.streaming_buffer.lines().map(|s| s.to_string()).collect()
                }
                crate::state::LeftTab::Execution => state
                    .executions
                    .iter()
                    .map(|s| {
                        let status = match s.status {
                            crate::state::StepStatus::Success => "OK",
                            crate::state::StepStatus::Failed => "FAIL",
                            crate::state::StepStatus::Running => "RUN",
                            crate::state::StepStatus::Pending => "PEND",
                        };
                        let content = s
                            .content_full
                            .as_deref()
                            .unwrap_or(s.content_preview.as_deref().unwrap_or(""));
                        format!("[{}] {} | {}", status, s.tool, crate::state::strip_ansi(content))
                    })
                    .collect(),
            },
            _ => state
                .log_entries
                .iter()
                .map(|e| {
                    format!(
                        "{} {}",
                        if e.is_error { "[!]" } else { "[*]" },
                        e.message
                    )
                })
                .collect(),
        },
        FocusedPanel::MiniLog | FocusedPanel::Input => state
            .log_entries
            .iter()
            .map(|e| {
                format!(
                    "{} {}",
                    if e.is_error { "[!]" } else { "[*]" },
                    e.message
                )
            })
            .collect(),
        FocusedPanel::Evolution => {
            // Evolution panel: collect weight lines
            let mut lines: Vec<String> = Vec::new();
            for w in state.evolution.all_weights() {
                lines.push(format!("{}: {:.4}", w.0, w.1));
            }
            lines
        }
    }
}

/// Scroll the focused panel to show the current search match line.
fn navigate_to_search_match(state: &mut TuiAppState) {
    if let Some(cur) = state.search_current_match {
        if let Some(&line_idx) = state.search_match_lines.get(cur) {
            match state.focused_panel {
                FocusedPanel::MainLeft => match state.phase {
                    AgentPhase::Planning | AgentPhase::Executing => match state.left_tab {
                        crate::state::LeftTab::Plan => {
                            state.plan_scroll = line_idx as u16;
                        }
                        crate::state::LeftTab::Execution => {
                            state.exec_scroll = line_idx as u16;
                        }
                    },
                    _ => {
                        state.log_scroll = line_idx as u16;
                        state.log_auto_scroll = false;
                    }
                },
                FocusedPanel::MiniLog | FocusedPanel::Input => {
                    state.log_scroll = line_idx as u16;
                    state.log_auto_scroll = false;
                }
                FocusedPanel::Evolution => {
                    state.evo_scroll = line_idx as u16;
                }
            }
        }
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

    let (left_pct, _right_pct) = state.split_pct();

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

/// Mouse click: determine which panel is under the cursor and focus it.
fn click_focus(app_state: &Arc<parking_lot::RwLock<TuiAppState>>, col: u16, row: u16) {
    let mut state = app_state.write();
    let term_size = crossterm::terminal::size().unwrap_or((80, 24));

    let needs_mini_log = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let footer_h = 1;
    let mini_log_h = if needs_mini_log { 3 } else { 0 };
    let main_h = term_size.1.saturating_sub(header_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = state.split_pct();

    let left_w = (term_size.0 as f64 * left_pct as f64 / 100.0) as u16;

    // Header row
    if row < header_h {
        return;
    }

    // Input/footer row
    if row >= header_h + main_h + mini_log_h {
        if state.awaiting_input {
            state.focused_panel = FocusedPanel::Input;
        }
        return;
    }

    let in_main = row < header_h + main_h;
    let in_left = col < left_w;

    if in_main && in_left {
        // Click on left main panel
        // Check for tab bar click during Planning/Executing
        if needs_mini_log && row == header_h {
            state.left_tab = state.left_tab.next();
            return;
        }
        state.focused_panel = FocusedPanel::MainLeft;
    } else if in_main && !in_left {
        state.focused_panel = FocusedPanel::Evolution;
    } else if needs_mini_log {
        state.focused_panel = FocusedPanel::MiniLog;
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
            push_log(
                state,
                format!(
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
                !output.success,
            );
        }
        AgentEvent::ExecutePhaseComplete { duration_ms, .. } => {
            state.phase = AgentPhase::Reflecting;
            state.total_duration_ms = Some(duration_ms);
            push_log(state, format!("执行完成 ({:.1}s)", duration_ms as f64 / 1000.0), false);
        }
        AgentEvent::SubAgentStarted { task } => {
            push_log(state, format!("子任务开始: {}", task), false);
        }
        AgentEvent::SubAgentCompleted { task, summary } => {
            push_log(state, format!("子任务完成: {} → {}", task, summary), false);
        }
        AgentEvent::ReplanNeeded { reason, attempt } => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.plan_ready = false;
            state.left_tab = LeftTab::Plan;
            push_log(state, format!("重规划 #{attempt}: {reason}"), false);
        }
        AgentEvent::ReplanComplete { new_steps_count } => {
            state.plan_steps_count = new_steps_count;
            state.plan_ready = true;
            push_log(state, format!("重规划完成: {new_steps_count} 个步骤"), false);
        }
        AgentEvent::ReflectPhaseStarted => {
            state.phase = AgentPhase::Reflecting;
        }
        AgentEvent::ReflectPhaseComplete { score, lesson } => {
            state.phase = AgentPhase::Evolving;
            push_log(state, format!("反思: score={:.2} | {}", score, lesson), score < 0.0);
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
            push_log(state, format!("结果: {}", summary), false);
            state.summary = Some(summary);
        }
        AgentEvent::GatewayModelsDiscovered { models, gateway_url } => {
            state.gateway_url = gateway_url.clone();
            state.gateway_models = models.clone();
            state.gateway_enabled = true;
            push_log(state, format!("Gateway: 发现 {} 个模型: {}", models.len(), models.join(", ")), false);
        }
        AgentEvent::GatewayRouteDecision {
            model,
            shg_triggered,
            reason,
        } => {
            state.last_route_decision = Some(format!("{model}: {reason}"));
            state.shg_triggered = shg_triggered;
            let shg_label = if shg_triggered { " [SHG]" } else { "" };
            push_log(state, format!("路由决策{shg_label}: → {model} ({reason})"), false);
        }
        AgentEvent::AgentError { message } => {
            push_log(state, message, true);
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
