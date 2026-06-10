// crates/tui/src/rich_text/highlight.rs
// Syntax highlighting for fenced code blocks using syntect.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn ss() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn ts() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn to_ratatui_color(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

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

/// Detect language from fenced code block info string.
pub fn detect_language(lang: &str) -> Option<&str> {
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
        "" => None,
        _ => None,
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
            let highlighted = highlighter
                .highlight_line(line, ss())
                .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);

            let spans: Vec<Span> = highlighted
                .into_iter()
                .map(|(style, text)| {
                    let fg = to_ratatui_color(style.foreground);
                    Span::styled(
                        text.trim_end_matches('\n').to_string(),
                        Style::default()
                            .fg(fg)
                            .bg(bg)
                            .add_modifier(to_modifier(style.font_style)),
                    )
                })
                .collect();

            lines.push(Line::from(spans));
        }

        lines
    } else {
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
    fn test_detect_language_aliases() {
        assert_eq!(detect_language("py"), Some("python"));
        assert_eq!(detect_language("js"), Some("javascript"));
        assert_eq!(detect_language("typescript"), Some("typescript"));
        assert_eq!(detect_language("ts"), Some("typescript"));
        assert_eq!(detect_language("bash"), Some("bash"));
        assert_eq!(detect_language("sh"), Some("bash"));
        assert_eq!(detect_language("shell"), Some("bash"));
        assert_eq!(detect_language("yml"), Some("yaml"));
        assert_eq!(detect_language("md"), Some("markdown"));
        assert_eq!(detect_language("cpp"), Some("cpp"));
        assert_eq!(detect_language("c++"), Some("cpp"));
    }

    #[test]
    fn test_detect_language_case_insensitive() {
        assert_eq!(detect_language("RUST"), Some("rust"));
        assert_eq!(detect_language("Python"), Some("python"));
        assert_eq!(detect_language("JSON"), Some("json"));
    }

    #[test]
    fn test_highlight_rust_code() {
        let lines = highlight_code(
            "rust",
            "fn main() {\n    println!(\"hi\");\n}\n",
            crate::theme::PANEL,
        );
        assert!(!lines.is_empty());
        for line in &lines {
            assert!(!line.spans.is_empty());
        }
    }

    #[test]
    fn test_highlight_no_language() {
        let lines = highlight_code("", "plain text\nmore text\n", crate::theme::PANEL);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_highlight_unknown_language() {
        let lines = highlight_code(
            "unknown-lang",
            "some code\nmore code\n",
            crate::theme::PANEL,
        );
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(!line.spans.is_empty());
        }
    }

    #[test]
    fn test_highlight_empty_code() {
        let lines = highlight_code("rust", "", crate::theme::PANEL);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_highlight_single_line() {
        let lines = highlight_code("python", "print('hello')", crate::theme::PANEL);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].spans.is_empty());
    }

    #[test]
    fn test_to_modifier_bold() {
        use syntect::highlighting::FontStyle;
        let m = to_modifier(FontStyle::BOLD);
        assert!(m.contains(Modifier::BOLD));
        assert!(!m.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_to_modifier_combined() {
        use syntect::highlighting::FontStyle;
        let m = to_modifier(FontStyle::BOLD | FontStyle::ITALIC);
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::ITALIC));
    }
}
