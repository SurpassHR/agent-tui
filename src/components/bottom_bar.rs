use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use super::Component;
use crate::theme::Theme;

/// 底部栏组件
///
/// 左侧：键盘快捷键
/// 右侧：Token / Context / 成本信息
#[derive(Default)]
pub struct BottomBar {
    pub context_percent: Option<f32>,
    pub context_window: Option<u32>,
    pub cache_read: u32,
    pub cache_write: u32,
    pub cost: f64,
    /// 中央状态提示（如「✓ 已复制」「选中中...」）
    pub status: String,
}

impl BottomBar {
    /// 格式化 token 数量（1.2K / 1.0M）
    fn fmt_tokens(val: u32) -> String {
        if val >= 1_000_000 {
            format!("{:.1}M", val as f64 / 1_000_000.0)
        } else if val >= 1_000 {
            format!("{}K", (val as f64 / 1_000.0).round() as u32)
        } else {
            val.to_string()
        }
    }

    /// 构建右侧信息文本
    fn right_info(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Context 占用: "12%/1.0M"
        if let (Some(pct), Some(win)) = (self.context_percent, self.context_window) {
            parts.push(format!("{:.0}%/{}", pct * 100.0, Self::fmt_tokens(win)));
        }

        // Token 流速: "↑57K ↓61K"（始终显示，初始为 0）
        parts.push(format!(
            "↑{} ↓{}",
            Self::fmt_tokens(self.cache_read),
            Self::fmt_tokens(self.cache_write)
        ));

        // Cost（始终显示，初始为 $0）
        if self.cost > 0.0 {
            parts.push(format!("${:.3}", self.cost));
        } else {
            parts.push("$0".to_string());
        }

        parts.join(" | ")
    }
}

impl Component for BottomBar {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_type(BorderType::Plain)
            .border_style(theme.border);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // ── 左侧：快捷键 ──
        let left = Line::from(vec![
            Span::from("> ").fg(theme.success),
            Span::from("Ctrl+C").fg(theme.heading).bold(),
            Span::from(" quit; "),
            Span::from("|").fg(theme.text_dim),
            Span::from(" Ctrl+Z").fg(theme.heading).bold(),
            Span::from(" abort; "),
            Span::from("|").fg(theme.text_dim),
            Span::from(" Tab").fg(theme.heading).bold(),
            Span::from(" focus; "),
        ]);

        let left_para = Paragraph::new(left)
            .fg(theme.text_dim)
            .style(Style::default().bg(theme.bg))
            .alignment(Alignment::Left);
        f.render_widget(left_para, inner);

        // ── 中央：状态提示 ──
        if !self.status.is_empty() {
            let right_w = self.right_info().len() as u16 + 2;
            let center_area = Rect::new(
                inner.x + 40,
                inner.y,
                inner.width.saturating_sub(40 + right_w + 4),
                inner.height,
            );
            f.render_widget(
                Paragraph::new(self.status.clone())
                    .fg(theme.accent)
                    .alignment(Alignment::Center),
                center_area,
            );
        }

        // ── 右侧：Token / Context / Cost ──
        let right = self.right_info();
        if !right.is_empty() {
            let right_w = right.len() as u16 + 2;
            if inner.width > right_w {
                let right_area = Rect::new(
                    inner.right().saturating_sub(right_w),
                    inner.y,
                    right_w,
                    inner.height,
                );
                f.render_widget(
                    Paragraph::new(right)
                        .fg(theme.text_dim)
                        .alignment(Alignment::Right),
                    right_area,
                );
            }
        }
    }
}
