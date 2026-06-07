// crates/tui/src/panels/settings.rs
// Multi-tab settings overlay — LLM, search, finance configuration with live preview.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{SettingsTab, TuiAppState};
use crate::theme;

/// The kinds of settings fields we render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Dropdown,
    Toggle,
    Text,
}

pub struct FieldDef {
    pub label: &'static str,
    pub kind: FieldKind,
}

impl FieldDef {
    pub fn rendered_value(&self, state: &TuiAppState) -> String {
        let s = &state.user_settings;
        match self.label {
            "提供商标识" => match s.llm_provider.as_str() {
                "anthropic" => "Anthropic".into(),
                "openai" => "OpenAI".into(),
                "deepseek" => "DeepSeek".into(),
                _ => {
                    if s.llm_provider.is_empty() {
                        "(未设置)".into()
                    } else {
                        s.llm_provider.clone()
                    }
                }
            },
            "模型名称" => {
                if s.llm_model.is_empty() {
                    "(使用默认)".into()
                } else {
                    s.llm_model.clone()
                }
            }
            "API Key" => crate::settings_store::UserSettings::mask_key(&s.llm_api_key),
            "Base URL" => {
                if s.llm_base_url.is_empty() {
                    "(使用默认)".into()
                } else {
                    s.llm_base_url.clone()
                }
            }
            "启用搜索" => {
                if s.search_enabled {
                    "● 已启用".into()
                } else {
                    "○ 已关闭".into()
                }
            }
            "搜索 Key" => crate::settings_store::UserSettings::mask_key(&s.search_api_key),
            "金融数据源" => match s.finance_provider.as_str() {
                "ftshare" | "" => "FTShare (默认优先)".into(),
                "tushare" => "TuShare + FTShare 备用".into(),
                "eastmoney" => "东方财富 + FTShare 备用".into(),
                "tencent" => "腾讯财经 + FTShare 备用".into(),
                "sina" => "新浪财经 + FTShare 备用".into(),
                _ => "FTShare (默认优先)".into(),
            },
            "TuShare Token" => {
                crate::settings_store::UserSettings::mask_key(&s.finance_tushare_token)
            }
            "预设主题" => state.theme_preset.clone(),
            "App ID" => {
                if s.feishu_app_id.is_empty() {
                    "(未设置)".into()
                } else {
                    s.feishu_app_id.clone()
                }
            }
            "App Secret" => crate::settings_store::UserSettings::mask_key(&s.feishu_app_secret),
            _ => "?".into(),
        }
    }
}

pub fn fields_for_tab(tab: SettingsTab) -> Vec<FieldDef> {
    match tab {
        SettingsTab::Llm => vec![
            FieldDef {
                label: "提供商标识",
                kind: FieldKind::Dropdown,
            },
            FieldDef {
                label: "模型名称",
                kind: FieldKind::Text,
            },
            FieldDef {
                label: "API Key",
                kind: FieldKind::Text,
            },
            FieldDef {
                label: "Base URL",
                kind: FieldKind::Text,
            },
        ],
        SettingsTab::Search => vec![
            FieldDef {
                label: "启用搜索",
                kind: FieldKind::Toggle,
            },
            FieldDef {
                label: "搜索 Key",
                kind: FieldKind::Text,
            },
        ],
        SettingsTab::Finance => vec![
            FieldDef {
                label: "金融数据源",
                kind: FieldKind::Dropdown,
            },
            FieldDef {
                label: "TuShare Token",
                kind: FieldKind::Text,
            },
        ],
        SettingsTab::Theme => vec![FieldDef {
            label: "预设主题",
            kind: FieldKind::Dropdown,
        }],
        SettingsTab::Feishu => vec![
            FieldDef {
                label: "App ID",
                kind: FieldKind::Text,
            },
            FieldDef {
                label: "App Secret",
                kind: FieldKind::Text,
            },
        ],
    }
}

pub fn render_settings(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    // Need area.height >= 10: inner = overlay_h-2, Layout needs 4 rows (3 fixed + 1 min)
    if area.width < 2 || area.height < 10 {
        return;
    }
    let overlay_w = 64.min(area.width.saturating_sub(4));
    let overlay_h = 22.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
    let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
    let overlay_area = Rect::new(x, y, overlay_w, overlay_h);

    // Dim background over full terminal area
    frame.render_widget(Clear, area);
    let bg_block = Block::default().style(Style::default().bg(theme::BG));
    frame.render_widget(bg_block, area);

    let dirty_mark = if state.settings_dirty { " *" } else { "" };
    let title = format!("Settings{}", dirty_mark);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL))
        .title(format!(" {title} "));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Length(1), // spacer
        Constraint::Min(1),    // field list
        Constraint::Length(1), // footer
    ])
    .split(inner);

    // ── Tab bar ──
    render_tab_bar(frame, chunks[0], state);

    // ── Field list ──
    let fields = fields_for_tab(state.settings_tab);
    render_field_list(frame, chunks[2], state, &fields);

    // ── Footer ──
    let hints = if state.settings_dirty_confirm {
        vec![
            Span::styled(
                " ⚠ 有未保存修改 ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::YELLOW)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                " 再按一次 Esc/s/q 放弃修改 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
            Span::styled(
                " 或 Ctrl+S 保存 ",
                Style::default().fg(theme::BG).bg(theme::GREEN),
            ),
        ]
    } else if state.settings_saved_flash > 0 {
        vec![Span::styled(
            " ✓ 设置已保存 ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GREEN)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]
    } else if state.settings_editing {
        vec![
            Span::styled(
                " Enter 确认 ",
                Style::default().fg(theme::BG).bg(theme::GREEN),
            ),
            Span::styled(
                " Esc 取消 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
            Span::styled(
                " Tab 切换页 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
        ]
    } else {
        let dirty_hint = if state.settings_dirty {
            vec![Span::styled(
                " Ctrl+S 保存 ",
                Style::default().fg(theme::BG).bg(theme::YELLOW),
            )]
        } else {
            vec![]
        };
        let mut base = vec![
            Span::styled(" ↑↓ 导航 ", Style::default().fg(theme::BG).bg(theme::CYAN)),
            Span::styled(
                " Tab 切换页 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
            Span::styled(
                " Enter 编辑 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
            Span::styled(
                " Space 切换 ",
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
            ),
        ];
        base.extend(dirty_hint);
        base.push(Span::styled(
            " Esc/s 关闭 ",
            Style::default().fg(theme::SUBTLE).bg(theme::PANEL),
        ));
        base
    };

    let hint_line = Line::from(hints);
    frame.render_widget(
        Paragraph::new(hint_line).style(Style::default().bg(theme::PANEL)),
        chunks[3],
    );
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let tabs = [
        SettingsTab::Llm,
        SettingsTab::Search,
        SettingsTab::Finance,
        SettingsTab::Feishu,
        SettingsTab::Theme,
    ];
    let spans: Vec<Span> = tabs
        .iter()
        .enumerate()
        .flat_map(|(i, t)| {
            let is_active = state.settings_tab == *t;
            let style = if is_active {
                Style::default().fg(theme::BG).bg(theme::CYAN)
            } else {
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL)
            };
            let label = format!(" {} ", t.label());
            if i > 0 {
                vec![
                    Span::styled(" ", Style::default().bg(theme::PANEL)),
                    Span::styled(label, style),
                ]
            } else {
                vec![Span::styled(label, style)]
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::PANEL)),
        area,
    );
}

fn render_field_list(frame: &mut Frame, area: Rect, state: &TuiAppState, fields: &[FieldDef]) {
    let lines: Vec<Line> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let is_focused = i == state.settings_field_focus;
            let label_style = if is_focused {
                Style::default().fg(theme::CYAN).bg(theme::PANEL)
            } else {
                Style::default().fg(theme::SUBTLE).bg(theme::PANEL)
            };

            let focus_marker = if is_focused { "▶ " } else { "  " };

            let value_text = if state.settings_editing && is_focused {
                state.settings_edit_buffer.clone()
            } else {
                f.rendered_value(state)
            };

            let value_style = if state.settings_editing && is_focused {
                Style::default().fg(theme::YELLOW).bg(theme::PANEL)
            } else {
                match f.kind {
                    FieldKind::Toggle => {
                        if value_text.contains('●') {
                            Style::default().fg(theme::GREEN).bg(theme::PANEL)
                        } else {
                            Style::default().fg(theme::SUBTLE).bg(theme::PANEL)
                        }
                    }
                    FieldKind::Dropdown => Style::default().fg(theme::TEXT).bg(theme::PANEL),
                    FieldKind::Text => Style::default().fg(theme::TEXT).bg(theme::PANEL),
                }
            };

            let kind_marker = match f.kind {
                FieldKind::Dropdown => " ▼",
                FieldKind::Toggle => "",
                FieldKind::Text => "",
            };

            let cursor = if state.settings_editing && is_focused {
                "│"
            } else {
                ""
            };

            Line::from(vec![
                Span::styled(focus_marker, label_style),
                Span::styled(format!("{}: ", f.label), label_style),
                Span::styled(format!("{value_text}{kind_marker}{cursor}"), value_style),
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::PANEL)),
        area,
    );
}
