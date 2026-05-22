# TUI Optimization Design Spec

**Date**: 2026-05-22
**Status**: Approved
**Based on**: 2026-05-21-hermes-agent-design.md

## Overview

优化 Hermes Agent TUI 界面的四个维度：信息架构 (A)、交互体验 (B)、视觉呈现 (C)、信息密度 (D)。采用阶段自适应布局 + 焦点系统的方案。

## 1. Information Architecture & Adaptive Layout

### Panel Responsibility Redesign

| Panel | Current Issue | New Design |
|---|---|---|
| Header | OK as-is | Keep, minor style tweaks |
| Plan | Mixes streaming + step list, duplicates Exec | **LLM raw streaming output only**, no step list |
| Execution | Simple flat step list | **Dedicated execution view**: step tree with indentation + progress bar |
| Evolution | Three sections stacked vertically | Collapsible sections; shrinks during Planning, expands during Evolving |
| Summary/Log | 3 lines, too little info | **Scrollable Log panel** with full history (max 200 entries) |
| Footer | None | **New**: 1-line keybinding hint bar |

### Phase-Adaptive Layout

Layout ratios change based on `AgentPhase`:

**Planning phase:**
- Plan panel: 80% width (left), Evolution: 20% (right)
- Plan shows streaming LLM tokens with blinking cursor

**Executing phase:**
- Execution panel: 75% width (left), Evolution: 25% (right)
- Execution shows step tree + overall progress bar

**Idle / Observing / Reflecting / Evolving:**
- Log panel: 60% width (left), Evolution: 40% (right)
- Shows full scrollable log history; Evolution expands during Evolving phase

No animation transitions — direct redraw on phase change.

## 2. Interaction Design

### Focus System

| Key | Action |
|---|---|
| `Tab` | Rotate focus clockwise: main-left → Evolution → Log → main-left... |
| `Shift+Tab` | Rotate focus counter-clockwise |
| Focused panel | Bright Cyan border; unfocused panels get DarkGray border |

### Scrolling

| Input | Behavior |
|---|---|
| `↑` / `↓` or `j` / `k` | Scroll focused panel by 1 line |
| `PageUp` / `PageDown` | Scroll focused panel by page |
| `Home` / `End` | Jump to top / bottom of focused panel |
| Mouse wheel | Scroll the panel under cursor (does not change focus) |

### Global Shortcuts

| Key | Action |
|---|---|
| `q` / `Esc` / `Ctrl+C` | Quit |
| `h` / `F1` | Toggle help overlay (lists all keybindings) |

### Input Mode (when agent awaits user input)

Keep existing: buffer captures chars, `Enter` submits, `Backspace` deletes. Add:
- `↑` / `↓`: scroll input history

### Footer

Single line, adapts to focus:
```
[Tab]切换焦点  [↑↓]滚动  [PgUp/PgDn]翻页  [q]退出  [h]帮助
```

## 3. Visual Design

### Color Palette

| Token | Color | Usage |
|---|---|---|
| Primary | Cyan | Panel titles, focused borders |
| Success | Green | Successful steps, positive scores |
| Warning | Yellow | Running steps, executing phase |
| Danger | Red | Failed steps, error logs |
| Info | LightBlue | Agent name, informational |
| Muted | DarkGray | Unfocused borders, placeholder text |
| Neutral | Gray | Secondary text, durations |
| Text | White | Body text |

Background: terminal default (dark), no custom background colors.

### Panel Borders

- Focused: Primary (Cyan), single-line bright
- Unfocused: Muted (DarkGray), single-line
- Title format: `[ Name ]` with semantic color

### Scroll Indicators

- Visible on focused panels only
- Right-side character-based scrollbar using `░` (track) + `█` (thumb)

### Step Execution Display

| Status | Icon | Color | Extra |
|---|---|---|---|
| Pending | `○` | Muted | — |
| Running | `◉` | Warning | Blink (alt-frame ◉/◎) |
| Success | `✓` | Success | + duration |
| Failed | `✗` | Danger | + red error summary |

### Progress Bar

Bottom of Execution panel during Executing phase:
```
████████████░░░░░░░░  6/10  60%
```
Fill: Primary, Track: Muted.

### Step Indentation

Indent by `layer` to reflect DAG dependencies:
```
Step 1: web_search        ✓  0.3s
  Step 2: parse_results   ◉  running...
Step 3: generate_report   ○  pending
```

### Log Panel

- Error lines: Danger color
- Normal lines: Neutral color
- Entries beyond most recent 3 get dimmed style
- Scrollable full history (max 200)

## 4. Implementation Scope

### State Changes (`state.rs`)
- Add `focused_panel: FocusedPanel` enum
- Add `exec_total_steps: usize`, `exec_completed_steps: usize` for progress bar
- Add `log_scroll: u16` for log panel scrolling
- Add `input_history: VecDeque<String>` for input history
- Add `help_visible: bool` for help overlay
- Add per-panel scroll offsets

### Render Changes (`render.rs`)
- Phase-aware layout ratios
- Widget-level scrollbar rendering helper
- Focus-aware border color selection

### Panel Changes
- `header.rs`: minor style updates
- `plan.rs`: remove step list, pure streaming display, scrollbar
- `execution.rs`: step tree with indentation, progress bar, scrollbar
- `evolution.rs`: collapsible sections, list all weights with scroll
- `summary.rs` → `log.rs`: full scrollable log panel
- `footer.rs` (new): keybinding hint bar
- `help.rs` (new): help overlay popup

### Run Changes (`run.rs`)
- Focus navigation (Tab/Shift+Tab)
- Per-focus scroll keys (arrows, j/k, Home/End)
- Mouse wheel event handling
- Help toggle (h/F1)
- Input history (up/down in input mode)
- Ctrl+C quit

## 5. Testing

- Compile check: `cargo check -p tui`
- Visual manual testing: run agent, verify each phase layout, scroll, focus switching
- Verify no regression in agent loop events
