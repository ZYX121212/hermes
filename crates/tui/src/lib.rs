// crates/tui/src/lib.rs
// TUI terminal interface for Hermes Agent using ratatui + crossterm.

pub mod panels;
pub mod render;
pub mod run;
pub mod state;
pub mod theme;

pub use run::run_tui;
pub use state::TuiInput;
