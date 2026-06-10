// crates/tui/src/rich_text/table.rs
// Markdown table parser -> ratatui Table widget.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table};

use crate::theme;

/// Parse a markdown table from lines and render as ratatui Table widget.
pub fn parse_markdown_table<'a>(lines: &[&str]) -> Option<Table<'a>> {
    if lines.len() < 2 {
        return None;
    }

    let header_cells = split_row(lines[0])?;
    let sep_cells = split_row(lines[1])?;

    if !sep_cells.iter().all(|c| {
        let t = c.trim();
        t.starts_with('-') || t.starts_with(":-")
    }) {
        return None;
    }

    let header_row = Row::new(
        header_cells
            .iter()
            .map(|c| {
                Cell::from(Span::styled(
                    c.trim().to_string(),
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect::<Vec<_>>(),
    )
    .style(Style::default().bg(theme::PANEL_ALT));

    let data_rows: Vec<Row> = lines[2..]
        .iter()
        .filter_map(|line| {
            let cells = split_row(line)?;
            Some(Row::new(
                cells
                    .iter()
                    .map(|c| {
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
        .map(|_| ratatui::layout::Constraint::Percentage(100 / column_count.max(1)))
        .collect();

    let table = Table::new(data_rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::PANEL)),
    );

    Some(table)
}

fn split_row(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    let cells: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();
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

/// Check if a line is a markdown table separator.
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
        // | not a table — only 1 pipe pair (start + end) but no internal pipes
        assert!(!is_table_header("| not a table"));
    }

    #[test]
    fn test_is_table_separator() {
        assert!(is_table_separator("| --- | --- |"));
        assert!(is_table_separator("| :--- | ---: |"));
        assert!(!is_table_separator("| A | B |"));
    }

    #[test]
    fn test_split_row() {
        let cells = split_row("| a | b | c |").unwrap();
        assert_eq!(cells, vec!["a", "b", "c"]);
        assert!(split_row("not a row").is_none());
    }

    #[test]
    fn test_split_row_empty_cell() {
        let cells = split_row("| a |  | c |").unwrap();
        assert_eq!(cells, vec!["a", "", "c"]);
    }

    #[test]
    fn test_split_row_single_cell() {
        let cells = split_row("| single |").unwrap();
        assert_eq!(cells, vec!["single"]);
    }

    #[test]
    fn test_split_row_no_leading_pipe() {
        assert!(split_row("a | b |").is_none());
    }

    #[test]
    fn test_split_row_no_trailing_pipe() {
        assert!(split_row("| a | b").is_none());
    }

    #[test]
    fn test_split_row_whitespace_trimmed() {
        let cells = split_row("|  hello  |  world  |").unwrap();
        assert_eq!(cells, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_markdown_table_valid() {
        let input = vec!["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"];
        // Table struct is private in ratatui, just verify Some is returned
        assert!(parse_markdown_table(&input).is_some());
    }

    #[test]
    fn test_parse_markdown_table_too_short() {
        assert!(parse_markdown_table(&["| A |"]).is_none());
        assert!(parse_markdown_table(&[]).is_none());
    }

    #[test]
    fn test_parse_markdown_table_bad_separator() {
        let input = vec!["| A | B |", "| x | y |"];
        assert!(parse_markdown_table(&input).is_none());
    }

    #[test]
    fn test_split_row_empty_inner() {
        // "||" → inner is "" → split gives [""] → not empty → Ok(vec![""])
        let cells = split_row("||").unwrap();
        assert_eq!(cells, vec![""]);
    }

    // ── Edge cases ──

    #[test]
    fn test_is_table_header_empty_string() {
        assert!(!is_table_header(""));
    }

    #[test]
    fn test_is_table_separator_empty_string() {
        assert!(!is_table_separator(""));
    }

    #[test]
    fn test_parse_table_alignment_colons() {
        let input = vec!["| A | B | C |", "| :--- | ---: | :---: |", "| 1 | 2 | 3 |"];
        assert!(parse_markdown_table(&input).is_some());
    }

    #[test]
    fn test_parse_table_three_columns() {
        let input = vec![
            "| Col A | Col B | Col C |",
            "| --- | --- | --- |",
            "| 1 | 2 | 3 |",
            "| 4 | 5 | 6 |",
        ];
        assert!(parse_markdown_table(&input).is_some());
    }

    #[test]
    fn test_parse_table_no_data_rows() {
        let input = vec!["| H1 | H2 |", "| --- | --- |"];
        assert!(parse_markdown_table(&input).is_some());
    }
}
