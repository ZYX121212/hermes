// crates/tui/src/panels/header.rs
// Single-line header: agent name, turn number, active phase indicator, stats.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, TuiAppState};
use crate::theme;

pub fn render_header(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let (phase_str, phase_color) = if state.awaiting_input {
        ("等待输入 / Enter 提交", theme::CYAN)
    } else if state.agent_done {
        ("完成 / 按 q 退出", theme::GREEN)
    } else {
        let s = match state.phase {
            AgentPhase::Idle => "空闲",
            AgentPhase::Observing => "观察中...",
            AgentPhase::Planning => "规划中...",
            AgentPhase::Executing => "执行中...",
            AgentPhase::Reflecting => "反思中...",
            AgentPhase::Evolving => "进化中...",
        };
        let c = match state.phase {
            AgentPhase::Idle => theme::SUBTLE,
            AgentPhase::Observing => theme::TEXT,
            AgentPhase::Planning => theme::CYAN,
            AgentPhase::Executing => theme::YELLOW,
            AgentPhase::Reflecting => theme::MAGENTA,
            AgentPhase::Evolving => theme::GREEN,
        };
        (s, c)
    };

    let left = Span::styled(
        format!("Hermes · {} ", state.agent_name),
        Style::default()
            .fg(theme::CYAN)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD),
    );
    let turn = Span::styled(
        format!(" 第 {} 轮 ", state.turn),
        Style::default().fg(theme::MUTED).bg(theme::BG),
    );

    let mut spans = vec![left, Span::raw("  "), turn];

    // Spinner: only show when agent is actually working
    if state.awaiting_input {
        spans.push(Span::styled(
            " ▸ ",
            Style::default().fg(theme::CYAN).bg(theme::BG),
        ));
    } else if !state.agent_done && state.phase != AgentPhase::Idle {
        // Phase-aware spinner animation
        let spinner = match state.phase {
            AgentPhase::Planning => {
                const BRAILLE: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                BRAILLE[(state.frame_count as usize) % BRAILLE.len()]
            }
            AgentPhase::Executing => {
                if state.frame_count % 4 < 2 {
                    '▶'
                } else {
                    '▷'
                }
            }
            AgentPhase::Reflecting => {
                const DOTS: &[char] = &['◌', '◌', '◌'];
                DOTS[(state.frame_count as usize / 4) % DOTS.len()]
            }
            _ => {
                const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
                SPINNER[(state.frame_count as usize) % SPINNER.len()]
            }
        };
        spans.push(Span::styled(
            format!(" {} ", spinner),
            Style::default().fg(phase_color).bg(theme::BG),
        ));
    } else if state.agent_done {
        spans.push(Span::styled(
            " ✓ ",
            Style::default().fg(theme::GREEN).bg(theme::BG),
        ));
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        phase_str.to_string(),
        Style::default()
            .fg(phase_color)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD),
    ));

    // Elapsed time (approximate: ~30fps = 1s per 30 frames)
    if !state.agent_done && state.phase != AgentPhase::Idle {
        let secs = state.frame_count / 30;
        let elapsed = if secs < 60 {
            format!(" {secs}s ")
        } else {
            format!(" {}m{}s ", secs / 60, secs % 60)
        };
        spans.push(Span::styled(
            elapsed,
            Style::default().fg(theme::SUBTLE).bg(theme::BG),
        ));
    } else if let Some(ms) = state.total_duration_ms {
        let secs = ms as f64 / 1000.0;
        spans.push(Span::styled(
            format!(" {secs:.1}s "),
            Style::default().fg(theme::SUBTLE).bg(theme::BG),
        ));
    }

    // Token usage (from shared UsageTracker)
    if let Some(ref tracker) = state.usage_tracker {
        let snap = tracker.snapshot();
        let total_tokens = snap.prompt_tokens + snap.completion_tokens;
        if total_tokens > 0 {
            let token_str = if total_tokens >= 1_000_000 {
                format!(" {:.1}M tk ", total_tokens as f64 / 1_000_000.0)
            } else if total_tokens >= 1_000 {
                format!(" {}K tk ", total_tokens / 1_000)
            } else {
                format!(" {} tk ", total_tokens)
            };
            spans.push(Span::styled(
                token_str,
                Style::default().fg(theme::SUBTLE).bg(theme::BG),
            ));
            // Cost estimate
            if snap.estimated_cost_usd > 0.0 {
                let cost_str = if snap.estimated_cost_usd < 0.01 {
                    "<$0.01".to_string()
                } else {
                    format!("${:.2}", snap.estimated_cost_usd)
                };
                spans.push(Span::styled(
                    format!(" {} ", cost_str),
                    Style::default().fg(theme::YELLOW).bg(theme::BG),
                ));
            }
            // Token usage bar
            let context_window: u64 = 200_000; // Claude default context
            if snap.prompt_tokens > 0 {
                let ratio = (snap.prompt_tokens as f64 / context_window as f64).min(1.0);
                let bar_width = (ratio * 10.0).round() as usize;
                let bar_color = if ratio < 0.5 {
                    theme::GREEN
                } else if ratio < 0.8 {
                    theme::YELLOW
                } else {
                    theme::RED
                };
                let bar = format!(" {}{}", "█".repeat(bar_width), "░".repeat(10 - bar_width));
                spans.push(Span::styled(
                    format!(" {} ", bar),
                    Style::default().fg(bar_color).bg(theme::BG),
                ));
            }
        }
        spans.push(Span::raw(" "));
    }

    // Error count badge
    let error_count = state.log_entries.iter().filter(|e| e.is_error).count();
    if error_count > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(" {} err ", error_count),
            Style::default()
                .fg(theme::BG)
                .bg(theme::RED)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Gateway info (right-aligned via padding)
    if state.gateway_enabled {
        let gw_label = if state.gateway_models.is_empty() {
            "Gateway · 路由中".to_string()
        } else {
            format!("Gateway · {} models", state.gateway_models.len())
        };
        let gw_color = if state.shg_triggered {
            theme::YELLOW
        } else {
            theme::BLUE
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(" {} ", gw_label),
            Style::default()
                .fg(theme::BG)
                .bg(gw_color)
                .add_modifier(Modifier::BOLD),
        ));
        if let Some(ref route) = state.last_route_decision {
            let preview = crate::state::truncate(route, 40);
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("→ {} ", preview),
                Style::default().fg(theme::MUTED).bg(theme::BG),
            ));
        }
    }

    let line = ratatui::text::Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme::BG)),
        area,
    );
}
