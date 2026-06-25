use ratatui::style::Color;

/// Meta-TUI 主题色定义
///
/// 阶段二仅实现 Cyan 主题，后续可扩展为多主题切换。
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub text: Color,
    pub text_dim: Color,
    pub border: Color,
    pub border_dim: Color,
    pub highlight_bg: Color,
    pub accent: Color,
    pub heading: Color,
    pub success: Color,
    pub selection_fg: Color,
    /// Diff 删除行背景色
    pub diff_red_bg: Color,
    /// Diff 新增行背景色
    pub diff_green_bg: Color,
    /// Markdown 标题颜色
    pub heading_color: Color,
    /// Markdown 引用块文字颜色
    pub blockquote_text: Color,
    /// Markdown 内联代码背景色
    pub inline_code_bg: Color,
    /// Markdown 代码块边框颜色
    pub code_border: Color,
    /// Markdown 代码块背景色
    pub code_bg: Color,
    /// Markdown 链接颜色
    pub link_color: Color,
    /// 警告/确认弹窗边框颜色
    pub warning: Color,
    /// 用户消息气泡背景色
    pub user_bg: Color,
}

impl Theme {
    /// 默认 Cyan 工业风主题
    pub const fn cyan() -> Self {
        Self {
            bg: Color::Rgb(0, 7, 14),
            text: Color::Rgb(223, 232, 232),
            text_dim: Color::Rgb(132, 155, 156),
            border: Color::Rgb(151, 255, 245),
            border_dim: Color::Rgb(76, 133, 135),
            highlight_bg: Color::Rgb(90, 90, 90),
            accent: Color::Rgb(255, 191, 92),
            heading: Color::Rgb(255, 191, 92),
            success: Color::Rgb(173, 255, 47),
            selection_fg: Color::Rgb(255, 255, 255),
            diff_red_bg: Color::Rgb(42, 26, 26),
            diff_green_bg: Color::Rgb(26, 42, 26),
            heading_color: Color::Rgb(255, 191, 92),
            blockquote_text: Color::Rgb(132, 155, 156),
            inline_code_bg: Color::Rgb(30, 40, 45),
            code_border: Color::Rgb(76, 133, 135),
            code_bg: Color::Rgb(10, 22, 28),
            link_color: Color::Rgb(151, 255, 245),
            warning: Color::Rgb(255, 159, 67),
            user_bg: Color::Rgb(45, 60, 72),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::cyan()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyan_theme_should_match_midnight_control_room_palette() {
        let theme = Theme::cyan();

        assert_eq!(theme.bg, Color::Rgb(0, 7, 14));
        assert_eq!(theme.text, Color::Rgb(223, 232, 232));
        assert_eq!(theme.text_dim, Color::Rgb(132, 155, 156));
        assert_eq!(theme.border, Color::Rgb(151, 255, 245));
        assert_eq!(theme.border_dim, Color::Rgb(76, 133, 135));
        assert_eq!(theme.highlight_bg, Color::Rgb(90, 90, 90));
        assert_eq!(theme.accent, Color::Rgb(255, 191, 92));
        assert_eq!(theme.heading, Color::Rgb(255, 191, 92));
        assert_eq!(theme.success, Color::Rgb(173, 255, 47));
        assert_eq!(theme.selection_fg, Color::Rgb(255, 255, 255));
        assert_eq!(theme.diff_red_bg, Color::Rgb(42, 26, 26));
        assert_eq!(theme.diff_green_bg, Color::Rgb(26, 42, 26));
        assert_eq!(theme.heading_color, Color::Rgb(255, 191, 92));
        assert_eq!(theme.blockquote_text, Color::Rgb(132, 155, 156));
        assert_eq!(theme.inline_code_bg, Color::Rgb(30, 40, 45));
        assert_eq!(theme.code_border, Color::Rgb(76, 133, 135));
        assert_eq!(theme.code_bg, Color::Rgb(10, 22, 28));
        assert_eq!(theme.link_color, Color::Rgb(151, 255, 245));
        assert_eq!(theme.warning, Color::Rgb(255, 159, 67));
        assert_eq!(theme.user_bg, Color::Rgb(45, 60, 72));
    }
}
