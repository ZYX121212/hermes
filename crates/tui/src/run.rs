// crates/tui/src/run.rs
// TUI orchestrator: terminal lifecycle, event drain+dispatch, render loop, keyboard + mouse input.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use agent_core::{AgentEvent, TaskStatus, ThinkingSubPhase};
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind};
use crossterm::ExecutableCommand;
use evolution::EvolutionEngine;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::state::{
    AgentPhase, FocusedPanel, LeftTab, LogEntry, StepExecState, StepStatus,
    TuiAppState, TuiInput,
};

/// Main entry point for TUI mode.
pub async fn run_tui<A>(
    mut agent: A,
    ctx: agent_core::context::Context,
    event_rx: UnboundedReceiver<AgentEvent>,
    evolution: Arc<EvolutionEngine>,
    agent_name: String,
    tui_input: Arc<TuiInput>,
    usage_tracker: Option<Arc<llm::usage::UsageTracker>>,
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
    let mut state_init = TuiAppState::new(
        agent_name.clone(),
        Arc::clone(&evolution),
    );
    state_init.usage_tracker = usage_tracker;
    let app_state = Arc::new(parking_lot::RwLock::new(state_init));

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
                if state.settings_saved_flash > 0 {
                    state.settings_saved_flash -= 1;
                }
            }

            // 2. Render frame
            {
                let state = app_state.read();
                if let Err(e) = terminal.draw(|f| crate::render::render_app(f, &state)) {
                    tracing::error!(error = %e, "Terminal draw failed");
                    drop(state);
                    let mut state = app_state.write();
                    state.should_quit = true;
                    push_log(&mut state, format!("Terminal render error: {e}"), true);
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
                                            open_step_overlay(&mut state, new_idx);
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
                                            open_step_overlay(&mut state, new_idx);
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

                        // ── Slash command result popup (scroll + close) ──
                        if state.slash_command_popup.is_some() {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    state.slash_command_popup = None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = p.scroll.saturating_sub(1);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = p.scroll.saturating_add(1);
                                    }
                                }
                                KeyCode::PageUp => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = p.scroll.saturating_sub(10);
                                    }
                                }
                                KeyCode::PageDown => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = p.scroll.saturating_add(10);
                                    }
                                }
                                KeyCode::Home => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = 0;
                                    }
                                }
                                KeyCode::End => {
                                    if let Some(ref mut p) = state.slash_command_popup {
                                        p.scroll = 10_000;
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
                            let fields = crate::panels::settings::fields_for_tab(state.settings_tab);
                            let field_count = fields.len().max(1);

                            // Text editing mode — limited keys
                            if state.settings_editing {
                                match key.code {
                                    KeyCode::Esc => {
                                        state.settings_editing = false;
                                        state.settings_edit_buffer.clear();
                                    }
                                    KeyCode::Enter => {
                                        let f = &fields[state.settings_field_focus % field_count];
                                        settings_apply_text(&mut state, f.label);
                                        state.settings_editing = false;
                                        state.settings_edit_buffer.clear();
                                        state.settings_dirty = true;
                                    }
                                    KeyCode::Backspace => {
                                        state.settings_edit_buffer.pop();
                                    }
                                    KeyCode::Char(c) => {
                                        state.settings_edit_buffer.push(c);
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // Normal mode
                            match key.code {
                                // ── Close ──
                                KeyCode::Esc => {
                                    if state.settings_dirty && !state.settings_dirty_confirm {
                                        state.settings_dirty_confirm = true;
                                    } else {
                                        state.settings_visible = false;
                                        state.settings_dirty_confirm = false;
                                    }
                                }
                                KeyCode::F(2) | KeyCode::Char('q') => {
                                    if state.settings_dirty && !state.settings_dirty_confirm {
                                        state.settings_dirty_confirm = true;
                                    } else {
                                        state.settings_visible = false;
                                        state.settings_dirty_confirm = false;
                                    }
                                }
                                KeyCode::Char('s') => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        // Ctrl+S: save
                                        if let Err(e) = state.user_settings.save() {
                                            tracing::warn!(error = %e, "Failed to save settings");
                                        } else {
                                            *tui_input.settings_changed.lock() =
                                                Some(state.user_settings.clone());
                                            state.settings_dirty = false;
                                            state.settings_saved_flash = 60; // ~2s at 30fps
                                            state.settings_dirty_confirm = false;
                                        }
                                    } else if state.settings_dirty && !state.settings_dirty_confirm {
                                        state.settings_dirty_confirm = true;
                                    } else {
                                        state.settings_visible = false;
                                        state.settings_dirty_confirm = false;
                                    }
                                }

                                // ── Tab navigation ──
                                KeyCode::Tab => {
                                    state.settings_tab = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.settings_tab.prev()
                                    } else {
                                        state.settings_tab.next()
                                    };
                                    state.settings_field_focus = 0;
                                }

                                // ── Field navigation ──
                                KeyCode::Up | KeyCode::Char('k') => {
                                    state.settings_field_focus =
                                        state.settings_field_focus.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    let next = state.settings_field_focus + 1;
                                    if next < field_count {
                                        state.settings_field_focus = next;
                                    }
                                }

                                // ── Edit / Toggle ──
                                KeyCode::Char(' ') => {
                                    let f = &fields[state.settings_field_focus % field_count];
                                    if matches!(f.kind, crate::panels::settings::FieldKind::Toggle) {
                                        settings_toggle(&mut state, f.label);
                                        state.settings_dirty = true;
                                        state.settings_dirty_confirm = false;
                                    }
                                }
                                KeyCode::Enter => {
                                    let f = &fields[state.settings_field_focus % field_count];
                                    state.settings_dirty_confirm = false;
                                    match f.kind {
                                        crate::panels::settings::FieldKind::Toggle => {
                                            settings_toggle(&mut state, f.label);
                                            state.settings_dirty = true;
                                        }
                                        crate::panels::settings::FieldKind::Dropdown => {
                                            settings_cycle_dropdown(&mut state, f.label);
                                            state.settings_dirty = true;
                                        }
                                        crate::panels::settings::FieldKind::Text => {
                                            state.settings_editing = true;
                                            state.settings_edit_buffer =
                                                settings_get_text(&state, f.label);
                                        }
                                    }
                                }

                                _ => {}
                            }
                            continue;
                        }

                        // ── Slash command input mode ──
                        if state.slash_command_active {
                            match key.code {
                                KeyCode::Esc => {
                                    state.slash_command_active = false;
                                    state.slash_command_buffer.clear();
                                    state.slash_command_cursor = 0;
                                }
                                KeyCode::Enter => {
                                    let cmd = state.slash_command_buffer.clone();
                                    state.slash_command_active = false;
                                    state.slash_command_buffer.clear();
                                    state.slash_command_cursor = 0;
                                    dispatch_slash_command(&mut state, &cmd);
                                }
                                KeyCode::Backspace => {
                                    let cursor = state.slash_command_cursor;
                                    if cursor > 0 {
                                        let byte_idx = state
                                            .slash_command_buffer
                                            .char_indices()
                                            .nth(cursor - 1)
                                            .map(|(i, _)| i)
                                            .unwrap_or(0);
                                        state.slash_command_buffer.remove(byte_idx);
                                        state.slash_command_cursor = cursor - 1;
                                    }
                                }
                                KeyCode::Left => {
                                    state.slash_command_cursor =
                                        state.slash_command_cursor.saturating_sub(1);
                                }
                                KeyCode::Right => {
                                    let max = state.slash_command_buffer.chars().count();
                                    if state.slash_command_cursor < max {
                                        state.slash_command_cursor += 1;
                                    }
                                }
                                KeyCode::Home => {
                                    state.slash_command_cursor = 0;
                                }
                                KeyCode::End => {
                                    state.slash_command_cursor =
                                        state.slash_command_buffer.chars().count();
                                }
                                KeyCode::Char(c) => {
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                                        let cursor = state.slash_command_cursor;
                                        let buf = &mut state.slash_command_buffer;
                                        let char_count = buf.chars().count();
                                        let cursor = cursor.min(char_count);
                                        let byte_idx = buf
                                            .char_indices()
                                            .nth(cursor)
                                            .map(|(i, _)| i)
                                            .unwrap_or(buf.len());
                                        buf.insert(byte_idx, c);
                                        state.slash_command_cursor = cursor + 1;
                                    }
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
                                    state.input_draft.clear();
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
                                    state.input_draft.clear();
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
                                        // Save current text as draft before entering history
                                        if state.input_history_pos.is_none() {
                                            state.input_draft =
                                                tui_input.buffer.lock().clone();
                                        }
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
                                            // Past end: restore saved draft
                                            let draft =
                                                std::mem::take(&mut state.input_draft);
                                            state.input_history_pos = None;
                                            let len = draft.chars().count();
                                            *tui_input.buffer.lock() = draft;
                                            *tui_input.cursor.lock() = len;
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
                                                &mut state,
                                                format!("找到 {match_count} 处匹配"),
                                                false,
                                            );
                                        } else {
                                            state.search_match_lines.clear();
                                            state.search_current_match = None;
                                            push_log(&mut state, "未找到匹配".into(), false);
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
                                KeyCode::Tab | KeyCode::BackTab => {
                                    // Ignore Tab during active search to preserve n/N navigation context
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
                                        state.search_active = false;
                                        state.input_cursor = 0;
                                    } else {
                                        state.should_quit = true;
                                    }
                                }
                                KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    // Only quit immediately when agent is done.
                                    // While agent is running, require Ctrl+C or Esc instead.
                                    if state.agent_done {
                                        state.should_quit = true;
                                    } else {
                                        push_log(
                                            &mut state,
                                            "Agent 仍在运行，请用 Esc 或 Ctrl+C 退出".into(),
                                            false,
                                        );
                                    }
                                }
                                KeyCode::Char('/') => {
                                    state.search_active = true;
                                    state.search_query.clear();
                                    state.input_cursor = 0;
                                    state.search_match_lines.clear();
                                    state.search_current_match = None;
                                    state.focused_panel = FocusedPanel::Input;
                                }
                                KeyCode::Char(':') => {
                                    state.slash_command_active = true;
                                    state.slash_command_buffer.clear();
                                    state.slash_command_cursor = 0;
                                    state.slash_command_popup = None;
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
                                        push_log(&mut state, "已复制到剪贴板".into(), false);
                                    }
                                }
                                // Evolution per-section toggles (only when evolution focused)
                                KeyCode::Char('w') if state.focused_panel == FocusedPanel::Evolution => {
                                    state.evo_weights_hidden = !state.evo_weights_hidden;
                                }
                                KeyCode::Char('t') if state.focused_panel == FocusedPanel::Evolution => {
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
                                    if let Some(f) = tui_input.stop_flag.as_ref() { f.store(true, std::sync::atomic::Ordering::Relaxed) }
                                    push_log(&mut state, "取消当前操作...".into(), false);
                                }
                                KeyCode::Char('f') if state.focused_panel == FocusedPanel::MiniLog || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing) && state.focused_panel == FocusedPanel::MainLeft) => {
                                    state.log_filter = state.log_filter.next();
                                }
                                KeyCode::Char('l')
                                    if !state.awaiting_input && state.output_overlay.is_none() =>
                                {
                                    state.log_visible = !state.log_visible;
                                }
                                KeyCode::Char('s')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    match export_to_file(&state) {
                                        Some((filename, _)) => {
                                            push_log(&mut state, format!("已导出: {filename}"), false);
                                        }
                                        None => {
                                            push_log(&mut state, "导出失败".into(), true);
                                        }
                                    }
                                }
                                KeyCode::Char('s') | KeyCode::F(2) => {
                                    state.settings_visible = !state.settings_visible;
                                }
                                KeyCode::BackTab => {
                                    // Don't change focus when search has active matches
                                    if state.search_match_lines.is_empty() {
                                        state.focused_panel = state.focused_panel.prev();
                                    }
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
                                        if state.search_match_lines.is_empty() {
                                            state.focused_panel = state.focused_panel.prev();
                                        }
                                    } else if state.search_match_lines.is_empty() {
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
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                        && !state.executions.is_empty()
                                    {
                                        state.exec_selected_index = Some(0);
                                        state.exec_scroll = 0;
                                    } else {
                                        scroll_to_top(&mut state);
                                    }
                                }
                                KeyCode::End => {
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                        && !state.executions.is_empty()
                                    {
                                        let last = state.executions.len() - 1;
                                        state.exec_selected_index = Some(last);
                                        state.exec_scroll = last.saturating_sub(7) as u16;
                                    } else {
                                        scroll_to_bottom(&mut state);
                                    }
                                }
                                // Evolution panel: Enter toggles section collapse
                                KeyCode::Enter => {
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                    {
                                        if let Some(idx) = state.exec_selected_index {
                                            open_step_overlay(&mut state, idx);
                                        }
                                    } else if state.focused_panel == FocusedPanel::Evolution {
                                        toggle_evolution_section(&mut state);
                                    } else {
                                        // Fallback: focus the input bar (Tab to cycle panels)
                                        state.focused_panel = FocusedPanel::Input;
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
        state.log_scroll = 0; // reset scroll so results panel shows from top
        if let Err(ref e) = result {
            let message = format!("执行失败: {e}");
            push_log(&mut state, message.clone(), true);
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

// ── Settings helpers ──

fn settings_toggle(state: &mut TuiAppState, label: &str) {
    if label == "启用搜索" { state.user_settings.search_enabled = !state.user_settings.search_enabled }
}

fn settings_cycle_dropdown(state: &mut TuiAppState, label: &str) {
    match label {
        "提供商标识" => {
            state.user_settings.llm_provider = match state.user_settings.llm_provider.as_str() {
                "deepseek" => "openai".into(),
                "openai" => "anthropic".into(),
                _ => "deepseek".into(),
            };
        }
        "金融数据源" => {
            state.user_settings.finance_provider = match state.user_settings.finance_provider.as_str() {
                "" => "sina".into(),
                "sina" => "tushare".into(),
                "tushare" => "ftshare".into(),
                _ => String::new(),
            };
        }
        _ => {}
    }
}

fn settings_get_text(state: &TuiAppState, label: &str) -> String {
    let s = &state.user_settings;
    match label {
        "提供商标识" => s.llm_provider.clone(),
        "模型名称" => s.llm_model.clone(),
        "API Key" => s.llm_api_key.clone(),
        "Base URL" => s.llm_base_url.clone(),
        "搜索 Key" => s.search_api_key.clone(),
        "金融数据源" => s.finance_provider.clone(),
        "TuShare Token" => s.finance_tushare_token.clone(),
        _ => String::new(),
    }
}

fn settings_apply_text(state: &mut TuiAppState, label: &str) {
    let val = state.settings_edit_buffer.clone();
    match label {
        "提供商标识" => state.user_settings.llm_provider = val,
        "模型名称" => state.user_settings.llm_model = val,
        "API Key" => state.user_settings.llm_api_key = val,
        "Base URL" => state.user_settings.llm_base_url = val,
        "搜索 Key" => state.user_settings.search_api_key = val,
        "金融数据源" => state.user_settings.finance_provider = val,
        "TuShare Token" => state.user_settings.finance_tushare_token = val,
        _ => {}
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

fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}
fn dispatch_slash_command(state: &mut TuiAppState, cmd: &str) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let head = parts.first().copied().unwrap_or("");
    let rest = parts[1..].join(" ");

    match head {
        "/help" | "/h" => {
            state.help_visible = !state.help_visible;
        }
        "/model" => {
            let msg = if rest.is_empty() {
                format!("当前模型: {}", if state.user_settings.llm_model.is_empty() { "(默认)" } else { &state.user_settings.llm_model })
            } else {
                state.user_settings.llm_model = rest;
                state.settings_dirty = true;
                format!("模型已设置为: {}", state.user_settings.llm_model)
            };
            push_log(state, msg, false);
        }
        "/personality" => {
            push_log(state, format!("人格设置: {} (需要后端支持)", rest), false);
        }
        "/status" => {
            let completed = state.exec_completed_steps;
            let total = state.exec_total_steps;
            let error_count = state.log_entries.iter().filter(|e| e.is_error).count();
            let phase_str = match state.phase {
                crate::state::AgentPhase::Idle => "空闲",
                crate::state::AgentPhase::Observing => "观察中",
                crate::state::AgentPhase::Planning => "规划中",
                crate::state::AgentPhase::Executing => "执行中",
                crate::state::AgentPhase::Reflecting => "反思中",
                crate::state::AgentPhase::Evolving => "进化中",
            };
            let lines = vec![
                format!("  回合数: {}", state.turn),
                format!("  阶段: {}", phase_str),
                format!("  步骤: {}/{} 已完成", completed, total),
                format!("  错误: {} 个", error_count),
                format!("  日志条目: {}", state.log_entries.len()),
                format!("  输入历史: {} 条", state.input_history.len()),
                String::new(),
                format!("  Gateway: {}", if state.gateway_enabled { "已启用" } else { "未启用" }),
                format!("  路由模式: {}", if state.gateway_mode.is_empty() { "(默认)" } else { &state.gateway_mode }),
            ];
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Status".into(),
                lines,
                scroll: 0,
            });
        }
        "/debug" => {
            let weights = state.evolution.all_weights();
            let insight_count = state.evolution.insight_count();
            let strategy_count = state.evolution.strategy_count();
            let mut lines = vec![
                format!("  回合: {}  阶段: {:?}", state.turn, state.phase),
                format!("  日志条目: {}  错误: {}", state.log_entries.len(), state.log_entries.iter().filter(|e| e.is_error).count()),
                format!("  进化引擎: {} insights, {} strategies", insight_count, strategy_count),
                format!("  当前学习率: {:.5}", state.evolution.current_learning_rate()),
                format!("  Gateway: {}  模型数: {}", state.gateway_enabled, state.gateway_models.len()),
                format!("  Settings dirty: {}", state.settings_dirty),
                String::new(),
                "  策略权重:".into(),
            ];
            for (name, w) in weights.iter().take(12) {
                lines.push(format!("    {}: {:+.4}", name, w));
            }
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Debug".into(),
                lines,
                scroll: 0,
            });
        }
        "/usage" => {
            let secs = state.frame_count / 30;
            let elapsed = if secs < 60 {
                format!("{}s", secs)
            } else {
                format!("{}m{}s", secs / 60, secs % 60)
            };
            let lines = vec![
                format!("  回合数: {}", state.turn),
                format!("  耗时: {}", elapsed),
                format!("  已执行步骤: {} / {}", state.exec_completed_steps, state.exec_total_steps),
                format!("  进化统计: {} 条 insight", state.evolution.insight_count()),
                String::new(),
                "  详细 token 用量请查看 LLM provider 后台。".into(),
            ];
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Usage".into(),
                lines,
                scroll: 0,
            });
        }
        "/sessions" => {
            let mut lines: Vec<String> = Vec::new();
            let session_dir = home_dir().join(".hermess").join("sessions");
            match std::fs::read_dir(&session_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            if let Ok(meta) = std::fs::metadata(&path) {
                                if let Ok(modified) = meta.modified() {
                                    if let Ok(dur) = modified.elapsed() {
                                        let age = if dur.as_secs() < 3600 {
                                            format!("{}m ago", dur.as_secs() / 60)
                                        } else if dur.as_secs() < 86400 {
                                            format!("{}h ago", dur.as_secs() / 3600)
                                        } else {
                                            format!("{}d ago", dur.as_secs() / 86400)
                                        };
                                        let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                        lines.push(format!("  {}  ({})", name, age));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    lines.push("  (无法读取会话目录)".into());
                }
            }
            if lines.is_empty() {
                lines.push("  (无已保存会话)".into());
                lines.push(String::new());
                lines.push("  使用 Ctrl+S 导出当前会话。".into());
            }
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Sessions".into(),
                lines,
                scroll: 0,
            });
        }
        "/skills" => {
            let mut lines: Vec<String> = Vec::new();
            let skills_dir = home_dir().join(".claude")
                .join("skills");
            if skills_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let ft = entry.file_type().map(|t| if t.is_dir() { "/" } else { "" }).unwrap_or("");
                        lines.push(format!("  {}{}", name, ft));
                    }
                }
            }
            if lines.is_empty() {
                lines.push("  (未找到已安装 skills)".into());
                lines.push(String::new());
                lines.push("  Skills 目录: ~/.claude/skills/".into());
            }
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Skills".into(),
                lines,
                scroll: 0,
            });
        }
        "/checkpoint" => {
            push_log(state, "[checkpoint] 功能需要后端事件 plumbing，暂不可用。使用 Ctrl+S 导出会话。".into(), false);
        }
        "/rollback" => {
            push_log(state, "[rollback] 功能需要后端事件 plumbing，暂不可用。".into(), false);
        }
        "/diff" => {
            push_log(state, "[diff] 功能需要后端事件 plumbing，暂不可用。".into(), false);
        }
        "/new" => {
            push_log(state, "[new] 需要会话管理器支持。请重新启动 Hermess 开始新会话。".into(), false);
        }
        "/load" => {
            push_log(state, format!("[load] 需要会话管理器支持: {}", if rest.is_empty() { "(请指定会话名)" } else { &rest }), false);
        }
        "/memory" | "/recall" => {
            push_log(state, format!("[{}] 需要后端 WorkingMemory 查询支持。", head.trim_start_matches('/')), false);
        }
        "/compress" => {
            push_log(state, "[compress] 需要后端上下文压缩支持。".into(), false);
        }
        "/cron" | "/kanban" => {
            push_log(state, format!("[{}] 功能开发中。", head.trim_start_matches('/')), false);
        }
        _ => {
            push_log(state, format!("未知命令: {}. 输入 : /help 查看可用命令。", head), false);
        }
    }
}

/// Push a log entry and auto-scroll if the user hasn't scrolled away.
/// Deduplicates consecutive identical messages by incrementing a repeat counter.
fn push_log(state: &mut TuiAppState, message: String, is_error: bool) {
    let clean = crate::state::strip_html(&message);
    // Dedup: if identical to the last entry, bump repeat count
    if let Some(last) = state.log_entries.back_mut() {
        if last.message == clean && last.is_error == is_error {
            last.repeat_count += 1;
            return;
        }
    }
    state.log_entries.push_back(LogEntry { message: clean, is_error, repeat_count: 0 });
    if state.log_auto_scroll {
        state.log_scroll = 10_000; // large enough to force bottom-scroll without overflow
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
                if delta < 0 {
                    state.log_auto_scroll = false;
                } else if state.log_scroll as usize >= state.log_entries.len().saturating_sub(1) {
                    // Re-enable auto-scroll when user scrolls back to bottom
                    state.log_auto_scroll = true;
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
            } else if state.log_scroll as usize >= state.log_entries.len().saturating_sub(1) {
                state.log_auto_scroll = true;
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
    let delta = 10_000_i16;
    scroll_focused(state, delta);
    // Only re-enable log auto-scroll when the log was actually scrolled,
    // not when Plan or Execution tab content was scrolled via MainLeft focus.
    match state.focused_panel {
        FocusedPanel::MiniLog | FocusedPanel::Input => {
            state.log_auto_scroll = true;
        }
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning | AgentPhase::Executing => {}
            _ => { state.log_auto_scroll = true; }
        },
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
            state.exec_selected_index = None;
            state.results_visible = true;
            // Don't set left_tab here — PlanPhaseStarted or ExecutePhaseStarted
            // will set it when the actual phase begins, avoiding flicker.
        }
        AgentEvent::PlanPhaseStarted => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.plan_ready = false;
            state.left_tab = LeftTab::Plan;
            state.plan_scroll = 0;
        }
        AgentEvent::PlanStreamingToken { token } => {
            state.streaming_buffer.push_str(&crate::state::strip_html(&token));
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
                let clean = crate::state::strip_html(&crate::state::strip_ansi(&output.content));
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
            state.plan_scroll = 0;
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
            state.summary_streaming_buffer.push_str(&crate::state::strip_html(&token));
        }
        AgentEvent::SummaryReady { summary } => {
            state.summary_streaming_buffer.clear();
            let clean_summary = crate::state::strip_html(&summary);
            push_log(state, format!("结果: {}", clean_summary), false);
            state.summary = Some(clean_summary);
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
        AgentEvent::TaskUpdated {
            task_id,
            title,
            status,
        } => {
            let status_str = match status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in-progress",
                TaskStatus::Completed => "completed",
            };
            push_log(
                state,
                format!("任务更新 [{status_str}]: {title} (#{})", &task_id[..8.min(task_id.len())]),
                false,
            );
        }
        AgentEvent::ThinkingPhaseChanged { sub_phase } => {
            let label = match sub_phase {
                ThinkingSubPhase::CallingLlm => "CallingLLM",
                ThinkingSubPhase::ParsingResponse => "Parsing",
                ThinkingSubPhase::ExecutingTool => "ExecTool",
                ThinkingSubPhase::WaitingForInput => "WaitingInput",
                ThinkingSubPhase::Idle => "Idle",
            };
            push_log(state, format!("思考阶段: {label}"), false);
        }
        AgentEvent::SetPersonality { name } => {
            state.agent_name = name;
        }
        AgentEvent::CompressContext => {
            push_log(state, "上下文压缩".to_string(), false);
        }
        AgentEvent::SaveCheckpoint => {
            push_log(state, "保存检查点".to_string(), false);
        }
        AgentEvent::RollbackCheckpoint => {
            push_log(state, "回滚检查点".to_string(), false);
        }
        AgentEvent::ResetSession => {
            push_log(state, "会话重置".to_string(), false);
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
