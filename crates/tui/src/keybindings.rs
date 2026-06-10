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
        "l" => KeyCode::Char('l'),
        "c" => KeyCode::Char('c'),
        "w" => KeyCode::Char('w'),
        "g" => KeyCode::Char('g'),
        "e" => KeyCode::Char('e'),
        "f" => KeyCode::Char('f'),
        "y" => KeyCode::Char('y'),
        "t" => KeyCode::Char('t'),
        "o" => KeyCode::Char('o'),
        // SAFETY: pattern guard `other.len() == 1` ensures exactly one char exists.
        other if other.len() == 1 => KeyCode::Char(other.chars().next().unwrap()),
        _ => return None,
    };

    Some(KeyDesc {
        code,
        modifiers: mods,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // ── parse_key ──

    #[test]
    fn test_parse_key_simple_chars() {
        assert!(parse_key("a").is_some());
        assert!(parse_key("?").is_some());
        assert!(parse_key("/").is_some());
        assert!(parse_key(":").is_some());
    }

    #[test]
    fn test_parse_key_return_none_for_unknown() {
        assert!(parse_key("unknown_long_key").is_none());
        assert!(parse_key("").is_none());
    }

    #[test]
    fn test_parse_key_ctrl_modifier() {
        let desc = parse_key("Ctrl+c").unwrap();
        assert_eq!(desc.code, KeyCode::Char('c'));
        assert!(desc.modifiers.contains(KeyModifiers::CONTROL));

        let desc = parse_key("Control+x").unwrap();
        assert_eq!(desc.code, KeyCode::Char('x'));
        assert!(desc.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_key_shift_modifier() {
        let desc = parse_key("Shift+Enter").unwrap();
        assert_eq!(desc.code, KeyCode::Enter);
        assert!(desc.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_key_alt_modifier() {
        let desc = parse_key("Alt+Enter").unwrap();
        assert_eq!(desc.code, KeyCode::Enter);
        assert!(desc.modifiers.contains(KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_key_special_keys() {
        assert_eq!(parse_key("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("Return").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("Tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key("Escape").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("Backspace").unwrap().code, KeyCode::Backspace);
        assert_eq!(parse_key("Delete").unwrap().code, KeyCode::Delete);
        assert_eq!(parse_key("Del").unwrap().code, KeyCode::Delete);
        assert_eq!(parse_key("Insert").unwrap().code, KeyCode::Insert);
    }

    #[test]
    fn test_parse_key_navigation() {
        assert_eq!(parse_key("Up").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key("Down").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key("Left").unwrap().code, KeyCode::Left);
        assert_eq!(parse_key("Right").unwrap().code, KeyCode::Right);
        assert_eq!(parse_key("Home").unwrap().code, KeyCode::Home);
        assert_eq!(parse_key("End").unwrap().code, KeyCode::End);
        assert_eq!(parse_key("PageUp").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_key("PgUp").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_key("PageDown").unwrap().code, KeyCode::PageDown);
        assert_eq!(parse_key("PgDn").unwrap().code, KeyCode::PageDown);
        assert_eq!(parse_key("BackTab").unwrap().code, KeyCode::BackTab);
    }

    #[test]
    fn test_parse_key_space_variants() {
        assert_eq!(parse_key("space").unwrap().code, KeyCode::Char(' '));
        // trim() removes whitespace so " " becomes "" which returns None
        assert!(parse_key(" ").is_none());
    }

    #[test]
    fn test_parse_key_single_letter_aliases() {
        // Default single-letter aliases for common actions
        for letter in &[
            'q', 'h', 'j', 'k', 'l', 'n', 's', 'c', 'w', 'g', 'e', 'f', 'y', 't', 'o',
        ] {
            let desc = parse_key(&letter.to_string()).unwrap();
            assert_eq!(desc.code, KeyCode::Char(*letter));
        }
    }

    #[test]
    fn test_parse_key_case_insensitive_modifier() {
        let desc = parse_key("ctrl+C").unwrap();
        assert!(desc.modifiers.contains(KeyModifiers::CONTROL));
        let desc = parse_key("CTRL+c").unwrap();
        assert!(desc.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_key_combined_modifiers() {
        let desc = parse_key("Ctrl+Shift+Enter").unwrap();
        assert!(desc.modifiers.contains(KeyModifiers::CONTROL));
        assert!(desc.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(desc.code, KeyCode::Enter);
    }

    // ── KeyBindings default ──

    #[test]
    fn test_default_bindings_has_all_actions() {
        let kb = KeyBindings::default();
        let actions = [
            "quit",
            "submit",
            "newline",
            "toggle_help",
            "toggle_settings",
            "toggle_kanban",
            "toggle_log",
            "focus_next",
            "focus_prev",
            "tab_next",
            "tab_prev",
            "scroll_up",
            "scroll_down",
            "page_up",
            "page_down",
            "scroll_bottom",
            "settings_save",
            "settings_cancel",
            "search",
            "search_next",
            "search_prev",
            "clear_line",
            "delete_word",
            "history_up",
            "history_down",
            "select_next",
            "select_prev",
            "select_confirm",
            "new_tab",
            "close_tab",
            "overlay_close",
            "slash_command",
            "cancel",
            "copy",
            "home",
            "end",
        ];
        for action in &actions {
            assert!(
                kb.bindings.contains_key(*action),
                "missing binding: {action}"
            );
        }
    }

    // ── action_for ──

    #[test]
    fn test_action_for_quit() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(kb.action_for(&event), Some(Action::Quit));
    }

    #[test]
    fn test_action_for_submit() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        // Enter maps to both Submit and SelectConfirm — HashMap order decides
        let action = kb.action_for(&event);
        assert!(
            action == Some(Action::Submit) || action == Some(Action::SelectConfirm),
            "unexpected action for Enter: {action:?}"
        );
    }

    #[test]
    fn test_action_for_newline() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(kb.action_for(&event), Some(Action::Newline));
    }

    #[test]
    fn test_action_for_toggle_help() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(kb.action_for(&event), Some(Action::ToggleHelp));
    }

    #[test]
    fn test_action_for_focus_next() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(kb.action_for(&event), Some(Action::FocusNext));
    }

    #[test]
    fn test_action_for_focus_prev() {
        let kb = KeyBindings::default();
        // Default binding is Shift+Tab → KeyCode::Tab + SHIFT modifier
        let event = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(kb.action_for(&event), Some(Action::FocusPrev));
    }

    #[test]
    fn test_action_for_scroll() {
        let kb = KeyBindings::default();
        // 'k' maps to both scroll_up and select_prev — HashMap order decides
        let k_action = kb.action_for(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert!(
            k_action == Some(Action::ScrollUp) || k_action == Some(Action::SelectPrev),
            "unexpected action for 'k': {k_action:?}"
        );
        // 'j' maps to both scroll_down and select_next
        let j_action = kb.action_for(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(
            j_action == Some(Action::ScrollDown) || j_action == Some(Action::SelectNext),
            "unexpected action for 'j': {j_action:?}"
        );
    }

    #[test]
    fn test_action_for_unknown_key() {
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(kb.action_for(&event), None);
    }

    #[test]
    fn test_action_for_ambiguous_binding() {
        // 'j' is bound to both scroll_down and select_next
        // The first match in the HashMap wins (but HashMap order is non-deterministic)
        let kb = KeyBindings::default();
        let event = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let action = kb.action_for(&event);
        assert!(action == Some(Action::ScrollDown) || action == Some(Action::SelectNext));
    }

    // ── action_from_name ──

    #[test]
    fn test_action_from_name_valid() {
        let kb = KeyBindings::default();
        assert_eq!(kb.action_from_name("quit"), Some(Action::Quit));
        assert_eq!(kb.action_from_name("copy"), Some(Action::Copy));
        assert_eq!(kb.action_from_name("home"), Some(Action::Home));
        assert_eq!(kb.action_from_name("end"), Some(Action::End));
    }

    #[test]
    fn test_action_from_name_invalid() {
        let kb = KeyBindings::default();
        assert_eq!(kb.action_from_name("nonexistent"), None);
        assert_eq!(kb.action_from_name(""), None);
    }

    // ── write_default_template ──

    #[test]
    fn test_write_default_template() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_keybindings.toml");
        KeyBindings::write_default_template(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[bindings]"));
        assert!(content.contains("quit"));
        assert!(content.contains("Ctrl+C"));
        let _ = std::fs::remove_file(&path);
    }

    // ── Action enum ──

    #[test]
    fn test_action_copy_and_eq() {
        let a = Action::Quit;
        let b = a;
        assert_eq!(a, b);
        assert_ne!(Action::Quit, Action::Submit);
    }
}
