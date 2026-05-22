// crates/tui/src/panels/evolution.rs
// Evolution panel: collapsible strategy weights, insight stats, learning rate.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::TuiAppState;

pub fn render_evolution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(" Evolution ", Style::default().fg(Color::Green)));

    let stats = state.evolution.stats();
    let win_rate = stats.win_rate() * 100.0;
    let lr = state.evolution.current_learning_rate();
    let insight_count = state.evolution.insight_count();

    let mut lines: Vec<Line> = Vec::new();

    // ── Stats section ──
    let collapse_icon = if state.evo_stats_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 统计数据", collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_stats_hidden {
        lines.push(Line::from(Span::styled(
            format!(
                "  Win Rate: {:.1}%  ({}+ / {}- / {} total)",
                win_rate, stats.positive, stats.negative, stats.total
            ),
            Style::default().fg(Color::White),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  Avg: {:.3}  ", stats.avg_score),
                Style::default().fg(if stats.avg_score >= 0.0 {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
            Span::styled(
                format!("Best: {:.3}  ", stats.best_score),
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                format!("Worst: {:.3}", stats.worst_score),
                Style::default().fg(Color::Red),
            ),
        ]));
    }

    lines.push(Line::from(Span::styled("", Style::default())));

    // ── Strategy weights section ──
    let w_collapse_icon = if state.evo_weights_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 策略权重", w_collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_weights_hidden {
        let weights = state.evolution.all_weights();
        if weights.is_empty() {
            lines.push(Line::from(Span::styled(
                "  暂无数据",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let max_abs = weights
                .iter()
                .map(|(_, w)| w.abs())
                .fold(0.0_f64, f64::max)
                .max(1.0);

            for (name, w) in weights.iter().take(24) {
                let bar_width = ((w.abs() / max_abs) * 12.0).round() as usize;
                let bar = if *w >= 0.0 {
                    "█".repeat(bar_width)
                } else {
                    "░".repeat(bar_width)
                };
                let color = if *w >= 0.0 {
                    Color::Green
                } else {
                    Color::Red
                };
                let name_span = Span::styled(
                    format!("  {:<14}", crate::state::truncate(name, 14)),
                    Style::default().fg(Color::White),
                );
                let bar_span = Span::styled(
                    format!(" {} {:+.3}", bar, w),
                    Style::default().fg(color),
                );
                lines.push(Line::from(vec![name_span, bar_span]));
            }
        }
    }

    lines.push(Line::from(Span::styled("", Style::default())));

    // ── Meta section ──
    let m_collapse_icon = if state.evo_meta_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 元信息", m_collapse_icon),
        Style::default().fg(Color::DarkGray),
    )));

    if !state.evo_meta_hidden {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  LR: {:.5}  ", lr),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Insights: {}  ", insight_count),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Strategies: {}", state.evolution.strategy_count()),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    let para = Paragraph::new(lines)
        .block(block)
        .scroll((state.evo_scroll, 0));

    frame.render_widget(para, area);
}
