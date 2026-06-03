// crates/tui/src/panels/context_ref.rs
// @-mention floating panel above input area.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{ContextRefItem, TuiAppState};
use crate::theme;

pub fn render_context_ref(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    if !state.context_ref_active || area.width < 6 {
        return;
    }
    if state.context_ref_items.is_empty() {
        return;
    }

    let popup_h = (state.context_ref_items.len() + 2).min(8) as u16;
    let popup_w = 50.min(area.width.saturating_sub(2));
    let y = area.y.saturating_sub(popup_h);

    let popup_area = Rect::new(area.x + 1, y, popup_w, popup_h);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL))
        .title(" @-引用 ");

    frame.render_widget(block, popup_area);

    let inner = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };

    let lines: Vec<Line> = state
        .context_ref_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == state.context_ref_selected;
            let source_icon = match item.source.as_str() {
                "file" => "F",
                "git" => "G",
                "mem" => "M",
                _ => "?",
            };
            let prefix = if is_sel { "> " } else { "  " };
            let bg = if is_sel { theme::PANEL_ALT } else { theme::PANEL };
            Line::from(vec![
                Span::styled(format!("{prefix}[{source_icon}] {}", item.label),
                    Style::default().fg(if is_sel { theme::CYAN } else { theme::TEXT }).bg(bg)),
                Span::styled(format!(" {}", item.preview),
                    Style::default().fg(theme::SUBTLE).bg(bg)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Populate suggestions based on query text.
pub fn populate_suggestions(state: &mut TuiAppState) {
    let query = state.context_ref_query.to_lowercase();
    state.context_ref_items.clear();

    // File suggestions
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten().take(10) {
            let name = entry.file_name().to_string_lossy().to_string();
            if query.is_empty() || name.to_lowercase().contains(&query) {
                if state.context_ref_items.len() < 5 {
                    state.context_ref_items.push(ContextRefItem {
                        source: "file".into(),
                        label: format!("file:{}", name),
                        preview: String::new(),
                    });
                }
            }
        }
    }

    // Git ref
    state.context_ref_items.push(ContextRefItem {
        source: "git".into(),
        label: "git:diff".into(),
        preview: "current changes".into(),
    });

    // Memory search
    if !query.is_empty() {
        state.context_ref_items.push(ContextRefItem {
            source: "mem".into(),
            label: format!("mem:{}", query),
            preview: "search memory...".into(),
        });
    }

    state.context_ref_selected = 0;
}
