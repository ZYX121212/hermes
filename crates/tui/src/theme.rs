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
        Self {
            colors: default_colors(),
        }
    }
}

impl Theme {
    pub fn load() -> Self {
        let path = std::env::var("HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(".hermess")
            .join("theme.toml");
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(t) = toml::from_str::<Theme>(&raw) {
                    tracing::info!(path = %path.display(), "Loaded theme");
                    return t;
                }
            }
        }
        if let Ok(name) = std::env::var("HERMESS_THEME") {
            return Self::preset(&name);
        }
        Self::preset("tokyo-night")
    }

    pub fn preset(name: &str) -> Self {
        match name {
            "dracula" => Self {
                colors: ThemeColors {
                    bg: "#282a36".into(),
                    panel: "#2c2f3e".into(),
                    panel_alt: "#353849".into(),
                    border: "#6272a4".into(),
                    border_focused: "#bd93f9".into(),
                    text: "#f8f8f2".into(),
                    muted: "#a0a0b0".into(),
                    subtle: "#6272a4".into(),
                    cyan: "#8be9fd".into(),
                    blue: "#6272a4".into(),
                    green: "#50fa7b".into(),
                    yellow: "#f1fa8c".into(),
                    red: "#ff5555".into(),
                    magenta: "#ff79c6".into(),
                },
            },
            "solarized-dark" => Self {
                colors: ThemeColors {
                    bg: "#002b36".into(),
                    panel: "#073642".into(),
                    panel_alt: "#0a4b57".into(),
                    border: "#586e75".into(),
                    border_focused: "#268bd2".into(),
                    text: "#839496".into(),
                    muted: "#586e75".into(),
                    subtle: "#657b83".into(),
                    cyan: "#2aa198".into(),
                    blue: "#268bd2".into(),
                    green: "#859900".into(),
                    yellow: "#b58900".into(),
                    red: "#dc322f".into(),
                    magenta: "#d33682".into(),
                },
            },
            "gruvbox" => Self {
                colors: ThemeColors {
                    bg: "#282828".into(),
                    panel: "#32302f".into(),
                    panel_alt: "#3c3836".into(),
                    border: "#504945".into(),
                    border_focused: "#83a598".into(),
                    text: "#ebdbb2".into(),
                    muted: "#a89984".into(),
                    subtle: "#665c54".into(),
                    cyan: "#83a598".into(),
                    blue: "#458588".into(),
                    green: "#b8bb26".into(),
                    yellow: "#fabd2f".into(),
                    red: "#fb4934".into(),
                    magenta: "#d3869b".into(),
                },
            },
            _ => Self::default(),
        }
    }

    pub fn preset_names() -> &'static [&'static str] {
        &["tokyo-night", "dracula", "solarized-dark", "gruvbox"]
    }

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

    pub fn bg(&self) -> Color {
        Self::parse(&self.colors.bg)
    }
    pub fn panel(&self) -> Color {
        Self::parse(&self.colors.panel)
    }
    pub fn panel_alt(&self) -> Color {
        Self::parse(&self.colors.panel_alt)
    }
    pub fn border(&self) -> Color {
        Self::parse(&self.colors.border)
    }
    pub fn border_focused(&self) -> Color {
        Self::parse(&self.colors.border_focused)
    }
    pub fn text(&self) -> Color {
        Self::parse(&self.colors.text)
    }
    pub fn muted(&self) -> Color {
        Self::parse(&self.colors.muted)
    }
    pub fn subtle(&self) -> Color {
        Self::parse(&self.colors.subtle)
    }
    pub fn cyan(&self) -> Color {
        Self::parse(&self.colors.cyan)
    }
    pub fn blue(&self) -> Color {
        Self::parse(&self.colors.blue)
    }
    pub fn green(&self) -> Color {
        Self::parse(&self.colors.green)
    }
    pub fn yellow(&self) -> Color {
        Self::parse(&self.colors.yellow)
    }
    pub fn red(&self) -> Color {
        Self::parse(&self.colors.red)
    }
    pub fn magenta(&self) -> Color {
        Self::parse(&self.colors.magenta)
    }
}

// ── Global theme ──

use std::sync::OnceLock;
static CURRENT_THEME: OnceLock<Theme> = OnceLock::new();

pub fn init_theme(theme: Theme) {
    let _ = CURRENT_THEME.set(theme);
}

pub fn current_theme() -> &'static Theme {
    CURRENT_THEME.get().unwrap_or(&*THEME_FALLBACK)
}

static THEME_FALLBACK: std::sync::LazyLock<Theme> = std::sync::LazyLock::new(|| Theme {
    colors: default_colors(),
});

// ── Backward-compatible color constants (match default tokyo-night theme) ──

/// 主背景色
pub const BG: Color = Color::Rgb(11, 18, 32);
/// 面板背景色
pub const PANEL: Color = Color::Rgb(15, 23, 42);
/// 交替面板背景色
pub const PANEL_ALT: Color = Color::Rgb(17, 31, 48);
/// 边框色
pub const BORDER: Color = Color::Rgb(51, 65, 85);
/// 焦点边框色（高亮蓝）
pub const BORDER_FOCUSED: Color = Color::Rgb(56, 189, 248);
/// 主文本色
pub const TEXT: Color = Color::Rgb(226, 232, 240);
/// 弱化文本色
pub const MUTED: Color = Color::Rgb(148, 163, 184);
/// 极弱文本色
pub const SUBTLE: Color = Color::Rgb(100, 116, 139);
/// 青色（信息提示）
pub const CYAN: Color = Color::Rgb(34, 211, 238);
/// 蓝色（链接/高亮）
pub const BLUE: Color = Color::Rgb(96, 165, 250);
/// 绿色（成功状态）
pub const GREEN: Color = Color::Rgb(52, 211, 153);
/// 黄色（警告状态）
pub const YELLOW: Color = Color::Rgb(251, 191, 36);
/// 红色（错误状态）
pub const RED: Color = Color::Rgb(248, 113, 113);
/// 紫色（特殊标记）
pub const MAGENTA: Color = Color::Rgb(216, 180, 254);

// ── Widget builders (unchanged from existing code) ──

pub fn panel_block(title: impl Into<String>, accent: Color, focused: bool) -> Block<'static> {
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
        Span::styled("\u{258c}", Style::default().fg(accent).bg(PANEL)),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Theme::default ──

    #[test]
    fn test_default_theme_colors() {
        let theme = Theme::default();
        assert_eq!(theme.colors.bg, "#0b1220");
        assert_eq!(theme.colors.text, "#e2e8f0");
        assert_eq!(theme.colors.cyan, "#22d3ee");
        assert!(theme.bg() != Color::Rgb(0, 0, 0)); // not black
    }

    // ── Theme::parse ──

    #[test]
    fn test_parse_valid_hex() {
        let color = Theme::parse("#ff0000");
        assert_eq!(color, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn test_parse_hex_without_hash() {
        let color = Theme::parse("00ff00");
        assert_eq!(color, Color::Rgb(0, 255, 0));
    }

    #[test]
    fn test_parse_short_hex_returns_white() {
        let color = Theme::parse("#fff");
        assert_eq!(color, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_parse_invalid_hex_returns_white() {
        let color = Theme::parse("#zzzzzz");
        assert_eq!(color, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_parse_empty_string() {
        let color = Theme::parse("");
        assert_eq!(color, Color::Rgb(255, 255, 255));
    }

    // ── Theme presets ──

    #[test]
    fn test_preset_tokyo_night() {
        let theme = Theme::preset("tokyo-night");
        assert_eq!(theme.colors.bg, "#0b1220");
    }

    #[test]
    fn test_preset_dracula() {
        let theme = Theme::preset("dracula");
        assert_eq!(theme.colors.bg, "#282a36");
        assert_eq!(theme.colors.red, "#ff5555");
    }

    #[test]
    fn test_preset_solarized_dark() {
        let theme = Theme::preset("solarized-dark");
        assert_eq!(theme.colors.bg, "#002b36");
        assert_eq!(theme.colors.blue, "#268bd2");
    }

    #[test]
    fn test_preset_gruvbox() {
        let theme = Theme::preset("gruvbox");
        assert_eq!(theme.colors.bg, "#282828");
        assert_eq!(theme.colors.yellow, "#fabd2f");
    }

    #[test]
    fn test_preset_unknown_falls_back_to_default() {
        let theme = Theme::preset("nonexistent-theme");
        assert_eq!(theme.colors.bg, "#0b1220"); // default
    }

    #[test]
    fn test_preset_names() {
        let names = Theme::preset_names();
        assert!(names.contains(&"tokyo-night"));
        assert!(names.contains(&"dracula"));
        assert!(names.contains(&"solarized-dark"));
        assert!(names.contains(&"gruvbox"));
        assert_eq!(names.len(), 4);
    }

    // ── Theme color accessors ──

    #[test]
    fn test_all_color_accessors() {
        let theme = Theme::default();
        // Verify all accessors return valid colors (no panics)
        let _ = theme.bg();
        let _ = theme.panel();
        let _ = theme.panel_alt();
        let _ = theme.border();
        let _ = theme.border_focused();
        let _ = theme.text();
        let _ = theme.muted();
        let _ = theme.subtle();
        let _ = theme.cyan();
        let _ = theme.blue();
        let _ = theme.green();
        let _ = theme.yellow();
        let _ = theme.red();
        let _ = theme.magenta();
    }

    // ── Global theme constants ──

    #[test]
    fn test_color_constants_are_set() {
        // All color constants should be valid non-zero colors (except potentially black)
        assert_ne!(BG, Color::Reset);
        assert_ne!(TEXT, Color::Reset);
        assert_ne!(CYAN, Color::Reset);
        assert_ne!(GREEN, Color::Reset);
        assert_ne!(RED, Color::Reset);
        assert_ne!(YELLOW, Color::Reset);
    }

    // ── Widget builders ──

    #[test]
    fn test_panel_block_does_not_panic() {
        // Builder methods should not panic
        let _block = panel_block("Test", CYAN, true);
        let _block2 = panel_block("Test", CYAN, false);
    }

    #[test]
    fn test_panel_block_focused_vs_unfocused() {
        // Both should construct without issues
        let _focused = panel_block("Test", CYAN, true);
        let _unfocused = panel_block("Test", CYAN, false);
    }

    #[test]
    fn test_title_line_is_non_empty() {
        let line = title_line("Header", CYAN);
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn test_empty_returns_styled_text() {
        let line = empty("No data");
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn test_key_returns_styled_span() {
        let span = key("Ctrl+C");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    // ── init_theme / current_theme ──

    #[test]
    fn test_init_and_current_theme() {
        let custom = Theme::preset("dracula");
        init_theme(custom);
        let current = current_theme();
        // OnceLock set works once — subsequent sets are no-ops,
        // so we just verify current_theme doesn't panic
        assert_eq!(current.colors.bg, "#282a36");
    }

    // ── Theme Deserialize (from TOML) ──

    #[test]
    fn test_theme_deserialize_from_toml() {
        let toml_str = r##"
[colors]
bg = "#111111"
panel = "#222222"
panel_alt = "#333333"
border = "#444444"
border_focused = "#555555"
text = "#eeeeee"
muted = "#cccccc"
subtle = "#999999"
cyan = "#00ffff"
blue = "#0000ff"
green = "#00ff00"
yellow = "#ffff00"
red = "#ff0000"
magenta = "#ff00ff"
"##;
        let theme: Theme = toml::from_str(toml_str).unwrap();
        assert_eq!(theme.colors.bg, "#111111");
        assert_eq!(theme.colors.red, "#ff0000");
    }
}
