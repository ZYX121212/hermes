// crates/tui/src/state.rs
// Mutable application state updated by agent events and read by the renderer.

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use evolution::EvolutionEngine;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::settings_store::UserSettings;

/// Shared input state for TUI interactive mode.
pub struct TuiInput {
    pub awaiting: AtomicBool,
    pub buffer: Mutex<String>,
    pub submitted: Mutex<Option<String>>,
    pub cursor: Mutex<usize>,
    /// Gateway route mode shared with LLM adapter (None = no adapter linkage).
    pub gateway_mode: Option<Arc<Mutex<Option<String>>>>,
    /// Shared stop flag from agent context — set to cancel current operation.
    pub stop_flag: Option<Arc<AtomicBool>>,
    /// User settings changed flag — main loop polls this to hot-reload components.
    pub settings_changed: Mutex<Option<UserSettings>>,
}

impl Default for TuiInput {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiInput {
    pub fn new() -> Self {
        Self {
            awaiting: AtomicBool::new(false),
            buffer: Mutex::new(String::new()),
            submitted: Mutex::new(None),
            cursor: Mutex::new(0),
            gateway_mode: None,
            stop_flag: None,
            settings_changed: Mutex::new(None),
        }
    }

    /// Read the current gateway route mode, or default.
    pub fn get_gateway_mode(&self) -> String {
        self.gateway_mode
            .as_ref()
            .and_then(|m| m.lock().clone())
            .unwrap_or_default()
    }

    /// Set the gateway route mode (writes to shared adapter state if linked).
    pub fn set_gateway_mode(&self, mode: &str) {
        if let Some(ref shared) = self.gateway_mode {
            *shared.lock() = Some(mode.to_string());
        }
    }
}

/// Which phase of the agent loop is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    Idle,
    Observing,
    Planning,
    Executing,
    Reflecting,
    Evolving,
}

impl TuiAppState {
    /// Get the effective left/right split ratio, respecting user override.
    pub fn split_pct(&self) -> (u16, u16) {
        if let Some(pct) = self.left_split_pct {
            (pct, 100 - pct)
        } else {
            self.phase
                .main_split_ratio(!self.evolution.all_weights().is_empty())
        }
    }
}

impl AgentPhase {
    /// Returns (left_pct, right_pct) for the main horizontal split.
    pub fn main_split_ratio(&self, has_weights: bool) -> (u16, u16) {
        match (self, has_weights) {
            (AgentPhase::Planning, false) => (85, 15),
            (AgentPhase::Planning, true) => (75, 25),
            (AgentPhase::Executing, false) => (80, 20),
            (AgentPhase::Executing, true) => (70, 30),
            (_, false) => (75, 25),
            (_, true) => (60, 40),
        }
    }
}

/// Panel focus target for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    MainLeft,
    Evolution,
    MiniLog,
    Input,
}

impl FocusedPanel {
    /// Tab cycling: MainLeft → Evolution → Input (skip invisible MiniLog).
    pub fn next(self) -> Self {
        match self {
            FocusedPanel::MainLeft => FocusedPanel::Evolution,
            FocusedPanel::Evolution => FocusedPanel::Input,
            FocusedPanel::MiniLog => FocusedPanel::Input,
            FocusedPanel::Input => FocusedPanel::MainLeft,
        }
    }

    /// Shift+Tab cycling: Input → Evolution → MainLeft.
    pub fn prev(self) -> Self {
        match self {
            FocusedPanel::MainLeft => FocusedPanel::Input,
            FocusedPanel::Evolution => FocusedPanel::MainLeft,
            FocusedPanel::MiniLog => FocusedPanel::Evolution,
            FocusedPanel::Input => FocusedPanel::Evolution,
        }
    }
}

/// Tab selection within the left panel during Planning/Executing phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Plan,
    Execution,
}

impl LeftTab {
    pub fn next(self) -> Self {
        match self {
            LeftTab::Plan => LeftTab::Execution,
            LeftTab::Execution => LeftTab::Plan,
        }
    }
}

/// Settings panel page tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Llm,
    Search,
    Finance,
    Theme,
    Feishu,
}

impl SettingsTab {
    pub fn next(self) -> Self {
        match self {
            SettingsTab::Llm => SettingsTab::Search,
            SettingsTab::Search => SettingsTab::Finance,
            SettingsTab::Finance => SettingsTab::Feishu,
            SettingsTab::Feishu => SettingsTab::Theme,
            SettingsTab::Theme => SettingsTab::Llm,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsTab::Llm => SettingsTab::Theme,
            SettingsTab::Search => SettingsTab::Llm,
            SettingsTab::Finance => SettingsTab::Search,
            SettingsTab::Feishu => SettingsTab::Finance,
            SettingsTab::Theme => SettingsTab::Feishu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::Llm => "LLM",
            SettingsTab::Search => "搜索",
            SettingsTab::Finance => "金融",
            SettingsTab::Theme => "主题",
            SettingsTab::Feishu => "飞书",
        }
    }
}

/// A session tab shown in the tab bar.
#[derive(Debug, Clone)]
pub struct SessionTab {
    pub name: String,
}

/// Slash-command result popup content.
#[derive(Debug, Clone)]
pub struct SlashResult {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: u16,
}

/// Full-screen overlay for viewing complete step output.
#[derive(Debug, Clone)]
pub struct StepOutputOverlay {
    pub step_id: uuid::Uuid,
    pub tool: String,
    pub status: StepStatus,
    pub duration_ms: Option<u64>,
    pub full_content: String,
    pub scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Success,
    Failed,
}

#[derive(Debug, Clone)]
pub struct StepExecState {
    pub step_id: Uuid,
    pub tool: String,
    pub status: StepStatus,
    pub content_preview: Option<String>,
    pub content_full: Option<String>, // 完整输出（上限 10KB）
    pub duration_ms: Option<u64>,
    pub layer: usize,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub message: String,
    pub is_error: bool,
    pub repeat_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFilter {
    All,
    ErrorsOnly,
}

impl LogFilter {
    pub fn next(self) -> Self {
        match self {
            LogFilter::All => LogFilter::ErrorsOnly,
            LogFilter::ErrorsOnly => LogFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LogFilter::All => "All",
            LogFilter::ErrorsOnly => "Errors",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextRefItem {
    pub source: String,
    pub label: String,
    pub preview: String,
}

#[derive(Debug, Clone)]
pub struct KanbanItem {
    pub id: String,
    pub title: String,
    pub status: KanbanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanbanStatus {
    Pending,
    InProgress,
    Completed,
}

pub struct TuiAppState {
    // Header
    pub agent_name: String,
    pub turn: u64,
    pub phase: AgentPhase,
    pub frame_count: u64,

    // Plan panel
    pub streaming_buffer: String,
    pub plan_steps_count: usize,
    pub plan_ready: bool,

    // Summary streaming
    pub summary_streaming_buffer: String,

    // Execution panel
    pub executions: Vec<StepExecState>,
    pub exec_total_steps: usize,
    pub exec_completed_steps: usize,

    // Log
    pub summary: Option<String>,
    pub log_entries: VecDeque<LogEntry>,

    // Evolution — read directly from shared engine
    pub evolution: Arc<EvolutionEngine>,
    pub evo_stats_hidden: bool,
    pub evo_weights_hidden: bool,
    pub evo_meta_hidden: bool,

    // Focus & UI control
    pub focused_panel: FocusedPanel,
    /// Custom left/right split percentage (None = use phase default).
    pub left_split_pct: Option<u16>,
    pub should_quit: bool,
    pub agent_done: bool,
    pub awaiting_input: bool,
    pub results_visible: bool,
    pub input_text: String,
    pub input_cursor: usize,
    /// Multiline input line count (1–8)
    pub input_line_count: u8,
    pub context_ref_active: bool,
    pub context_ref_query: String,
    pub context_ref_items: Vec<ContextRefItem>,
    pub context_ref_selected: usize,
    pub kanban_visible: bool,
    pub kanban_items: Vec<KanbanItem>,
    pub help_visible: bool,
    pub help_scroll: u16,
    pub settings_visible: bool,

    // Token usage tracker (shared with agent)
    pub usage_tracker: Option<Arc<llm::usage::UsageTracker>>,

    // Search
    pub search_query: String,
    pub search_active: bool,
    pub search_match_lines: Vec<usize>,
    pub search_current_match: Option<usize>,

    // Slash command mode
    pub slash_command_active: bool,
    pub slash_command_buffer: String,
    pub slash_command_cursor: usize,
    pub slash_command_popup: Option<SlashResult>,

    // Input history
    pub input_history: VecDeque<String>,
    pub input_history_pos: Option<usize>,
    pub input_draft: String, // saved current text before browsing history

    // Scroll offsets
    pub plan_scroll: u16,
    pub exec_scroll: u16,
    pub log_scroll: u16,
    pub log_auto_scroll: bool,
    pub log_filter: LogFilter,
    pub evo_scroll: u16,

    // Tab selection for left panel during Planning/Executing
    pub left_tab: LeftTab,

    // Execution step selection for overlay
    pub exec_selected_index: Option<usize>,

    // Full-screen output overlay state
    pub output_overlay: Option<StepOutputOverlay>,

    // Total agent duration for results report
    pub total_duration_ms: Option<u64>,

    // Gateway routing info
    pub gateway_enabled: bool,
    pub gateway_url: String,
    pub gateway_models: Vec<String>,
    pub gateway_mode: String,
    pub last_route_decision: Option<String>,
    pub shg_triggered: bool,

    // Log panel visibility
    pub log_visible: bool,

    // Settings panel state
    pub settings_tab: SettingsTab,
    pub settings_field_focus: usize,
    pub settings_editing: bool,
    pub settings_edit_buffer: String,
    pub settings_dirty: bool,
    pub settings_saved_flash: u8, // frames remaining for "已保存" flash (0 = hidden)
    pub settings_dirty_confirm: bool, // true when Esc/s pressed once while dirty
    pub user_settings: UserSettings,

    // Theme
    pub theme_preset: String, // "tokyo-night", "dracula", etc.

    // Session tabs
    pub session_tabs: Vec<SessionTab>,
    pub active_tab_index: usize,
}

impl TuiAppState {
    pub fn new(agent_name: String, evolution: Arc<EvolutionEngine>) -> Self {
        Self {
            agent_name,
            turn: 0,
            phase: AgentPhase::Idle,
            frame_count: 0,
            streaming_buffer: String::new(),
            plan_steps_count: 0,
            plan_ready: false,
            summary_streaming_buffer: String::new(),
            executions: Vec::new(),
            exec_total_steps: 0,
            exec_completed_steps: 0,
            summary: None,
            log_entries: VecDeque::new(),
            evolution,
            evo_stats_hidden: true,
            evo_weights_hidden: true,
            evo_meta_hidden: true,
            focused_panel: FocusedPanel::MainLeft,
            left_split_pct: None,
            should_quit: false,
            agent_done: false,
            awaiting_input: false,
            results_visible: true,
            input_text: String::new(),
            input_cursor: 0,
            input_line_count: 1,
            context_ref_active: false,
            context_ref_query: String::new(),
            context_ref_items: Vec::new(),
            context_ref_selected: 0,
            kanban_visible: false,
            kanban_items: Vec::new(),
            help_visible: false,
            help_scroll: 0,
            settings_visible: false,
            input_history: VecDeque::with_capacity(50),
            input_history_pos: None,
            input_draft: String::new(),
            plan_scroll: 0,
            exec_scroll: 0,
            log_scroll: 0,
            log_auto_scroll: true,
            log_filter: LogFilter::All,
            evo_scroll: 0,
            left_tab: LeftTab::Execution,
            exec_selected_index: None,
            output_overlay: None,
            total_duration_ms: None,
            gateway_enabled: false,
            gateway_url: String::new(),
            gateway_models: Vec::new(),
            gateway_mode: String::new(),
            last_route_decision: None,
            shg_triggered: false,
            log_visible: false,
            settings_tab: SettingsTab::Llm,
            settings_field_focus: 0,
            settings_editing: false,
            settings_edit_buffer: String::new(),
            settings_dirty: false,
            settings_saved_flash: 0,
            settings_dirty_confirm: false,
            user_settings: UserSettings::default(),
            usage_tracker: None,
            search_query: String::new(),
            search_active: false,
            search_match_lines: Vec::new(),
            search_current_match: None,
            slash_command_active: false,
            slash_command_buffer: String::new(),
            slash_command_cursor: 0,
            slash_command_popup: None,
            theme_preset: "tokyo-night".into(),
            session_tabs: vec![SessionTab {
                name: "会话1".into(),
            }],
            active_tab_index: 0,
        }
    }
}

/// Truncate text to `max_chars` characters, handling multi-byte safely.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

/// Strip HTML tags and decode common HTML entities from text.
pub fn strip_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Skip until '>'
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
            }
        } else if ch == '&' {
            // Collect entity name up to ';'
            let mut entity = String::new();
            let mut found_semi = false;
            for c in chars.by_ref() {
                if c == ';' {
                    found_semi = true;
                    break;
                }
                entity.push(c);
                if entity.len() > 8 {
                    break;
                }
            }
            if found_semi {
                match entity.as_str() {
                    "lt" => result.push('<'),
                    "gt" => result.push('>'),
                    "amp" => result.push('&'),
                    "quot" => result.push('"'),
                    "apos" | "#39" => result.push('\''),
                    "nbsp" => result.push(' '),
                    _ => {
                        // numeric entities like &#123;
                        if let Some(num_str) = entity.strip_prefix("#") {
                            if let Ok(n) = num_str.parse::<u32>() {
                                if let Some(c) = char::from_u32(n) {
                                    result.push(c);
                                    continue;
                                }
                            }
                        }
                        // unknown entity — keep as-is
                        result.push('&');
                        result.push_str(&entity);
                        result.push(';');
                    }
                }
            } else {
                // No closing semicolon — treat & as literal
                result.push('&');
                result.push_str(&entity);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Strip ANSI escape sequences from text.
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Estimate the number of rendered terminal rows after ratatui wrapping.
pub fn wrapped_line_count(text: &str, width: u16) -> usize {
    let width = width.max(1) as usize;
    text.lines()
        .map(|line| {
            let chars = line.chars().count();
            chars.saturating_sub(1) / width + 1
        })
        .sum::<usize>()
}

/// Render a character-based scrollbar for a panel.
/// Clamp scroll position to a safe range: 0 .. max(0, content_lines - viewport_h).
/// Prevents overflow in ratatui's `area.height + scroll.y` calculation.
pub fn clamp_scroll(scroll: u16, content_lines: usize, viewport_h: u16) -> u16 {
    let max = content_lines
        .saturating_sub(viewport_h as usize)
        .min(10_000) as u16;
    scroll.min(max)
}

/// `scroll`: current scroll offset, `content_height`: total lines of content,
/// `viewport_height`: visible lines in the panel.
pub fn render_scrollbar(scroll: u16, content_height: usize, viewport_height: u16) -> String {
    let vh = viewport_height.max(1) as usize;
    let ch = content_height.max(1);
    if ch <= vh {
        return String::new(); // no scrollbar needed
    }
    let thumb_h = ((vh as f64 / ch as f64) * vh as f64).ceil() as usize;
    let max_thumb = vh.saturating_sub(thumb_h);
    let thumb_pos = {
        let pos = ((scroll as f64 / (ch - vh) as f64) * max_thumb as f64).round() as usize;
        pos.min(max_thumb)
    };

    let mut bar = String::with_capacity(vh);
    for i in 0..vh {
        if i >= thumb_pos && i < thumb_pos + thumb_h {
            bar.push('█');
        } else {
            bar.push('│');
        }
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AgentPhase::main_split_ratio ──

    #[test]
    fn test_split_ratio_sums_to_100() {
        for phase in &[
            AgentPhase::Idle,
            AgentPhase::Observing,
            AgentPhase::Planning,
            AgentPhase::Executing,
            AgentPhase::Reflecting,
            AgentPhase::Evolving,
        ] {
            for has_weights in [false, true] {
                let (l, r) = phase.main_split_ratio(has_weights);
                assert_eq!(
                    l + r,
                    100,
                    "phase={phase:?} has_weights={has_weights}: {l}+{r} != 100"
                );
            }
        }
    }

    #[test]
    fn test_split_ratio_planning_wider() {
        // Planning should have wider left panel than Idle
        let (plan_l, _) = AgentPhase::Planning.main_split_ratio(false);
        let (idle_l, _) = AgentPhase::Idle.main_split_ratio(false);
        assert!(
            plan_l >= idle_l,
            "Planning left {plan_l} should be >= Idle left {idle_l}"
        );
    }

    #[test]
    fn test_split_ratio_with_weights_narrower_left() {
        // With weights, left panel should be narrower
        for phase in &[
            AgentPhase::Planning,
            AgentPhase::Executing,
            AgentPhase::Idle,
        ] {
            let (l_no, _) = phase.main_split_ratio(false);
            let (l_yes, _) = phase.main_split_ratio(true);
            assert!(
                l_yes < l_no,
                "phase={phase:?}: with weights {l_yes} should be < without {l_no}"
            );
        }
    }

    // ── FocusedPanel ──

    #[test]
    fn test_focused_panel_cycle() {
        // 3 visible panels: MainLeft -> Evolution -> Input -> MainLeft
        assert_eq!(FocusedPanel::MainLeft.next(), FocusedPanel::Evolution);
        assert_eq!(FocusedPanel::Evolution.next(), FocusedPanel::Input);
        assert_eq!(FocusedPanel::Input.next(), FocusedPanel::MainLeft);
        // MiniLog invisible, skips to Input
        assert_eq!(FocusedPanel::MiniLog.next(), FocusedPanel::Input);

        assert_eq!(FocusedPanel::Input.prev(), FocusedPanel::Evolution);
        assert_eq!(FocusedPanel::Evolution.prev(), FocusedPanel::MainLeft);
        assert_eq!(FocusedPanel::MainLeft.prev(), FocusedPanel::Input);
        assert_eq!(FocusedPanel::MiniLog.prev(), FocusedPanel::Evolution);
    }

    // ── LeftTab ──

    #[test]
    fn test_left_tab_toggle() {
        assert_eq!(LeftTab::Plan.next(), LeftTab::Execution);
        assert_eq!(LeftTab::Execution.next(), LeftTab::Plan);
    }

    // ── truncate ──

    #[test]
    fn test_truncate_no_op() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        let result = truncate("hello world", 5);
        assert!(result.starts_with("hello"));
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_truncate_utf8_boundary() {
        // Chinese characters are 3 bytes each in UTF-8
        let text = "你好世界测试文本";
        let result = truncate(text, 3);
        assert_eq!(result.chars().count(), 4); // 3 chars + '…'
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate("", 5), "");
    }

    // ── strip_ansi ──

    #[test]
    fn test_strip_ansi_color_codes() {
        assert_eq!(strip_ansi("\x1b[32mgreen\x1b[0m"), "green");
        assert_eq!(strip_ansi("\x1b[1;31mbold red\x1b[0m"), "bold red");
    }

    #[test]
    fn test_strip_ansi_no_escape() {
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn test_strip_ansi_empty() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_only_escape() {
        assert_eq!(strip_ansi("\x1b[0m"), "");
    }

    #[test]
    fn test_strip_ansi_complex() {
        let input = "\x1b[35m📝 Summary\x1b[0m\n\x1b[2mDetails\x1b[0m";
        let result = strip_ansi(input);
        assert!(result.contains("📝 Summary"));
        assert!(result.contains("Details"));
        assert!(!result.contains('\x1b'));
    }

    #[test]
    fn test_strip_ansi_non_color_csi() {
        assert_eq!(strip_ansi("\x1b[2Khello\x1b[?25l"), "hello");
    }

    // ── wrapped_line_count ──

    #[test]
    fn test_wrapped_line_count_single_line() {
        assert_eq!(wrapped_line_count("hello", 10), 1);
    }

    #[test]
    fn test_wrapped_line_count_wraps_long_line() {
        assert_eq!(wrapped_line_count("abcdef", 3), 2);
    }

    #[test]
    fn test_wrapped_line_count_multiple_lines() {
        assert_eq!(wrapped_line_count("abcde\nxy", 3), 3);
    }

    // ── render_scrollbar ──

    #[test]
    fn test_scrollbar_no_bar_when_content_fits() {
        // content_height <= viewport_height → no scrollbar
        let bar = render_scrollbar(0, 5, 10);
        assert!(bar.is_empty());
    }

    #[test]
    fn test_scrollbar_has_thumb() {
        let bar = render_scrollbar(0, 100, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.contains('█'), "should have thumb chars: {bar:?}");
        assert!(bar.contains('│'), "should have track chars: {bar:?}");
    }

    #[test]
    fn test_scrollbar_at_end() {
        let bar = render_scrollbar(u16::MAX, 100, 10);
        assert_eq!(bar.chars().count(), 10);
        // Thumb should be at the bottom
        assert!(bar.ends_with('█'));
    }

    #[test]
    fn test_scrollbar_min_viewport() {
        // viewport_height = 0 should be clamped to 1
        let bar = render_scrollbar(0, 100, 0);
        assert_eq!(bar.chars().count(), 1);
    }

    #[test]
    fn test_scrollbar_min_content() {
        // content_height = 0 should be clamped to 1
        let bar = render_scrollbar(0, 0, 10);
        assert!(bar.is_empty(), "content=0 <= viewport=10, no bar needed");
    }

    // ── TuiAppState initial values ──

    #[test]
    fn test_split_pct_uses_override_when_set() {
        let evo = Arc::new(evolution::EvolutionEngine::new(
            0.01,
            Arc::new(memory::MockMemoryStore::default()),
        ));
        let mut state = TuiAppState::new("test".into(), evo);
        state.phase = AgentPhase::Idle;
        // No override: uses phase default
        let default = state.split_pct();
        // Set override
        state.left_split_pct = Some(80);
        let (l, r) = state.split_pct();
        assert_eq!((l, r), (80, 20));
        // Remove override
        state.left_split_pct = None;
        let restored = state.split_pct();
        assert_eq!(restored, default);
    }

    #[test]
    fn test_initial_state_defaults() {
        let evo = Arc::new(evolution::EvolutionEngine::new(
            0.01,
            Arc::new(memory::MockMemoryStore::default()),
        ));
        let state = TuiAppState::new("test".into(), evo);
        assert_eq!(state.agent_name, "test");
        assert_eq!(state.turn, 0);
        assert_eq!(state.phase, AgentPhase::Idle);
        assert_eq!(state.focused_panel, FocusedPanel::MainLeft);
        assert_eq!(state.left_tab, LeftTab::Execution);
        assert!(!state.should_quit);
        assert!(!state.agent_done);
        assert!(state.results_visible);
        assert!(!state.log_visible);
        assert!(state.output_overlay.is_none());
        assert!(state.exec_selected_index.is_none());
    }

    // ── strip_html ──

    #[test]
    fn test_strip_html_removes_tags() {
        assert_eq!(strip_html("<div>hello</div>"), "hello");
        assert_eq!(strip_html("<p>text</p>"), "text");
        assert_eq!(strip_html("<br/>"), "");
        assert_eq!(strip_html("<br />"), "");
    }

    #[test]
    fn test_strip_html_nested_tags() {
        assert_eq!(strip_html("<div><p>nested</p></div>"), "nested");
        assert_eq!(strip_html("<span class=\"foo\">content</span>"), "content");
    }

    #[test]
    fn test_strip_html_decodes_entities() {
        assert_eq!(strip_html("&lt;div&gt;"), "<div>");
        assert_eq!(strip_html("&amp;"), "&");
        assert_eq!(strip_html("&quot;"), "\"");
        assert_eq!(strip_html("&apos;"), "'");
        assert_eq!(strip_html("&#39;"), "'");
        assert_eq!(strip_html("&nbsp;"), " ");
    }

    #[test]
    fn test_strip_html_mixed_content() {
        assert_eq!(strip_html("<div>hello <b>world</b></div>"), "hello world");
        assert_eq!(strip_html("text &amp; <br/> more"), "text &  more");
    }

    #[test]
    fn test_strip_html_plain_text_passthrough() {
        assert_eq!(strip_html("plain text"), "plain text");
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn test_strip_html_incomplete_entity() {
        // & without ; should be kept as-is
        assert_eq!(strip_html("a & b"), "a & b");
        assert_eq!(strip_html("a &lt b"), "a &lt b");
    }

    // ── UTF-8 boundary safety (regression test for content_full truncation) ──

    #[test]
    fn test_utf8_boundary_safe_truncation() {
        // Simulate the truncation logic in handle_event
        let text = "a".repeat(9998) + "你好世界"; // 9998 ASCII + 4 multi-byte chars
        assert!(text.len() > 10000); // bytes > 10000
        let limit = 10_000;
        let mut end = limit;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let _s = text[..end].to_string(); // must not panic
        assert!(end <= limit);
        assert!(
            end < 10000,
            "end should have stepped back from 10000 boundary"
        );
    }

    // ── TuiInput get/set gateway_mode ──

    #[test]
    fn test_gateway_mode_default_empty() {
        let input = TuiInput::new();
        assert_eq!(input.get_gateway_mode(), "");
    }

    #[test]
    fn test_gateway_mode_set_without_shared_is_noop() {
        let input = TuiInput::new();
        // gateway_mode is None by default, set_gateway_mode should not panic
        input.set_gateway_mode("cost");
        assert_eq!(input.get_gateway_mode(), "");
    }

    #[test]
    fn test_gateway_mode_set_and_get() {
        let mut input = TuiInput::new();
        let shared = Arc::new(parking_lot::Mutex::new(None::<String>));
        input.gateway_mode = Some(Arc::clone(&shared));
        input.set_gateway_mode("cost");
        assert_eq!(input.get_gateway_mode(), "cost");
        input.set_gateway_mode("latency");
        assert_eq!(input.get_gateway_mode(), "latency");
    }

    // ── clamp_scroll ──

    #[test]
    fn test_clamp_scroll_within_bounds() {
        assert_eq!(clamp_scroll(3, 10, 5), 3);
    }

    #[test]
    fn test_clamp_scroll_at_max() {
        // max = 10 - 5 = 5, clamped to 10_000 → 5
        assert_eq!(clamp_scroll(10, 10, 5), 5);
    }

    #[test]
    fn test_clamp_scroll_zero_content() {
        assert_eq!(clamp_scroll(0, 0, 5), 0);
    }

    #[test]
    fn test_clamp_scroll_viewport_larger_than_content() {
        // content_lines < viewport → max = 0
        assert_eq!(clamp_scroll(5, 3, 10), 0);
    }

    #[test]
    fn test_clamp_scroll_huge_scroll_capped_at_10k() {
        // max = 15_000 - 5 = 14_995, clamped to 10_000
        assert_eq!(clamp_scroll(12_000, 15_000, 5), 10_000);
    }

    // ── render_scrollbar ──

    #[test]
    fn test_render_scrollbar_no_scrollbar_when_content_fits() {
        assert_eq!(render_scrollbar(0, 5, 10), "");
    }

    #[test]
    fn test_render_scrollbar_basic() {
        let bar = render_scrollbar(0, 30, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.contains('█'), "should have a thumb");
    }

    #[test]
    fn test_render_scrollbar_thumb_moves() {
        let bar_top = render_scrollbar(0, 30, 10);
        let bar_bottom = render_scrollbar(20, 30, 10);
        // Thumb positions should differ
        assert_ne!(bar_top, bar_bottom);
    }

    #[test]
    fn test_render_scrollbar_zero_viewport() {
        // viewport 0 → clamp to 1 → content 30 <= 1? no → scrollbar rendered
        let bar = render_scrollbar(0, 30, 0);
        assert!(!bar.is_empty());
    }

    // ── SettingsTab ──

    #[test]
    fn test_settings_tab_next_full_cycle() {
        use SettingsTab::*;
        assert_eq!(Llm.next(), Search);
        assert_eq!(Search.next(), Finance);
        assert_eq!(Finance.next(), Feishu);
        assert_eq!(Feishu.next(), Theme);
        assert_eq!(Theme.next(), Llm);
    }

    #[test]
    fn test_settings_tab_prev_full_cycle() {
        use SettingsTab::*;
        assert_eq!(Llm.prev(), Theme);
        assert_eq!(Theme.prev(), Feishu);
        assert_eq!(Feishu.prev(), Finance);
        assert_eq!(Finance.prev(), Search);
        assert_eq!(Search.prev(), Llm);
    }

    #[test]
    fn test_settings_tab_label_all() {
        use SettingsTab::*;
        assert_eq!(Llm.label(), "LLM");
        assert_eq!(Search.label(), "搜索");
        assert_eq!(Finance.label(), "金融");
        assert_eq!(Theme.label(), "主题");
        assert_eq!(Feishu.label(), "飞书");
    }

    // ── LogFilter ──

    #[test]
    fn test_log_filter_next_toggles() {
        assert_eq!(LogFilter::All.next(), LogFilter::ErrorsOnly);
        assert_eq!(LogFilter::ErrorsOnly.next(), LogFilter::All);
    }

    #[test]
    fn test_log_filter_label() {
        assert_eq!(LogFilter::All.label(), "All");
        assert_eq!(LogFilter::ErrorsOnly.label(), "Errors");
    }

    // ── render_scrollbar boundary cases ──

    #[test]
    fn test_scrollbar_content_exactly_equals_viewport() {
        // content == viewport → no scrollbar needed
        let bar = render_scrollbar(0, 10, 10);
        assert_eq!(bar, "", "no scrollbar when content fits exactly");
    }

    #[test]
    fn test_scrollbar_content_zero() {
        // content=0 → clamps to 1, viewport=5 → 1 <= 5 → no scrollbar
        let bar = render_scrollbar(0, 0, 5);
        assert_eq!(bar, "");
    }

    #[test]
    fn test_scrollbar_content_one_more_than_viewport() {
        // content=11, viewport=10 → scrollbar shown, length == viewport
        let bar = render_scrollbar(0, 11, 10);
        assert!(!bar.is_empty());
        assert_eq!(bar.chars().count(), 10);
    }

    #[test]
    fn test_scrollbar_max_scroll_thumb_at_bottom() {
        // When scrolled to max, thumb should be at bottom of bar
        let bar = render_scrollbar(u16::MAX, 100, 10);
        // thumb char should appear near the end
        let chars: Vec<char> = bar.chars().collect();
        assert_eq!(chars.len(), 10);
        assert!(chars.contains(&'█'));
        // The last char should be the thumb
        assert_eq!(*chars.last().unwrap(), '█');
    }

    #[test]
    fn test_scrollbar_scroll_zero_thumb_at_top() {
        let bar = render_scrollbar(0, 100, 10);
        let chars: Vec<char> = bar.chars().collect();
        // The first char should be the thumb
        assert_eq!(chars[0], '█');
    }

    #[test]
    fn test_scrollbar_large_content_small_viewport() {
        // Very large content vs tiny viewport
        let bar = render_scrollbar(0, 10000, 3);
        assert_eq!(bar.chars().count(), 3);
        assert!(bar.contains('█'));
    }

    #[test]
    fn test_scrollbar_output_only_block_and_pipe() {
        // Bar should only contain '█' and '│'
        let bar = render_scrollbar(5, 50, 10);
        for c in bar.chars() {
            assert!(c == '█' || c == '│', "unexpected char in scrollbar: {c:?}");
        }
    }

    #[test]
    fn test_scrollbar_mid_scroll_thumb_not_at_extremes() {
        let bar = render_scrollbar(50, 200, 20);
        let chars: Vec<char> = bar.chars().collect();
        assert_eq!(chars.len(), 20);
        // Thumb should exist but not at top (index 0) or bottom (index 19)
        // (scroll is roughly in the middle)
        assert!(chars.contains(&'█'));
    }

    // ── FocusedPanel — extended bidirectional cycle ──

    #[test]
    fn test_focused_panel_full_forward_cycle() {
        let mut panel = FocusedPanel::MainLeft;
        panel = panel.next(); // Evolution
        assert_eq!(panel, FocusedPanel::Evolution);
        panel = panel.next(); // Input
        assert_eq!(panel, FocusedPanel::Input);
        panel = panel.next(); // back to MainLeft
        assert_eq!(panel, FocusedPanel::MainLeft);
    }

    #[test]
    fn test_focused_panel_full_backward_cycle() {
        let mut panel = FocusedPanel::MainLeft;
        panel = panel.prev(); // Input
        assert_eq!(panel, FocusedPanel::Input);
        panel = panel.prev(); // Evolution
        assert_eq!(panel, FocusedPanel::Evolution);
        panel = panel.prev(); // back to MainLeft
        assert_eq!(panel, FocusedPanel::MainLeft);
    }

    #[test]
    fn test_focused_panel_minilog_forward_skips_to_input() {
        // MiniLog.next() → Input (MiniLog is skipped in Tab cycle)
        assert_eq!(FocusedPanel::MiniLog.next(), FocusedPanel::Input);
    }

    #[test]
    fn test_focused_panel_minilog_backward_goes_to_evolution() {
        assert_eq!(FocusedPanel::MiniLog.prev(), FocusedPanel::Evolution);
    }

    #[test]
    fn test_focused_panel_forward_and_back_returns_to_start() {
        let start = FocusedPanel::Input;
        let next = start.next();
        let back = next.prev();
        // prev(next(Input)) = prev(MainLeft) = Input
        assert_eq!(back, start);
    }

    #[test]
    fn test_focused_panel_backward_and_forward_returns_to_start() {
        let start = FocusedPanel::Evolution;
        let prev = start.prev();
        let fwd = prev.next();
        // next(prev(Evolution)) = next(MainLeft) = Evolution
        assert_eq!(fwd, start);
    }

    // ── TuiInput — construction and defaults ──

    #[test]
    fn test_tui_input_new_awaiting_false() {
        let ti = TuiInput::new();
        assert!(!ti.awaiting.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_tui_input_default_buffer_empty() {
        let ti = TuiInput::new();
        assert!(ti.buffer.lock().is_empty());
    }

    #[test]
    fn test_tui_input_default_submitted_none() {
        let ti = TuiInput::new();
        assert!(ti.submitted.lock().is_none());
    }

    #[test]
    fn test_tui_input_default_cursor_zero() {
        let ti = TuiInput::new();
        assert_eq!(*ti.cursor.lock(), 0);
    }

    #[test]
    fn test_tui_input_get_gateway_mode_default_empty() {
        let ti = TuiInput::new();
        assert_eq!(ti.get_gateway_mode(), "");
    }

    #[test]
    fn test_tui_input_get_gateway_mode_with_value() {
        use parking_lot::Mutex;
        let mode = Arc::new(Mutex::new(Some("gateway-mode".to_string())));
        let mut ti = TuiInput::new();
        ti.gateway_mode = Some(Arc::clone(&mode));
        assert_eq!(ti.get_gateway_mode(), "gateway-mode");
    }

    // ── TuiAppState — help_scroll field ──

    #[test]
    fn test_tui_state_initial_help_scroll_is_zero() {
        let mem: Arc<dyn agent_core::MemoryStore> = Arc::new(memory::MockMemoryStore::default());
        let evo = Arc::new(evolution::EvolutionEngine::new(0.01, mem));
        let state = TuiAppState::new("test".into(), evo);
        assert_eq!(state.help_scroll, 0);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_tui_state_initial_agent_done_false() {
        let mem: Arc<dyn agent_core::MemoryStore> = Arc::new(memory::MockMemoryStore::default());
        let evo = Arc::new(evolution::EvolutionEngine::new(0.01, mem));
        let state = TuiAppState::new("test".into(), evo);
        assert!(!state.agent_done);
        assert_eq!(state.phase, AgentPhase::Idle);
    }

    // ── AgentPhase::main_split_ratio ──

    #[test]
    fn test_split_ratio_planning_no_weights() {
        let r = AgentPhase::Planning.main_split_ratio(false);
        assert_eq!(r, (85, 15));
    }

    #[test]
    fn test_split_ratio_planning_with_weights() {
        let r = AgentPhase::Planning.main_split_ratio(true);
        assert_eq!(r, (75, 25));
    }

    #[test]
    fn test_split_ratio_executing_no_weights() {
        let r = AgentPhase::Executing.main_split_ratio(false);
        assert_eq!(r, (80, 20));
    }

    #[test]
    fn test_split_ratio_executing_with_weights() {
        let r = AgentPhase::Executing.main_split_ratio(true);
        assert_eq!(r, (70, 30));
    }

    #[test]
    fn test_split_ratio_idle_no_weights() {
        let r = AgentPhase::Idle.main_split_ratio(false);
        assert_eq!(r, (75, 25));
    }

    #[test]
    fn test_split_ratio_idle_with_weights() {
        let r = AgentPhase::Idle.main_split_ratio(true);
        assert_eq!(r, (60, 40));
    }

    #[test]
    fn test_split_ratio_always_sums_to_100() {
        for phase in &[
            AgentPhase::Idle,
            AgentPhase::Observing,
            AgentPhase::Planning,
            AgentPhase::Executing,
            AgentPhase::Reflecting,
            AgentPhase::Evolving,
        ] {
            for has_weights in [false, true] {
                let (l, r) = phase.main_split_ratio(has_weights);
                assert_eq!(l + r, 100, "split doesn't sum to 100 for {phase:?}");
            }
        }
    }

    // ── strip_html — additional edge cases ──

    #[test]
    fn test_strip_html_ampersand_lt() {
        assert_eq!(strip_html("a &lt; b"), "a < b");
    }

    #[test]
    fn test_strip_html_ampersand_gt() {
        assert_eq!(strip_html("a &gt; b"), "a > b");
    }

    #[test]
    fn test_strip_html_ampersand_amp() {
        assert_eq!(strip_html("a &amp; b"), "a & b");
    }

    #[test]
    fn test_strip_html_ampersand_quot() {
        assert_eq!(strip_html("&quot;hello&quot;"), "\"hello\"");
    }

    #[test]
    fn test_strip_html_nbsp() {
        assert_eq!(strip_html("a&nbsp;b"), "a b");
    }

    #[test]
    fn test_strip_html_numeric_entity() {
        // &#65; = 'A'
        assert_eq!(strip_html("&#65;"), "A");
    }

    #[test]
    fn test_strip_html_unclosed_tag_skips() {
        // Unclosed tag — strips until end
        let result = strip_html("<unclosed");
        // Should be empty (everything after < is skipped)
        assert_eq!(result, "");
    }

    #[test]
    fn test_strip_html_empty_string() {
        assert_eq!(strip_html(""), "");
    }

    // ── clamp_scroll ──

    #[test]
    fn test_clamp_scroll_above_max_clamped() {
        // scroll=100, content=10, viewport=5 → max scroll = 10-5=5
        let clamped = clamp_scroll(100, 10, 5);
        assert_eq!(clamped, 5);
    }

    #[test]
    fn test_clamp_scroll_at_zero_stays_zero() {
        assert_eq!(clamp_scroll(0, 20, 10), 0);
    }

    #[test]
    fn test_clamp_scroll_content_smaller_than_viewport() {
        // content <= viewport → max scroll = 0
        assert_eq!(clamp_scroll(50, 5, 10), 0);
    }

    #[test]
    fn test_clamp_scroll_exact_max() {
        // scroll == max allowed → keep
        let clamped = clamp_scroll(5, 15, 10);
        assert_eq!(clamped, 5);
    }

    // ── wrapped_line_count — edge cases ──

    #[test]
    fn test_wrapped_line_count_empty_string() {
        // Empty string has 0 lines via .lines()
        assert_eq!(wrapped_line_count("", 80), 0);
    }

    #[test]
    fn test_wrapped_line_count_exactly_fits() {
        // "hello" (5 chars) with width=5 → 1 line
        assert_eq!(wrapped_line_count("hello", 5), 1);
    }

    #[test]
    fn test_wrapped_line_count_overflow_by_one() {
        // "hello!" (6 chars) with width=5 → 2 lines
        assert_eq!(wrapped_line_count("hello!", 5), 2);
    }

    #[test]
    fn test_wrapped_line_count_zero_width_clamp() {
        // width=0 is clamped to 1, so every char is its own line
        let count = wrapped_line_count("abc", 0);
        // 3 chars, width 1 → 3 lines
        assert_eq!(count, 3);
    }

    // ── truncate — extra cases ──

    #[test]
    fn test_truncate_at_exact_limit() {
        let s = truncate("hello", 5);
        assert_eq!(s, "hello"); // exactly max_chars, no truncation
    }

    #[test]
    fn test_truncate_one_over_limit() {
        let s = truncate("hello!", 5);
        assert!(s.ends_with('…'));
        assert!(s.starts_with("hello"));
    }

    #[test]
    fn test_truncate_unicode_counts_chars_not_bytes() {
        // "日本語" = 3 chars; max=2 → truncated
        let s = truncate("日本語", 2);
        assert!(s.ends_with('…'));
        assert!(s.contains('日'));
    }

    // ── KanbanItem / KanbanStatus ──

    #[test]
    fn test_kanban_item_construction() {
        let item = KanbanItem {
            id: "item-1".into(),
            title: "Test Task".into(),
            status: KanbanStatus::Pending,
        };
        assert_eq!(item.title, "Test Task");
        assert_eq!(item.status, KanbanStatus::Pending);
    }

    #[test]
    fn test_kanban_status_variants_distinct() {
        assert_ne!(KanbanStatus::Pending, KanbanStatus::InProgress);
        assert_ne!(KanbanStatus::InProgress, KanbanStatus::Completed);
        assert_ne!(KanbanStatus::Pending, KanbanStatus::Completed);
    }

    #[test]
    fn test_kanban_item_status_mutability() {
        let mut item = KanbanItem {
            id: "item-2".into(),
            title: "Task".into(),
            status: KanbanStatus::Pending,
        };
        item.status = KanbanStatus::InProgress;
        assert_eq!(item.status, KanbanStatus::InProgress);
        item.status = KanbanStatus::Completed;
        assert_eq!(item.status, KanbanStatus::Completed);
    }
}
