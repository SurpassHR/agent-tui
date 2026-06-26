#[allow(unused_imports)]
pub use crate::state::*;

use crate::action::Action;
use crate::backend::AgentBackend;
use crate::errors::Result;
use crate::message::{ChatMessage, ContentBlock};
use crate::utils::*;
use std::collections::HashMap;
use tokio::sync::mpsc;

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
    /// 当前活跃 agent（浏览会话 ID）
    pub active_agent: Option<String>,
    /// 活跃会话 ID 集合（is_online 绿点依据）
    pub active_sessions: std::collections::HashSet<String>,
    /// 多 agent 进程管理器
    pub agent_manager: crate::backend::agent_manager::AgentManager,
    /// agent 事件统一转发 channel（由 run_tui 创建，供 ConnectSession 使用）
    pub event_tx:
        Option<tokio::sync::mpsc::UnboundedSender<(String, crate::backend::event::PiEvent)>>,
    /// 默认 pi 进程的事件转发目标 agent_id（运行时动态更新）
    pub forwarding_agent_tx: Option<tokio::sync::watch::Sender<String>>,
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
            active_sessions: std::collections::HashSet::new(),
            agent_manager: crate::backend::agent_manager::AgentManager::new(),
            event_tx: None,
            forwarding_agent_tx: None,
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
            active_sessions: std::collections::HashSet::new(),
            agent_manager: crate::backend::agent_manager::AgentManager::new(),
            event_tx: None,
            forwarding_agent_tx: None,
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

            Action::ThinkingStart { agent_id } => {
                self.start_new_assistant_thinking(&agent_id);
            }

            Action::ThinkingFinalize { agent_id, text } => {
                self.finalize_thinking(&agent_id, &text);
            }

            Action::ContentUpdate { agent_id, content } => {
                tracing::debug!(
                    "HANDLE ContentUpdate agent={} blocks={}",
                    agent_id,
                    content.len()
                );
                // 更新最后一条 Assistant 消息的 content 数组；
                // 若不存在则创建（处理首帧 message_update 不含 delta 的情况）
                let msgs = self.messages.entry(agent_id.clone()).or_default();
                if let Some(last) = tail_assistant_mut(msgs) {
                    last.content = content;
                } else {
                    let mut msg = ChatMessage::assistant(&agent_id, "");
                    msg.content = content;
                    msgs.push(msg);
                }
                self.sync_messages_to_main_view(&agent_id);
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

            Action::ConnectSession {
                session_id,
                file_path,
            } => {
                tracing::info!("连接到会话: {}", session_id);
                // 启动 pi agent 进程
                let cwd = std::env::current_dir().unwrap_or_default();
                let event_tx = self.event_tx.clone().unwrap_or_else(|| {
                    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                    tx
                });
                match self
                    .agent_manager
                    .spawn(
                        session_id.clone(),
                        std::path::PathBuf::from(&file_path),
                        cwd,
                        event_tx,
                    )
                    .await
                {
                    Ok(()) => {
                        self.active_sessions.insert(session_id.clone());
                        // 发送 set_model 给新 pi 进程
                        let model = self.tui.current_model.clone();
                        if !model.is_empty() {
                            if let Some(session_client) = self.agent_manager.client_mut(&session_id)
                            {
                                let _ = session_client
                                    .notify(serde_json::json!({
                                        "type": "set_model",
                                        "provider": "local",
                                        "modelId": model,
                                    }))
                                    .await;
                            }
                        }
                        // 从工作区树获取会话名称
                        let name = self
                            .tui
                            .workspaces
                            .iter()
                            .find_map(|ws| ws.sessions.iter().find(|s| s.id == session_id))
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        self.session.name = Some(name.clone());
                        // 同时切换到浏览该会话
                        self.active_agent = Some(session_id.clone());
                        if let Ok(msgs) = load_session_messages(&file_path) {
                            self.messages.insert(session_id.clone(), msgs);
                        }
                        self.sync_messages_to_main_view(&session_id);
                        self.sync_components();
                        self.tui.bottom_bar.status = format!("已连接: {}", name);
                    }
                    Err(e) => {
                        self.tui.bottom_bar.status = format!("连接失败: {}", e);
                    }
                }
                crate::persistence::save(&self.build_persist_state());
            }

            Action::DisconnectSession(session_id) => {
                self.agent_manager.kill(&session_id).await;
                self.active_sessions.remove(&session_id);
                self.sync_components();
                let name = self
                    .tui
                    .workspaces
                    .iter()
                    .find_map(|ws| ws.sessions.iter().find(|s| s.id == session_id))
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| session_id.clone());
                self.tui.bottom_bar.status = format!("已断开: {}", name);
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
                tracing::info!("CreateSession: ws_idx={} name={}", workspace_index, name);
                if let Some(ws) = self.tui.workspaces.get(workspace_index) {
                    let sessions_dir = pi_sessions_dir();
                    let ws_encoded = encode_workspace_name(&ws.cwd);
                    tracing::info!("CreateSession: cwd={} encoded={}", ws.cwd, ws_encoded);
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
                        tracing::info!(
                            "CreateSession: spawning pi agent session_id={} path={}",
                            session_id,
                            session_path.display()
                        );
                        // 预创建 JSONL 文件（写入 session metadata），确保重启后 populate_workspaces 可发现
                        // pi agent spawn 后也会写入完整格式，此处仅保证文件立刻存在
                        if let Some(parent) = session_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let header = serde_json::json!({
                            "type": "session",
                            "version": 3,
                            "id": session_id,
                            "timestamp": format!(
                                "2026-06-25T00:00:00.{}Z",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    % 1000
                            ),
                            "cwd": ws.cwd,
                        });
                        if let Ok(header_str) = serde_json::to_string(&header) {
                            let _ = std::fs::write(&session_path, format!("{}\n", header_str));
                        }
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let event_tx = self.event_tx.clone().unwrap_or_else(|| {
                            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                            tx
                        });
                        match self
                            .agent_manager
                            .spawn(session_id.clone(), session_path.clone(), cwd, event_tx)
                            .await
                        {
                            Ok(()) => {
                                tracing::info!("CreateSession: spawn 成功");
                                self.active_sessions.insert(session_id.clone());
                                // 发送 set_model 给新 pi 进程（否则 pi 不知道用哪个模型）
                                let model = self.tui.current_model.clone();
                                if !model.is_empty() {
                                    if let Some(session_client) =
                                        self.agent_manager.client_mut(&session_id)
                                    {
                                        let _ = session_client
                                            .notify(serde_json::json!({
                                                "type": "set_model",
                                                "provider": "local",
                                                "modelId": model,
                                            }))
                                            .await;
                                    }
                                }
                                // 名称暂存到 session_names（供 extract_session_name 查找）
                                self.tui
                                    .session_names
                                    .insert(session_id.clone(), name.clone());
                                // 直接在工作区数据结构中添加新 session 节点
                                // （不依赖 populate_workspaces 文件扫描，pi 可能还没写文件）
                                if let Some(ws) = self.tui.workspaces.get_mut(workspace_index) {
                                    ws.sessions.push(SessionNode {
                                        id: session_id.clone(),
                                        name: name.clone(),
                                        file_path: Some(session_path.to_string_lossy().to_string()),
                                        message_count: 0,
                                        is_online: true,
                                    });
                                }
                                // 切换浏览到新会话
                                self.active_agent = Some(session_id.clone());
                                self.messages.insert(session_id.clone(), Vec::new());
                                self.sync_messages_to_main_view(&session_id);
                                self.sync_components();
                                // 持久化：关闭后重启时恢复该会话
                                crate::persistence::save(&self.build_persist_state());
                                self.tui.bottom_bar.status = format!("已创建会话: {}", name);
                            }
                            Err(e) => {
                                tracing::error!("创建会话失败: {}", e);
                                self.tui.bottom_bar.status = format!("创建失败: {}", e);
                            }
                        }
                    } else {
                        tracing::warn!(
                            "CreateSession: 未找到工作区目录 sessions_dir={} encoded={}",
                            sessions_dir.display(),
                            ws_encoded
                        );
                        self.tui.bottom_bar.status = "未找到工作区目录".to_string();
                    }
                } else {
                    tracing::error!(
                        "CreateSession: workspace_index={} 越界, total={}",
                        workspace_index,
                        self.tui.workspaces.len()
                    );
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
                        let session_id = sess.id.clone();
                        // 仅更新数据模型中的名称（不修改 JSONL 文件内容）
                        if let Some(w) = self.tui.workspaces.get_mut(workspace_index) {
                            if let Some(s) = w.sessions.get_mut(session_index) {
                                s.name = new_name.clone();
                            }
                        }
                        // 持久化自定义名称，重启后恢复
                        self.tui.session_names.insert(session_id, new_name.clone());
                        crate::persistence::save(&self.build_persist_state());
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
        if let Some(msg) = tail_assistant_mut(msgs) {
            let content_already_has_delta =
                msg.text.is_empty() && text_content_ends_with(&msg.content, text);
            msg.text.push_str(text);
            if !content_already_has_delta {
                append_text_content_block(&mut msg.content, text);
            }
        } else {
            let mut msg = ChatMessage::assistant(agent_id, text);
            append_text_content_block(&mut msg.content, text);
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 追加 thinking 文本到最后一条 assistant 消息
    /// 如果不存在 assistant 消息，创建一个新的
    fn append_to_last_assistant_thinking(&mut self, agent_id: &str, text: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        if let Some(msg) = tail_assistant_mut(msgs) {
            let content_already_has_delta = msg.thinking.as_deref().unwrap_or_default().is_empty()
                && thinking_content_ends_with(&msg.content, text);
            let t = msg.thinking.get_or_insert_with(String::new);
            t.push_str(text);
            if !content_already_has_delta {
                append_thinking_content_block(&mut msg.content, text);
            }
        } else {
            let mut msg = ChatMessage::assistant(agent_id, "");
            msg.thinking = Some(text.to_string());
            append_thinking_content_block(&mut msg.content, text);
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 开始一个新的 thinking 块
    fn start_new_assistant_thinking(&mut self, agent_id: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        if let Some(msg) = tail_assistant_mut(msgs) {
            msg.thinking.get_or_insert_with(String::new);
            push_empty_thinking_content_block(&mut msg.content);
        } else {
            let mut msg = ChatMessage::assistant(agent_id, "");
            msg.thinking = Some(String::new());
            push_empty_thinking_content_block(&mut msg.content);
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 定型 thinking 文本
    fn finalize_thinking(&mut self, agent_id: &str, text: &str) {
        let msgs = self.messages.entry(agent_id.to_string()).or_default();
        if let Some(msg) = tail_assistant_mut(msgs) {
            if !text.is_empty() {
                msg.thinking = Some(text.to_string());
                replace_last_thinking_content_block(&mut msg.content, text);
            }
        } else if !text.is_empty() {
            let mut msg = ChatMessage::assistant(agent_id, "");
            msg.thinking = Some(text.to_string());
            append_thinking_content_block(&mut msg.content, text);
            msgs.push(msg);
        }
        self.sync_messages_to_main_view(agent_id);
    }

    /// 同步消息到 MainView
    pub(crate) fn sync_messages_to_main_view(&mut self, agent_id: &str) {
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
}

fn append_text_content_block(content: &mut Vec<ContentBlock>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(ContentBlock::Text { text: current }) = content.last_mut() {
        current.push_str(text);
    } else {
        content.push(ContentBlock::Text {
            text: text.to_string(),
        });
    }
}

fn tail_assistant_mut(messages: &mut [ChatMessage]) -> Option<&mut ChatMessage> {
    messages
        .last_mut()
        .filter(|message| matches!(message.role, crate::message::ChatRole::Assistant))
}

fn text_content_ends_with(content: &[ContentBlock], text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let current = content
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<String>();
    current.ends_with(text)
}

fn append_thinking_content_block(content: &mut Vec<ContentBlock>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(ContentBlock::Thinking { thinking }) = content.last_mut() {
        thinking.push_str(text);
    } else {
        content.push(ContentBlock::Thinking {
            thinking: text.to_string(),
        });
    }
}

fn push_empty_thinking_content_block(content: &mut Vec<ContentBlock>) {
    if matches!(
        content.last(),
        Some(ContentBlock::Thinking { thinking }) if thinking.is_empty()
    ) {
        return;
    }
    content.push(ContentBlock::Thinking {
        thinking: String::new(),
    });
}

fn thinking_content_ends_with(content: &[ContentBlock], text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    content
        .iter()
        .rev()
        .find_map(|block| {
            if let ContentBlock::Thinking { thinking } = block {
                Some(thinking.ends_with(text))
            } else {
                None
            }
        })
        .unwrap_or(false)
}

fn replace_last_thinking_content_block(content: &mut Vec<ContentBlock>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(block) = content
        .iter_mut()
        .rev()
        .find(|block| matches!(block, ContentBlock::Thinking { .. }))
    {
        if let ContentBlock::Thinking { thinking } = block {
            *thinking = text.to_string();
        }
    } else {
        append_thinking_content_block(content, text);
    }
}
