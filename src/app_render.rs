use crate::app::App;
use crate::components::Component;
use crate::state::*;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;

impl App {
    /// 渲染前同步所有组件数据
    pub(crate) fn sync_components(&mut self) {
        // 同步 forwarding_agent_tx（确保默认 pi 的事件转发到当前的 active_agent）
        if let Some(ref tx) = self.forwarding_agent_tx {
            let new_id = self
                .active_agent
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let _ = tx.send(new_id);
        }

        if let Some(agent_id) = &self.active_agent {
            if let Some(msgs) = self.messages.get(agent_id) {
                self.tui.main_view.messages = msgs.clone();
            }
        }
        // AGENTS 面板始终显示 "pi" 作为当前 agent 名称，不与 session ID 混淆
        self.tui.agent_panel.active_agent = "pi".to_string();
        self.tui.agent_panel.agent_status = Some(self.agent_status.clone());
        self.tui
            .agent_panel
            .subagents
            .clone_from(&self.tui.subagents);

        // 同步 sidebar 数据
        // ACTIVE SESSION 区域显示所有活跃会话的名称
        let has_real_name = !self.active_sessions.is_empty();
        let session_name = if has_real_name {
            let names: Vec<String> = self
                .active_sessions
                .iter()
                .filter_map(|sid| {
                    self.tui
                        .workspaces
                        .iter()
                        .find_map(|ws| ws.sessions.iter().find(|s| s.id == *sid))
                        .map(|s| s.name.clone())
                })
                .collect();
            if names.is_empty() {
                "No agent".to_string()
            } else {
                format!(
                    "{} 个活跃 ({}..)",
                    names.len(),
                    names.first().map(|n| n.as_str()).unwrap_or("")
                )
            }
        } else {
            "No agent".to_string()
        };
        self.tui.sidebar.active_session = session_name;
        if has_real_name {
            self.tui.sidebar.session_id.clone_from(&self.session.id);
        } else {
            self.tui.sidebar.session_id.clear();
        }
        // 同步工作区会话的 is_online 标记
        for ws in &mut self.tui.workspaces {
            for s in &mut ws.sessions {
                s.is_online = self.active_sessions.contains(&s.id);
            }
        }
        self.tui.sidebar.providers.clone_from(&self.tui.providers);
        self.tui.sidebar.router_running = self.tui.router_running;
        self.tui
            .sidebar
            .current_model
            .clone_from(&self.tui.current_model);
        // 同步思考级别到侧边栏
        self.tui.sidebar.thinking_level = self
            .runtime
            .thinking_level
            .clone()
            .unwrap_or_else(|| "high".into());
        self.tui.sidebar.port = self.tui.router_port;
        self.tui.sidebar.active_provider_idx = self.tui.active_provider_idx;
        // Clamp provider_cursor to valid range（0=title, 1..=N=providers, N+1=add）
        let max_provider = self.tui.providers.len() + 1;
        if self.tui.provider_cursor > max_provider {
            self.tui.provider_cursor = max_provider;
        }
        self.tui.sidebar.provider_cursor = self.tui.provider_cursor;
        self.tui.sidebar.model_cursor = self.tui.model_cursor;
        self.tui
            .sidebar
            .model_search
            .clone_from(&self.tui.model_search);
        self.tui.sidebar.message_count = self
            .active_agent
            .as_ref()
            .and_then(|id| self.messages.get(id))
            .map(|m| m.len())
            .unwrap_or(0);
        // 从 focus_panel 推导各组件 has_focus
        self.tui.sidebar.has_focus = self.tui.focus_panel == FocusPanel::Sidebar;
        self.tui.main_view.has_focus = self.tui.focus_panel == FocusPanel::MainView;
        self.tui.agent_panel.has_focus = self.tui.focus_panel == FocusPanel::AgentPanel;
        self.tui.sidebar.cursor = self.tui.sidebar_cursor;
        self.tui.sidebar.active_session_cursor = self.tui.active_session_cursor;
        // 同步子区到组件
        self.tui.sidebar.subsection = self.tui.sidebar_subsection;
        self.tui.main_view.subsection = self.tui.main_view_subsection;
        self.tui.agent_panel.subsection = self.tui.agent_panel_subsection;
        self.tui.agent_panel.cursor = self.tui.agent_cursor;
        self.tui.agent_panel.skills.clone_from(&self.tui.skills);
        self.tui.agent_panel.skill_cursor = self.tui.skill_cursor;
        self.tui.agent_panel.mcps.clone_from(&self.tui.mcps);
        self.tui.agent_panel.mcp_cursor = self.tui.mcp_cursor;
        self.tui.main_view.message_cursor = self.tui.message_cursor;
        self.tui.main_view.scroll_mode = self.tui.scroll_mode;
        // 同步工作区数据到 sidebar 组件
        self.tui.sidebar.workspaces.clone_from(&self.tui.workspaces);
        // 同步选区状态到所有组件
        let sel = self.tui.selection.clone();
        self.tui.sidebar.selection.clone_from(&sel);
        self.tui.main_view.selection.clone_from(&sel);
        self.tui.agent_panel.selection.clone_from(&sel);

        // 同步 Token / Context / Cost 到底栏
        self.tui.bottom_bar.context_percent = self.runtime.context_percent;
        self.tui.bottom_bar.context_window = self.runtime.context_window;
        self.tui.bottom_bar.cache_read = self.runtime.cache_read;
        self.tui.bottom_bar.cache_write = self.runtime.cache_write;
        self.tui.bottom_bar.cost = self.runtime.cost;
    }

    /// 渲染 TUI 界面
    pub fn render_tui(&mut self, f: &mut ratatui::Frame) {
        self.sync_components();
        let theme = &self.tui.theme;
        f.render_widget(
            ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(theme.bg)),
            f.area(),
        );

        // 垂直切分：Content / BottomBar
        let main_layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(1),
                ratatui::layout::Constraint::Length(2),
            ])
            .split(f.area());

        // 水平切分中间区域：固定会话栏 / 弹性内容区 / 固定 agent 栏
        let columns = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Length(40),
                ratatui::layout::Constraint::Min(40),
                ratatui::layout::Constraint::Length(40),
            ])
            .split(main_layout[0]);

        // 渲染各组件
        self.tui.sidebar.render(f, columns[0], theme);
        self.tui.main_view.render(f, columns[1], theme);
        self.tui.agent_panel.render(f, columns[2], theme);
        self.tui.bottom_bar.render(f, main_layout[1], theme);

        // Popup 浮层（最高 Z-index，最后渲染）
        self.tui.popup.render(f, columns[1], theme);

        // Provider 详情 Popup（最高 Z-index）
        if let Some(idx) = self.tui.provider_popup {
            if idx < self.tui.providers.len() {
                let p = &self.tui.providers[idx];
                let popup_area = crate::components::popup::centered_rect(65, 55, f.area());
                f.render_widget(ratatui::widgets::Clear, popup_area);
                let block = ratatui::widgets::Block::default()
                    .title(format!(" ◆ {} ", p.name))
                    .borders(ratatui::widgets::Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Plain)
                    .border_style(theme.border);
                let inner = block.inner(popup_area);
                f.render_widget(&block, popup_area);
                let mut ln: Vec<Line<'static>> = Vec::new();
                ln.push(Line::from(vec![
                    Span::from("  ID       ").fg(theme.text_dim),
                    Span::from(p.id.clone()).fg(theme.text),
                ]));
                ln.push(Line::from(vec![
                    Span::from("  Base URL ").fg(theme.text_dim),
                    Span::from(p.base_url.clone()).fg(theme.text_dim),
                ]));
                ln.push(Line::from(vec![
                    Span::from("  endpoint ").fg(theme.text_dim),
                    Span::from(p.endpoint_type.clone()).fg(theme.accent),
                ]));
                ln.push(Line::from(""));
                ln.push(Line::from(
                    Span::from(format!("  models ({})", p.models.len()))
                        .fg(theme.heading)
                        .bold(),
                ));
                for m in &p.models {
                    let ctx = if m.context_window >= 1_000_000 {
                        format!("{}M", m.context_window / 1_000_000)
                    } else {
                        format!("{}K", m.context_window / 1000)
                    };
                    ln.push(Line::from(
                        Span::from(format!("    o {:25} [{}] {:>6}", m.id, m.tier, ctx))
                            .fg(theme.text_dim),
                    ));
                }
                ln.push(Line::from(""));
                ln.push(Line::from(
                    Span::from("  [Enter] activate  [e] edit  [Esc] close").fg(theme.text_dim),
                ));
                f.render_widget(
                    ratatui::widgets::Paragraph::new(ln)
                        .style(ratatui::style::Style::default().bg(theme.bg)),
                    inner,
                );
            } else {
                self.tui.provider_popup = None;
            }
        }

        // ── Provider 编辑表单弹窗 ──
        if let Some(ref editor) = self.tui.provider_editor {
            if editor.model_mgr.is_none() {
                let term_area = f.area();
                let popup_w = ((term_area.width as u32 * 70) / 100) as u16;
                let inner_w = popup_w.saturating_sub(2);
                let box_w = inner_w.saturating_sub(8).max(24) as usize;
                let value_w = box_w.saturating_sub(4);
                let rule = "─".repeat(inner_w.saturating_sub(4).min(58) as usize);
                let field_border = "─".repeat(value_w);

                let icon = if editor.is_new { "＋" } else { "✎" };
                let title = if editor.is_new {
                    format!(" {} 添加 Provider ", icon)
                } else {
                    format!(" {} 编辑 Provider ", icon)
                };

                let mask_secret = |value: &str, width: usize| -> String {
                    if value.is_empty() {
                        String::new()
                    } else {
                        let mut chars = value.chars();
                        let head = chars.next().map(|c| c.to_string()).unwrap_or_default();
                        let hidden_len = chars.count().min(width.saturating_sub(2));
                        format!("{}{}▪", head, "•".repeat(hidden_len))
                    }
                };

                // ── 构建内容行 ──
                let mut ln: Vec<Line<'static>> = Vec::new();

                // ── 端点类型选择器 ──
                {
                    let f_ep = editor.field_focus == 6;
                    let ep_label = if f_ep {
                        " ▎端点类型".to_string()
                    } else {
                        "  端点类型".to_string()
                    };
                    ln.push(Line::from(
                        Span::from(ep_label)
                            .fg(if f_ep { theme.accent } else { theme.text_dim })
                            .bold(),
                    ));
                    let opts = [
                        ("openai_compat", "OpenAI 兼容"),
                        ("openai_responses", "OpenAI Responses"),
                        ("anthropic_messages", "Anthropic Messages"),
                        ("gemini", "Gemini"),
                    ];
                    for (val, label) in &opts {
                        let selected = *val == editor.draft.endpoint_type;
                        let (marker, fg) = if selected {
                            ("◆", theme.accent)
                        } else {
                            ("○", theme.text_dim)
                        };
                        ln.push(Line::from(
                            Span::from(format!("    {} {}", marker, label)).fg(fg),
                        ));
                    }
                }

                // 分隔线
                ln.push(Line::from(
                    Span::from(format!(" {}", rule)).fg(theme.border_dim),
                ));

                // 字段表单组
                let field_defs: [(usize, &str, bool); 4] = [
                    (0, "Provider ID", false),
                    (1, "显示名称", false),
                    (2, "Base URL", false),
                    (3, "API Key", true),
                ];
                for &(fi, label, secret) in &field_defs {
                    let f = editor.field_focus == fi;
                    let raw = match fi {
                        0 => editor.draft.id.as_str(),
                        1 => editor.draft.name.as_str(),
                        2 => editor.draft.base_url.as_str(),
                        3 => editor.draft.api_key.as_str(),
                        _ => "",
                    };
                    let display = if secret && !raw.is_empty() {
                        mask_secret(raw, value_w)
                    } else {
                        raw.to_string()
                    };
                    let cur = if f { "█" } else { " " };
                    let label_text = if f {
                        format!(" ▎{}", label)
                    } else {
                        format!("  {}", label)
                    };
                    ln.push(Line::from(
                        Span::from(label_text)
                            .fg(if f { theme.accent } else { theme.text_dim })
                            .bold(),
                    ));
                    // 上边框
                    ln.push(Line::from(vec![
                        Span::from("  ┌").fg(theme.border_dim),
                        Span::from(field_border.clone()).fg(theme.border_dim),
                        Span::from("┐").fg(theme.border_dim),
                    ]));
                    // 内容行
                    let value = pad_value(&format!("{}{}", display, cur), value_w);
                    ln.push(Line::from(vec![
                        Span::from("  │").fg(theme.border_dim),
                        Span::from(value).fg(if raw.is_empty() {
                            theme.border_dim
                        } else {
                            theme.text
                        }),
                        Span::from("│").fg(theme.border_dim),
                    ]));
                    // 下边框
                    ln.push(Line::from(vec![
                        Span::from("  └").fg(theme.border_dim),
                        Span::from(field_border.clone()).fg(theme.border_dim),
                        Span::from("┘").fg(theme.border_dim),
                    ]));
                }

                // 模型
                {
                    let f = editor.field_focus == 5;
                    let label_part = if f {
                        format!(" ▎{}", "模型")
                    } else {
                        format!("  {}", "模型")
                    };
                    let cur = if f { "█" } else { "" };
                    ln.push(Line::from(
                        Span::from(format!(
                            "{}（每行一个，格式：id:tier:contextWindow）",
                            label_part
                        ))
                        .fg(if f { theme.accent } else { theme.text_dim })
                        .bold(),
                    ));
                    let model_border = "─".repeat(value_w);
                    ln.push(Line::from(vec![
                        Span::from("  ┌").fg(theme.border_dim),
                        Span::from(model_border.clone()).fg(theme.border_dim),
                        Span::from("┐").fg(theme.border_dim),
                    ]));
                    if editor.models_fetching {
                        let value = pad_value(&format!("⏳ 拉取模型列表中...{}", cur), value_w);
                        ln.push(Line::from(vec![
                            Span::from("  │").fg(theme.border_dim),
                            Span::from(value).fg(theme.accent),
                            Span::from("│").fg(theme.border_dim),
                        ]));
                    } else if editor.models_text.is_empty() {
                        let value = pad_value(&format!("(空){}", cur), value_w);
                        ln.push(Line::from(vec![
                            Span::from("  │").fg(theme.border_dim),
                            Span::from(value).fg(theme.border_dim),
                            Span::from("│").fg(theme.border_dim),
                        ]));
                    } else if f {
                        for line in editor.models_text.lines().take(3) {
                            let value = pad_value(&format!("{}{}", line, cur), value_w);
                            ln.push(Line::from(vec![
                                Span::from("  │").fg(theme.border_dim),
                                Span::from(value).fg(theme.text),
                                Span::from("│").fg(theme.border_dim),
                            ]));
                        }
                    } else {
                        let first = editor.models_text.lines().next().unwrap_or("");
                        let cnt = editor.models_text.lines().count();
                        let t: String = first.chars().take(value_w.saturating_sub(10)).collect();
                        let s = if cnt > 1 {
                            format!("{}… (+{}行)", t, cnt - 1)
                        } else {
                            t.to_string()
                        };
                        let value = pad_value(&s, value_w);
                        ln.push(Line::from(vec![
                            Span::from("  │").fg(theme.border_dim),
                            Span::from(value).fg(theme.text_dim),
                            Span::from("│").fg(theme.border_dim),
                        ]));
                    }
                    ln.push(Line::from(vec![
                        Span::from("  └").fg(theme.border_dim),
                        Span::from(model_border).fg(theme.border_dim),
                        Span::from("┘").fg(theme.border_dim),
                    ]));
                }

                // 底部分隔 + 操作栏
                ln.push(Line::from(
                    Span::from(format!(" {}", rule)).fg(theme.border_dim),
                ));
                let hint_text = match editor.field_focus {
                    6 => "  [Space] 切换端点类型",
                    _ => "",
                };
                ln.push(Line::from(vec![
                    Span::from(" "),
                    Span::from("[✓ 保存]").fg(theme.heading).bold(),
                    Span::from("  "),
                    Span::from("[取消]").fg(theme.text),
                    Span::from(format!(
                        "{}  [Tab] 字段  [Enter] 保存  [Esc] 取消",
                        hint_text
                    ))
                    .fg(theme.text_dim),
                ]));

                // 动态高度
                let content_h = ln.len() as u16;
                let need_h = content_h.saturating_add(2);
                let max_h = term_area.height.saturating_sub(4);
                let popup_h = need_h.min(max_h).max(12);
                let v_layout = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Length(popup_h),
                    ratatui::layout::Constraint::Fill(1),
                ])
                .split(term_area);
                let h_layout = ratatui::layout::Layout::horizontal([
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Percentage(70),
                    ratatui::layout::Constraint::Fill(1),
                ])
                .split(v_layout[1]);
                let popup_area = h_layout[1];

                let block = ratatui::widgets::Block::default()
                    .title(title.clone())
                    .borders(ratatui::widgets::Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Plain)
                    .border_style(theme.border);
                let inner = block.inner(popup_area);

                f.render_widget(Clear, popup_area);
                f.render_widget(&block, popup_area);
                f.render_widget(
                    ratatui::widgets::Paragraph::new(ln)
                        .style(ratatui::style::Style::default().bg(theme.bg)),
                    inner,
                );
            }
        }

        // ── 模型管理弹窗（覆盖在表单之上）──
        if let Some(ref editor) = self.tui.provider_editor {
            if let Some(ref mgr) = editor.model_mgr {
                use ratatui::style::Style;
                use ratatui::widgets::{BorderType, Borders, Clear, List, ListItem, Paragraph};
                let ta = f.area();
                let mc = editor.draft.models.len();
                let lh = (mc.saturating_add(4) as u16).clamp(6, 24);
                let sh = lh.saturating_add(4).min(ta.height.saturating_sub(6));
                let vl = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Length(sh),
                    ratatui::layout::Constraint::Fill(1),
                ])
                .split(ta);
                let hl = ratatui::layout::Layout::horizontal([
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Percentage(55),
                    ratatui::layout::Constraint::Fill(1),
                ])
                .split(vl[1]);
                let sa = hl[1];
                if let Some(ref fe) = mgr.editor {
                    let eh = 14u16;
                    let ev = ratatui::layout::Layout::vertical([
                        ratatui::layout::Constraint::Fill(1),
                        ratatui::layout::Constraint::Length(eh),
                        ratatui::layout::Constraint::Fill(1),
                    ])
                    .split(sa);
                    let ehl = ratatui::layout::Layout::horizontal([
                        ratatui::layout::Constraint::Fill(1),
                        ratatui::layout::Constraint::Percentage(65),
                        ratatui::layout::Constraint::Fill(1),
                    ])
                    .split(ev[1]);
                    let ea = ehl[1];
                    let iw = ea.width.saturating_sub(2) as usize;
                    let vw = iw.saturating_sub(8).max(16);
                    let eb = "─".repeat(vw);
                    let mut el: Vec<Line> = Vec::new();
                    for &(fi, lbl) in &[(0, "ID"), (1, "Tier"), (2, "Context Window")] {
                        let act = fe.field_focus == fi;
                        let raw = match fi {
                            0 => fe.draft_id.as_str(),
                            1 => fe.draft_tier.as_str(),
                            2 => fe.draft_ctx.as_str(),
                            _ => "",
                        };
                        el.push(Line::from(
                            Span::from(if act {
                                format!(" ▎{}", lbl)
                            } else {
                                format!("  {}", lbl)
                            })
                            .fg(if act { theme.accent } else { theme.text_dim })
                            .bold(),
                        ));
                        el.push(Line::from(vec![
                            Span::from("  ┌").fg(theme.border_dim),
                            Span::from(eb.clone()).fg(theme.border_dim),
                            Span::from("┐").fg(theme.border_dim),
                        ]));
                        let cur = if act { "█" } else { "" };
                        el.push(Line::from(vec![
                            Span::from("  │").fg(theme.border_dim),
                            Span::from(pad_value(&format!("{}{}", raw, cur), vw)).fg(
                                if raw.is_empty() {
                                    theme.border_dim
                                } else {
                                    theme.text
                                },
                            ),
                            Span::from("│").fg(theme.border_dim),
                        ]));
                        el.push(Line::from(vec![
                            Span::from("  └").fg(theme.border_dim),
                            Span::from(eb.clone()).fg(theme.border_dim),
                            Span::from("┘").fg(theme.border_dim),
                        ]));
                    }
                    el.push(Line::from(Span::from(" ").fg(theme.border_dim)));
                    el.push(Line::from(vec![
                        Span::from(" [✓ 确认]").fg(theme.heading).bold(),
                        Span::from("  [取消]").fg(theme.text),
                        Span::from("  [Tab] 字段  [Enter] 确认  [Esc] 取消").fg(theme.text_dim),
                    ]));
                    let eb2 = ratatui::widgets::Block::default()
                        .title(" ✎ 编辑模型 ")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Plain)
                        .border_style(theme.border);
                    let ei = eb2.inner(ea);
                    f.render_widget(Clear, ea);
                    f.render_widget(&eb2, ea);
                    f.render_widget(Paragraph::new(el).style(Style::default().bg(theme.bg)), ei);
                } else {
                    let sw = sa.width.saturating_sub(2) as usize;
                    let svw = sw.saturating_sub(8).max(16);
                    let mut si: Vec<ListItem> = Vec::new();
                    for (i, m) in editor.draft.models.iter().enumerate() {
                        let sel = i == mgr.cursor;
                        let si2 = if sel { "▶" } else { " " };
                        let ck = if m.enabled { "☑" } else { "☐" };
                        let meta = format!("{} · {}", m.tier, m.context_window);
                        let mw = meta.chars().count().min(svw.saturating_sub(4));
                        let id =
                            m.id.chars()
                                .take(svw.saturating_sub(mw + 6))
                                .collect::<String>();
                        let pad = svw.saturating_sub(
                            unicode_width::UnicodeWidthStr::width(id.as_str())
                                .saturating_add(mw)
                                .saturating_add(4),
                        );
                        let ls = format!(
                            "{} {} {}{}{}{}",
                            si2,
                            ck,
                            id,
                            " ".repeat(pad),
                            meta,
                            if sel { " ◀" } else { "" }
                        );
                        let fc = if m.enabled {
                            if sel {
                                theme.accent
                            } else {
                                theme.text
                            }
                        } else {
                            theme.text_dim
                        };
                        let st = if sel {
                            Style::default().fg(fc).bg(theme.highlight_bg)
                        } else {
                            Style::default().fg(fc)
                        };
                        si.push(ListItem::new(Line::from(Span::from(ls).style(st))));
                    }
                    {
                        let is_a = mgr.cursor == mc;
                        let si2 = if is_a { "▶" } else { " " };
                        si.push(ListItem::new(Line::from(
                            Span::from(format!("{} + 添加模型", si2)).style(if is_a {
                                Style::default().fg(theme.accent).bg(theme.highlight_bg)
                            } else {
                                Style::default().fg(theme.accent)
                            }),
                        )));
                    }
                    {
                        let si2 = mc + 1;
                        let is_s = mgr.cursor == si2;
                        let si3 = if is_s { "▶" } else { " " };
                        si.push(ListItem::new(Line::from(
                            Span::from(format!("{} ✓ 保存并返回", si3))
                                .style(if is_s {
                                    Style::default().fg(theme.success).bg(theme.highlight_bg)
                                } else {
                                    Style::default().fg(theme.success)
                                })
                                .bold(),
                        )));
                    }
                    let ec = editor.draft.models.iter().filter(|m| m.enabled).count();
                    let sb2 = ratatui::widgets::Block::default()
                        .title(format!(" 管理模型  ({}/{}) ", ec, mc))
                        .borders(Borders::ALL)
                        .border_type(BorderType::Plain)
                        .border_style(theme.border);
                    let si2 = sb2.inner(sa);
                    f.render_widget(Clear, sa);
                    f.render_widget(&sb2, sa);
                    f.render_widget(List::new(si), si2);
                    if si2.height > 3 {
                        let fy = si2.y + si2.height - 1;
                        f.render_widget(
                            Paragraph::new(Line::from(
                                Span::from("  ↑↓ Space 开关  Enter 编辑/保存  Del 删除  Esc 返回")
                                    .fg(theme.text_dim),
                            )),
                            Rect::new(si2.x, fy, si2.width, 1),
                        );
                    }
                }
            }
        }

        // 渲染后将组件中收集的选中文本同步回主 selection
        //（apply_selection 修改的是组件的克隆，mouse up 读的是主 selection）
        // 按 panel 字段选择对应组件，避免跨栏误回写
        match self.tui.selection.panel {
            SelectionPanel::Sidebar => {
                if !self.tui.sidebar.selection.selected_text.is_empty() {
                    self.tui
                        .selection
                        .selected_text
                        .clone_from(&self.tui.sidebar.selection.selected_text);
                }
            }
            SelectionPanel::Content => {
                if !self.tui.main_view.selection.selected_text.is_empty() {
                    self.tui
                        .selection
                        .selected_text
                        .clone_from(&self.tui.main_view.selection.selected_text);
                }
            }
            SelectionPanel::AgentPanel => {
                if !self.tui.agent_panel.selection.selected_text.is_empty() {
                    self.tui
                        .selection
                        .selected_text
                        .clone_from(&self.tui.agent_panel.selection.selected_text);
                }
            }
            SelectionPanel::None => {}
        }

        // ═══ 工作区重命名/创建 Popup ═══
        if let Some(ref state) = self.tui.workspace_rename {
            use ratatui::layout::{Constraint, Direction, Layout};
            use ratatui::style::{Style, Stylize};
            use ratatui::widgets::{BorderType, Borders, Clear, Paragraph};

            let ta = f.area();
            let (title, hint) = match state {
                RenameState::CreateWorkspace => {
                    ("添加工作区", "输入项目目录路径后 Enter 确认，Esc 取消")
                }
                RenameState::CreateSession { .. } => {
                    ("新建会话", "输入名称后 Enter 确认，Esc 取消")
                }
                RenameState::RenameWorkspace { .. } => {
                    ("重命名工作区", "输入新名称后 Enter 确认，Esc 取消")
                }
                RenameState::RenameSession { .. } => (
                    "重命名会话",
                    "输入新名称后 Enter 确认 | Tab AI 自动命名 | Esc 取消",
                ),
            };

            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(4),
                    Constraint::Fill(1),
                ])
                .split(ta);
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(50),
                    Constraint::Fill(1),
                ])
                .split(v[1]);
            let pa = h[1];

            let block = ratatui::widgets::Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(theme.border);
            let inner = block.inner(pa);

            f.render_widget(Clear, pa);
            f.render_widget(&block, pa);

            let cursor = if self.tui.workspace_rename.is_some() {
                "█"
            } else {
                ""
            };
            let input_line = format!(" {}{} ", self.tui.rename_input, cursor);
            f.render_widget(
                Paragraph::new(Line::from(input_line.fg(theme.text)))
                    .style(Style::default().bg(theme.bg)),
                inner,
            );

            // 提示行
            f.render_widget(
                Paragraph::new(Line::from(hint.fg(theme.text_dim).dim()))
                    .style(Style::default().bg(theme.bg)),
                Rect::new(inner.x, inner.y + 1, inner.width, 1),
            );
        }

        // ═══ 删除确认弹窗 ═══
        if let Some(ref cd) = self.tui.confirm_delete {
            use ratatui::layout::{Alignment, Constraint, Direction, Layout};
            use ratatui::style::{Style, Stylize};
            use ratatui::widgets::{BorderType, Borders, Clear, Paragraph};

            let ta = f.area();
            let (title, desc_lines): (&str, Vec<String>) = match &cd.target {
                ConfirmDeleteTarget::Workspace { name, .. } => (
                    "⚠ 确认删除工作区",
                    vec![
                        format!("确定要删除工作区「{}」及其所有会话吗？", name),
                        "此操作不可撤销。".into(),
                    ],
                ),
                ConfirmDeleteTarget::Session { name, .. } => (
                    "⚠ 确认删除会话",
                    vec![
                        format!("确定要删除会话「{}」吗？", name),
                        "此操作不可撤销。".into(),
                    ],
                ),
            };

            // 动态高度：空行 + 描述行 + 空行 + 提示行 + 上下各 1 内边距
            let popup_height = (desc_lines.len() + 5) as u16;

            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(popup_height),
                    Constraint::Fill(1),
                ])
                .split(ta);
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(50),
                    Constraint::Fill(1),
                ])
                .split(v[1]);
            let pa = h[1];

            let block = ratatui::widgets::Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(theme.warning);
            let inner = block.inner(pa);

            f.render_widget(Clear, pa);
            f.render_widget(&block, pa);

            let mut content: Vec<Line<'static>> = vec![Line::from("")];
            for desc in &desc_lines {
                content.push(Line::from(desc.clone().fg(theme.text)));
            }
            content.push(Line::from(""));
            content.push(Line::from(
                "[Enter] 确认删除    [Esc] 取消".fg(theme.text_dim).dim(),
            ));
            f.render_widget(
                Paragraph::new(content)
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(theme.bg)),
                inner,
            );
        }
    }
}
