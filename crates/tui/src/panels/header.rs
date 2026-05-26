// crates/tui/src/panels/header.rs
// Single-line header: agent name, turn number, active phase indicator.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, TuiAppState};
use crate::theme;

pub fn render_header(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let (phase_str, phase_color) = if state.agent_done {
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

    let spinner = match state.frame_count % 8 {
        0 => '⣾',
        1 => '⣽',
        2 => '⣻',
        3 => '⢿',
        4 => '⡿',
        5 => '⣟',
        6 => '⣯',
        7 => '⣷',
        _ => '⣾',
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
    let spinner_span = Span::styled(
        format!(" {} ", spinner),
        Style::default().fg(phase_color).bg(theme::BG),
    );
    let phase = Span::styled(
        phase_str.to_string(),
        Style::default()
            .fg(phase_color)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD),
    );

    let line = ratatui::text::Line::from(vec![
        left,
        Span::styled("  ", Style::default().bg(theme::BG)),
        turn,
        Span::styled("  ", Style::default().bg(theme::BG)),
        spinner_span,
        Span::styled("  ", Style::default().bg(theme::BG)),
        phase,
    ]);
    let para = Paragraph::new(line).style(Style::default().bg(theme::BG));

    frame.render_widget(para, area);
}
