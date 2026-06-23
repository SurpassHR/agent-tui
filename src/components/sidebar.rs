use ratatui::Frame;
use ratatui::layout::{Alignment, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::Component;
use crate::app::{SelectionState, SidebarSubsection, WorkspaceNode};
use crate::selection;
use crate::theme::Theme;

/// 左侧面板组件 — 三区独立布局
///
/// ┌─────────────────────────────┐
/// │ ● 首页重构讨论  sess_8a3f   │ ← 固定顶部
/// │                             │
/// │ 工作区                      │ ← 弹性中间（内部滚动）
/// │ ▼ 📁 agent-tui        (5)  │
/// │   ├─ ● 首页重构讨论  12条  │
/// │   ...                       │
/// ├─────────────────────────────┤
/// │ MODEL  deepseek-v4-flash    │ ← 固定底部
/// │ TOKENS  读:12.4K / 写:3.2K  │
/// │ TOTAL  22.3K                │
/// └─────────────────────────────┘
#[derive(Default)]
pub struct Sidebar {
    /// 当前活跃会话名称（人类可读）
    pub active_session: String,
    /// 当前活跃会话 ID（机器标识）
    pub session_id: String,
    /// 当前会话消息数量
    pub message_count: usize,
    // 工作区树数据（由 sync_components 每次渲染前写入）
    pub workspaces: Vec<WorkspaceNode>,
    // 焦点与导航
    pub has_focus: bool,
    pub cursor: usize,
    /// 当前聚焦的子区
    pub subsection: SidebarSubsection,
    /// 选区状态（由 tui.rs 在每帧渲染前写入）
    pub selection: SelectionState,
    // Provider 路由数据
    pub providers: Vec<crate::provider::ProviderInfo>,
    pub router_running: bool,
    pub current_model: String,
    pub provider_cursor: usize,
    pub model_cursor: usize,
    pub selecting_model: bool,
}

/// 底部 provider 区最小行数（动态扩展）
const FOOTER_LINES: u16 = 8;

impl Sidebar {
    /// 计算工作区的滚动偏移，使光标保持在可见区域内
    fn compute_ws_scroll(
        workspaces: &[WorkspaceNode],
        cursor: usize,
        has_focus: bool,
        ws_total: usize,
        ws_avail: usize,
    ) -> usize {
        if !has_focus || ws_total <= ws_avail {
            return 0;
        }

        // 计算光标在完整内容中的行位置
        let mut cursor_line: usize = 0;
        // 不计入 "工作区" 标题行（ws_total 包含它，但 cursor_line 从内容开始）
        // 第 0 行是 "工作区" 标题，光标实际内容从第 1 行开始
        // 但我们用 flat_idx 计数时已经跳过标题，所以这里 +1 对齐
        let mut flat_idx = 0;
        for ws in workspaces {
            if flat_idx == cursor {
                break;
            }
            flat_idx += 1;
            cursor_line += 1; // ws header 行
            if ws.expanded {
                for _ in &ws.sessions {
                    if flat_idx == cursor {
                        break;
                    }
                    flat_idx += 1;
                    cursor_line += 1;
                }
            }
        }
        // cursor_line 现在是对应 ws_lines 中的索引（0-based）
        // 但 ws_lines[0] 是 "工作区" 标题，所以实际光标行要 +1
        cursor_line += 1;

        // 简单策略：如果光标超出可见区，滚动使其出现在 1/3 位置
        if cursor_line >= ws_avail {
            cursor_line
                .saturating_sub(ws_avail / 3)
                .min(ws_total.saturating_sub(ws_avail))
        } else {
            0
        }
    }

    /// 活跃会话行
    fn render_active_session(&self, theme: &Theme) -> Line<'static> {
        let name = if self.active_session.is_empty() {
            "暂无活跃会话"
        } else {
            &self.active_session
        };
        let session_id = if self.session_id.is_empty() {
            String::new()
        } else {
            format!("  {}", &self.session_id[..self.session_id.len().min(12)])
        };
        Line::from(vec![
            Span::from("● ").fg(theme.success),
            Span::from(name.to_string()).fg(theme.success).bold(),
            Span::from(session_id).fg(theme.border_dim),
            Span::from(format!("  {}条", self.message_count)).fg(theme.text_dim),
        ])
    }

    /// 生成工作区树的所有 Line（不含头部 "工作区" 标题）
    fn render_workspace_tree(&self, theme: &Theme) -> (Vec<Line<'static>>, usize, usize) {
        let mut lines = Vec::new();
        let mut ws_count = 0;
        let mut sess_count = 0;
        let mut flat_idx = 0;

        for ws in &self.workspaces {
            ws_count += 1;
            sess_count += ws.sessions.len();

            let is_header = self.has_focus && self.cursor == flat_idx;
            flat_idx += 1;

            let chevron = if ws.expanded { "▼ " } else { "▶ " };
            let count_str = ws.sessions.len().to_string();

            let mut ws_line = Line::from(vec![
                Span::from(chevron).fg(theme.border_dim),
                Span::from("📁 ").fg(theme.text_dim),
                Span::from(ws.name.clone()).fg(theme.text),
                Span::from(format!(" ({})", count_str)).fg(theme.border_dim),
            ]);
            if is_header {
                ws_line = ws_line.patch_style(
                    ratatui::style::Style::default().bg(theme.highlight_bg).fg(theme.selection_fg),
                );
            }
            lines.push(ws_line);

            // 展开的 sessions
            if ws.expanded {
                for (si, session) in ws.sessions.iter().enumerate() {
                    let is_sess = self.has_focus && self.cursor == flat_idx;
                    flat_idx += 1;

                    let prefix = if si == ws.sessions.len() - 1 {
                        "  └─ "
                    } else {
                        "  ├─ "
                    };
                    let dot = if session.is_online { "●" } else { "○" };
                    let dot_color = if session.is_online {
                        theme.success
                    } else {
                        theme.text_dim
                    };
                    let msg_str = format!("{}条", session.message_count);

                    let sess_name_color = if session.id == self.active_session {
                        theme.accent
                    } else {
                        theme.text
                    };

                    let mut sess_line = Line::from(vec![
                        Span::from(prefix).fg(theme.border_dim),
                        Span::from(dot.to_string()).fg(dot_color),
                        Span::from(" "),
                        Span::from(session.name.clone()).fg(sess_name_color),
                        Span::from(format!(" {:>4}", msg_str)).fg(theme.border_dim),
                    ]);
                    if is_sess {
                        sess_line = sess_line.patch_style(
                            ratatui::style::Style::default().bg(theme.highlight_bg).fg(theme.selection_fg),
                        );
                    } else {
                        let sess_style = ratatui::style::Style::default()
                            .fg(if session.id == self.active_session { theme.accent } else { theme.text })
                            .bg(theme.bg);
                        sess_line = sess_line.style(sess_style);
                    }
                    lines.push(sess_line);
                }
            }
        }

        (lines, ws_count, sess_count)
    }

    /// 底部 Provider + MODEL 区块 — PROVIDER 在上，MODEL 跟随切换
    fn render_footer(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let sep = "─".repeat(width.saturating_sub(1).max(1) as usize);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let focused = self.has_focus && self.subsection == SidebarSubsection::Provider;

        // ── 全宽分隔线 ──
        lines.push(Line::from(vec![Span::from(sep.clone()).fg(theme.border_dim)]));

        // ── PROVIDER 区块 ──
        let status = if self.router_running && !self.providers.is_empty() {
            format!(" ◈ :8001")
        } else if self.router_running {
            " ◇ no providers".to_string()
        } else {
            " ◇ offline".to_string()
        };
        let is_on_providers = focused && !self.selecting_model;
        let p_title = if is_on_providers {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("PROVIDER ({})", self.providers.len()).fg(theme.accent).bold(),
                Span::from(status).fg(if self.router_running { theme.success } else { theme.text_dim }),
            ])
        } else {
            let mut s = vec![
                Span::from(" "),
                Span::from(format!("PROVIDER ({})", self.providers.len())).fg(theme.heading).bold(),
            ];
            if self.router_running {
                s.push(Span::from(status).fg(theme.success));
            }
            Line::from(s)
        };
        lines.push(p_title);

        if self.providers.is_empty() {
            lines.push(Line::from(" ○ 无配置".to_string().fg(theme.text_dim)));
            if is_on_providers {
                lines.push(Line::from(
                    Span::from(" [+] add provider").fg(theme.accent)
                ));
            } else {
                lines.push(Line::from(
                    Span::from(" [+] add provider").fg(theme.text_dim)
                ));
            }
        } else {
            for (i, p) in self.providers.iter().enumerate() {
                let is_active = if self.current_model.is_empty() {
                    i == 0
                } else {
                    p.models.iter().any(|m| m.id == self.current_model)
                };
                let is_provider_selected = is_on_providers && i == self.provider_cursor;
                let (fg, bg) = if is_provider_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else if is_active {
                    (theme.accent, theme.bg)
                } else {
                    (theme.text_dim, theme.bg)
                };
                let cnt = p.models.len();
                let mut text = if is_active {
                    format!("◆ {}  ({} models)", p.name, cnt)
                } else {
                    format!("○ {}  ({} models)", p.name, cnt)
                };
                if p.bridge { text.push_str(" 🔗"); }
                lines.push(Line::from(Span::from(text)).style(Style::default().fg(fg).bg(bg)));
            }
        }

        // Add provider hint (when focused)
        if is_on_providers {
            lines.push(Line::from(
                Span::from("  + add provider  [Space]").fg(theme.text_dim)
            ));
        }

        // ── MODEL 区块（独立跟随 active provider） ──
        let active_provider = if self.current_model.is_empty() {
            self.providers.first()
        } else {
            self.providers.iter().find(|p| p.models.iter().any(|m| m.id == self.current_model))
        };

        if let Some(ap) = active_provider {
            lines.push(Line::from(vec![Span::from(sep.clone()).fg(theme.border_dim)]));

            let is_on_models = focused && self.selecting_model;
            let m_title = if is_on_models {
                Line::from(vec![
                    "▎".to_string().fg(theme.accent),
                    format!("MODEL  {}", ap.id).fg(theme.accent).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
            } else {
                Line::from(vec![
                    Span::from(" MODEL  "),
                    Span::from(ap.id.clone()).fg(theme.accent).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
            };
            lines.push(m_title);

            for (mi, m) in ap.models.iter().enumerate() {
                let is_model_active = self.current_model == m.id;
                let is_model_selected = is_on_models && mi == self.model_cursor;
                let (mf, mb) = if is_model_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else if is_model_active {
                    (theme.success, theme.bg)
                } else {
                    (theme.text_dim, theme.bg)
                };
                let marker = if is_model_active { " ←" } else { "" };
                let ctx = if m.context_window >= 1_000_000 {
                    format!("{}M", m.context_window / 1_000_000)
                } else {
                    format!("{}K", m.context_window / 1000)
                };
                lines.push(Line::from(
                    Span::from(format!("  {} [{}] {} {}{}",
                        if is_model_active { "◆" } else { "○" },
                        m.tier, ctx, m.id, marker))
                ).style(Style::default().fg(mf).bg(mb)));
            }
        }

        lines
    }
}

impl Component for Sidebar {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        // ── 外层 RIGHT 边框（聚焦时使用 accent 色） ──
        let border_color = if self.has_focus {
            theme.accent
        } else {
            theme.border
        };
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_type(ratatui::widgets::BorderType::Plain)
            .border_style(border_color);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // ── 计算各区域高度 ──
        let top_h = 3u16; // ACTIVE SESSION 标题 + 会话行 + 空行
        let footer_h = FOOTER_LINES;
        let mid_h = inner.height.saturating_sub(top_h + footer_h).max(1);

        // 如果终端太矮，优先保留 top 和 mid
        let actual_footer_h = if inner.height < top_h + footer_h {
            inner.height.saturating_sub(top_h).min(footer_h)
        } else {
            footer_h
        };

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(top_h),
                ratatui::layout::Constraint::Length(mid_h),
                ratatui::layout::Constraint::Length(actual_footer_h),
            ])
            .split(inner);

        // ── 顶部：活跃会话 ──
        let top_title = if self.has_focus && self.subsection == SidebarSubsection::ActiveSession {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "ACTIVE SESSION".to_string().fg(theme.accent).bold(),
            ])
        } else {
            Line::from(" ACTIVE SESSION".to_string().fg(theme.heading).bold())
        };
        let top_lines = vec![
            top_title,
            self.render_active_session(theme),
            Line::from(""),
        ];

        // ── 中部：工作区树（带内部滚动） ──
        let mut ws_lines: Vec<Line<'static>> = Vec::new();
        let ws_title = if self.has_focus && self.subsection == SidebarSubsection::Workspace {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "工作区".to_string().fg(theme.accent).bold(),
            ])
        } else {
            Line::from(" 工作区".to_string().fg(theme.heading).bold())
        };
        ws_lines.push(ws_title);
        let (tree_lines, ws_count, sess_count) = self.render_workspace_tree(theme);
        if tree_lines.is_empty() {
            ws_lines.push(Line::from("  (暂无)".to_string().fg(theme.text_dim)));
        } else {
            ws_lines.extend(tree_lines);
            ws_lines.push(Line::from(
                format!("共 {} 个工作区，{} 个会话", ws_count, sess_count).fg(theme.border_dim),
            ));
        }

        let ws_avail = sections[1].height as usize;
        let ws_total = ws_lines.len();

        // 计算滚动偏移，使光标保持可见
        let scroll = Self::compute_ws_scroll(
            &self.workspaces,
            self.cursor,
            self.has_focus,
            ws_total,
            ws_avail,
        );

        let visible_ws: Vec<Line<'static>> =
            ws_lines.into_iter().skip(scroll).take(ws_avail).collect();

        // 选区高亮
        let mut top_para_lines = top_lines;
        selection::apply_selection(&mut top_para_lines, sections[0], &mut self.selection, theme);

        let top_para = Paragraph::new(top_para_lines)
            .fg(theme.text_dim)
            .style(Style::default().bg(theme.bg))
            .alignment(Alignment::Left);
        f.render_widget(top_para, sections[0]);

        // 将选区高亮应用到工作区树行
        let mut ws_para_lines = visible_ws;
        selection::apply_selection(&mut ws_para_lines, sections[1], &mut self.selection, theme);

        let ws_para = Paragraph::new(ws_para_lines)
            .fg(theme.text_dim)
            .style(Style::default().bg(theme.bg))
            .alignment(Alignment::Left);
        f.render_widget(ws_para, sections[1]);

        // ── 底部：模型 + Token（固定，不随工作区滚动） ──
        let footer_lines = self.render_footer(sections[2].width, theme);
        let visible_footer: Vec<Line<'static>> = footer_lines
            .into_iter()
            .take(sections[2].height as usize)
            .collect();
        let mut footer_lines = visible_footer;
        selection::apply_selection(&mut footer_lines, sections[2], &mut self.selection, theme);

        let footer_para = Paragraph::new(footer_lines)
            .fg(theme.text_dim)
            .style(Style::default().bg(theme.bg))
            .alignment(Alignment::Left);
        f.render_widget(footer_para, sections[2]);
    }
}
