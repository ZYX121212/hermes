# Hermes TUI 深度优化设计

日期：2026-05-23

## 目标

提升 TUI 信息密度和可用性，解决四个核心问题：
1. Plan 内容在规划阶段结束后消失，无法回顾
2. Agent 完成后界面大部分空白，缺乏结构化结果
3. 步骤输出被截断（50-80 字符），无法查看完整结果
4. Evolution 面板在无数据时占用固定比例浪费屏幕

## 不改动的部分

- Agent 核心循环逻辑不变
- 事件系统不变
- 键盘快捷键体系不变（新增不超过 3 个键位）
- 不新增外部依赖

---

## 功能一：Plan Tab 切换

### 概述
Planning/Executing 阶段左侧面板支持 Tab 切换 `[Plan]` 和 `[Exec]`，用户可随时回顾规划内容。

### 状态变更

`state.rs` 新增：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Plan,
    Execution,
}

impl LeftTab {
    pub fn next(self) -> Self { /* Plan -> Execution, Execution -> Plan */ }
}
```

`TuiAppState` 新增字段：
- `pub left_tab: LeftTab` — 当前选中的左侧 Tab

### 渲染变更

`render.rs`: Planning/Executing 阶段在左侧面板顶部渲染 Tab 栏（单行高度），Tab 栏下方根据 `left_tab` 值渲染对应内容：
- `LeftTab::Plan` → `panels::plan::render_plan()`
- `LeftTab::Execution` → `panels::execution::render_execution()`

`plan.rs`: 支持在 Executing 阶段也能渲染（当前仅在 Planning 阶段调用）。Plan 内容从 `streaming_buffer` 读取。计划阶段结束后 buffer 不清空，保留直到下一轮 `TurnStarted`。

### 键盘交互

`run.rs`: 当 `phase` 为 Planning 或 Executing 且 `focused_panel == MainLeft` 时，Tab 键切换 `left_tab`（而非切换 FocusedPanel）。Shift+Tab 仍然切换 FocusedPanel（全局面板切换）。

### 涉及文件
- `crates/tui/src/state.rs`
- `crates/tui/src/render.rs`
- `crates/tui/src/panels/plan.rs`
- `crates/tui/src/run.rs`

---

## 功能二：结构化执行报告

### 概述
Agent 完成后（`agent_done && phase == Idle`），左侧渲染结构化 Results 面板替代纯 Log。

### 新增文件

`panels/results.rs` — 渲染结构化报告，包含：
1. **Summary 横幅** — `state.summary` 黄色高亮
2. **关键指标行** — 总耗时（从 executions 计算）、步骤成功率、反思评分
3. **步骤列表** — 从 `state.executions` 读取，每行显示图标/工具名/耗时/preview
4. **反思内容** — 从 log_entries 中提取最近一条反思消息

### 布局

Results 面板内部使用垂直 Layout：
- Summary 行（1-2 行）
- 分隔线
- 指标行（1 行）
- 步骤列表（可滚动）
- 分隔线
- 反思内容（1-3 行）
- 底部提示 "按 Tab 切换至完整 Log"

### 渲染条件

`render.rs`: Idle 阶段 `agent_done == true` 时左侧渲染 Results；`agent_done == false` 时渲染 Log（保持现有行为）。

### 涉及文件
- 新增 `crates/tui/src/panels/results.rs`
- `crates/tui/src/panels/mod.rs` — 注册模块
- `crates/tui/src/render.rs`
- `crates/tui/src/state.rs` — 新增 `total_duration_ms: Option<u64>` 字段

---

## 功能三：Overlay 弹窗查看完整输出

### 概述
Execution 面板中选中步骤按 Enter 弹出全屏 overlay，展示完整工具输出。

### 状态变更

`state.rs` 新增：

```rust
pub struct StepOutputOverlay {
    pub step_id: Uuid,
    pub tool: String,
    pub status: StepStatus,
    pub duration_ms: Option<u64>,
    pub full_content: String,
    pub scroll: u16,
}

// TuiAppState 新增字段:
pub exec_selected_index: Option<usize>,  // Execution 列表中高亮的步骤索引
pub output_overlay: Option<StepOutputOverlay>,  // 打开时非 None
```

### 数据来源

需要存储完整输出内容。`StepExecState` 当前只有 `content_preview`（截断到 100 字符）。新增字段 `content_full: Option<String>` 存储完整输出。

`run.rs` 中 `StepCompleted` 处理：同时设置 `content_preview`（截断 100）和 `content_full`（strip_ansi，上限 10KB 防内存膨胀）。

### Overlay 渲染

`panels/overlay.rs` — 全屏半透明 overlay：
- 外层 block 边框（亮色），标题显示 tool name + step_id 短格式
- 内层：tool/duration/status 信息行 + 分隔线 + 可滚动内容区域
- 支持 PageUp/PageDown/Home/End/j/k 滚动

### 键盘交互

`run.rs`:
- Executing + MainLeft 聚焦时，Up/Down/j/k 移动 `exec_selected_index`（selected_index 变化时自动调整 exec_scroll 确保选中行可见）
- 鼠标滚轮在 Execution 面板仍可自由滚动，不依赖 selected_index
- Enter → 构建 `StepOutputOverlay` 并设置 `state.output_overlay = Some(...)`
- Overlay 打开时：Esc/Enter/q 关闭；Up/Down/j/k/PageUp/PageDown 滚动 overlay 内容；其他键忽略

### 涉及文件
- 新增 `crates/tui/src/panels/overlay.rs`
- `crates/tui/src/panels/mod.rs`
- `crates/tui/src/state.rs`
- `crates/tui/src/run.rs` — 键盘处理重构
- `crates/tui/src/render.rs` — overlay 渲染在最顶层
- `crates/tui/src/panels/execution.rs` — 高亮选中行

---

## 功能四：Evolution 面板动态宽度

### 概述
根据 `all_weights().is_empty()` 动态调整主面板与 Evolution 面板的水平分割比例。

### 比例表

| 阶段 | 无策略数据 (left/right) | 有策略数据 (left/right) |
|------|------------------------|------------------------|
| Planning | 85/15 | 75/25 |
| Executing | 80/20 | 70/30 |
| Idle / 其他 | 75/25 | 60/40 |

### 实现

`render.rs` 中 `render_app` 的水平分割计算，在现有 `match phase` 后加一层 `if state.evolution.all_weights().is_empty()` 判断，直接修改比例常量。

无需新增 state 字段，纯渲染逻辑变更。

### 键盘交互

无变更。

### 涉及文件
- `crates/tui/src/render.rs`

---

## 键盘快捷键汇总

| 键 | 上下文 | 行为 |
|----|--------|------|
| Tab | Planning/Executing + MainLeft 聚焦 | 切换左侧面板 Plan/Exec Tab |
| Shift+Tab | 全局 | 切换 FocusedPanel |
| Up/Down/j/k | Execution + MainLeft | 移动步骤选中高亮 |
| Enter | Execution + MainLeft + 有选中步骤 | 打开输出 overlay |
| Esc/Enter/q | Overlay 打开 | 关闭 overlay |
| Up/Down/j/k/PageUp/PageDown | Overlay 打开 | 滚动 overlay 内容 |
| Tab | Idle + agent_done | 切换 Results / Log 面板 |

---

## 自检清单

- [ ] Plan buffer 在 TurnStarted 时清空，非 Planning→Executing 阶段切换时
- [ ] Executing 阶段的步骤选中不影响其他面板的 Up/Down 滚动行为
- [ ] Overlay 关闭后恢复原有焦点和滚动位置
- [ ] Evolution 动态比例切换不导致闪烁或布局抖动
- [ ] Results 面板在 agent_done=false 时不出现
- [ ] 所有新增状态字段在 TuiAppState::new() 中有默认值
- [ ] 不引入新的 panic 点
- [ ] 不增删外部 crate 依赖
