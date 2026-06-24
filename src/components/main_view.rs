use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::markdown;
use super::Component;
use crate::app::{MainViewSubsection, ScrollMode, SelectionState};
use crate::message::{ChatMessage, ChatRole, ToolStatus};
use crate::selection;
use crate::theme::Theme;

/// 中央面板组件（消息列表 + 输入框）
///
/// Phase 4 RPC 模式：从 ANSI 缓冲区改为结构化 ChatMessage 列表渲染。
/// 输入框嵌入底部，支持文本输入和发送。
pub struct MainView {
    /// 消息列表
    pub messages: Vec<ChatMessage>,
    /// 滚动偏移
    pub scroll_offset: usize,
    /// 输入缓冲区
    pub input_buffer: String,
    /// 是否显示思考过程
    pub show_thinking: bool,
    /// 输入框是否获得焦点
    pub has_focus: bool,
    /// 当前聚焦的子区
    pub subsection: MainViewSubsection,
    /// 消息光标索引（指向 messages 数组）
    pub message_cursor: usize,
    /// 消息列表滚动模式
    pub scroll_mode: ScrollMode,
    /// 选区状态
    pub selection: SelectionState,
    /// 块光标（可交互块的扁平全局索引）
    pub block_cursor: usize,
    /// 块展开状态 key = "{msg_id}:{block_index}"
    pub block_states: std::collections::HashMap<String, crate::message::BlockExpanded>,
    /// 详情/进入视图
    pub entered_view: Option<crate::message::EnteredView>,
    /// 输入补全 popup
    pub completion_popup: Option<crate::message::CompletionPopup>,
}

impl Default for MainView {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            input_buffer: String::new(),
            show_thinking: false,
            has_focus: false,
            subsection: MainViewSubsection::Messages,
            message_cursor: 0,
            scroll_mode: ScrollMode::TailFollow,
            selection: SelectionState::default(),
            block_cursor: 0,
            block_states: std::collections::HashMap::new(),
            entered_view: None,
            completion_popup: None,
        }
    }
}

impl MainView {
    /// 添加用户消息（快捷方法）
    pub fn add_user_message(&mut self, agent_id: &str, text: &str) {
        self.messages.push(ChatMessage::user(agent_id, text));
    }

    /// 无消息时的等待状态（保留阶段三的空状态）
    fn render_empty(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        // 空状态也保留底部输入框，只是消息区域显示提示
        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg));
        f.render_widget(block, area);

        let inner = inset_content(area);
        let input_height = 2u16; // 输入提示行 + 空行
        let sep_height = 1u16; // 分割线
        let msg_area_height = inner.height.saturating_sub(input_height + sep_height);
        let sep_area = Rect::new(inner.x, inner.y + msg_area_height, inner.width, sep_height);
        let input_area = Rect::new(
            inner.x,
            inner.y + msg_area_height + sep_height,
            inner.width,
            input_height,
        );

        // 全宽分割线
        let sep = "─".repeat(inner.width.saturating_sub(1).max(1) as usize);
        f.render_widget(
            Paragraph::new(Line::from(sep.fg(theme.border_dim)))
                .style(Style::default().bg(theme.bg)),
            sep_area,
        );

        // 补全 popup（在输入框上方）
        self.render_completion_popup(f, input_area, theme);

        // 输入框
        let input_line = self.render_input_inner(theme);
        f.render_widget(Paragraph::new(input_line), input_area);
    }

    /// 渲染消息行
    /// 根据 block_cursor 判断每块的高亮状态
    fn render_message(
        message: &ChatMessage,
        theme: &Theme,
        main_view: &Self,
        msg_index: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let blocks = crate::message::build_block_refs(&main_view.messages);

        match message.role {
            ChatRole::User => {
                lines.push(Line::from(vec![" 你 ".to_string().fg(theme.accent).bold()]));
                // 分隔线和内容
                lines.push(Line::from(vec![" ─".to_string().fg(theme.text_dim)]));
                let clean = crate::strip_ansi(&message.text);
                let md_lines = markdown::render(&clean, theme);
                for md_line in md_lines {
                    let mut spans = vec![Span::from(" ")];
                    spans.extend(md_line.spans.into_iter());
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));
            }

            ChatRole::Assistant => {
                lines.push(Line::from(vec![" pi ".to_string().fg(theme.accent).bold()]));
                lines.push(Line::from(vec![" ─".to_string().fg(theme.text_dim)]));

                // 若 content 为空（ContentUpdate 尚未到达），回退到 text/thinking 字段
                if message.content.is_empty() {
                    if let Some(ref thinking) = message.thinking {
                        if !thinking.is_empty() {
                            let clean = crate::strip_ansi(thinking);
                            let preview: String =
                                clean.lines().take(3).collect::<Vec<_>>().join("\n");
                            lines.push(Line::from(vec![format!(" [思考] {}", preview)
                                .fg(theme.text_dim)
                                .add_modifier(Modifier::DIM)]));
                        }
                    }
                    let clean = crate::strip_ansi(&message.text);
                    let md_lines = markdown::render(&clean, theme);
                    for md_line in md_lines {
                        let mut spans = vec![Span::from(" ")];
                        spans.extend(md_line.spans.into_iter());
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(""));
                    return lines;
                }

                // 遍历 content 数组渲染内容块
                for (block_idx, block) in message.content.iter().enumerate() {
                    let key = format!("{}:{}", message.id, block_idx);
                    let state = main_view
                        .block_states
                        .get(&key)
                        .copied()
                        .unwrap_or(crate::message::BlockExpanded::Collapsed);

                    // 判断此块是否被光标选中
                    let is_block_selected = main_view.has_focus
                        && main_view.subsection == MainViewSubsection::Messages
                        && main_view.block_cursor < blocks.len()
                        && blocks[main_view.block_cursor].msg_index == msg_index
                        && blocks[main_view.block_cursor].block_index == block_idx;

                    match block {
                        crate::message::ContentBlock::Thinking { thinking } => match state {
                            crate::message::BlockExpanded::Collapsed => {
                                let clean = crate::strip_ansi(thinking);
                                let preview: String =
                                    clean.lines().take(1).collect::<Vec<_>>().join("\n");
                                let chars = clean.chars().count();
                                let text = format!(" ▶ [思考] {} 字 — {}", chars, preview);
                                let span = text.fg(theme.text_dim);
                                if is_block_selected {
                                    lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                                } else {
                                    lines.push(Line::from(vec![span]));
                                }
                            }
                            crate::message::BlockExpanded::Expanded => {
                                let clean = crate::strip_ansi(thinking);
                                let span = format!(" ▼ [思考] {} 字", clean.chars().count())
                                    .fg(theme.text_dim);
                                if is_block_selected {
                                    lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                                } else {
                                    lines.push(Line::from(vec![span]));
                                }
                                // 按 ANSI 解析渲染思考内容
                                let thinking_base = Style::default()
                                    .fg(theme.text_dim)
                                    .add_modifier(Modifier::DIM);
                                for think_line in thinking.lines() {
                                    let prefix = Span::from("   ").style(thinking_base);
                                    let mut spans = vec![prefix];
                                    spans
                                        .extend(crate::parse_ansi_spans(think_line, thinking_base));
                                    let mut line = Line::from(spans);
                                    if is_block_selected {
                                        line = line.bg(theme.highlight_bg);
                                    }
                                    lines.push(line);
                                }
                            }
                        },
                        crate::message::ContentBlock::Text { text } => {
                            let clean = crate::strip_ansi(text);
                            let md_lines = markdown::render(&clean, theme);
                            for md_line in md_lines {
                                let mut spans = vec![Span::from(" ")];
                                spans.extend(md_line.spans.into_iter());
                                lines.push(Line::from(spans));
                            }
                        }
                        crate::message::ContentBlock::ToolCall {
                            name,
                            arguments,
                            result,
                            is_error,
                            ..
                        } => {
                            let icon = if *is_error {
                                "✗"
                            } else if result.is_some() {
                                "✓"
                            } else {
                                "▶"
                            };
                            let args_summary = format_args_summary(arguments);
                            match state {
                                crate::message::BlockExpanded::Collapsed => {
                                    let text = format!(
                                        " {} {} {} {}",
                                        icon,
                                        name,
                                        args_summary,
                                        if *is_error { "(出错)" } else { "" }
                                    );
                                    let span = text.fg(theme.text_dim);
                                    if is_block_selected {
                                        lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                                    } else {
                                        lines.push(Line::from(vec![span]));
                                    }
                                }
                                crate::message::BlockExpanded::Expanded => {
                                    let text = format!(
                                        " {} {} {}",
                                        icon,
                                        name,
                                        if *is_error { "(出错)" } else { "" }
                                    );
                                    let span = text.fg(theme.text_dim);
                                    if is_block_selected {
                                        lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                                    } else {
                                        lines.push(Line::from(vec![span]));
                                    }
                                    // 显示参数摘要
                                    if let Ok(args_str) = serde_json::to_string_pretty(arguments) {
                                        for arg_line in args_str.lines().take(5) {
                                            let s = format!("   {}", arg_line).fg(theme.text_dim);
                                            if is_block_selected {
                                                lines.push(Line::from(vec![
                                                    s.bg(theme.highlight_bg)
                                                ]));
                                            } else {
                                                lines.push(Line::from(vec![s]));
                                            }
                                        }
                                    }
                                    // 显示结果摘要
                                    if let Some(ref r) = result {
                                        if let Some(content) =
                                            r.get("content").and_then(|c| c.as_str())
                                        {
                                            lines.push(Line::from(vec![
                                                "   ---".fg(theme.text_dim)
                                            ]));
                                            for out_line in content.lines().take(3) {
                                                let s =
                                                    format!("   {}", out_line).fg(theme.text_dim);
                                                if is_block_selected {
                                                    lines.push(Line::from(vec![
                                                        s.bg(theme.highlight_bg)
                                                    ]));
                                                } else {
                                                    lines.push(Line::from(vec![s]));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {} // Image 不渲染
                    }
                }
                lines.push(Line::from(""));
            }

            ChatRole::Tool => {
                if let Some(ref tool_call) = message.tool_call {
                    let icon = match tool_call.status {
                        ToolStatus::Running => "▶",
                        ToolStatus::Done => "✓",
                        ToolStatus::Error => "✗",
                    };
                    lines.push(Line::from(vec![format!(
                        " {} {} {}",
                        icon,
                        tool_call.tool_name,
                        " ".to_string().dim()
                    )
                    .fg(theme.text_dim)]));
                }
            }

            ChatRole::System => {
                let clean = crate::strip_ansi(&message.text);
                let md_lines = markdown::render(&clean, theme);
                for md_line in md_lines {
                    let mut spans = vec![Span::from(" ")];
                    spans.extend(md_line.spans.into_iter());
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));
            }

            ChatRole::Error => {
                let clean = crate::strip_ansi(&message.text);
                let md_lines = markdown::render(&clean, theme);
                for md_line in md_lines {
                    let mut spans = vec![Span::from(" ⚠ ")];
                    spans.extend(md_line.spans.into_iter());
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));
            }
        }

        lines
    }
}

impl Component for MainView {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        // entered_view 分支：全屏 Detail 视图
        if let Some(ref view) = self.entered_view {
            return self.render_entered(f, area, theme, view);
        }

        if self.messages.is_empty() && self.input_buffer.is_empty() {
            return self.render_empty(f, area, theme);
        }

        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg));
        f.render_widget(block, area);

        let inner = inset_content(area);

        // 计算可用行数（预留底部分割线 + 输入行）
        let input_height = 2u16; // 输入提示行 + 空行
        let sep_height = 1u16; // 分割线
        let msg_area_height = inner.height.saturating_sub(input_height + sep_height);
        let msg_area = Rect::new(inner.x, inner.y, inner.width, msg_area_height);
        let sep_area = Rect::new(inner.x, inner.y + msg_area_height, inner.width, sep_height);
        let input_area = Rect::new(
            inner.x,
            inner.y + msg_area_height + sep_height,
            inner.width,
            input_height,
        );

        // 渲染消息区域
        let mut all_lines: Vec<Line<'static>> = Vec::new();
        // 记录每条消息对应的行数范围（用于 Pinned 滚动模式）
        let mut msg_line_ranges: Vec<(usize, usize)> = Vec::new();
        for (i, msg) in self.messages.iter().enumerate() {
            let start = all_lines.len();
            all_lines.extend(Self::render_message(msg, theme, self, i));
            let end = all_lines.len();
            msg_line_ranges.push((start, end));
        }

        // 滚动：根据滚动模式决定显示区域
        let mut visible_lines: Vec<Line<'static>> = if all_lines.is_empty() {
            Vec::new()
        } else if self.scroll_mode == ScrollMode::Pinned
            && !msg_line_ranges.is_empty()
            && self.message_cursor < msg_line_ranges.len()
        {
            let (start_line, _end_line) = msg_line_ranges[self.message_cursor];
            let start = start_line.saturating_sub(msg_area_height as usize / 3);
            let start = start.min(all_lines.len().saturating_sub(msg_area_height as usize));
            all_lines[start..]
                .iter()
                .take(msg_area_height as usize)
                .cloned()
                .collect()
        } else if all_lines.len() > msg_area_height as usize {
            let start = all_lines.len().saturating_sub(msg_area_height as usize);
            all_lines[start..].to_vec()
        } else {
            all_lines
        };

        // 选区高亮
        selection::apply_selection(&mut visible_lines, msg_area, &mut self.selection, theme);

        let msg_paragraph = Paragraph::new(visible_lines)
            .fg(theme.text)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false });
        f.render_widget(msg_paragraph, msg_area);

        // 全宽分割线
        let sep = "─".repeat(inner.width.saturating_sub(1).max(1) as usize);
        f.render_widget(
            Paragraph::new(Line::from(sep.fg(theme.border_dim)))
                .style(Style::default().bg(theme.bg)),
            sep_area,
        );

        // 补全 popup（在输入框上方）
        self.render_completion_popup(f, input_area, theme);

        // 输入框
        let input_line = self.render_input_inner(theme);
        f.render_widget(Paragraph::new(input_line), input_area);
    }
}

impl MainView {
    /// 渲染详情视图（Diff / FullOutput / Subagent）
    fn render_entered(
        &self,
        f: &mut Frame,
        area: Rect,
        theme: &Theme,
        view: &crate::message::EnteredView,
    ) {
        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg));
        f.render_widget(block, area);

        match view {
            crate::message::EnteredView::Diff { path, diff_lines } => {
                self.render_diff(f, area, theme, path, diff_lines);
            }
            crate::message::EnteredView::FullOutput {
                title,
                content,
                scroll,
            } => {
                self.render_full_output(f, area, theme, title, content, *scroll);
            }
            crate::message::EnteredView::Subagent { agent_id, messages } => {
                // Subagent 递归渲染（简化版：显示消息列表）
                let title = format!("subagent: {}", agent_id);
                let content = messages
                    .iter()
                    .map(|m| {
                        format!(
                            "[{}] {}",
                            match m.role {
                                crate::message::ChatRole::User => "你",
                                crate::message::ChatRole::Assistant => "pi",
                                _ => "..",
                            },
                            m.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.render_full_output(f, area, theme, &title, &content, 0);
            }
        }
    }

    /// 渲染 Diff 视图
    fn render_diff(
        &self,
        f: &mut Frame,
        area: Rect,
        theme: &Theme,
        path: &str,
        diff_lines: &[crate::message::DiffLine],
    ) {
        let inner = inset_content(area);
        let plus_count = diff_lines.iter().filter(|l| l.kind == '+').count();
        let minus_count = diff_lines.iter().filter(|l| l.kind == '-').count();

        let mut lines: Vec<Line<'static>> = Vec::new();
        let header = format!("┌─ Diff: {} (+{} / -{}) ─", path, plus_count, minus_count);
        lines.push(Line::from(vec![header.fg(theme.text_dim)]));

        for dl in diff_lines {
            let prefix = match dl.kind {
                '-' => format!("  -  {:>4} │ ", dl.old_line.unwrap_or(0)),
                '+' => format!("  +  {:>4} │ ", dl.new_line.unwrap_or(0)),
                _ => format!("     {:>4} │ ", dl.old_line.unwrap_or(0)),
            };
            match dl.kind {
                '-' => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text).bg(theme.diff_red_bg),
                    ]));
                }
                '+' => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text).bg(theme.diff_green_bg),
                    ]));
                }
                _ => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text),
                    ]));
                }
            }
        }

        let footer = "└".to_string() + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
        lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

        // 裁剪到可见区域
        let visible: Vec<Line> = lines
            .into_iter()
            .take(inner.height.saturating_sub(1) as usize)
            .collect();

        let paragraph = Paragraph::new(visible)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }

    /// 渲染完整输出视图
    fn render_full_output(
        &self,
        f: &mut Frame,
        area: Rect,
        theme: &Theme,
        title: &str,
        content: &str,
        _scroll: usize,
    ) {
        let inner = inset_content(area);
        let mut lines: Vec<Line<'static>> = Vec::new();

        let header = format!("┌─ {} ─", title);
        lines.push(Line::from(vec![header.fg(theme.text_dim)]));

        let clean = crate::strip_ansi(content);
        let md_lines = markdown::render(&clean, theme);
        for md_line in md_lines {
            let mut spans = vec![Span::from(" ")];
            spans.extend(md_line.spans.into_iter());
            lines.push(Line::from(spans));
        }

        let footer = "└".to_string() + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
        lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

        let visible: Vec<Line> = lines
            .into_iter()
            .take(inner.height.saturating_sub(1) as usize)
            .collect();

        let paragraph = Paragraph::new(visible)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }

    /// 渲染补全 popup（在输入框上方）
    fn render_completion_popup(&self, f: &mut Frame, input_area: Rect, theme: &Theme) {
        if let Some(ref popup) = self.completion_popup {
            let popup_height = (popup.items.len().min(8) + 2) as u16;
            let popup_width = popup
                .items
                .iter()
                .map(|i| i.label.len() + 4)
                .max()
                .unwrap_or(20)
                .min(input_area.width as usize) as u16;
            let popup_area = Rect::new(
                input_area.x,
                input_area.y.saturating_sub(popup_height),
                popup_width,
                popup_height,
            );
            let mut popup_lines: Vec<Line> = Vec::new();
            for (i, item) in popup.items.iter().enumerate().take(8) {
                let prefix = if i == popup.cursor { "▶" } else { " " };
                let text = format!("{} {} {}", prefix, item.prefix, item.label);
                if i == popup.cursor {
                    popup_lines.push(Line::from(vec![text.bg(theme.highlight_bg)]));
                } else {
                    popup_lines.push(Line::from(vec![text.fg(theme.text_dim)]));
                }
            }
            f.render_widget(
                Paragraph::new(popup_lines)
                    .style(Style::default().bg(theme.bg))
                    .block(Block::default().borders(Borders::NONE)),
                popup_area,
            );
        }
    }

    /// 构建底部输入框内容行
    fn render_input_inner(&self, theme: &Theme) -> Line<'static> {
        let cursor = if self.has_focus { "█" } else { "" };
        let (prefix_style, text_style) = if self.has_focus {
            (theme.accent, theme.text)
        } else {
            (theme.text_dim, theme.text_dim)
        };

        if self.input_buffer.is_empty() {
            Line::from(vec![
                "> ".to_string().fg(prefix_style),
                cursor.to_string().fg(prefix_style),
                "输入你的问题...".to_string().fg(theme.text_dim),
            ])
        } else {
            Line::from(vec![
                "> ".to_string().fg(prefix_style),
                self.input_buffer.clone().fg(text_style),
                cursor.to_string().fg(prefix_style),
            ])
        }
    }
}

fn inset_content(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(4),
        area.height,
    )
}
/// 格式化工具调用参数为简短摘要
fn format_args_summary(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        let keys: Vec<String> = obj.keys().take(3).cloned().collect();
        if keys.is_empty() {
            String::new()
        } else {
            let summary = keys.join(", ");
            if obj.len() > 3 {
                format!("({}, …)", summary)
            } else {
                format!("({})", summary)
            }
        }
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ChatMessage, ToolCallInfo, ToolStatus};
    use crate::theme::Theme;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    #[test]
    fn test_empty_state() {
        let mv = MainView::default();
        assert!(mv.messages.is_empty());
        assert_eq!(mv.input_buffer, "");
    }

    #[test]
    fn test_add_user_message() {
        let mut mv = MainView::default();
        mv.add_user_message("test", "Hello");
        assert_eq!(mv.messages.len(), 1);
        assert_eq!(mv.messages[0].text, "Hello");
    }

    #[test]
    fn test_render_output() {
        let mut mv = MainView::default();
        mv.add_user_message("test", "Hello World");
        mv.input_buffer = String::from("test input");

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        // Should not panic - just verify we can render
        let buffer = terminal.backend().buffer();
        let has_text = buffer
            .content()
            .iter()
            .any(|c| c.symbol() == "H" || c.symbol() == "h");
        assert!(has_text, "Expected some rendered content");
    }

    #[test]
    fn test_tool_message() {
        let mut mv = MainView::default();
        let tool_info = ToolCallInfo {
            tool_name: "bash".into(),
            tool_call_id: "test-1".into(),
            status: ToolStatus::Done,
            args: json!({"command": "ls"}),
            result: Some(json!({"output": "file1.txt"})),
            detail_text: String::new(),
        };
        mv.messages.push(ChatMessage::tool("test", tool_info));
        assert_eq!(mv.messages.len(), 1);
    }

    #[test]
    fn test_user_input_buffer() {
        let mut mv = MainView::default();
        mv.input_buffer = "show me files".to_string();
        assert_eq!(mv.input_buffer, "show me files");
    }

    #[test]
    fn test_render_with_content_blocks() {
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![
            crate::message::ContentBlock::Thinking {
                thinking: "分析中...".to_string(),
            },
            crate::message::ContentBlock::Text {
                text: "这是回复".to_string(),
            },
        ];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        // Should not panic — verify we have rendered content blocks
        let buffer = terminal.backend().buffer();
        let has_text = buffer
            .content()
            .iter()
            .any(|c| c.symbol() == "▶" || c.symbol() == "分" || c.symbol() == "这");
        assert!(has_text, "Expected some rendered content from blocks");
    }
}
