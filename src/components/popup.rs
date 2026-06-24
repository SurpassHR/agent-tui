use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::Component;
use crate::theme::Theme;

/// Popup 浮层组件（Z-index 叠加层）
///
/// 使用 `Clear` widget 清除目标区域后在顶层渲染，
/// 产生浮层覆盖效果。
///
/// 阶段二：数据为空，等待阶段三接入 session 详情逻辑。
#[derive(Default)]
pub struct Popup {
    pub visible: bool,
    pub title: String,
    pub description: String,
}

impl Component for Popup {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        // 计算居中弹窗区域（宽度 70%，高度 45%）
        let popup_area = centered_rect(70, 45, area);

        // Clear 产生叠加效果
        f.render_widget(Clear, popup_area);

        // Popup 边框
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(theme.border);

        // 标题
        let title_line = Line::from(self.title.clone().fg(theme.text));
        let description_text = if self.description.is_empty() {
            Line::from("  待接入...".to_string().dim())
        } else {
            Line::from(self.description.clone())
        };

        let content = vec![
            title_line,
            Line::from(""),
            description_text,
            Line::from(""),
            Line::from("  按 Ctrl+C 关闭".to_string().fg(theme.text_dim)),
        ];

        let paragraph = Paragraph::new(content).block(block);
        f.render_widget(paragraph, popup_area);

        // 在右上角画一个 X 关闭按钮
        let close_x = popup_area.right().saturating_sub(4);
        let close_y = popup_area.y + 1;
        if close_x <= popup_area.right() {
            let close_area = Rect::new(close_x, close_y, 3, 1);
            f.render_widget(Paragraph::new("[X]").fg(theme.text_dim), close_area);
        }
    }
}

/// 计算居中矩形（相对指定区域的居中百分比）
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v_layout = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Percentage(percent_y),
        Constraint::Fill(1),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(percent_x),
        Constraint::Fill(1),
    ])
    .split(v_layout[1])[1]
}
