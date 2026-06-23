use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::action::Action;
use crate::backend::AgentBackend;
use crate::components::Component;
use crate::components::agent_panel::AgentPanel;
use crate::components::bottom_bar::BottomBar;
use crate::components::main_view::MainView;
use crate::components::popup::Popup;
use crate::components::sidebar::Sidebar;
use crate::errors::Result;
use crate::message::{ChatMessage, ChatRole};
use crate::theme::Theme;
use ratatui::text::{Line, Span};
use ratatui::style::Stylize;

/// Session 信息
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    /// session UUID
    pub id: String,
    /// session 文件路径
    pub file_path: Option<String>,
    /// session 显示名称
    pub name: Option<String>,
}

/// 工作区节点（树形结构）
#[derive(Debug, Clone)]
pub struct WorkspaceNode {
    pub name: String,
    pub sessions: Vec<SessionNode>,
    pub expanded: bool,
}

/// 会话节点（工作区下的叶子）
#[derive(Debug, Clone)]
pub struct SessionNode {
    pub id: String,
    pub name: String,
    pub file_path: Option<String>,
    pub message_count: usize,
    pub is_online: bool,
}

/// Agent 运行状态
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Starting,
    Idle,
    Running,
    Error,
    Closed,
}

/// Agent 运行时状态（模型、token、cost）
#[derive(Debug, Clone, Default)]
pub struct AgentRuntimeState {
    pub model_name: Option<String>,
    pub provider: Option<String>,
    pub thinking_level: Option<String>,
    pub is_streaming: bool,
    pub context_tokens: Option<u32>,
    pub context_window: Option<u32>,
    pub context_percent: Option<f32>,
    pub cache_read: u32,
    pub cache_write: u32,
    pub cost: f64,
}

/// subagent 定义（从 .md 文件 YAML frontmatter 解析）
#[derive(Debug, Clone)]
pub struct SubAgentInfo {
    pub name: String,
    pub description: String,
    pub model: String,
}

/// skill 定义（从 skills/*/SKILL.md YAML frontmatter 解析）
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
}

/// MCP server 定义（从 mcp.json / mcp-cache.json 解析）
#[derive(Debug, Clone)]
pub struct McpInfo {
    pub name: String,
    pub command: String,
    /// 缓存 server 的工具数（无 command 时用）
    pub tool_count: usize,
}

/// 聚焦的面板
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FocusPanel {
    Sidebar,
    #[default]
    MainView,
    AgentPanel,
}

/// 侧边栏子区
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SidebarSubsection {
    ActiveSession,
    #[default]
    Workspace,
    Provider,
}

/// 主视图子区
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum MainViewSubsection {
    Messages,
    #[default]
    Input,
}

/// Agent 面板子区
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AgentPanelSubsection {
    #[default]
    Agents,
    Skills,
    Mcps,
    Tasks,
}

/// 消息列表滚动模式
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ScrollMode {
    /// 尾部跟随：新消息到达自动滚动到底部
    #[default]
    TailFollow,
    /// 固定到光标所在消息
    Pinned,
}

/// TUI 模式的状态数据
pub struct TuiState {
    pub active_session: String,
    pub current_time: String,
    pub theme: Theme,

    // 组件实例
    pub sidebar: Sidebar,
    pub main_view: MainView,
    pub agent_panel: AgentPanel,
    pub bottom_bar: BottomBar,
    pub popup: Popup,

    // 工作区数据
    pub workspaces: Vec<WorkspaceNode>,
    /// 焦点面板
    pub focus_panel: FocusPanel,
    /// 侧边栏光标位置（扁平化列表中的索引）
    pub sidebar_cursor: usize,
    /// 侧边栏子区
    pub sidebar_subsection: SidebarSubsection,
    /// 主视图子区
    pub main_view_subsection: MainViewSubsection,
    /// Agent 面板子区
    pub agent_panel_subsection: AgentPanelSubsection,
    /// Agent 面板内 subagent 列表选中光标（仅索引 Vec<SubAgentInfo>）
    pub agent_cursor: usize,
    /// 消息列表中选择的消息索引
    pub message_cursor: usize,
    /// 消息列表滚动模式
    pub scroll_mode: ScrollMode,
    /// 鼠标拖拽选区状态
    pub selection: SelectionState,
    /// subagent 列表（从 agents/*.md 解析）
    pub subagents: Vec<SubAgentInfo>,
    /// HTTP Router 运行状态
    pub router_running: bool,
    /// Provider 配置
    pub providers: Vec<crate::provider::ProviderInfo>,
    /// 当前选中模型
    pub current_model: String,
    /// Provider 列表中光标位置
    pub provider_cursor: usize,
    /// Provider 详情 popup：None=关闭, Some(idx)=显示第 idx 个 provider
    pub provider_popup: Option<usize>,
    /// 模型列表中光标位置
    pub model_cursor: usize,
    /// 是否正在选择模型（展开状态）
    pub selecting_model: bool,
    /// skill 列表（从 skills/*/SKILL.md 解析）
    pub skills: Vec<SkillInfo>,
    /// Agent 面板内 skill 列表选中光标
    pub skill_cursor: usize,
    /// MCP server 列表（从 mcp.json 解析）
    pub mcps: Vec<McpInfo>,
    /// Agent 面板内 MCP 列表选中光标
    pub mcp_cursor: usize,
}

/// 鼠标选中所在的栏
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SelectionPanel {
    #[default]
    None,
    Sidebar,
    Content,
    AgentPanel,
}

/// 鼠标拖拽选区
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// 选中所在的栏
    pub panel: SelectionPanel,
    /// 是否已激活（mouse down 后到 mouse up 前）
    pub active: bool,
    /// 鼠标按下的终端坐标 (col, row)
    pub anchor: Option<(u16, u16)>,
    /// 鼠标拖拽到的终端坐标 (col, row)
    pub focus: Option<(u16, u16)>,
    /// 最近一次渲染时收集的选中文本（由 apply_selection 填充）
    pub selected_text: String,
}

impl Default for TuiState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            active_session: String::new(),
            current_time: String::new(),
            theme: Theme::cyan(),
            sidebar: Sidebar::default(),
            main_view: MainView::default(),
            agent_panel: AgentPanel::default(),
            bottom_bar: BottomBar::default(),
            popup: Popup::default(),
            workspaces: Vec::new(),
            focus_panel: FocusPanel::MainView,
            sidebar_cursor: 0,
            sidebar_subsection: SidebarSubsection::Workspace,
            main_view_subsection: MainViewSubsection::Input,
            agent_panel_subsection: AgentPanelSubsection::Agents,
            agent_cursor: 0,
            message_cursor: 0,
            scroll_mode: ScrollMode::TailFollow,
            selection: SelectionState::default(),
            subagents: Vec::new(),
            skills: Vec::new(),
            skill_cursor: 0,
            mcps: Vec::new(),
            mcp_cursor: 0,
            router_running: false,
            providers: Vec::new(),
            current_model: String::new(),
            provider_cursor: 0,
            model_cursor: 0,
            selecting_model: false,
            provider_popup: None,
        }
    }

    /// 计算侧边栏可见项的扁平化数量（展开的工作区占用 header + sessions，折叠的只占 header）
    pub fn sidebar_visible_count(&self) -> usize {
        let mut count = 0;
        for ws in &self.workspaces {
            count += 1; // workspace header
            if ws.expanded {
                count += ws.sessions.len();
            }
        }
        count
    }

    /// 获取扁平化索引对应的项信息（用于渲染光标）
    /// 返回 (is_workspace, ws_index, session_index_option)
    pub fn sidebar_item_at(&self, cursor: usize) -> Option<(bool, usize, Option<usize>)> {
        let mut idx = 0;
        for (wi, ws) in self.workspaces.iter().enumerate() {
            if idx == cursor {
                return Some((true, wi, None));
            }
            idx += 1;
            if ws.expanded {
                for si in 0..ws.sessions.len() {
                    if idx == cursor {
                        return Some((false, wi, Some(si)));
                    }
                    idx += 1;
                }
            }
        }
        None
    }
}

/// 应用状态机
///
/// 管理 AgentBackend / PiRpcBackend 生命周期，处理 Action 事件。
pub struct App {
    // --- PTY 模式字段 ---
    /// Agent 后端实例（PTY headless 模式使用）
    backend: Option<Box<dyn AgentBackend>>,
    /// 向主循环发送 Action 的 channel sender
    action_tx: mpsc::Sender<Action>,
    /// 子进程是否正在运行
    pub running: bool,

    // --- RPC 模式字段 ---
    /// 是否使用 RPC 模式
    pub use_rpc: bool,
    /// 会话消息列表（agent_id → messages）
    pub messages: HashMap<String, Vec<ChatMessage>>,
    /// 当前活跃 agent
    pub active_agent: Option<String>,
    /// 运行时状态
    pub runtime: AgentRuntimeState,
    /// Agent 状态
    pub agent_status: AgentStatus,
    /// Session 信息
    pub session: SessionInfo,

    // --- 共享 ---
    /// TUI 状态
    pub tui: TuiState,
}

impl App {
    /// 创建 PTY 模式 App
    pub fn new(backend: Box<dyn AgentBackend>, action_tx: mpsc::Sender<Action>) -> Self {
        Self {
            backend: Some(backend),
            action_tx,
            running: false,
            use_rpc: false,
            messages: HashMap::new(),
            active_agent: None,
            runtime: AgentRuntimeState::default(),
            agent_status: AgentStatus::Closed,
            session: SessionInfo::default(),
            tui: TuiState::new(),
        }
    }

    /// 创建 RPC 模式 App
    pub fn new_rpc(action_tx: mpsc::Sender<Action>) -> Self {
        let mut tui = TuiState::new();
        tui.main_view.has_focus = true;
        tui.focus_panel = FocusPanel::MainView;
        Self {
            backend: None,
            action_tx,
            running: false,
            use_rpc: true,
            messages: HashMap::new(),
            active_agent: Some("default".into()),
            runtime: AgentRuntimeState::default(),
            agent_status: AgentStatus::Starting,
            session: SessionInfo::default(),
            tui,
        }
    }

    /// 启动子进程（PTY 模式）
    pub async fn start(&mut self) -> Result<()> {
        if let Some(ref mut backend) = self.backend {
            let tx = self.action_tx.clone();
            backend.start(tx).await?;
            self.running = true;
        }
        Ok(())
    }

    /// 处理一个 Action
    pub async fn handle_action(&mut self, action: Action) -> Result<()> {
        match action {
            // --- 用户输入 ---
            Action::UserSubmitInput(input) => {
                if self.use_rpc {
                    let agent_id = self.active_agent.clone().unwrap_or_default();
                    let msg = ChatMessage::user(&agent_id, &input);
                    self.push_message(&agent_id, msg);
                } else if let Some(ref mut backend) = self.backend {
                    backend.send_input(&input).await?;
                }
            }

            // --- RPC 事件 ---
            Action::MessageAppend { agent_id, text } => {
                self.append_to_last_assistant(&agent_id, &text);
            }

            Action::MessageFinalize { .. } => {
                // 消息定型完成，无需额外操作
            }

            Action::ThinkingAppend { agent_id, text } => {
                self.append_to_last_assistant_thinking(&agent_id, &text);
            }

            Action::ThinkingFinalize { agent_id, text } => {
                self.finalize_thinking(&agent_id, &text);
            }

            Action::ToolEvent {
                agent_id,
                tool_name,
                tool_call_id,
                status,
                args,
                result,
                ..
            } => {
                let tool_info = crate::message::ToolCallInfo {
                    tool_name,
                    tool_call_id,
                    status,
                    args: args.unwrap_or_default(),
                    result,
                    detail_text: String::new(),
                };
                let msg = ChatMessage::tool(&agent_id, tool_info);
                self.push_message(&agent_id, msg);
            }

            Action::AgentStatusChange { status, .. } => {
                self.agent_status = status;
            }

            Action::RuntimeStateUpdate(state) => {
                self.runtime = state;
            }

            Action::AgentMessagesLoaded { messages, .. } => {
                if let Some(agent_id) = &self.active_agent {
                    self.messages.insert(agent_id.clone(), messages);
                }
            }

            Action::AutoRetryStatus { text, .. } => {
                let agent_id = self.active_agent.clone().unwrap_or_default();
                let msg = ChatMessage::system(&agent_id, &text);
                self.push_message(&agent_id, msg);
            }

            // --- PTY 事件 ---
            #[allow(deprecated)]
            Action::PtyExit => {
                self.running = false;
            }

            // --- UI 控制 ---
            Action::TogglePopup => {
                self.tui.popup.visible = !self.tui.popup.visible;
            }

            // --- 侧边栏交互 ---
            Action::ToggleWorkspace(index) => {
                if let Some(ws) = self.tui.workspaces.get_mut(index) {
                    ws.expanded = !ws.expanded;
                }
            }

            Action::SelectSession(session_id) => {
                self.tui.active_session = session_id.clone();
                tracing::debug!("切换到会话: {}", session_id);
            }

            Action::SidebarMove(delta) => {
                let total = self.tui.sidebar_visible_count();
                if total > 0 {
                    let cur = self.tui.sidebar_cursor as i32;
                    let new = (cur + delta).clamp(0, total as i32 - 1);
                    self.tui.sidebar_cursor = new as usize;
                }
            }

            Action::CycleFocusPanel(dir) => {
                let panels = [
                    FocusPanel::Sidebar,
                    FocusPanel::MainView,
                    FocusPanel::AgentPanel,
                ];
                let current = self.tui.focus_panel;
                let idx = panels.iter().position(|p| *p == current).unwrap_or(1);
                let next = ((idx as i32 + dir).rem_euclid(3)) as usize;
                self.tui.focus_panel = panels[next];
            }

            Action::CycleFocusSubsection(dir) => {
                match self.tui.focus_panel {
                    FocusPanel::Sidebar => {
                        let subs = [
                            SidebarSubsection::ActiveSession,
                            SidebarSubsection::Workspace,
                            SidebarSubsection::Provider,
                        ];
                        let current = self.tui.sidebar_subsection;
                        let idx = subs.iter().position(|s| *s == current).unwrap_or(1);
                        let next = ((idx as i32 + dir).rem_euclid(3)) as usize;
                        self.tui.sidebar_subsection = subs[next];
                    }
                    FocusPanel::MainView => {
                        let subs = [MainViewSubsection::Messages, MainViewSubsection::Input];
                        let current = self.tui.main_view_subsection;
                        let idx = subs.iter().position(|s| *s == current).unwrap_or(1);
                        let next = ((idx as i32 + dir).rem_euclid(2)) as usize;
                        self.tui.main_view_subsection = subs[next];
                    }
                    FocusPanel::AgentPanel => {
                        let subs = [
                            AgentPanelSubsection::Agents,
                            AgentPanelSubsection::Skills,
                            AgentPanelSubsection::Mcps,
                            AgentPanelSubsection::Tasks,
                        ];
                        let current = self.tui.agent_panel_subsection;
                        let idx = subs.iter().position(|s| *s == current).unwrap_or(0);
                        let next = ((idx as i32 + dir).rem_euclid(4)) as usize;
                        self.tui.agent_panel_subsection = subs[next];
                    }
                }
            }

            _ => {
                tracing::debug!("未处理的 action: {:?}", action);
            }
        }
        Ok(())
    }

    /// 终止子进程（PTY 模式）
    pub async fn terminate(&mut self) -> Result<()> {
        self.running = false;
        if let Some(ref mut backend) = self.backend {
            backend.terminate().await?;
        }
        Ok(())
    }

    // --- 消息管理 ---

    /// 添加消息到指定 agent
    pub fn push_message(&mut self, agent_id: &str, msg: ChatMessage) {
        self.messages
            .entry(agent_id.to_string())
            .or_default()
            .push(msg);
        // 同步到 MainView
        if let Some(agent_id) = &self.active_agent {
            if let Some(msgs) = self.messages.get(agent_id) {
                self.tui.main_view.messages = msgs.clone();
            }
        }
    }

    /// 追加文本到最后一条 assistant 消息
    /// 如果不存在 assistant 消息，创建一个新消息
    fn append_to_last_assistant(&mut self, agent_id: &str, text: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        let found = msgs.iter_mut().rev().any(|m| {
            if matches!(m.role, crate::message::ChatRole::Assistant) {
                m.text.push_str(text);
                true
            } else {
                false
            }
        });
        if !found {
            msgs.push(ChatMessage::assistant(agent_id, text));
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 追加 thinking 文本到最后一条 assistant 消息
    /// 如果不存在 assistant 消息，创建一个新的
    fn append_to_last_assistant_thinking(&mut self, agent_id: &str, text: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        let found = msgs.iter_mut().rev().any(|m| {
            if matches!(m.role, crate::message::ChatRole::Assistant) {
                let t = m.thinking.get_or_insert_with(String::new);
                t.push_str(text);
                true
            } else {
                false
            }
        });
        if !found {
            let mut msg = ChatMessage::assistant(agent_id, "");
            msg.thinking = Some(text.to_string());
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 定型 thinking 文本
    fn finalize_thinking(&mut self, agent_id: &str, text: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        let found = msgs.iter_mut().rev().any(|m| {
            if matches!(m.role, crate::message::ChatRole::Assistant) {
                m.thinking = Some(text.to_string());
                true
            } else {
                false
            }
        });
        if !found {
            let mut msg = ChatMessage::assistant(agent_id, "");
            msg.thinking = Some(text.to_string());
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 同步消息到 MainView
    fn sync_messages_to_main_view(&mut self, agent_id: &str) {
        if let Some(msgs) = self.messages.get(agent_id) {
            self.tui.main_view.messages = msgs.clone();
        }
    }

    /// 渲染前同步所有组件数据
    fn sync_components(&mut self) {
        if let Some(agent_id) = &self.active_agent {
            if let Some(msgs) = self.messages.get(agent_id) {
                self.tui.main_view.messages = msgs.clone();
            }
        }
        self.tui.agent_panel.active_agent = self.active_agent.clone().unwrap_or_default();
        self.tui.agent_panel.agent_status = Some(self.agent_status.clone());
        self.tui
            .agent_panel
            .subagents
            .clone_from(&self.tui.subagents);

        // 同步 sidebar 数据
        // 会话名优先级：pi 返回的名称 → 当前会话第一条用户消息 → "New Session"
        let has_real_name = self.session.name.is_some()
            || self
                .active_agent
                .as_ref()
                .and_then(|id| self.messages.get(id))
                .is_some_and(|msgs| {
                    msgs.iter()
                        .any(|m| m.role == ChatRole::User && !m.text.is_empty())
                });
        let session_name = self
            .session
            .name
            .clone()
            .or_else(|| {
                // 从当前会话消息中找第一条用户消息作为名称
                self.active_agent
                    .as_ref()
                    .and_then(|id| self.messages.get(id))
                    .and_then(|msgs| {
                        msgs.iter().find_map(|m| {
                            if m.role == ChatRole::User && !m.text.is_empty() {
                                let text = m.text.trim();
                                let display = if text.chars().count() > 30 {
                                    let end: usize =
                                        text.chars().take(30).map(|c| c.len_utf8()).sum();
                                    format!("{}…", &text[..end])
                                } else {
                                    text.to_string()
                                };
                                Some(display)
                            } else {
                                None
                            }
                        })
                    })
            })
            .unwrap_or_else(|| "New Session".to_string());
        // 只有存在有意义名称时才显示 session_id，否则隐藏 UUID
        self.tui.sidebar.active_session = session_name;
        if has_real_name {
            self.tui.sidebar.session_id.clone_from(&self.session.id);
        } else {
            self.tui.sidebar.session_id.clear();
        }
        // 同步 provider 数据到 sidebar
        self.tui.sidebar.providers.clone_from(&self.tui.providers);
        self.tui.sidebar.router_running = self.tui.router_running;
        self.tui.sidebar.current_model.clone_from(&self.tui.current_model);
        self.tui.sidebar.provider_cursor = self.tui.provider_cursor;
        self.tui.sidebar.model_cursor = self.tui.model_cursor;
        self.tui.sidebar.selecting_model = self.tui.selecting_model;
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
                    .title(format!(" {} {} ", if p.bridge { "\u{1f517}" } else { "\u{25c6}" }, p.name))
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
                    Span::from("  mode     ").fg(theme.text_dim),
                    Span::from(if p.bridge { "bridge" } else { "standard" }).fg(theme.accent),
                ]));
                ln.push(Line::from(""));
                ln.push(Line::from(Span::from(format!("  models ({})", p.models.len())).fg(theme.heading).bold()));
                for m in &p.models {
                    let ctx = if m.context_window >= 1_000_000 {
                        format!("{}M", m.context_window / 1_000_000)
                    } else {
                        format!("{}K", m.context_window / 1000)
                    };
                    ln.push(Line::from(Span::from(format!("    o {:25} [{}] {:>6}", m.id, m.tier, ctx)).fg(theme.text_dim)));
                }
                ln.push(Line::from(""));
                ln.push(Line::from(Span::from("  [Esc] close  [Enter] activate").fg(theme.text_dim)));
                f.render_widget(ratatui::widgets::Paragraph::new(ln).style(ratatui::style::Style::default().bg(theme.bg)), inner);
            } else { self.tui.provider_popup = None; }
        }

        // 渲染后将组件中收集的选中文本同步回主 selection
        //（apply_selection 修改的是组件的克隆，mouse up 读的是主 selection）
        if !self.tui.sidebar.selection.selected_text.is_empty() {
            self.tui
                .selection
                .selected_text
                .clone_from(&self.tui.sidebar.selection.selected_text);
        } else if !self.tui.main_view.selection.selected_text.is_empty() {
            self.tui
                .selection
                .selected_text
                .clone_from(&self.tui.main_view.selection.selected_text);
        } else if !self.tui.agent_panel.selection.selected_text.is_empty() {
            self.tui
                .selection
                .selected_text
                .clone_from(&self.tui.agent_panel.selection.selected_text);
        }
    }
}

impl App {
    /// 从 session 目录扫描并填充工作区数据
    pub fn populate_workspaces(&mut self) {
        let sessions_dir = pi_sessions_dir();
        let mut workspaces = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let raw_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if raw_name.is_empty() {
                    continue;
                }
                // 解码为可读的项目目录名
                let ws_name = decode_workspace_name(&raw_name);

                let mut sessions = Vec::new();
                if let Ok(file_entries) = std::fs::read_dir(&path) {
                    for file_entry in file_entries.flatten() {
                        let fpath = file_entry.path();
                        if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                            continue;
                        }
                        let id = fpath
                            .file_stem()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // 从 jsonl 文件解析会话名称（第一条用户消息）
                        let name = extract_session_name(&fpath)
                            .unwrap_or_else(|| "New Session".to_string());
                        // 粗略估计消息数：每行算一条消息
                        let message_count = if let Ok(content) = std::fs::read_to_string(&fpath) {
                            content.lines().count()
                        } else {
                            0
                        };
                        sessions.push(SessionNode {
                            id,
                            name,
                            file_path: Some(fpath.to_string_lossy().to_string()),
                            message_count,
                            is_online: false,
                        });
                    }
                }

                // 按消息数降序排列
                sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));

                let expanded = workspaces.is_empty(); // 第一个工作区默认展开
                workspaces.push(WorkspaceNode {
                    name: ws_name,
                    sessions,
                    expanded,
                });
            }
        }

        // 按工作区名称排序
        workspaces.sort_by(|a, b| a.name.cmp(&b.name));

        self.tui.workspaces = workspaces;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.running {
            tracing::warn!("App dropped while running — 确保主循环已调用 terminate()");
        }
    }
}

impl App {
    /// 扫描 agents/*.md 文件解析 subagent 列表
    pub fn populate_subagents(&mut self) {
        let mut agents = Vec::new();

        // 数据源优先级：用户目录 > 内置
        let builtin_dir = dirs_for("agents", false);
        let user_dir = dirs_for("agents", true);

        // 先加载内置，用户同名文件覆盖
        for dir in [&builtin_dir, &user_dir] {
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let fpath = entry.path();
                    if fpath.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&fpath) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    if let Some(info) = parse_agent_md(&content) {
                        // 用户目录覆盖内置
                        if dir == &user_dir {
                            if let Some(pos) = agents
                                .iter()
                                .position(|a: &SubAgentInfo| a.name == info.name)
                            {
                                agents[pos] = info;
                                continue;
                            }
                        }
                        agents.push(info);
                    }
                }
            }
        }

        self.tui.subagents = agents;
    }

    /// 扫描 skills 目录解析 skill 列表
    pub fn populate_skills(&mut self) {
        let mut skills = Vec::new();
        let skills_dir = std::path::PathBuf::from("/home/hr/.agents/skills");

        if skills_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let skill_md = path.join("SKILL.md");
                    if !skill_md.exists() {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&skill_md) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    if let Some(info) = parse_skill_md(&content) {
                        skills.push(info);
                    }
                }
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        self.tui.skills = skills;
    }

    /// 扫描 mcp.json + mcp-cache.json 解析 MCP server 列表
    pub fn populate_mcps(&mut self) {
        let mut mcps = Vec::new();

        // 定位 pi 配置目录（PI_CODING_AGENT_DIR 或 ~/.pi/agent）
        let pi_home = std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .map(|h| std::path::PathBuf::from(h).join(".pi").join("agent"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

        // 来源 1：mcp.json（用户显式配置的 server）
        let config_path = pi_home.join("mcp.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, info) in servers {
                            let command = info
                                .get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .to_string();
                            mcps.push(McpInfo { name: name.clone(), command, tool_count: 0 });
                        }
                    }
                }
            }
        }

        // 来源 2：mcp-cache.json（已连接的 server 缓存）
        let cache_path = pi_home.join("mcp-cache.json");
        if cache_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&cache_path) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("servers").and_then(|v| v.as_object()) {
                        for (name, info) in servers {
                            // 跳过已在 mcp.json 中定义的 server
                            if mcps.iter().any(|m: &McpInfo| m.name == *name) {
                                continue;
                            }
                            let tool_count = info
                                .get("tools")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            mcps.push(McpInfo {
                                name: name.clone(),
                                command: String::new(),
                                tool_count,
                            });
                        }
                    }
                }
            }
        }

        mcps.sort_by(|a, b| a.name.cmp(&b.name));
        self.tui.mcps = mcps;
    }
}

/// 解析 agent .md 文件的 YAML frontmatter
fn parse_agent_md(content: &str) -> Option<SubAgentInfo> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start();
    let end = rest.find("---")?;
    let yaml_text = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();
    let mut model = String::new();

    for line in yaml_text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("model:") {
            model = val.trim().to_string();
        }
    }

    if name.is_empty() {
        return None;
    }
    Some(SubAgentInfo {
        name,
        description,
        model,
    })
}

/// 解析 skill SKILL.md 文件的 YAML frontmatter
fn parse_skill_md(content: &str) -> Option<SkillInfo> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start();
    let end = rest.find("---")?;
    let yaml_text = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();

    for line in yaml_text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        }
    }

    if name.is_empty() {
        return None;
    }
    Some(SkillInfo { name, description })
}

/// 获取 pi 相关目录
fn dirs_for(subdir: &str, user: bool) -> std::path::PathBuf {
    let base = if user {
        std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(".pi").join("agent")
            })
    } else {
        std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(".pi").join("agent")
            })
            .join("npm")
            .join("node_modules")
            .join("pi-subagents")
    };
    base.join(subdir)
}

/// 将 pi 编码的工作区路径名解码为可读的目录名
///
/// `--media-hr-Data-Codes-agent-tui--` → `agent-tui`
/// `--home-hr-.pi-agent--` → `.pi-agent`
/// `--tmp--` → `tmp`
fn decode_workspace_name(encoded: &str) -> String {
    let inner = encoded.trim_start_matches('-').trim_end_matches('-');
    if inner.is_empty() {
        return encoded.to_string();
    }

    let parts: Vec<&str> = inner.split('-').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        // 短路径直接返回最后一段
        return parts.last().unwrap_or(&inner).to_string();
    }

    // 应从名称尾部剔除的通用路径前缀
    let strip_patterns = [
        "home",
        "media",
        "mnt",
        "tmp",
        "usr",
        "var",
        "opt",
        "etc",
        "srv",
        "root",
        "run",
        "boot",
        "hr",
        "Data",
        "Codes",
        "Projects",
        "workspace",
        "git",
        "src",
        "dev",
        "app",
        ".config",
        ".pi",
        ".local",
        "node_modules",
        "Users",
        "Documents",
        "Downloads",
        "Desktop",
    ];

    // 从尾部向前遍历，遇到通用路径前缀就停下
    let mut name_parts: Vec<&str> = Vec::new();
    for part in parts.iter().rev() {
        if name_parts.is_empty() || !strip_patterns.contains(part) {
            name_parts.push(part);
        } else if name_parts.len() <= 1 {
            // 如果只有一个有效段，允许保留一个 strip pattern
            name_parts.push(part);
        } else {
            break;
        }
    }

    name_parts.reverse();
    let name = name_parts.join("-");

    if name.is_empty() {
        parts.last().unwrap_or(&inner).to_string()
    } else {
        name
    }
}

/// 格式化会话名称：UUID 过长时截断显示
///
/// `019ef344-f862-708a-8e4d-f9a16cc8325b` → `019ef344…`
/// 从 JSONL session 文件中提取第一条用户消息作为会话名称
///
/// 返回截断到 35 字符的纯文本，失败时返回 None。
fn extract_session_name(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines().take(20) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let val: serde_json::Value = serde_json::from_str(line).ok()?;
        if val.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let msg = val.get("message")?;
        if msg.get("role").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let text = msg
            .get("content")?
            .as_array()?
            .first()?
            .get("text")?
            .as_str()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 截断到 35 字符（按字符数，非字节），过长时加 …
        if trimmed.chars().count() > 35 {
            let end: usize = trimmed.chars().take(35).map(|c| c.len_utf8()).sum();
            return Some(format!("{}…", &trimmed[..end]));
        }
        return Some(trimmed.to_string());
    }
    None
}

/// 获取 pi session 目录的路径
fn pi_sessions_dir() -> std::path::PathBuf {
    let base = std::env::var("PI_CODING_AGENT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".pi").join("agent")
        });
    base.join("sessions")
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc;

    use super::*;

    fn row_text(terminal: &Terminal<TestBackend>, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| terminal.backend().buffer()[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn render_tui_should_use_three_column_layout_without_top_bar() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.agent_status = AgentStatus::Idle;
        app.session.name = Some("demo-session".to_string());
        app.runtime.model_name = Some("deepseek-v4-flash".to_string());
        app.runtime.provider = Some("openrouter".to_string());

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("创建测试终端失败");

        terminal
            .draw(|f| {
                app.render_tui(f);
            })
            .expect("渲染 TUI 失败");

        // 第 0 行：sidebar 以空格开头（非聚焦时 ACTIVE SESSION 前有空格）
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, 0)].symbol(),
            " ",
            "row 0 should start with space before ACTIVE SESSION header"
        );

        // 第 1 行是活跃会话行（● demo-session）
        assert_eq!(
            buffer[(0, 1)].symbol(),
            "●",
            "row 1 should start with active session dot"
        );

        // 验证三栏分隔符位置
        let separators: Vec<u16> = (0..120)
            .filter(|x| buffer[(*x, 0)].symbol() == "│")
            .collect();
        assert_eq!(separators, vec![39, 80], "separators at expected positions");

        assert_eq!(
            buffer[(81, 0)].symbol(),
            " ",
            "agent panel should start with space before AGENTS (row 0)"
        );

        // 验证底栏存在（TestBackend 创建为 120x32）
        let last_row = 31u16;
        let bottom = row_text(&terminal, last_row, 120);
        assert!(
            bottom.contains("Ctrl+C"),
            "bottom bar should show shortcuts"
        );
    }
}
