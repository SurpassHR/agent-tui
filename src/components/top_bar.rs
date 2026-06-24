use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::Component;
use crate::app::AgentStatus;
use crate::theme::Theme;

/// 顶部状态栏组件
///
/// 显示 agent 状态、模型信息、会话 ID。
#[derive(Default)]
pub struct TopBar {
    pub current_time: String,
    pub session_id: String,
    pub agent_status: Option<AgentStatus>,
    pub model_name: Option<String>,
    pub provider: Option<String>,
}

impl TopBar {
    fn status_indicator(&self, theme: &Theme) -> (String, Color) {
        match self.agent_status {
            Some(AgentStatus::Running) => ("● running".into(), theme.success),
            Some(AgentStatus::Idle) => ("● idle".into(), theme.success),
            Some(AgentStatus::Starting) => ("◌ starting".into(), theme.heading),
            Some(AgentStatus::Error) => ("● error".into(), Color::Red),
            Some(AgentStatus::Closed) | None => ("○ closed".into(), theme.text_dim),
        }
    }
}

impl Component for TopBar {
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let (status_text, status_color) = self.status_indicator(theme);
        let model_text = match (&self.model_name, &self.provider) {
            (Some(name), Some(prov)) => format!("{}@{}", name, prov),
            (Some(name), None) => name.clone(),
            _ => "--".to_string(),
        };

        let status_line = Line::from(vec![
            " ".into(),
            status_text.fg(status_color),
            " | ".fg(theme.text_dim),
            model_text.fg(theme.accent),
            " | ".fg(theme.text_dim),
            if self.current_time.is_empty() {
                "--".into()
            } else {
                format!("time: {}", self.current_time).into()
            },
            " | ".fg(theme.text_dim),
            if self.session_id.is_empty() {
                "session: --".into()
            } else {
                format!("session: {}", self.session_id).into()
            },
        ])
        .alignment(Alignment::Left);

        let paragraph = Paragraph::new(status_line)
            .fg(theme.text)
            .style(Style::default().bg(theme.bg));
        f.render_widget(paragraph, area);
    }
}
