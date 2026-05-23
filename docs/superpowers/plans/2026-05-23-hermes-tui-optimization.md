# Hermes TUI 深度优化 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 提升 TUI 信息密度和可用性：Plan 持久化、结构化结果报告、Overlay 弹窗查看完整输出、Evolution 动态宽度。

**Architecture:** 纯 TUI 层改动，不触 agent-core。6 个任务按依赖排序：数据结构奠基 → 独立小功能 → 核心交互 → 集成验证。每个任务编译通过并手动验证 TUI 行为。

**Tech Stack:** Rust, ratatui, crossterm, parking_lot

---

### Task 1: state.rs — 数据结构奠基

**Files:**
- Modify: `crates/tui/src/state.rs`

所有后续任务依赖此任务的数据结构。

- [ ] **Step 1: 在 state.rs 顶部新增 `LeftTab` 枚举**

在 `FocusedPanel` 定义之后、`TuiAppState` 定义之前插入：

```rust
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
```

- [ ] **Step 2: 新增 `StepOutputOverlay` 结构体**

紧接 `LeftTab` impl block 之后：

```rust
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
```

- [ ] **Step 3: 在 `StepExecState` 中新增 `content_full` 字段**

```rust
#[derive(Debug, Clone)]
pub struct StepExecState {
    pub step_id: Uuid,
    pub tool: String,
    pub status: StepStatus,
    pub content_preview: Option<String>,
    pub content_full: Option<String>,   // 新增：完整输出（上限 10KB）
    pub duration_ms: Option<u64>,
    pub layer: usize,
}
```

- [ ] **Step 4: 在 `TuiAppState` 中新增字段**

在原有字段末尾、`evo_scroll` 之后添加：

```rust
// Tab selection for left panel during Planning/Executing
pub left_tab: LeftTab,

// Execution step selection for overlay
pub exec_selected_index: Option<usize>,

// Full-screen output overlay state
pub output_overlay: Option<StepOutputOverlay>,

// Total agent duration for results report
pub total_duration_ms: Option<u64>,
```

注意：`content_full` 字段已经在 `StepExecState` 中添加了，这里不需要再加。需要删除 TuiAppState 中重复的 `content_full`。

说明：`content_full` 字段添加在 `StepExecState` 结构体中（Step 3），不在 `TuiAppState` 中。

- [ ] **Step 5: 更新 `TuiAppState::new()` 初始化新字段**

在 `new()` 函数返回 `Self { ... }` 的字段列表中，`evo_scroll: 0,` 之后添加：

```rust
            left_tab: LeftTab::Execution,
            exec_selected_index: None,
            output_overlay: None,
            total_duration_ms: None,
```

- [ ] **Step 6: 编译验证**

```bash
cargo build -p tui 2>&1 | head -20
```
预期：编译通过，仅有 unused field 的 warning（后续任务会使用）。

- [ ] **Step 7: Commit**

```bash
git add crates/tui/src/state.rs
git commit -m "feat(tui): add LeftTab, StepOutputOverlay, content_full, and new state fields"
```

---

### Task 2: Evolution 面板动态宽度

**Files:**
- Modify: `crates/tui/src/render.rs:38-42`

纯渲染逻辑变更，不依赖其他任务。

- [ ] **Step 1: 修改 render.rs 的水平分割比例**

将现有的 `match state.phase` 比例计算替换为带权重判断的版本：

```rust
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
```

同时更新 `run.rs` 中 `scroll_mouse` 函数的布局计算以匹配。将 `scroll_mouse` 中的比例计算也更新为同样的逻辑：

```rust
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
```

- [ ] **Step 2: 编译验证**

```bash
cargo build -p tui 2>&1
```
预期：编译通过。

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/render.rs crates/tui/src/run.rs
git commit -m "feat(tui): dynamic evolution panel width based on strategy data"
```

---

### Task 3: StepCompleted 存储完整输出

**Files:**
- Modify: `crates/tui/src/run.rs:462-500`

- [ ] **Step 1: 修改 `handle_event` 中 `StepCompleted` 分支**

在 `handle_event` 函数中，找到 `AgentEvent::StepCompleted { output }` 处理，在设置 `step.content_preview` 的同时添加 `content_full`（strip_ansi，截断到 10KB）：

将现有的：
```rust
step.content_preview = Some(crate::state::truncate(
    &crate::state::strip_ansi(&output.content),
    100,
));
```

替换为：
```rust
let clean = crate::state::strip_ansi(&output.content);
step.content_full = Some({
    let limit = 10_000; // 10KB upper bound
    if clean.len() > limit {
        let mut s = clean[..limit].to_string();
        s.push_str("…[truncated]");
        s
    } else {
        clean.clone()
    }
});
step.content_preview = Some(crate::state::truncate(&clean, 100));
```

- [ ] **Step 2: 编译验证**

```bash
cargo build -p tui 2>&1
```
预期：编译通过。

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/run.rs
git commit -m "feat(tui): store full step output in StepExecState for overlay viewing"
```

---

### Task 4: Plan Tab 切换

**Files:**
- Modify: `crates/tui/src/render.rs:53-78`
- Modify: `crates/tui/src/panels/plan.rs:12-65`
- Modify: `crates/tui/src/run.rs` — 键盘处理部分

- [ ] **Step 1: 修改 render.rs — 左侧面板渲染逻辑**

将 Planning/Executing 阶段的左侧渲染逻辑改为带 Tab 栏的布局。替换 `match state.phase` 左侧部分（约 53-78 行）：

```rust
// ── Render left panel based on phase ──
match state.phase {
    AgentPhase::Planning | AgentPhase::Executing => {
        // Split left area: tab bar (1) + content (fill)
        let left_chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(left_area);

        let tab_area = left_chunks[0];
        let content_area = left_chunks[1];

        // Render tab bar
        let plan_style = if state.left_tab == LeftTab::Plan {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Gray)
        };
        let exec_style = if state.left_tab == LeftTab::Execution {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Gray)
        };
        let tabs_text = Line::from(vec![
            Span::styled("[Plan]", plan_style),
            Span::raw(" "),
            Span::styled("[Exec]", exec_style),
        ]);
        frame.render_widget(Paragraph::new(tabs_text), tab_area);

        // Render content based on selected tab
        let main_focused = state.focused_panel == FocusedPanel::MainLeft;
        match state.left_tab {
            LeftTab::Plan => {
                panels::plan::render_plan(frame, content_area, state, main_focused);
            }
            LeftTab::Execution => {
                panels::execution::render_execution(frame, content_area, state, main_focused);
            }
        }
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
```

需要在 render.rs 顶部添加必要的 import：
```rust
use ratatui::style::{Modifier, ...};  // 确认 Modifier 已在 import 中
use ratatui::text::{Line, Span};       // 确认已在 import 中
use ratatui::widgets::Paragraph;
```

检查现有 import 并确保 `Modifier` 已导入。

- [ ] **Step 2: 修改 run.rs — Tab 键在 Planning/Executing 阶段切换 LeftTab**

在键盘处理的 Normal mode 部分，修改 `KeyCode::Tab` 处理：

```rust
KeyCode::Tab => {
    // In Planning/Executing phase, Tab switches left panel tabs
    if matches!(state.phase, AgentPhase::Planning | AgentPhase::Executing)
        && state.focused_panel == FocusedPanel::MainLeft
    {
        state.left_tab = state.left_tab.next();
    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
        state.focused_panel = state.focused_panel.prev();
    } else {
        state.focused_panel = state.focused_panel.next();
    }
}
```

- [ ] **Step 3: 修改 run.rs — TurnStarted 时重置 left_tab**

在 `handle_event` 中 `AgentEvent::TurnStarted { .. }` 分支添加：

```rust
state.left_tab = LeftTab::Execution;
```

（新回合开始默认显示 Execution tab）

- [ ] **Step 4: 修改 run.rs — PlanPhaseStarted 时切换到 Plan tab**

在 `handle_event` 中 `AgentEvent::PlanPhaseStarted` 分支添加：

```rust
state.left_tab = LeftTab::Plan;
```

- [ ] **Step 5: 修改 plan.rs — 支持在 Executing 阶段渲染**

`render_plan` 函数当前在 `streaming_buffer` 为空且 `!plan_ready` 时显示 "等待规划..."。在 Executing 阶段，plan 已 ready，应该显示静态内容（无闪烁光标）。修改第 37 行：

```rust
let cursor = if state.phase == AgentPhase::Planning && state.frame_count % 16 < 8 { "▌" } else { " " };
let content = format!("{}{}", state.streaming_buffer, cursor);
```

并在文件顶部确认 import 中包含 `AgentPhase`：
```rust
use crate::state::{render_scrollbar, AgentPhase, TuiAppState};
```

- [ ] **Step 6: 编译验证**

```bash
cargo build -p tui 2>&1
```
预期：编译通过。

- [ ] **Step 7: Commit**

```bash
git add crates/tui/src/render.rs crates/tui/src/run.rs crates/tui/src/panels/plan.rs
git commit -m "feat(tui): add Plan/Exec tab switching in left panel"
```

---

### Task 5: Overlay 弹窗查看完整输出

**Files:**
- Create: `crates/tui/src/panels/overlay.rs`
- Modify: `crates/tui/src/panels/mod.rs`
- Modify: `crates/tui/src/panels/execution.rs`
- Modify: `crates/tui/src/run.rs`
- Modify: `crates/tui/src/render.rs`

- [ ] **Step 1: 创建 panels/overlay.rs**

```rust
// crates/tui/src/panels/overlay.rs
// Full-screen overlay for inspecting complete step output.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, StepOutputOverlay};

pub fn render_overlay(frame: &mut Frame, area: Rect, overlay: &StepOutputOverlay) {
    // Dimmed background
    let dim = Paragraph::new("")
        .style(Style::default());
    frame.render_widget(Clear, area);
    frame.render_widget(dim, area);

    // Overlay takes 80% of screen
    let ow = (area.width as f64 * 0.8) as u16;
    let oh = (area.height as f64 * 0.8) as u16;
    let ox = area.x + (area.width.saturating_sub(ow)) / 2;
    let oy = area.y + (area.height.saturating_sub(oh)) / 2;
    let overlay_rect = Rect::new(ox, oy, ow, oh);

    let status_icon = match overlay.status {
        crate::state::StepStatus::Success => "✓",
        crate::state::StepStatus::Failed => "✗",
        crate::state::StepStatus::Running => "◎",
        crate::state::StepStatus::Pending => "○",
    };

    let title = format!(
        " {} {} | {} | {:.1}s ",
        status_icon,
        overlay.tool,
        overlay.step_id.to_string().chars().take(8).collect::<String>(),
        overlay.duration_ms.unwrap_or(0) as f64 / 1000.0,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(title, Style::default().fg(Color::White)));

    let inner = block.inner(overlay_rect);
    let viewport_h = inner.height.saturating_sub(2); // reserve for info + separator

    let mut lines: Vec<Line> = Vec::new();

    // Info line
    let duration_str = overlay
        .duration_ms
        .map(|d| format!("{:.1}s", d as f64 / 1000.0))
        .unwrap_or_else(|| "N/A".to_string());
    lines.push(Line::from(Span::styled(
        format!("Tool: {}  |  Duration: {}  |  Status: {}", overlay.tool, duration_str, status_icon),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(inner.width.saturating_sub(2).max(20) as usize),
        Style::default().fg(Color::DarkGray),
    )));

    // Content lines (preserving newlines)
    for line in overlay.full_content.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        )));
    }

    let content_height = lines.len().saturating_sub(2); // exclude info + separator

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((overlay.scroll, 0));

    frame.render_widget(para, overlay_rect);

    // Scrollbar
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(overlay.scroll, content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| Line::from(Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray))))
            .collect();
        let bar_rect = Rect {
            x: overlay_rect.x + overlay_rect.width.saturating_sub(1),
            y: overlay_rect.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
```

- [ ] **Step 2: 注册 overlay 模块**

修改 `crates/tui/src/panels/mod.rs`，在末尾添加：

```rust
pub mod overlay;
```

- [ ] **Step 3: 修改 execution.rs — 高亮选中步骤行**

在 `render_execution` 函数中，构建 lines 时对选中行加高亮背景。找到 `.map(|step| { ... })` 并修改 tool span 的 style：

在 `let tool = Span::styled(...)` 之前，先判断是否选中：

```rust
let is_selected = state.exec_selected_index == Some(idx);
```

然后在构建 line 时，如果 `is_selected` 则包裹在带有背景色的 Span 中。具体做法：修改返回的 Line，当 `is_selected` 时，给所有 spans 添加背景色：

在 `.map(|step| {` 闭包的末尾、`Line::from(spans)` 之前，添加：

```rust
            if let Some(sel_idx) = state.exec_selected_index {
                if state.executions.iter().position(|s| s.step_id == step.step_id) == Some(sel_idx) {
                    for span in &mut spans {
                        *span = span.clone().style(Style::default().fg(span.style.fg.unwrap_or(Color::White)).bg(Color::DarkGray));
                    }
                }
            }
```

更简洁的做法：在 `.enumerate()` 后比较索引。将 `.iter()` 改为 `.iter().enumerate()`，然后：

```rust
let is_selected = state.exec_selected_index == Some(idx);

// ... 构建 spans 后:
let line_style = if is_selected {
    Style::default().bg(Color::DarkGray)
} else {
    Style::default()
};
Line::from(spans).style(line_style)
```

- [ ] **Step 4: 修改 run.rs — Execution 面板键盘交互**

在 Normal mode 键盘处理中，修改 Execution + MainLeft 聚焦时的 Up/Down/j/k 行为。找到现有的 `KeyCode::Up | KeyCode::Char('k')` 和 `KeyCode::Down | KeyCode::Char('j')` 处理，加入条件判断：

```rust
KeyCode::Up | KeyCode::Char('k') => {
    if state.phase == AgentPhase::Executing && state.focused_panel == FocusedPanel::MainLeft {
        let len = state.executions.len();
        if len > 0 {
            let idx = state.exec_selected_index.unwrap_or(0);
            let new_idx = if idx > 0 { idx - 1 } else { 0 };
            state.exec_selected_index = Some(new_idx);
            // Auto-scroll to keep selected visible
            if (new_idx as u16) < state.exec_scroll {
                state.exec_scroll = new_idx as u16;
            }
        }
    } else {
        scroll_focused(&mut state, -1);
    }
}
KeyCode::Down | KeyCode::Char('j') => {
    if state.phase == AgentPhase::Executing && state.focused_panel == FocusedPanel::MainLeft {
        let len = state.executions.len();
        if len > 0 {
            let idx = state.exec_selected_index.unwrap_or(0);
            let new_idx = if idx + 1 < len { idx + 1 } else { len - 1 };
            state.exec_selected_index = Some(new_idx);
            // Auto-scroll to keep selected visible
            let viewport = 10_u16; // approximate
            if (new_idx as u16) >= state.exec_scroll + viewport {
                state.exec_scroll = (new_idx as u16).saturating_sub(viewport - 1);
            }
        }
    } else {
        scroll_focused(&mut state, 1);
    }
}
```

- [ ] **Step 5: 修改 run.rs — Enter 键打开 overlay**

在 Normal mode 的 `KeyCode::Enter` 处理中，加入 overlay 打开逻辑：

```rust
KeyCode::Enter => {
    if state.output_overlay.is_some() {
        state.output_overlay = None;
    } else if state.phase == AgentPhase::Executing
        && state.focused_panel == FocusedPanel::MainLeft
    {
        if let Some(idx) = state.exec_selected_index {
            if let Some(step) = state.executions.get(idx) {
                state.output_overlay = Some(StepOutputOverlay {
                    step_id: step.step_id,
                    tool: step.tool.clone(),
                    status: step.status.clone(),
                    duration_ms: step.duration_ms,
                    full_content: step.content_full.clone().unwrap_or_else(||
                        step.content_preview.clone().unwrap_or_default()
                    ),
                    scroll: 0,
                });
            }
        }
    } else if state.focused_panel == FocusedPanel::Evolution {
        toggle_evolution_section(&mut state);
    }
}
```

- [ ] **Step 6: 修改 run.rs — Overlay 打开时的键盘处理**

在键盘处理的最顶部（Help overlay 检查之后、Input mode 检查之前），添加 overlay 模式检查：

```rust
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
```

- [ ] **Step 7: 修改 render.rs — overlay 渲染**

在 `render_app` 函数末尾（`help_visible` 渲染之后），添加 overlay 渲染（overlay 应该在 help 之上，因为打开 overlay 时不应同时看到 help）：

```rust
// ── Render output overlay (on top of everything) ──
if let Some(ref overlay) = state.output_overlay {
    panels::overlay::render_overlay(frame, area, overlay);
}
```

将此代码放在 help overlay 渲染之后、函数结束之前。

- [ ] **Step 8: 编译验证**

```bash
cargo build -p tui 2>&1
```
预期：编译通过。

- [ ] **Step 9: Commit**

```bash
git add crates/tui/src/panels/overlay.rs crates/tui/src/panels/mod.rs crates/tui/src/panels/execution.rs crates/tui/src/run.rs crates/tui/src/render.rs
git commit -m "feat(tui): add full-screen overlay for complete step output viewing"
```

---

### Task 6: 结构化执行报告

**Files:**
- Create: `crates/tui/src/panels/results.rs`
- Modify: `crates/tui/src/panels/mod.rs`
- Modify: `crates/tui/src/render.rs`
- Modify: `crates/tui/src/run.rs` — 记录 total_duration_ms

- [ ] **Step 1: 创建 panels/results.rs**

```rust
// crates/tui/src/panels/results.rs
// Structured results report shown after agent completes.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{render_scrollbar, TuiAppState};

pub fn render_results(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Results ", Style::default().fg(Color::Green)));

    let inner = block.inner(area);
    let viewport_h = inner.height;
    let text_width = area.width.saturating_sub(2).max(20) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Summary
    if let Some(ref summary) = state.summary {
        lines.push(Line::from(Span::styled(
            format!(" 结果: {}", summary),
            Style::default().fg(Color::Yellow),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " 结果: (无)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(text_width),
        Style::default().fg(Color::DarkGray),
    )));

    // Key metrics
    let total_duration = state.total_duration_ms.map(|d| format!("{:.1}s", d as f64 / 1000.0))
        .unwrap_or_else(|| "N/A".to_string());
    let completed = state.exec_completed_steps;
    let total = state.exec_total_steps;
    let success_count = state.executions.iter()
        .filter(|s| s.status == crate::state::StepStatus::Success)
        .count();

    lines.push(Line::from(vec![
        Span::styled(
            format!(" 总耗时: {}  ", total_duration),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("步骤: {}/{}  ", completed, total),
            Style::default().fg(if completed == total && total > 0 { Color::Green } else { Color::Yellow }),
        ),
        Span::styled(
            format!("成功: {}/{}", success_count, total),
            Style::default().fg(if success_count == total && total > 0 { Color::Green } else { Color::Red }),
        ),
    ]));

    lines.push(Line::from(Span::raw("")));

    // Step list
    if !state.executions.is_empty() {
        for step in &state.executions {
            let (icon, color) = match step.status {
                crate::state::StepStatus::Success => ("✓", Color::Green),
                crate::state::StepStatus::Failed => ("✗", Color::Red),
                crate::state::StepStatus::Running => ("◎", Color::Yellow),
                crate::state::StepStatus::Pending => ("○", Color::DarkGray),
            };
            let indent = "  ".repeat(step.layer.min(4));
            let duration = step.duration_ms
                .map(|d| format!("({:.1}s)", d as f64 / 1000.0))
                .unwrap_or_default();

            let tool_text = format!("{} {} {} {}", indent, icon, step.tool, duration);
            lines.push(Line::from(Span::styled(tool_text, Style::default().fg(color))));

            if let Some(ref preview) = step.content_preview {
                let preview_line = format!("  {}   {}", indent, crate::state::truncate(preview, text_width.saturating_sub(indent.len() + 4)));
                lines.push(Line::from(Span::styled(
                    preview_line,
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    lines.push(Line::from(Span::raw("")));

    // Reflection (last reflection entry from log)
    if let Some(reflection) = state.log_entries.iter()
        .rev()
        .find(|e| e.message.starts_with("反思:"))
    {
        lines.push(Line::from(Span::styled(
            "─".repeat(text_width),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            &reflection.message,
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        " 按 q 退出    |    按 Tab 切换到 Log 面板",
        Style::default().fg(Color::DarkGray),
    )));

    let content_height = lines.len();

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.log_scroll, 0));

    frame.render_widget(para, area);

    // Scrollbar
    if content_height > viewport_h as usize {
        let bar = render_scrollbar(state.log_scroll, content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| Line::from(Span::styled(ch.to_string(), Style::default().fg(Color::DarkGray))))
            .collect();
        let bar_rect = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
```

- [ ] **Step 2: 注册 results 模块**

修改 `crates/tui/src/panels/mod.rs`，在 `pub mod overlay;` 之后添加：

```rust
pub mod results;
```

- [ ] **Step 3: 修改 render.rs — Idle 阶段条件渲染**

将 `match state.phase` 的 `_ => { panels::log::render_log(...) }` 分支改为：

```rust
_ => {
    if state.agent_done {
        panels::results::render_results(
            frame,
            left_area,
            state,
            state.focused_panel == FocusedPanel::MainLeft,
        );
    } else {
        panels::log::render_log(
            frame,
            left_area,
            state,
            state.focused_panel == FocusedPanel::MainLeft,
        );
    }
}
```

- [ ] **Step 4: 修改 run.rs — ExecutePhaseComplete 记录 total_duration_ms**

在 `handle_event` 的 `AgentEvent::ExecutePhaseComplete { duration_ms, .. }` 分支中添加：

```rust
state.total_duration_ms = Some(duration_ms);
```

- [ ] **Step 5: 编译验证**

```bash
cargo build -p tui 2>&1
```
预期：编译通过。

- [ ] **Step 6: Commit**

```bash
git add crates/tui/src/panels/results.rs crates/tui/src/panels/mod.rs crates/tui/src/render.rs crates/tui/src/run.rs
git commit -m "feat(tui): add structured results report panel after agent completion"
```

---

### Task 7: 集成验证与清理

**Files:**
- Modify: 无新增，仅验证

- [ ] **Step 1: 完整编译检查**

```bash
cargo build 2>&1
```
预期：全 workspace 编译通过，无 warning（或仅有预存的 unused field warning）。

- [ ] **Step 2: 检查是否有编译 warning**

```bash
cargo build -p tui 2>&1 | grep -i warning
```
预期：仅有无害的 warning（如 `dead_code` 对于 `AgentEvent` 的未使用变体）。

- [ ] **Step 3: 运行现有测试确保无回归**

```bash
cargo test 2>&1
```
预期：所有现有测试通过。

- [ ] **Step 4: 手动验证清单**

启动 TUI 模式并确认：
```bash
# 使用 DeepSeek 配置运行 TUI 模式
cargo run -- --config config/deepseek.toml --tui
```

检查项：
1. Planning 阶段：按 Tab 可在 Plan/Exec 标签间切换
2. Executing 阶段：Up/Down 移动步骤选中高亮，Enter 打开 overlay，Esc 关闭
3. Overlay 中：j/k 滚动内容，PageUp/PageDown 翻页
4. Agent 完成后：左侧显示 Results 面板（summary + 步骤列表 + 反思）
5. Evolution 面板：无策略数据时宽度缩减（约 15-20%）

- [ ] **Step 5: 检查 left_tab 重置逻辑**

确认新回合开始时 `left_tab` 重置为 `Execution`，PlanPhaseStarted 时切换为 `Plan`。

- [ ] **Step 6: Commit（如有微调）**

```bash
git add -A
git commit -m "chore(tui): final integration verification and cleanup"
```

---

## 文件变更汇总

| 文件 | 操作 | 所属任务 |
|------|------|---------|
| `crates/tui/src/state.rs` | 修改 | Task 1 |
| `crates/tui/src/render.rs` | 修改 | Task 2, 4, 5, 6 |
| `crates/tui/src/run.rs` | 修改 | Task 2, 3, 4, 5, 6 |
| `crates/tui/src/panels/plan.rs` | 修改 | Task 4 |
| `crates/tui/src/panels/execution.rs` | 修改 | Task 5 |
| `crates/tui/src/panels/overlay.rs` | 新建 | Task 5 |
| `crates/tui/src/panels/results.rs` | 新建 | Task 6 |
| `crates/tui/src/panels/mod.rs` | 修改 | Task 5, 6 |
