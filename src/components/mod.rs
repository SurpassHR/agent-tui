use ratatui::layout::Rect;
use ratatui::Frame;

use crate::theme::Theme;

/// 所有 TUI 组件必须实现的渲染接口
pub trait Component {
    /// 在指定的 Rect 区域渲染组件
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme);
}

pub mod agent_panel;
pub mod bottom_bar;
pub mod main_view;
pub mod popup;
pub mod sidebar;
pub mod top_bar;

pub use agent_panel::AgentPanel;
pub use bottom_bar::BottomBar;
pub use main_view::MainView;
pub use popup::Popup;
pub use sidebar::Sidebar;
pub use top_bar::TopBar;
