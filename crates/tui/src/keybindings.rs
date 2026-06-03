// crates/tui/src/keybindings.rs
// Configurable keyboard shortcuts loaded from ~/.hermess/keybindings.toml.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

    Some(KeyDesc { code, modifiers: mods })
}

/// Keybinding map with TOML deserialization support.
#[derive(Debug, Clone)]
pub struct KeyBindings {
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
        let path = std::env::var("HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(".hermess")
            .join("keybindings.toml");
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(table) = toml::from_str::<toml::Table>(&raw) {
                    if let Some(bindings_table) = table.get("bindings").and_then(|v| v.as_table()) {
                        for (key, val) in bindings_table {
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

    /// Write default keybindings to a file as a template.
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
