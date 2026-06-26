use std::collections::HashMap;

use crate::components::agent_panel::AgentPanel;
use crate::components::bottom_bar::BottomBar;
use crate::components::main_view::MainView;
use crate::components::popup::Popup;
use crate::components::sidebar::Sidebar;
use crate::theme::Theme;

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
    /// 侧边栏光标位置（0 = ACTIVE SESSION title / WORKSPACE title / ...）
    pub sidebar_cursor: usize,
    /// 活跃会话区选中光标（0 = ACTIVE SESSION title）
    pub active_session_cursor: usize,
    /// 上次 Enter 操作追踪（session_id, timestamp），用于双击检测
    pub last_enter_session: Option<(String, std::time::Instant)>,
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
    /// 手动创建 session 的名称映射（session_id → name，供 extract_session_name 查找）
    pub session_names: HashMap<String, String>,
    /// 标记下次渲染时保存 TUI 快照
    pub snapshot_pending: bool,
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
                thinking_level_map: None,
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

/// 编辑表单视觉焦点顺序：端点类型 → ID → 名称 → Base URL → API Key → 模型 → 桥接
const FIELD_ORDER: [usize; 7] = [6, 0, 1, 2, 3, 5, 4];
// 6=endpoint_type, 0=ID, 1=name, 2=base_url, 3=api_key, 5=models

pub fn prev_field(current: usize) -> usize {
    if let Some(pos) = FIELD_ORDER.iter().position(|&f| f == current) {
        FIELD_ORDER[(pos + FIELD_ORDER.len() - 1) % FIELD_ORDER.len()]
    } else {
        current
    }
}

pub fn next_field(current: usize) -> usize {
    if let Some(pos) = FIELD_ORDER.iter().position(|&f| f == current) {
        FIELD_ORDER[(pos + 1) % FIELD_ORDER.len()]
    } else {
        current
    }
}
