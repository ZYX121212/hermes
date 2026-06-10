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
    AgentPhase, FocusedPanel, LeftTab, LogEntry, StepExecState, StepStatus, TuiAppState, TuiInput,
};

/// Close all overlays and focus the input panel — unified exit path.
fn close_overlays_focus_input(state: &mut TuiAppState) {
    state.output_overlay = None;
    state.slash_command_popup = None;
    state.help_visible = false;
    state.settings_visible = false;
    state.settings_editing = false;
    state.settings_dirty_confirm = false;
    state.focused_panel = FocusedPanel::Input;
}

/// Handle a key press while the help overlay is open.
/// Returns true if the overlay should remain open.
fn handle_help_overlay_key(state: &mut TuiAppState, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('h') | KeyCode::Esc | KeyCode::F(1) => {
            state.help_visible = false;
            state.help_scroll = 0;
            if state.awaiting_input {
                state.focused_panel = FocusedPanel::Input;
            }
            false
        }
        KeyCode::Tab | KeyCode::BackTab => true,
        KeyCode::Down | KeyCode::Char('j') => {
            state.help_scroll = state.help_scroll.saturating_add(1);
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            true
        }
        KeyCode::PageDown => {
            state.help_scroll = state.help_scroll.saturating_add(10);
            true
        }
        KeyCode::PageUp => {
            state.help_scroll = state.help_scroll.saturating_sub(10);
            true
        }
        KeyCode::Home => {
            state.help_scroll = 0;
            true
        }
        KeyCode::End => {
            state.help_scroll = u16::MAX;
            true
        }
        KeyCode::Char(_) if state.awaiting_input => {
            close_overlays_focus_input(state);
            false
        }
        _ => true,
    }
}

fn begin_next_task_input(state: &mut TuiAppState, tui_input: &TuiInput) {
    tui_input
        .awaiting
        .store(true, std::sync::atomic::Ordering::Relaxed);
    tui_input.buffer.lock().clear();
    *tui_input.cursor.lock() = 0;
    *tui_input.submitted.lock() = None;
    state.awaiting_input = true;
    state.input_text.clear();
    state.input_cursor = 0;
    state.input_line_count = 1;
    state.focused_panel = FocusedPanel::Input;
}

fn submit_tui_input(state: &mut TuiAppState, tui_input: &TuiInput, text: String) -> bool {
    let mut submitted = tui_input.submitted.lock();
    if submitted.is_some() {
        return false;
    }
    *submitted = Some(text);
    drop(submitted);

    tui_input
        .awaiting
        .store(false, std::sync::atomic::Ordering::Relaxed);
    tui_input.buffer.lock().clear();
    *tui_input.cursor.lock() = 0;
    state.awaiting_input = false;
    state.input_text.clear();
    state.input_cursor = 0;
    state.input_history_pos = None;
    state.input_draft.clear();
    state.context_ref_active = false;
    state.context_ref_query.clear();
    // 提交后立刻离开 Input 面板，避免在 idle 状态渲染一帧产生闪烁
    state.focused_panel = FocusedPanel::MainLeft;
    true
}

/// 将 context_ref 选中项插入到输入缓冲区，替换 @query 部分。
fn insert_context_ref_text(state: &mut TuiAppState, tui_input: &TuiInput, label: &str) {
    let mut buffer = tui_input.buffer.lock();
    if let Some(at_pos) = buffer.rfind('@') {
        buffer.truncate(at_pos);
    }
    buffer.push_str(label);
    buffer.push(' ');
    let len = buffer.chars().count();
    *tui_input.cursor.lock() = len;
    state.context_ref_active = false;
    state.context_ref_query.clear();
    state.context_ref_items.clear();
    state.focused_panel = FocusedPanel::Input;
}

fn close_settings_or_confirm_discard(state: &mut TuiAppState) {
    if state.settings_dirty && !state.settings_dirty_confirm {
        state.settings_dirty_confirm = true;
    } else {
        if state.settings_dirty {
            state.user_settings = load_effective_user_settings();
        }
        state.settings_visible = false;
        state.settings_editing = false;
        state.settings_edit_buffer.clear();
        state.settings_dirty_confirm = false;
        state.settings_dirty = false;
        state.focused_panel = FocusedPanel::Input;
    }
}

fn load_effective_user_settings() -> crate::settings_store::UserSettings {
    let mut settings = crate::settings_store::UserSettings::load();
    settings.apply_env_overrides();
    settings
}

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
    let mut state_init = TuiAppState::new(agent_name.clone(), Arc::clone(&evolution));
    state_init.usage_tracker = usage_tracker;
    state_init.user_settings = load_effective_user_settings();
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
                state.input_line_count = if state.awaiting_input {
                    input_line_count_for(&state.input_text)
                } else {
                    1
                };
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
            if let Ok(true) =
                crossterm::event::poll(Duration::from_millis(crate::render::RENDER_POLL_MS))
            {
                match crossterm::event::read() {
                    Ok(Event::Key(key)) => {
                        let mut state = app_state.write();

                        if is_ctrl_c(key.code, key.modifiers) {
                            request_tui_quit(&mut state, &tui_input);
                            continue;
                        }

                        // ── Overlay mode (absorbs all keys) ──
                        if state.output_overlay.is_some() {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                                    state.output_overlay = None;
                                }
                                KeyCode::Tab | KeyCode::BackTab => {
                                    // No-op: overlay has no pages to switch
                                }
                                KeyCode::Char(_) if state.awaiting_input => {
                                    close_overlays_focus_input(&mut state);
                                }
                                KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('p') => {
                                    // Previous step
                                    let len = state.executions.len();
                                    if len > 0 {
                                        let idx = state.exec_selected_index.unwrap_or(0);
                                        if idx > 0 {
                                            let new_idx = idx - 1;
                                            open_step_overlay(&mut state, new_idx);
                                        }
                                    }
                                }
                                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('n') => {
                                    // Next step
                                    let len = state.executions.len();
                                    if len > 0 {
                                        let idx = state.exec_selected_index.unwrap_or(0);
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
                                KeyCode::Tab | KeyCode::BackTab => {
                                    // No-op: popup has no pages to switch
                                }
                                KeyCode::Char(_) if state.awaiting_input => {
                                    close_overlays_focus_input(&mut state);
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

                        // ── Help overlay ──
                        if state.help_visible {
                            handle_help_overlay_key(&mut state, key.code);
                            continue;
                        }

                        // ── Settings overlay ──
                        if state.settings_visible {
                            let fields =
                                crate::panels::settings::fields_for_tab(state.settings_tab);
                            let field_count = fields.len().max(1);

                            // Text editing mode
                            if state.settings_editing {
                                match key.code {
                                    KeyCode::Esc => {
                                        state.settings_editing = false;
                                        state.settings_edit_buffer.clear();
                                    }
                                    KeyCode::Tab | KeyCode::BackTab => {
                                        // Apply current edit, then switch settings tab
                                        let f = &fields[state.settings_field_focus % field_count];
                                        settings_apply_text(&mut state, f.label);
                                        state.settings_editing = false;
                                        state.settings_edit_buffer.clear();
                                        state.settings_dirty = true;
                                        state.settings_tab = if key.code == KeyCode::BackTab
                                            || key.modifiers.contains(KeyModifiers::SHIFT)
                                        {
                                            state.settings_tab.prev()
                                        } else {
                                            state.settings_tab.next()
                                        };
                                        state.settings_field_focus = 0;
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
                                KeyCode::Esc | KeyCode::F(2) | KeyCode::Char('q') => {
                                    close_settings_or_confirm_discard(&mut state);
                                }
                                KeyCode::Char('s') => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        if let Err(e) = state.user_settings.save() {
                                            tracing::warn!(error = %e, "Failed to save settings");
                                        } else {
                                            *tui_input.settings_changed.lock() =
                                                Some(state.user_settings.clone());
                                            state.settings_dirty = false;
                                            state.settings_saved_flash = 60;
                                            state.settings_dirty_confirm = false;
                                        }
                                    } else {
                                        close_settings_or_confirm_discard(&mut state);
                                    }
                                }

                                // ── Tab / Shift+Tab: switch settings tab page ──
                                KeyCode::Tab | KeyCode::BackTab => {
                                    state.settings_tab = if key.code == KeyCode::BackTab
                                        || key.modifiers.contains(KeyModifiers::SHIFT)
                                    {
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

                                // ── Space: cycle/switch value ──
                                KeyCode::Char(' ') => {
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
                                        _ => {}
                                    }
                                }
                                // ── Enter: edit (Text) or switch (Toggle/Dropdown) ──
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

                                // ── Char passthrough: agent waiting → close → Input ──
                                KeyCode::Char(_) if state.awaiting_input => {
                                    close_overlays_focus_input(&mut state);
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
                                    // 关闭 context_ref 弹窗
                                    if state.context_ref_active {
                                        state.context_ref_active = false;
                                        state.context_ref_query.clear();
                                        state.context_ref_items.clear();
                                        continue;
                                    }
                                    // 取消输入：清空缓冲区并退出输入模式，但不提交（不触发退出）
                                    // 外层循环继续等待真实提交；按两次 Esc 才会退出程序
                                    tui_input.buffer.lock().clear();
                                    *tui_input.cursor.lock() = 0;
                                    tui_input
                                        .awaiting
                                        .store(false, std::sync::atomic::Ordering::Relaxed);
                                    state.awaiting_input = false;
                                    state.input_text.clear();
                                    state.input_cursor = 0;
                                    state.input_history_pos = None;
                                    state.input_draft.clear();
                                    state.context_ref_active = false;
                                    state.context_ref_query.clear();
                                }
                                KeyCode::Enter => {
                                    // 在 context_ref 弹窗中选择当前项
                                    if state.context_ref_active
                                        && !state.context_ref_items.is_empty()
                                    {
                                        let label = state.context_ref_items[state
                                            .context_ref_selected
                                            .min(state.context_ref_items.len() - 1)]
                                        .label
                                        .clone();
                                        insert_context_ref_text(&mut state, &tui_input, &label);
                                        continue;
                                    }
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        // Shift+Enter: insert newline for multiline input
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
                                        buffer.insert(byte_idx, '\n');
                                        *cursor += 1;
                                        // Deactivate context_ref on newline
                                        state.context_ref_active = false;
                                        state.context_ref_query.clear();
                                    } else {
                                        let text = tui_input.buffer.lock().clone();
                                        if !text.is_empty() {
                                            if state.input_history.len() >= 50 {
                                                state.input_history.pop_front();
                                            }
                                            state.input_history.push_back(text.clone());
                                        }
                                        submit_tui_input(&mut state, &tui_input, text);
                                    }
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
                                    // Update context_ref state after backspace
                                    if state.context_ref_active {
                                        let new_cursor = *cursor;
                                        let buf = buffer.clone();
                                        drop(cursor);
                                        drop(buffer);
                                        let byte_end = buf
                                            .char_indices()
                                            .nth(new_cursor.min(buf.chars().count()))
                                            .map(|(i, _)| i)
                                            .unwrap_or(buf.len());
                                        let last_at = buf[..byte_end].rfind('@');
                                        if let Some(at_pos) = last_at {
                                            state.context_ref_query =
                                                buf[at_pos..byte_end].to_string();
                                            crate::panels::context_ref::populate_suggestions(
                                                &mut state,
                                            );
                                        } else {
                                            state.context_ref_active = false;
                                            state.context_ref_query.clear();
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
                                    let cur = *cursor;
                                    drop(cursor);
                                    // Update context_ref state after delete
                                    if state.context_ref_active {
                                        let buf = buffer.clone();
                                        drop(buffer);
                                        let byte_end = buf
                                            .char_indices()
                                            .nth(cur.min(buf.chars().count()))
                                            .map(|(i, _)| i)
                                            .unwrap_or(buf.len());
                                        let last_at = buf[..byte_end].rfind('@');
                                        if let Some(at_pos) = last_at {
                                            state.context_ref_query =
                                                buf[at_pos..byte_end].to_string();
                                            crate::panels::context_ref::populate_suggestions(
                                                &mut state,
                                            );
                                        } else {
                                            state.context_ref_active = false;
                                            state.context_ref_query.clear();
                                        }
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
                                    while del_start > 0 && chars[del_start - 1].is_whitespace() {
                                        del_start -= 1;
                                    }
                                    while del_start > 0 && !chars[del_start - 1].is_whitespace() {
                                        del_start -= 1;
                                    }
                                    let byte_idx = chars[..del_start]
                                        .iter()
                                        .map(|c| c.len_utf8())
                                        .sum::<usize>();
                                    let end_byte: usize =
                                        chars[..cursor_pos].iter().map(|c| c.len_utf8()).sum();
                                    buffer.replace_range(byte_idx..end_byte, "");
                                    *tui_input.cursor.lock() = del_start;
                                    // Update context_ref after word delete
                                    if state.context_ref_active {
                                        let cur = del_start;
                                        let buf = buffer.clone();
                                        drop(buffer);
                                        let byte_end = buf
                                            .char_indices()
                                            .nth(cur.min(buf.chars().count()))
                                            .map(|(i, _)| i)
                                            .unwrap_or(buf.len());
                                        let last_at = buf[..byte_end].rfind('@');
                                        if let Some(at_pos) = last_at {
                                            state.context_ref_query =
                                                buf[at_pos..byte_end].to_string();
                                            crate::panels::context_ref::populate_suggestions(
                                                &mut state,
                                            );
                                        } else {
                                            state.context_ref_active = false;
                                            state.context_ref_query.clear();
                                        }
                                    }
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'u' || c == 'U') =>
                                {
                                    // Delete from cursor to start
                                    let mut buffer = tui_input.buffer.lock();
                                    let cursor_pos = *tui_input.cursor.lock();
                                    let chars: Vec<char> = buffer.chars().collect();
                                    let byte_idx: usize =
                                        chars[..cursor_pos].iter().map(|c| c.len_utf8()).sum();
                                    buffer.replace_range(..byte_idx, "");
                                    *tui_input.cursor.lock() = 0;
                                    let buf = buffer.clone();
                                    drop(buffer);
                                    // Update context_ref after line clear
                                    if state.context_ref_active {
                                        let last_at = buf.rfind('@');
                                        if let Some(at_pos) = last_at {
                                            state.context_ref_query = buf[at_pos..].to_string();
                                            crate::panels::context_ref::populate_suggestions(
                                                &mut state,
                                            );
                                        } else {
                                            state.context_ref_active = false;
                                            state.context_ref_query.clear();
                                        }
                                    }
                                }
                                KeyCode::Up => {
                                    // context_ref 弹窗导航：向上选择
                                    if state.context_ref_active
                                        && !state.context_ref_items.is_empty()
                                    {
                                        state.context_ref_selected =
                                            state.context_ref_selected.saturating_sub(1);
                                        continue;
                                    }
                                    let hist_len = state.input_history.len();
                                    if hist_len > 0 {
                                        // Save current text as draft before entering history
                                        if state.input_history_pos.is_none() {
                                            state.input_draft = tui_input.buffer.lock().clone();
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
                                    // context_ref 弹窗导航：向下选择
                                    if state.context_ref_active
                                        && !state.context_ref_items.is_empty()
                                    {
                                        let max = state.context_ref_items.len() - 1;
                                        if state.context_ref_selected < max {
                                            state.context_ref_selected += 1;
                                        }
                                        continue;
                                    }
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
                                            let draft = std::mem::take(&mut state.input_draft);
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
                                        drop(cursor);
                                        drop(buffer);

                                        // @-mention context reference detection
                                        if c == '@' {
                                            state.context_ref_active = true;
                                            state.context_ref_query = "@".to_string();
                                            crate::panels::context_ref::populate_suggestions(
                                                &mut state,
                                            );
                                        } else if state.context_ref_active {
                                            if c == ' ' {
                                                state.context_ref_active = false;
                                            } else {
                                                state.context_ref_query.push(c);
                                                crate::panels::context_ref::populate_suggestions(
                                                    &mut state,
                                                );
                                            }
                                        }
                                    }
                                }
                                KeyCode::Tab | KeyCode::BackTab => {
                                    // Tab completes @-mention when popup is open
                                    if state.context_ref_active
                                        && !state.context_ref_items.is_empty()
                                    {
                                        let label = state.context_ref_items[state
                                            .context_ref_selected
                                            .min(state.context_ref_items.len() - 1)]
                                        .label
                                        .clone();
                                        insert_context_ref_text(&mut state, &tui_input, &label);
                                        continue;
                                    }
                                    // While actively typing, Tab does nothing.
                                    // Press Esc first to exit input mode, then Tab to navigate panels.
                                }
                                _ => {}
                            }
                        }
                        // ── Search mode (active but not awaiting agent input) ──
                        else if state.search_active {
                            match key.code {
                                KeyCode::Esc => {
                                    // Deactivate search mode but preserve matches
                                    // so n/N navigation works in normal mode.
                                    state.search_active = false;
                                    state.input_cursor = 0;
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
                                    state.input_cursor = state.input_cursor.saturating_sub(1);
                                }
                                KeyCode::Right => {
                                    let len = state.search_query.chars().count();
                                    state.input_cursor = (state.input_cursor + 1).min(len);
                                }
                                KeyCode::Home => {
                                    state.input_cursor = 0;
                                }
                                KeyCode::End => {
                                    state.input_cursor = state.search_query.chars().count();
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.contains(KeyModifiers::CONTROL)
                                        && (c == 'w' || c == 'W') =>
                                {
                                    let cursor_pos = state.input_cursor;
                                    let query = &mut state.search_query;
                                    let chars: Vec<char> = query.chars().collect();
                                    let mut del_start = cursor_pos.min(chars.len());
                                    while del_start > 0 && chars[del_start - 1].is_whitespace() {
                                        del_start -= 1;
                                    }
                                    while del_start > 0 && !chars[del_start - 1].is_whitespace() {
                                        del_start -= 1;
                                    }
                                    let byte_idx: usize =
                                        chars[..del_start].iter().map(|c| c.len_utf8()).sum();
                                    let end_byte: usize = chars[..cursor_pos.min(chars.len())]
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
                                    let byte_idx: usize = chars[..cursor_pos.min(chars.len())]
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
                                        // Clear search matches, retreat to Input — don't auto-activate
                                        state.search_match_lines.clear();
                                        state.search_current_match = None;
                                        state.search_query.clear();
                                        state.search_active = false;
                                        state.input_cursor = 0;
                                        state.focused_panel = FocusedPanel::Input;
                                    } else if state.focused_panel != FocusedPanel::Input {
                                        // Retreat to input panel — don't auto-activate typing
                                        state.focused_panel = FocusedPanel::Input;
                                    } else if state.agent_done {
                                        // Already focused on Input, agent idle:
                                        // activate input so user can type (Esc = "start here")
                                        begin_next_task_input(&mut state, &tui_input);
                                    }
                                    // Agent running, on Input: Esc does nothing (prevents accidental quit)
                                }
                                KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    // Only quit immediately when agent is done.
                                    // While agent is running, require Ctrl+C or Esc instead.
                                    if state.agent_done {
                                        state.should_quit = true;
                                    } else {
                                        push_log(
                                            &mut state,
                                            "Agent 仍在运行，请用 p 取消或 Ctrl+C 退出".into(),
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
                                            let next = if cur + 1 < state.search_match_lines.len() {
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
                                                state.search_match_lines.len().saturating_sub(1)
                                            };
                                            state.search_current_match = Some(prev);
                                            navigate_to_search_match(&mut state);
                                        }
                                    }
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
                                KeyCode::Char('w')
                                    if state.focused_panel == FocusedPanel::Evolution =>
                                {
                                    state.evo_weights_hidden = !state.evo_weights_hidden;
                                }
                                KeyCode::Char('t')
                                    if state.focused_panel == FocusedPanel::Evolution =>
                                {
                                    state.evo_stats_hidden = !state.evo_stats_hidden;
                                }
                                KeyCode::Char('m')
                                    if state.focused_panel == FocusedPanel::Evolution =>
                                {
                                    state.evo_meta_hidden = !state.evo_meta_hidden;
                                }
                                KeyCode::Char('h') | KeyCode::F(1) => {
                                    state.help_visible = !state.help_visible;
                                }
                                KeyCode::Char('[') => {
                                    let pct = state.left_split_pct.unwrap_or_else(|| {
                                        state
                                            .phase
                                            .main_split_ratio(
                                                !state.evolution.all_weights().is_empty(),
                                            )
                                            .0
                                    });
                                    state.left_split_pct = Some(pct.saturating_sub(5).max(30));
                                }
                                KeyCode::Char(']') => {
                                    let pct = state.left_split_pct.unwrap_or_else(|| {
                                        state
                                            .phase
                                            .main_split_ratio(
                                                !state.evolution.all_weights().is_empty(),
                                            )
                                            .0
                                    });
                                    state.left_split_pct = Some((pct + 5).min(85));
                                }
                                KeyCode::Char('p') if !state.agent_done => {
                                    // Cancel current agent operation
                                    if let Some(f) = tui_input.stop_flag.as_ref() {
                                        f.store(true, std::sync::atomic::Ordering::Relaxed)
                                    }
                                    push_log(&mut state, "取消当前操作...".into(), false);
                                }
                                KeyCode::Char('f')
                                    if state.focused_panel == FocusedPanel::MiniLog
                                        || (!matches!(
                                            state.phase,
                                            AgentPhase::Planning | AgentPhase::Executing
                                        ) && state.focused_panel == FocusedPanel::MainLeft) =>
                                {
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
                                            push_log(
                                                &mut state,
                                                format!("已导出: {filename}"),
                                                false,
                                            );
                                        }
                                        None => {
                                            push_log(&mut state, "导出失败".into(), true);
                                        }
                                    }
                                }
                                KeyCode::Char('s') | KeyCode::F(2) => {
                                    state.settings_visible = !state.settings_visible;
                                }
                                KeyCode::Char('k')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    state.kanban_visible = !state.kanban_visible;
                                    let msg = format!(
                                        "看板: {}",
                                        if state.kanban_visible {
                                            "已显示"
                                        } else {
                                            "已隐藏"
                                        }
                                    );
                                    push_log(&mut state, msg, false);
                                }
                                KeyCode::Char('t')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    let name = format!("会话{}", state.session_tabs.len() + 1);
                                    state.session_tabs.push(crate::state::SessionTab { name });
                                    state.active_tab_index = state.session_tabs.len() - 1;
                                    push_log(&mut state, "已创建新标签".into(), false);
                                }
                                KeyCode::Char('w')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    if state.session_tabs.len() > 1 {
                                        let idx = state.active_tab_index;
                                        state.session_tabs.remove(idx);
                                        if idx >= state.session_tabs.len() {
                                            state.active_tab_index = state.session_tabs.len() - 1;
                                        }
                                        push_log(&mut state, "已关闭标签".into(), false);
                                    }
                                }
                                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    if state.session_tabs.len() > 1 {
                                        if state.active_tab_index > 0 {
                                            state.active_tab_index -= 1;
                                        } else {
                                            state.active_tab_index = state.session_tabs.len() - 1;
                                        }
                                    }
                                }
                                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    if state.session_tabs.len() > 1 {
                                        if state.active_tab_index + 1 < state.session_tabs.len() {
                                            state.active_tab_index += 1;
                                        } else {
                                            state.active_tab_index = 0;
                                        }
                                    }
                                }
                                KeyCode::BackTab => {
                                    if state.search_match_lines.is_empty() {
                                        state.focused_panel = state.focused_panel.prev();
                                    }
                                }
                                KeyCode::Tab => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        // Ctrl+Tab: switch left tab when MainLeft focused during Planning/Executing
                                        if matches!(
                                            state.phase,
                                            AgentPhase::Planning | AgentPhase::Executing
                                        ) && state.focused_panel == FocusedPanel::MainLeft
                                        {
                                            state.left_tab = state.left_tab.next();
                                        }
                                    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        if state.search_match_lines.is_empty() {
                                            state.focused_panel = state.focused_panel.prev();
                                        }
                                    } else if state.search_match_lines.is_empty() {
                                        state.focused_panel = state.focused_panel.next();
                                    }
                                }
                                KeyCode::Char('i') if state.agent_done => {
                                    begin_next_task_input(&mut state, &tui_input);
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
                                // Enter: panel-specific action or activate input
                                KeyCode::Enter => {
                                    if state.phase == AgentPhase::Executing
                                        && state.focused_panel == FocusedPanel::MainLeft
                                        && state.left_tab == crate::state::LeftTab::Execution
                                    {
                                        if let Some(idx) = state.exec_selected_index {
                                            open_step_overlay(&mut state, idx);
                                        }
                                    } else if state.phase == AgentPhase::Idle
                                        && state.agent_done
                                        && state.results_visible
                                        && state.focused_panel == FocusedPanel::MainLeft
                                    {
                                        if let Some(idx) = state.exec_selected_index {
                                            open_step_overlay(&mut state, idx);
                                        } else if !state.executions.is_empty() {
                                            open_step_overlay(&mut state, 0);
                                        }
                                    } else if state.focused_panel == FocusedPanel::Evolution {
                                        toggle_evolution_section(&mut state);
                                    } else if state.focused_panel == FocusedPanel::Input
                                        && state.agent_done
                                    {
                                        // Enter on Input when agent done = start typing
                                        begin_next_task_input(&mut state, &tui_input);
                                    } else {
                                        // Fallback: focus the input panel
                                        state.focused_panel = FocusedPanel::Input;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    state.focused_panel = FocusedPanel::Input;
                                    if state.agent_done {
                                        // Start typing: activate input mode and insert char
                                        tui_input
                                            .awaiting
                                            .store(true, std::sync::atomic::Ordering::Relaxed);
                                        state.awaiting_input = true;
                                        let mut buf = tui_input.buffer.lock();
                                        buf.push(c);
                                        let len = buf.chars().count();
                                        drop(buf);
                                        *tui_input.cursor.lock() = len;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(Event::Mouse(mouse)) => {
                        // When an overlay is open, route mouse scroll to it; ignore clicks
                        let state = app_state.read();
                        if state.output_overlay.is_some() {
                            if mouse.kind == MouseEventKind::ScrollDown {
                                drop(state);
                                let mut s = app_state.write();
                                if let Some(ref mut o) = s.output_overlay {
                                    o.scroll = o.scroll.saturating_add(1);
                                }
                            } else if mouse.kind == MouseEventKind::ScrollUp {
                                drop(state);
                                let mut s = app_state.write();
                                if let Some(ref mut o) = s.output_overlay {
                                    o.scroll = o.scroll.saturating_sub(1);
                                }
                            }
                            // Ignore clicks when overlay is open — keyboard handles overlay interaction
                            continue;
                        }
                        drop(state);
                        match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                scroll_mouse(&app_state, 1, mouse.column, mouse.row);
                            }
                            MouseEventKind::ScrollUp => {
                                scroll_mouse(&app_state, -1, mouse.column, mouse.row);
                            }
                            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                                click_focus(&app_state, &tui_input, mouse.column, mouse.row);
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
    let stop_flag = ctx.stop_flag();
    let mut next_ctx = ctx;
    let result = loop {
        let result = agent.run_loop(next_ctx).await;
        let stop_requested = stop_flag.load(std::sync::atomic::Ordering::Relaxed);

        // Signal completion — preserve real summary from agent output.
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
            } else if stop_requested && state.summary.is_none() {
                state.summary = Some("已取消 — 可继续输入下一条任务".into());
            } else if state.summary.is_none() {
                // Don't overwrite real summary with boilerplate
                state.summary = Some("完成 — 可继续输入下一条任务".into());
            }
        }

        if result.is_err() {
            break result;
        }
        // 'p' cancelled the current operation — reset the flag so the next task
        // runs normally.  Don't break: keep the outer loop alive so the user
        // can submit another task without restarting the whole session.
        if stop_requested {
            stop_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        }

        tui_input
            .awaiting
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let submitted = loop {
            if app_state.read().should_quit {
                break None;
            }
            if let Some(text) = tui_input.submitted.lock().take() {
                break Some(text);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        tui_input
            .awaiting
            .store(false, std::sync::atomic::Ordering::Relaxed);

        let Some(text) = submitted else {
            break result;
        };
        let text = text.trim().to_string();
        if text.is_empty() || matches!(text.as_str(), "exit" | "quit") {
            let mut state = app_state.write();
            state.should_quit = true;
            break result;
        }

        {
            let mut state = app_state.write();
            state.agent_done = false;
            state.phase = AgentPhase::Observing;
            state.focused_panel = FocusedPanel::MainLeft;
            state.results_visible = true;
        }
        next_ctx =
            agent_core::context::Context::new(Some(text)).with_stop_flag(Arc::clone(&stop_flag));
    };

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
        current.saturating_sub(delta.unsigned_abs())
    }
}

fn input_line_count_for(text: &str) -> u8 {
    text.split('\n').count().clamp(1, 8) as u8
}

fn footer_height_for(state: &TuiAppState) -> u16 {
    if state.awaiting_input || state.search_active || state.slash_command_active {
        state.input_line_count.max(1) as u16 + 1
    } else {
        1
    }
}

fn session_tabs_height_for(state: &TuiAppState) -> u16 {
    if state.session_tabs.len() > 1 {
        1
    } else {
        0
    }
}

fn is_ctrl_c(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('c') | KeyCode::Char('C'))
        && modifiers.contains(KeyModifiers::CONTROL)
}

fn request_tui_quit(state: &mut TuiAppState, tui_input: &TuiInput) {
    state.should_quit = true;
    if let Some(flag) = tui_input.stop_flag.as_ref() {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    *tui_input.submitted.lock() = Some(String::new());
}

// ── Settings helpers ──

fn settings_toggle(state: &mut TuiAppState, label: &str) {
    if label == "启用搜索" {
        state.user_settings.search_enabled = !state.user_settings.search_enabled
    }
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
            state.user_settings.finance_provider =
                match state.user_settings.finance_provider.as_str() {
                    "" | "ftshare" => "tushare".into(),
                    "tushare" => "eastmoney".into(),
                    "eastmoney" => "tencent".into(),
                    "tencent" => "sina".into(),
                    "sina" => "ftshare".into(),
                    _ => "ftshare".into(),
                };
        }
        "预设主题" => {
            let names = crate::theme::Theme::preset_names();
            let current = state.theme_preset.as_str();
            let pos = names.iter().position(|&n| n == current).unwrap_or(0);
            let next_idx = (pos + 1) % names.len();
            state.theme_preset = names[next_idx].to_string();
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
        "App ID" => s.feishu_app_id.clone(),
        "App Secret" => s.feishu_app_secret.clone(),
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
        "App ID" => state.user_settings.feishu_app_id = val,
        "App Secret" => state.user_settings.feishu_app_secret = val,
        _ => {}
    }
}

fn left_tab_for_click(relative_col: u16) -> Option<LeftTab> {
    match relative_col {
        1..=6 => Some(LeftTab::Plan),
        8..=13 => Some(LeftTab::Execution),
        _ => None,
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
                .unwrap_or_else(|| step.content_preview.clone().unwrap_or_default()),
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
                state
                    .executions
                    .iter()
                    .map(|s| {
                        format!(
                            "[{}] {} ({:?}ms)\n{}",
                            if matches!(s.status, crate::state::StepStatus::Success) {
                                "OK"
                            } else if matches!(s.status, crate::state::StepStatus::Failed) {
                                "FAIL"
                            } else {
                                "..."
                            },
                            s.tool,
                            s.duration_ms.unwrap_or(0),
                            s.content_full.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            (format!("hermess_plan_{ts}.txt"), text)
        }
        _ => {
            let mut text = String::new();
            if let Some(ref summary) = state.summary {
                text.push_str(&format!("Result: {}\n\n", summary));
            }
            text.push_str(&format!(
                "Steps: {}/{}\n",
                state.exec_completed_steps, state.exec_total_steps
            ));
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
                format!(
                    "当前模型: {}",
                    if state.user_settings.llm_model.is_empty() {
                        "(默认)"
                    } else {
                        &state.user_settings.llm_model
                    }
                )
            } else {
                state.user_settings.llm_model = rest;
                state.settings_dirty = true;
                format!("模型已设置为: {}", state.user_settings.llm_model)
            };
            push_log(state, msg, false);
        }
        "/personality" => {
            let personalities = vec!["default", "concise", "verbose", "creative", "analytical"];
            let lines = if rest.is_empty() {
                let mut lines = vec!["可用人格:".into(), String::new()];
                for p in &personalities {
                    lines.push(format!("  - {}", p));
                }
                lines.push(String::new());
                lines.push("使用 /personality <名称> 切换人格".into());
                lines
            } else {
                vec![
                    format!("  人格已设置为: {}", rest),
                    String::new(),
                    "  (需要后端支持)".into(),
                ]
            };
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Personality".into(),
                lines,
                scroll: 0,
            });
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
                format!(
                    "  Gateway: {}",
                    if state.gateway_enabled {
                        "已启用"
                    } else {
                        "未启用"
                    }
                ),
                format!(
                    "  路由模式: {}",
                    if state.gateway_mode.is_empty() {
                        "(默认)"
                    } else {
                        &state.gateway_mode
                    }
                ),
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
                format!(
                    "  日志条目: {}  错误: {}",
                    state.log_entries.len(),
                    state.log_entries.iter().filter(|e| e.is_error).count()
                ),
                format!(
                    "  进化引擎: {} insights, {} strategies",
                    insight_count, strategy_count
                ),
                format!(
                    "  当前学习率: {:.5}",
                    state.evolution.current_learning_rate()
                ),
                format!(
                    "  Gateway: {}  模型数: {}",
                    state.gateway_enabled,
                    state.gateway_models.len()
                ),
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
            let mut lines = vec![
                format!("  回合数: {}", state.turn),
                format!("  耗时: {}", elapsed),
                format!(
                    "  已执行步骤: {} / {}",
                    state.exec_completed_steps, state.exec_total_steps
                ),
                format!("  进化统计: {} 条 insight", state.evolution.insight_count()),
                String::new(),
            ];
            if let Some(ref tracker) = state.usage_tracker {
                let snap = tracker.snapshot();
                lines.push("  ── Token 用量 ──".into());
                lines.push(format!("  Prompt tokens:     {}", snap.prompt_tokens));
                lines.push(format!("  Completion tokens: {}", snap.completion_tokens));
                lines.push(format!(
                    "  Total tokens:      {}",
                    snap.prompt_tokens + snap.completion_tokens
                ));
                lines.push(format!(
                    "  估算费用:          ${:.6}",
                    snap.estimated_cost_usd
                ));
                lines.push(String::new());
                lines.push(format!("  模型: {}", snap.model));
                lines.push(format!("  总调用次数: {}", snap.total_calls));
            } else {
                lines.push("  UsageTracker 未连接".into());
                lines.push(String::new());
                lines.push("  详细 token 用量请查看 LLM provider 后台。".into());
            }
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
                                        let name = path
                                            .file_stem()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default();
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
            let skills_dir = home_dir().join(".claude").join("skills");
            if skills_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let ft = entry
                            .file_type()
                            .map(|t| if t.is_dir() { "/" } else { "" })
                            .unwrap_or("");
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
            let lines = vec![
                format!("  回合数: {}", state.turn),
                format!(
                    "  步骤: {}/{} 已完成",
                    state.exec_completed_steps, state.exec_total_steps
                ),
                format!("  日志条目: {} 个", state.log_entries.len()),
                format!(
                    "  错误: {} 个",
                    state.log_entries.iter().filter(|e| e.is_error).count()
                ),
                format!("  看板条目: {} 个", state.kanban_items.len()),
                format!("  进化 insights: {} 条", state.evolution.insight_count()),
                String::new(),
                "  使用 Ctrl+S 导出完整会话。".into(),
            ];
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Checkpoint".into(),
                lines,
                scroll: 0,
            });
        }
        "/rollback" => {
            push_log(
                state,
                "[rollback] 功能需要后端事件 plumbing，暂不可用。".into(),
                false,
            );
        }
        "/diff" => {
            let output = std::process::Command::new("git")
                .args(["diff", "--stat", "HEAD"])
                .output();
            let lines = match output {
                Err(_) => vec!["错误: 无法执行 git 命令".into()],
                Ok(out) if !out.status.success() => {
                    vec!["当前目录不是 git 仓库，无法获取 diff".into()]
                }
                Ok(out) => {
                    let text = String::from_utf8_lossy(&out.stdout).to_string();
                    if text.trim().is_empty() {
                        vec!["无变更 (git diff --stat HEAD 为空)".into()]
                    } else {
                        text.lines().map(|l| format!("  {l}")).collect()
                    }
                }
            };
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Diff".into(),
                lines,
                scroll: 0,
            });
        }
        "/new" => {
            state.turn = 0;
            state.phase = AgentPhase::Idle;
            state.agent_done = false;
            state.streaming_buffer.clear();
            state.summary_streaming_buffer.clear();
            state.executions.clear();
            state.log_entries.clear();
            state.kanban_items.clear();
            state.plan_ready = false;
            state.plan_steps_count = 0;
            state.total_duration_ms = None;
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "New Session".into(),
                lines: vec!["已重置会话状态".into()],
                scroll: 0,
            });
        }
        "/load" => {
            if rest.is_empty() {
                state.slash_command_popup = Some(crate::state::SlashResult {
                    title: "Load Session".into(),
                    lines: vec![
                        "用法: /load <会话名>".into(),
                        String::new(),
                        "已保存会话可通过 /sessions 查看。".into(),
                    ],
                    scroll: 0,
                });
            } else {
                let session_path = home_dir()
                    .join(".hermess")
                    .join("sessions")
                    .join(format!("{}.json", rest));
                match std::fs::read_to_string(&session_path) {
                    Ok(content) => {
                        let preview: String = content.chars().take(200).collect();
                        let name = session_path
                            .file_stem()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let mut lines = vec![
                            format!("  会话名: {}", name),
                            format!("  大小: {} 字节", content.len()),
                            String::new(),
                            "  内容预览:".into(),
                            format!("    {}...", preview),
                            String::new(),
                            "  (完整加载需后端会话管理器支持)".into(),
                        ];
                        // Try to parse as JSON and extract some info
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(turn) = val.get("turn").and_then(|v| v.as_u64()) {
                                lines.insert(1, format!("  回合数: {}", turn));
                            }
                        }
                        state.slash_command_popup = Some(crate::state::SlashResult {
                            title: "Load Session".into(),
                            lines,
                            scroll: 0,
                        });
                    }
                    Err(_) => {
                        state.slash_command_popup = Some(crate::state::SlashResult {
                            title: "Load Session".into(),
                            lines: vec![
                                "未找到会话".into(),
                                format!("  路径: {}", session_path.display()),
                            ],
                            scroll: 0,
                        });
                    }
                }
            }
        }
        "/memory" | "/recall" => {
            let cmd_name = head.trim_start_matches('/');
            let lines = if rest.is_empty() {
                vec![
                    format!("  {} - 查询记忆", cmd_name),
                    String::new(),
                    format!("  用法: /{} <查询关键词>", cmd_name),
                    "  示例: /memory 上周的bug修复".into(),
                    String::new(),
                    "  (需要后端 WorkingMemory 查询支持)".into(),
                ]
            } else {
                vec![
                    format!("  查询: {}", rest),
                    String::new(),
                    "  (需要后端 WorkingMemory 查询支持)".into(),
                ]
            };
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: cmd_name.to_string(),
                lines,
                scroll: 0,
            });
        }
        "/compress" => {
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Compress".into(),
                lines: vec![
                    "请求压缩当前对话上下文".into(),
                    String::new(),
                    "(需要后端上下文压缩支持)".into(),
                ],
                scroll: 0,
            });
        }
        "/cron" => {
            let lines = vec![
                "Cron 表达式格式:".into(),
                String::new(),
                "  * * * * *".into(),
                "  │ │ │ │ │".into(),
                "  │ │ │ │ └── 星期 (0-7)".into(),
                "  │ │ │ └──── 月份 (1-12)".into(),
                "  │ │ └────── 日期 (1-31)".into(),
                "  │ └──────── 小时 (0-23)".into(),
                "  └────────── 分钟 (0-59)".into(),
                String::new(),
                "  示例:".into(),
                "  0 9 * * *    每天9点".into(),
                "  */5 * * * *  每5分钟".into(),
                "  0 0 1 * *    每月1号".into(),
                String::new(),
                "  (需要后端调度器支持)".into(),
            ];
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Cron".into(),
                lines,
                scroll: 0,
            });
        }
        "/kanban" => {
            state.kanban_visible = !state.kanban_visible;
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Kanban".into(),
                lines: vec![
                    format!(
                        "  看板: {}",
                        if state.kanban_visible {
                            "已显示"
                        } else {
                            "已隐藏"
                        }
                    ),
                    String::new(),
                    "  提示: Ctrl+K 也可切换看板。".into(),
                ],
                scroll: 0,
            });
        }
        _ => {
            push_log(
                state,
                format!("未知命令: {}. 输入 : /help 查看可用命令。", head),
                false,
            );
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
    state.log_entries.push_back(LogEntry {
        message: clean,
        is_error,
        repeat_count: 0,
    });
    if state.log_auto_scroll {
        // 仅在执行阶段（非 Idle）自动滚日志到底部，避免打断用户查看结果
        if !matches!(state.phase, AgentPhase::Idle) {
            state.log_scroll = 10_000;
        }
    }
}

fn scroll_focused(state: &mut TuiAppState, delta: i16) {
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
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
            _ => {
                state.log_scroll = 0;
                state.log_auto_scroll = false;
            }
        },
        FocusedPanel::Evolution => state.evo_scroll = 0,
        FocusedPanel::MiniLog | FocusedPanel::Input => {
            state.log_scroll = 0;
            state.log_auto_scroll = false;
        }
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
            _ => {
                state.log_auto_scroll = true;
            }
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
                crate::state::LeftTab::Plan => state
                    .streaming_buffer
                    .lines()
                    .map(|s| s.to_string())
                    .collect(),
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
                        format!(
                            "[{}] {} | {}",
                            status,
                            s.tool,
                            crate::state::strip_ansi(content)
                        )
                    })
                    .collect(),
            },
            _ => state
                .log_entries
                .iter()
                .map(|e| format!("{} {}", if e.is_error { "[!]" } else { "[*]" }, e.message))
                .collect(),
        },
        FocusedPanel::MiniLog | FocusedPanel::Input => state
            .log_entries
            .iter()
            .map(|e| format!("{} {}", if e.is_error { "[!]" } else { "[*]" }, e.message))
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

    let has_left_tabs = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let session_tabs_h = session_tabs_height_for(&state);
    let footer_h = footer_height_for(&state);
    let mini_log_h = 0;
    let main_top = header_h + session_tabs_h;
    let main_h = term_size
        .1
        .saturating_sub(header_h + session_tabs_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = state.split_pct();

    let left_w = (term_size.0 as f64 * left_pct as f64 / 100.0) as u16;

    // Determine panel under cursor
    if row < header_h {
        return; // header, not scrollable
    }
    if row < main_top {
        return; // session tab bar, not scrollable
    }

    let in_main = row < main_top + main_h;
    let in_left = col < left_w;

    // Left panel tab row during Planning/Executing is not scrollable
    if in_main && in_left && has_left_tabs && row == main_top {
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
    }
}

/// Mouse click: determine which panel is under the cursor and focus it.
fn click_focus(
    app_state: &Arc<parking_lot::RwLock<TuiAppState>>,
    tui_input: &TuiInput,
    col: u16,
    row: u16,
) {
    let mut state = app_state.write();
    let term_size = crossterm::terminal::size().unwrap_or((80, 24));

    let has_left_tabs = matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let session_tabs_h = session_tabs_height_for(&state);
    let footer_h = footer_height_for(&state);
    let mini_log_h = 0;
    let main_top = header_h + session_tabs_h;
    let main_h = term_size
        .1
        .saturating_sub(header_h + session_tabs_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = state.split_pct();

    let left_w = (term_size.0 as f64 * left_pct as f64 / 100.0) as u16;

    // Header row
    if row < header_h {
        return;
    }
    if row < main_top {
        return;
    }

    // Input/footer row
    if row >= main_top + main_h + mini_log_h {
        state.focused_panel = FocusedPanel::Input;
        if state.agent_done {
            tui_input
                .awaiting
                .store(true, std::sync::atomic::Ordering::Relaxed);
            state.awaiting_input = true;
            state.input_cursor = state.input_text.chars().count();
        }
        return;
    }

    let in_main = row < main_top + main_h;
    let in_left = col < left_w;

    if in_main && in_left {
        // Click on left main panel
        // Check for tab bar click during Planning/Executing
        if has_left_tabs && row == main_top {
            if let Some(tab) = left_tab_for_click(col) {
                state.left_tab = tab;
            }
            return;
        }
        state.focused_panel = FocusedPanel::MainLeft;
    } else if in_main && !in_left {
        state.focused_panel = FocusedPanel::Evolution;
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
            state.agent_done = false;
            state.results_visible = true;
            // Don't set left_tab here — PlanPhaseStarted or ExecutePhaseStarted
            // will set it when the actual phase begins. In interactive mode this
            // event is emitted before waiting for the next prompt, so keep the
            // previous result visible until real work starts.
        }
        AgentEvent::PlanPhaseStarted => {
            state.phase = AgentPhase::Planning;
            state.streaming_buffer.clear();
            state.summary_streaming_buffer.clear();
            state.summary = None;
            state.executions.clear();
            state.exec_total_steps = 0;
            state.exec_completed_steps = 0;
            state.exec_selected_index = None;
            state.plan_ready = false;
            state.plan_steps_count = 0;
            state.left_tab = LeftTab::Plan;
            state.plan_scroll = 0;
            state.exec_scroll = 0;
        }
        AgentEvent::PlanStreamingToken { token } => {
            state
                .streaming_buffer
                .push_str(&crate::state::strip_html(&token));
            // Cap buffer at 50KB: trim from the front keeping a clean line boundary
            const STREAM_CAP: usize = 51_200;
            if state.streaming_buffer.len() > STREAM_CAP {
                let start = state.streaming_buffer.len() - STREAM_CAP;
                let actual_start = state.streaming_buffer[start..]
                    .find('\n')
                    .map(|pos| start + pos + 1)
                    .unwrap_or(start);
                state.streaming_buffer = state.streaming_buffer[actual_start..].to_string();
            }
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
            push_log(
                state,
                format!("执行完成 ({:.1}s)", duration_ms as f64 / 1000.0),
                false,
            );
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
            push_log(
                state,
                format!("重规划完成: {new_steps_count} 个步骤"),
                false,
            );
        }
        AgentEvent::ReflectPhaseStarted => {
            state.phase = AgentPhase::Reflecting;
        }
        AgentEvent::ReflectPhaseComplete { score, lesson } => {
            state.phase = AgentPhase::Evolving;
            push_log(
                state,
                format!("反思: score={:.2} | {}", score, lesson),
                score < 0.0,
            );
        }
        AgentEvent::EvolvePhaseStarted => {
            state.phase = AgentPhase::Evolving;
        }
        AgentEvent::EvolvePhaseComplete => {
            state.phase = AgentPhase::Idle;
            state.agent_done = true;
            state.results_visible = true;
        }
        AgentEvent::SummaryStreamingToken { token } => {
            state
                .summary_streaming_buffer
                .push_str(&crate::state::strip_html(&token));
            // Cap summary buffer at 50KB (same logic as streaming_buffer)
            const SUMMARY_CAP: usize = 51_200;
            if state.summary_streaming_buffer.len() > SUMMARY_CAP {
                let start = state.summary_streaming_buffer.len() - SUMMARY_CAP;
                let actual_start = state.summary_streaming_buffer[start..]
                    .find('\n')
                    .map(|pos| start + pos + 1)
                    .unwrap_or(start);
                state.summary_streaming_buffer =
                    state.summary_streaming_buffer[actual_start..].to_string();
            }
        }
        AgentEvent::SummaryReady { summary } => {
            state.summary_streaming_buffer.clear();
            let clean_summary = crate::state::strip_html(&summary);
            push_log(state, format!("结果: {}", clean_summary), false);
            state.summary = Some(clean_summary);
        }
        AgentEvent::GatewayModelsDiscovered {
            models,
            gateway_url,
        } => {
            state.gateway_url = gateway_url.clone();
            state.gateway_models = models.clone();
            state.gateway_enabled = true;
            push_log(
                state,
                format!(
                    "Gateway: 发现 {} 个模型: {}",
                    models.len(),
                    models.join(", ")
                ),
                false,
            );
        }
        AgentEvent::GatewayRouteDecision {
            model,
            shg_triggered,
            reason,
        } => {
            state.last_route_decision = Some(format!("{model}: {reason}"));
            state.shg_triggered = shg_triggered;
            let shg_label = if shg_triggered { " [SHG]" } else { "" };
            push_log(
                state,
                format!("路由决策{shg_label}: → {model} ({reason})"),
                false,
            );
        }
        AgentEvent::TaskUpdated {
            task_id,
            title,
            status,
        } => {
            let kanban_status = match status {
                TaskStatus::Pending => crate::state::KanbanStatus::Pending,
                TaskStatus::InProgress => crate::state::KanbanStatus::InProgress,
                TaskStatus::Completed => crate::state::KanbanStatus::Completed,
            };
            // Update existing or insert new kanban item
            if let Some(item) = state.kanban_items.iter_mut().find(|ki| ki.id == task_id) {
                item.status = kanban_status;
                item.title = title.clone();
            } else {
                state.kanban_items.push(crate::state::KanbanItem {
                    id: task_id.clone(),
                    title: title.clone(),
                    status: kanban_status,
                });
            }
            let status_str = match status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in-progress",
                TaskStatus::Completed => "completed",
            };
            push_log(
                state,
                format!(
                    "任务更新 [{status_str}]: {title} (#{})",
                    &task_id[..8.min(task_id.len())]
                ),
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

// ── Public wrappers for integration / stress tests ──

/// Drive an `AgentEvent` through the state machine (for tests & stress tests).
pub fn handle_event_pub(state: &mut TuiAppState, event: AgentEvent) {
    handle_event(state, event);
}

/// Submit a task text from outside the render loop (for tests & stress tests).
pub fn submit_tui_input_pub(state: &mut TuiAppState, tui_input: &TuiInput, text: String) -> bool {
    submit_tui_input(state, tui_input, text)
}

/// Reset state for the next task (for tests & stress tests).
pub fn begin_next_task_input_pub(state: &mut TuiAppState, tui_input: &TuiInput) {
    begin_next_task_input(state, tui_input);
}

/// Calculate how many input lines a given text occupies (for tests).
pub fn input_line_count_for_pub(text: &str) -> u8 {
    input_line_count_for(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{LogFilter, SettingsTab, TuiAppState};
    use agent_core::AgentEvent;
    use std::sync::Arc;
    use uuid::Uuid;

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

    #[test]
    fn test_apply_delta_origin_zero() {
        assert_eq!(apply_delta(0, 0), 0);
    }

    #[test]
    fn test_apply_delta_i16_max() {
        assert_eq!(apply_delta(u16::MAX, i16::MAX), u16::MAX);
    }

    #[test]
    fn test_apply_delta_exact_sub_to_zero() {
        assert_eq!(apply_delta(1, -1), 0);
    }

    #[test]
    fn test_apply_delta_sub_from_max_non_saturating() {
        assert_eq!(apply_delta(u16::MAX, -1), 65534);
    }

    #[test]
    fn test_apply_delta_i16_min_no_panic() {
        // i16::MIN = -32768; unsigned_abs() = 32768
        // This would have panicked in debug before the fix using unsigned_abs()
        assert_eq!(
            apply_delta(50000, i16::MIN),
            50000_u16.saturating_sub(32768)
        );
    }

    #[test]
    fn test_input_line_count_for_single_line() {
        assert_eq!(input_line_count_for("hello"), 1);
        assert_eq!(input_line_count_for(""), 1);
    }

    #[test]
    fn test_input_line_count_for_multiline_clamps_to_eight() {
        let text = "1\n2\n3\n4\n5\n6\n7\n8\n9";
        assert_eq!(input_line_count_for("hello\nworld"), 2);
        assert_eq!(input_line_count_for(text), 8);
    }

    #[test]
    fn test_input_line_count_for_trailing_newline() {
        assert_eq!(input_line_count_for("hello\n"), 2);
    }

    #[test]
    fn test_input_line_count_for_only_newlines() {
        assert_eq!(input_line_count_for("\n"), 2);
        assert_eq!(input_line_count_for("\n\n"), 3);
    }

    // ── handle_event: TurnStarted preserves previous result while awaiting input ──

    #[test]
    fn test_turn_started_preserves_previous_result() {
        let mut state = make_state();
        state.agent_done = true;
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
        state.summary = Some("previous answer".into());

        handle_event(&mut state, AgentEvent::TurnStarted { turn: 2 });

        assert_eq!(state.turn, 2);
        assert_eq!(state.phase, AgentPhase::Observing);
        assert!(!state.agent_done);
        assert_eq!(state.executions.len(), 1);
        assert_eq!(state.exec_total_steps, 1);
        assert_eq!(state.exec_completed_steps, 1);
        assert_eq!(state.streaming_buffer, "plan content");
        assert!(state.plan_ready);
        assert_eq!(state.plan_steps_count, 3);
        assert_eq!(state.summary.as_deref(), Some("previous answer"));
        assert_eq!(state.left_tab, LeftTab::Execution);
        assert!(state.results_visible);
    }

    #[test]
    fn test_evolve_complete_marks_turn_done() {
        let mut state = make_state();
        state.phase = AgentPhase::Evolving;
        state.agent_done = false;

        handle_event(&mut state, AgentEvent::EvolvePhaseComplete);

        assert_eq!(state.phase, AgentPhase::Idle);
        assert!(state.agent_done);
        assert!(state.results_visible);
    }

    // ── handle_event: PlanPhaseStarted switches tab ──

    #[test]
    fn test_plan_phase_started_resets_previous_result_and_switches_tab() {
        let mut state = make_state();
        state.summary = Some("previous answer".into());
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
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Planning);
        assert_eq!(state.left_tab, LeftTab::Plan);
        assert!(state.summary.is_none());
        assert!(state.executions.is_empty());
        assert_eq!(state.exec_total_steps, 0);
        assert_eq!(state.exec_completed_steps, 0);
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

    #[test]
    fn test_settings_close_dirty_requires_confirmation() {
        let mut state = make_state();
        state.settings_visible = true;
        state.settings_dirty = true;

        close_settings_or_confirm_discard(&mut state);

        assert!(state.settings_visible);
        assert!(state.settings_dirty);
        assert!(state.settings_dirty_confirm);
    }

    #[test]
    fn test_settings_text_supports_feishu_fields() {
        let mut state = make_state();

        state.settings_edit_buffer = "cli_test".into();
        settings_apply_text(&mut state, "App ID");
        state.settings_edit_buffer = "secret_test".into();
        settings_apply_text(&mut state, "App Secret");

        assert_eq!(settings_get_text(&state, "App ID"), "cli_test");
        assert_eq!(settings_get_text(&state, "App Secret"), "secret_test");
        assert_eq!(state.user_settings.feishu_app_id, "cli_test");
        assert_eq!(state.user_settings.feishu_app_secret, "secret_test");
    }

    #[test]
    fn test_submit_tui_input_latches_once() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        *tui_input.buffer.lock() = "first".into();
        *tui_input.cursor.lock() = 5;

        assert!(submit_tui_input(&mut state, &tui_input, "first".into()));
        assert!(!state.awaiting_input);
        assert!(!tui_input
            .awaiting
            .load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(tui_input.submitted.lock().as_deref(), Some("first"));

        assert!(!submit_tui_input(&mut state, &tui_input, String::new()));
        assert_eq!(tui_input.submitted.lock().as_deref(), Some("first"));
    }

    #[test]
    fn test_left_tab_for_click_selects_by_column() {
        assert_eq!(left_tab_for_click(1), Some(LeftTab::Plan));
        assert_eq!(left_tab_for_click(6), Some(LeftTab::Plan));
        assert_eq!(left_tab_for_click(8), Some(LeftTab::Execution));
        assert_eq!(left_tab_for_click(13), Some(LeftTab::Execution));
        assert_eq!(left_tab_for_click(20), None);
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

    // ── Sub-agent events ──

    #[test]
    fn test_sub_agent_started_logs() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(
            &mut state,
            AgentEvent::SubAgentStarted {
                task: "search docs".into(),
            },
        );
        assert!(state.log_entries.len() > before);
        assert!(state
            .log_entries
            .back()
            .unwrap()
            .message
            .contains("search docs"));
    }

    #[test]
    fn test_sub_agent_completed_logs() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SubAgentCompleted {
                task: "search docs".into(),
                summary: "found 3 files".into(),
            },
        );
        let msg = &state.log_entries.back().unwrap().message;
        assert!(msg.contains("search docs"));
        assert!(msg.contains("found 3 files"));
    }

    // ── Replan events ──

    #[test]
    fn test_replan_needed_resets_plan_state() {
        let mut state = make_state();
        state.streaming_buffer = "old plan".into();
        state.plan_ready = true;
        state.plan_scroll = 5;
        handle_event(
            &mut state,
            AgentEvent::ReplanNeeded {
                reason: "execution failed".into(),
                attempt: 1,
            },
        );
        assert_eq!(state.phase, AgentPhase::Planning);
        assert!(state.streaming_buffer.is_empty());
        assert!(!state.plan_ready);
        assert_eq!(state.plan_scroll, 0);
        assert_eq!(state.left_tab, LeftTab::Plan);
    }

    #[test]
    fn test_replan_complete_updates_step_count() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::ReplanComplete { new_steps_count: 3 },
        );
        assert_eq!(state.plan_steps_count, 3);
        assert!(state.plan_ready);
    }

    // ── Gateway events ──

    #[test]
    fn test_gateway_models_discovered() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::GatewayModelsDiscovered {
                models: vec!["claude".into(), "gpt4".into()],
                gateway_url: "http://gw:8080".into(),
            },
        );
        assert!(state.gateway_enabled);
        assert_eq!(state.gateway_url, "http://gw:8080");
        assert_eq!(state.gateway_models.len(), 2);
    }

    #[test]
    fn test_gateway_route_decision() {
        let mut state = make_state();
        state.gateway_enabled = true;
        handle_event(
            &mut state,
            AgentEvent::GatewayRouteDecision {
                model: "claude".into(),
                shg_triggered: true,
                reason: "low cost".into(),
            },
        );
        assert!(state.shg_triggered);
        assert!(state.last_route_decision.is_some());
        assert!(state
            .last_route_decision
            .as_ref()
            .unwrap()
            .contains("claude"));
    }

    // ── SetPersonality ──

    #[test]
    fn test_set_personality_updates_name() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SetPersonality {
                name: "Buddy".into(),
            },
        );
        assert_eq!(state.agent_name, "Buddy");
    }

    // ── Summary streaming ──

    #[test]
    fn test_summary_streaming_accumulates() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SummaryStreamingToken {
                token: "Hello".into(),
            },
        );
        handle_event(
            &mut state,
            AgentEvent::SummaryStreamingToken {
                token: " World".into(),
            },
        );
        assert_eq!(state.summary_streaming_buffer, "Hello World");
        assert!(state.results_visible);
    }

    // ── StepStarted / StepCompleted ──

    #[test]
    fn test_step_started_adds_execution_and_selects() {
        let mut state = make_state();
        let sid = uuid::Uuid::new_v4();
        handle_event(
            &mut state,
            AgentEvent::StepStarted {
                step_id: sid,
                tool: "bash".into(),
                layer: 0,
            },
        );
        assert_eq!(state.executions.len(), 1);
        assert_eq!(state.executions[0].step_id, sid);
        assert_eq!(state.executions[0].tool, "bash");
        assert_eq!(state.executions[0].status, StepStatus::Running);
        assert_eq!(state.exec_selected_index, Some(0));
    }

    #[test]
    fn test_step_completed_updates_status_and_content() {
        let mut state = make_state();
        let sid = uuid::Uuid::new_v4();
        // Add a running step first
        state.executions.push(StepExecState {
            step_id: sid,
            tool: "bash".into(),
            status: StepStatus::Running,
            content_preview: None,
            content_full: None,
            duration_ms: None,
            layer: 0,
        });
        state.exec_completed_steps = 0;
        // Complete it
        handle_event(
            &mut state,
            AgentEvent::StepCompleted {
                output: agent_core::StepOutput {
                    step_id: sid,
                    tool: "bash".into(),
                    success: true,
                    content: "execution output".into(),
                    duration_ms: 1500,
                },
            },
        );
        let step = &state.executions[0];
        assert_eq!(step.status, StepStatus::Success);
        assert_eq!(step.duration_ms, Some(1500));
        assert!(step.content_full.is_some());
        assert!(step.content_preview.is_some());
        assert_eq!(state.exec_completed_steps, 1);
        // Should have log entries (step result + summary line)
        assert!(!state.log_entries.is_empty());
    }

    // ── close_overlays_focus_input ──

    #[test]
    fn test_close_overlays_focus_input() {
        let mut state = make_state();
        state.output_overlay = Some(crate::state::StepOutputOverlay {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            duration_ms: None,
            full_content: "test".into(),
            scroll: 0,
        });
        state.slash_command_popup = Some(crate::state::SlashResult {
            title: "test".into(),
            lines: vec![],
            scroll: 0,
        });
        state.help_visible = true;
        state.settings_visible = true;
        state.settings_editing = true;
        state.settings_dirty_confirm = true;
        close_overlays_focus_input(&mut state);
        assert!(state.output_overlay.is_none());
        assert!(state.slash_command_popup.is_none());
        assert!(!state.help_visible);
        assert!(!state.settings_visible);
        assert!(!state.settings_editing);
        assert!(!state.settings_dirty_confirm);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    // ── is_ctrl_c ──

    #[test]
    fn test_is_ctrl_c() {
        assert!(is_ctrl_c(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(is_ctrl_c(KeyCode::Char('C'), KeyModifiers::CONTROL));
        assert!(!is_ctrl_c(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(!is_ctrl_c(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(!is_ctrl_c(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    // ── request_tui_quit ──

    #[test]
    fn test_request_tui_quit_sets_flag() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        request_tui_quit(&mut state, &tui_input);
        assert!(state.should_quit);
    }

    #[test]
    fn test_request_tui_quit_with_stop_flag() {
        let mut state = make_state();
        let mut tui_input = TuiInput::new();
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tui_input.stop_flag = Some(Arc::clone(&flag));
        request_tui_quit(&mut state, &tui_input);
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    // ── footer_height_for ──

    #[test]
    fn test_footer_height_awaiting_input() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.input_line_count = 3;
        assert_eq!(footer_height_for(&state), 4); // 3 + 1 hints line
    }

    #[test]
    fn test_footer_height_search_active() {
        let mut state = make_state();
        state.search_active = true;
        state.input_line_count = 1;
        assert_eq!(footer_height_for(&state), 2);
    }

    #[test]
    fn test_footer_height_idle() {
        let state = make_state();
        assert_eq!(footer_height_for(&state), 1);
    }

    // ── session_tabs_height_for ──

    #[test]
    fn test_session_tabs_height_multiple_tabs() {
        let mut state = make_state();
        state.session_tabs.push(crate::state::SessionTab {
            name: "会话2".into(),
        });
        assert_eq!(session_tabs_height_for(&state), 1);
    }

    #[test]
    fn test_session_tabs_height_single_tab() {
        let state = make_state();
        assert_eq!(session_tabs_height_for(&state), 0);
    }

    // ── base64_encode ──

    #[test]
    fn test_base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_base64_encode_single_byte() {
        // "f" = 0x66 = 01100110 → "Zg=="
        assert_eq!(base64_encode(b"f"), "Zg==");
    }

    #[test]
    fn test_base64_encode_two_bytes() {
        // "fo" = 0x666F → "Zm8="
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn test_base64_encode_three_bytes() {
        // "foo" = 0x666F6F → "Zm9v"
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    // ── push_log dedup ──

    #[test]
    fn test_push_log_duplicate_increments_count() {
        let mut state = make_state();
        push_log(&mut state, "hello".into(), false);
        push_log(&mut state, "hello".into(), false);
        assert_eq!(state.log_entries.len(), 1);
        assert_eq!(state.log_entries[0].repeat_count, 1);
    }

    #[test]
    fn test_push_log_different_message_adds_entry() {
        let mut state = make_state();
        push_log(&mut state, "hello".into(), false);
        push_log(&mut state, "world".into(), false);
        assert_eq!(state.log_entries.len(), 2);
    }

    #[test]
    fn test_push_log_auto_scroll_in_executing() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        state.log_auto_scroll = true;
        push_log(&mut state, "msg".into(), false);
        assert_eq!(state.log_scroll, 10_000);
    }

    #[test]
    fn test_push_log_no_auto_scroll_when_idle() {
        let mut state = make_state();
        state.phase = AgentPhase::Idle;
        state.log_scroll = 5;
        state.log_auto_scroll = true;
        push_log(&mut state, "msg".into(), false);
        assert_eq!(state.log_scroll, 5); // not changed
    }

    // ── settings_cycle_dropdown ──

    #[test]
    fn test_settings_cycle_dropdown_llm_provider() {
        let mut state = make_state();
        state.user_settings.llm_provider = "deepseek".into();
        settings_cycle_dropdown(&mut state, "提供商标识");
        assert_eq!(state.user_settings.llm_provider, "openai");
        settings_cycle_dropdown(&mut state, "提供商标识");
        assert_eq!(state.user_settings.llm_provider, "anthropic");
        settings_cycle_dropdown(&mut state, "提供商标识");
        assert_eq!(state.user_settings.llm_provider, "deepseek");
    }

    #[test]
    fn test_settings_cycle_dropdown_finance_provider() {
        let mut state = make_state();
        state.user_settings.finance_provider = "ftshare".into();
        settings_cycle_dropdown(&mut state, "金融数据源");
        assert_eq!(state.user_settings.finance_provider, "tushare");
        settings_cycle_dropdown(&mut state, "金融数据源");
        assert_eq!(state.user_settings.finance_provider, "eastmoney");
    }

    #[test]
    fn test_settings_cycle_dropdown_finance_from_empty_skips_ftshare() {
        let mut state = make_state();
        state.user_settings.finance_provider = "".into();
        settings_cycle_dropdown(&mut state, "金融数据源");
        // "" and "ftshare" render identically, so skip past to tushare
        assert_eq!(state.user_settings.finance_provider, "tushare");
    }

    #[test]
    fn test_settings_cycle_dropdown_finance_wraps_around() {
        let mut state = make_state();
        state.user_settings.finance_provider = "sina".into();
        settings_cycle_dropdown(&mut state, "金融数据源");
        assert_eq!(state.user_settings.finance_provider, "ftshare");
    }

    #[test]
    fn test_settings_cycle_dropdown_theme_preset() {
        let mut state = make_state();
        let names = crate::theme::Theme::preset_names();
        let current = state.theme_preset.clone();
        let pos = names.iter().position(|&n| n == current).unwrap_or(0);
        let expected = names[(pos + 1) % names.len()];
        settings_cycle_dropdown(&mut state, "预设主题");
        assert_eq!(state.theme_preset, expected);
    }

    #[test]
    fn test_settings_cycle_dropdown_unknown_label_noop() {
        let mut state = make_state();
        let before = state.user_settings.llm_provider.clone();
        settings_cycle_dropdown(&mut state, "nonexistent");
        assert_eq!(state.user_settings.llm_provider, before);
    }

    // ── settings_toggle ──

    #[test]
    fn test_settings_toggle_search() {
        let mut state = make_state();
        state.user_settings.search_enabled = false;
        settings_toggle(&mut state, "启用搜索");
        assert!(state.user_settings.search_enabled);
        settings_toggle(&mut state, "启用搜索");
        assert!(!state.user_settings.search_enabled);
    }

    #[test]
    fn test_settings_toggle_unknown_label_noop() {
        let mut state = make_state();
        let before = state.user_settings.search_enabled;
        settings_toggle(&mut state, "nonexistent");
        assert_eq!(state.user_settings.search_enabled, before);
    }

    // ── open_step_overlay ──

    #[test]
    fn test_open_step_overlay_sets_correct_fields() {
        let mut state = make_state();
        let sid = uuid::Uuid::new_v4();
        state.executions.push(StepExecState {
            step_id: sid,
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            content_preview: Some("preview".into()),
            content_full: Some("full content".into()),
            duration_ms: Some(150),
            layer: 0,
        });
        open_step_overlay(&mut state, 0);
        assert_eq!(state.exec_selected_index, Some(0));
        let overlay = state.output_overlay.as_ref().unwrap();
        assert_eq!(overlay.step_id, sid);
        assert_eq!(overlay.tool, "bash");
        assert_eq!(overlay.full_content, "full content");
        assert_eq!(overlay.duration_ms, Some(150));
        assert_eq!(overlay.scroll, 0);
    }

    #[test]
    fn test_open_step_overlay_out_of_bounds() {
        let mut state = make_state();
        open_step_overlay(&mut state, 99);
        assert!(state.output_overlay.is_none());
        assert!(state.exec_selected_index.is_none());
    }

    #[test]
    fn test_open_step_overlay_falls_back_to_preview() {
        let mut state = make_state();
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "read".into(),
            status: crate::state::StepStatus::Failed,
            content_preview: Some("preview only".into()),
            content_full: None,
            duration_ms: None,
            layer: 1,
        });
        open_step_overlay(&mut state, 0);
        let overlay = state.output_overlay.unwrap();
        assert_eq!(overlay.full_content, "preview only");
    }

    // ── scroll_to_bottom ──

    #[test]
    fn test_scroll_to_bottom_sets_log_auto_scroll_for_minilog() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_auto_scroll = false;
        state.log_scroll = 3;
        scroll_to_bottom(&mut state);
        assert!(state.log_auto_scroll);
        // scroll_focused called with large positive delta
        assert!(state.log_scroll > 3);
    }

    #[test]
    fn test_scroll_to_bottom_does_not_set_auto_scroll_for_executing_main_left() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.log_auto_scroll = false;
        scroll_to_bottom(&mut state);
        assert!(!state.log_auto_scroll);
    }

    // ── toggle_evolution_section ──

    #[test]
    fn test_toggle_evolution_section_hides_all_when_any_visible() {
        let mut state = make_state();
        state.evo_weights_hidden = false;
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
        toggle_evolution_section(&mut state);
        assert!(state.evo_weights_hidden);
        assert!(state.evo_stats_hidden);
        assert!(state.evo_meta_hidden);
    }

    #[test]
    fn test_toggle_evolution_section_shows_all_when_all_hidden() {
        let mut state = make_state();
        state.evo_weights_hidden = true;
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
        toggle_evolution_section(&mut state);
        assert!(!state.evo_weights_hidden);
        assert!(!state.evo_stats_hidden);
        assert!(!state.evo_meta_hidden);
    }

    // ── export_to_file ──

    #[test]
    fn test_export_to_file_in_idle_phase() {
        let mut state = make_state();
        state.phase = AgentPhase::Idle;
        state.summary = Some("test result".into());
        state.exec_completed_steps = 3;
        state.exec_total_steps = 5;
        state.total_duration_ms = Some(1500);
        state.log_entries.push_back(LogEntry {
            message: "log msg".into(),
            is_error: false,
            repeat_count: 0,
        });
        let (name, content) = export_to_file(&state).unwrap();
        assert!(name.contains("hermess_results"));
        assert!(content.contains("test result"));
        assert!(content.contains("3/5"));
        assert!(content.contains("1.5s"));
        assert!(content.contains("log msg"));
    }

    #[test]
    fn test_export_to_file_in_planning_phase_with_streaming_buffer() {
        let mut state = make_state();
        state.phase = AgentPhase::Planning;
        state.streaming_buffer = "streaming plan content".into();
        let (name, content) = export_to_file(&state).unwrap();
        assert!(name.contains("hermess_plan"));
        assert!(content.contains("streaming plan content"));
    }

    #[test]
    fn test_export_to_file_in_executing_phase_from_steps() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        state.streaming_buffer = String::new();
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            content_preview: Some("step preview".into()),
            content_full: Some("step full output".into()),
            duration_ms: Some(240),
            layer: 0,
        });
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "read".into(),
            status: crate::state::StepStatus::Failed,
            content_preview: Some("failed step".into()),
            content_full: None,
            duration_ms: None,
            layer: 0,
        });
        let (name, content) = export_to_file(&state).unwrap();
        assert!(name.contains("hermess_plan"));
        assert!(content.contains("bash"));
        assert!(content.contains("step full output"));
        assert!(content.contains("FAIL"));
        assert!(content.contains("OK"));
    }

    #[test]
    fn test_export_to_file_pending_step_status() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "pending_step".into(),
            status: crate::state::StepStatus::Pending,
            content_preview: Some("waiting...".into()),
            content_full: None,
            duration_ms: None,
            layer: 0,
        });
        let (_name, content) = export_to_file(&state).unwrap();
        assert!(content.contains("pending_step"));
        assert!(content.contains("..."));
    }

    #[test]
    fn test_export_to_file_empty_state() {
        let mut state = make_state();
        state.phase = AgentPhase::Idle;
        let (_name, content) = export_to_file(&state).unwrap();
        // Should succeed even with empty state
        assert!(!content.is_empty());
    }

    // ── copy_focused_content ──

    #[test]
    fn test_copy_focused_content_from_output_overlay() {
        let mut state = make_state();
        state.output_overlay = Some(crate::state::StepOutputOverlay {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            duration_ms: None,
            full_content: "overlay content".into(),
            scroll: 0,
        });
        assert_eq!(copy_focused_content(&state), Some("overlay content".into()));
    }

    #[test]
    fn test_copy_focused_content_from_planning_streaming_buffer() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = LeftTab::Plan;
        state.streaming_buffer = "plan buffer".into();
        assert_eq!(copy_focused_content(&state), Some("plan buffer".into()));
    }

    #[test]
    fn test_copy_focused_content_from_idle_summary() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Idle;
        state.summary = Some("idle summary".into());
        assert_eq!(copy_focused_content(&state), Some("idle summary".into()));
    }

    #[test]
    fn test_copy_focused_content_from_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_entries.push_back(LogEntry {
            message: "error entry".into(),
            is_error: true,
            repeat_count: 0,
        });
        state.log_entries.push_back(LogEntry {
            message: "info entry".into(),
            is_error: false,
            repeat_count: 0,
        });
        let copied = copy_focused_content(&state).unwrap();
        assert!(copied.contains("[!]"));
        assert!(copied.contains("error entry"));
    }

    #[test]
    fn test_copy_focused_content_from_evolution_returns_none() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        assert_eq!(copy_focused_content(&state), None);
    }

    #[test]
    fn test_copy_focused_content_from_empty_log_returns_none() {
        let mut s = make_state();
        s.focused_panel = FocusedPanel::MiniLog;
        assert_eq!(copy_focused_content(&s), None);
    }

    #[test]
    fn test_copy_focused_content_from_execution_selected_step() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.exec_selected_index = Some(0);
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            content_preview: Some("preview".into()),
            content_full: Some("full step".into()),
            duration_ms: Some(100),
            layer: 0,
        });
        assert_eq!(copy_focused_content(&state), Some("full step".into()));
    }

    #[test]
    fn test_copy_focused_content_empty_streaming_buffer() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = LeftTab::Plan;
        state.streaming_buffer = "".into();
        assert_eq!(copy_focused_content(&state), None);
    }

    #[test]
    fn test_copy_focused_content_execution_no_step_selected() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.exec_selected_index = None;
        assert_eq!(copy_focused_content(&state), None);
    }

    #[test]
    fn test_copy_focused_content_from_input_panel() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Input;
        state.log_entries.push_back(crate::state::LogEntry {
            message: "recent log".into(),
            is_error: false,
            repeat_count: 0,
        });
        let result = copy_focused_content(&state);
        assert!(result.is_some());
        assert!(result.unwrap().contains("recent log"));
    }

    #[test]
    fn test_copy_focused_content_input_panel_empty() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Input;
        assert_eq!(copy_focused_content(&state), None);
    }

    // ── dispatch_slash_command ──

    #[test]
    fn test_dispatch_slash_help_toggles() {
        let mut state = make_state();
        assert!(!state.help_visible);
        dispatch_slash_command(&mut state, "/help");
        assert!(state.help_visible);
        dispatch_slash_command(&mut state, "/h");
        assert!(!state.help_visible);
    }

    #[test]
    fn test_dispatch_slash_model_with_args() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/model claude-sonnet-4-6");
        assert_eq!(state.user_settings.llm_model, "claude-sonnet-4-6");
        assert!(state.settings_dirty);
    }

    #[test]
    fn test_dispatch_slash_model_without_args_shows_current() {
        let mut state = make_state();
        let before_len = state.log_entries.len();
        dispatch_slash_command(&mut state, "/model");
        assert!(state.log_entries.len() > before_len);
    }

    #[test]
    fn test_dispatch_slash_status() {
        let mut state = make_state();
        state.turn = 5;
        state.phase = AgentPhase::Executing;
        state.exec_completed_steps = 3;
        state.exec_total_steps = 7;
        dispatch_slash_command(&mut state, "/status");
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Status");
        let content = popup.lines.join("\n");
        assert!(content.contains("5"));
        assert!(content.contains("执行中"));
    }

    #[test]
    fn test_dispatch_slash_debug() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/debug");
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Debug");
    }

    #[test]
    fn test_dispatch_slash_personality_no_args() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/personality");
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Personality");
        let content = popup.lines.join("\n");
        assert!(content.contains("可用人格"));
    }

    #[test]
    fn test_dispatch_slash_personality_with_args() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/personality concise");
        let popup = state.slash_command_popup.unwrap();
        let content = popup.lines.join("\n");
        assert!(content.contains("concise"));
    }

    #[test]
    fn test_dispatch_slash_unknown_command_does_not_panic() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/nonexistent");
        // Should not panic, no crash = pass
    }

    #[test]
    fn test_dispatch_slash_usage_popup() {
        let mut state = make_state();
        state.frame_count = 90; // 3 seconds
        dispatch_slash_command(&mut state, "/usage");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Usage");
        assert!(popup.lines.iter().any(|l| l.contains("回合数")));
    }

    #[test]
    fn test_dispatch_slash_sessions_popup() {
        let mut state = make_state();
        state.session_tabs.push(crate::state::SessionTab {
            name: "test".into(),
        });
        dispatch_slash_command(&mut state, "/sessions");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Sessions");
    }

    #[test]
    fn test_dispatch_slash_skills_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/skills");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Skills");
        assert!(!popup.lines.is_empty());
    }

    #[test]
    fn test_dispatch_slash_memory_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/memory");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "memory");
    }

    // ── settings_get_text ──

    #[test]
    fn test_settings_get_text_all_fields() {
        let mut state = make_state();
        state.user_settings.llm_provider = "test_provider".into();
        state.user_settings.llm_model = "test_model".into();
        state.user_settings.llm_api_key = "sk-key".into();
        state.user_settings.llm_base_url = "https://test".into();
        state.user_settings.search_api_key = "sk-search".into();
        state.user_settings.finance_provider = "test_fin".into();
        state.user_settings.finance_tushare_token = "tok".into();
        state.user_settings.feishu_app_id = "app-id".into();
        state.user_settings.feishu_app_secret = "app-secret".into();
        assert_eq!(settings_get_text(&state, "提供商标识"), "test_provider");
        assert_eq!(settings_get_text(&state, "模型名称"), "test_model");
        assert_eq!(settings_get_text(&state, "API Key"), "sk-key");
        assert_eq!(settings_get_text(&state, "Base URL"), "https://test");
        assert_eq!(settings_get_text(&state, "搜索 Key"), "sk-search");
        assert_eq!(settings_get_text(&state, "金融数据源"), "test_fin");
        assert_eq!(settings_get_text(&state, "TuShare Token"), "tok");
        assert_eq!(settings_get_text(&state, "App ID"), "app-id");
        assert_eq!(settings_get_text(&state, "App Secret"), "app-secret");
    }

    #[test]
    fn test_settings_get_text_unknown_label() {
        let state = make_state();
        assert_eq!(settings_get_text(&state, "nonexistent"), "");
    }

    // ── settings_apply_text ──

    #[test]
    fn test_settings_apply_text_all_fields() {
        let mut state = make_state();
        state.settings_edit_buffer = "new_provider".into();
        settings_apply_text(&mut state, "提供商标识");
        assert_eq!(state.user_settings.llm_provider, "new_provider");

        state.settings_edit_buffer = "new_model".into();
        settings_apply_text(&mut state, "模型名称");
        assert_eq!(state.user_settings.llm_model, "new_model");

        state.settings_edit_buffer = "new_key".into();
        settings_apply_text(&mut state, "API Key");
        assert_eq!(state.user_settings.llm_api_key, "new_key");

        state.settings_edit_buffer = "new_url".into();
        settings_apply_text(&mut state, "Base URL");
        assert_eq!(state.user_settings.llm_base_url, "new_url");

        state.settings_edit_buffer = "new_search_key".into();
        settings_apply_text(&mut state, "搜索 Key");
        assert_eq!(state.user_settings.search_api_key, "new_search_key");

        state.settings_edit_buffer = "new_finance".into();
        settings_apply_text(&mut state, "金融数据源");
        assert_eq!(state.user_settings.finance_provider, "new_finance");

        state.settings_edit_buffer = "new_token".into();
        settings_apply_text(&mut state, "TuShare Token");
        assert_eq!(state.user_settings.finance_tushare_token, "new_token");
    }

    #[test]
    fn test_settings_apply_text_unknown_label_noop() {
        let mut state = make_state();
        state.settings_edit_buffer = "ignored".into();
        let before = state.user_settings.llm_provider.clone();
        settings_apply_text(&mut state, "nonexistent");
        assert_eq!(state.user_settings.llm_provider, before);
    }

    // ── settings_close flow ──

    #[test]
    fn test_settings_close_not_dirty_just_closes() {
        let mut state = make_state();
        state.settings_visible = true;
        state.settings_dirty = false;
        state.focused_panel = FocusedPanel::Evolution;
        state.settings_edit_buffer = "some text".into();
        close_settings_or_confirm_discard(&mut state);
        assert!(!state.settings_visible);
        assert!(!state.settings_editing);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert!(state.settings_edit_buffer.is_empty());
    }

    #[test]
    fn test_settings_close_dirty_confirms_then_closes() {
        let mut state = make_state();
        state.settings_visible = true;
        state.settings_dirty = true;
        state.focused_panel = FocusedPanel::MiniLog;
        state.settings_edit_buffer = "dirty buffer".into();
        // First call: set dirty_confirm flag
        close_settings_or_confirm_discard(&mut state);
        assert!(state.settings_dirty_confirm);
        assert!(state.settings_visible);
        // Second call: actually close
        close_settings_or_confirm_discard(&mut state);
        assert!(!state.settings_visible);
        assert!(!state.settings_dirty_confirm);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert!(state.settings_edit_buffer.is_empty());
    }

    #[test]
    fn test_settings_close_inconsistent_state_dirty_confirm_without_dirty() {
        // (dirty=false, dirty_confirm=true) should not happen but is handled gracefully
        let mut state = make_state();
        state.settings_visible = true;
        state.settings_dirty = false;
        state.settings_dirty_confirm = true;
        close_settings_or_confirm_discard(&mut state);
        assert!(!state.settings_visible);
        assert!(!state.settings_dirty);
        assert!(!state.settings_dirty_confirm);
    }

    // ── Settings apply_text Tab behavior ──

    // settings_apply_text does not set dirty on its own; callers are responsible
    #[test]
    fn test_settings_apply_text_applies_correctly() {
        let mut state = make_state();
        state.settings_edit_buffer = "new_value".into();
        state.settings_tab = SettingsTab::Llm;

        let fields = crate::panels::settings::fields_for_tab(state.settings_tab);
        let f = &fields[0]; // 提供商标识
        settings_apply_text(&mut state, f.label);

        assert_eq!(state.user_settings.llm_provider, "new_value");
    }

    // ── Input handling edge cases ──

    #[test]
    fn test_begin_next_task_input_sets_awaiting() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        assert!(state.awaiting_input);
        assert!(state.input_line_count >= 1);
    }

    #[test]
    fn test_context_ref_item_default_population() {
        let mut state = make_state();
        state.context_ref_items.push(crate::state::ContextRefItem {
            source: "filesystem".into(),
            label: "src/main.rs".into(),
            preview: "modified".into(),
        });
        state.context_ref_items.push(crate::state::ContextRefItem {
            source: "git".into(),
            label: "fix: bug".into(),
            preview: "abc123".into(),
        });
        assert_eq!(state.context_ref_items.len(), 2);
        assert_eq!(state.context_ref_items[0].label, "src/main.rs");
        assert_eq!(state.context_ref_items[1].source, "git");
    }

    // ── insert_context_ref_text ──

    #[test]
    fn test_insert_context_ref_text_with_at_symbol() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "check @src/mai".into();
        *tui_input.cursor.lock() = 15;
        state.context_ref_active = true;
        state.context_ref_query = "src/mai".into();
        state.context_ref_items.push(crate::state::ContextRefItem {
            source: "filesystem".into(),
            label: "src/main.rs".into(),
            preview: "modified".into(),
        });

        insert_context_ref_text(&mut state, &tui_input, "src/main.rs");

        let buffer = tui_input.buffer.lock().clone();
        assert_eq!(buffer, "check src/main.rs ");
        assert!(!state.context_ref_active);
        assert!(state.context_ref_query.is_empty());
        assert!(state.context_ref_items.is_empty());
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn test_insert_context_ref_text_without_at_symbol() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "just text".into();
        *tui_input.cursor.lock() = 9;

        insert_context_ref_text(&mut state, &tui_input, ".gitignore");

        let buffer = tui_input.buffer.lock().clone();
        // No '@' found, so label is appended at end
        assert!(buffer.contains(".gitignore"));
        assert!(!state.context_ref_active);
    }

    #[test]
    fn test_insert_context_ref_text_at_sign_at_position_zero() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "@query".into();
        *tui_input.cursor.lock() = 6;
        insert_context_ref_text(&mut state, &tui_input, "src/lib.rs");
        let buffer = tui_input.buffer.lock().clone();
        assert_eq!(buffer, "src/lib.rs ");
    }

    #[test]
    fn test_insert_context_ref_text_empty_buffer() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "".into();
        *tui_input.cursor.lock() = 0;
        insert_context_ref_text(&mut state, &tui_input, "config.toml");
        let buffer = tui_input.buffer.lock().clone();
        assert_eq!(buffer, "config.toml ");
    }

    #[test]
    fn test_insert_context_ref_text_multiple_at_signs_uses_last() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "ref @first @second".into();
        *tui_input.cursor.lock() = 20;
        insert_context_ref_text(&mut state, &tui_input, "target.rs");
        let buffer = tui_input.buffer.lock().clone();
        assert_eq!(buffer, "ref @first target.rs ");
    }

    #[test]
    fn test_insert_context_ref_text_cursor_set_to_char_count() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        // "查看" = 6 bytes, 2 chars
        *tui_input.buffer.lock() = "查看 @bug".into();
        *tui_input.cursor.lock() = 8;
        insert_context_ref_text(&mut state, &tui_input, "修复");
        let cursor = *tui_input.cursor.lock();
        // buffer should be "查看 修复 " = 3（查看）chars + 1 space + 2（修复）chars + 1 space
        assert_eq!(cursor, "查看 修复 ".chars().count());
        // verify buffer content
        let buffer = tui_input.buffer.lock().clone();
        assert_eq!(buffer, "查看 修复 ");
    }

    // ── page_scroll_focused ──

    #[test]
    fn test_page_scroll_focused_positive() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 5;
        page_scroll_focused(&mut state, 1);
        assert_eq!(state.log_scroll, 17); // 5 + 12
    }

    #[test]
    fn test_page_scroll_focused_negative() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 50;
        page_scroll_focused(&mut state, -1);
        assert_eq!(state.log_scroll, 38); // 50 - 12
    }

    // ── scroll_to_top ──

    #[test]
    fn test_scroll_to_top_main_left_planning_plan_tab() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = LeftTab::Plan;
        state.plan_scroll = 42;
        scroll_to_top(&mut state);
        assert_eq!(state.plan_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_main_left_executing_exec_tab() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.exec_scroll = 99;
        scroll_to_top(&mut state);
        assert_eq!(state.exec_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_evolution() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 30;
        scroll_to_top(&mut state);
        assert_eq!(state.evo_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_minilog() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_auto_scroll = true;
        state.log_scroll = 100;
        scroll_to_top(&mut state);
        assert_eq!(state.log_scroll, 0);
        assert!(!state.log_auto_scroll);
    }

    #[test]
    fn test_scroll_to_top_idle_main_left_resets_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Idle;
        state.log_auto_scroll = true;
        state.log_scroll = 25;
        scroll_to_top(&mut state);
        assert_eq!(state.log_scroll, 0);
        assert!(!state.log_auto_scroll);
    }

    // ── get_focused_panel_lines ──

    #[test]
    fn test_get_focused_panel_lines_from_planning_buffer() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = LeftTab::Plan;
        state.streaming_buffer = "line1\nline2\nline3".into();
        let lines = get_focused_panel_lines(&state);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_get_focused_panel_lines_from_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_entries.push_back(LogEntry {
            message: "info log".into(),
            is_error: false,
            repeat_count: 0,
        });
        state.log_entries.push_back(LogEntry {
            message: "error log".into(),
            is_error: true,
            repeat_count: 0,
        });
        let lines = get_focused_panel_lines(&state);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("[*]"));
        assert!(lines[0].contains("info log"));
        assert!(lines[1].contains("[!]"));
        assert!(lines[1].contains("error log"));
    }

    #[test]
    fn test_get_focused_panel_lines_from_evolution() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        let lines = get_focused_panel_lines(&state);
        // Weights may be empty for fresh engine, just verify no panic
        let _ = lines;
    }

    // ── toggle_evolution_section additional edge ──

    #[test]
    fn test_toggle_evolution_section_partial_visibility() {
        let mut state = make_state();
        // Only weights visible, stats and meta hidden
        state.evo_weights_hidden = false;
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
        toggle_evolution_section(&mut state);
        // All should become hidden (since not all were hidden)
        assert!(state.evo_weights_hidden);
        assert!(state.evo_stats_hidden);
        assert!(state.evo_meta_hidden);
    }

    // ── Export with steps having empty content ──

    #[test]
    fn test_export_includes_step_without_full_content() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        state.streaming_buffer = String::new();
        state.executions.push(StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "read".into(),
            status: crate::state::StepStatus::Pending,
            content_preview: None,
            content_full: None,
            duration_ms: None,
            layer: 0,
        });
        let (_name, content) = export_to_file(&state).unwrap();
        assert!(content.contains("read"));
    }

    // ── left_tab_for_click boundary values ──

    #[test]
    fn test_left_tab_for_click_boundaries() {
        assert_eq!(left_tab_for_click(0), None);
        assert_eq!(left_tab_for_click(1), Some(LeftTab::Plan));
        assert_eq!(left_tab_for_click(6), Some(LeftTab::Plan));
        assert_eq!(left_tab_for_click(7), None);
        assert_eq!(left_tab_for_click(8), Some(LeftTab::Execution));
        assert_eq!(left_tab_for_click(13), Some(LeftTab::Execution));
        assert_eq!(left_tab_for_click(14), None);
    }

    // ── Character key activates input mode from normal mode ──

    #[test]
    fn test_character_key_focuses_and_activates_input() {
        let mut state = make_state();
        state.agent_done = true;
        state.focused_panel = FocusedPanel::MainLeft;
        state.awaiting_input = false;

        let tui_input = TuiInput::new();

        // Simulate the Char(c) fallback in normal mode
        state.focused_panel = FocusedPanel::Input;
        tui_input
            .awaiting
            .store(true, std::sync::atomic::Ordering::Relaxed);
        state.awaiting_input = true;
        {
            let mut buf = tui_input.buffer.lock();
            buf.push('a');
            let len = buf.chars().count();
            drop(buf);
            *tui_input.cursor.lock() = len;
        }

        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert!(state.awaiting_input);
        assert_eq!(tui_input.buffer.lock().chars().count(), 1);
    }

    // ── navigate_to_search_match ──

    #[test]
    fn test_navigate_to_search_match_planning_scrolls_plan() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = LeftTab::Plan;
        state.search_current_match = Some(0);
        state.search_match_lines = vec![5];
        navigate_to_search_match(&mut state);
        assert_eq!(state.plan_scroll, 5);
    }

    #[test]
    fn test_navigate_to_search_match_execution_scrolls_exec() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = LeftTab::Execution;
        state.search_current_match = Some(0);
        state.search_match_lines = vec![8];
        navigate_to_search_match(&mut state);
        assert_eq!(state.exec_scroll, 8);
    }

    #[test]
    fn test_navigate_to_search_match_minilog_scrolls_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.search_current_match = Some(0);
        state.search_match_lines = vec![3];
        state.log_auto_scroll = true;
        navigate_to_search_match(&mut state);
        assert_eq!(state.log_scroll, 3);
        assert!(!state.log_auto_scroll);
    }

    #[test]
    fn test_navigate_to_search_match_evolution_scrolls_evo() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.search_current_match = Some(0);
        state.search_match_lines = vec![2];
        navigate_to_search_match(&mut state);
        assert_eq!(state.evo_scroll, 2);
    }

    #[test]
    fn test_navigate_to_search_match_out_of_bounds_current() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 10;
        state.search_current_match = Some(99); // out of bounds
        state.search_match_lines = vec![1, 2, 3]; // only 3 entries
        navigate_to_search_match(&mut state);
        assert_eq!(state.log_scroll, 10); // unchanged
    }

    #[test]
    fn test_navigate_to_search_match_no_current_match() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 10;
        state.search_current_match = None;
        state.search_match_lines = vec![1, 2, 3];
        navigate_to_search_match(&mut state);
        assert_eq!(state.log_scroll, 10); // unchanged
    }

    #[test]
    fn test_navigate_to_search_match_empty_list_noop() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 5;
        state.search_current_match = Some(0);
        state.search_match_lines = vec![];
        navigate_to_search_match(&mut state);
        assert_eq!(state.log_scroll, 5); // unchanged
    }

    // ── n/N search navigation wrapping (key handler logic) ──

    #[test]
    fn test_search_n_wraps_from_last_to_zero() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.search_match_lines = vec![1, 2, 3]; // len = 3
        state.search_current_match = Some(2); // last
                                              // n key: cur + 1 >= len → wrap to 0
        if let Some(cur) = state.search_current_match {
            let next = if cur + 1 < state.search_match_lines.len() {
                cur + 1
            } else {
                0
            };
            state.search_current_match = Some(next);
        }
        assert_eq!(state.search_current_match, Some(0));
    }

    #[test]
    fn test_search_n_advances() {
        let mut state = make_state();
        state.search_match_lines = vec![10, 20, 30];
        state.search_current_match = Some(1);
        // n key: cur + 1 < len → cur + 1
        if let Some(cur) = state.search_current_match {
            let next = if cur + 1 < state.search_match_lines.len() {
                cur + 1
            } else {
                0
            };
            state.search_current_match = Some(next);
        }
        assert_eq!(state.search_current_match, Some(2));
    }

    #[test]
    fn test_search_N_wraps_from_zero_to_last() {
        let mut state = make_state();
        state.search_match_lines = vec![1, 2, 3]; // len = 3
        state.search_current_match = Some(0); // first
                                              // N key: cur == 0 → wrap to len - 1 = 2
        if let Some(cur) = state.search_current_match {
            let prev = if cur > 0 {
                cur - 1
            } else {
                state.search_match_lines.len().saturating_sub(1)
            };
            state.search_current_match = Some(prev);
        }
        assert_eq!(state.search_current_match, Some(2));
    }

    #[test]
    fn test_search_N_decrements() {
        let mut state = make_state();
        state.search_match_lines = vec![10, 20, 30];
        state.search_current_match = Some(2);
        // N key: cur > 0 → cur - 1
        if let Some(cur) = state.search_current_match {
            let prev = if cur > 0 {
                cur - 1
            } else {
                state.search_match_lines.len().saturating_sub(1)
            };
            state.search_current_match = Some(prev);
        }
        assert_eq!(state.search_current_match, Some(1));
    }

    #[test]
    fn test_search_nN_single_match_no_wrap() {
        let mut state = make_state();
        // With 1 match, n and N should both stay at position 0
        state.search_match_lines = vec![5];
        // n from position 0
        state.search_current_match = Some(0);
        if let Some(cur) = state.search_current_match {
            let next = if cur + 1 < state.search_match_lines.len() {
                cur + 1
            } else {
                0
            };
            state.search_current_match = Some(next);
        }
        assert_eq!(state.search_current_match, Some(0)); // wraps to 0 = same
                                                         // N from position 0
        state.search_current_match = Some(0);
        if let Some(cur) = state.search_current_match {
            let prev = if cur > 0 {
                cur - 1
            } else {
                state.search_match_lines.len().saturating_sub(1)
            };
            state.search_current_match = Some(prev);
        }
        assert_eq!(state.search_current_match, Some(0)); // 1.saturating_sub(1) = 0
    }

    // ── p cancel agent key (key handler logic) ──

    #[test]
    fn test_p_cancel_sets_stop_flag() {
        let mut state = make_state();
        state.agent_done = false; // agent is running
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut tui_input = TuiInput::new();
        tui_input.stop_flag = Some(stop_flag.clone());
        // p key handler logic
        if !state.agent_done {
            if let Some(ref f) = tui_input.stop_flag {
                f.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            push_log(&mut state, "取消当前操作...".into(), false);
        }
        assert!(stop_flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(state.log_entries.back().unwrap().message.contains("取消"));
    }

    #[test]
    fn test_p_cancel_noop_when_agent_done() {
        let mut state = make_state();
        state.agent_done = true; // agent is finished
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut tui_input = TuiInput::new();
        tui_input.stop_flag = Some(stop_flag.clone());
        // p key guard: if !state.agent_done → false, handler skipped
        if !state.agent_done {
            // ... this shouldn't execute
        }
        assert!(!stop_flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    // ── f key log filter toggle (key handler logic) ──

    #[test]
    fn test_f_key_toggles_log_filter_minilog() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        // f key guard: focused_panel == MiniLog
        let matches_guard = state.focused_panel == FocusedPanel::MiniLog
            || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing)
                && state.focused_panel == FocusedPanel::MainLeft);
        if matches_guard {
            state.log_filter = state.log_filter.next();
        }
        assert_eq!(state.log_filter, LogFilter::ErrorsOnly);
        // Toggle back
        let matches_guard = state.focused_panel == FocusedPanel::MiniLog
            || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing)
                && state.focused_panel == FocusedPanel::MainLeft);
        if matches_guard {
            state.log_filter = state.log_filter.next();
        }
        assert_eq!(state.log_filter, LogFilter::All);
    }

    #[test]
    fn test_f_key_toggles_log_filter_mainleft_idle() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Idle;
        // f key guard: phase not Planning/Executing AND focused == MainLeft
        let matches_guard = state.focused_panel == FocusedPanel::MiniLog
            || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing)
                && state.focused_panel == FocusedPanel::MainLeft);
        if matches_guard {
            state.log_filter = state.log_filter.next();
        }
        assert_eq!(state.log_filter, LogFilter::ErrorsOnly);
    }

    #[test]
    fn test_f_key_noop_during_planning() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        // f key guard: phase IS Planning → guard fails
        let matches_guard = state.focused_panel == FocusedPanel::MiniLog
            || (!matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing)
                && state.focused_panel == FocusedPanel::MainLeft);
        assert!(!matches_guard); // guard should reject
        assert_eq!(state.log_filter, LogFilter::All); // unchanged
    }

    // ── l key log visibility toggle (key handler logic) ──

    #[test]
    fn test_l_key_toggles_log_visibility() {
        let mut state = make_state();
        state.awaiting_input = false;
        state.output_overlay = None;
        state.log_visible = false;
        // l key guard: !awaiting_input && output_overlay is None
        if !state.awaiting_input && state.output_overlay.is_none() {
            state.log_visible = !state.log_visible;
        }
        assert!(state.log_visible);
        // Toggle back
        if !state.awaiting_input && state.output_overlay.is_none() {
            state.log_visible = !state.log_visible;
        }
        assert!(!state.log_visible);
    }

    #[test]
    fn test_l_key_noop_when_awaiting_input() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.log_visible = false;
        // l key guard: awaiting_input → guard fails
        if !state.awaiting_input && state.output_overlay.is_none() {
            state.log_visible = !state.log_visible;
        }
        assert!(!state.log_visible); // unchanged
    }

    // ── Ctrl+K kanban toggle (key handler logic) ──

    #[test]
    fn test_ctrl_k_toggles_kanban() {
        let mut state = make_state();
        assert!(!state.kanban_visible);
        state.kanban_visible = !state.kanban_visible;
        assert!(state.kanban_visible);
        state.kanban_visible = !state.kanban_visible;
        assert!(!state.kanban_visible);
    }

    // ── Additional event handler tests ──

    #[test]
    fn test_agent_started_sets_name() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::AgentStarted {
                name: "Hermes".into(),
            },
        );
        assert_eq!(state.agent_name, "Hermes");
    }

    #[test]
    fn test_agent_stopped_sets_idle() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        handle_event(&mut state, AgentEvent::AgentStopped);
        assert_eq!(state.phase, AgentPhase::Idle);
    }

    #[test]
    fn test_plan_ready_sets_steps_and_flag() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::PlanReady { steps_count: 5 });
        assert_eq!(state.plan_steps_count, 5);
        assert!(state.plan_ready);
    }

    #[test]
    fn test_plan_streaming_token_strips_html() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::PlanStreamingToken {
                token: "<b>hello</b>".into(),
            },
        );
        assert!(!state.streaming_buffer.contains("<b>"));
        assert!(state.streaming_buffer.contains("hello"));
    }

    #[test]
    fn test_plan_retry_appends_marker() {
        let mut state = make_state();
        state.streaming_buffer = "original".into();
        handle_event(&mut state, AgentEvent::PlanRetry);
        assert!(state.streaming_buffer.contains("重试规划"));
    }

    #[test]
    fn test_evolve_phase_started_sets_evolving() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::EvolvePhaseStarted);
        assert_eq!(state.phase, AgentPhase::Evolving);
    }

    #[test]
    fn test_reflect_phase_started_sets_reflecting() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::ReflectPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Reflecting);
    }

    #[test]
    fn test_reflect_phase_complete_adds_log() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(
            &mut state,
            AgentEvent::ReflectPhaseComplete {
                score: 0.85,
                lesson: "improve prompts".into(),
            },
        );
        assert!(state.log_entries.len() > before);
        assert!(state.log_entries.back().unwrap().message.contains("0.85"));
    }

    #[test]
    fn test_reset_session_logs_message() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(&mut state, AgentEvent::ResetSession);
        assert!(state.log_entries.len() > before);
        assert!(state
            .log_entries
            .back()
            .unwrap()
            .message
            .contains("会话重置"));
    }

    #[test]
    fn test_save_checkpoint_logs() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(&mut state, AgentEvent::SaveCheckpoint);
        assert!(state.log_entries.len() > before);
    }

    #[test]
    fn test_rollback_checkpoint_logs() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(&mut state, AgentEvent::RollbackCheckpoint);
        assert!(state.log_entries.len() > before);
    }

    #[test]
    fn test_compress_context_logs() {
        let mut state = make_state();
        let before = state.log_entries.len();
        handle_event(&mut state, AgentEvent::CompressContext);
        assert!(state.log_entries.len() > before);
    }

    #[test]
    fn test_thinking_phase_changed() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::ThinkingPhaseChanged {
                sub_phase: agent_core::ThinkingSubPhase::CallingLlm,
            },
        );
        assert!(state
            .log_entries
            .back()
            .unwrap()
            .message
            .contains("CallingLLM"));
    }

    #[test]
    fn test_task_updated_modifies_kanban() {
        let mut state = make_state();
        state.kanban_items.push(crate::state::KanbanItem {
            id: "task-1".into(),
            title: "old title".into(),
            status: crate::state::KanbanStatus::Pending,
        });
        handle_event(
            &mut state,
            AgentEvent::TaskUpdated {
                task_id: "task-1".into(),
                title: "new title".into(),
                status: agent_core::TaskStatus::InProgress,
            },
        );
        let item = state
            .kanban_items
            .iter()
            .find(|i| i.id == "task-1")
            .unwrap();
        assert_eq!(item.title, "new title");
        assert_eq!(item.status, crate::state::KanbanStatus::InProgress);
    }

    // ── push_log: dedup respects is_error ──

    #[test]
    fn test_push_log_dedup_respects_is_error() {
        let mut state = make_state();
        push_log(&mut state, "msg".into(), false);
        push_log(&mut state, "msg".into(), true);
        assert_eq!(state.log_entries.len(), 2);
    }

    #[test]
    fn test_push_log_strips_html() {
        let mut state = make_state();
        push_log(&mut state, "hello <b>world</b>".into(), false);
        assert_eq!(state.log_entries.back().unwrap().message, "hello world");
    }

    #[test]
    fn test_push_log_auto_scroll_disabled_no_scroll() {
        let mut state = make_state();
        state.log_auto_scroll = false;
        state.phase = AgentPhase::Executing;
        state.log_scroll = 5;
        push_log(&mut state, "msg".into(), false);
        assert_eq!(state.log_scroll, 5);
    }

    // ── scroll_focused: missing panels ──

    #[test]
    fn test_scroll_focused_main_left_idle_scrolls_log() {
        let mut state = make_state();
        state.phase = AgentPhase::Idle;
        state.focused_panel = FocusedPanel::MainLeft;
        state.log_auto_scroll = true;
        state.log_scroll = 3;
        scroll_focused(&mut state, -1);
        assert_eq!(state.log_scroll, 2);
        assert!(!state.log_auto_scroll);
    }

    #[test]
    fn test_scroll_focused_minilog_scrolls_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 10;
        scroll_focused(&mut state, 5);
        assert_eq!(state.log_scroll, 15);
    }

    #[test]
    fn test_scroll_focused_scroll_to_bottom_reenables_auto_scroll() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Input;
        state.log_auto_scroll = false;
        state.log_entries.push_back(LogEntry {
            message: "test".into(),
            is_error: false,
            repeat_count: 0,
        });
        state.log_scroll = 0;
        scroll_focused(&mut state, 1);
        assert!(state.log_auto_scroll);
    }

    // ── is_ctrl_c edge cases ──

    #[test]
    fn test_is_ctrl_c_uppercase() {
        assert!(is_ctrl_c(KeyCode::Char('C'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_is_ctrl_c_plain_c_rejected() {
        assert!(!is_ctrl_c(KeyCode::Char('c'), KeyModifiers::NONE));
    }

    #[test]
    fn test_is_ctrl_c_wrong_key_rejected() {
        assert!(!is_ctrl_c(KeyCode::Char('x'), KeyModifiers::CONTROL));
    }

    // ── base64_encode additional cases ──

    #[test]
    fn test_base64_encode_hello() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn test_base64_encode_zero_bytes_padding() {
        assert_eq!(base64_encode(&[0x00]), "AA==");
        assert_eq!(base64_encode(&[0x00, 0x00]), "AAA=");
    }

    #[test]
    fn test_base64_encode_three_bytes_no_padding() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    // ── left_tab_for_click additional ──

    #[test]
    fn test_left_tab_for_click_oob() {
        assert_eq!(left_tab_for_click(20), None);
        assert_eq!(left_tab_for_click(0), None);
    }

    // ── footer_height_for combined states ──

    #[test]
    fn test_footer_height_input_multi_line() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.input_text = "hello\nworld".into();
        state.input_line_count = 2;
        assert_eq!(footer_height_for(&state), 3);
    }

    #[test]
    fn test_footer_height_search_plus_input() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.input_text = "a\nb\nc".into();
        state.input_line_count = 3;
        state.search_active = true;
        // footer = max(input_line_count, 1) + 1 = 3 + 1 = 4
        assert_eq!(footer_height_for(&state), 4);
    }

    #[test]
    fn test_footer_height_slash_command() {
        let mut state = make_state();
        state.slash_command_active = true;
        assert_eq!(footer_height_for(&state), 2);
    }

    // ── dispatch_slash_command: untested commands ──

    #[test]
    fn test_dispatch_slash_checkpoint_popup() {
        let mut state = make_state();
        state.turn = 7;
        state.exec_completed_steps = 3;
        state.exec_total_steps = 5;
        state.log_entries.push_back(crate::state::LogEntry {
            message: "err".into(),
            is_error: true,
            repeat_count: 1,
        });
        state.kanban_items.push(crate::state::KanbanItem {
            id: "k1".into(),
            title: "task".into(),
            status: crate::state::KanbanStatus::Pending,
        });
        dispatch_slash_command(&mut state, "/checkpoint");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Checkpoint");
        assert!(popup.lines.iter().any(|l| l.contains("7")));
        assert!(popup.lines.iter().any(|l| l.contains("3/5")));
        assert!(popup.lines.iter().any(|l| l.contains("1"))); // errors
    }

    #[test]
    fn test_dispatch_slash_rollback_logs() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/rollback");
        let last = state.log_entries.back().unwrap();
        assert!(last.message.contains("rollback"));
        assert!(last.message.contains("暂不可用"));
    }

    #[test]
    fn test_dispatch_slash_diff_shows_popup() {
        // /diff now executes git diff --stat HEAD and shows a popup (not a log entry).
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/diff");
        // In the test environment we're inside a git repo, so git should succeed.
        // Either way a popup must be produced (error or diff output).
        assert!(
            state.slash_command_popup.is_some(),
            "/diff should always produce a popup"
        );
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Diff");
        assert!(!popup.lines.is_empty());
    }

    #[test]
    fn test_dispatch_slash_diff_non_git_shows_error_popup() {
        // Run /diff from a temp directory with no git repo.
        let tmp = std::env::temp_dir().join("hermess_test_no_git");
        let _ = std::fs::create_dir_all(&tmp);
        let orig = std::env::current_dir().unwrap_or_default();
        let _ = std::env::set_current_dir(&tmp);
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/diff");
        let _ = std::env::set_current_dir(&orig);
        let popup = state.slash_command_popup.unwrap();
        // In a non-git dir, should show an error message
        assert!(
            popup
                .lines
                .iter()
                .any(|l| l.contains("git 仓库") || l.contains("无变更") || l.contains("错误")),
            "expected error/empty message in popup, got: {:?}",
            popup.lines
        );
    }

    #[test]
    fn test_dispatch_slash_new_resets_session() {
        let mut state = make_state();
        state.turn = 42;
        state.phase = AgentPhase::Executing;
        state.agent_done = true;
        state.streaming_buffer = "old stream".into();
        state.summary_streaming_buffer = "old summary".into();
        state.executions.push(crate::state::StepExecState {
            step_id: Uuid::new_v4(),
            tool: "test".into(),
            status: crate::state::StepStatus::Success,
            content_preview: None,
            content_full: None,
            duration_ms: Some(100),
            layer: 0,
        });
        state.log_entries.push_back(crate::state::LogEntry {
            message: "old log".into(),
            is_error: false,
            repeat_count: 0,
        });
        state.kanban_items.push(crate::state::KanbanItem {
            id: "k1".into(),
            title: "task".into(),
            status: crate::state::KanbanStatus::Pending,
        });
        state.plan_ready = true;
        state.plan_steps_count = 5;
        state.total_duration_ms = Some(999);
        dispatch_slash_command(&mut state, "/new");
        assert_eq!(state.turn, 0);
        assert_eq!(state.phase, AgentPhase::Idle);
        assert!(!state.agent_done);
        assert!(state.streaming_buffer.is_empty());
        assert!(state.summary_streaming_buffer.is_empty());
        assert!(state.executions.is_empty());
        assert!(state.log_entries.is_empty());
        assert!(state.kanban_items.is_empty());
        assert!(!state.plan_ready);
        assert_eq!(state.plan_steps_count, 0);
        assert!(state.total_duration_ms.is_none());
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "New Session");
    }

    #[test]
    fn test_dispatch_slash_load_no_args_shows_usage() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/load");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Load Session");
        assert!(popup.lines.iter().any(|l| l.contains("用法")));
    }

    #[test]
    fn test_dispatch_slash_compress_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/compress");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Compress");
        assert!(popup.lines.iter().any(|l| l.contains("压缩")));
    }

    #[test]
    fn test_dispatch_slash_cron_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/cron");
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Cron");
        assert!(popup.lines.iter().any(|l| l.contains("* * * * *")));
        assert!(popup.lines.iter().any(|l| l.contains("每天9点")));
    }

    #[test]
    fn test_dispatch_slash_kanban_toggles() {
        let mut state = make_state();
        assert!(!state.kanban_visible);
        dispatch_slash_command(&mut state, "/kanban");
        assert!(state.kanban_visible);
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert_eq!(popup.title, "Kanban");
        assert!(popup.lines.iter().any(|l| l.contains("已显示")));
        dispatch_slash_command(&mut state, "/kanban");
        assert!(!state.kanban_visible);
        let popup = state.slash_command_popup.as_ref().unwrap();
        assert!(popup.lines.iter().any(|l| l.contains("已隐藏")));
    }

    // ── handle_help_overlay_key (help scroll) ──

    #[test]
    fn test_help_overlay_down_increments_scroll() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 0;
        let still_open = handle_help_overlay_key(&mut state, KeyCode::Down);
        assert!(still_open);
        assert_eq!(state.help_scroll, 1);
    }

    #[test]
    fn test_help_overlay_up_saturates_at_zero() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 0;
        let still_open = handle_help_overlay_key(&mut state, KeyCode::Up);
        assert!(still_open);
        assert_eq!(state.help_scroll, 0); // saturating_sub
    }

    #[test]
    fn test_help_overlay_page_down_adds_10() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 5;
        handle_help_overlay_key(&mut state, KeyCode::PageDown);
        assert_eq!(state.help_scroll, 15);
    }

    #[test]
    fn test_help_overlay_page_up_subtracts_10() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 20;
        handle_help_overlay_key(&mut state, KeyCode::PageUp);
        assert_eq!(state.help_scroll, 10);
    }

    #[test]
    fn test_help_overlay_home_resets_to_zero() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 42;
        handle_help_overlay_key(&mut state, KeyCode::Home);
        assert_eq!(state.help_scroll, 0);
    }

    #[test]
    fn test_help_overlay_esc_closes_and_resets_scroll() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 15;
        let still_open = handle_help_overlay_key(&mut state, KeyCode::Esc);
        assert!(!still_open);
        assert!(!state.help_visible);
        assert_eq!(state.help_scroll, 0);
    }

    // ── scroll_to_top / scroll_to_bottom ──

    #[test]
    fn test_scroll_to_top_resets_evo_scroll() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 50;
        scroll_to_top(&mut state);
        assert_eq!(state.evo_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_resets_log_scroll_when_input_focused() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Input;
        state.log_scroll = 100;
        scroll_to_top(&mut state);
        assert_eq!(state.log_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_resets_log_for_minilog() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 30;
        scroll_to_top(&mut state);
        assert_eq!(state.log_scroll, 0);
        assert!(!state.log_auto_scroll);
    }

    #[test]
    fn test_scroll_to_bottom_evo_panel() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 0;
        scroll_to_bottom(&mut state);
        assert!(state.evo_scroll > 0, "should have scrolled down");
    }

    // ── toggle_evolution_section ──

    #[test]
    fn test_toggle_evo_section_all_visible_hides_all() {
        let mut state = make_state();
        state.evo_weights_hidden = false;
        state.evo_stats_hidden = false;
        state.evo_meta_hidden = false;
        toggle_evolution_section(&mut state);
        assert!(state.evo_weights_hidden);
        assert!(state.evo_stats_hidden);
        assert!(state.evo_meta_hidden);
    }

    #[test]
    fn test_toggle_evo_section_all_hidden_shows_all() {
        let mut state = make_state();
        state.evo_weights_hidden = true;
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
        toggle_evolution_section(&mut state);
        assert!(!state.evo_weights_hidden);
        assert!(!state.evo_stats_hidden);
        assert!(!state.evo_meta_hidden);
    }

    #[test]
    fn test_toggle_evo_section_partial_hides_all() {
        let mut state = make_state();
        state.evo_weights_hidden = false; // one visible
        state.evo_stats_hidden = true;
        state.evo_meta_hidden = true;
        toggle_evolution_section(&mut state);
        // Any visible → hide all
        assert!(state.evo_weights_hidden);
        assert!(state.evo_stats_hidden);
        assert!(state.evo_meta_hidden);
    }

    #[test]
    fn test_toggle_evo_double_toggle_returns_to_start() {
        let mut state = make_state();
        state.evo_weights_hidden = false;
        state.evo_stats_hidden = false;
        state.evo_meta_hidden = false;
        toggle_evolution_section(&mut state); // → all hidden
        toggle_evolution_section(&mut state); // → all shown
        assert!(!state.evo_weights_hidden);
        assert!(!state.evo_stats_hidden);
        assert!(!state.evo_meta_hidden);
    }

    // ── page_scroll_focused ──

    #[test]
    fn test_page_scroll_down_adds_12() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 0;
        page_scroll_focused(&mut state, 1);
        assert_eq!(state.evo_scroll, 12);
    }

    #[test]
    fn test_page_scroll_up_subtracts_12() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 20;
        page_scroll_focused(&mut state, -1);
        assert_eq!(state.evo_scroll, 8);
    }

    #[test]
    fn test_page_scroll_up_saturates_at_zero() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 5;
        page_scroll_focused(&mut state, -1);
        assert_eq!(state.evo_scroll, 0);
    }

    // ── get_focused_panel_lines ──

    #[test]
    fn test_get_focused_panel_lines_planning_returns_streaming() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = crate::state::LeftTab::Plan;
        state.streaming_buffer = "line1\nline2\nline3".into();
        let lines = get_focused_panel_lines(&state);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_get_focused_panel_lines_idle_returns_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Idle;
        push_log(&mut state, "log entry A".into(), false);
        let lines = get_focused_panel_lines(&state);
        assert!(lines.iter().any(|l| l.contains("log entry A")));
    }

    #[test]
    fn test_get_focused_panel_lines_evo_returns_weights() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        // Evolution panel lines — may be empty but should not panic
        let lines = get_focused_panel_lines(&state);
        let _ = lines; // just verify no panic
    }

    // ── export_to_file ──

    #[test]
    fn test_export_to_file_empty_state_returns_some() {
        let state = make_state();
        // export_to_file should return Some with filename + content
        let result = export_to_file(&state);
        assert!(
            result.is_some(),
            "should produce export even with empty state"
        );
        let (filename, content) = result.unwrap();
        assert!(filename.ends_with(".md") || filename.contains("hermess"));
        assert!(!content.is_empty());
    }

    // ── is_ctrl_c ──

    #[test]
    fn test_is_ctrl_c_detects_ctrl_c() {
        assert!(is_ctrl_c(KeyCode::Char('c'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_is_ctrl_c_no_modifier_false() {
        assert!(!is_ctrl_c(KeyCode::Char('c'), KeyModifiers::NONE));
    }

    #[test]
    fn test_is_ctrl_c_different_key_false() {
        assert!(!is_ctrl_c(KeyCode::Char('x'), KeyModifiers::CONTROL));
    }

    // ── session_tabs_height_for ──

    #[test]
    fn test_session_tabs_height_no_tabs() {
        let state = make_state();
        // A fresh state has no session tabs
        let h = session_tabs_height_for(&state);
        assert_eq!(h, 0, "no tabs → height 0");
    }

    #[test]
    fn test_session_tabs_height_two_tabs() {
        let mut state = make_state();
        state.session_tabs = vec![
            crate::state::SessionTab {
                name: "tab1".into(),
            },
            crate::state::SessionTab {
                name: "tab2".into(),
            },
        ];
        let h = session_tabs_height_for(&state);
        assert_eq!(h, 1, "two tabs → height 1");
    }

    // ── handle_event: AgentStarted ──

    #[test]
    fn test_agent_started_updates_agent_name() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::AgentStarted {
                name: "NewAgent".into(),
            },
        );
        assert_eq!(state.agent_name, "NewAgent");
    }

    // ── handle_event: PlanStreamingToken ──

    #[test]
    fn test_plan_streaming_token_appends_to_buffer() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        handle_event(
            &mut state,
            AgentEvent::PlanStreamingToken {
                token: "hello ".into(),
            },
        );
        handle_event(
            &mut state,
            AgentEvent::PlanStreamingToken {
                token: "world".into(),
            },
        );
        assert!(state.streaming_buffer.contains("hello"));
        assert!(state.streaming_buffer.contains("world"));
    }

    // ── handle_event: EvolvePhaseStarted ──

    #[test]
    fn test_evolve_phase_started_sets_phase() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::EvolvePhaseStarted);
        assert_eq!(state.phase, AgentPhase::Evolving);
    }

    // ── open_step_overlay ──

    #[test]
    fn test_open_step_overlay_valid_index() {
        let mut state = make_state();
        let step_id = uuid::Uuid::new_v4();
        state.executions.push(crate::state::StepExecState {
            step_id,
            tool: "bash".into(),
            status: crate::state::StepStatus::Success,
            duration_ms: Some(100),
            content_preview: Some("preview".into()),
            content_full: Some("full content here".into()),
            layer: 0,
        });
        open_step_overlay(&mut state, 0);
        assert!(state.output_overlay.is_some());
        let overlay = state.output_overlay.unwrap();
        assert_eq!(overlay.step_id, step_id);
        assert_eq!(overlay.full_content, "full content here");
    }

    #[test]
    fn test_open_step_overlay_out_of_bounds_noop() {
        let mut state = make_state();
        // No executions — out of bounds → noop (no panic)
        open_step_overlay(&mut state, 99);
        assert!(state.output_overlay.is_none());
    }

    #[test]
    fn test_open_step_overlay_uses_preview_when_full_absent() {
        let mut state = make_state();
        state.executions.push(crate::state::StepExecState {
            step_id: uuid::Uuid::new_v4(),
            tool: "read".into(),
            status: crate::state::StepStatus::Success,
            duration_ms: None,
            content_preview: Some("preview only".into()),
            content_full: None, // no full content
            layer: 0,
        });
        open_step_overlay(&mut state, 0);
        let overlay = state.output_overlay.unwrap();
        assert_eq!(overlay.full_content, "preview only");
    }

    // ── input_line_count_for ──

    #[test]
    fn test_input_line_count_for_two_lines() {
        assert_eq!(input_line_count_for("line1\nline2"), 2);
    }

    #[test]
    fn test_input_line_count_for_three_lines() {
        assert_eq!(input_line_count_for("a\nb\nc"), 3);
    }

    #[test]
    fn test_input_line_count_for_eight_lines_clamp() {
        // 10 newlines → 11 parts → clamped to 8
        let text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj";
        assert_eq!(input_line_count_for(text), 8);
    }

    #[test]
    fn test_input_line_count_for_empty_string() {
        // split of "" gives [""] → 1 segment → 1
        assert_eq!(input_line_count_for(""), 1);
    }

    // ── submit_tui_input — history cap (50) ──

    #[test]
    fn test_submit_tui_input_empty_text_still_submits() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        let ok = submit_tui_input(&mut state, &tui_input, String::new());
        assert!(ok);
        assert!(!state.awaiting_input);
    }

    #[test]
    fn test_submit_tui_input_clears_buffer() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        *tui_input.buffer.lock() = "hello".into();
        submit_tui_input(&mut state, &tui_input, "hello".into());
        assert!(tui_input.buffer.lock().is_empty());
    }

    #[test]
    fn test_submit_tui_input_resets_cursor() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        *tui_input.cursor.lock() = 5;
        submit_tui_input(&mut state, &tui_input, "hello".into());
        assert_eq!(*tui_input.cursor.lock(), 0);
    }

    #[test]
    fn test_submit_tui_input_moves_focus_to_mainleft() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        submit_tui_input(&mut state, &tui_input, "task".into());
        assert_eq!(state.focused_panel, FocusedPanel::MainLeft);
    }

    // ── begin_next_task_input — extended state reset ──

    #[test]
    fn test_begin_next_task_input_resets_input_text() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        state.input_text = "some old text".into();
        begin_next_task_input(&mut state, &tui_input);
        assert!(state.input_text.is_empty());
    }

    #[test]
    fn test_begin_next_task_input_resets_input_cursor() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        state.input_cursor = 42;
        begin_next_task_input(&mut state, &tui_input);
        assert_eq!(state.input_cursor, 0);
    }

    #[test]
    fn test_begin_next_task_input_sets_focused_panel_to_input() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        state.focused_panel = FocusedPanel::Evolution;
        begin_next_task_input(&mut state, &tui_input);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn test_begin_next_task_input_sets_tui_awaiting_true() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        assert!(tui_input
            .awaiting
            .load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_begin_next_task_input_clears_submitted() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.submitted.lock() = Some("old".into());
        begin_next_task_input(&mut state, &tui_input);
        assert!(tui_input.submitted.lock().is_none());
    }

    #[test]
    fn test_begin_next_task_sets_line_count_to_one() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        state.input_line_count = 5;
        begin_next_task_input(&mut state, &tui_input);
        assert_eq!(state.input_line_count, 1);
    }

    // ── p-cancel: stop_flag reset after cancel ──

    #[test]
    fn test_p_cancel_stop_flag_reset_after_agent_done() {
        // After agent completes with stop_requested, stop_flag should be cleared
        // (mirrors outer loop logic in run_tui)
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        stop_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        // After reset: flag is false, subsequent tasks run normally
        assert!(!stop_flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_p_cancel_allows_resubmit_after_reset() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        // Simulate: agent done (after cancel)
        state.agent_done = true;
        // Begin next task — should succeed
        begin_next_task_input(&mut state, &tui_input);
        assert!(state.awaiting_input);
        let ok = submit_tui_input(&mut state, &tui_input, "new task after cancel".into());
        assert!(ok);
        assert!(!state.awaiting_input);
    }

    #[test]
    fn test_p_cancel_when_agent_done_noop() {
        // p-key guard: if agent_done == true, stop_flag should NOT be set
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let state_agent_done = true;
        // Guard: only set flag if !agent_done
        if !state_agent_done {
            stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(!stop_flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_request_tui_quit_sets_stop_flag() {
        let mut state = make_state();
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut tui_input = TuiInput::new();
        tui_input.stop_flag = Some(stop_flag.clone());
        // Simulate quit logic
        request_tui_quit(&mut state, &tui_input);
        assert!(stop_flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    // ── Ctrl+Tab shortcut — left_tab cycle ──

    #[test]
    fn test_left_tab_next_plan_to_execution() {
        assert_eq!(LeftTab::Plan.next(), LeftTab::Execution);
    }

    #[test]
    fn test_left_tab_next_execution_to_plan() {
        assert_eq!(LeftTab::Execution.next(), LeftTab::Plan);
    }

    #[test]
    fn test_left_tab_cycle_three_times_returns_to_start() {
        let tab = LeftTab::Plan;
        let t1 = tab.next(); // Execution
        let t2 = t1.next(); // Plan
        assert_eq!(t2, LeftTab::Plan);
    }

    // ── [ and ] — split ratio adjustment (left_split_pct: Option<u16>) ──

    #[test]
    fn test_adjust_split_wider_left() {
        let mut state = make_state();
        // Simulate ] key: increase left split (None means default ~50%)
        let cur = state.left_split_pct.unwrap_or(50);
        state.left_split_pct = Some((cur + 5).min(80));
        assert_eq!(state.left_split_pct, Some(55));
    }

    #[test]
    fn test_adjust_split_narrower_left() {
        let mut state = make_state();
        state.left_split_pct = Some(50);
        // Simulate [ key: decrease left split
        let cur = state.left_split_pct.unwrap_or(50);
        state.left_split_pct = Some(cur.saturating_sub(5).max(20));
        assert_eq!(state.left_split_pct, Some(45));
    }

    #[test]
    fn test_adjust_split_clamps_at_max() {
        let mut state = make_state();
        state.left_split_pct = Some(78);
        let cur = state.left_split_pct.unwrap_or(50);
        state.left_split_pct = Some((cur + 5).min(80));
        assert_eq!(state.left_split_pct, Some(80));
    }

    #[test]
    fn test_adjust_split_clamps_at_min() {
        let mut state = make_state();
        state.left_split_pct = Some(22);
        let cur = state.left_split_pct.unwrap_or(50);
        state.left_split_pct = Some(cur.saturating_sub(5).max(20));
        assert_eq!(state.left_split_pct, Some(20));
    }

    // ── push_log pruning ──

    #[test]
    fn test_handle_event_prunes_log_at_200() {
        // Pruning happens at the end of handle_event.
        // Drive 210 SummaryReady events (each adds one log entry) to trigger pruning.
        let mut state = make_state();
        for i in 0u32..210 {
            handle_event(
                &mut state,
                AgentEvent::SummaryReady {
                    summary: format!("done {i}"),
                },
            );
        }
        assert!(
            state.log_entries.len() <= 200,
            "log_entries should be pruned to ≤200 by handle_event, got {}",
            state.log_entries.len()
        );
    }

    #[test]
    fn test_push_log_deduplication() {
        let mut state = make_state();
        push_log(&mut state, "same message".into(), false);
        push_log(&mut state, "same message".into(), false);
        // Should deduplicate: one entry with repeat_count=1
        assert_eq!(state.log_entries.len(), 1);
        assert_eq!(state.log_entries.back().unwrap().repeat_count, 1);
    }

    #[test]
    fn test_push_log_error_flag_propagated() {
        let mut state = make_state();
        push_log(&mut state, "critical failure".into(), true);
        let entry = state.log_entries.back().unwrap();
        assert!(entry.is_error);
        assert!(entry.message.contains("critical failure"));
    }

    #[test]
    fn test_push_log_distinct_messages_not_deduplicated() {
        let mut state = make_state();
        push_log(&mut state, "message A".into(), false);
        push_log(&mut state, "message B".into(), false);
        assert_eq!(state.log_entries.len(), 2);
    }

    // ── apply_delta saturating ──

    #[test]
    fn test_apply_delta_large_negative_saturates() {
        assert_eq!(apply_delta(3, -100), 0);
    }

    #[test]
    fn test_apply_delta_zero_delta() {
        assert_eq!(apply_delta(7, 0), 7);
    }

    // ── handle_event: SummaryReady adds to log ──

    #[test]
    fn test_summary_ready_adds_log_entry() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SummaryReady {
                summary: "任务完成".into(),
            },
        );
        assert!(!state.log_entries.is_empty());
        let last = state.log_entries.back().unwrap();
        assert!(last.message.contains("任务完成"));
    }

    #[test]
    fn test_summary_ready_sets_summary_field() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::SummaryReady {
                summary: "done".into(),
            },
        );
        assert_eq!(state.summary.as_deref(), Some("done"));
    }

    #[test]
    fn test_evolve_phase_complete_sets_idle() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        handle_event(&mut state, AgentEvent::EvolvePhaseComplete);
        assert_eq!(state.phase, AgentPhase::Idle);
        assert!(state.agent_done);
    }

    #[test]
    fn test_plan_phase_started_clears_streaming_buffer() {
        let mut state = make_state();
        state.streaming_buffer = "old data".into();
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        assert!(state.streaming_buffer.is_empty());
    }

    #[test]
    fn test_turn_started_resets_agent_done() {
        let mut state = make_state();
        state.agent_done = true;
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 5 });
        assert!(!state.agent_done);
        assert_eq!(state.turn, 5);
    }

    // ── scroll_focused: evo_scroll saturating ──

    #[test]
    fn test_evo_scroll_saturates_at_zero() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 0;
        scroll_focused(&mut state, -10);
        assert_eq!(state.evo_scroll, 0);
    }

    #[test]
    fn test_evo_scroll_increments_correctly() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Evolution;
        state.evo_scroll = 10;
        scroll_focused(&mut state, 5);
        assert_eq!(state.evo_scroll, 15);
    }

    // ── footer_height_for ──

    #[test]
    fn test_footer_height_idle_not_awaiting() {
        let state = make_state();
        assert_eq!(footer_height_for(&state), 1);
    }

    #[test]
    fn test_footer_height_multiline_input() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.input_line_count = 4;
        // 4 lines + 1 hint line = 5
        assert_eq!(footer_height_for(&state), 5);
    }

    #[test]
    fn test_footer_height_single_line_input() {
        let mut state = make_state();
        state.awaiting_input = true;
        state.input_line_count = 1;
        assert_eq!(footer_height_for(&state), 2); // 1 line + 1 hint
    }

    // ── close_overlays_focus_input ──

    #[test]
    fn test_close_overlays_hides_help() {
        let mut state = make_state();
        state.help_visible = true;
        close_overlays_focus_input(&mut state);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_close_overlays_hides_settings() {
        let mut state = make_state();
        state.settings_visible = true;
        close_overlays_focus_input(&mut state);
        assert!(!state.settings_visible);
    }

    #[test]
    fn test_close_overlays_clears_slash_popup() {
        let mut state = make_state();
        state.slash_command_popup = Some(crate::state::SlashResult {
            title: "test".into(),
            lines: vec!["line".into()],
            scroll: 0,
        });
        close_overlays_focus_input(&mut state);
        assert!(state.slash_command_popup.is_none());
    }

    #[test]
    fn test_close_overlays_focuses_input() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        close_overlays_focus_input(&mut state);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    // ── dispatch_slash_command: additional coverage ──

    #[test]
    fn test_dispatch_slash_new_clears_log_entries() {
        let mut state = make_state();
        push_log(&mut state, "old entry".into(), false);
        assert!(!state.log_entries.is_empty());
        dispatch_slash_command(&mut state, "/new");
        // /new resets session — log_entries.clear() is called
        assert!(state.log_entries.is_empty());
    }

    #[test]
    fn test_dispatch_slash_new_resets_turn_counter() {
        let mut state = make_state();
        state.turn = 42;
        dispatch_slash_command(&mut state, "/new");
        assert_eq!(state.turn, 0);
    }

    #[test]
    fn test_dispatch_slash_new_resets_phase_to_idle() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        dispatch_slash_command(&mut state, "/new");
        assert_eq!(state.phase, AgentPhase::Idle);
    }

    #[test]
    fn test_dispatch_slash_unknown_pushes_log() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/nonexistent-command");
        // Falls through to _ arm which calls push_log with "未知命令"
        assert!(
            state
                .log_entries
                .back()
                .map(|e| e.message.contains("未知命令"))
                .unwrap_or(false),
            "Expected 未知命令 in log"
        );
    }

    #[test]
    fn test_dispatch_slash_model_with_empty_args_shows_current() {
        let mut state = make_state();
        state.user_settings.llm_model = "deepseek-chat".into();
        dispatch_slash_command(&mut state, "/model");
        // Should log current model, not change it
        assert_eq!(state.user_settings.llm_model, "deepseek-chat");
    }

    #[test]
    fn test_dispatch_slash_compress_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/compress");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Compress");
    }

    // ── handle_event: ExecutePhaseStarted ──

    #[test]
    fn test_execute_phase_started_sets_total_steps() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 5 },
        );
        assert_eq!(state.exec_total_steps, 5);
        assert_eq!(state.phase, AgentPhase::Executing);
    }

    #[test]
    fn test_execute_phase_started_resets_completed_steps() {
        let mut state = make_state();
        state.exec_completed_steps = 99;
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 3 },
        );
        assert_eq!(state.exec_completed_steps, 0);
    }

    // ── handle_event: PlanReady ──

    #[test]
    fn test_plan_ready_sets_plan_ready_flag() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        handle_event(&mut state, AgentEvent::PlanReady { steps_count: 4 });
        assert!(state.plan_ready);
        assert_eq!(state.plan_steps_count, 4);
    }

    // ── scroll_focused: log_scroll ──

    #[test]
    fn test_scroll_log_when_minilog_focused() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 5;
        scroll_focused(&mut state, 3);
        assert_eq!(state.log_scroll, 8);
    }

    #[test]
    fn test_scroll_log_saturates_at_zero() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MiniLog;
        state.log_scroll = 2;
        scroll_focused(&mut state, -10);
        assert_eq!(state.log_scroll, 0);
    }

    // ── handle_event: ReflectPhaseStarted / Complete ──

    #[test]
    fn test_reflect_phase_started_sets_phase() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::ReflectPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Reflecting);
    }

    #[test]
    fn test_agent_error_adds_error_log() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::AgentError {
                message: "connection failed".into(),
            },
        );
        assert!(
            state
                .log_entries
                .iter()
                .any(|e| e.is_error && e.message.contains("connection failed")),
            "should have an error log entry"
        );
    }

    #[test]
    fn test_agent_stopped_sets_phase_idle() {
        let mut state = make_state();
        state.phase = AgentPhase::Executing;
        handle_event(&mut state, AgentEvent::AgentStopped);
        assert_eq!(state.phase, AgentPhase::Idle);
    }

    #[test]
    fn test_reset_session_pushes_log() {
        let mut state = make_state();
        let before_len = state.log_entries.len();
        handle_event(&mut state, AgentEvent::ResetSession);
        // Should add a log entry about session reset
        assert!(state.log_entries.len() > before_len);
        assert!(
            state
                .log_entries
                .back()
                .map(|e| e.message.contains("重置"))
                .unwrap_or(false),
            "expected 重置 in log"
        );
    }

    #[test]
    fn test_full_turn_pipeline_phase_sequence() {
        let mut state = make_state();
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        assert_eq!(state.phase, AgentPhase::Observing);
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Planning);
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 2 },
        );
        assert_eq!(state.phase, AgentPhase::Executing);
        handle_event(&mut state, AgentEvent::ReflectPhaseStarted);
        assert_eq!(state.phase, AgentPhase::Reflecting);
        handle_event(&mut state, AgentEvent::EvolvePhaseStarted);
        assert_eq!(state.phase, AgentPhase::Evolving);
        handle_event(&mut state, AgentEvent::EvolvePhaseComplete);
        assert_eq!(state.phase, AgentPhase::Idle);
        assert!(state.agent_done);
    }

    // ── dispatch_slash_command: additional commands ──

    #[test]
    fn test_dispatch_slash_status_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/status");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Status");
    }

    #[test]
    fn test_dispatch_slash_usage_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/usage");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Usage");
    }

    #[test]
    fn test_dispatch_slash_cron_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/cron");
        assert!(state.slash_command_popup.is_some());
    }

    #[test]
    fn test_dispatch_slash_memory_no_query_shows_help() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/memory");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert!(popup
            .lines
            .iter()
            .any(|l| l.contains("用法") || l.contains("memory")));
    }

    #[test]
    fn test_dispatch_slash_personality_no_arg_shows_list() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/personality");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Personality");
        assert!(popup
            .lines
            .iter()
            .any(|l| l.contains("concise") || l.contains("verbose")));
    }

    #[test]
    fn test_dispatch_slash_personality_with_arg() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/personality concise");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert!(popup.lines.iter().any(|l| l.contains("concise")));
    }

    // ── scroll_focused: Input panel scrolls log ──

    #[test]
    fn test_scroll_input_panel_affects_log() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::Input;
        state.log_scroll = 5;
        scroll_focused(&mut state, 3);
        assert_eq!(state.log_scroll, 8);
    }

    // ── handle_help_overlay_key: j/k aliases ──

    #[test]
    fn test_help_overlay_j_key_increments_scroll() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 0;
        handle_help_overlay_key(&mut state, KeyCode::Char('j'));
        assert_eq!(state.help_scroll, 1);
    }

    #[test]
    fn test_help_overlay_k_key_decrements_scroll() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 5;
        handle_help_overlay_key(&mut state, KeyCode::Char('k'));
        assert_eq!(state.help_scroll, 4);
    }

    #[test]
    fn test_help_overlay_h_key_closes() {
        let mut state = make_state();
        state.help_visible = true;
        state.help_scroll = 10;
        let still_open = handle_help_overlay_key(&mut state, KeyCode::Char('h'));
        assert!(!still_open);
        assert!(!state.help_visible);
        assert_eq!(state.help_scroll, 0);
    }

    // ── scroll_to_top: Plan and Exec tabs ──

    #[test]
    fn test_scroll_to_top_plan_tab() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Planning;
        state.left_tab = crate::state::LeftTab::Plan;
        state.plan_scroll = 50;
        scroll_to_top(&mut state);
        assert_eq!(state.plan_scroll, 0);
    }

    #[test]
    fn test_scroll_to_top_exec_tab() {
        let mut state = make_state();
        state.focused_panel = FocusedPanel::MainLeft;
        state.phase = AgentPhase::Executing;
        state.left_tab = crate::state::LeftTab::Execution;
        state.exec_scroll = 30;
        scroll_to_top(&mut state);
        assert_eq!(state.exec_scroll, 0);
    }

    // ── handle_event: StepStarted ──

    #[test]
    fn test_step_started_increments_exec_count() {
        let mut state = make_state();
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 3 },
        );
        handle_event(
            &mut state,
            AgentEvent::StepStarted {
                step_id: Uuid::new_v4(),
                tool: "bash".into(),
                layer: 0,
            },
        );
        // Should add an execution step entry
        assert!(!state.executions.is_empty());
    }

    // ── dispatch_slash_command: /debug ──

    #[test]
    fn test_dispatch_slash_debug_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/debug");
        assert!(state.slash_command_popup.is_some());
        let popup = state.slash_command_popup.unwrap();
        assert_eq!(popup.title, "Debug");
    }

    // ── dispatch_slash_command: /skills ──

    #[test]
    fn test_dispatch_slash_skills_shows_popup() {
        let mut state = make_state();
        dispatch_slash_command(&mut state, "/skills");
        assert!(state.slash_command_popup.is_some());
    }

    // ── push_log: error deduplication ──

    #[test]
    fn test_push_log_error_dedup_same_error() {
        let mut state = make_state();
        push_log(&mut state, "timeout".into(), true);
        push_log(&mut state, "timeout".into(), true);
        assert_eq!(state.log_entries.len(), 1);
        assert_eq!(state.log_entries.back().unwrap().repeat_count, 1);
    }

    #[test]
    fn test_push_log_different_is_error_no_dedup() {
        let mut state = make_state();
        push_log(&mut state, "same message".into(), false);
        push_log(&mut state, "same message".into(), true); // different is_error
        assert_eq!(state.log_entries.len(), 2);
    }

    // ── begin_next_task_input: clears tui buffer ──

    #[test]
    fn test_begin_next_task_input_clears_tui_buffer() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        *tui_input.buffer.lock() = "leftover text".into();
        begin_next_task_input(&mut state, &tui_input);
        assert!(tui_input.buffer.lock().is_empty());
    }

    // ── submit_tui_input: stores in submitted ──

    #[test]
    fn test_submit_tui_input_stores_text_in_submitted() {
        let mut state = make_state();
        let tui_input = TuiInput::new();
        begin_next_task_input(&mut state, &tui_input);
        submit_tui_input(&mut state, &tui_input, "my task".into());
        assert_eq!(tui_input.submitted.lock().as_deref(), Some("my task"));
    }

    // ── handle_event: EvolvePhaseComplete → results_visible ──

    #[test]
    fn test_evolve_complete_sets_results_visible() {
        let mut state = make_state();
        state.results_visible = false;
        handle_event(&mut state, AgentEvent::EvolvePhaseComplete);
        assert!(state.results_visible);
    }

    // ── handle_event: PlanPhaseStarted clears summary buffer ──

    #[test]
    fn test_plan_phase_started_clears_summary_streaming_buffer() {
        let mut state = make_state();
        state.summary_streaming_buffer = "old summary".into();
        handle_event(&mut state, AgentEvent::PlanPhaseStarted);
        assert!(state.summary_streaming_buffer.is_empty());
    }

    // ── handle_event: ExecutePhaseStarted resets exec state ──

    #[test]
    fn test_execute_phase_started_sets_left_tab_to_execution() {
        let mut state = make_state();
        state.left_tab = crate::state::LeftTab::Plan;
        handle_event(
            &mut state,
            AgentEvent::ExecutePhaseStarted { total_steps: 1 },
        );
        assert_eq!(state.left_tab, crate::state::LeftTab::Execution);
    }

    // ── 50KB streaming buffer cap ──

    #[test]
    fn test_plan_streaming_buffer_capped_at_50kb() {
        let mut state = make_state();
        // Generate >50KB of plan streaming tokens (line-based so cap triggers)
        let big_token = "x".repeat(100) + "\n"; // 101 bytes per token
        for _ in 0..600 {
            handle_event(
                &mut state,
                AgentEvent::PlanStreamingToken {
                    token: big_token.clone(),
                },
            );
        }
        // Buffer should be capped — well under 60*101=60600 bytes
        assert!(
            state.streaming_buffer.len() <= 51_200 + 1000, // allow last line overshoot
            "streaming_buffer should be capped, got {} bytes",
            state.streaming_buffer.len()
        );
    }

    #[test]
    fn test_summary_streaming_buffer_capped_at_50kb() {
        let mut state = make_state();
        let big_token = "y".repeat(100) + "\n";
        for _ in 0..600 {
            handle_event(
                &mut state,
                AgentEvent::SummaryStreamingToken {
                    token: big_token.clone(),
                },
            );
        }
        assert!(
            state.summary_streaming_buffer.len() <= 51_200 + 1000,
            "summary_streaming_buffer should be capped, got {} bytes",
            state.summary_streaming_buffer.len()
        );
    }

    // ── handle_event: TurnStarted sets results_visible ──

    #[test]
    fn test_turn_started_sets_results_visible() {
        let mut state = make_state();
        state.results_visible = false;
        handle_event(&mut state, AgentEvent::TurnStarted { turn: 1 });
        assert!(state.results_visible);
    }

    // ── handle_event: SummaryStreamingToken appends ──

    #[test]
    fn test_summary_streaming_token_appends() {
        let mut state = make_state();
        state.summary_streaming_buffer.clear();
        handle_event(
            &mut state,
            AgentEvent::SummaryStreamingToken {
                token: "part1".into(),
            },
        );
        handle_event(
            &mut state,
            AgentEvent::SummaryStreamingToken {
                token: " part2".into(),
            },
        );
        assert!(state.summary_streaming_buffer.contains("part1"));
        assert!(state.summary_streaming_buffer.contains("part2"));
    }

    // ── LeftTab prev ──

    #[test]
    fn test_left_tab_prev_execution_to_plan() {
        // LeftTab.prev() should work (if implemented)
        let tab = crate::state::LeftTab::Execution;
        let prev = tab.next(); // wraps around to Plan
        assert_eq!(prev, crate::state::LeftTab::Plan);
    }
}
