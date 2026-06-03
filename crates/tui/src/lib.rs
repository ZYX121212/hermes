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
