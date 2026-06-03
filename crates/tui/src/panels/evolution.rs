// crates/tui/src/panels/evolution.rs
// Evolution panel: collapsible strategy weights, insight stats, learning rate.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{clamp_scroll, render_scrollbar, wrapped_line_count, TuiAppState};
use crate::theme;

pub fn render_evolution(frame: &mut Frame, area: Rect, state: &TuiAppState, focused: bool) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let block = theme::panel_block("Evolution", theme::GREEN, focused);
    let inner = block.inner(area);
    let narrow = inner.width < 36;

    let stats = state.evolution.stats();
    let win_rate = stats.win_rate() * 100.0;
    let lr = state.evolution.current_learning_rate();
    let insight_count = state.evolution.insight_count();

    let mut lines: Vec<Line> = Vec::new();

    // ── Stats section ──
    let collapse_icon = if state.evo_stats_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 统计数据", collapse_icon),
        Style::default()
            .fg(theme::CYAN)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    )));

    if !state.evo_stats_hidden {
        if narrow {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Win {:.1}%  ", win_rate),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("+{} ", stats.positive),
                    Style::default().fg(theme::GREEN).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("-{}", stats.negative),
                    Style::default().fg(theme::RED).bg(theme::PANEL),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  Avg {:.3}", stats.avg_score),
                Style::default()
                    .fg(if stats.avg_score >= 0.0 {
                        theme::GREEN
                    } else {
                        theme::RED
                    })
                    .bg(theme::PANEL),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Best {:.3}  ", stats.best_score),
                    Style::default().fg(theme::GREEN).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("Worst {:.3}", stats.worst_score),
                    Style::default().fg(theme::RED).bg(theme::PANEL),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Win {:.1}%  ", win_rate),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("+{}  ", stats.positive),
                    Style::default().fg(theme::GREEN).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("-{}  ", stats.negative),
                    Style::default().fg(theme::RED).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("total {}", stats.total),
                    Style::default().fg(theme::MUTED).bg(theme::PANEL),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Avg {:.3}  ", stats.avg_score),
                    Style::default()
                        .fg(if stats.avg_score >= 0.0 {
                            theme::GREEN
                        } else {
                            theme::RED
                        })
                        .bg(theme::PANEL),
                ),
                Span::styled(
                    format!("Best {:.3}  ", stats.best_score),
                    Style::default().fg(theme::GREEN).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("Worst {:.3}", stats.worst_score),
                    Style::default().fg(theme::RED).bg(theme::PANEL),
                ),
            ]));
        }
    }

    lines.push(Line::from(Span::styled(
        "",
        Style::default().bg(theme::PANEL),
    )));

    // ── Strategy weights section ──
    let w_collapse_icon = if state.evo_weights_hidden {
        "▶"
    } else {
        "▼"
    };
    lines.push(Line::from(Span::styled(
        format!("{} 策略权重", w_collapse_icon),
        Style::default()
            .fg(theme::CYAN)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    )));

    if !state.evo_weights_hidden {
        let weights = state.evolution.all_weights();
        if weights.is_empty() {
            lines.push(Line::from(Span::styled(
                "  暂无数据",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            )));
        } else {
            let max_abs = weights
                .iter()
                .map(|(_, w)| w.abs())
                .fold(0.0_f64, f64::max)
                .max(1.0);

            let name_width = inner.width.saturating_sub(15).clamp(8, 14) as usize;
            let max_bar_width = inner
                .width
                .saturating_sub(name_width as u16 + 12)
                .clamp(1, 12) as usize;

            for (name, w) in weights.iter().take(24) {
                let bar_width = ((w.abs() / max_abs) * max_bar_width as f64).round() as usize;
                let bar = if *w >= 0.0 {
                    "█".repeat(bar_width)
                } else {
                    "░".repeat(bar_width)
                };
                let color = if *w >= 0.0 { theme::GREEN } else { theme::RED };
                let name_span = Span::styled(
                    format!(
                        "  {:<width$}",
                        crate::state::truncate(name, name_width),
                        width = name_width
                    ),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                );
                let bar_span = Span::styled(
                    format!(" {} {:+.2}", bar, w),
                    Style::default().fg(color).bg(theme::PANEL),
                );
                lines.push(Line::from(vec![name_span, bar_span]));
            }
        }
    }

    lines.push(Line::from(Span::styled(
        "",
        Style::default().bg(theme::PANEL),
    )));

    // ── Meta section ──
    let m_collapse_icon = if state.evo_meta_hidden { "▶" } else { "▼" };
    lines.push(Line::from(Span::styled(
        format!("{} 元信息", m_collapse_icon),
        Style::default()
            .fg(theme::CYAN)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    )));

    if !state.evo_meta_hidden {
        if narrow {
            lines.push(Line::from(Span::styled(
                format!("  LR {:.5}", lr),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            )));
            lines.push(Line::from(Span::styled(
                format!("  Insights {}", insight_count),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            )));
            lines.push(Line::from(Span::styled(
                format!("  Strategies {}", state.evolution.strategy_count()),
                Style::default().fg(theme::TEXT).bg(theme::PANEL),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  LR: {:.5}  ", lr),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("Insights: {}  ", insight_count),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                ),
                Span::styled(
                    format!("Strategies: {}", state.evolution.strategy_count()),
                    Style::default().fg(theme::TEXT).bg(theme::PANEL),
                ),
            ]));
        }
    }

    let content_text: String = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<Vec<&str>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content_height = wrapped_line_count(&content_text, inner.width.saturating_sub(1));
    let viewport_h = inner.height;

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL))
        .scroll((clamp_scroll(state.evo_scroll, content_height, viewport_h), 0));

    frame.render_widget(para, area);

    if content_height > viewport_h as usize {
        let bar = render_scrollbar(clamp_scroll(state.evo_scroll, content_height, viewport_h), content_height, viewport_h);
        let bar_lines: Vec<Line> = bar
            .chars()
            .map(|ch| {
                Line::from(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
                ))
            })
            .collect();
        let bar_rect = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y + 1,
            width: 1,
            height: viewport_h,
        };
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }
}
