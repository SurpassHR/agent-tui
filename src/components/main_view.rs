use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::Component;
use crate::app::{MainViewSubsection, SelectionState, ScrollMode};
use crate::message::{ChatMessage, ChatRole, ToolStatus};
use crate::selection;
use crate::theme::Theme;

/// 中央面板组件（消息列表 + 输入框）
///
/// Phase 4 RPC 模式：从 ANSI 缓冲区改为结构化 ChatMessage 列表渲染。
/// 输入框嵌入底部，支持文本输入和发送。
#[derive(Default)]
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

        // 输入框
        let input_line = self.render_input_inner(theme);
        f.render_widget(Paragraph::new(input_line), input_area);
    }

    /// 渲染消息行
    /// 如果 `is_selected` 为 true，整条消息使用 highlight_bg 背景高亮
    fn render_message(message: &ChatMessage, theme: &Theme, is_selected: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        match message.role {
            ChatRole::User => {
                lines.push(Line::from(vec![" 你 ".to_string().fg(theme.accent).bold()]));
                // 分隔线和内容
                lines.push(Line::from(vec![" ─".to_string().fg(theme.text_dim)]));
                for text_line in message.text.lines() {
                    lines.push(Line::from(vec![format!(" {}", text_line).fg(theme.text)]));
                }
                lines.push(Line::from(""));
            }

            ChatRole::Assistant => {
                lines.push(Line::from(vec![" pi ".to_string().fg(theme.accent).bold()]));
                lines.push(Line::from(vec![" ─".to_string().fg(theme.text_dim)]));

                // 思考过程（可折叠）
                if let Some(ref thinking) = message.thinking {
                    if !thinking.is_empty() {
                        // 仅显示前几行作为预览
                        let preview: String =
                            thinking.lines().take(3).collect::<Vec<_>>().join("\n");
                        lines.push(Line::from(vec![
                            format!(" [思考] {}", preview).fg(theme.text_dim).dim(),
                        ]));
                    }
                }

                // 消息文本
                for text_line in message.text.lines() {
                    lines.push(Line::from(vec![format!(" {}", text_line).fg(theme.text)]));
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
                    lines.push(Line::from(vec![
                        format!(
                            " {} {} {}",
                            icon,
                            tool_call.tool_name,
                            " ".to_string().dim()
                        )
                        .fg(theme.text_dim),
                    ]));
                }
            }

            ChatRole::System => {
                lines.push(Line::from(vec![
                    format!(" {}", message.text).fg(theme.text_dim).dim(),
                ]));
                lines.push(Line::from(""));
            }

            ChatRole::Error => {
                lines.push(Line::from(vec![
                    format!(" ⚠ {}", message.text).fg(theme.accent),
                ]));
                lines.push(Line::from(""));
            }
        }

        // 如果选中，给所有行应用 highlight_bg
        if is_selected {
            for line in &mut lines {
                let bg_style = Style::default().bg(theme.highlight_bg);
                *line = line.clone().patch_style(bg_style);
            }
        }

        lines
    }

}

impl Component for MainView {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
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
            let is_selected = self.has_focus
                && self.subsection == MainViewSubsection::Messages
                && i == self.message_cursor;
            all_lines.extend(Self::render_message(msg, theme, is_selected));
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

        // 输入框
        let input_line = self.render_input_inner(theme);
        f.render_widget(Paragraph::new(input_line), input_area);
    }
}

impl MainView {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ChatMessage, ToolCallInfo, ToolStatus};
    use crate::theme::Theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
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
}
