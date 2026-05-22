# TUI Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 全面优化 Hermes TUI 的界面和交互：阶段自适应布局、焦点系统、统一视觉调色板、滚动条、鼠标支持和快捷键系统。

**Architecture:** 在现有 `crates/tui/` crate 基础上改造。核心改动：state.rs 增加焦点/布局状态字段；各 panel 文件增加 `focused` 参数并调整渲染逻辑；render.rs 实现阶段感知的动态布局；run.rs 增加焦点导航、鼠标事件、输入历史。新增 footer.rs 和 help.rs 两个 panel 文件。

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, parking_lot

---

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `crates/tui/src/state.rs` | Modify | 新增 FocusedPanel 枚举、阶段/进度/焦点/历史字段 |
| `crates/tui/src/panels/footer.rs` | Create | 底部单行快捷键提示栏 |
| `crates/tui/src/panels/help.rs` | Create | 帮助 overlay 弹窗（全屏快捷键表） |
| `crates/tui/src/panels/log.rs` | Create (from summary.rs) | 完整可滚动日志面板 |
| `crates/tui/src/panels/summary.rs` | Delete | 被 log.rs 替代 |
| `crates/tui/src/panels/header.rs` | Modify | 微调样式 |
| `crates/tui/src/panels/plan.rs` | Modify | 移除 step list，纯 streaming 显示 + 滚动条 |
| `crates/tui/src/panels/execution.rs` | Modify | 层级缩进 + 进度条 + 滚动条 |
| `crates/tui/src/panels/evolution.rs` | Modify | 可折叠 sections + 滚动条 |
| `crates/tui/src/panels/mod.rs` | Modify | 更新模块声明 |
| `crates/tui/src/render.rs` | Modify | 阶段感知布局 + 焦点边框 + 底部 footer |
| `crates/tui/src/run.rs` | Modify | 焦点导航、滚动键、鼠标滚轮、帮助切换、输入历史 |

---

### Task 1: State foundation — add types and fields

**Files:**
- Modify: `crates/tui/src/state.rs`

- [ ] **Step 1: Add FocusedPanel enum and new fields to TuiAppState**

Replace the entire content of `crates/tui/src/state.rs`:

```rust
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
    pub duration_ms: Option<u64>,
    pub layer: usize,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub message: String,
    pub is_error: bool,
}

/// Per-panel layout rects updated each frame for mouse hit-testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutRects {
    pub main_left: (u16, u16, u16, u16),
    pub evolution: (u16, u16, u16, u16),
    pub mini_log: (u16, u16, u16, u16),
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

    // Layout rects for mouse hit-testing
    pub layout: LayoutRects,
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
            layout: LayoutRects::default(),
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
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p tui 2>&1
```

Expected: compiles with warnings about unused fields (will be used in later tasks).

---

### Task 2: Create footer panel

**Files:**
- Create: `crates/tui/src/panels/footer.rs`

- [ ] **Step 1: Write footer.rs**

```rust
// crates/tui/src/panels/footer.rs
// Single-line footer with context-sensitive keybinding hints.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, FocusedPanel, TuiAppState};

pub fn render_footer(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let hint = if state.help_visible {
        "[Esc/h/F1]关闭帮助"
    } else if state.awaiting_input {
        "[Enter]提交  [Backspace]删除  [↑↓]历史"
    } else {
        match (state.focused_panel, state.phase) {
            (FocusedPanel::MainLeft, AgentPhase::Planning) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助"
            }
            (FocusedPanel::MainLeft, AgentPhase::Executing) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助"
            }
            (FocusedPanel::MainLeft, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Home/End]首尾  [q]退出  [h]帮助"
            }
            (FocusedPanel::Evolution, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Enter]展开/折叠  [q]退出  [h]帮助"
            }
            (FocusedPanel::MiniLog, _) => {
                "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [q]退出  [h]帮助"
            }
        }
    };

    let span = Span::styled(hint, Style::default().fg(Color::DarkGray));
    let para = Paragraph::new(span);
    frame.render_widget(para, area);
}
```

---

### Task 3: Create help overlay panel

**Files:**
- Create: `crates/tui/src/panels/help.rs`

- [ ] **Step 1: Write help.rs**

```rust
// crates/tui/src/panels/help.rs
// Full-screen help overlay listing all keyboard shortcuts.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;

pub fn render_help(frame: &mut Frame, area: Rect, _state: &TuiAppState) {
    // Center a 50x16 help box
    let h_margin = (area.width.saturating_sub(50)) / 2;
    let v_margin = (area.height.saturating_sub(16)) / 2;

    let popup_area = Rect {
        x: area.x + h_margin,
        y: area.y + v_margin,
        width: 50.min(area.width),
        height: 16.min(area.height),
    };

    // Clear background behind popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" Help — 快捷键 ", Style::default().fg(Color::Cyan)));

    let inner = Layout::vertical([Constraint::Length(1), Constraint::Min(1)])
        .split(popup_area)[1];

    let lines = vec![
        Line::from(Span::styled("── 全局 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  q / Esc / Ctrl+C    ", Style::default().fg(Color::White)),
            Span::styled("退出程序", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  h / F1              ", Style::default().fg(Color::White)),
            Span::styled("显示/关闭此帮助", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled("── 焦点与滚动 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  Tab / Shift+Tab     ", Style::default().fg(Color::White)),
            Span::styled("顺时针/逆时针切换焦点面板", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  ↑↓ / j k            ", Style::default().fg(Color::White)),
            Span::styled("聚焦面板逐行滚动", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  PgUp / PgDn         ", Style::default().fg(Color::White)),
            Span::styled("聚焦面板翻页", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("  Home / End          ", Style::default().fg(Color::White)),
            Span::styled("跳到顶部/底部", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled("── Evolution 面板 ──", Style::default().fg(Color::DarkGray))),
        Line::from(vec![
            Span::styled("  Enter               ", Style::default().fg(Color::White)),
            Span::styled("展开/折叠当前 section", Style::default().fg(Color::Gray)),
        ]),
    ];

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, popup_area);
}
```

---

### Task 4: Transform summary.rs into log.rs

**Files:**
- Create: `crates/tui/src/panels/log.rs`
- Delete: `crates/tui/src/panels/summary.rs`

- [ ] **Step 1: Write log.rs (full scrollable log with scrollbar)**

```rust
// crates/tui/src/panels/log.rs
// Scrollable log panel showing agent log entries with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Log ", Style::default().fg(Color::Magenta)));

    let inner = block.inner(area);
    let viewport_h = inner.height;

    if state.log_entries.is_empty() && state.summary.is_none() {
        let text = Paragraph::new("暂无日志")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let lines: Vec<Line> = state
        .log_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let color = if entry.is_error {
                Color::Red
            } else {
                // Dim older entries (beyond last 3)
                let threshold = state.log_entries.len().saturating_sub(3);
                if i < threshold {
                    Color::DarkGray
                } else {
                    Color::Gray
                }
            };
            Line::from(Span::styled(&entry.message, Style::default().fg(color)))
        })
        .collect();

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.log_scroll, 0));

    frame.render_widget(para, area);

    // Render scrollbar on the right edge
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.log_scroll, content_height, viewport_h);
        for (i, ch) in bar.chars().enumerate() {
            let bar_span = Span::styled(
                ch.to_string(),
                Style::default().fg(Color::DarkGray),
            );
            let bar_area = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y + 1 + i as u16,
                width: 1,
                height: 1,
            };
            frame.render_widget(Paragraph::new(bar_span), bar_area);
        }
    }
}

/// Render a compact mini-log (3-line version used during Planning/Executing phases).
pub fn render_mini_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Log ", Style::default().fg(Color::Magenta)));

    let count = state.log_entries.len();
    let start = if count > 3 { count - 3 } else { 0 };

    let lines: Vec<Line> = state
        .log_entries
        .iter()
        .skip(start)
        .map(|entry| {
            let color = if entry.is_error {
                Color::Red
            } else {
                Color::Gray
            };
            Line::from(Span::styled(
                crate::state::truncate(&entry.message, area.width.saturating_sub(4) as usize),
                Style::default().fg(color),
            ))
        })
        .collect();

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
```

- [ ] **Step 2: Delete summary.rs**

```bash
rm /Users/nova/proj/rust_hermess/crates/tui/src/panels/summary.rs
```

---

### Task 5: Update panels/mod.rs

**Files:**
- Modify: `crates/tui/src/panels/mod.rs`

- [ ] **Step 1: Replace module declarations**

Replace the entire content:

```rust
pub mod evolution;
pub mod execution;
pub mod footer;
pub mod header;
pub mod help;
pub mod input;
pub mod log;
pub mod plan;
```

---

### Task 6: Update header panel — minor style

**Files:**
- Modify: `crates/tui/src/panels/header.rs`

- [ ] **Step 1: Update header.rs with refined styling**

```rust
// crates/tui/src/panels/header.rs
// Single-line header: agent name, turn number, active phase indicator.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, TuiAppState};

pub fn render_header(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let (phase_str, phase_color) = if state.agent_done {
        ("完成 — 按 q 退出", Color::Green)
    } else {
        let s = match state.phase {
            AgentPhase::Idle => "空闲",
            AgentPhase::Observing => "观察中...",
            AgentPhase::Planning => "规划中...",
            AgentPhase::Executing => "执行中...",
            AgentPhase::Reflecting => "反思中...",
            AgentPhase::Evolving => "进化中...",
        };
        let c = match state.phase {
            AgentPhase::Idle => Color::DarkGray,
            AgentPhase::Observing => Color::White,
            AgentPhase::Planning => Color::Cyan,
            AgentPhase::Executing => Color::Yellow,
            AgentPhase::Reflecting => Color::Magenta,
            AgentPhase::Evolving => Color::Green,
        };
        (s, c)
    };

    let spinner = match state.frame_count % 8 {
        0 => '⣾', 1 => '⣽', 2 => '⣻', 3 => '⢿',
        4 => '⡿', 5 => '⣟', 6 => '⣯', _ => '⣷',
    };

    let left = Span::styled(
        format!("🜁 {} ", state.agent_name),
        Style::default().fg(Color::LightBlue),
    );
    let turn = Span::styled(
        format!("— 第 {} 轮 — ", state.turn),
        Style::default().fg(Color::White),
    );
    let spinner_span = Span::styled(
        format!("{} ", spinner),
        Style::default().fg(phase_color),
    );
    let phase = Span::styled(phase_str, Style::default().fg(phase_color));

    let line = ratatui::text::Line::from(vec![left, turn, spinner_span, phase]);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}
```

---

### Task 7: Update plan panel — pure streaming, remove step list

**Files:**
- Modify: `crates/tui/src/panels/plan.rs`

- [ ] **Step 1: Rewrite plan.rs for pure streaming display with scrollbar**

```rust
// crates/tui/src/panels/plan.rs
// Plan panel: shows streaming LLM output during planning with scrollbar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_plan(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Plan ({}) ", state.plan_steps_count),
            Style::default().fg(Color::Cyan),
        ));

    let inner = block.inner(area);

    if state.streaming_buffer.is_empty() && !state.plan_ready {
        let text = Paragraph::new("等待规划...")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    let cursor = if state.frame_count % 16 < 8 { "▌" } else { " " };
    let content = format!("{}{}", state.streaming_buffer, cursor);

    let line_count = content.lines().count();

    let text = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((state.plan_scroll, 0));

    frame.render_widget(text, area);

    // Scrollbar
    let vh = inner.height.max(1);
    if line_count > vh as usize {
        let bar = render_scrollbar(state.plan_scroll, line_count, vh);
        for (i, ch) in bar.chars().enumerate() {
            let bar_span = Span::styled(
                ch.to_string(),
                Style::default().fg(Color::DarkGray),
            );
            let bar_area = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y + 1 + i as u16,
                width: 1,
                height: 1,
            };
            frame.render_widget(Paragraph::new(bar_span), bar_area);
        }
    }
}
```

---

### Task 8: Update execution panel — layer indentation + progress bar

**Files:**
- Modify: `crates/tui/src/panels/execution.rs`

- [ ] **Step 1: Rewrite execution.rs with indentation and progress bar**

```rust
// crates/tui/src/panels/execution.rs
// Execution panel: per-step progress with layer indentation, progress bar, scrollbar.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_execution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let completed = state.exec_completed_steps;
    let total = state.exec_total_steps;

    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Execution ({}/{}) ", completed, total),
            Style::default().fg(Color::Yellow),
        ));

    if state.executions.is_empty() {
        let text = Paragraph::new("等待执行...")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(text, area);
        return;
    }

    // Split: step list + progress bar
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .split(block.inner(area));

    let list_area = chunks[0];
    let bar_area = chunks[1];

    let inner = block.inner(area);
    let viewport_h = list_area.height.max(1);

    // Step lines with layer indentation
    let lines: Vec<Line> = state
        .executions
        .iter()
        .map(|step| {
            let indent = "  ".repeat(step.layer.min(6));

            let (icon, color) = match step.status {
                crate::state::StepStatus::Pending => ("○", Color::DarkGray),
                crate::state::StepStatus::Running => {
                    let blink = if state.frame_count % 16 < 8 { "◉" } else { "◎" };
                    (blink, Color::Yellow)
                }
                crate::state::StepStatus::Success => ("✓", Color::Green),
                crate::state::StepStatus::Failed => ("✗", Color::Red),
            };

            let tool = Span::styled(
                format!("{}{} {}", indent, icon, step.tool),
                Style::default().fg(color),
            );

            let duration = step.duration_ms.map(|d| {
                Span::styled(
                    format!("  ({:.1}s)", d as f64 / 1000.0),
                    Style::default().fg(Color::DarkGray),
                )
            });

            let content = step.content_preview.as_deref().map_or_else(
                || Span::raw(""),
                |c| {
                    let clean = crate::state::strip_ansi(c);
                    let short = crate::state::truncate(&clean, 50);
                    Span::styled(format!("  {}", short), Style::default().fg(Color::Gray))
                },
            );

            let mut spans = vec![tool];
            if let Some(d) = duration {
                spans.push(d);
            }
            spans.push(content);
            Line::from(spans)
        })
        .collect();

    let content_height = lines.len();

    // Render step list
    let para = Paragraph::new(lines)
        .block(Block::default())
        .scroll((state.exec_scroll, 0));
    frame.render_widget(para, block.inner(area));

    // Render block border on top
    frame.render_widget(
        Paragraph::new("").block(block),
        area,
    );

    // Progress bar
    if total > 0 {
        let bar_width = (bar_area.width.saturating_sub(2)) as usize;
        let filled = if total > 0 {
            (completed as f64 / total as f64 * bar_width as f64).round() as usize
        } else {
            0
        };
        let empty = bar_width.saturating_sub(filled);
        let bar_text = format!(
            " {}{}  {}/{}  {}%",
            "█".repeat(filled),
            "░".repeat(empty),
            completed,
            total,
            if total > 0 { completed * 100 / total } else { 0 }
        );

        let bar_span = Span::styled(bar_text, Style::default().fg(Color::Cyan));
        let bar_para = Paragraph::new(bar_span);
        frame.render_widget(bar_para, bar_area);
    }

    // Scrollbar
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.exec_scroll, content_height, viewport_h);
        for (i, ch) in bar.chars().enumerate() {
            let bar_span = Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray));
            let bar_rect = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: list_area.y + i as u16,
                width: 1,
                height: 1,
            };
            frame.render_widget(Paragraph::new(bar_span), bar_rect);
        }
    }
}
```

---

### Task 9: Update evolution panel — collapsible sections

**Files:**
- Modify: `crates/tui/src/panels/evolution.rs`

- [ ] **Step 1: Rewrite evolution.rs with collapsible sections and scrollbar**

```rust
// crates/tui/src/panels/evolution.rs
// Evolution panel: collapsible strategy weights, insight stats, learning rate.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;

pub fn render_evolution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Evolution ", Style::default().fg(Color::Green)));

    let stats = state.evolution.stats();
    let win_rate = stats.win_rate() * 100.0;
    let lr = state.evolution.current_learning_rate();
    let insight_count = state.evolution.insight_count();

    let mut lines: Vec<Line> = Vec::new();

    // ── Stats section ──
    let collapse_icon = if state.evo_stats_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 统计数据", collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_stats_hidden {
        lines.push(Line::from(Span::styled(
            format!(
                "  Win Rate: {:.1}%  ({}+ / {}- / {} total)",
                win_rate, stats.positive, stats.negative, stats.total
            ),
            Style::default().fg(Color::White),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  Avg: {:.3}  ", stats.avg_score),
                Style::default().fg(if stats.avg_score >= 0.0 {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
            Span::styled(
                format!("Best: {:.3}  ", stats.best_score),
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                format!("Worst: {:.3}", stats.worst_score),
                Style::default().fg(Color::Red),
            ),
        ]));
    }

    lines.push(Line::from(Span::styled("", Style::default())));

    // ── Strategy weights section ──
    let w_collapse_icon = if state.evo_weights_hidden {
        "▶"
    } else {
        "▼"
    };
    lines.push(Line::from(Span::styled(
        format!("{} 策略权重", w_collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_weights_hidden {
        let weights = state.evolution.all_weights();
        if weights.is_empty() {
            lines.push(Line::from(Span::styled(
                "  暂无数据",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let max_abs = weights
                .iter()
                .map(|(_, w)| w.abs())
                .fold(0.0_f64, f64::max)
                .max(1.0);

            for (name, w) in weights.iter().take(24) {
                let bar_width = ((w.abs() / max_abs) * 12.0).round() as usize;
                let bar = if *w >= 0.0 {
                    "█".repeat(bar_width)
                } else {
                    "░".repeat(bar_width)
                };
                let color = if *w >= 0.0 {
                    Color::Green
                } else {
                    Color::Red
                };
                let name_span = Span::styled(
                    format!("  {:<14}", crate::state::truncate(name, 14)),
                    Style::default().fg(Color::White),
                );
                let bar_span = Span::styled(
                    format!(" {} {:+.3}", bar, w),
                    Style::default().fg(color),
                );
                lines.push(Line::from(vec![name_span, bar_span]));
            }
        }
    }

    lines.push(Line::from(Span::styled("", Style::default())));

    // ── Meta section ──
    let m_collapse_icon = if state.evo_meta_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 元信息", m_collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_meta_hidden {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  LR: {:.5}  ", lr),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Insights: {}  ", insight_count),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Strategies: {}", state.evolution.strategy_count()),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.evo_scroll, 0));

    frame.render_widget(para, area);
}
```

---

### Task 10: Update render.rs — phase-adaptive layout + footer

**Files:**
- Modify: `crates/tui/src/render.rs`

- [ ] **Step 1: Rewrite render.rs with phase-aware layout**

```rust
// crates/tui/src/render.rs
// Main render function: phase-adaptive layout with focus-aware borders.

use ratatui::layout::{Constraint, Layout, Rect};
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
    let (left_pct, right_pct) = match state.phase {
        AgentPhase::Planning => (80, 20),
        AgentPhase::Executing => (75, 25),
        _ => (60, 40),
    };

    let h_chunks = Layout::horizontal([
        Constraint::Percentage(left_pct),
        Constraint::Percentage(right_pct),
    ])
    .split(main_area);

    let left_area = h_chunks[0];
    let right_area = h_chunks[1];

    // ── Update layout rects for mouse hit-testing ──
    // (We can't mutate state here because we only have &TuiAppState.
    // Layout rects are stored in a separate approach — see note below.)
    // Mouse hit-testing will compute layout independently in run.rs.

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

    // ── Render footer ──
    panels::footer::render_footer(frame, footer_area, state);

    // ── Render input bar (overlays footer when awaiting input) ──
    if state.awaiting_input {
        panels::input::render_input(frame, footer_area, state);
    }

    // ── Render help overlay (on top of everything) ──
    if state.help_visible {
        panels::help::render_help(frame, area, state);
    }
}
```

---

### Task 11: Update run.rs — interaction overhaul

**Files:**
- Modify: `crates/tui/src/run.rs`

- [ ] **Step 1: Rewrite run.rs with focus navigation, mouse wheel, input history, help toggle**

Replace the entire file:

```rust
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
                    let _ = e;
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
                        let state = app_state.read();
                        match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                drop(state);
                                scroll_mouse(app_state.clone(), 1, mouse.column, mouse.row);
                            }
                            MouseEventKind::ScrollUp => {
                                drop(state);
                                scroll_mouse(
                                    app_state.clone(),
                                    -1,
                                    mouse.column,
                                    mouse.row,
                                );
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

    // Signal completion
    {
        let mut state = app_state.write();
        state.agent_done = true;
        state.phase = AgentPhase::Idle;
        state.summary = Some("完成 — 按 q 或 Esc 退出".into());
    }

    let _ = tui_task.await;

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
    let max = u16::MAX;
    match state.focused_panel {
        FocusedPanel::MainLeft => match state.phase {
            AgentPhase::Planning => state.plan_scroll = max,
            AgentPhase::Executing => state.exec_scroll = max,
            _ => state.log_scroll = max,
        },
        FocusedPanel::Evolution => state.evo_scroll = max,
        FocusedPanel::MiniLog => state.log_scroll = max,
    }
}

fn toggle_evolution_section(state: &mut TuiAppState) {
    // Toggle the currently-focused section: cycle through stats, weights, meta
    // Simple approach: toggle all three in order, or use focused sub-section.
    // For simplicity, toggle weights first (most likely to overflow), then stats, then meta.
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
fn scroll_mouse(
    app_state: Arc<parking_lot::RwLock<TuiAppState>>,
    delta: i16,
    col: u16,
    row: u16,
) {
    let state = app_state.read();
    // Determine panel by hardcoded heuristic: recompute layout from terminal size
    // to know which panel the mouse is over.
    let term_size = crossterm::terminal::size().unwrap_or((80, 24));

    let needs_mini_log =
        matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing);

    let header_h = 1;
    let footer_h = 1;
    let mini_log_h = if needs_mini_log { 3 } else { 0 };
    let main_h = term_size.1.saturating_sub(header_h + footer_h + mini_log_h);

    let (left_pct, _right_pct) = match state.phase {
        AgentPhase::Planning => (80, 20),
        AgentPhase::Executing => (75, 25),
        _ => (60, 40),
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
                drop(state);
                let mut s = app_state.write();
                s.plan_scroll = apply_delta(s.plan_scroll, delta);
            }
            AgentPhase::Executing => {
                drop(state);
                let mut s = app_state.write();
                s.exec_scroll = apply_delta(s.exec_scroll, delta);
            }
            _ => {
                drop(state);
                let mut s = app_state.write();
                s.log_scroll = apply_delta(s.log_scroll, delta);
            }
        }
    } else if in_main && !in_left {
        // Evolution panel
        drop(state);
        let mut s = app_state.write();
        s.evo_scroll = apply_delta(s.evo_scroll, delta);
    } else if needs_mini_log {
        // Mini-log area
        drop(state);
        let mut s = app_state.write();
        s.log_scroll = apply_delta(s.log_scroll, delta);
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
            state.exec_total_steps = steps_count;
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
```

---

### Task 12: Build and verify

- [ ] **Step 1: Check compilation**

```bash
cargo check -p tui 2>&1
```

Expected: compilation succeeds with no errors.

- [ ] **Step 2: Build release**

```bash
cargo build --release -p tui 2>&1
```

Expected: build succeeds.

- [ ] **Step 3: Build full project**

```bash
cargo check 2>&1
```

Expected: full workspace compiles.
