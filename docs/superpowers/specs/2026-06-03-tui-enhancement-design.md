# TUI Enhancement Design — Hermess Terminal Interface

Date: 2026-06-03 | Status: Approved

## Scope

Enhance the existing ratatui-based TUI (`crates/tui/`) to fill all feature gaps identified against the spec, covering 5 phases.

## Architecture (unchanged)

```
run.rs (event loop + keyboard/mouse dispatch)
  ├─ handle_event() ── AgentEvent → state mutation
  ├─ keyboard dispatch ── input/command/navigation modes
  └─ per-frame sync TuiInput → TuiAppState

render.rs (layout dispatch)
  └─ delegates to panels/* modules
```

Additions:
- `panels/kanban.rs` — task kanban board
- `panels/context_ref.rs` — @-mention reference picker
- `rich_text/` directory (split from `rich_text.rs`) — `mod.rs`, `highlight.rs`, `table.rs`, `latex.rs`
- `keybindings.rs` — configurable keybinding loader

## P1 — Core Interaction

### Multiline Input
- `Enter` → submit; `Shift+Enter` → insert `\n`
- `input.rs` renders 1–8 lines dynamically
- `TuiAppState.input_line_count: u8`

### Markdown Enhancement
- Code highlighting: `syntect` crate, fenced blocks with language tag → ratatui colored `Span`s
- Table: parse `|col|col|` syntax → ratatui `Table` widget
- Split `rich_text.rs` → `rich_text/{mod, highlight, table}.rs`

### @-mention References
- Typing `@` triggers floating panel above input (5 lines)
- Sources: `@file:<path>`, `@git:diff`, `@mem:<query>`
- `context_ref.rs` panel, keyboard interception via crossterm

### LaTeX → Unicode
- Lookup table for ~100 common math symbols: `\alpha→α`, `\int→∫`, etc.
- Detect `$...$` / `$$...$$` boundaries
- `rich_text/latex.rs`

## P2 — Execution Visualization

### Kanban Board
- `panels/kanban.rs` — 3-column layout: Pending / In Progress / Completed
- Data: new `AgentEvent::TaskUpdated { id, status, title }`
- `TuiAppState.kanban_items: Vec<KanbanItem>`
- Toggle via `/kanban` or auto-show in mini-log area during Planning phase

### Thinking Animation
Sub-phase specific animations via new `AgentEvent::ThinkingPhase`:
| Sub-phase | Animation |
|-----------|-----------|
| LLM call | braille spinner `⠋⠙⠹...` |
| Parse response | pulse dots |
| Tool exec | blink `▶` |
| Wait input | pulse `●` |

`header.rs` selects animation per sub-phase.

### Execution Step Enhancements
- Fold/expand tool output with `Enter`
- Truncation hint: "Press Enter to view full output (1.2KB)"
- Granular duration: `<1ms` / `12ms` / `1.2s` / `2m3s`

## P3 — Slash Commands

### TUI-only (5 commands)
| Command | Implementation |
|---------|---------------|
| `/new` | Reset TuiAppState, emit `AgentEvent::Reset` |
| `/load <name>` | Deserialize session from `~/.hermess/sessions/<name>.json` |
| `/memory <q>` | Call `WorkingMemory::search()` |
| `/recall <q>` | Alias for `/memory` |
| `/cron` | Query scheduler crate for registered tasks |

### Backend-dependent (7 commands)
| Command | Implementation |
|---------|---------------|
| `/personality` | Emit `AgentEvent::SetPersonality` |
| `/compress` | Emit `AgentEvent::CompressContext` |
| `/checkpoint` | Emit `AgentEvent::SaveCheckpoint` |
| `/rollback` | Emit `AgentEvent::RollbackCheckpoint` |
| `/diff` | Compare current vs last checkpoint in popup |
| `/kanban` | Toggle `state.kanban_visible` |
| `/usage` | Rich table from UsageTracker breakdown |

All results rendered via `SlashResult` popup.

## P4 — Personalization

### Custom Themes
- `Theme` struct loaded from `~/.hermess/theme.toml` with defaults
- Built-in presets: `tokyo-night` (default), `dracula`, `solarized-dark`, `gruvbox`
- Settings panel gains "Theme" tab with live preview

### Keybindings
- `keybindings.rs`: `KeyBindings` struct, default bindings
- Load overrides from `~/.hermess/keybindings.toml`
- ~30 bindable actions (quit, submit, newline, toggle_help, focus_next, etc.)
- `run.rs` dispatch switches to table lookup: `bindings.action_for(event)`

## P5 — Dashboard

### Multi-Session Tabs
- `TuiAppState.sessions: Vec<SessionTab>`, each with independent state
- `Ctrl+T` new tab, `Ctrl+W` close, `Ctrl+←/→` switch
- Tab bar renders between header and main area (1 line)
- Max 9 tabs

### /usage Detailed View
- Table with columns: model, prompt_tokens, completion_tokens, cost
- Fetched from `UsageTracker`

## File Impact Summary

| File | Action |
|------|--------|
| `state.rs` | Add fields: kanban_items, input_line_count, thinking_subphase, sessions, theme |
| `run.rs` | New slash handlers, keybinding table lookup, multiline keyboard handling |
| `render.rs` | Add kanban panel area, tab bar, @-mention zone |
| `input.rs` | Multiline rendering and cursor |
| `rich_text.rs` → `rich_text/*` | Split + highlight.rs, table.rs, latex.rs |
| `theme.rs` | Theme struct, TOML loader, presets |
| `header.rs` | Sub-phase animations |
| `execution.rs` | Fold/expand, granular duration |
| `panels/kanban.rs` | New — kanban board panel |
| `panels/context_ref.rs` | New — @-mention reference picker |
| `panels/settings.rs` | Add Theme tab |
| `keybindings.rs` | New — keybinding loader + lookup |
| `Cargo.toml` | Add `syntect` dependency |
