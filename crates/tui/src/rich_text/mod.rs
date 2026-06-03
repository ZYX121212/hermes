// crates/tui/src/rich_text.rs
// Lightweight markdown-to-ratatui-Span renderer for single lines.

pub mod highlight;
pub mod latex;
pub mod table;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;

/// Render a single line of text with basic markdown formatting.
/// Supports: `inline code`, **bold**, *italic*.
pub fn render_markdown_line(text: &str, base_style: Style) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut pos = 0;

    while pos < len {
        // Inline code: `...`
        if chars[pos] == '`' {
            if let Some(end) = find_closing(&chars, pos + 1, '`') {
                let code: String = chars[pos + 1..end].iter().collect();
                spans.push(Span::styled(
                    format!(" {} ", code),
                    Style::default()
                        .fg(theme::YELLOW)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD),
                ));
                pos = end + 1;
                continue;
            }
            // Unmatched backtick: treat as plain text below
        }

        // Bold: **...**
        if pos + 1 < len && chars[pos] == '*' && chars[pos + 1] == '*' {
            if let Some(end) = find_closing_double(&chars, pos + 2, '*') {
                if end + 1 < len && chars[end + 1] == '*' {
                    let bold: String = chars[pos + 2..end].iter().collect();
                    spans.push(Span::styled(
                        bold,
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                    pos = end + 2;
                    continue;
                }
            }
            // Unmatched **: treat as plain text below
        }

        // Italic: *...* (single asterisk, not part of **)
        if chars[pos] == '*' && !(pos + 1 < len && chars[pos + 1] == '*') {
            if let Some(end) = find_closing(&chars, pos + 1, '*') {
                let italic: String = chars[pos + 1..end].iter().collect();
                spans.push(Span::styled(italic, base_style));
                pos = end + 1;
                continue;
            }
            // Unmatched *: treat as plain text below
        }

        // Plain text: advance to next marker
        let start = pos;
        while pos < len && chars[pos] != '`' && chars[pos] != '*' {
            pos += 1;
        }
        if pos > start {
            let plain: String = chars[start..pos].iter().collect();
            spans.push(Span::styled(plain, base_style));
        } else if pos < len {
            // We're at a marker that didn't match — emit it as plain text
            spans.push(Span::styled(chars[pos].to_string(), base_style));
            pos += 1;
        }
    }

    if spans.is_empty() {
        spans.push(Span::styled("", base_style));
    }

    Line::from(spans)
}

/// Render multiple lines, detecting code block fences.
/// Lines between ``` fences are rendered in MUTED style.
pub fn render_markdown_lines(text: &str, base_style: Style) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        if raw_line.trim().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        let style = if in_code_block {
            Style::default().fg(theme::MUTED).bg(theme::PANEL)
        } else {
            base_style
        };

        lines.push(render_markdown_line(raw_line, style));
    }

    lines
}

/// Find closing delimiter, returns Some(index) or None.
fn find_closing(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == target)
}

/// Find position just before a double delimiter (e.g. `**` → position of first `*`).
fn find_closing_double(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == target && i + 1 < chars.len() && chars[i + 1] == target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn test_style() -> Style {
        Style::default().fg(theme::TEXT).bg(theme::PANEL)
    }

    #[test]
    fn test_plain_text() {
        let line = render_markdown_line("hello world", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_inline_code() {
        let line = render_markdown_line("use `Box::new` here", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(" Box::new "));
    }

    #[test]
    fn test_bold() {
        let line = render_markdown_line("this is **bold** text", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "this is bold text");
    }

    #[test]
    fn test_unmatched_backtick_no_infinite_loop() {
        // This would have caused an infinite loop before the fix
        let line = render_markdown_line("unmatched `backtick", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "unmatched `backtick");
    }

    #[test]
    fn test_unmatched_asterisk_no_infinite_loop() {
        let line = render_markdown_line("lone * asterisk", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "lone * asterisk");
    }

    #[test]
    fn test_unmatched_double_asterisk_no_infinite_loop() {
        let line = render_markdown_line("unmatched ** bold", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "unmatched ** bold");
    }

    #[test]
    fn test_empty_line() {
        let line = render_markdown_line("", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "");
    }

    #[test]
    fn test_only_backtick() {
        let line = render_markdown_line("`", test_style());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "`");
    }

    #[test]
    fn test_code_block_fences() {
        let input = "normal\n```\ncode inside\n```\nafter";
        let lines = render_markdown_lines(input, test_style());
        // Should have 3 content lines (fence lines skipped)
        assert_eq!(lines.len(), 3);
        let line_texts: Vec<String> = lines.iter().map(|l| {
            l.spans.iter().map(|s| s.content.as_ref()).collect()
        }).collect();
        assert_eq!(line_texts[0], "normal");
        assert_eq!(line_texts[1], "code inside");
        assert_eq!(line_texts[2], "after");
    }
}
