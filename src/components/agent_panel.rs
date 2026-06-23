use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::Component;
use crate::app::{AgentPanelSubsection, AgentStatus, SelectionState, SubAgentInfo};
use crate::selection;
use crate::theme::Theme;

/// 右侧面板组件 — subagent 列表
///
/// 显示当前 pi agent 可调用的所有 subagent。
/// 数据源：`~/.pi/agent/agents/*.md` 和 pi-subagents 内置定义。
#[derive(Default)]
pub struct AgentPanel {
    pub active_agent: String,
    pub agent_status: Option<AgentStatus>,
    pub subagents: Vec<SubAgentInfo>,
    /// 是否有焦点
    pub has_focus: bool,
    /// 当前聚焦的子区
    pub subsection: AgentPanelSubsection,
    /// subagent 选中光标（仅索引 subagents 列表，不含 pi agent）
    pub cursor: usize,
    /// 选区状态
    pub selection: SelectionState,
}

impl AgentPanel {
    fn status_text(&self) -> (String, ratatui::style::Color) {
        match self.agent_status {
            Some(AgentStatus::Running) => {
                ("running".into(), ratatui::style::Color::Rgb(173, 255, 47))
            }
            Some(AgentStatus::Idle) => ("idle".into(), ratatui::style::Color::Rgb(173, 255, 47)),
            Some(AgentStatus::Starting) => {
                ("starting".into(), ratatui::style::Color::Rgb(255, 191, 92))
            }
            Some(AgentStatus::Error) => ("error".into(), ratatui::style::Color::Red),
            Some(AgentStatus::Closed) | None => {
                ("closed".into(), ratatui::style::Color::Rgb(132, 155, 156))
            }
        }
    }
}

impl Component for AgentPanel {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        // ── 边框：聚焦时使用 accent 色 ──
        let border_color = if self.has_focus {
            theme.accent
        } else {
            theme.border
        };
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(border_color);

        let inner = block.inner(area);
        f.render_widget(block, area);

        let agent = if self.active_agent.is_empty() {
            "pi"
        } else {
            &self.active_agent
        };
        let (status, status_color) = self.status_text();

        let mut lines: Vec<Line<'static>> = Vec::new();

        // ── AGENTS ──
        let count = self.subagents.len();
        let agents_title = if self.has_focus && self.subsection == AgentPanelSubsection::Agents {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("AGENTS ({})", count).fg(theme.accent).bold(),
            ])
        } else {
            Line::from(vec![
                Span::from(" "),
                Span::from(format!("AGENTS ({})", count)).fg(theme.heading).bold(),
            ])
        };
        lines.push(agents_title);

        // 当前 pi agent（始终高亮显示）
        lines.push(
            Line::from(vec![
                Span::from("● ").fg(status_color),
                Span::from(agent.to_string()).fg(theme.selection_fg),
                Span::from(format!("  {}", status)).fg(status_color),
            ])
            .style(Style::default().bg(theme.highlight_bg)),
        );

        // subagent 列表（可选中）
        for (i, sa) in self.subagents.iter().enumerate() {
            if sa.name == agent {
                continue;
            }
            let is_selected = self.has_focus
                && self.subsection == AgentPanelSubsection::Agents
                && i == self.cursor;
            let (fg, bg) = if is_selected {
                (theme.selection_fg, theme.highlight_bg)
            } else {
                (theme.text_dim, theme.bg)
            };
            let style = Style::default().fg(fg).bg(bg);
            lines.push(
                Line::from(vec![
                    Span::from("○ "),
                    Span::from(sa.name.clone()),
                    Span::from(format!("  {}", sa.description)),
                ])
                .style(style),
            );
        }

        // ── TASKS ──
        lines.push(Line::from(""));
        let tasks_title = if self.has_focus && self.subsection == AgentPanelSubsection::Tasks {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "TASKS".to_string().fg(theme.accent).bold(),
            ])
        } else {
            Line::from(vec![
                Span::from(" "),
                Span::from("TASKS").fg(theme.heading).bold(),
            ])
        };
        lines.push(tasks_title);
        lines.push(Line::from("○ 暂无任务".to_string().fg(theme.text_dim)));

        selection::apply_selection(&mut lines, inner, &mut self.selection, theme);

        f.render_widget(
            Paragraph::new(lines)
                .fg(theme.text_dim)
                .style(Style::default().bg(theme.bg)),
            inner,
        );
    }
}
