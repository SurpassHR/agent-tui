use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::Component;
use crate::app::{
    AgentPanelSubsection, AgentStatus, McpInfo, SelectionState, SkillInfo, SubAgentInfo,
};
use crate::selection;
use crate::theme::Theme;

/// 右侧面板组件 — subagent 列表 + skill 列表
///
/// 显示当前 pi agent 可调用的所有 subagent 和可用 skills。
/// subagent 数据源：`~/.pi/agent/agents/*.md` 和 pi-subagents 内置定义。
/// skills 数据源：`/home/hr/.agents/skills/*/SKILL.md`
#[derive(Default)]
pub struct AgentPanel {
    pub active_agent: String,
    pub agent_status: Option<AgentStatus>,
    pub subagents: Vec<SubAgentInfo>,
    /// 是否有焦点
    pub has_focus: bool,
    /// 当前聚焦的子区
    pub subsection: AgentPanelSubsection,
    /// subagent 选中光标（0 = AGENTS title, 1..=N = subagent 列表项）
    pub cursor: usize,
    /// skill 列表
    pub skills: Vec<SkillInfo>,
    /// skill 选中光标（0 = SKILLS title, 1..=N = skill 列表项）
    pub skill_cursor: usize,
    /// MCP server 列表
    pub mcps: Vec<McpInfo>,
    /// MCP 选中光标（0 = MCPS title, 1..=N = MCP 列表项）
    pub mcp_cursor: usize,
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
        // 左右各留 1 列内边距
        let padded = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
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
        let agents_selected =
            self.has_focus && self.subsection == AgentPanelSubsection::Agents && self.cursor == 0;
        let agents_title = if agents_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("AGENTS ({})", count).fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == AgentPanelSubsection::Agents {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("AGENTS ({})", count).fg(theme.accent).bold(),
            ])
        } else {
            Line::from(vec![
                Span::from(" "),
                Span::from(format!("AGENTS ({})", count))
                    .fg(theme.heading)
                    .bold(),
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

        // subagent 列表（可选中，cursor 偏移: 0=title, i+1=item）
        for (i, sa) in self.subagents.iter().enumerate() {
            if sa.name == agent {
                continue;
            }
            let is_selected = self.has_focus
                && self.subsection == AgentPanelSubsection::Agents
                && i + 1 == self.cursor;
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

        // ── SKILLS ──
        lines.push(Line::from(""));
        let skill_count = self.skills.len();
        let skills_selected = self.has_focus
            && self.subsection == AgentPanelSubsection::Skills
            && self.skill_cursor == 0;
        let skills_title = if skills_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("SKILLS ({})", skill_count).fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == AgentPanelSubsection::Skills {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("SKILLS ({})", skill_count).fg(theme.accent).bold(),
            ])
        } else {
            Line::from(vec![
                Span::from(" "),
                Span::from(format!("SKILLS ({})", skill_count))
                    .fg(theme.heading)
                    .bold(),
            ])
        };
        lines.push(skills_title);

        if skill_count == 0 {
            lines.push(Line::from("○ 暂无".to_string().fg(theme.text_dim)));
        } else {
            let display_max = 6; // 最多显示 6 条，其余折叠
            for (i, sk) in self.skills.iter().enumerate() {
                if i >= display_max {
                    break;
                }
                let is_selected = self.has_focus
                    && self.subsection == AgentPanelSubsection::Skills
                    && i + 1 == self.skill_cursor;
                let (fg, bg) = if is_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else {
                    (theme.text_dim, theme.bg)
                };
                let style = Style::default().fg(fg).bg(bg);
                lines.push(
                    Line::from(vec![Span::from("○ "), Span::from(sk.name.clone())]).style(style),
                );
            }
            if skill_count > display_max {
                lines.push(Line::from(
                    format!("… 还有 {} 个", skill_count - display_max).fg(theme.text_dim),
                ));
            }
        }

        // ── MCPS ──
        lines.push(Line::from(""));
        let mcp_count = self.mcps.len();
        let mcps_selected =
            self.has_focus && self.subsection == AgentPanelSubsection::Mcps && self.mcp_cursor == 0;
        let mcps_title = if mcps_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("MCPS ({})", mcp_count).fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == AgentPanelSubsection::Mcps {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("MCPS ({})", mcp_count).fg(theme.accent).bold(),
            ])
        } else {
            Line::from(vec![
                Span::from(" "),
                Span::from(format!("MCPS ({})", mcp_count))
                    .fg(theme.heading)
                    .bold(),
            ])
        };
        lines.push(mcps_title);

        if mcp_count == 0 {
            lines.push(Line::from("○ 暂无".to_string().fg(theme.text_dim)));
        } else {
            for (i, mcp) in self.mcps.iter().enumerate() {
                let is_selected = self.has_focus
                    && self.subsection == AgentPanelSubsection::Mcps
                    && i + 1 == self.mcp_cursor;
                let (fg, bg) = if is_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else {
                    (theme.text_dim, theme.bg)
                };
                let style = Style::default().fg(fg).bg(bg);
                let detail = if mcp.command.is_empty() {
                    format!("  ({} tools)", mcp.tool_count)
                } else {
                    format!("  {}", mcp.command)
                };
                lines.push(
                    Line::from(vec![
                        Span::from("◈ "),
                        Span::from(mcp.name.clone()),
                        Span::from(detail).fg(theme.text_dim),
                    ])
                    .style(style),
                );
            }
        }

        // ── TASKS ──
        lines.push(Line::from(""));
        let tasks_selected = self.has_focus && self.subsection == AgentPanelSubsection::Tasks;
        let tasks_title = if tasks_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "TASKS".to_string().fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == AgentPanelSubsection::Tasks {
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

        selection::apply_selection(&mut lines, padded, &mut self.selection, theme);

        f.render_widget(
            Paragraph::new(lines)
                .fg(theme.text_dim)
                .style(Style::default().bg(theme.bg)),
            padded,
        );
    }
}
