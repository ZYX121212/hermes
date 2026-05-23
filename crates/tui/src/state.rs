// crates/tui/src/state.rs
// Mutable application state updated by agent events and read by the renderer.

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use evolution::EvolutionEngine;
use parking_lot::Mutex;
use uuid::Uuid;

/// Shared input state for TUI interactive mode.
pub struct TuiInput {
    pub awaiting: AtomicBool,
    pub buffer: Mutex<String>,
    pub submitted: Mutex<Option<String>>,
}

impl TuiInput {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            awaiting: AtomicBool::new(false),
            buffer: Mutex::new(String::new()),
            submitted: Mutex::new(None),
        })
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

/// Panel focus target for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    MainLeft,
    Evolution,
    MiniLog,
}

impl FocusedPanel {
    pub fn next(self) -> Self {
        match self {
            FocusedPanel::MainLeft => FocusedPanel::Evolution,
            FocusedPanel::Evolution => FocusedPanel::MiniLog,
            FocusedPanel::MiniLog => FocusedPanel::MainLeft,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            FocusedPanel::MainLeft => FocusedPanel::MiniLog,
            FocusedPanel::Evolution => FocusedPanel::MainLeft,
            FocusedPanel::MiniLog => FocusedPanel::Evolution,
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
    pub content_full: Option<String>,   // 完整输出（上限 10KB）
    pub duration_ms: Option<u64>,
    pub layer: usize,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub message: String,
    pub is_error: bool,
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
    pub should_quit: bool,
    pub agent_done: bool,
    pub awaiting_input: bool,
    pub input_text: String,
    pub help_visible: bool,

    // Input history
    pub input_history: VecDeque<String>,
    pub input_history_pos: Option<usize>,

    // Scroll offsets
    pub plan_scroll: u16,
    pub exec_scroll: u16,
    pub log_scroll: u16,
    pub evo_scroll: u16,

    // Tab selection for left panel during Planning/Executing
    pub left_tab: LeftTab,

    // Execution step selection for overlay
    pub exec_selected_index: Option<usize>,

    // Full-screen output overlay state
    pub output_overlay: Option<StepOutputOverlay>,

    // Total agent duration for results report
    pub total_duration_ms: Option<u64>,

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
            executions: Vec::new(),
            exec_total_steps: 0,
            exec_completed_steps: 0,
            summary: None,
            log_entries: VecDeque::new(),
            evolution,
            evo_stats_hidden: false,
            evo_weights_hidden: false,
            evo_meta_hidden: false,
            focused_panel: FocusedPanel::MainLeft,
            should_quit: false,
            agent_done: false,
            awaiting_input: false,
            input_text: String::new(),
            help_visible: false,
            input_history: VecDeque::with_capacity(50),
            input_history_pos: None,
            plan_scroll: 0,
            exec_scroll: 0,
            log_scroll: 0,
            evo_scroll: 0,
            left_tab: LeftTab::Execution,
            exec_selected_index: None,
            output_overlay: None,
            total_duration_ms: None,
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

/// Strip ANSI escape sequences from text.
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Render a character-based scrollbar for a panel.
/// `scroll`: current scroll offset, `content_height`: total lines of content,
/// `viewport_height`: visible lines in the panel.
pub fn render_scrollbar(scroll: u16, content_height: usize, viewport_height: u16) -> String {
    let vh = viewport_height.max(1) as usize;
    let ch = content_height.max(1);
    if ch <= vh {
        return String::new(); // no scrollbar needed
    }
    let thumb_h = ((vh as f64 / ch as f64) * vh as f64).ceil() as usize;
    let thumb_pos = if ch > vh {
        ((scroll as f64 / (ch - vh) as f64) * (vh - thumb_h) as f64).round() as usize
    } else {
        0
    };

    let mut bar = String::with_capacity(vh);
    for i in 0..vh {
        if i >= thumb_pos && i < thumb_pos + thumb_h {
            bar.push('█');
        } else {
            bar.push('░');
        }
    }
    bar
}
