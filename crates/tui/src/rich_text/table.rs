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

    let table = Table::new(data_rows, widths)
        .header(header_row)
        .block(
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
        assert!(!is_table_header("| not a table"));
    }

    #[test]
    fn test_is_table_separator() {
        assert!(is_table_separator("| --- | --- |"));
        assert!(is_table_separator("| :--- | ---: |"));
    }

    #[test]
    fn test_split_row() {
        let cells = split_row("| a | b | c |").unwrap();
        assert_eq!(cells, vec!["a", "b", "c"]);
        assert!(split_row("not a row").is_none());
    }
}
