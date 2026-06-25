use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::action::Action;
use crate::backend::AgentBackend;
use crate::components::agent_panel::AgentPanel;
use crate::components::bottom_bar::BottomBar;
use crate::components::main_view::MainView;
use crate::components::popup::Popup;
use crate::components::sidebar::Sidebar;
use crate::components::Component;
use crate::errors::Result;
use crate::message::{ChatMessage, ChatRole};
use crate::theme::Theme;
use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;

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

/// 工作区树重命名/创建状态
#[derive(Debug, Clone)]
pub enum RenameState {
    CreateSession { ws_idx: usize },
    RenameSession { ws_idx: usize, si: usize },
    RenameWorkspace { ws_idx: usize },
    CreateWorkspace,
}
#[derive(Debug, Clone)]
pub struct WorkspaceNode {
    /// 工作区对应的实际文件系统路径（从 JSONL cwd 字段提取）
    pub cwd: String,
    /// 侧边栏显示名称（取 cwd 最后一级，重名时自动加父级）
    pub display_name: String,
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
    Model,
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
    /// HTTP Router 实际绑定的端口
    pub router_port: u16,
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
    pub model_search: String,
    /// 标记用户刚在 MODEL 子区选择了模型，tui.rs 据此发送 RPC 给 pi
    pub model_just_switched: bool,
    /// 当前按 Space 选中的 Provider 索引（None = 无 Provider 被选中）
    pub active_provider_idx: Option<usize>,
    pub provider_editor: Option<ProviderEditor>,
    /// 共享配置（与 router 共享的 Arc<RwLock<ProviderConfig>>）
    pub shared_config: Option<crate::provider::SharedConfig>,
    pub models_fetch_rx: Option<tokio::sync::oneshot::Receiver<Option<String>>>,
    /// skill 列表（从 skills/*/SKILL.md 解析）
    pub skills: Vec<SkillInfo>,
    /// Agent 面板内 skill 列表选中光标
    pub skill_cursor: usize,
    /// MCP server 列表（从 mcp.json 解析）
    pub mcps: Vec<McpInfo>,
    /// Agent 面板内 MCP 列表选中光标
    pub mcp_cursor: usize,
    /// 持久化禁用标记（单元测试设为 true 以避免污染真实配置文件）
    pub persistence_disabled: bool,
    /// AI 重命名：待发送给 pi 的 prompt（tui.rs 检测后通过 RPC 发出）
    pub pending_ai_prompt: Option<String>,
    /// 工作区重命名/创建状态（弹出输入模式）
    pub workspace_rename: Option<RenameState>,
    /// 重命名/创建时的输入缓冲区
    pub rename_input: String,
    /// 删除确认弹窗状态（None = 无待确认操作）
    pub confirm_delete: Option<ConfirmDelete>,
}

/// 删除确认弹窗的待确认信息
#[derive(Debug, Clone)]
pub struct ConfirmDelete {
    pub target: ConfirmDeleteTarget,
}

#[derive(Debug, Clone)]
pub enum ConfirmDeleteTarget {
    /// 删除工作区（含所有会话）
    Workspace { index: usize, name: String },
    /// 删除工作区下的指定会话
    Session {
        workspace_index: usize,
        session_index: usize,
        name: String,
    },
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

/// Provider 编辑状态（表单编辑模式）
#[derive(Debug, Clone)]
pub struct ProviderEditor {
    pub is_new: bool,
    pub index: usize,
    pub draft: crate::provider::ProviderInfo,
    pub field_focus: usize,
    pub models_text: String,
    pub models_fetching: bool,
    pub model_mgr: Option<ModelManager>,
}

#[derive(Debug, Clone)]
pub struct ModelManager {
    pub cursor: usize,
    pub editor: Option<ModelFieldEditor>,
}

#[derive(Debug, Clone)]
pub struct ModelFieldEditor {
    pub field_focus: usize,
    pub draft_id: String,
    pub draft_tier: String,
    pub draft_ctx: String,
    pub model_index: usize,
}

pub fn rebuild_models_text(models: &[crate::provider::ModelInfo]) -> String {
    models
        .iter()
        .map(|m| format!("{}:{}:{}", m.id, m.tier, m.context_window))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn merge_models_text(
    text: &str,
    existing: &[crate::provider::ModelInfo],
) -> Vec<crate::provider::ModelInfo> {
    let parsed: Vec<crate::provider::ModelInfo> = text
        .split('\n')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.trim().split(':').collect();
            let id = parts.first()?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            let tier = parts.get(1).map(|s| s.trim()).unwrap_or("T2").to_string();
            let ctx = parts
                .get(2)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(128000);
            let enabled = existing
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.enabled)
                .unwrap_or(false);
            Some(crate::provider::ModelInfo {
                id,
                name: String::new(),
                context_window: ctx,
                reasoning: tier != "T1",
                tier,
                enabled,
            })
        })
        .collect();
    if parsed.is_empty() {
        existing.to_vec()
    } else {
        parsed
    }
}

pub fn pad_value(value: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let display_w = UnicodeWidthStr::width(value);
    if display_w >= width {
        let mut w = 0usize;
        let clipped: String = value
            .chars()
            .take_while(|c| {
                let cw = unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
                if w + cw <= width {
                    w += cw;
                    true
                } else {
                    false
                }
            })
            .collect();
        clipped
    } else {
        let pad = width - display_w;
        format!("{}{}", value, " ".repeat(pad))
    }
}

/// 编辑表单视觉焦点顺序：模式切换 → Provider ID → 名称 → Base URL → API Key → 模型
const FIELD_ORDER: [usize; 6] = [4, 0, 1, 2, 3, 5];

fn prev_field(current: usize) -> usize {
    if let Some(pos) = FIELD_ORDER.iter().position(|&f| f == current) {
        FIELD_ORDER[(pos + FIELD_ORDER.len() - 1) % FIELD_ORDER.len()]
    } else {
        current
    }
}

fn next_field(current: usize) -> usize {
    if let Some(pos) = FIELD_ORDER.iter().position(|&f| f == current) {
        FIELD_ORDER[(pos + 1) % FIELD_ORDER.len()]
    } else {
        current
    }
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
            shared_config: None,
            subagents: Vec::new(),
            skills: Vec::new(),
            skill_cursor: 0,
            mcps: Vec::new(),
            mcp_cursor: 0,
            router_running: false,
            router_port: 8001,
            providers: Vec::new(),
            current_model: String::new(),
            provider_cursor: 0,
            model_cursor: 0,
            model_search: String::new(),
            model_just_switched: false,
            active_provider_idx: None,
            provider_popup: None,
            provider_editor: None,
            models_fetch_rx: None,
            persistence_disabled: false,
            pending_ai_prompt: None,
            workspace_rename: None,
            rename_input: String::new(),
            confirm_delete: None,
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

    /// 打开已有 Provider 编辑表单，预填充数据。
    fn open_edit_provider_editor(&mut self, idx: usize) {
        if let Some(p) = self.providers.get(idx) {
            self.provider_popup = None;
            let models_text = p
                .models
                .iter()
                .map(|m| format!("{}:{}:{}", m.id, m.tier, m.context_window))
                .collect::<Vec<_>>()
                .join("\n");
            self.provider_editor = Some(ProviderEditor {
                is_new: false,
                index: idx,
                draft: p.clone(),
                field_focus: 4,
                models_text,
                models_fetching: false,
                model_mgr: None,
            });
        }
    }

    /// 处理 Provider section 键盘事件
    /// 返回 true 表示事件被消费，false 表示未处理
    pub fn handle_provider_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.is_some() {
            return self.handle_provider_editor_key(key);
        }
        if let Some(popup_idx) = self.provider_popup {
            match key {
                KeyCode::Esc => {
                    self.provider_popup = None;
                    true
                }
                KeyCode::Enter => {
                    if let Some(p) = self.providers.get(popup_idx) {
                        if let Some(first) = p.models.first() {
                            self.active_provider_idx = Some(popup_idx);
                            self.current_model = first.id.clone();
                            self.model_just_switched = true;
                        }
                    }
                    self.provider_popup = None;
                    true
                }
                _ => false,
            }
        } else {
            match key {
                KeyCode::Up => {
                    if self.provider_cursor > 0 {
                        self.provider_cursor -= 1;
                    }
                    true
                }
                KeyCode::Down => {
                    if self.provider_cursor < self.providers.len() {
                        self.provider_cursor += 1;
                    }
                    true
                }
                KeyCode::Enter => {
                    if self.provider_cursor == self.providers.len() || self.providers.is_empty() {
                        self.provider_popup = None;
                        self.provider_editor = Some(ProviderEditor {
                            is_new: true,
                            index: self.providers.len(),
                            draft: crate::provider::ProviderInfo {
                                id: String::new(),
                                name: String::new(),
                                enabled: true,
                                bridge: false,
                                base_url: String::new(),
                                api_key: String::new(),
                                models: vec![],
                            },
                            field_focus: 4,
                            models_text: String::new(),
                            models_fetching: false,
                            model_mgr: None,
                        });
                    } else if !self.providers.is_empty() {
                        self.open_edit_provider_editor(self.provider_cursor);
                    }
                    true
                }
                KeyCode::Char(' ')
                    if !self.providers.is_empty()
                        && self.provider_cursor < self.providers.len() =>
                {
                    // Space → toggle：激活 / 取消激活
                    if let Some(p) = self.providers.get(self.provider_cursor) {
                        let already_active = self.active_provider_idx == Some(self.provider_cursor);
                        if already_active {
                            self.active_provider_idx = None;
                            self.current_model.clear();
                        } else if let Some(first) = p.models.first() {
                            self.active_provider_idx = Some(self.provider_cursor);
                            self.current_model = first.id.clone();
                        }
                        self.model_just_switched = true;
                    }
                    true
                }
                KeyCode::Char('e')
                    if !self.providers.is_empty()
                        && self.provider_cursor < self.providers.len() =>
                {
                    // e → 编辑 provider
                    self.open_edit_provider_editor(self.provider_cursor);
                    true
                }
                KeyCode::Char('d')
                    if !self.providers.is_empty()
                        && self.provider_cursor < self.providers.len() =>
                {
                    // d → 切换 provider enabled/disabled
                    if let Some(p) = self.providers.get_mut(self.provider_cursor) {
                        p.enabled = !p.enabled;
                        // 禁用活跃 Provider 时清除选中状态
                        if !p.enabled && self.active_provider_idx == Some(self.provider_cursor) {
                            self.active_provider_idx = None;
                            self.current_model.clear();
                        }
                        self.sync_provider_config();
                    }
                    true
                }
                KeyCode::Left | KeyCode::Esc => true,
                KeyCode::Char('+') => {
                    let default = crate::provider::ProviderInfo {
                        id: "new-provider".into(),
                        name: "New Provider".into(),
                        enabled: true,
                        bridge: false,
                        base_url: "https://api.openai.com/v1".into(),
                        api_key: String::new(),
                        models: vec![crate::provider::ModelInfo {
                            id: "gpt-4o".into(),
                            name: "GPT-4o".into(),
                            context_window: 128000,
                            reasoning: true,
                            tier: "T3".into(),
                            enabled: true,
                        }],
                    };
                    self.providers.push(default);
                    self.sync_provider_config();
                    true
                }
                _ => false,
            }
        }
    }

    /// 统一保存 provider 配置到磁盘，并同步更新与 router 共享的内存配置
    pub fn sync_provider_config(&self) {
        // 持久化是否被禁用（单元测试关闭以避免污染真实配置）
        if self.persistence_disabled {
            return;
        }
        let path = crate::provider::config_path();
        let cfg = crate::provider::ProviderConfig {
            port: self.router_port,
            current_model: Some(self.current_model.clone()),
            providers: self.providers.clone(),
        };
        crate::provider::ProviderConfig::save(&path, &cfg);
        // 同步到 router 共享的内存配置（若不可用则仅写磁盘）
        if let Some(ref shared) = self.shared_config {
            match shared.try_write() {
                Ok(mut guard) => {
                    *guard = cfg;
                }
                Err(_) => {
                    tracing::warn!("无法获取 SharedConfig 写锁，router 可能使用过期配置");
                }
            }
        }
    }

    /// 处理 MODEL 子区键盘事件（支持搜索过滤 + 滚动）
    pub fn handle_model_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.is_some() {
            return false;
        }
        match key {
            KeyCode::Up => {
                if self.model_cursor > 0 {
                    self.model_cursor -= 1;
                }
                true
            }
            KeyCode::Down => {
                let filtered = self.filtered_models();
                if self.model_cursor + 1 < filtered.len() {
                    self.model_cursor += 1;
                }
                true
            }
            KeyCode::Enter => {
                let model_id = {
                    let filtered = self.filtered_models();
                    filtered.get(self.model_cursor).map(|m| m.id.clone())
                };
                if let Some(mid) = model_id {
                    // MODEL 区显示的模型均来自当前活跃 Provider，直接用其索引
                    self.active_provider_idx = self.active_provider_index();
                    self.current_model = mid;
                    self.model_just_switched = true;
                }
                true
            }
            KeyCode::Char(' ') => {
                // Space → toggle：切换模型选中/取消
                let model_id = {
                    let filtered = self.filtered_models();
                    filtered.get(self.model_cursor).map(|m| m.id.clone())
                };
                if let Some(ref mid) = model_id {
                    if self.current_model == *mid {
                        // 当前选中 → 取消（保留 active_provider_idx，MODEL 区仍显示当前 Provider 模型）
                        self.current_model.clear();
                    } else {
                        // 未选中 → 选中（MODEL 区模型来自当前活跃 Provider）
                        self.active_provider_idx = self.active_provider_index();
                        self.current_model = mid.clone();
                    }
                    self.model_just_switched = true;
                }
                true
            }
            KeyCode::Backspace => {
                self.model_search.pop();
                self.model_cursor = 0;
                true
            }
            KeyCode::Esc => {
                self.model_search.clear();
                self.model_cursor = 0;
                true
            }
            KeyCode::Char(c) if c != ' ' => {
                self.model_search.push(c);
                self.model_cursor = 0;
                true
            }
            _ => false,
        }
    }

    /// 返回已开启 + 搜索匹配的模型列表
    pub fn filtered_models(&self) -> Vec<&crate::provider::ModelInfo> {
        let ap = match self.active_provider_for_models() {
            Some(p) => p,
            None => return vec![],
        };
        let q = self.model_search.to_lowercase();
        ap.models
            .iter()
            .filter(|m| {
                m.enabled && (self.model_search.is_empty() || m.id.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 获取当前活跃 provider（用于 MODEL 子区显示模型列表）
    /// 优先按 active_provider_idx 查找，其次按 current_model 匹配，只考虑启用的 provider
    pub fn active_provider_for_models(&self) -> Option<&crate::provider::ProviderInfo> {
        self.active_provider_index()
            .and_then(|idx| self.providers.get(idx))
    }

    /// 获取当前活跃 provider 的索引
    /// 优先按 active_provider_idx → current_model 匹配 → 第一个启用的 Provider
    pub fn active_provider_index(&self) -> Option<usize> {
        // 优先使用按索引选中的 Provider
        if let Some(idx) = self.active_provider_idx {
            if let Some(p) = self.providers.get(idx) {
                if p.enabled {
                    return Some(idx);
                }
            }
        }
        // 回退：按 current_model 匹配
        if let Some((i, _)) = self
            .providers
            .iter()
            .enumerate()
            .filter(|(_, p)| p.enabled)
            .find(|(_, p)| p.models.iter().any(|m| m.id == self.current_model))
        {
            return Some(i);
        }
        // 最终回退：第一个启用的 Provider
        self.providers
            .iter()
            .enumerate()
            .find(|(_, p)| p.enabled)
            .map(|(i, _)| i)
    }

    fn handle_provider_editor_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.as_ref().unwrap().model_mgr.is_some() {
            return self.handle_model_manager_key(key);
        }
        match key {
            KeyCode::Esc => {
                self.provider_editor = None;
                true
            }
            KeyCode::Tab => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = next_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.bridge
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            KeyCode::Enter => {
                if self.provider_editor.as_ref().unwrap().field_focus == 5 {
                    let editor = self.provider_editor.as_mut().unwrap();
                    editor.draft.models =
                        merge_models_text(&editor.models_text, &editor.draft.models);
                    editor.model_mgr = Some(ModelManager {
                        cursor: 0,
                        editor: None,
                    });
                    return true;
                }
                let editor = self.provider_editor.take().unwrap();
                let idx = editor.index;
                let draft = editor.draft;
                if editor.is_new {
                    self.providers.push(draft);
                } else if idx < self.providers.len() {
                    self.providers[idx] = draft;
                }
                self.sync_provider_config();
                true
            }
            KeyCode::Backspace => {
                if let Some(ref mut editor) = self.provider_editor {
                    match editor.field_focus {
                        0 => {
                            editor.draft.id.pop();
                        }
                        1 => {
                            editor.draft.name.pop();
                        }
                        2 => {
                            editor.draft.base_url.pop();
                        }
                        3 => {
                            editor.draft.api_key.pop();
                        }
                        _ => {}
                    }
                }
                true
            }
            KeyCode::Char(' ') => {
                if let Some(ref mut editor) = self.provider_editor {
                    if editor.field_focus == 4 {
                        editor.draft.bridge = !editor.draft.bridge;
                    } else if editor.field_focus != 5 {
                        match editor.field_focus {
                            0 => {
                                editor.draft.id.push(' ');
                            }
                            1 => {
                                editor.draft.name.push(' ');
                            }
                            2 => {
                                editor.draft.base_url.push(' ');
                            }
                            3 => {
                                editor.draft.api_key.push(' ');
                            }
                            _ => {}
                        }
                    }
                }
                true
            }
            KeyCode::Char(c) => {
                if let Some(ref mut editor) = self.provider_editor {
                    match editor.field_focus {
                        0 => {
                            editor.draft.id.push(c);
                        }
                        1 => {
                            editor.draft.name.push(c);
                        }
                        2 => {
                            editor.draft.base_url.push(c);
                        }
                        3 => {
                            editor.draft.api_key.push(c);
                        }
                        4 | 5 => {}
                        _ => {}
                    }
                }
                true
            }
            KeyCode::Up => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = prev_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.bridge
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            KeyCode::Down => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = next_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.bridge
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn handle_model_manager_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self
            .provider_editor
            .as_ref()
            .unwrap()
            .model_mgr
            .as_ref()
            .unwrap()
            .editor
            .is_some()
        {
            return self.handle_model_field_editor_key(key);
        }
        let editor = self.provider_editor.as_mut().unwrap();
        let mgr = editor.model_mgr.as_mut().unwrap();
        let mc = editor.draft.models.len();
        let last = mc + 1;
        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                if mgr.cursor > 0 {
                    mgr.cursor -= 1;
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if mgr.cursor < last {
                    mgr.cursor += 1;
                }
                true
            }
            KeyCode::Enter => {
                if mgr.cursor < mc {
                    let m = editor.draft.models[mgr.cursor].clone();
                    mgr.editor = Some(ModelFieldEditor {
                        field_focus: 0,
                        draft_id: m.id.clone(),
                        draft_tier: m.tier.clone(),
                        draft_ctx: m.context_window.to_string(),
                        model_index: mgr.cursor,
                    });
                } else if mgr.cursor == mc {
                    let idx = editor.draft.models.len();
                    editor.draft.models.push(crate::provider::ModelInfo {
                        id: "new-model".into(),
                        name: String::new(),
                        context_window: 128000,
                        reasoning: true,
                        tier: "T2".into(),
                        enabled: true,
                    });
                    mgr.cursor = idx;
                    mgr.editor = Some(ModelFieldEditor {
                        field_focus: 0,
                        draft_id: "new-model".into(),
                        draft_tier: "T2".into(),
                        draft_ctx: "128000".into(),
                        model_index: idx,
                    });
                } else {
                    editor.models_text = rebuild_models_text(&editor.draft.models);
                    editor.model_mgr = None;
                    let taken = self.provider_editor.take().unwrap();
                    let idx = taken.index;
                    let draft = taken.draft;
                    if taken.is_new {
                        self.providers.push(draft);
                    } else if idx < self.providers.len() {
                        self.providers[idx] = draft;
                    }
                    self.sync_provider_config();
                }
                true
            }
            KeyCode::Delete | KeyCode::Backspace => {
                if mgr.cursor < mc {
                    editor.draft.models.remove(mgr.cursor);
                    if mgr.cursor > 0 && mgr.cursor >= editor.draft.models.len() {
                        mgr.cursor = editor.draft.models.len();
                    }
                }
                true
            }
            KeyCode::Char(' ') => {
                if mgr.cursor < mc {
                    if let Some(m) = editor.draft.models.get_mut(mgr.cursor) {
                        m.enabled = !m.enabled;
                    }
                }
                true
            }
            KeyCode::Tab => true,
            KeyCode::Esc => {
                editor.models_text = rebuild_models_text(&editor.draft.models);
                editor.model_mgr = None;
                true
            }
            _ => false,
        }
    }

    fn handle_model_field_editor_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        const FO: [usize; 3] = [0, 1, 2];
        let editor = self.provider_editor.as_mut().unwrap();
        let mgr = editor.model_mgr.as_mut().unwrap();
        let fe = mgr.editor.as_mut().unwrap();
        match key {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                let pos = FO.iter().position(|&f| f == fe.field_focus).unwrap_or(0);
                fe.field_focus = FO[match key {
                    KeyCode::Up => (pos + 2) % 3,
                    _ => (pos + 1) % 3,
                }];
                true
            }
            KeyCode::Enter => {
                let tu = fe.draft_tier.to_uppercase();
                if fe.draft_id.trim().is_empty() {
                    fe.field_focus = 0;
                    return true;
                }
                if tu != "T1" && tu != "T2" && tu != "T3" {
                    fe.field_focus = 1;
                    return true;
                }
                match fe.draft_ctx.parse::<u32>() {
                    Ok(v) if v > 0 => {}
                    _ => {
                        fe.field_focus = 2;
                        return true;
                    }
                }
                if fe.model_index < editor.draft.models.len() {
                    editor.draft.models[fe.model_index] = crate::provider::ModelInfo {
                        id: fe.draft_id.trim().to_string(),
                        name: String::new(),
                        context_window: fe.draft_ctx.parse().unwrap_or(128000),
                        reasoning: tu != "T1",
                        tier: tu,
                        enabled: true,
                    };
                }
                mgr.editor = None;
                true
            }
            KeyCode::Esc => {
                mgr.editor = None;
                true
            }
            KeyCode::Backspace => {
                match fe.field_focus {
                    0 => {
                        fe.draft_id.pop();
                    }
                    1 => {
                        fe.draft_tier.pop();
                    }
                    2 => {
                        fe.draft_ctx.pop();
                    }
                    _ => {}
                }
                true
            }
            KeyCode::Char(c) => {
                match fe.field_focus {
                    0 => {
                        fe.draft_id.push(c);
                    }
                    1 => {
                        fe.draft_tier.push(c);
                    }
                    2 => {
                        fe.draft_ctx.push(c);
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

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

            Action::ContentUpdate { agent_id, content } => {
                tracing::info!(
                    "HANDLE ContentUpdate agent={} blocks={}",
                    agent_id,
                    content.len()
                );
                // 更新最后一条 Assistant 消息的 content 数组；
                // 若不存在则创建（处理首帧 message_update 不含 delta 的情况）
                let msgs = self.messages.entry(agent_id.clone()).or_default();
                if let Some(last) = msgs
                    .iter_mut()
                    .rev()
                    .find(|m| matches!(m.role, crate::message::ChatRole::Assistant))
                {
                    last.content = content;
                } else {
                    let mut msg = ChatMessage::assistant(&agent_id, "");
                    msg.content = content;
                    msgs.push(msg);
                }
            }

            Action::ToggleBlock {
                agent_id: _,
                msg_id,
                block_index,
            } => {
                let key = format!("{}:{}", msg_id, block_index);
                let state = self
                    .tui
                    .main_view
                    .block_states
                    .entry(key)
                    .or_insert(crate::message::BlockExpanded::Collapsed);
                *state = match *state {
                    crate::message::BlockExpanded::Collapsed => {
                        crate::message::BlockExpanded::Expanded
                    }
                    crate::message::BlockExpanded::Expanded => {
                        crate::message::BlockExpanded::Collapsed
                    }
                };
            }

            Action::EnterBlock {
                agent_id,
                msg_id,
                block_index,
            } => {
                let msgs = self.messages.get(&agent_id);
                if let Some(msgs) = msgs {
                    let blocks = crate::message::build_block_refs(msgs);
                    let key = format!("{}:{}", msg_id, block_index);
                    if let Some(block_ref) = blocks
                        .iter()
                        .find(|b| format!("{}:{}", b.msg_id, b.block_index) == key)
                    {
                        let msg = &msgs[block_ref.msg_index];
                        match block_ref.kind {
                            crate::message::BlockKind::Thinking => {
                                let thinking_text =
                                    if let Some(crate::message::ContentBlock::Thinking {
                                        thinking,
                                    }) = msg.content.get(block_ref.block_index)
                                    {
                                        thinking.clone()
                                    } else {
                                        String::new()
                                    };
                                if thinking_text.lines().count() > 5 {
                                    self.tui.main_view.entered_view =
                                        Some(crate::message::EnteredView::FullOutput {
                                            title: "思考过程".to_string(),
                                            content: thinking_text,
                                            scroll: 0,
                                        });
                                } else {
                                    // ≤ 5 行：折叠时展开，已展开时无效果
                                    let block_key = format!("{}:{}", msg_id, block_index);
                                    let state = self
                                        .tui
                                        .main_view
                                        .block_states
                                        .entry(block_key)
                                        .or_insert(crate::message::BlockExpanded::Collapsed);
                                    if *state == crate::message::BlockExpanded::Collapsed {
                                        *state = crate::message::BlockExpanded::Expanded;
                                    }
                                }
                            }
                            crate::message::BlockKind::ToolCall => {
                                if let Some(crate::message::ContentBlock::ToolCall {
                                    id: _,
                                    name,
                                    arguments,
                                    result,
                                    is_error: _,
                                }) = msg.content.get(block_ref.block_index)
                                {
                                    match name.as_str() {
                                        "edit" | "write" => {
                                            let old_text = arguments
                                                .get("oldText")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let new_text = arguments
                                                .get("newText")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let path = arguments
                                                .get("path")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("unknown");
                                            let diff_lines = compute_diff_lines(old_text, new_text);
                                            self.tui.main_view.entered_view =
                                                Some(crate::message::EnteredView::Diff {
                                                    path: path.to_string(),
                                                    diff_lines,
                                                    scroll: 0,
                                                });
                                        }
                                        "read" | "bash" => {
                                            let content = result
                                                .as_ref()
                                                .and_then(|r| r.get("content"))
                                                .and_then(|c| c.as_str())
                                                .unwrap_or("");
                                            self.tui.main_view.entered_view =
                                                Some(crate::message::EnteredView::FullOutput {
                                                    title: name.to_string(),
                                                    content: content.to_string(),
                                                    scroll: 0,
                                                });
                                        }
                                        "subagent" => {
                                            self.tui.main_view.entered_view =
                                                Some(crate::message::EnteredView::FullOutput {
                                                    title: format!("subagent: {}", name),
                                                    content: format!("{:?}", result),
                                                    scroll: 0,
                                                });
                                        }
                                        _ => {
                                            let content = result
                                                .as_ref()
                                                .map(|r| format!("{:?}", r))
                                                .unwrap_or_default();
                                            self.tui.main_view.entered_view =
                                                Some(crate::message::EnteredView::FullOutput {
                                                    title: name.clone(),
                                                    content,
                                                    scroll: 0,
                                                });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Action::ExitBlock => {
                self.tui.main_view.entered_view = None;
            }

            Action::ToolEvent {
                agent_id,
                tool_name,
                tool_call_id,
                status,
                args,
                result,
                is_error,
            } => {
                let _ = is_error;
                let msgs = self.messages.entry(agent_id.clone()).or_default();
                match status {
                    crate::message::ToolStatus::Running => {
                        // 创建新的 Tool ChatMessage
                        let tool_info = crate::message::ToolCallInfo {
                            tool_name,
                            tool_call_id,
                            status,
                            args: args.unwrap_or_default(),
                            result: None,
                            detail_text: String::new(),
                        };
                        let msg = ChatMessage::tool(&agent_id, tool_info);
                        msgs.push(msg);
                    }
                    _ => {
                        // Done 或 Error → 按 tool_call_id 找到并更新
                        if let Some(msg) = msgs.iter_mut().rev().find(|m| {
                            m.role == crate::message::ChatRole::Tool
                                && m.tool_call
                                    .as_ref()
                                    .is_some_and(|tc| tc.tool_call_id == tool_call_id)
                        }) {
                            if let Some(ref mut tc) = msg.tool_call {
                                tc.status = status.clone();
                                tc.result = result;
                                // 更新显示文本
                                let icon = match tc.status {
                                    crate::message::ToolStatus::Done => "✓",
                                    crate::message::ToolStatus::Error => "✗",
                                    crate::message::ToolStatus::Running => "▶",
                                };
                                msg.text = format!("{} {}", icon, tc.tool_name);
                            }
                        }
                    }
                }
                self.sync_messages_to_main_view(&agent_id);
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
                    // 保存 UI 状态到 state.json
                    crate::persistence::save(&self.build_persist_state());
                }
            }

            Action::SelectSession(session_id) => {
                self.tui.active_session = session_id.clone();
                tracing::info!("切换到会话: {}", session_id);

                // 查找 session 的 file_path
                let file_path = self.tui.workspaces.iter().find_map(|ws| {
                    ws.sessions
                        .iter()
                        .find(|s| s.id == session_id)
                        .and_then(|s| s.file_path.clone())
                });

                if let Some(ref path) = file_path {
                    tracing::info!("加载会话文件: {}", path);
                    match load_session_messages(path) {
                        Ok(messages) => {
                            self.messages.insert(session_id.clone(), messages);
                            self.active_agent = Some(session_id.clone());
                            // 更新 session 信息（供 sync_components 同步到 sidebar）
                            self.session.id = session_id.clone();
                            self.session.file_path = Some(path.clone());
                            // 从工作区数据中获取 session 名称
                            if let Some(ws) = self
                                .tui
                                .workspaces
                                .iter()
                                .find(|ws| ws.sessions.iter().any(|s| s.id == session_id))
                            {
                                if let Some(sess) = ws.sessions.iter().find(|s| s.id == session_id)
                                {
                                    self.session.name = Some(sess.name.clone());
                                }
                            }
                            // 同步消息到 MainView
                            self.sync_messages_to_main_view(&session_id);
                            // 同步组件状态
                            self.sync_components();
                            tracing::info!(
                                "会话加载完成: {} 条消息",
                                self.messages.get(&session_id).map(|m| m.len()).unwrap_or(0)
                            );
                        }
                        Err(e) => {
                            tracing::error!("加载会话文件失败: {} — {}", path, e);
                            self.tui.bottom_bar.status = format!("加载会话失败: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("未找到会话文件: {}", session_id);
                    // 即使没有文件，也切换 active_agent（新会话）
                    self.active_agent = Some(session_id.clone());
                    self.session.id = session_id.clone();
                    self.session.name = None;
                    self.session.file_path = None;
                    // 清空当前消息
                    self.messages.remove(&session_id);
                    self.sync_messages_to_main_view(&session_id);
                    self.sync_components();
                }
                // 保存 UI 状态到 state.json
                crate::persistence::save(&self.build_persist_state());
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

            Action::CycleFocusSubsection(dir) => match self.tui.focus_panel {
                FocusPanel::Sidebar => {
                    let subs = [
                        SidebarSubsection::ActiveSession,
                        SidebarSubsection::Workspace,
                        SidebarSubsection::Provider,
                        SidebarSubsection::Model,
                    ];
                    let current = self.tui.sidebar_subsection;
                    let idx = subs.iter().position(|s| *s == current).unwrap_or(1);
                    let next = ((idx as i32 + dir).rem_euclid(4)) as usize;
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
            },

            // --- 工作区 CRUD ---
            Action::CreateWorkspace(ref path) => {
                // 验证路径是否存在
                let p = std::path::Path::new(path);
                if !p.exists() || !p.is_dir() {
                    self.tui.bottom_bar.status = format!("路径不存在或不是目录: {}", path);
                } else {
                    let encoded = encode_workspace_name(path);
                    let sessions_dir = pi_sessions_dir();
                    let ws_dir = sessions_dir.join(&encoded);
                    if let Err(e) = std::fs::create_dir_all(&ws_dir) {
                        tracing::error!("创建工作区目录失败: {} — {}", ws_dir.display(), e);
                        self.tui.bottom_bar.status = format!("创建失败: {}", e);
                    } else {
                        // 直接插入新工作区（新目录尚无 JSONL，populate_workspaces 会跳过）
                        let display_name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.clone());
                        let new_ws = WorkspaceNode {
                            cwd: path.clone(),
                            display_name,
                            sessions: Vec::new(),
                            expanded: true,
                        };
                        self.tui.workspaces.push(new_ws);
                        compute_display_names(&mut self.tui.workspaces);
                        self.tui
                            .workspaces
                            .sort_by(|a, b| a.display_name.cmp(&b.display_name));
                        self.sync_components();
                        self.tui.bottom_bar.status = format!("已添加工作区: {}", path);
                    }
                }
            }

            Action::CreateSession {
                workspace_index,
                ref name,
            } => {
                if let Some(ws) = self.tui.workspaces.get(workspace_index) {
                    let sessions_dir = pi_sessions_dir();
                    let ws_encoded = encode_workspace_name(&ws.cwd);
                    let actual_dir = std::fs::read_dir(&sessions_dir).ok().and_then(|entries| {
                        entries.flatten().find_map(|e| {
                            let p = e.path();
                            if p.is_dir()
                                && p.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default()
                                    == ws_encoded
                            {
                                Some(p)
                            } else {
                                None
                            }
                        })
                    });

                    if let Some(ws_dir) = actual_dir {
                        let session_id = uuid_v4_simple();
                        let session_path = ws_dir.join(format!("{}.jsonl", session_id));
                        // 创建空的 jsonl 文件
                        if let Err(e) = std::fs::write(&session_path, "") {
                            tracing::error!("创建会话文件失败: {} — {}", session_path.display(), e);
                            self.tui.bottom_bar.status = format!("创建失败: {}", e);
                        } else {
                            self.populate_workspaces();
                            self.sync_components();
                            self.tui.bottom_bar.status = format!("已创建会话: {}", name);
                        }
                    } else {
                        self.tui.bottom_bar.status = "未找到工作区目录".to_string();
                    }
                }
            }

            Action::DeleteWorkspace(index) => {
                if let Some(ws) = self.tui.workspaces.get(index) {
                    let sessions_dir = pi_sessions_dir();
                    let ws_cwd = ws.cwd.clone();
                    let ws_display = ws.display_name.clone();
                    // 通过 cwd 编码匹配目录
                    if let Some(ws_dir) =
                        std::fs::read_dir(&sessions_dir).ok().and_then(|entries| {
                            entries.flatten().find_map(|e| {
                                let p = e.path();
                                if p.is_dir()
                                    && dir_matches_cwd(
                                        &p.file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default(),
                                        &ws_cwd,
                                    )
                                {
                                    Some(p)
                                } else {
                                    None
                                }
                            })
                        })
                    {
                        if let Err(e) = std::fs::remove_dir_all(&ws_dir) {
                            tracing::error!("删除工作区目录失败: {} — {}", ws_dir.display(), e);
                            self.tui.bottom_bar.status = format!("删除失败: {}", e);
                        } else {
                            self.populate_workspaces();
                            self.sync_components();
                            self.tui.bottom_bar.status = format!("已删除工作区: {}", ws_display);
                        }
                    }
                }
            }

            Action::DeleteSession {
                workspace_index,
                session_index,
            } => {
                if let Some(ws) = self.tui.workspaces.get(workspace_index) {
                    if let Some(sess) = ws.sessions.get(session_index) {
                        if let Some(ref path) = sess.file_path {
                            let name = sess.name.clone();
                            if let Err(e) = std::fs::remove_file(path) {
                                tracing::error!("删除会话文件失败: {} — {}", path, e);
                                self.tui.bottom_bar.status = format!("删除失败: {}", e);
                            } else {
                                self.populate_workspaces();
                                self.sync_components();
                                self.tui.bottom_bar.status = format!("已删除会话: {}", name);
                            }
                        }
                    }
                }
            }

            Action::RenameWorkspace {
                index,
                ref new_name,
            } => {
                if let Some(ws) = self.tui.workspaces.get(index) {
                    let old_cwd = ws.cwd.clone();
                    let old_display = ws.display_name.clone();
                    let sessions_dir = pi_sessions_dir();
                    if let Some(ws_dir) =
                        std::fs::read_dir(&sessions_dir).ok().and_then(|entries| {
                            entries.flatten().find_map(|e| {
                                let p = e.path();
                                if p.is_dir()
                                    && dir_matches_cwd(
                                        &p.file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default(),
                                        &old_cwd,
                                    )
                                {
                                    Some(p)
                                } else {
                                    None
                                }
                            })
                        })
                    {
                        let new_encoded = encode_workspace_name(new_name);
                        let new_dir = sessions_dir.join(&new_encoded);
                        if let Err(e) = std::fs::rename(&ws_dir, &new_dir) {
                            tracing::error!(
                                "重命名工作区目录失败: {} -> {} — {}",
                                ws_dir.display(),
                                new_dir.display(),
                                e
                            );
                            self.tui.bottom_bar.status = format!("重命名失败: {}", e);
                        } else {
                            self.populate_workspaces();
                            self.sync_components();
                            self.tui.bottom_bar.status =
                                format!("「{}」→「{}」", old_display, new_name);
                        }
                    }
                }
            }

            Action::RenameSession {
                workspace_index,
                session_index,
                ref new_name,
            } => {
                if let Some(ws) = self.tui.workspaces.get(workspace_index) {
                    if let Some(sess) = ws.sessions.get(session_index) {
                        let old_name = sess.name.clone();
                        // 仅更新数据模型中的名称（不修改 JSONL 文件内容）
                        if let Some(w) = self.tui.workspaces.get_mut(workspace_index) {
                            if let Some(s) = w.sessions.get_mut(session_index) {
                                s.name = new_name.clone();
                            }
                        }
                        self.sync_components();
                        self.tui.bottom_bar.status = format!("「{}」→「{}」", old_name, new_name);
                    }
                }
            }

            Action::AiRenameSession {
                workspace_index,
                session_index,
            } => {
                if let Some(ws) = self.tui.workspaces.get(workspace_index) {
                    if let Some(sess) = ws.sessions.get(session_index) {
                        if let Some(ref path) = sess.file_path {
                            match std::fs::read_to_string(path) {
                                Ok(content) => {
                                    let prompt = build_ai_rename_prompt(&content, &sess.name);
                                    self.tui.pending_ai_prompt = Some(prompt);
                                }
                                Err(e) => {
                                    tracing::error!("读取会话文件失败: {} — {}", path, e);
                                }
                            }
                        }
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
            tracing::debug!(
                "SYNC agent={} msgs={} loaded",
                agent_id,
                self.tui.main_view.messages.len()
            );
        } else {
            tracing::debug!("SYNC agent={} NOT FOUND in messages map", agent_id);
        }
    }

    /// 渲染前同步所有组件数据
    fn sync_components(&mut self) {
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
        self.tui
            .sidebar
            .current_model
            .clone_from(&self.tui.current_model);
        self.tui.sidebar.port = self.tui.router_port;
        self.tui.sidebar.active_provider_idx = self.tui.active_provider_idx;
        // Clamp provider_cursor to valid range（含 add provider 行）
        let max_provider = self.tui.providers.len();
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
                    .title(format!(
                        " {} {} ",
                        if p.bridge { "\u{1f517}" } else { "\u{25c6}" },
                        p.name
                    ))
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
                let is_std = !editor.draft.bridge;
                let (std_fg, brg_fg) = if is_std {
                    (theme.success, theme.text_dim)
                } else {
                    (theme.text_dim, theme.success)
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

                // 模式条
                {
                    let f_mode = editor.field_focus == 4;
                    let mode_label = if f_mode {
                        " ▎模式".to_string()
                    } else {
                        "  模式".to_string()
                    };
                    ln.push(Line::from(
                        Span::from(mode_label)
                            .fg(if f_mode { theme.accent } else { theme.text_dim })
                            .bold(),
                    ));
                    ln.push(Line::from(vec![
                        Span::from(format!("  {} 标准模式", if is_std { "◉" } else { "○" }))
                            .fg(std_fg)
                            .bold(),
                        Span::from("  │  ").fg(theme.border_dim),
                        Span::from(format!("{} 桥接模式", if !is_std { "◉" } else { "○" }))
                            .fg(brg_fg)
                            .bold(),
                        Span::from(if !is_std { " 🔗" } else { "" }).fg(brg_fg),
                    ]));
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
                    if secret && !is_std {
                        continue;
                    }
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
                let mode_hint = if editor.field_focus == 4 {
                    "  [Space] 切换模式"
                } else {
                    ""
                };
                ln.push(Line::from(vec![
                    Span::from(" "),
                    Span::from("[✓ 保存]").fg(theme.heading).bold(),
                    Span::from("  "),
                    Span::from("[取消]").fg(theme.text),
                    Span::from(format!(
                        "{}  [Tab] 字段  [Enter] 保存  [Esc] 取消",
                        mode_hint
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

impl App {
    /// 从当前 TUI 状态构建持久化数据结构
    pub fn build_persist_state(&self) -> crate::persistence::UiPersistState {
        crate::persistence::UiPersistState {
            active_session_id: self.active_agent.clone(),
            expanded_workspaces: self
                .tui
                .workspaces
                .iter()
                .filter(|ws| ws.expanded)
                .map(|ws| ws.cwd.clone())
                .collect(),
        }
    }

    /// 从 state.json 恢复工作区展开状态和活跃会话
    ///
    /// 应在 `populate_workspaces()` 之后调用。
    pub fn restore_persisted_state(&mut self) {
        let state = crate::persistence::load();

        // 恢复工作区展开状态
        for ws in &mut self.tui.workspaces {
            ws.expanded = state.expanded_workspaces.contains(&ws.cwd);
        }

        // 恢复活跃会话
        if let Some(ref session_id) = state.active_session_id {
            // 验证会话是否仍然存在
            let session_exists = self
                .tui
                .workspaces
                .iter()
                .any(|ws| ws.sessions.iter().any(|s| s.id == *session_id));
            if session_exists {
                tracing::info!("恢复上次活跃会话: {}", session_id);
                self.tui.active_session = session_id.clone();
                // 查找 file_path 并加载消息
                if let Some(path) = self.tui.workspaces.iter().find_map(|ws| {
                    ws.sessions
                        .iter()
                        .find(|s| s.id == *session_id)
                        .and_then(|s| s.file_path.clone())
                }) {
                    match load_session_messages(&path) {
                        Ok(messages) => {
                            self.messages.insert(session_id.clone(), messages);
                            self.active_agent = Some(session_id.clone());
                            self.session.id = session_id.clone();
                            self.session.file_path = Some(path);
                            if let Some(ws) = self
                                .tui
                                .workspaces
                                .iter()
                                .find(|ws| ws.sessions.iter().any(|s| s.id == *session_id))
                            {
                                if let Some(sess) = ws.sessions.iter().find(|s| s.id == *session_id)
                                {
                                    self.session.name = Some(sess.name.clone());
                                }
                            }
                            self.sync_messages_to_main_view(session_id);
                            self.sync_components();
                        }
                        Err(e) => {
                            tracing::warn!("恢复会话消息失败: {} — {}", path, e);
                            // 加载失败时仍设置 active_agent 和同步组件以保持 UI 一致
                            self.active_agent = Some(session_id.clone());
                            self.session.id = session_id.clone();
                            self.sync_components();
                        }
                    }
                }
            } else {
                tracing::info!("上次活跃会话已不存在，不恢复");
            }
        }
    }

    /// 从 session 目录扫描并填充工作区数据
    ///
    /// 从每个目录下的第一个 JSONL 文件提取 `cwd` 字段作为工作区标识，
    /// 无 JSONL 或无法提取 cwd 的目录不显示。同名末端目录自动加父级区分。
    /// 已有工作区的展开状态在重建时保留。
    pub fn populate_workspaces(&mut self) {
        let sessions_dir = pi_sessions_dir();
        let mut ws_map: std::collections::HashMap<String, WorkspaceNode> =
            std::collections::HashMap::new();

        // 保存当前工作区的展开状态（按 cwd 索引）
        let old_expanded: std::collections::HashMap<String, bool> = self
            .tui
            .workspaces
            .iter()
            .map(|ws| (ws.cwd.clone(), ws.expanded))
            .collect();

        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                // 扫描目录中的 JSONL 文件
                let mut jsonl_files: Vec<std::path::PathBuf> = Vec::new();
                if let Ok(file_entries) = std::fs::read_dir(&path) {
                    for fe in file_entries.flatten() {
                        let fp = fe.path();
                        if fp.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            jsonl_files.push(fp);
                        }
                    }
                }
                // 无 JSONL 的目录不显示
                if jsonl_files.is_empty() {
                    continue;
                }

                // 从第一个 JSONL 文件提取 cwd
                let cwd = jsonl_files
                    .first()
                    .and_then(|fp| extract_workspace_cwd(fp))
                    .unwrap_or_default();
                if cwd.is_empty() {
                    continue;
                }

                // 构建会话列表
                let mut sessions = Vec::new();
                for fp in &jsonl_files {
                    let id = fp
                        .file_stem()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let name =
                        extract_session_name(fp).unwrap_or_else(|| "New Session".to_string());
                    let message_count = if let Ok(content) = std::fs::read_to_string(fp) {
                        content.lines().count()
                    } else {
                        0
                    };
                    sessions.push(SessionNode {
                        id,
                        name,
                        file_path: Some(fp.to_string_lossy().to_string()),
                        message_count,
                        is_online: false,
                    });
                }
                sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));

                // 按 cwd 去重合并：同一 cwd 的多个目录合并会话
                if let Some(existing) = ws_map.get_mut(&cwd) {
                    existing.sessions.extend(sessions);
                    existing
                        .sessions
                        .sort_by(|a, b| b.message_count.cmp(&a.message_count));
                } else {
                    ws_map.insert(
                        cwd.clone(),
                        WorkspaceNode {
                            cwd: cwd.clone(),
                            display_name: String::new(), // 稍后计算
                            sessions,
                            // 已有工作区保留原展开状态，新工作区默认展开
                            expanded: *old_expanded.get(&cwd).unwrap_or(&true),
                        },
                    );
                }
            }
        }

        // 转换为 Vec 并计算 display_name
        let mut workspaces: Vec<WorkspaceNode> = ws_map.into_values().collect();
        compute_display_names(&mut workspaces);
        workspaces.sort_by(|a, b| a.display_name.cmp(&b.display_name));

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
                            mcps.push(McpInfo {
                                name: name.clone(),
                                command,
                                tool_count: 0,
                            });
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
/// 从 JSONL 文件第一行提取 `cwd` 字段
fn extract_workspace_cwd(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?;
    let val: serde_json::Value = serde_json::from_str(first_line).ok()?;
    val.get("cwd")?.as_str().map(String::from)
}

/// 为一组工作区计算去重后的 display_name
fn compute_display_names(workspaces: &mut [WorkspaceNode]) {
    let mut names: Vec<String> = workspaces
        .iter()
        .map(|ws| {
            std::path::Path::new(&ws.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| ws.cwd.clone())
        })
        .collect();

    loop {
        let mut dup_set = std::collections::HashSet::new();
        let mut has_dup = false;
        for n in &names {
            if !dup_set.insert(n.clone()) {
                has_dup = true;
                break;
            }
        }
        if !has_dup {
            break;
        }
        let mut count_map: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for n in &names {
            *count_map.entry(n.clone()).or_insert(0) += 1;
        }
        for (i, ws) in workspaces.iter().enumerate() {
            if *count_map.get(&names[i]).unwrap_or(&0) > 1 {
                let path = std::path::Path::new(&ws.cwd);
                if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                    names[i] = format!("{}/{}", parent.to_string_lossy(), names[i]);
                }
            }
        }
    }

    for (i, ws) in workspaces.iter_mut().enumerate() {
        ws.display_name = names[i].clone();
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

/// 从 pi session JSONL 文件中加载所有消息
///
/// pi session 文件格式为 JSONL，每行一个 JSON 对象：
/// ```json
/// {"type": "message", "message": {"role": "user", "content": [{"type": "text", "text": "..."}]}}
/// ```
fn load_session_messages(path: &str) -> std::result::Result<Vec<ChatMessage>, String> {
    use crate::message::{ChatRole, ContentBlock, ToolCallInfo, ToolStatus};

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(format!("读取文件失败: {}", e)),
    };
    let mut messages: Vec<ChatMessage> = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("第 {} 行 JSON 解析失败: {} — 跳过", line_num + 1, e);
                continue;
            }
        };

        // 只处理 type="message" 的行
        if val.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }

        let msg = match val.get("message") {
            Some(m) => m,
            None => continue,
        };

        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let chat_role = match role {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            "toolResult" => ChatRole::Tool,
            "system" => ChatRole::System,
            _ => continue,
        };

        let msg_id = msg
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // 解析 content 内容块数组
        let content_blocks: Vec<ContentBlock> = msg
            .get("content")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|block| {
                        let block_type = block.get("type").and_then(|v| v.as_str())?;
                        match block_type {
                            "text" => {
                                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                Some(ContentBlock::Text {
                                    text: text.to_string(),
                                })
                            }
                            "thinking" => {
                                let thinking =
                                    block.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                                Some(ContentBlock::Thinking {
                                    thinking: thinking.to_string(),
                                })
                            }
                            "toolCall" => {
                                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let arguments = block.get("arguments").cloned().unwrap_or_default();
                                let result = block.get("result").cloned();
                                let is_error = block
                                    .get("is_error")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                Some(ContentBlock::ToolCall {
                                    id: id.to_string(),
                                    name: name.to_string(),
                                    arguments,
                                    result,
                                    is_error,
                                })
                            }
                            _ => None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 构建显示文本
        let mut text = String::new();
        let mut thinking: Option<String> = None;
        let mut tool_call: Option<ToolCallInfo> = None;

        match chat_role {
            ChatRole::User => {
                // 从第一个 text 块提取文本
                text = content_blocks
                    .iter()
                    .find_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
            }
            ChatRole::Assistant => {
                let mut parts: Vec<String> = Vec::new();
                for block in &content_blocks {
                    match block {
                        ContentBlock::Text { text: t } => {
                            parts.push(t.clone());
                        }
                        ContentBlock::Thinking { thinking: th } => {
                            thinking = Some(th.clone());
                        }
                        ContentBlock::ToolCall {
                            name,
                            arguments: _,
                            result: _,
                            is_error: _,
                            ..
                        } => {
                            parts.push(format!("[tool: {}]", name));
                        }
                        _ => {}
                    }
                }
                text = parts.join("");
            }
            ChatRole::Tool => {
                // toolResult — 查找 tool_call_id 和内容
                let tool_use_id = msg
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_name = "tool".to_string();
                let result_content = content_blocks
                    .iter()
                    .find_map(|b| {
                        if let ContentBlock::Text { text: t } = b {
                            Some(t.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                text = result_content.clone();
                tool_call = Some(ToolCallInfo {
                    tool_name,
                    tool_call_id: tool_use_id.to_string(),
                    status: ToolStatus::Done,
                    args: serde_json::json!({}),
                    result: Some(serde_json::json!({"content": result_content})),
                    detail_text: String::new(),
                });
            }
            _ => {}
        }

        let timestamp = msg.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);

        let final_role = if chat_role == ChatRole::User
            && text.starts_with("Task: You are a delegated subagent")
        {
            // pi 内部 subagent 任务描述，不是用户输入，按 System 渲染
            ChatRole::System
        } else {
            chat_role
        };

        messages.push(ChatMessage {
            id: msg_id,
            content: if final_role == ChatRole::Tool {
                vec![]
            } else {
                content_blocks
            },
            agent_id: String::new(),
            role: final_role,
            text,
            thinking,
            tool_call,
            timestamp,
            meta: None,
        });
    }

    Ok(messages)
}

/// 使用 similar crate 计算统一 diff
fn compute_diff_lines(old_text: &str, new_text: &str) -> Vec<crate::message::DiffLine> {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let (kind, old_line, new_line) = match change.tag() {
            ChangeTag::Delete => ('-', Some(change.old_index().unwrap_or(0) + 1), None),
            ChangeTag::Insert => ('+', None, Some(change.new_index().unwrap_or(0) + 1)),
            ChangeTag::Equal => (
                ' ',
                Some(change.old_index().unwrap_or(0) + 1),
                Some(change.new_index().unwrap_or(0) + 1),
            ),
        };
        lines.push(crate::message::DiffLine {
            kind,
            old_line,
            new_line,
            text: change.value().to_string(),
        });
    }
    lines
}

/// 将用户友好的工作区名称编码为 pi 目录格式
/// 将文件系统路径编码为 pi sessions 目录名
///
/// pi 编码规则：`/` → `-`，前后加 `--`。
/// 例如 `/home/hr/Projects/agent-tui` → `--home-hr-Projects-agent-tui--`
fn encode_workspace_name(path: &str) -> String {
    format!("--{}--", path.replace('/', "-"))
}

/// 将 cwd 与 sessions 目录名匹配
///
/// 检查 `dir_name` 是否对应给定的 `cwd` 路径。
fn dir_matches_cwd(dir_name: &str, cwd: &str) -> bool {
    encode_workspace_name(cwd) == dir_name
}

/// 生成简单的 UUID（8 位 hex）
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", ts % 0xFFFFFFFFFFFFFFFFu128)
}

/// 构建 AI 重命名 prompt
///
/// 从会话 JSONL 内容中提取最近的消息，构造分析 prompt
fn build_ai_rename_prompt(session_content: &str, current_name: &str) -> String {
    let lines: Vec<&str> = session_content.lines().collect();
    // 取最近 30 行
    let recent: Vec<&str> = if lines.len() > 30 {
        lines[lines.len() - 30..].to_vec()
    } else {
        lines
    };
    let _truncated = recent.join("\n");

    // 提取用户问题和助手回复的关键信息
    let mut user_texts: Vec<String> = Vec::new();
    for line in &recent {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(msg) = v.get("message") {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" {
                    if let Some(text) = msg
                        .get("content")
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|block| block.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        let t = text.trim();
                        if t.len() > 80 {
                            user_texts.push(format!("{}…", &t[..80]));
                        } else {
                            user_texts.push(t.to_string());
                        }
                    }
                }
            }
        }
    }

    let user_summary = user_texts.join("\n  ");

    format!(
        "请为以下对话生成 3 个候选会话名称（简短、描述性的英文名，用连字符连接）。当前名称为「{current_name}」。\n\n对话概要:\n  {user_summary}\n\n请只输出 3 个候选名称，每行一个，不要编号或其他文字。"
    )
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
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

        // 第 1 行是活跃会话行（● demo-session），左侧有 1 列内边距
        assert_eq!(
            buffer[(1, 1)].symbol(),
            "●",
            "row 1 should start with active session dot (after left padding)"
        );

        // 验证三栏分隔符位置
        let separators: Vec<u16> = (0..120)
            .filter(|x| buffer[(*x, 0)].symbol() == "│")
            .collect();
        assert_eq!(separators, vec![39, 80], "separators at expected positions");

        assert_eq!(
            buffer[(81, 0)].symbol(),
            " ",
            "agent panel should have left padding space (col 81)"
        );

        // 验证底栏存在（TestBackend 创建为 120x32）
        let last_row = 31u16;
        let bottom = row_text(&terminal, last_row, 120);
        assert!(
            bottom.contains("Ctrl+C"),
            "bottom bar should show shortcuts"
        );
    }

    #[test]
    fn test_provider_enter_opens_popup() {
        // RED: handle_provider_key 还不存在，因此需要先写测试
        // 期望：在 Provider section 按下 Enter 时 provider_popup 变为 Some(0)
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            bridge: false,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![crate::provider::ModelInfo {
                id: "deepseek-chat".into(),
                name: "DeepSeek Chat".into(),
                context_window: 64000,
                reasoning: false,
                tier: "T2".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = None;
        state.provider_cursor = 0;

        state.handle_provider_key(crossterm::event::KeyCode::Enter);

        let editor = state
            .provider_editor
            .as_ref()
            .expect("Enter 应该打开 Provider 编辑表单");
        assert!(!editor.is_new, "编辑已有 provider 时 is_new 应为 false");
        assert_eq!(
            editor.draft.id, "deepseek",
            "编辑表单应预填充 provider 数据"
        );
        assert_eq!(state.provider_popup, None, "不应同时打开 popup");
    }

    #[test]
    fn test_provider_enter_on_empty_list_keeps_popup_closed() {
        let mut state = TuiState::new();
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = None;
        state.provider_cursor = 0;

        state.handle_provider_key(crossterm::event::KeyCode::Enter);

        assert_eq!(
            state.provider_popup, None,
            "无 provider 时 Enter 不应打开 popup"
        );
    }

    #[test]
    fn test_provider_popup_esc_closes() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            bridge: false,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = Some(0);

        // Esc 关闭 popup
        state.handle_provider_key(crossterm::event::KeyCode::Esc);
        assert_eq!(state.provider_popup, None, "Esc 应关闭 popup");
    }

    #[test]
    fn test_provider_popup_enter_activates_first_model() {
        let mut state = TuiState::new();
        state.current_model = "old-model".into();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            bridge: false,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![crate::provider::ModelInfo {
                id: "deepseek-chat".into(),
                name: "DeepSeek Chat".into(),
                context_window: 64000,
                reasoning: false,
                tier: "T2".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = Some(0);

        // Enter 激活第一个 model
        state.handle_provider_key(crossterm::event::KeyCode::Enter);
        assert_eq!(
            state.current_model, "deepseek-chat",
            "Enter 应激活第一个 model"
        );
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "Enter 应设置 active_provider_idx"
        );
        assert_eq!(state.provider_popup, None, "激活后 popup 应关闭");
    }

    #[test]
    fn test_provider_nav_down_up() {
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
            crate::provider::ProviderInfo {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
            crate::provider::ProviderInfo {
                id: "c".into(),
                name: "C".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;

        // Down → cursor=1
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 1, "Down 应移动到第二个 provider");

        // Down → cursor=2
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 2, "Down 应移动到第三个 provider");

        // Down → cursor=3 (add provider row)
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 3, "Down 应到 add provider 行");

        // Down → stays at 3 (end of list)
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 3, "Down 在末尾不应越界");

        // Up → cursor=2
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 2, "Up 应回到第三个");

        // Up → cursor=1
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 1, "Up 应回到第二个");

        // Up → cursor=0
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 0, "Up 应回到第一个");

        // Up → stays at 0 (start of list)
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 0, "Up 在开头不应越界");
    }

    #[test]
    fn test_model_nav_down_up_enter() {
        let mut state = TuiState::new();
        state.current_model = "m1".to_string(); // 预设当前模型
        state.active_provider_idx = Some(0); // 预设活跃 Provider
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 0;

        // Down 选择第二个 model
        state.handle_model_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.model_cursor, 1, "Down 应移动到第二个 model");

        // Enter 选中 model
        state.handle_model_key(crossterm::event::KeyCode::Enter);
        assert_eq!(state.current_model, "m2", "Enter 应切换到选中的 model");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "Enter 应保持当前活跃 Provider 不变"
        );
    }

    #[test]
    fn test_model_space_toggle_activate() {
        // Space 选中当前未选中的模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.active_provider_idx = Some(0);
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 1; // 光标在 m2 (未选中)

        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "m2", "Space 应切换到 m2");
        assert_eq!(state.active_provider_idx, Some(0), "不应切换 Provider");
        assert!(state.model_just_switched);
    }

    #[test]
    fn test_model_space_toggle_deactivate() {
        // Space 取消当前已选中的模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.active_provider_idx = Some(0);
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 0; // 光标在 m1 (已选中)

        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "", "Space 应取消选中");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "取消选中应保留 active_provider_idx（MODEL 区仍显示原 Provider 模型）"
        );
    }

    #[test]
    fn test_model_space_does_not_switch_provider_on_same_model_id() {
        // 两个 Provider 有同名模型，在 Provider 1 的 MODEL 区 Space 切换不应跳到 Provider 0
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "elysiver".into(),
                name: "Elysiver".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared-model".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared-model".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 0;
        // 当前在 deepseek Provider (索引 1) 的 MODEL 区
        state.active_provider_idx = Some(1);
        state.current_model = "shared-model".into();

        // Space 取消选中
        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "", "Space 应取消选中");

        // Space 重新选中 — 应保持在 deepseek (索引 1)
        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "shared-model");
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "同名模型不应导致 Provider 跳到第一个 (elysiver)"
        );
    }

    #[test]
    fn test_model_enter_does_not_switch_provider_on_same_model_id() {
        // Enter 在 MODEL 区不应切换 Provider
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "dup".into(),
                    name: "Dup".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "dup".into(),
                    name: "Dup".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 0;
        state.active_provider_idx = Some(1); // 在 Provider B
        state.current_model = "other".into();

        state.handle_model_key(crossterm::event::KeyCode::Enter);
        assert_eq!(state.current_model, "dup");
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "Enter 不应把 Provider 从 B 切到 A"
        );
    }

    #[test]
    fn test_provider_space_sets_active_provider_idx() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;

        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, Some(0));
        assert_eq!(state.current_model, "m1");
    }

    #[test]
    fn test_provider_space_toggle_deactivates() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;
        state.active_provider_idx = Some(0);
        state.current_model = "m1".into();

        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, None);
        assert_eq!(state.current_model, "");
    }

    #[test]
    fn test_provider_space_does_not_cross_affect() {
        // 两个 Provider，Space 切换 A 不应影响 B 的活跃状态判断
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "ma".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "mb".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;

        // Space 切换 Provider A
        state.provider_cursor = 0;
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, Some(0));
        assert_eq!(state.current_model, "ma");

        // 切换到 Provider B
        state.provider_cursor = 1;
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "Space 应激活 Provider B，而不是保持 A"
        );
        assert_eq!(state.current_model, "mb");

        // Provider A 已不再是活跃
        assert_ne!(state.active_provider_idx, Some(0));
    }

    #[test]
    fn test_provider_d_key_clears_active_idx_when_disabling_active() {
        let mut state = TuiState::new();
        state.persistence_disabled = true;
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;
        state.active_provider_idx = Some(0);
        state.current_model = "m1".into();

        // 禁用活跃 Provider
        state.handle_provider_key(crossterm::event::KeyCode::Char('d'));
        assert!(!state.providers[0].enabled, "Provider 应被禁用");
        assert_eq!(
            state.active_provider_idx, None,
            "禁用活跃 Provider 应清除 active_provider_idx"
        );
        assert_eq!(
            state.current_model, "",
            "禁用活跃 Provider 应清除 current_model"
        );
    }

    #[test]
    fn test_provider_d_key_keeps_active_idx_when_disabling_inactive() {
        // 禁用非活跃 Provider 不应清除 active_provider_idx
        let mut state = TuiState::new();
        state.persistence_disabled = true;
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "ma".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "mb".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.active_provider_idx = Some(0); // A 是活跃的
        state.current_model = "ma".into();

        // 禁用非活跃的 Provider B
        state.provider_cursor = 1;
        state.handle_provider_key(crossterm::event::KeyCode::Char('d'));
        assert!(!state.providers[1].enabled, "Provider B 应被禁用");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "禁用非活跃 Provider 不应清除 active_provider_idx"
        );
        assert_eq!(state.current_model, "ma", "current_model 不应被清除");
    }

    #[test]
    fn test_active_provider_for_models_prefers_idx() {
        // active_provider_for_models 应优先使用 active_provider_idx
        let mut state = TuiState::new();
        state.current_model = "shared".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "first".into(),
                name: "First".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "second".into(),
                name: "Second".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = Some(1); // 指定 second
        let ap = state.active_provider_for_models().unwrap();
        assert_eq!(ap.id, "second", "应按 idx 返回 second，不是 first");
    }

    #[test]
    fn test_active_provider_for_models_fallback_to_current_model() {
        // 无 active_provider_idx 时回退到 current_model 匹配
        let mut state = TuiState::new();
        state.current_model = "model-b".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "model-a".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "model-b".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = None;
        let ap = state.active_provider_for_models().unwrap();
        assert_eq!(ap.id, "b", "回退应按 current_model 匹配找到 Provider B");
    }

    #[test]
    fn test_active_provider_index_prefers_idx() {
        let mut state = TuiState::new();
        state.current_model = "shared".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "first".into(),
                name: "First".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "second".into(),
                name: "Second".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = Some(1);
        let idx = state.active_provider_index().unwrap();
        assert_eq!(idx, 1, "应返回 idx=1，不按 current_model 匹配到 0");
    }

    #[test]
    fn test_active_provider_index_fallback() {
        let mut state = TuiState::new();
        state.current_model = "m2".into();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "p".into(),
            name: "P".into(),
            enabled: true,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];

        state.active_provider_idx = None;
        let idx = state.active_provider_index().unwrap();
        assert_eq!(idx, 0, "回退应按 current_model='m2' 匹配到索引 0");
    }

    #[test]
    fn test_filtered_models_uses_active_provider() {
        // filtered_models 应从 active_provider_for_models 返回的 Provider 取模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                id: "p0".into(),
                name: "P0".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                id: "p1".into(),
                name: "P1".into(),
                enabled: true,
                bridge: false,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.active_provider_idx = Some(1); // 指定 p1

        let models = state.filtered_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "m2", "应从 p1 取模型，不是 p0");
    }

    #[test]
    fn test_handle_provider_key_space_on_disabled_provider_noop_via_guard() {
        // guard 条件已防止禁用/空 provider 触发 Space，此测试验证 guard 生效
        // disabled provider 的 cursor 仍可定位，但 Space 的 guard 检查列表非空+游标在范围内
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            id: "ds".into(),
            name: "DS".into(),
            enabled: false,
            bridge: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;
        state.current_model = "x".into();

        // Space guard: provider_cursor < providers.len() → true
        // 但 provider has no models, so "else if" branch doesn't fire
        // current_model should remain unchanged
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(
            state.current_model, "x",
            "无模型的 provider 不应切换 current_model"
        );
        assert_eq!(
            state.active_provider_idx, None,
            "无模型的 provider 不应设为活跃"
        );
    }

    #[test]
    fn test_toggle_block() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.tui.main_view.block_states.insert(
            "msg-1:0".to_string(),
            crate::message::BlockExpanded::Collapsed,
        );
        // 模拟 ToggleBlock
        let key = "msg-1:0".to_string();
        let state = app.tui.main_view.block_states.get_mut(&key).unwrap();
        *state = crate::message::BlockExpanded::Expanded;
        assert_eq!(*state, crate::message::BlockExpanded::Expanded);
    }

    #[test]
    fn test_exit_block() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.tui.main_view.entered_view = Some(crate::message::EnteredView::FullOutput {
            title: "test".into(),
            content: "hello".into(),
            scroll: 0,
        });
        app.tui.main_view.entered_view = None;
        assert!(app.tui.main_view.entered_view.is_none());
    }

    #[test]
    fn test_compute_diff_lines() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nline2_changed\nline3\nline4\n";
        let lines = compute_diff_lines(old, new);
        assert!(
            lines.iter().any(|l| l.kind == '-'),
            "should have deleted lines"
        );
        assert!(
            lines.iter().any(|l| l.kind == '+'),
            "should have added lines"
        );
        assert!(
            lines.iter().any(|l| l.kind == ' '),
            "should have unchanged lines"
        );
    }

    #[test]
    fn test_build_block_refs_empty() {
        let msgs: Vec<ChatMessage> = vec![];
        let refs = crate::message::build_block_refs(&msgs);
        assert!(refs.is_empty(), "empty messages should give no block refs");
    }

    #[test]
    fn test_build_block_refs_with_content() {
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![
            crate::message::ContentBlock::Thinking {
                thinking: "思考中...".to_string(),
            },
            crate::message::ContentBlock::Text {
                text: "回复内容".to_string(),
            },
            crate::message::ContentBlock::ToolCall {
                id: "tc-1".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
                result: None,
                is_error: false,
            },
        ];
        let msgs = vec![msg];
        let refs = crate::message::build_block_refs(&msgs);
        assert_eq!(
            refs.len(),
            2,
            "Thinking + ToolCall = 2 blocks, Text skipped"
        );
        assert_eq!(refs[0].kind, crate::message::BlockKind::Thinking);
        assert_eq!(refs[1].kind, crate::message::BlockKind::ToolCall);
    }

    #[test]
    fn test_tool_event_running_creates_message() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.active_agent = Some("agent-1".to_string());

        // 发送 Running 事件
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Running,
                args: Some(serde_json::json!({"command": "ls"})),
                result: None,
                is_error: false,
            })
            .await
            .ok();
        });

        let msgs = app.messages.get("agent-1").unwrap();
        assert_eq!(msgs.len(), 1, "should have 1 tool message");
        assert_eq!(msgs[0].role, crate::message::ChatRole::Tool);
    }

    #[test]
    fn test_tool_event_done_updates_existing() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.active_agent = Some("agent-1".to_string());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 先发送 Running
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Running,
                args: Some(serde_json::json!({"command": "ls"})),
                result: None,
                is_error: false,
            })
            .await
            .ok();

            // 再发送 Done（同一 tool_call_id）
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Done,
                args: None,
                result: Some(serde_json::json!({"content": "ok"})),
                is_error: false,
            })
            .await
            .ok();
        });

        let msgs = app.messages.get("agent-1").unwrap();
        assert_eq!(
            msgs.len(),
            1,
            "should still be 1 tool message (updated, not duplicated)"
        );
        assert_eq!(
            msgs[0].tool_call.as_ref().unwrap().status,
            crate::message::ToolStatus::Done
        );
        assert!(msgs[0].text.contains("✓"), "Done should show ✓");
    }
}
