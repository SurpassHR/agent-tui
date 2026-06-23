use serde_json::Value;

use crate::message::{ChatMessage, ToolStatus};

/// 全局通信动作枚举 — 所有组件间的消息传递骨架
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    // --- PTY 基础流（阶段一，待移除） ---
    /// 子进程的普通标准输出（即将废弃）
    #[deprecated(note = "将迁移到 MessageAppend")]
    PtyStdout(String),
    /// 子进程已退出（即将废弃）
    #[deprecated(note = "将迁移到 AgentStatusChange")]
    PtyExit,

    // --- 用户交互 ---
    /// 用户在底部输入框提交指令
    UserSubmitInput(String),

    // --- 拦截器外挂能力预留（待移除） ---
    /// 拦截到任务阶段更新（即将废弃）
    #[deprecated(note = "结构化事件将直接提供此数据")]
    InterceptedTask(String),
    /// 拦截到结构化简介（即将废弃）
    #[deprecated(note = "结构化事件将直接提供此数据")]
    InterceptedPopup { title: String, content: String },
    /// 拦截到 Token 统计（即将废弃）
    #[deprecated(note = "将迁移到 RuntimeStateUpdate")]
    UpdateTokenUsage { input: u32, output: u32 },

    // --- RPC 事件（Phase 4 新增） ---
    /// 流式追加助手消息文本（打字机效果）
    MessageAppend { agent_id: String, text: String },
    /// 助手消息定型完成
    MessageFinalize { agent_id: String },
    /// 流式追加思考过程文本
    ThinkingAppend { agent_id: String, text: String },
    /// 思考过程完成
    ThinkingFinalize { agent_id: String, text: String },
    /// 工具执行事件（start/update/end）
    ToolEvent {
        agent_id: String,
        tool_name: String,
        tool_call_id: String,
        status: ToolStatus,
        args: Option<Value>,
        result: Option<Value>,
        is_error: bool,
    },
    /// Agent 状态变更
    AgentStatusChange {
        agent_id: String,
        status: super::app::AgentStatus,
    },
    /// 运行时状态更新（模型、token、cost）
    RuntimeStateUpdate(super::app::AgentRuntimeState),
    /// 历史消息加载完成
    AgentMessagesLoaded {
        agent_id: String,
        messages: Vec<ChatMessage>,
    },
    /// 自动重试状态消息
    AutoRetryStatus { agent_id: String, text: String },

    // --- UI 交互控制 ---
    /// 切换浮窗显示状态
    TogglePopup,
    /// 切换左侧会话
    SwitchSession(String),

    // --- 侧边栏交互（Workspace 树） ---
    /// 切换指定索引的工作区展开/折叠
    ToggleWorkspace(usize),
    /// 选择指定 ID 的会话
    SelectSession(String),
    /// 侧边栏焦点移动（正=下，负=上）
    SidebarMove(i32),
    /// 面板焦点循环（1=向右，-1=向左）
    CycleFocusPanel(i32),
    /// 子区焦点切换（1=向下，-1=向上）
    CycleFocusSubsection(i32),
}
