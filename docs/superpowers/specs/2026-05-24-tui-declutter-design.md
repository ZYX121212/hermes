# TUI 界面精简设计

## 目标

解决当前 TUI 界面的显示杂乱问题：颜色过多、信息密度过高、布局冗余。

## 原则

- **做减法**：保留核心功能和信息可及性，移除视觉噪音
- **不牺牲功能**：所有快捷键保留，只是不在 footer 全部显示
- **渐进式暴露**：常用操作直接可见，完整操作列表在 Help 面板

---

## 1. 布局结构

### Header (1行)

去掉多色背景块，统一暗底前景色区分：

```
 Hermes · demo    第 3 轮   执行中
 青字               灰字      黄字(动态)
```

变更：
- 所有 `fg(BG) bg(color)` 反色块 → `fg(color) bg(BG)` 前景色模式
- spinner 保留（唯一动画元素）

### Tab Bar (1行, 仅 Planning/Executing 阶段)

```
[ Plan ] [ Exec ]
```

变更：
- 移除 `"Tab 切换标签  Shift+Tab 切换焦点"` 永久提示文字（移至 Help 面板）
- 仅在 `phase == Planning | Executing` 时渲染，其他阶段消失

### Main Area 水平分割

保持现有一左一右比例不变。

### Mini-Log (仅 Planning/Executing 阶段)

从 3 行减为 2 行，只显示最新 2 条日志。

变化：
- `Constraint::Length(3)` → `Constraint::Length(2)`
- 跳过 summary 单独展示

### Footer (1行)

根据焦点只显示 2-3 个最核心操作：

| 焦点 | 显示 |
|------|------|
| MainLeft (Plan tab) | `[Tab] Exec  [↑↓] 滚动  [q] 退出  [h] 帮助` |
| MainLeft (Exec tab) | `[Tab] Plan  [↑↓] 选择  [Enter] 详情  [q] 退出  [h] 帮助` |
| MainLeft (Log/Results) | `[Tab] 切换  [↑↓] 滚动  [q] 退出  [h] 帮助` |
| Evolution | `[↑↓] 滚动  [Enter] 折叠  [q] 退出  [h] 帮助` |
| MiniLog | `[↑↓] 滚动  [q] 退出  [h] 帮助` |

移除 footer 中的：`Shift+Tab`、`PgUp/PgDn`、`Home/End`、`jk` 提示

---

## 2. 颜色体系

### 语义色（不变）

| 用途 | 颜色 | 值 |
|------|------|-----|
| 成功/正面 | GREEN | #34d399 |
| 警告/进行中 | YELLOW | #fbbf24 |
| 错误/失败 | RED | #f87171 |
| 强调/交互 | CYAN | #22d3ee |
| 信息 | BLUE | #60a5fa |

### 面板标记

- 不再使用 MAGENTA 作为面板标识色
- Log 面板 border 改用默认 BORDER 色
- 每个面板 title 前的 `▌` 保留 accent 色区分

### 面板 Border

- 默认：`BORDER` (#334155)
- 焦点：`BORDER_FOCUSED` (#38bdf8)
- 不再按面板类型设置不同 border 颜色

---

## 3. 动画精简

只保留 **Header spinner** 作为唯一动画：

- Plan streaming cursor `▌` → 静态显示（不再闪烁）
- Exec Running 状态 icon → 静态 `◉` + `Modifier::BOLD`（不再 `◉`/`◎` 交替）

代码变更：
- `plan.rs:30-34`: `cursor` 始终为 `"▌"`
- `execution.rs:54-59`: `blink` 始终为 `"◉"` 不带 frame_count 判断

---

## 4. 面板内容精简

### Execution 面板

- 内容预览截断从 50 字符 → 30 字符
- 保持其他不变

### Log / Results 面板

- 保持现有渲染逻辑不变
- Results 面板不再使用 MAGENTA，改用默认 border

### Evolution 面板

- 初始默认折叠所有 section（`evo_stats_hidden`、`evo_weights_hidden`、`evo_meta_hidden` 初始值改为 `true`），用户可按 `Enter` 展开各项
- 不随 phase 变化自动折叠/展开

### Scrollbar

- 轨道字符从 `░` → `│`（细线）
- 滑块 `█` 不变

---

## 5. 影响文件

| 文件 | 变更 |
|------|------|
| `crates/tui/src/panels/header.rs` | 反色块 → 前景色模式 |
| `crates/tui/src/panels/footer.rs` | 精简提示文字，按焦点分支 |
| `crates/tui/src/panels/plan.rs` | 取消 streaming cursor 闪烁 |
| `crates/tui/src/panels/execution.rs` | 取消 Running icon 闪烁，缩短预览 |
| `crates/tui/src/panels/log.rs` | mini-log 2行，移除 summary |
| `crates/tui/src/panels/evolution.rs` | scrollbar 字符 `░`→`│` |
| `crates/tui/src/panels/results.rs` | 同上 |
| `crates/tui/src/render.rs` | tab bar 条件渲染，移除提示文字，mini-log 高度 |
| `crates/tui/src/state.rs` | scrollbar 轨道字符、默认折叠状态 |
| `crates/tui/src/theme.rs` | 移除 MAGENTA 使用 |

---

## 6. 测试

- 现有测试应继续通过（颜色值变更不破坏测试逻辑）
- 新增测试：scrollbar 轨道字符变更后的渲染验证
- 手动验证：启动 TUI 后各阶段视觉效果确认无闪烁冲突
