# TUI 界面精简 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 TUI 界面的视觉杂乱：减少颜色轰炸、动画冲突、冗余文字、信息过密。

**Architecture:** 所有变更限于 `crates/tui/src/` 目录内。不做结构性重构，只做视觉参数的减法 —— 改变颜色值、去掉闪烁、缩短文字、调整高度。不影响任何外部接口。

**Tech Stack:** Rust, ratatui, crossterm

---

### Task 1: state.rs — scrollbar 轨道字符 + Evolution 默认折叠

**Files:**
- Modify: `crates/tui/src/state.rs:288-296` (render_scrollbar)
- Modify: `crates/tui/src/state.rs:207-209` (TuiAppState::new)

- [ ] **Step 1: 修改 scrollbar 轨道字符 `░` → `│`**

`crates/tui/src/state.rs:293`，将：
```rust
bar.push('░');
```
改为：
```rust
bar.push('│');
```

- [ ] **Step 2: 修改 Evolution 默认折叠状态**

`crates/tui/src/state.rs:207-209`，将：
```rust
evo_stats_hidden: false,
evo_weights_hidden: false,
evo_meta_hidden: false,
```
改为：
```rust
evo_stats_hidden: true,
evo_weights_hidden: true,
evo_meta_hidden: true,
```

- [ ] **Step 3: 运行已有测试确保 scrollbar 测试仍然通过**

Run: `cargo test -p tui -- render_scrollbar`
Expected: 全部测试 PASS（字符替换不影响逻辑）

- [ ] **Step 4: 更新 scrollbar 测试中的字符断言**

在 `state.rs` 的测试中找到对 `░` 的断言（测试 `test_scrollbar_has_thumb` 第 470 行），将：
```rust
assert!(bar.contains('░'), "should have track chars: {bar:?}");
```
改为：
```rust
assert!(bar.contains('│'), "should have track chars: {bar:?}");
```

Run: `cargo test -p tui`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/state.rs
git commit -m "fix(tui): change scrollbar track to thin line, default evolution sections collapsed"
```

---

### Task 2: header.rs — 去反色块，改前景色

**Files:**
- Modify: `crates/tui/src/panels/header.rs:48-77`

- [ ] **Step 1: 重写 header 渲染为前景色模式**

将 `crates/tui/src/panels/header.rs` 的 `render_header` 函数中行 48-77 替换。

`left` span (行 48-54) 从反色块改为前景色：
```rust
// Before:
let left = Span::styled(
    format!(" Hermes  {} ", state.agent_name),
    Style::default()
        .fg(theme::BG)
        .bg(theme::BLUE)
        .add_modifier(Modifier::BOLD),
);

// After:
let left = Span::styled(
    format!("Hermes · {} ", state.agent_name),
    Style::default()
        .fg(theme::CYAN)
        .bg(theme::BG)
        .add_modifier(Modifier::BOLD),
);
```

`phase` span (行 63-69) 从反色块改为前景色：
```rust
// Before:
let phase = Span::styled(
    format!(" {} ", phase_str),
    Style::default()
        .fg(theme::BG)
        .bg(phase_color)
        .add_modifier(Modifier::BOLD),
);

// After:
let phase = Span::styled(
    format!("{}", phase_str),
    Style::default()
        .fg(phase_color)
        .bg(theme::BG)
        .add_modifier(Modifier::BOLD),
);
```

行 71-77 的 Line 构建简化为：
```rust
let line = ratatui::text::Line::from(vec![
    left,
    Span::styled("  ", Style::default().bg(theme::BG)),
    turn,
    Span::styled("  ", Style::default().bg(theme::BG)),
    spinner_span,
    Span::styled("  ", Style::default().bg(theme::BG)),
    phase,
]);
let para = Paragraph::new(line).style(Style::default().bg(theme::BG));
```

- [ ] **Step 2: 编译检查**

Run: `cargo build -p tui 2>&1 | head -20`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/panels/header.rs
git commit -m "fix(tui): simplify header to foreground-only colors, remove color blocks"
```

---

### Task 3: plan.rs + execution.rs — 去除闪烁动画

**Files:**
- Modify: `crates/tui/src/panels/plan.rs:30-34`
- Modify: `crates/tui/src/panels/execution.rs:54-59,96`

- [ ] **Step 1: plan.rs — cursor 改为静态**

`crates/tui/src/panels/plan.rs:30-34`，将：
```rust
let cursor = if state.phase == AgentPhase::Planning && state.frame_count % 16 < 8 {
    "▌"
} else {
    " "
};
```
改为：
```rust
let cursor = if state.phase == AgentPhase::Planning {
    "▌"
} else {
    ""
};
```

- [ ] **Step 2: execution.rs — Running icon 改为静态**

`crates/tui/src/panels/execution.rs:54-59`，将：
```rust
crate::state::StepStatus::Running => {
    let blink = if state.frame_count % 16 < 8 {
        "◉"
    } else {
        "◎"
    };
    (blink, theme::YELLOW)
}
```
改为：
```rust
crate::state::StepStatus::Running => ("◉", theme::YELLOW),
```

- [ ] **Step 3: execution.rs — 内容预览截断从 50 → 30**

`crates/tui/src/panels/execution.rs:96`，将：
```rust
let short = crate::state::truncate(&clean, 50);
```
改为：
```rust
let short = crate::state::truncate(&clean, 30);
```

- [ ] **Step 4: 编译检查**

Run: `cargo build -p tui 2>&1 | head -20`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/panels/plan.rs crates/tui/src/panels/execution.rs
git commit -m "fix(tui): remove blink animations on cursor and running icon, shorten preview"
```

---

### Task 4: log.rs — Mini-log 移除 summary 展示

**Files:**
- Modify: `crates/tui/src/panels/log.rs:108-129`

- [ ] **Step 1: render_mini_log — 删除 summary 分支**

删除 `crates/tui/src/panels/log.rs:117-129`（summary 优先展示分支），让 mini-log 直接展示最近日志：

将行 108-129：
```rust
pub fn render_mini_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let block = theme::panel_block("Activity", theme::MAGENTA, focused);

    let count = state.log_entries.len();

    // inner width for truncation
    let text_width = area.width.saturating_sub(2) as usize;

    // Show summary in mini-log if present
    if let Some(ref summary) = state.summary {
        let line = Line::from(Span::styled(
            format!(
                " 结果: {}",
                crate::state::truncate(summary, text_width.saturating_sub(4))
            ),
            Style::default().fg(theme::YELLOW).bg(theme::PANEL),
        ));
        let para = Paragraph::new(line).block(block);
        frame.render_widget(para, area);
        return;
    }

    if count == 0 {
```

改为：
```rust
pub fn render_mini_log(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let block = theme::panel_block("Activity", theme::MAGENTA, focused);

    let count = state.log_entries.len();
    let text_width = area.width.saturating_sub(2) as usize;

    if count == 0 {
```

同时将 `count.saturating_sub(3)` (行 138) 改为 `count.saturating_sub(2)` 以匹配 2 行高度。

- [ ] **Step 2: render_log — 移除 MAGENTA 替换为 MUTED**

`crates/tui/src/panels/log.rs:14`，将：
```rust
let block = theme::panel_block("Log", theme::MAGENTA, focused);
```
改为：
```rust
let block = theme::panel_block("Log", theme::MUTED, focused);
```

`crates/tui/src/panels/log.rs:109`，同文件内 `render_mini_log` 中：
```rust
let block = theme::panel_block("Activity", theme::MAGENTA, focused);
```
改为：
```rust
let block = theme::panel_block("Activity", theme::MUTED, focused);
```

- [ ] **Step 3: 编译检查**

Run: `cargo build -p tui 2>&1 | head -20`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/panels/log.rs
git commit -m "fix(tui): simplify mini-log to 2 lines, remove summary, use muted accent"
```

---

### Task 5: footer.rs — 精简快捷键提示

**Files:**
- Modify: `crates/tui/src/panels/footer.rs` (整个 `render_footer`)

- [ ] **Step 1: 重写 footer hint 逻辑**

替换 `crates/tui/src/panels/footer.rs:14-57` 的 hint 匹配逻辑。

**Before** (当前完整的 match 块):
```rust
let hint = if state.help_visible {
    "[Esc/h/F1]关闭帮助".to_string()
} else if state.output_overlay.is_some() {
    "[Esc/Enter/q]关闭  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [Home/End]首尾".to_string()
} else if state.awaiting_input {
    "[Enter]提交  [Backspace]删除  [↑↓]历史".to_string()
} else {
    match (state.focused_panel, state.phase) {
        (FocusedPanel::MainLeft, AgentPhase::Planning) => {
            let tab_hint = if state.left_tab == LeftTab::Plan {
                "[Tab]切换至Exec"
            } else {
                "[Tab]切换至Plan"
            };
            format!(
                "{tab_hint}  [Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助"
            )
        }
        (FocusedPanel::MainLeft, AgentPhase::Executing) => {
            if state.left_tab == LeftTab::Execution {
                "[Tab]切换至Plan  [Shift+Tab]切换焦点  [↑↓/jk]选择步骤  [Enter]查看完整输出  [PgUp/PgDn]翻页  [q]退出  [h]帮助".to_string()
            } else {
                "[Tab]切换至Exec  [Shift+Tab]切换焦点  [↑↓/jk]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助".to_string()
            }
        }
        (FocusedPanel::MainLeft, _) if state.agent_done => {
            if state.results_visible {
                "[Tab]切换至Log  [Shift+Tab]切换焦点  [↑↓/jk]滚动  [Home/End]首尾  [q]退出  [h]帮助".to_string()
            } else {
                "[Tab]切换至Results  [Shift+Tab]切换焦点  [↑↓/jk]滚动  [Home/End]首尾  [q]退出  [h]帮助".to_string()
            }
        }
        (FocusedPanel::MainLeft, _) => {
            "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Home/End]首尾  [q]退出  [h]帮助".to_string()
        }
        (FocusedPanel::Evolution, _) => {
            "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [Enter]展开/折叠  [q]退出  [h]帮助"
                .to_string()
        }
        (FocusedPanel::MiniLog, _) => {
            "[Tab/Shift+Tab]切换焦点  [↑↓/jk]滚动  [q]退出  [h]帮助".to_string()
        }
    }
};
```

**After**:
```rust
let hint = if state.help_visible {
    "[Esc] 关闭帮助".to_string()
} else if state.output_overlay.is_some() {
    "[Esc] 关闭  [↑↓] 滚动  [PgUp/PgDn] 翻页".to_string()
} else if state.awaiting_input {
    "[Enter] 提交  [Backspace] 删除  [↑↓] 历史".to_string()
} else {
    match (state.focused_panel, state.phase, state.left_tab, state.agent_done) {
        (FocusedPanel::MainLeft, AgentPhase::Planning, LeftTab::Plan, _) => {
            "[Tab] Exec  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::MainLeft, AgentPhase::Executing, LeftTab::Execution, _) => {
            "[Tab] Plan  [↑↓] 选择  [Enter] 详情  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::MainLeft, p, _, _)
            if matches!(p, AgentPhase::Planning | AgentPhase::Executing) =>
        {
            "[Tab] 切换  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::MainLeft, _, _, true) => {
            "[Tab] 切换  [↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::MainLeft, _, _, _) => {
            "[↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::Evolution, _, _, _) => {
            "[↑↓] 滚动  [Enter] 折叠  [q] 退出  [h] 帮助".to_string()
        }
        (FocusedPanel::MiniLog, _, _, _) => {
            "[↑↓] 滚动  [q] 退出  [h] 帮助".to_string()
        }
    }
};
```

- [ ] **Step 2: 清理不再使用的 import**

如果 `AgentPhase` 或 `LeftTab` 的 match 匹配方式改变导致 unused import warning，调整 import。本次修改中 `AgentPhase` 和 `LeftTab` 仍然使用，不需要改动 import。

- [ ] **Step 3: 编译检查**

Run: `cargo build -p tui 2>&1 | head -20`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/panels/footer.rs
git commit -m "fix(tui): simplify footer hints to 2-3 core shortcuts per focus"
```

---

### Task 6: render.rs — Tab bar 去提示文字 + Mini-log 高度

**Files:**
- Modify: `crates/tui/src/render.rs:24-25` (Constraint)
- Modify: `crates/tui/src/render.rs:88-98` (tabs_text)

- [ ] **Step 1: Mini-log 高度从 3 → 2**

`crates/tui/src/render.rs:25`，将：
```rust
Constraint::Length(3), // mini-log
```
改为：
```rust
Constraint::Length(2), // mini-log
```

- [ ] **Step 2: Tab bar 移除提示文字**

`crates/tui/src/render.rs:88-97`，将：
```rust
let tabs_text = Line::from(vec![
    Span::styled(" ", bg_style),
    Span::styled(" PLAN ", plan_style),
    Span::styled(" ", bg_style),
    Span::styled(" EXEC ", exec_style),
    Span::styled(
        "  Tab 切换标签  Shift+Tab 切换焦点",
        Style::default().fg(Color::DarkGray).bg(theme::BG),
    ),
]);
```
改为：
```rust
let tabs_text = Line::from(vec![
    Span::styled(" ", bg_style),
    Span::styled(" PLAN ", plan_style),
    Span::styled(" ", bg_style),
    Span::styled(" EXEC ", exec_style),
]);
```

- [ ] **Step 3: 清理不再使用的 import**

移除 `render.rs` 顶部对 `Color` 的 import（如果不再使用）。检查 `Color::DarkGray` 是否唯一使用处 — 如果是，从 `ratatui::style::Color` 中移除 `Color`：

`crates/tui/src/render.rs:5`，将：
```rust
use ratatui::style::{Color, Modifier, Style};
```
改为：
```rust
use ratatui::style::{Modifier, Style};
```

- [ ] **Step 4: 编译检查**

Run: `cargo build -p tui 2>&1 | head -20`
Expected: 编译成功（无 warning）

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/render.rs
git commit -m "fix(tui): remove tab bar hint text, shrink mini-log to 2 rows"
```

---

### Task 7: 最终验证

- [ ] **Step 1: 完整编译**

Run: `cargo build -p tui 2>&1`
Expected: 编译成功，无 warning

- [ ] **Step 2: 运行所有 tui 测试**

Run: `cargo test -p tui 2>&1`
Expected: 全部 PASS

- [ ] **Step 3: 运行 clippy**

Run: `cargo clippy -p tui -- -D warnings 2>&1 | head -30`
Expected: 无错误

- [ ] **Step 4: 查看完整 diff 确认变更范围**

Run: `git diff main -- crates/tui/`
Expected: 仅在预期文件中有变更，无意外修改

- [ ] **Step 5: 如需要，最终 commit**

```bash
# 如有遗漏的修改
git add crates/tui/
git commit -m "chore(tui): final cleanup after declutter verification"
```
