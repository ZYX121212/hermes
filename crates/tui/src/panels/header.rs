// crates/tui/src/panels/header.rs
// Single-line header: agent name, turn number, active phase indicator.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AgentPhase, TuiAppState};

pub fn render_header(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let (phase_str, phase_color) = if state.agent_done {
        ("完成 — 按 q 退出", Color::Green)
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
            AgentPhase::Idle => Color::DarkGray,
            AgentPhase::Observing => Color::White,
            AgentPhase::Planning => Color::Cyan,
            AgentPhase::Executing => Color::Yellow,
            AgentPhase::Reflecting => Color::Magenta,
            AgentPhase::Evolving => Color::Green,
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
        format!("🜁 {} ", state.agent_name),
        Style::default().fg(Color::LightBlue),
    );
    let turn = Span::styled(
        format!("— 第 {} 轮 — ", state.turn),
        Style::default().fg(Color::White),
    );
    let spinner_span =
        Span::styled(format!("{} ", spinner), Style::default().fg(phase_color));
    let phase = Span::styled(phase_str, Style::default().fg(phase_color));

    let line = ratatui::text::Line::from(vec![
        left,
        turn,
        Span::raw("— "),
        spinner_span,
        phase,
    ]);
    let para = Paragraph::new(line);

    frame.render_widget(para, area);
}
