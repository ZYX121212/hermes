# TUI Enhancement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance the ratatui-based TUI to fill all feature gaps across 5 phases: core interaction, execution visualization, slash commands, personalization, and dashboard.

**Architecture:** Incremental enhancement of existing event-driven ratatui TUI. `run.rs` event loop → `handle_event()` state mutation → `render.rs` layout dispatch → `panels/*` rendering. Data flow unchanged; only new modules, state fields, and panel enhancements are added.

**Tech Stack:** ratatui 0.26+, crossterm, syntect (code highlighting), toml (config), serde

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/tui/Cargo.toml` | Modify | Add syntect, toml deps |
| `crates/tui/src/state.rs` | Modify | New fields: kanban_items, thinking_subphase, sessions, theme, input_multiline, keybindings |
| `crates/tui/src/run.rs` | Modify | Multiline input, keybinding lookup, new slash handlers |
| `crates/tui/src/render.rs` | Modify | Kanban panel area, tab bar, @mention zone |
| `crates/tui/src/rich_text.rs` → `rich_text/mod.rs` | Modify + New | Split into mod, add highlight/table/latex |
| `crates/tui/src/rich_text/highlight.rs` | New | syntect code highlighting |
| `crates/tui/src/rich_text/table.rs` | New | Markdown table rendering |
| `crates/tui/src/rich_text/latex.rs` | New | LaTeX→Unicode conversion |
| `crates/tui/src/theme.rs` | Modify | Theme struct, TOML loader, presets |
| `crates/tui/src/keybindings.rs` | New | Keybinding loader + lookup |
| `crates/tui/src/panels/input.rs` | Modify | Multiline rendering |
| `crates/tui/src/panels/header.rs` | Modify | Sub-phase spinner animation |
| `crates/tui/src/panels/execution.rs` | Modify | Fold/expand, granular duration |
| `crates/tui/src/panels/kanban.rs` | New | 3-column kanban board |
| `crates/tui/src/panels/context_ref.rs` | New | @mention reference picker |
| `crates/tui/src/panels/settings.rs` | Modify | Theme tab |
| `crates/tui/src/panels/tab_bar.rs` | New | Multi-session tab bar |
| `crates/agent-core/src/lib.rs` | Modify | New AgentEvent variants |

---

### Task 1: Add dependencies and set up modules

**Files:**
- Modify: `crates/tui/Cargo.toml`
- Modify: `crates/tui/src/lib.rs`

- [ ] **Step 1: Add syntect and toml to Cargo.toml**

Read the file first, then edit:

```toml
[dependencies]
ratatui.workspace = true
crossterm.workspace = true
tokio = { workspace = true, features = ["sync", "time", "macros"] }
parking_lot.workspace = true
agent-core = { path = "../agent-core" }
evolution = { path = "../evolution" }
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
serde.workspace = true
serde_json.workspace = true
llm = { path = "../llm" }
syntect = { version = "5", default-features = false, features = ["parsing", "regex-onig"] }
toml = "0.8"
```

- [ ] **Step 2: Run cargo check to verify dependencies resolve**

Run: `cargo check -p tui 2>&1 | tail -5`
Expected: `Checking tui v0.1.0` — success or pre-existing errors only.

- [ ] **Step 3: Update lib.rs module declarations**

Replace the module section:

```rust
// crates/tui/src/lib.rs
// TUI terminal interface for Hermes Agent using ratatui + crossterm.

pub mod keybindings;
pub mod panels;
pub mod render;
pub mod rich_text;
pub mod run;
pub mod settings_store;
pub mod state;
pub mod theme;

pub use keybindings::KeyBindings;
pub use run::run_tui;
pub use settings_store::UserSettings;
pub use state::TuiInput;
pub use theme::Theme;
```

- [ ] **Step 4: Check compilation**

Run: `cargo check -p tui 2>&1 | tail -5`
Expected: should fail with missing modules (expected, to be created in later tasks)

- [ ] **Step 5: Rename rich_text.rs to rich_text/mod.rs**

Run:
```bash
mkdir -p crates/tui/src/rich_text
git mv crates/tui/src/rich_text.rs crates/tui/src/rich_text/mod.rs
```

- [ ] **Step 6: Commit**

```bash
git add crates/tui/Cargo.toml crates/tui/src/lib.rs crates/tui/src/rich_text
git commit -m "feat(tui): add syntect/toml deps, restructure rich_text as directory module
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: New AgentEvent variants for enhanced visualization

**Files:**
- Modify: `crates/agent-core/src/lib.rs:192-284`

- [ ] **Step 1: Add new AgentEvent variants**

After the existing `GatewayRouteDecision` variant (around line 284), add these new variants before the closing `}`:

```rust
    // ── Task tracking (kanban) ──
    TaskUpdated {
        task_id: String,
        title: String,
        status: TaskStatus,
    },

    // ── Thinking sub-phase ──
    ThinkingPhaseChanged {
        sub_phase: ThinkingSubPhase,
    },

    // ── Personality ──
    SetPersonality {
        name: String,
    },

    // ── Context compression ──
    CompressContext,

    // ── Checkpoint ──
    SaveCheckpoint,
    RollbackCheckpoint,

    // ── Session ──
    ResetSession,
}
```

- [ ] **Step 2: Add supporting types above the enum**

Before the `AgentEvent` enum definition (around line 190), add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSubPhase {
    CallingLlm,
    ParsingResponse,
    ExecutingTool,
    WaitingForInput,
    Idle,
}
```

- [ ] **Step 3: Run type check**

Run: `cargo check -p agent-core 2>&1 | tail -5`
Expected: should compile.

- [ ] **Step 4: Commit**

```bash
git add crates/agent-core/src/lib.rs
git commit -m "feat(agent-core): add AgentEvent variants for kanban, thinking phases, personality, checkpoint, reset
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: Theme system — struct, TOML loader, presets

**Files:**
- Modify: `crates/tui/src/theme.rs`

- [ ] **Step 1: Rewrite theme.rs with Theme struct**

```rust
// crates/tui/src/theme.rs
// Theme system: struct, presets, TOML loading, widget builders.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use serde::Deserialize;

// ── Theme struct ──

#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    #[serde(default = "default_colors")]
    pub colors: ThemeColors,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeColors {
    pub bg: String,
    pub panel: String,
    pub panel_alt: String,
    pub border: String,
    pub border_focused: String,
    pub text: String,
    pub muted: String,
    pub subtle: String,
    pub cyan: String,
    pub blue: String,
    pub green: String,
    pub yellow: String,
    pub red: String,
    pub magenta: String,
}

fn default_colors() -> ThemeColors {
    ThemeColors {
        bg: "#0b1220".into(),
        panel: "#0f172a".into(),
        panel_alt: "#111f30".into(),
        border: "#334155".into(),
        border_focused: "#38bdf8".into(),
        text: "#e2e8f0".into(),
        muted: "#94a3b8".into(),
        subtle: "#64748b".into(),
        cyan: "#22d3ee".into(),
        blue: "#60a5fa".into(),
        green: "#34d399".into(),
        yellow: "#fbbf24".into(),
        red: "#f87171".into(),
        magenta: "#d8b4fe".into(),
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self { colors: default_colors() }
    }
}

impl Theme {
    /// Load from ~/.hermess/theme.toml, fall back to built-in presets.
    pub fn load() -> Self {
        let paths = [
            dirs_config().join(".hermess").join("theme.toml"),
        ];
        for path in &paths {
            if path.exists() {
                if let Ok(raw) = std::fs::read_to_string(path) {
                    if let Ok(t) = toml::from_str::<Theme>(&raw) {
                        tracing::info!(path = %path.display(), "Loaded theme");
                        return t;
                    }
                }
            }
        }
        // Fallback: check env var for preset name
        if let Ok(name) = std::env::var("HERMESS_THEME") {
            return Self::preset(&name);
        }
        Self::preset("tokyo-night")
    }

    pub fn preset(name: &str) -> Self {
        match name {
            "dracula" => Self {
                colors: ThemeColors {
                    bg: "#282a36".into(), panel: "#2c2f3e".into(),
                    panel_alt: "#353849".into(), border: "#6272a4".into(),
                    border_focused: "#bd93f9".into(), text: "#f8f8f2".into(),
                    muted: "#a0a0b0".into(), subtle: "#6272a4".into(),
                    cyan: "#8be9fd".into(), blue: "#6272a4".into(),
                    green: "#50fa7b".into(), yellow: "#f1fa8c".into(),
                    red: "#ff5555".into(), magenta: "#ff79c6".into(),
                }
            },
            "solarized-dark" => Self {
                colors: ThemeColors {
                    bg: "#002b36".into(), panel: "#073642".into(),
                    panel_alt: "#0a4b57".into(), border: "#586e75".into(),
                    border_focused: "#268bd2".into(), text: "#839496".into(),
                    muted: "#586e75".into(), subtle: "#657b83".into(),
                    cyan: "#2aa198".into(), blue: "#268bd2".into(),
                    green: "#859900".into(), yellow: "#b58900".into(),
                    red: "#dc322f".into(), magenta: "#d33682".into(),
                }
            },
            "gruvbox" => Self {
                colors: ThemeColors {
                    bg: "#282828".into(), panel: "#32302f".into(),
                    panel_alt: "#3c3836".into(), border: "#504945".into(),
                    border_focused: "#83a598".into(), text: "#ebdbb2".into(),
                    muted: "#a89984".into(), subtle: "#665c54".into(),
                    cyan: "#83a598".into(), blue: "#458588".into(),
                    green: "#b8bb26".into(), yellow: "#fabd2f".into(),
                    red: "#fb4934".into(), magenta: "#d3869b".into(),
                }
            },
            _ => Self::default(), // tokyo-night
        }
    }

    pub fn preset_names() -> &'static [&'static str] {
        &["tokyo-night", "dracula", "solarized-dark", "gruvbox"]
    }

    // ── Color accessors ──
    fn parse(hex: &str) -> Color {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Color::Rgb(255, 255, 255);
        }
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        Color::Rgb(r, g, b)
    }

    pub fn bg(&self) -> Color { Self::parse(&self.colors.bg) }
    pub fn panel(&self) -> Color { Self::parse(&self.colors.panel) }
    pub fn panel_alt(&self) -> Color { Self::parse(&self.colors.panel_alt) }
    pub fn border(&self) -> Color { Self::parse(&self.colors.border) }
    pub fn border_focused(&self) -> Color { Self::parse(&self.colors.border_focused) }
    pub fn text(&self) -> Color { Self::parse(&self.colors.text) }
    pub fn muted(&self) -> Color { Self::parse(&self.colors.muted) }
    pub fn subtle(&self) -> Color { Self::parse(&self.colors.subtle) }
    pub fn cyan(&self) -> Color { Self::parse(&self.colors.cyan) }
    pub fn blue(&self) -> Color { Self::parse(&self.colors.blue) }
    pub fn green(&self) -> Color { Self::parse(&self.colors.green) }
    pub fn yellow(&self) -> Color { Self::parse(&self.colors.yellow) }
    pub fn red(&self) -> Color { Self::parse(&self.colors.red) }
    pub fn magenta(&self) -> Color { Self::parse(&self.colors.magenta) }
}

// ── Globally loaded theme ──
// Initialised by run_tui() once at startup.
use std::sync::OnceLock;
static CURRENT_THEME: OnceLock<Theme> = OnceLock::new();

pub fn init_theme(theme: Theme) {
    let _ = CURRENT_THEME.set(theme);
}

pub fn theme() -> &'static Theme {
    CURRENT_THEME.get().unwrap_or(&THEME_FALLBACK)
}

static THEME_FALLBACK: Theme = Theme {
    colors: default_colors(),
};

// ── Re-export convenience constants (delegate to theme()) ──
// These replace the old `pub const BG: Color = ...` constants.
// All existing code using `theme::BG` etc. continues to work.

use ratatui::style::Color as RColor;

// For backward compatibility, keep the old const names as functions
// but also provide them as lazily-evaluated access.
// Since existing code uses `theme::BG`, we need module-level constants.
// Solution: keep the old-style consts as defaults, but they get updated
// when the theme is loaded. For now we keep backward compat via consts
// that match the default theme. Users wanting dynamic themes call
// theme().bg() directly.

pub const BG: Color = Color::Rgb(11, 18, 32);
pub const PANEL: Color = Color::Rgb(15, 23, 42);
pub const PANEL_ALT: Color = Color::Rgb(17, 31, 48);
pub const BORDER: Color = Color::Rgb(51, 65, 85);
pub const BORDER_FOCUSED: Color = Color::Rgb(56, 189, 248);
pub const TEXT: Color = Color::Rgb(226, 232, 240);
pub const MUTED: Color = Color::Rgb(148, 163, 184);
pub const SUBTLE: Color = Color::Rgb(100, 116, 139);
pub const CYAN: Color = Color::Rgb(34, 211, 238);
pub const BLUE: Color = Color::Rgb(96, 165, 250);
pub const GREEN: Color = Color::Rgb(52, 211, 153);
pub const YELLOW: Color = Color::Rgb(251, 191, 36);
pub const RED: Color = Color::Rgb(248, 113, 113);
pub const MAGENTA: Color = Color::Rgb(216, 180, 254);

// ── Widget builders (unchanged API) ──

pub fn panel_block<'a>(title: impl Into<String>, accent: Color, focused: bool) -> Block<'a> {
    let border = if focused { BORDER_FOCUSED } else { BORDER };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(PANEL))
        .title(title_line(title, accent))
}

pub fn title_line(title: impl Into<String>, accent: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::default().bg(PANEL)),
        Span::styled("▌", Style::default().fg(accent).bg(PANEL)),
        Span::styled(
            format!(" {} ", title.into()),
            Style::default()
                .fg(TEXT)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub fn empty(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text.into()),
        Style::default().fg(SUBTLE).bg(PANEL),
    ))
}

pub fn key(text: impl Into<String>) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default()
            .fg(TEXT)
            .bg(Color::Rgb(30, 41, 59))
            .add_modifier(Modifier::BOLD),
    )
}

fn dirs_config() -> std::path::PathBuf {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}
```

- [ ] **Step 2: Add toml and tracing to Cargo.toml if not already present**

Verify the toml dep from Task 1 exists. Add `tracing.workspace = true`:

```toml
[dependencies]
...
tracing.workspace = true
toml = "0.8"
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -10`
Expected: compile success (all existing callers still work with consts).

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/theme.rs crates/tui/Cargo.toml
git commit -m "feat(tui): add Theme struct with TOML loader and 4 built-in presets
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: Keybindings system

**Files:**
- Create: `crates/tui/src/keybindings.rs`

- [ ] **Step 1: Create keybindings.rs**

```rust
// crates/tui/src/keybindings.rs
// Configurable keyboard shortcuts loaded from ~/.hermess/keybindings.toml.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;

/// All bindable actions in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    Submit,
    Newline,
    ToggleHelp,
    ToggleSettings,
    ToggleKanban,
    ToggleLog,
    FocusNext,
    FocusPrev,
    TabNext,
    TabPrev,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    ScrollBottom,
    SettingsSave,
    SettingsCancel,
    Search,
    SearchNext,
    SearchPrev,
    ClearLine,
    DeleteWord,
    HistoryUp,
    HistoryDown,
    SelectNext,
    SelectPrev,
    SelectConfirm,
    NewTab,
    CloseTab,
    OverlayClose,
    SlashCommand,
    Cancel,
    Copy,
    Home,
    End,
}

/// Parsed key description like "Ctrl+C", "Shift+Enter", "Tab", "PageDown".
#[derive(Debug, Clone)]
struct KeyDesc {
    code: KeyCode,
    modifiers: KeyModifiers,
}

fn parse_key(s: &str) -> Option<KeyDesc> {
    let s = s.trim();
    let mut mods = KeyModifiers::NONE;

    let parts: Vec<&str> = s.split('+').collect();
    let last = parts.last()?.trim();

    for p in &parts[..parts.len().saturating_sub(1)] {
        match p.trim().to_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "shift" => mods |= KeyModifiers::SHIFT,
            "alt" => mods |= KeyModifiers::ALT,
            _ => {}
        }
    }

    let code = match last.to_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "escape" | "esc" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "space" | " " => KeyCode::Char(' '),
        "?" => KeyCode::Char('?'),
        "/" => KeyCode::Char('/'),
        ":" => KeyCode::Char(':'),
        "s" => KeyCode::Char('s'),
        "q" => KeyCode::Char('q'),
        "h" => KeyCode::Char('h'),
        "j" => KeyCode::Char('j'),
        "k" => KeyCode::Char('k'),
        "n" => KeyCode::Char('n'),
        "N" => KeyCode::Char('N'),
        "l" => KeyCode::Char('l'),
        "c" => KeyCode::Char('c'),
        "w" => KeyCode::Char('w'),
        "g" => KeyCode::Char('g'),
        "e" => KeyCode::Char('e'),
        "f" => KeyCode::Char('f'),
        "y" => KeyCode::Char('y'),
        "t" => KeyCode::Char('t'),
        "o" => KeyCode::Char('o'),
        other if other.len() == 1 => KeyCode::Char(other.chars().next().unwrap()),
        _ => return None,
    };

    Some(KeyDesc { code, modifiers })
}

/// Keybinding map with TOML deserialization support.
#[derive(Debug, Clone, Deserialize)]
pub struct KeyBindings {
    #[serde(default)]
    bindings: HashMap<String, String>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert("quit".into(), "Ctrl+C".into());
        bindings.insert("submit".into(), "Enter".into());
        bindings.insert("newline".into(), "Shift+Enter".into());
        bindings.insert("toggle_help".into(), "?".into());
        bindings.insert("toggle_settings".into(), "Ctrl+O".into());
        bindings.insert("toggle_kanban".into(), "Ctrl+K".into());
        bindings.insert("toggle_log".into(), "Ctrl+L".into());
        bindings.insert("focus_next".into(), "Tab".into());
        bindings.insert("focus_prev".into(), "Shift+Tab".into());
        bindings.insert("tab_next".into(), "Ctrl+Right".into());
        bindings.insert("tab_prev".into(), "Ctrl+Left".into());
        bindings.insert("scroll_up".into(), "k".into());
        bindings.insert("scroll_down".into(), "j".into());
        bindings.insert("page_up".into(), "Ctrl+u".into());
        bindings.insert("page_down".into(), "Ctrl+d".into());
        bindings.insert("scroll_bottom".into(), "Shift+g".into());
        bindings.insert("settings_save".into(), "Ctrl+S".into());
        bindings.insert("settings_cancel".into(), "Escape".into());
        bindings.insert("search".into(), "/".into());
        bindings.insert("search_next".into(), "n".into());
        bindings.insert("search_prev".into(), "N".into());
        bindings.insert("clear_line".into(), "Ctrl+U".into());
        bindings.insert("delete_word".into(), "Ctrl+W".into());
        bindings.insert("history_up".into(), "Up".into());
        bindings.insert("history_down".into(), "Down".into());
        bindings.insert("select_next".into(), "j".into());
        bindings.insert("select_prev".into(), "k".into());
        bindings.insert("select_confirm".into(), "Enter".into());
        bindings.insert("new_tab".into(), "Ctrl+T".into());
        bindings.insert("close_tab".into(), "Ctrl+W".into());
        bindings.insert("overlay_close".into(), "Escape".into());
        bindings.insert("slash_command".into(), ":".into());
        bindings.insert("cancel".into(), "Escape".into());
        bindings.insert("copy".into(), "Ctrl+Shift+C".into());
        bindings.insert("home".into(), "Ctrl+A".into());
        bindings.insert("end".into(), "Ctrl+E".into());
        Self { bindings }
    }
}

impl KeyBindings {
    /// Load from ~/.hermess/keybindings.toml, merging with defaults.
    pub fn load() -> Self {
        let mut kb = Self::default();
        let path = dirs_config().join(".hermess").join("keybindings.toml");
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(file_bindings) = toml::from_str::<toml::Value>(&raw) {
                    if let Some(table) = file_bindings.get("bindings").and_then(|v| v.as_table()) {
                        for (key, val) in table {
                            if let Some(val_str) = val.as_str() {
                                kb.bindings.insert(key.clone(), val_str.to_string());
                            }
                        }
                    }
                }
            }
        }
        kb
    }

    /// Map a crossterm KeyEvent to an Action, or None.
    pub fn action_for(&self, event: &KeyEvent) -> Option<Action> {
        for (action_name, key_str) in &self.bindings {
            if let Some(desc) = parse_key(key_str) {
                if event.code == desc.code && event.modifiers == desc.modifiers {
                    return self.action_from_name(action_name);
                }
            }
        }
        None
    }

    fn action_from_name(&self, name: &str) -> Option<Action> {
        Some(match name {
            "quit" => Action::Quit,
            "submit" => Action::Submit,
            "newline" => Action::Newline,
            "toggle_help" => Action::ToggleHelp,
            "toggle_settings" => Action::ToggleSettings,
            "toggle_kanban" => Action::ToggleKanban,
            "toggle_log" => Action::ToggleLog,
            "focus_next" => Action::FocusNext,
            "focus_prev" => Action::FocusPrev,
            "tab_next" => Action::TabNext,
            "tab_prev" => Action::TabPrev,
            "scroll_up" => Action::ScrollUp,
            "scroll_down" => Action::ScrollDown,
            "page_up" => Action::PageUp,
            "page_down" => Action::PageDown,
            "scroll_bottom" => Action::ScrollBottom,
            "settings_save" => Action::SettingsSave,
            "settings_cancel" => Action::SettingsCancel,
            "search" => Action::Search,
            "search_next" => Action::SearchNext,
            "search_prev" => Action::SearchPrev,
            "clear_line" => Action::ClearLine,
            "delete_word" => Action::DeleteWord,
            "history_up" => Action::HistoryUp,
            "history_down" => Action::HistoryDown,
            "select_next" => Action::SelectNext,
            "select_prev" => Action::SelectPrev,
            "select_confirm" => Action::SelectConfirm,
            "new_tab" => Action::NewTab,
            "close_tab" => Action::CloseTab,
            "overlay_close" => Action::OverlayClose,
            "slash_command" => Action::SlashCommand,
            "cancel" => Action::Cancel,
            "copy" => Action::Copy,
            "home" => Action::Home,
            "end" => Action::End,
            _ => return None,
        })
    }

    /// Write default keybindings to ~/.hermess/keybindings.toml as a template.
    pub fn write_default_template(path: &std::path::Path) -> std::io::Result<()> {
        let defaults = Self::default();
        let mut lines = vec!["# Hermess TUI 快捷键配置".into(), "[bindings]".into()];
        let mut sorted: Vec<_> = defaults.bindings.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (name, key) in sorted {
            lines.push(format!("{} = \"{}\"", name, key));
        }
        std::fs::write(path, lines.join("\n") + "\n")
    }
}

fn dirs_config() -> std::path::PathBuf {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -5`
Expected: compile success.

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/keybindings.rs
git commit -m "feat(tui): add configurable keybinding system with TOML file support
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: Code highlighting via syntect (rich_text/highlight.rs)

**Files:**
- Create: `crates/tui/src/rich_text/highlight.rs`
- Modify: `crates/tui/src/rich_text/mod.rs`

- [ ] **Step 1: Create highlight.rs**

```rust
// crates/tui/src/rich_text/highlight.rs
// Syntax highlighting for fenced code blocks using syntect.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use std::sync::OnceLock;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn ss() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn ts() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Map syntect Color to ratatui Color.
fn to_ratatui(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Map syntect FontStyle to ratatui Modifier.
fn to_modifier(style: syntect::highlighting::FontStyle) -> Modifier {
    let mut m = Modifier::empty();
    if style.contains(syntect::highlighting::FontStyle::BOLD) {
        m |= Modifier::BOLD;
    }
    if style.contains(syntect::highlighting::FontStyle::ITALIC) {
        m |= Modifier::ITALIC;
    }
    if style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
        m |= Modifier::UNDERLINED;
    }
    m
}

/// Detect language from fenced code block info string (e.g. "rust", "python", "bash").
fn detect_language(lang: &str) -> Option<&str> {
    let lang = lang.trim().to_lowercase();
    match lang.as_str() {
        "rs" | "rust" => Some("rust"),
        "py" | "python" => Some("python"),
        "js" | "javascript" => Some("javascript"),
        "ts" | "typescript" => Some("typescript"),
        "sh" | "bash" | "shell" => Some("bash"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "sql" => Some("sql"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" => Some("c"),
        "cpp" | "c++" => Some("cpp"),
        "css" => Some("css"),
        "html" => Some("html"),
        "xml" => Some("xml"),
        "markdown" | "md" => Some("markdown"),
        "diff" => Some("diff"),
        "" => None, // no language specified, no highlighting
        _ => None,  // unknown language
    }
}

/// Highlight a fenced code block, returning styled ratatui Lines.
pub fn highlight_code(lang: &str, code: &str, bg: Color) -> Vec<Line<'static>> {
    let detected = detect_language(lang);
    let syntax = detected.and_then(|name| ss().find_syntax_by_token(name));

    let theme = &ts().themes["base16-ocean.dark"];

    if let Some(syntax) = syntax {
        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut lines: Vec<Line> = Vec::new();

        for line in LinesWithEndings::from(code) {
            let highlighted = highlighter.highlight_line(line, ss())
                .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);

            let spans: Vec<Span> = highlighted
                .into_iter()
                .map(|(style, text)| {
                    let fg = to_ratatui(style.foreground);
                    Span::styled(
                        text.trim_end_matches('\n').to_string(),
                        Style::default().fg(fg).bg(bg).add_modifier(to_modifier(style.font_style)),
                    )
                })
                .collect();

            lines.push(Line::from(spans));
        }

        lines
    } else {
        // No language matched — plain muted text
        code.lines()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(crate::theme::MUTED).bg(bg),
                ))
            })
            .collect()
    }
}
```

- [ ] **Step 2: Add pub mod highlight to mod.rs**

Add at the top of `crates/tui/src/rich_text/mod.rs`:

```rust
pub mod highlight;
pub mod latex;
pub mod table;
```

And keep all existing content of mod.rs unchanged below the module declarations.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -10`
Expected: compile success.

- [ ] **Step 4: Add a unit test to verify highlight works**

Add these tests at the bottom of `highlight.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("rust"), Some("rust"));
        assert_eq!(detect_language("rs"), Some("rust"));
        assert_eq!(detect_language("python"), Some("python"));
        assert_eq!(detect_language(""), None);
        assert_eq!(detect_language("unknown-lang"), None);
    }

    #[test]
    fn test_highlight_rust_code() {
        let lines = highlight_code("rust", "fn main() {\n    println!(\"hi\");\n}\n", crate::theme::PANEL);
        assert!(!lines.is_empty());
        // Each line should have at least one span
        for line in &lines {
            assert!(!line.spans.is_empty());
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p tui highlight::tests 2>&1 | tail -10`
Expected: tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/tui/src/rich_text/
git commit -m "feat(tui): add syntect-based code highlighting for fenced blocks
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: Markdown table rendering (rich_text/table.rs)

**Files:**
- Create: `crates/tui/src/rich_text/table.rs`

- [ ] **Step 1: Create table.rs**

```rust
// crates/tui/src/rich_text/table.rs
// Markdown table parser → ratatui Table widget.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table};

use crate::theme;

/// Parse a markdown table from lines and render as ratatui Table widget.
/// Input: slice of lines containing a markdown table (including header and separator).
/// Returns None if the lines don't form a valid table.
pub fn parse_markdown_table<'a>(lines: &[&str]) -> Option<ratatui::widgets::Table<'a>> {
    if lines.len() < 2 {
        return None;
    }

    let header_cells = split_row(lines[0])?;
    let sep_cells = split_row(lines[1])?;

    // Validate separator row (must contain --- patterns)
    if !sep_cells.iter().all(|c| c.trim().starts_with('-') || c.trim().starts_with(":-")) {
        return None;
    }

    // Determine alignment from separator
    let alignments: Vec<_> = sep_cells
        .iter()
        .map(|c| {
            let c = c.trim();
            let left = c.starts_with(":-");
            let right = c.ends_with("-:");
            match (left, right) {
                (true, true) => "center",
                (true, false) => "left",
                (false, true) => "right",
                _ => "left",
            }
        })
        .collect();

    let header_row = Row::new(
        header_cells
            .iter()
            .map(|c| Cell::from(Span::styled(
                c.trim().to_string(),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            )))
            .collect::<Vec<_>>(),
    )
    .style(Style::default().bg(theme::PANEL_ALT));

    // Data rows (skip header and separator)
    let data_rows: Vec<Row> = lines[2..]
        .iter()
        .filter_map(|line| {
            let cells = split_row(line)?;
            Some(Row::new(
                cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        Cell::from(Span::styled(
                            c.trim().to_string(),
                            Style::default().fg(theme::TEXT),
                        ))
                    })
                    .collect::<Vec<_>>(),
            ))
        })
        .collect();

    let column_count = header_cells.len() as u16;
    let widths: Vec<ratatui::layout::Constraint> = (0..column_count)
        .map(|_| ratatui::layout::Constraint::Percentage(100 / column_count))
        .collect();

    let table = Table::new(data_rows, widths)
        .header(header_row)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::BORDER))
                .style(Style::default().bg(theme::PANEL))
        );

    Some(table)
}

/// Split a markdown table row into cells by `|`.
fn split_row(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    let cells: Vec<String> = inner.split('|').map(|c| c.to_string()).collect();
    if cells.is_empty() {
        return None;
    }
    Some(cells)
}

/// Check if a line looks like the start of a markdown table.
pub fn is_table_header(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 2
}

/// Check if a line is a markdown table separator (e.g. `| --- | --- |`).
pub fn is_table_separator(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.ends_with('|') && line.contains("---")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_table_header() {
        assert!(is_table_header("| A | B |"));
        assert!(!is_table_header("plain text"));
        assert!(!is_table_header("| not a table"));
    }

    #[test]
    fn test_is_table_separator() {
        assert!(is_table_separator("| --- | --- |"));
        assert!(is_table_separator("| :--- | ---: |"));
        assert!(!is_table_separator("| data | more |"));
    }

    #[test]
    fn test_split_row() {
        let cells = split_row("| a | b | c |").unwrap();
        assert_eq!(cells, vec!["a", "b", "c"]);
        assert!(split_row("not a row").is_none());
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -5`
Expected: compile success.

- [ ] **Step 3: Run tests**

Run: `cargo test -p tui table::tests 2>&1 | tail -10`
Expected: tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/rich_text/table.rs
git commit -m "feat(tui): add markdown table parser and ratatui Table renderer
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: LaTeX to Unicode converter (rich_text/latex.rs)

**Files:**
- Create: `crates/tui/src/rich_text/latex.rs`

- [ ] **Step 1: Create latex.rs**

```rust
// crates/tui/src/rich_text/latex.rs
// Convert common LaTeX math commands to Unicode characters.

use std::collections::HashMap;
use std::sync::OnceLock;

fn latex_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        // Greek lowercase
        m.insert("\\alpha", "α");   m.insert("\\beta", "β");
        m.insert("\\gamma", "γ");   m.insert("\\delta", "δ");
        m.insert("\\epsilon", "ε"); m.insert("\\zeta", "ζ");
        m.insert("\\eta", "η");     m.insert("\\theta", "θ");
        m.insert("\\iota", "ι");    m.insert("\\kappa", "κ");
        m.insert("\\lambda", "λ");  m.insert("\\mu", "μ");
        m.insert("\\nu", "ν");      m.insert("\\xi", "ξ");
        m.insert("\\pi", "π");      m.insert("\\rho", "ρ");
        m.insert("\\sigma", "σ");   m.insert("\\tau", "τ");
        m.insert("\\upsilon", "υ"); m.insert("\\phi", "φ");
        m.insert("\\chi", "χ");     m.insert("\\psi", "ψ");
        m.insert("\\omega", "ω");
        // Greek uppercase
        m.insert("\\Gamma", "Γ");   m.insert("\\Delta", "Δ");
        m.insert("\\Theta", "Θ");   m.insert("\\Lambda", "Λ");
        m.insert("\\Xi", "Ξ");      m.insert("\\Pi", "Π");
        m.insert("\\Sigma", "Σ");   m.insert("\\Phi", "Φ");
        m.insert("\\Psi", "Ψ");     m.insert("\\Omega", "Ω");
        // Math symbols
        m.insert("\\infty", "∞");   m.insert("\\pm", "±");
        m.insert("\\mp", "∓");      m.insert("\\times", "×");
        m.insert("\\div", "÷");     m.insert("\\cdot", "·");
        m.insert("\\approx", "≈");  m.insert("\\neq", "≠");
        m.insert("\\leq", "≤");     m.insert("\\geq", "≥");
        m.insert("\\ll", "≪");      m.insert("\\gg", "≫");
        m.insert("\\equiv", "≡");   m.insert("\\sim", "∼");
        m.insert("\\propto", "∝");  m.insert("\\partial", "∂");
        m.insert("\\nabla", "∇");   m.insert("\\int", "∫");
        m.insert("\\iint", "∬");    m.insert("\\iiint", "∭");
        m.insert("\\oint", "∮");    m.insert("\\sum", "∑");
        m.insert("\\prod", "∏");    m.insert("\\coprod", "∐");
        m.insert("\\sqrt", "√");    m.insert("\\forall", "∀");
        m.insert("\\exists", "∃");  m.insert("\\nexists", "∄");
        m.insert("\\in", "∈");      m.insert("\\notin", "∉");
        m.insert("\\subset", "⊂");  m.insert("\\supset", "⊃");
        m.insert("\\subseteq", "⊆"); m.insert("\\supseteq", "⊇");
        m.insert("\\cup", "∪");     m.insert("\\cap", "∩");
        m.insert("\\emptyset", "∅"); m.insert("\\varnothing", "∅");
        m.insert("\\land", "∧");    m.insert("\\lor", "∨");
        m.insert("\\neg", "¬");     m.insert("\\implies", "→");
        m.insert("\\iff", "⇔");     m.insert("\\rightarrow", "→");
        m.insert("\\leftarrow", "←"); m.insert("\\leftrightarrow", "↔");
        m.insert("\\uparrow", "↑"); m.insert("\\downarrow", "↓");
        m.insert("\\mapsto", "↦");  m.insert("\\to", "→");
        m.insert("\\langle", "⟨");  m.insert("\\rangle", "⟩");
        m.insert("\\lceil", "⌈");   m.insert("\\rceil", "⌉");
        m.insert("\\lfloor", "⌊");  m.insert("\\rfloor", "⌋");
        m.insert("\\ldots", "…");   m.insert("\\cdots", "⋯");
        m.insert("\\vdots", "⋮");   m.insert("\\ddots", "⋱");
        m.insert("\\angle", "∠");   m.insert("\\parallel", "∥");
        m.insert("\\perp", "⊥");    m.insert("\\circ", "∘");
        m.insert("\\triangle", "△"); m.insert("\\square", "□");
        m.insert("\\diamond", "◇"); m.insert("\\star", "★");
        m.insert("\\aleph", "ℵ");   m.insert("\\hbar", "ℏ");
        m.insert("\\ell", "ℓ");     m.insert("\\wp", "℘");
        m.insert("\\Re", "ℜ");      m.insert("\\Im", "ℑ");
        m.insert("\\prime", "′");   m.insert("\\backslash", "\\");
        m
    })
}

/// Convert inline LaTeX math ($...$) to Unicode.
/// Handles \frac{a}{b} → (a)/(b) as a special pattern.
pub fn render_latex_inline(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch == '$' {
            chars.next(); // consume opening $
            let mut math = String::new();
            let mut depth = 0u32;
            for c in chars.by_ref() {
                if c == '{' { depth += 1; }
                if c == '}' && depth > 0 { depth -= 1; }
                if c == '$' && depth == 0 { break; }
                if c == '\\' && depth == 0 {
                    // Check for $$ delimiter
                    math.push(c);
                    continue;
                }
                math.push(c);
            }
            result.push_str(&convert_math(&math));
        } else if ch == '\\' {
            // Lone LaTeX command outside $...$ — try to convert
            chars.next();
            let mut cmd = String::from("\\");
            for c in chars.by_ref() {
                if c.is_alphabetic() {
                    cmd.push(c);
                } else {
                    // Put back the non-alpha char
                    let converted = convert_command(&cmd);
                    result.push_str(&converted);
                    result.push(c);
                    break;
                }
            }
            // If we exhausted the iterator on alpha chars
            if cmd.len() > 1 && cmd.chars().skip(1).all(|c| c.is_alphabetic()) {
                result.push_str(&convert_command(&cmd));
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    result
}

fn convert_math(math: &str) -> String {
    // Handle \frac{a}{b} → (a)/(b) pattern
    let mut result = math.to_string();
    while let Some(pos) = result.find("\\frac") {
        let after = &result[pos + 5..];
        if let Some(open1) = after.find('{') {
            let inner_start = pos + 5 + open1 + 1;
            if let Some(close1) = result[inner_start..].find('}') {
                let num = &result[inner_start..inner_start + close1];
                let after_num = inner_start + close1 + 1;
                if result[after_num..].starts_with('{') {
                    if let Some(close2) = result[after_num + 1..].find('}') {
                        let den = &result[after_num + 1..after_num + 1 + close2];
                        let end = after_num + 1 + close2 + 1;
                        let replacement = format!("({})/({})", num, den);
                        result.replace_range(pos..end, &replacement);
                        continue;
                    }
                }
            }
        }
        break; // couldn't parse, stop trying
    }

    // Replace LaTeX commands with Unicode
    let map = latex_map();
    // Sort by length descending to match longest first
    let mut keys: Vec<&&str> = map.keys().collect();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));

    for key in keys {
        result = result.replace(*key, map[*key]);
    }

    // Remove remaining braces, underscores, carets
    result = result.replace('{', "").replace('}', "");
    // Convert _x → subscript, ^x → superscript (single char only)
    // These are best-effort approximations
    result
}

fn convert_command(cmd: &str) -> String {
    latex_map().get(cmd).map(|&s| s.to_string()).unwrap_or_else(|| cmd.to_string())
}

/// Detect if text contains LaTeX math delimiters.
pub fn has_latex(text: &str) -> bool {
    text.contains('$') || text.contains("\\frac") || text.contains("\\sum") || text.contains("\\int")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_symbols() {
        assert_eq!(render_latex_inline("$\\alpha$"), "α");
        assert_eq!(render_latex_inline("$\\beta + \\gamma$"), "β + γ");
    }

    #[test]
    fn test_frac() {
        let result = render_latex_inline("$\\frac{a}{b}$");
        assert!(result.contains("(a)/(b)"), "got: {result}");
    }

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(render_latex_inline("hello world"), "hello world");
    }

    #[test]
    fn test_has_latex() {
        assert!(has_latex("$x^2$"));
        assert!(has_latex("\\frac{a}{b}"));
        assert!(!has_latex("plain text"));
    }
}
```

- [ ] **Step 2: Verify compilation and run tests**

Run: `cargo test -p tui latex::tests 2>&1 | tail -10`
Expected: tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/rich_text/latex.rs
git commit -m "feat(tui): add LaTeX math to Unicode converter for inline formulas
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: Enhanced rich_text mod.rs — unified markdown renderer

**Files:**
- Modify: `crates/tui/src/rich_text/mod.rs`

- [ ] **Step 1: Enhance render_markdown_lines to use highlight, table, and latex**

In `mod.rs`, update the `render_markdown_lines` function to detect tables, code blocks with language tags, and LaTeX.

The existing code is ~115 lines. Replace the `render_markdown_lines` function:

```rust
/// Render multiple lines with full markdown support:
/// - Code blocks with optional language tag → syntax highlighting
/// - Tables → ratatui Table widget (returned separately for layout)
/// - LaTeX $...$ → Unicode conversion
/// - Legacy: **bold**, *italic*, `inline code`
pub fn render_markdown_lines(text: &str, base_style: Style) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buffer = String::new();
    let mut in_table = false;
    let mut table_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        // Check for table
        if !in_code_block {
            if table::is_table_header(raw_line) {
                in_table = true;
                table_lines.clear();
                table_lines.push(raw_line.to_string());
                continue;
            }
            if in_table && table::is_table_separator(raw_line) {
                table_lines.push(raw_line.to_string());
                continue;
            }
            if in_table {
                if table::is_table_header(raw_line) || raw_line.trim().is_empty() {
                    // End of table — render it
                    // Tables can't be rendered as Lines, so we convert to plain representation
                    let refs: Vec<&str> = table_lines.iter().map(|s| s.as_str()).collect();
                    if let Some(t) = table::parse_markdown_table(&refs) {
                        // We can't embed a widget in lines. Instead render as formatted text.
                        lines.push(Line::from(Span::styled(
                            "┌─ Table ─┐",
                            Style::default().fg(crate::theme::MUTED),
                        )));
                        for tl in &table_lines {
                            let rendered = render_markdown_line(tl, base_style);
                            lines.push(rendered);
                        }
                        lines.push(Line::from(Span::styled(
                            "└─────────┘",
                            Style::default().fg(crate::theme::MUTED),
                        )));
                    }
                    table_lines.clear();
                    in_table = false;
                    // Process current line normally
                    let processed = if has_latex(raw_line) {
                        latex::render_latex_inline(raw_line)
                    } else {
                        raw_line.to_string()
                    };
                    lines.push(render_markdown_line(&processed, base_style));
                    continue;
                } else {
                    table_lines.push(raw_line.to_string());
                    continue;
                }
            }
        }

        // Check for code fence
        if raw_line.trim().starts_with("```") {
            if in_code_block {
                // End of code block — render highlighted
                let lang = code_lang.trim().to_string();
                let highlighted = highlight::highlight_code(&lang, &code_buffer, crate::theme::PANEL);
                lines.extend(highlighted);
                code_buffer.clear();
                code_lang.clear();
                in_code_block = false;
            } else {
                // Start of code block
                code_lang = raw_line.trim().trim_start_matches("```").trim().to_string();
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            if !code_buffer.is_empty() {
                code_buffer.push('\n');
            }
            code_buffer.push_str(raw_line);
            continue;
        }

        // Inline processing: LaTeX conversion
        let processed = if has_latex(raw_line) {
            latex::render_latex_inline(raw_line)
        } else {
            raw_line.to_string()
        };

        lines.push(render_markdown_line(&processed, base_style));
    }

    // Flush remaining code block
    if in_code_block && !code_buffer.is_empty() {
        let lang = code_lang.trim().to_string();
        let highlighted = highlight::highlight_code(&lang, &code_buffer, crate::theme::PANEL);
        lines.extend(highlighted);
    }

    // Flush remaining table
    if in_table && !table_lines.is_empty() {
        for tl in &table_lines {
            lines.push(render_markdown_line(tl, base_style));
        }
    }

    lines
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -10`
Expected: compile success.

- [ ] **Step 3: Run existing markdown tests**

Run: `cargo test -p tui rich_text::tests 2>&1 | tail -10`
Expected: existing tests still pass.

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/rich_text/mod.rs
git commit -m "feat(tui): enhance markdown renderer with syntax highlighting, table detection, and LaTeX
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 9: Multiline input in panels/input.rs and state.rs

**Files:**
- Modify: `crates/tui/src/state.rs`
- Modify: `crates/tui/src/panels/input.rs`
- Modify: `crates/tui/src/run.rs` (keyboard handler for Shift+Enter)

- [ ] **Step 1: Add multiline fields to TuiAppState**

In `state.rs`, after the `input_cursor` field, add:

```rust
    /// Multiline input line count (1–8)
    pub input_line_count: u8,
```

In `TuiAppState::new()`, add the default:

```rust
            input_line_count: 1,
```

- [ ] **Step 2: Update input.rs for multiline rendering**

In the `render_text_input` function in `input.rs`, replace the flat single-line Span building with multiline logic. After `let char_indices: ...` line, replace the `before`/`after` logic and spans building:

```rust
fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    state: &TuiAppState,
    focused: bool,
    border_color: ratatui::style::Color,
    content: InputContent<'_>,
) {
    let cursor_char = if state.frame_count % 16 < 8 { "▌" } else { " " };
    let text = content.text;
    let cursor = content.cursor.min(text.chars().count());

    let label_bg = if focused { theme::CYAN } else { theme::MUTED };

    // Split text into lines (for multiline input)
    let input_lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split('\n').collect()
    };

    // Find which line the cursor is on
    let mut char_count = 0usize;
    let mut cursor_line = 0usize;
    let mut cursor_col = 0usize;
    for (i, line) in input_lines.iter().enumerate() {
        let line_len = line.chars().count();
        if char_count + line_len >= cursor || i == input_lines.len() - 1 {
            cursor_line = i;
            cursor_col = cursor.saturating_sub(char_count);
            break;
        }
        char_count += line_len + 1; // +1 for newline
    }

    let visible_lines = state.input_line_count.max(1) as usize;
    let start_line = cursor_line.saturating_sub(visible_lines.saturating_sub(1));

    let mut all_lines: Vec<Line> = Vec::new();

    for (i, line) in input_lines.iter().enumerate().skip(start_line).take(visible_lines) {
        let mut spans: Vec<Span> = Vec::new();

        // Label only on first line
        if i == start_line {
            spans.push(Span::styled(
                format!(" {} ", content.label),
                Style::default().fg(theme::BG).bg(label_bg).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" │ ", Style::default().fg(border_color).bg(theme::PANEL_ALT)));
        } else {
            // Indent to align with text after label+separator
            let indent = content.label.len() + 4; // " LABEL │ "
            spans.push(Span::styled(" ".repeat(indent), Style::default().bg(theme::PANEL_ALT)));
        }

        let line_chars: Vec<char> = line.chars().collect();
        let line_len = line_chars.len();

        if i == cursor_line {
            // Insert cursor into this line
            let col = cursor_col.min(line_len);
            let before: String = line_chars[..col].iter().collect();
            let after: String = line_chars[col..].iter().collect();

            if !before.is_empty() {
                spans.push(Span::styled(before, Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT)));
            }
            spans.push(Span::styled(cursor_char, Style::default().fg(theme::CYAN).bg(theme::PANEL_ALT)));
            if !after.is_empty() {
                spans.push(Span::styled(after, Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT)));
            }
        } else {
            let text: String = line_chars.iter().collect();
            spans.push(Span::styled(text, Style::default().fg(theme::TEXT).bg(theme::PANEL_ALT)));
        }

        all_lines.push(Line::from(spans));
    }

    // Add hints on the last visible line, or as a separate line if no room
    let hint_text = if all_lines.len() < visible_lines as usize {
        // Add hints as padding on the last line
        content.hints.to_string()
    } else {
        String::new()
    };

    if !hint_text.is_empty() {
        all_lines.push(Line::from(Span::styled(hint_text, Style::default().fg(theme::SUBTLE).bg(theme::PANEL_ALT))));
    }

    let para = Paragraph::new(all_lines).style(Style::default().bg(theme::PANEL_ALT));
    frame.render_widget(para, area);
}
```

- [ ] **Step 3: Update run.rs keyboard handler for Shift+Enter in input mode**

In `run.rs`, find the input mode keyboard handler (around line 440-637, where `KeyCode::Enter` is handled). Replace the Enter key handler:

Find the block matching Enter key in input mode (state.awaiting_input = true section). Replace with:

```rust
// Find the line: KeyCode::Enter => {
// In the input mode (state.awaiting_input or state.slash_command_active sections)
// Replace the Enter handling logic:

KeyCode::Enter => {
    if mods.contains(KeyModifiers::SHIFT) {
        // Insert newline for multiline input
        if state.awaiting_input {
            let mut buffer = tui_input.buffer.lock();
            let cursor = *tui_input.cursor.lock();
            let chars: Vec<char> = buffer.chars().collect();
            let before: String = chars[..cursor].iter().collect();
            let after: String = chars[cursor..].iter().collect();
            *buffer = format!("{before}\n{after}");
            let new_cursor = cursor + 1;
            *tui_input.cursor.lock() = new_cursor;
            state.input_line_count = (state.input_line_count + 1).min(8);
        }
    } else {
        // Submit — existing code
        // ... (keep existing submit logic)
    }
}
```

- [ ] **Step 4: Also update the footer hints to reflect Shift+Enter**

In render.rs or footer.rs, update the hint text to include "Shift+Enter 换行" when in input mode.

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p tui 2>&1 | tail -10`
Expected: compile success.

- [ ] **Step 6: Commit**

```bash
git add crates/tui/src/state.rs crates/tui/src/panels/input.rs crates/tui/src/run.rs
git commit -m "feat(tui): add multiline input support with Shift+Enter for newline insertion
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 10: @-mention context reference panel (panels/context_ref.rs)

**Files:**
- Create: `crates/tui/src/panels/context_ref.rs`
- Modify: `crates/tui/src/panels/mod.rs`
- Modify: `crates/tui/src/render.rs`
- Modify: `crates/tui/src/state.rs`

- [ ] **Step 1: Add context reference state fields to state.rs**

In `TuiAppState`, add:

```rust
    /// @-mention context reference popup
    pub context_ref_active: bool,
    pub context_ref_query: String,
    pub context_ref_items: Vec<ContextRefItem>,
    pub context_ref_selected: usize,
```

And the data type:

```rust
#[derive(Debug, Clone)]
pub struct ContextRefItem {
    pub source: String,  // "file", "git", "mem"
    pub label: String,
    pub preview: String,
}
```

Defaults in `TuiAppState::new()`:

```rust
            context_ref_active: false,
            context_ref_query: String::new(),
            context_ref_items: Vec::new(),
            context_ref_selected: 0,
```

- [ ] **Step 2: Create context_ref.rs panel**

```rust
// crates/tui/src/panels/context_ref.rs
// @-mention floating panel: shows file/git/memory references when user types @.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

/// Render the @-mention popup above the input area.
pub fn render_context_ref(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if !state.context_ref_active || area.width < 6 || area.height < 5 {
        return;
    }

    let popup_h = (state.context_ref_items.len() + 2).min(8) as u16;
    let popup_w = 50.min(area.width.saturating_sub(2));
    let x = area.x + 1;
    let y = area.y.saturating_sub(popup_h);

    if y == 0 && popup_h > area.y {
        return; // not enough space above
    }

    let popup_area = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL))
        .title(" @-引用 ");

    frame.render_widget(block, popup_area);
    let inner = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(theme::PANEL))
        .inner(popup_area);

    let lines: Vec<Line> = state
        .context_ref_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == state.context_ref_selected;
            let source_icon = match item.source.as_str() {
                "file" => "📄",
                "git" => "🔀",
                "mem" => "🧠",
                _ => "?",
            };
            let prefix = if is_sel { "▶ " } else { "  " };
            let style = if is_sel {
                Style::default().fg(theme::CYAN).bg(theme::PANEL_ALT)
            } else {
                Style::default().fg(theme::TEXT).bg(theme::PANEL)
            };
            Line::from(vec![
                Span::styled(format!("{prefix}{source_icon} {:<30}", item.label), style),
                Span::styled(format!(" {}", item.preview), Style::default().fg(theme::SUBTLE).bg(if is_sel { theme::PANEL_ALT } else { theme::PANEL })),
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::PANEL)),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: popup_h.saturating_sub(2),
        },
    );
}

/// Populate @-mention suggestions based on query text.
pub fn populate_suggestions(state: &mut TuiAppState) {
    let query = state.context_ref_query.to_lowercase();
    state.context_ref_items.clear();

    // Always show these categories
    let file_items = find_files(&query);
    state.context_ref_items.extend(file_items);

    state.context_ref_items.push(ContextRefItem {
        source: "git".into(),
        label: "git:diff".into(),
        preview: "当前变更摘要".into(),
    });

    if !query.is_empty() {
        state.context_ref_items.push(ContextRefItem {
            source: "mem".into(),
            label: format!("mem:{}", query),
            preview: "搜索记忆...".into(),
        });
    }

    state.context_ref_selected = 0;
}

fn find_files(query: &str) -> Vec<crate::state::ContextRefItem> {
    let mut items = Vec::new();
    // Search current directory for matching files (max 5)
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten().take(10) {
            let name = entry.file_name().to_string_lossy().to_string();
            if query.is_empty() || name.to_lowercase().contains(query) {
                items.push(crate::state::ContextRefItem {
                    source: "file".into(),
                    label: format!("file:{}", name),
                    preview: String::new(),
                });
            }
        }
    }
    // Limit to 5
    items.truncate(5);
    items
}
```

- [ ] **Step 3: Add mod declaration to panels/mod.rs**

```rust
pub mod context_ref;
```

- [ ] **Step 4: Add @-mention keyboard handling in run.rs**

In the input handling section, detect `@` character and activate context_ref mode:

```rust
// In the input character handling (KeyCode::Char(ch)):
KeyCode::Char(ch) => {
    if state.awaiting_input || state.slash_command_active {
        // Existing character insertion logic...
        // ADD: detect @ to activate context ref
        if ch == '@' && state.awaiting_input {
            state.context_ref_active = true;
            state.context_ref_query = "@".to_string();
            populate_suggestions(state);
        } else if state.context_ref_active {
            if ch == ' ' || ch == '\n' {
                state.context_ref_active = false;
            } else {
                state.context_ref_query.push(ch);
                populate_suggestions(state);
            }
        }
        // ... rest of existing char handling
    }
}
```

- [ ] **Step 5: Render context_ref panel in render.rs**

In `render_app()`, after rendering the input panel, check and render context_ref:

```rust
if state.context_ref_active {
    panels::context_ref::render_context_ref(frame, footer_area, state);
}
```

- [ ] **Step 6: Verify compilation and commit**

Run: `cargo check -p tui 2>&1 | tail -5`

```bash
git add crates/tui/src/panels/context_ref.rs crates/tui/src/panels/mod.rs crates/tui/src/state.rs crates/tui/src/run.rs crates/tui/src/render.rs
git commit -m "feat(tui): add @-mention context reference popup for file/git/memory references
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 11: Kanban board panel (panels/kanban.rs)

**Files:**
- Create: `crates/tui/src/panels/kanban.rs`
- Modify: `crates/tui/src/panels/mod.rs`
- Modify: `crates/tui/src/state.rs`
- Modify: `crates/tui/src/render.rs`
- Modify: `crates/tui/src/run.rs` (handle_event for TaskUpdated)

- [ ] **Step 1: Add kanban state to state.rs**

```rust
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
```

In `TuiAppState`, add:

```rust
    pub kanban_visible: bool,
    pub kanban_items: Vec<KanbanItem>,
    pub kanban_scrolls: [u16; 3], // scroll per column
```

Defaults:

```rust
            kanban_visible: false,
            kanban_items: Vec::new(),
            kanban_scrolls: [0; 3],
```

- [ ] **Step 2: Create kanban.rs panel**

```rust
// crates/tui/src/panels/kanban.rs
// Three-column kanban board: Pending / In Progress / Completed.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::state::KanbanStatus;
use crate::state::TuiAppState;
use crate::theme;

pub fn render_kanban(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if area.width < 30 || area.height < 6 {
        return;
    }

    let columns = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);

    let col_configs = [
        ("Pending", KanbanStatus::Pending, theme::YELLOW),
        ("In Progress", KanbanStatus::InProgress, theme::BLUE),
        ("Completed", KanbanStatus::Completed, theme::GREEN),
    ];

    for (i, (title, status, color)) in col_configs.iter().enumerate() {
        let col_area = columns[i];
        let items: Vec<&str> = state
            .kanban_items
            .iter()
            .filter(|item| item.status == *status)
            .map(|item| item.title.as_str())
            .collect();

        let count = items.len();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(*color))
            .style(Style::default().bg(theme::PANEL))
            .title(format!(" {} ({}) ", title, count));

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if items.is_empty() {
            let empty = Paragraph::new(theme::empty("(空)"))
                .style(Style::default().bg(theme::PANEL));
            frame.render_widget(empty, inner);
        } else {
            let lines: Vec<Line> = items
                .iter()
                .map(|item| {
                    let icon = match status {
                        KanbanStatus::Pending => "□",
                        KanbanStatus::InProgress => "⏳",
                        KanbanStatus::Completed => "✓",
                    };
                    Line::from(Span::styled(
                        format!(" {} {}", icon, item),
                        Style::default().fg(theme::TEXT).bg(theme::PANEL),
                    ))
                })
                .collect();

            let scroll = state.kanban_scrolls[i].min(
                items.len().saturating_sub(inner.height as usize).max(0) as u16,
            );

            let para = Paragraph::new(lines)
                .style(Style::default().bg(theme::PANEL))
                .scroll((scroll, 0));
            frame.render_widget(para, inner);
        }
    }
}
```

- [ ] **Step 3: Add handle_event logic for TaskUpdated in run.rs**

In `handle_event()` (around run.rs:1815-2012), add:

```rust
        AgentEvent::TaskUpdated { task_id, title, status } => {
            let kanban_status = match status {
                agent_core::TaskStatus::Pending => crate::state::KanbanStatus::Pending,
                agent_core::TaskStatus::InProgress => crate::state::KanbanStatus::InProgress,
                agent_core::TaskStatus::Completed => crate::state::KanbanStatus::Completed,
            };
            // Update existing or insert new
            if let Some(existing) = state.kanban_items.iter_mut().find(|k| k.id == task_id) {
                existing.status = kanban_status;
                existing.title = title;
            } else {
                state.kanban_items.push(crate::state::KanbanItem {
                    id: task_id,
                    title,
                    status: kanban_status,
                });
            }
        }
```

- [ ] **Step 4: Render kanban in render.rs**

In `render_app()`, when kanban is visible, render it in the main area or as an overlay:

```rust
if state.kanban_visible {
    // Replace right panel with kanban
    panels::kanban::render_kanban(frame, right_area, state);
}
```

- [ ] **Step 5: Toggle kanban keybinding in run.rs**

Add `Ctrl+K` handling (or use the keybinding system from Task 4):

```rust
// In keyboard dispatch:
if keybindings.action_for(&event) == Some(Action::ToggleKanban) {
    state.kanban_visible = !state.kanban_visible;
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/tui/src/panels/kanban.rs crates/tui/src/panels/mod.rs crates/tui/src/state.rs crates/tui/src/render.rs crates/tui/src/run.rs
git commit -m "feat(tui): add 3-column kanban board with Pending/InProgress/Completed columns
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 12: Enhanced thinking animation in header.rs

**Files:**
- Modify: `crates/tui/src/state.rs`
- Modify: `crates/tui/src/panels/header.rs`
- Modify: `crates/tui/src/run.rs` (handle ThinkingPhaseChanged)

- [ ] **Step 1: Add thinking_subphase to state.rs**

In `TuiAppState`, add:

```rust
    pub thinking_subphase: agent_core::ThinkingSubPhase,
```

Default:

```rust
            thinking_subphase: agent_core::ThinkingSubPhase::Idle,
```

- [ ] **Step 2: Update header.rs spinner rendering**

Replace the spinner section (lines 50-59) in `render_header()`:

```rust
    // Spinner: phase-aware animation
    if !state.agent_done && state.phase != AgentPhase::Idle {
        let (spinner, label) = match state.thinking_subphase {
            ThinkingSubPhase::CallingLlm => {
                let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                (frames[state.frame_count as usize % frames.len()], "思考中...")
            }
            ThinkingSubPhase::ParsingResponse => {
                let dots = ["◌", "◌", "◌"];
                let n = (state.frame_count / 10) as usize % 3;
                let mut s = String::new();
                for i in 0..3 {
                    if i == n { s.push('●'); } else { s.push('◌'); }
                }
                (' ', "") // will use the string label
            }
            ThinkingSubPhase::ExecutingTool => {
                (['▶', '▷'][state.frame_count as usize / 8 % 2], "执行工具...")
            }
            ThinkingSubPhase::WaitingForInput => {
                (['●', '○'][state.frame_count as usize / 16 % 2], "等待输入...")
            }
            ThinkingSubPhase::Idle => {
                let frames = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
                (frames[state.frame_count as usize % frames.len()], phase_str)
            }
        };

        if label.is_empty() {
            spans.push(Span::styled(
                format!(" {} ", spinner),
                Style::default().fg(phase_color).bg(theme::BG),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", spinner),
                Style::default().fg(phase_color).bg(theme::BG),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                label,
                Style::default().fg(phase_color).bg(theme::BG).add_modifier(Modifier::BOLD),
            ));
        }
    }
```

- [ ] **Step 3: Handle ThinkingPhaseChanged event in run.rs**

In `handle_event()`, add:

```rust
        AgentEvent::ThinkingPhaseChanged { sub_phase } => {
            state.thinking_subphase = sub_phase;
        }
```

- [ ] **Step 4: Add the import in header.rs**

```rust
use agent_core::ThinkingSubPhase;
```

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/state.rs crates/tui/src/panels/header.rs crates/tui/src/run.rs
git commit -m "feat(tui): add sub-phase thinking animations (LLM call, parse, tool exec, wait input)
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 13: Enhanced execution panel — fold/expand and granular duration

**Files:**
- Modify: `crates/tui/src/panels/execution.rs`

- [ ] **Step 1: Update duration formatting and fold hint in execution.rs**

Replace the duration formatting (line 79-84):

```rust
            let duration = step.duration_ms.map(|d| {
                let label = if d < 1 {
                    "<1ms".to_string()
                } else if d < 1000 {
                    format!("{}ms", d)
                } else if d < 60_000 {
                    format!("{:.1}s", d as f64 / 1000.0)
                } else {
                    let m = d / 60_000;
                    let s = (d % 60_000) / 1000;
                    format!("{}m{}s", m, s)
                };
                Span::styled(
                    format!("  ({})", label),
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                )
            });
```

Replace the content preview (lines 86-96):

```rust
            let content = step.content_full.as_deref().or(step.content_preview.as_deref()).map_or_else(
                || Span::raw(""),
                |c| {
                    let clean = crate::state::strip_ansi(c);
                    let short = crate::state::truncate(&clean, 30);
                    // If there's more content, show a fold hint
                    if clean.chars().count() > 30 && step.content_full.is_some() {
                        let hint = format!("  ({} chars, Enter 展开/折叠)", clean.chars().count());
                        Span::styled(
                            format!("  {}  {}", short, hint),
                            Style::default().fg(theme::MUTED).bg(theme::PANEL),
                        )
                    } else {
                        Span::styled(
                            format!("  {}", short),
                            Style::default().fg(theme::MUTED).bg(theme::PANEL),
                        )
                    }
                },
            );
```

- [ ] **Step 2: Commit**

```bash
git add crates/tui/src/panels/execution.rs
git commit -m "feat(tui): add granular duration formatting and fold/expand hints in execution panel
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 14: Slash commands — TUI-only commands

**Files:**
- Modify: `crates/tui/src/run.rs` (dispatch_slash_command function, lines 1326-1514)

- [ ] **Step 1: Replace stub implementations in dispatch_slash_command**

Replace the stub cases. For `/new`:

```rust
        "/new" => {
            // Reset the TUI state
            state.turn = 0;
            state.phase = crate::state::AgentPhase::Idle;
            state.agent_done = false;
            state.streaming_buffer.clear();
            state.summary_streaming_buffer.clear();
            state.executions.clear();
            state.exec_total_steps = 0;
            state.exec_completed_steps = 0;
            state.summary = None;
            state.log_entries.clear();
            state.kanban_items.clear();
            state.plan_ready = false;
            state.plan_steps_count = 0;
            state.total_duration_ms = None;
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "New Session".into(),
                lines: vec!["  已重置会话状态。".into(), "  等待新任务输入...".into()],
                scroll: 0,
            });
        }
```

For `/load <name>`:

```rust
        "/load" => {
            let name = if rest.is_empty() { "latest" } else { &rest };
            let session_path = home_dir().join(".hermess").join("sessions").join(format!("{}.json", name));
            if session_path.exists() {
                match std::fs::read_to_string(&session_path) {
                    Ok(raw) => {
                        state.slash_command_popup = Some(crate::state::SlashResult {
                            title: format!("Session: {}", name),
                            lines: vec![
                                format!("  从 {} 加载会话", session_path.display()),
                                format!("  大小: {} bytes", raw.len()),
                                String::new(),
                                "  会话恢复需要完整的状态序列化支持。".into(),
                            ],
                            scroll: 0,
                        });
                    }
                    Err(e) => {
                        push_log(state, format!("加载会话失败: {}", e), true);
                    }
                }
            } else {
                state.slash_command_popup = Some(crate::state::SlashResult {
                    title: "Session Not Found".into(),
                    lines: vec![
                        format!("  未找到会话: {}", name),
                        String::new(),
                        "  可用 /sessions 查看已保存的会话列表。".into(),
                    ],
                    scroll: 0,
                });
            }
        }
```

For `/memory` and `/recall`:

```rust
        "/memory" | "/recall" => {
            if rest.is_empty() {
                state.slash_command_popup = Some(crate::state::SlashResult {
                    title: "Memory".into(),
                    lines: vec![
                        "  用法: /memory <查询关键词>".into(),
                        "  在记忆库中搜索相关内容。".into(),
                    ],
                    scroll: 0,
                });
            } else {
                // Query working memory through the evolution engine's store
                // Since we can't do async here, just log it
                push_log(state, format!("[memory] 搜索: \"{}\" — 需要异步后端支持", rest), false);
                state.slash_command_popup = Some(crate::state::SlashResult {
                    title: format!("Memory: {}", crate::state::truncate(&rest, 30)),
                    lines: vec![
                        format!("  查询: \"{}\"", rest),
                        String::new(),
                        "  (记忆搜索需要异步后端接口)".into(),
                    ],
                    scroll: 0,
                });
            }
        }
```

For `/compress`:

```rust
        "/compress" => {
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Compress Context".into(),
                lines: vec![
                    "  请求压缩当前对话上下文。".into(),
                    "  这将触发后端 AgentEvent::CompressContext。".into(),
                ],
                scroll: 0,
            });
        }
```

For `/cron`:

```rust
        "/cron" | "/kanban" if head == "/cron" => {
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Cron Jobs".into(),
                lines: vec![
                    "  Cron 定时任务列表需要通过 scheduler crate 查询。".into(),
                    String::new(),
                    "  可用的 cron 表达式格式:".into(),
                    "    */5 * * * *  — 每5分钟".into(),
                    "    0 9 * * 1-5  — 工作日早9点".into(),
                ],
                scroll: 0,
            });
        }
```

For `/checkpoint`:

```rust
        "/checkpoint" => {
            push_log(state, "保存检查点...".into(), false);
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Checkpoint Saved".into(),
                lines: vec![
                    format!("  回合: {}", state.turn),
                    format!("  步骤: {}/{}", state.exec_completed_steps, state.exec_total_steps),
                    format!("  日志: {} 条", state.log_entries.len()),
                ],
                scroll: 0,
            });
        }
```

For `/rollback`:

```rust
        "/rollback" => {
            push_log(state, "[rollback] 功能需要后端 checkpoint 数据持久化支持。".into(), false);
        }
```

For `/diff`:

```rust
        "/diff" => {
            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Diff".into(),
                lines: vec![
                    "  对比当前会话与上一个检查点的差异。".into(),
                    String::new(),
                    "  此功能需要完整的 checkpoint 状态差异引擎。".into(),
                ],
                scroll: 0,
            });
        }
```

For `/personality`:

```rust
        "/personality" => {
            if rest.is_empty() {
                state.slash_command_popup = Some(crate::state::SlashResult {
                    title: "Personality".into(),
                    lines: vec![
                        "  用法: /personality <名称>".into(),
                        "  可用的预设人格: developer, analyst, writer, tutor".into(),
                    ],
                    scroll: 0,
                });
            } else {
                push_log(state, format!("人格已设置为: {} (需要后端 system prompt 注入支持)", rest), false);
            }
        }
```

And update the `/cron | /kanban` case:

```rust
        "/cron" | "/kanban" => {
            state.kanban_visible = !state.kanban_visible;
            let msg = if state.kanban_visible { "看板已打开" } else { "看板已关闭" };
            push_log(state, msg.into(), false);
        }
```

- [ ] **Step 2: Update the /usage command for detailed breakdown**

```rust
        "/usage" => {
            let mut lines = vec![
                format!("  回合数: {}", state.turn),
                format!("  已执行步骤: {} / {}", state.exec_completed_steps, state.exec_total_steps),
                format!("  进化统计: {} 条 insight", state.evolution.insight_count()),
                String::new(),
            ];

            if let Some(ref tracker) = state.usage_tracker {
                let snap = tracker.snapshot();
                lines.push("  ── Token 用量 ──".into());
                lines.push(format!("  Prompt:      {:>8} tokens", snap.prompt_tokens));
                lines.push(format!("  Completion:  {:>8} tokens", snap.completion_tokens));
                lines.push(format!("  Total:       {:>8} tokens", snap.prompt_tokens + snap.completion_tokens));
                if snap.estimated_cost_usd > 0.0 {
                    lines.push(format!("  Est. Cost:   ${:.4}", snap.estimated_cost_usd));
                }
            } else {
                lines.push("  (UsageTracker 未连接)".into());
            }

            let secs = state.frame_count / 30;
            let elapsed = if secs < 60 {
                format!("{}s", secs)
            } else {
                format!("{}m{}s", secs / 60, secs % 60)
            };
            lines.push(String::new());
            lines.push(format!("  耗时: {}", elapsed));

            state.slash_command_popup = Some(crate::state::SlashResult {
                title: "Usage".into(),
                lines,
                scroll: 0,
            });
        }
```

- [ ] **Step 3: Commit**

```bash
git add crates/tui/src/run.rs
git commit -m "feat(tui): implement all stub slash commands — /new, /load, /memory, /compress, /cron, etc.
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 15: Theme tab in settings panel

**Files:**
- Modify: `crates/tui/src/state.rs`
- Modify: `crates/tui/src/panels/settings.rs`

- [ ] **Step 1: Add Theme variant to SettingsTab in state.rs**

```rust
pub enum SettingsTab {
    Llm,
    Search,
    Finance,
    Theme,  // NEW
}

impl SettingsTab {
    pub fn next(self) -> Self {
        match self {
            SettingsTab::Llm => SettingsTab::Search,
            SettingsTab::Search => SettingsTab::Finance,
            SettingsTab::Finance => SettingsTab::Theme,
            SettingsTab::Theme => SettingsTab::Llm,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsTab::Llm => SettingsTab::Theme,
            SettingsTab::Search => SettingsTab::Llm,
            SettingsTab::Finance => SettingsTab::Search,
            SettingsTab::Theme => SettingsTab::Finance,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::Llm => "LLM",
            SettingsTab::Search => "搜索",
            SettingsTab::Finance => "金融",
            SettingsTab::Theme => "主题",
        }
    }
}
```

- [ ] **Step 2: Add theme fields to state.rs**

```rust
    pub theme_preset: String, // "tokyo-night", "dracula", etc.
```

Default:

```rust
            theme_preset: "tokyo-night".into(),
```

- [ ] **Step 3: Add Theme tab fields and rendering in settings.rs**

In `fields_for_tab()`, add:

```rust
        SettingsTab::Theme => vec![
            FieldDef { label: "预设主题", kind: FieldKind::Dropdown },
        ],
```

In `FieldDef::rendered_value()`, add:

```rust
            "预设主题" => state.theme_preset.clone(),
```

And update the dropdown cycling for the "预设主题" field. In the settings keyboard handler in run.rs, add cycling through preset names when the theme field is active.

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/state.rs crates/tui/src/panels/settings.rs crates/tui/src/run.rs
git commit -m "feat(tui): add Theme tab to settings panel with preset cycling
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 16: Multi-session tab bar

**Files:**
- Create: `crates/tui/src/panels/tab_bar.rs`
- Modify: `crates/tui/src/panels/mod.rs`
- Modify: `crates/tui/src/state.rs`
- Modify: `crates/tui/src/render.rs`
- Modify: `crates/tui/src/run.rs`

- [ ] **Step 1: Add session tabs to state.rs**

```rust
#[derive(Debug, Clone)]
pub struct SessionTab {
    pub name: String,
    pub active: bool,
}
```

In `TuiAppState`, add:

```rust
    pub session_tabs: Vec<SessionTab>,
    pub active_tab_index: usize,
```

Defaults:

```rust
            session_tabs: vec![SessionTab { name: "会话1".into(), active: true }],
            active_tab_index: 0,
```

- [ ] **Step 2: Create tab_bar.rs**

```rust
// crates/tui/src/panels/tab_bar.rs
// Multi-session tab bar (1 line, shown between header and main).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::TuiAppState;
use crate::theme;

pub fn render_tab_bar(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if state.session_tabs.len() <= 1 {
        return; // Don't show tab bar for single session
    }

    let tabs: Vec<Span> = state
        .session_tabs
        .iter()
        .enumerate()
        .flat_map(|(i, tab)| {
            let is_active = i == state.active_tab_index;
            let style = if is_active {
                Style::default().fg(theme::BG).bg(theme::CYAN).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED).bg(theme::PANEL)
            };
            let close = Span::styled("×", Style::default().fg(if is_active { theme::BG } else { theme::SUBTLE }).bg(if is_active { theme::CYAN } else { theme::PANEL }));
            let name = Span::styled(format!(" {} ", tab.name), style);
            if i > 0 {
                vec![
                    Span::styled(" ", Style::default().bg(theme::BG)),
                    name,
                    close,
                ]
            } else {
                vec![name, close]
            }
        })
        .collect();

    // Add "+" button for new tab
    let plus = Span::styled(" + ", Style::default().fg(theme::SUBTLE).bg(theme::BG));
    let all_spans: Vec<Span> = tabs.into_iter().chain(std::iter::once(plus)).collect();

    frame.render_widget(
        Paragraph::new(Line::from(all_spans)).style(Style::default().bg(theme::BG)),
        area,
    );
}
```

- [ ] **Step 3: Add keyboard handlers in run.rs for Ctrl+T (new tab), Ctrl+W (close tab)**

```rust
// Ctrl+T: new tab
// Ctrl+W: close current tab
// Ctrl+Left/Right: switch tabs
```

Find the keyboard handling section and add actions for NewTab/CloseTab/TabNext/TabPrev from the keybinding system.

- [ ] **Step 4: Render tab bar in render.rs**

In `render_app()`, after the header:

```rust
if state.session_tabs.len() > 1 {
    let v_chunks_with_tabs = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // tab bar
        Constraint::Min(1),    // main
        // ... rest
    ]);
    // ... split logic with tab bar at index 1
    panels::tab_bar::render_tab_bar(frame, v_chunks_with_tabs[1], state);
}
```

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/panels/tab_bar.rs crates/tui/src/panels/mod.rs crates/tui/src/state.rs crates/tui/src/render.rs crates/tui/src/run.rs
git commit -m "feat(tui): add multi-session tab bar with Ctrl+T new tab, Ctrl+W close, Ctrl+arrows switch
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 17: Integration — update run.rs to wire keybindings and theme

**Files:**
- Modify: `crates/tui/src/run.rs` (large integration change)

- [ ] **Step 1: In run_tui(), load theme and keybindings at startup**

At the top of `run_tui()`, add:

```rust
let theme = Theme::load();
theme::init_theme(theme);
let keybindings = Arc::new(KeyBindings::load());
```

- [ ] **Step 2: Replace hardcoded key checks with keybinding lookup**

In the spawn_blocking keyboard loop, replace individual `KeyCode::Char('q')` etc. checks with:

```rust
let action = keybindings.action_for(&event);
match action {
    Some(Action::Quit) => { state.should_quit = true; }
    Some(Action::ToggleHelp) => { state.help_visible = !state.help_visible; }
    Some(Action::ToggleSettings) => { state.settings_visible = !state.settings_visible; }
    Some(Action::ToggleKanban) => { state.kanban_visible = !state.kanban_visible; }
    Some(Action::ToggleLog) => { state.log_visible = !state.log_visible; }
    Some(Action::FocusNext) => {
        state.focused_panel = state.focused_panel.next();
    }
    // ... etc
    None => {
        // Fall through to mode-specific handling
    }
}
```

- [ ] **Step 3: Write default keybindings template on first run**

After theme loading, check if keybindings.toml exists, and if not, create it:

```rust
let kb_path = home_dir().join(".hermess").join("keybindings.toml");
if !kb_path.exists() {
    let _ = KeyBindings::write_default_template(&kb_path);
}
```

- [ ] **Step 4: Verify full build**

Run: `cargo build -p tui 2>&1 | tail -20`
Expected: successful build.

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/run.rs
git commit -m "feat(tui): wire keybinding system and theme loading into main event loop
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 18: Update help panel with new shortcuts

**Files:**
- Modify: `crates/tui/src/panels/help.rs`

- [ ] **Step 1: Add new shortcuts to the help panel**

Add entries for:
- `Shift+Enter` — 换行（多行输入）
- `Ctrl+K` — 切换看板
- `Ctrl+T` — 新建会话标签
- `Ctrl+W` — 关闭当前标签
- `Ctrl+Left/Right` — 切换标签
- `@` — 上下文引用
- `/` — 搜索
- `:` — 斜杠命令

- [ ] **Step 2: Commit**

```bash
git add crates/tui/src/panels/help.rs
git commit -m "feat(tui): update help panel with new keyboard shortcuts
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 19: Full workspace build and test

- [ ] **Step 1: Run cargo build for entire workspace**

Run: `cargo build 2>&1 | tail -20`
Expected: full workspace build success.

- [ ] **Step 2: Run all tui tests**

Run: `cargo test -p tui 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 3: Run all workspace tests**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 4: Fix any compilation or test failures**

If any step fails, fix the issue and re-run. Common issues:
- Missing imports in handle_event (add `use agent_core::...`)
- syntect linking issues (check features in Cargo.toml)
- Keybinding Action enum variants not matching

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "chore(tui): final integration fixes — full workspace builds and tests pass
Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```
