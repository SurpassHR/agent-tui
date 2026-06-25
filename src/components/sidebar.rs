use ratatui::layout::{Alignment, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

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
    /// 工作区树选中光标（0 = WORKSPACE title, 1..=N = 树节点）
    pub cursor: usize,
    /// 当前聚焦的子区
    pub subsection: SidebarSubsection,
    /// 活跃会话区选中光标（0 = ACTIVE SESSION title）
    pub active_session_cursor: usize,
    /// 选区状态（由 tui.rs 在每帧渲染前写入）
    pub selection: SelectionState,
    // Provider 路由数据
    pub providers: Vec<crate::provider::ProviderInfo>,
    pub router_running: bool,
    pub current_model: String,
    /// Provider 选中光标（0 = PROVIDER title, 1..=N = provider 列表项，N+1 = [+] add）
    pub provider_cursor: usize,
    /// Model 选中光标（0 = MODEL title, 1..=N = model 列表项）
    pub model_cursor: usize,
    /// MODEL 子区搜索文本
    pub model_search: String,
    /// MODEL 子区滚动偏移
    pub model_scroll: usize,
    /// Provider Router 实际绑定的端口
    pub port: u16,
    /// 当前选中的 Provider 索引（Space 切换控制）
    pub active_provider_idx: Option<usize>,
    /// 最近一次渲染的子区布局 [活跃会话区, 工作区树, 分割线, PROVIDER+MODEL]
    pub layout_sections: [Rect; 4],
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            active_session: String::new(),
            session_id: String::new(),
            message_count: 0,
            workspaces: Vec::new(),
            has_focus: false,
            cursor: 0,
            subsection: SidebarSubsection::Workspace,
            active_session_cursor: 0,
            selection: SelectionState::default(),
            providers: Vec::new(),
            router_running: false,
            current_model: String::new(),
            provider_cursor: 0,
            model_cursor: 0,
            model_search: String::new(),
            model_scroll: 0,
            port: 0,
            active_provider_idx: None,
            layout_sections: [Rect::new(0, 0, 0, 0); 4],
        }
    }
}

/// 底部 provider 区最小行数（动态扩展）
const FOOTER_LINES: u16 = 8;

impl Sidebar {
    /// 计算工作区的滚动偏移，使光标保持在可见区域内
    /// cursor 0 = WORKSPACE title（滚动到顶），1..=N = 树节点
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
        // title 行始终在最顶部
        if cursor == 0 {
            return 0;
        }
        let cursor = cursor - 1; // 去掉 title 偏移

        // 计算光标在完整内容中的行位置
        let mut cursor_line: usize = 0;
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
    fn render_active_sessions(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut idx = 1usize;
        for ws in &self.workspaces {
            for s in &ws.sessions {
                if !s.is_online {
                    continue;
                }
                let is_selected = self.has_focus
                    && self.subsection == SidebarSubsection::ActiveSession
                    && self.active_session_cursor == idx;
                let (fg, bg) = if is_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else {
                    (theme.success, theme.bg)
                };
                let sid_short = &s.id[..s.id.len().min(12)];
                let msg_str = format!("{}条", s.message_count);
                lines.push(
                    Line::from(vec![
                        "● ".fg(theme.success),
                        Span::from(s.name.clone()).fg(fg).bold(),
                        Span::from(format!("  {}", sid_short)).fg(theme.border_dim),
                        Span::from(format!(" {:>4}", msg_str)).fg(theme.text_dim),
                    ])
                    .style(Style::default().bg(bg)),
                );
                idx += 1;
            }
        }
        if lines.is_empty() {
            lines.push(Line::from("○ 暂无活跃会话".to_string().fg(theme.text_dim)));
        }
        lines
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

            let is_header = self.has_focus && self.cursor == flat_idx + 1;
            flat_idx += 1;

            let chevron = if ws.expanded { "▼ " } else { "▶ " };
            let count_str = ws.sessions.len().to_string();

            let mut ws_line = Line::from(vec![
                Span::from(chevron).fg(theme.border_dim),
                Span::from("📁 ").fg(theme.text_dim),
                Span::from(ws.display_name.clone()).fg(theme.text),
                Span::from(format!(" ({})", count_str)).fg(theme.border_dim),
            ]);
            if is_header {
                ws_line = ws_line.patch_style(
                    ratatui::style::Style::default()
                        .bg(theme.highlight_bg)
                        .fg(theme.selection_fg),
                );
            }
            lines.push(ws_line);

            // 展开的 sessions
            if ws.expanded {
                for (si, session) in ws.sessions.iter().enumerate() {
                    let is_sess = self.has_focus && self.cursor == flat_idx + 1;
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
                            ratatui::style::Style::default()
                                .bg(theme.highlight_bg)
                                .fg(theme.selection_fg),
                        );
                    } else {
                        let sess_style = ratatui::style::Style::default()
                            .fg(if session.id == self.active_session {
                                theme.accent
                            } else {
                                theme.text
                            })
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
    fn render_footer(&self, width: u16, footer_height: u16, theme: &Theme) -> Vec<Line<'static>> {
        let sep = "─".repeat(width.saturating_sub(1).max(1) as usize);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let focused_on_providers = self.has_focus && self.subsection == SidebarSubsection::Provider;
        let focused_on_models = self.has_focus && self.subsection == SidebarSubsection::Model;

        // ── 全宽分隔线 ──
        lines.push(Line::from(vec![
            Span::from(sep.clone()).fg(theme.border_dim)
        ]));

        // ── PROVIDER 区块 ──
        let status = if self.router_running && !self.providers.is_empty() {
            " ◈ 已就绪".to_string()
        } else if self.router_running {
            " ◇ 无配置".to_string()
        } else {
            " ◇ 离线".to_string()
        };
        let is_on_providers = focused_on_providers;
        let provider_selected = is_on_providers && self.provider_cursor == 0;
        let p_title = if provider_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("PROVIDER ({})", self.providers.len())
                    .fg(theme.accent)
                    .bold(),
                Span::from(status).fg(if self.router_running {
                    theme.success
                } else {
                    theme.text_dim
                }),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if is_on_providers {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                format!("PROVIDER ({})", self.providers.len())
                    .fg(theme.accent)
                    .bold(),
                Span::from(status).fg(if self.router_running {
                    theme.success
                } else {
                    theme.text_dim
                }),
            ])
        } else {
            let mut s = vec![
                Span::from(" "),
                Span::from(format!("PROVIDER ({})", self.providers.len()))
                    .fg(theme.heading)
                    .bold(),
            ];
            if self.router_running {
                s.push(Span::from(status).fg(theme.success));
            }
            Line::from(s)
        };
        lines.push(p_title);

        if self.providers.is_empty() {
            // Empty state: title at cursor 0, add button at cursor 1
            if is_on_providers && self.provider_cursor == 1 {
                lines.push(
                    Line::from(Span::from("  ◆ [+] add provider").fg(theme.selection_fg))
                        .style(ratatui::style::Style::default().bg(theme.highlight_bg)),
                );
            } else {
                lines.push(Line::from(
                    Span::from("  ○ [+] add provider").fg(theme.text_dim),
                ));
            }
        } else {
            for (i, p) in self.providers.iter().enumerate() {
                let is_active = self.active_provider_idx == Some(i);
                let is_provider_selected = is_on_providers && i + 1 == self.provider_cursor;
                let (fg, bg) = if is_provider_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else if is_active && p.enabled {
                    (theme.success, theme.bg)
                } else {
                    (theme.text_dim, theme.bg)
                };
                let cnt = p.models.len();
                let icon = if !p.enabled {
                    "⊗"
                } else if is_active {
                    "◆"
                } else {
                    "○"
                };
                let mut text = format!("  {} {}  ({} models)", icon, p.name, cnt);
                if !p.endpoint_type.is_empty() && p.endpoint_type != "openai_compat" {
                    text.push_str(&format!(" [{}]", p.endpoint_type));
                }
                if p.bridge {
                    text.push_str(" 🔗");
                }
                lines.push(
                    Line::from(vec![Span::from(text)])
                        .style(ratatui::style::Style::default().fg(fg).bg(bg)),
                );
            }

            let is_add_selected =
                is_on_providers && self.provider_cursor == self.providers.len() + 1;
            let (fg, bg, prefix) = if is_add_selected {
                (theme.selection_fg, theme.highlight_bg, "◆")
            } else {
                (theme.text_dim, theme.bg, "○")
            };
            lines.push(
                Line::from(Span::from(format!("  {} [+] add provider", prefix)))
                    .style(ratatui::style::Style::default().fg(fg).bg(bg)),
            );
        }

        // ── MODEL 区块（独立跟随 active provider，支持搜索过滤 + 滚动） ──
        // 优先用 active_provider_idx 按索引定位，避免同名模型 ID 跨 Provider 误匹配
        let active_provider = self
            .active_provider_idx
            .and_then(|idx| self.providers.get(idx))
            .filter(|p| p.enabled)
            .or_else(|| {
                self.providers
                    .iter()
                    .filter(|p| p.enabled)
                    .find(|p| p.models.iter().any(|m| m.id == self.current_model))
            })
            .or_else(|| self.providers.iter().find(|p| p.enabled));

        if let Some(ap) = active_provider {
            lines.push(Line::from(vec![
                Span::from(sep.clone()).fg(theme.border_dim)
            ]));

            let is_on_models = focused_on_models;
            let model_selected = is_on_models && self.model_cursor == 0;
            let m_title = if model_selected {
                Line::from(vec![
                    "▎".to_string().fg(theme.accent),
                    format!("MODEL  {}", ap.id).fg(theme.accent).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
                .style(Style::default().bg(theme.highlight_bg))
            } else if is_on_models {
                Line::from(vec![
                    "▎".to_string().fg(theme.accent),
                    format!("MODEL  {}", ap.id).fg(theme.accent).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
            } else if focused_on_providers {
                // Provider 子区聚焦时：不带 ▎ 指示条，用 heading 色表明可达
                Line::from(vec![
                    Span::from(" MODEL  ").fg(theme.heading),
                    Span::from(ap.id.clone()).fg(theme.heading).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
            } else {
                Line::from(vec![
                    Span::from(" MODEL  ").fg(theme.heading),
                    Span::from(ap.id.clone()).fg(theme.accent).bold(),
                    Span::from(format!("  {} models", ap.models.len())).fg(theme.text_dim),
                ])
            };
            lines.push(m_title);

            // 搜索栏（非空时显示）
            if !self.model_search.is_empty() {
                lines.push(Line::from(
                    Span::from(format!("  🔍 {}", self.model_search)).fg(theme.accent),
                ));
            }

            // 过滤：只显示已开启 + 搜索匹配的模型
            let q = self.model_search.to_lowercase();
            let filtered: Vec<&crate::provider::ModelInfo> = ap
                .models
                .iter()
                .filter(|m| {
                    m.enabled && (self.model_search.is_empty() || m.id.to_lowercase().contains(&q))
                })
                .collect();

            // 已推入的行数 = 前面所有行（分隔线 + PROVIDER + MODEL 标题 + 可能的搜索栏）
            let pushed = lines.len();
            // 可用行数（保留一行给底栏之外的部分）
            let available = (footer_height as usize).saturating_sub(pushed);

            // 计算 scroll 偏移使 cursor 可见（cursor=0=title，无需滚动到模型）
            let mut scroll = self.model_scroll;
            let cursor = if self.model_cursor > 0 {
                (self.model_cursor - 1).min(filtered.len().saturating_sub(1))
            } else {
                scroll
            };
            if filtered.len() > available {
                if cursor < scroll {
                    scroll = cursor;
                } else if cursor >= scroll + available {
                    scroll = cursor.saturating_add(1).saturating_sub(available);
                }
            } else {
                scroll = 0;
            }

            // 取可见范围
            let visible: Vec<&&crate::provider::ModelInfo> =
                filtered.iter().skip(scroll).take(available).collect();

            for (vi, m) in visible.iter().enumerate() {
                let is_model_active = self.current_model == m.id;
                let is_model_selected = is_on_models && (scroll + vi + 1) == self.model_cursor;
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
                lines.push(
                    Line::from(Span::from(format!(
                        "  {} [{}] {} {}{}",
                        if is_model_active { "◆" } else { "○" },
                        m.tier,
                        ctx,
                        m.id,
                        marker
                    )))
                    .style(Style::default().fg(mf).bg(mb)),
                );
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
        // 左右各留 1 列内边距
        let padded = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        f.render_widget(block, area);

        // ── 先构建活跃会话区内容以确定高度 ──
        let active_sess_lines = self.render_active_sessions(theme);
        let top_h = 1u16 + active_sess_lines.len() as u16 + 1u16; // title + 会话行 + 空行
                                                                  // 统一用 14 行上限，footer 内容实际撑多高就是多高，不因子区切换跳动
        const MAX_FOOTER: u16 = 14;
        let max_footer = MAX_FOOTER;
        let provisional = self.render_footer(padded.width, max_footer, theme);
        let footer_content_h = (provisional.len() as u16).clamp(FOOTER_LINES, max_footer);
        let footer_h = if padded.height < top_h + FOOTER_LINES {
            padded.height.saturating_sub(top_h).min(FOOTER_LINES)
        } else {
            footer_content_h.min(padded.height.saturating_sub(top_h + 1))
        };
        let mid_h = padded.height.saturating_sub(top_h + footer_h).max(1);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(top_h),
                ratatui::layout::Constraint::Length(mid_h),
                ratatui::layout::Constraint::Length(footer_h),
            ])
            .split(padded);

        // ── 顶部：活跃会话 ──
        let active_selected = self.has_focus
            && self.subsection == SidebarSubsection::ActiveSession
            && self.active_session_cursor == 0;
        let top_title = if active_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "ACTIVE SESSION".to_string().fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == SidebarSubsection::ActiveSession {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "ACTIVE SESSION".to_string().fg(theme.accent).bold(),
            ])
        } else {
            Line::from(" ACTIVE SESSION".to_string().fg(theme.heading).bold())
        };
        let mut top_lines = vec![top_title];
        top_lines.extend(active_sess_lines);
        top_lines.push(Line::from(""));

        // ── 中部：工作区树（带内部滚动） ──
        let mut ws_lines: Vec<Line<'static>> = Vec::new();
        let ws_selected =
            self.has_focus && self.subsection == SidebarSubsection::Workspace && self.cursor == 0;
        let ws_title = if ws_selected {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "WORKSPACE".to_string().fg(theme.accent).bold(),
            ])
            .style(Style::default().bg(theme.highlight_bg))
        } else if self.has_focus && self.subsection == SidebarSubsection::Workspace {
            Line::from(vec![
                "▎".to_string().fg(theme.accent),
                "WORKSPACE".to_string().fg(theme.accent).bold(),
            ])
        } else {
            Line::from(" WORKSPACE".to_string().fg(theme.heading).bold())
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

        // ── 底部：模型 + Token（使用预渲染内容，精确匹配高度） ──
        let mut footer_lines: Vec<Line<'static>> = provisional
            .into_iter()
            .take(sections[2].height as usize)
            .collect();
        selection::apply_selection(&mut footer_lines, sections[2], &mut self.selection, theme);

        let footer_para = Paragraph::new(footer_lines)
            .fg(theme.text_dim)
            .style(Style::default().bg(theme.bg))
            .alignment(Alignment::Left);
        f.render_widget(footer_para, sections[2]);

        // 存储布局矩形供鼠标滚轮命中测试
        self.layout_sections = [
            sections[0],           // 活跃会话区
            sections[1],           // 工作区树
            Rect::new(0, 0, 0, 0), // 分割线（无滚动内容）
            sections[2],           // PROVIDER+MODEL 区
        ];
    }
}
