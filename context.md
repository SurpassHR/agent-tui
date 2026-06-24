# agent-tui 项目关键结构侦察报告

## 1. src/action.rs — Action 枚举所有 variant

**文件**: `src/action.rs` (完整)

所有 variant 列表：

```rust
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    // --- PTY 基础流（阶段一，待移除） ---
    #[deprecated] PtyStdout(String),       // 子进程普通输出
    #[deprecated] PtyExit,                 // 子进程退出

    // --- 用户交互 ---
    UserSubmitInput(String),               // 用户在底部输入框提交指令

    // --- 拦截器外挂（待移除） ---
    #[deprecated] InterceptedTask(String),
    #[deprecated] InterceptedPopup { title: String, content: String },
    #[deprecated] UpdateTokenUsage { input: u32, output: u32 },

    // --- RPC 事件 ---
    MessageAppend { agent_id: String, text: String },
    MessageFinalize { agent_id: String },
    ThinkingAppend { agent_id: String, text: String },
    ThinkingFinalize { agent_id: String, text: String },
    ToolEvent { agent_id: String, tool_name: String, tool_call_id: String, status: ToolStatus, args: Option<Value>, result: Option<Value>, is_error: bool },
    AgentStatusChange { agent_id: String, status: AgentStatus },
    RuntimeStateUpdate(AgentRuntimeState),
    AgentMessagesLoaded { agent_id: String, messages: Vec<ChatMessage> },
    AutoRetryStatus { agent_id: String, text: String },

    // --- 内容块更新 ---
    ContentUpdate { agent_id: String, content: Vec<ContentBlock> },
    ToggleBlock { agent_id: String, msg_id: String, block_index: usize },
    EnterBlock { agent_id: String, msg_id: String, block_index: usize },
    ExitBlock,

    // --- UI 交互控制 ---
    TogglePopup,                           // 切换浮窗
    SwitchSession(String),                 // 切换左侧会话

    // --- ★ 侧边栏相关（Workspace 树） ---
    ToggleWorkspace(usize),                // 切换工作区展开/折叠
    SelectSession(String),                 // 选择指定 ID 的会话
    SidebarMove(i32),                      // 侧边栏焦点移动（正=下，负=上）
    CycleFocusPanel(i32),                  // 面板焦点循环（1=右，-1=左）
    CycleFocusSubsection(i32),             // 子区焦点切换（1=下，-1=上）
}
```

## 2. src/app.rs — handle_action 中 ToggleWorkspace 和 SelectSession

**行号 1431–1484**

```rust
// 行 1431
Action::ToggleWorkspace(index) => {
    if let Some(ws) = self.tui.workspaces.get_mut(index) {
        ws.expanded = !ws.expanded;
    }
}

// 行 1437
Action::SelectSession(session_id) => {
    self.tui.active_session = session_id.clone();
    // 查找 session 的 file_path
    let file_path = self.tui.workspaces.iter().find_map(|ws| {
        ws.sessions.iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.file_path.clone())
    });
    // ... 加载会话文件、同步消息到 MainView、sync_components ...
}
```

## 3. src/app.rs — TuiState 结构体定义（workspaces 字段）

**行号 196–291** (字段 `pub workspaces: Vec<WorkspaceNode>` 在行 218)

```rust
pub struct TuiState {
    pub active_session: String,              // ~行 197
    pub current_time: String,
    pub theme: Theme,

    // 组件实例
    pub sidebar: Sidebar,
    pub main_view: MainView,
    pub agent_panel: AgentPanel,
    pub bottom_bar: BottomBar,
    pub popup: Popup,

    // 工作区数据
    pub workspaces: Vec<WorkspaceNode>,      // ← 行 218
    pub focus_panel: FocusPanel,
    pub sidebar_cursor: usize,
    pub sidebar_subsection: SidebarSubsection,
    pub main_view_subsection: MainViewSubsection,
    pub agent_panel_subsection: AgentPanelSubsection,
    pub agent_cursor: usize,
    pub message_cursor: usize,
    pub scroll_mode: ScrollMode,
    pub selection: SelectionState,
    pub subagents: Vec<SubAgentInfo>,
    pub router_running: bool,
    pub router_port: u16,
    pub providers: Vec<crate::provider::ProviderInfo>,
    pub current_model: String,
    pub provider_cursor: usize,
    pub provider_popup: Option<usize>,
    pub model_cursor: usize,
    pub model_search: String,
    pub model_just_switched: bool,
    pub active_provider_idx: Option<usize>,
    pub provider_editor: Option<ProviderEditor>,
    pub shared_config: Option<crate::provider::SharedConfig>,
    pub models_fetch_rx: Option<tokio::sync::oneshot::Receiver<Option<String>>>,
    pub skills: Vec<SkillInfo>,
    pub skill_cursor: usize,
    pub mcps: Vec<McpInfo>,
    pub mcp_cursor: usize,
    pub persistence_disabled: bool,
}
```

## 4. src/app.rs — populate_workspaces 方法

**行号 2338–2380**

```rust
impl App {
    pub fn populate_workspaces(&mut self) {
        let sessions_dir = pi_sessions_dir();
        let mut workspaces = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() { continue; }
                let raw_name = path.file_name()...unwrap_or_default();
                let ws_name = decode_workspace_name(&raw_name);

                let mut sessions = Vec::new();
                if let Ok(file_entries) = std::fs::read_dir(&path) {
                    // 扫描每个 jsonl 文件作为 session
                    // 解析 session name、message_count
                }
                sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));
                let expanded = workspaces.is_empty(); // 第一个展开
                workspaces.push(WorkspaceNode { name: ws_name, sessions, expanded });
            }
        }
        workspaces.sort_by(|a, b| a.name.cmp(&b.name));
        self.tui.workspaces = workspaces;
    }
}
```

被调用位置：`tui.rs` 行 710 (`app.populate_workspaces()`)

## 5. src/tui.rs — 侧边栏键盘事件分发

**行号 593–639**

```rust
// Sidebar + Workspace 子区：↑/↓/Enter
_ if *focus == FocusPanel::Sidebar
    && app.tui.sidebar_subsection == SidebarSubsection::Workspace =>
{
    match key.code {
        KeyCode::Up => {
            app.handle_action(Action::SidebarMove(-1)).await.ok();
        }
        KeyCode::Down => {
            app.handle_action(Action::SidebarMove(1)).await.ok();
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let cursor = app.tui.sidebar_cursor;
            if let Some((is_ws, ws_idx, sess_idx)) = app.tui.sidebar_item_at(cursor) {
                if is_ws {
                    app.handle_action(Action::ToggleWorkspace(ws_idx)).await.ok();
                } else if let Some(si) = sess_idx {
                    // ... SelectSession
                }
            }
        }
        _ => {}
    }
}
```

其他 sidebar 子区：
- **Provider** (行 ~642): 委托 `app.tui.handle_provider_key(key.code)`
- **Model** (行 ~660): 委托 `app.tui.handle_model_key(key.code)` + 同步到 pi RPC

## 6. src/components/sidebar.rs — Sidebar 结构体和 render 方法

**行号 56–82** (结构体), **行 246–352** (render 方法)

Sidebar 结构体：
```rust
pub struct Sidebar {
    pub active_session: String,
    pub session_id: String,
    pub message_count: usize,
    pub workspaces: Vec<WorkspaceNode>,     // sync_components 时从 tui.workspaces 写入
    pub has_focus: bool,
    pub cursor: usize,
    pub subsection: SidebarSubsection,
    pub selection: SelectionState,
    pub providers: Vec<crate::provider::ProviderInfo>,
    pub router_running: bool,
    pub current_model: String,
    pub provider_cursor: usize,
    pub model_cursor: usize,
    pub model_search: String,
    pub model_scroll: usize,
    pub port: u16,
    pub active_provider_idx: Option<usize>,
}
```

render 方法（行 246）三区布局：
1. **顶部 ACTIVE SESSION** (固定高度 3)
2. **中部 WORKSPACE 树** (弹性，含内部滚动 `compute_ws_scroll`)
3. **底部 PROVIDER + MODEL 区块** (动态高度，最大 14 行)

## 7. src/components/popup.rs — Popup 结构体

**行号 14–22**

```rust
pub struct Popup {
    pub visible: bool,
    pub title: String,
    pub description: String,
}
```

仅有一种 popup 类型（通用文本弹窗），居中显示（宽 70%、高 45%）。
`centered_rect(percent_x, percent_y, area)` 辅助函数在行 55。

另有两个独立弹窗在 `app.rs` 的 `render_tui` 中直接渲染：
- **Provider 详情弹窗** (`provider_popup: Option<usize>`) — 行 ~1650
- **Provider 编辑表单弹窗** (`provider_editor: Option<ProviderEditor>`) — 行 ~1700
- **模型管理弹窗** (`editor.model_mgr`) — 行 ~1850

## 8. src/backend/rpc_client.rs — 发送 RPC 的方法签名

**文件**: `src/backend/rpc_client.rs`

`PiRpcClient` 的两个核心发送方法：

```rust
// 行 98
/// 发送请求并等待响应（带超时）
pub async fn request(
    &mut self,
    cmd: Value,
    timeout: std::time::Duration,
) -> Result<RpcResponse>

// 行 145
/// 发送通知（不需要等待响应）
pub async fn notify(&mut self, cmd: Value) -> Result<()>
```

## 9. src/backend/rpc.rs — PiRpcBackend（无 send_input 方法）

**文件**: `src/backend/rpc.rs`

`PiRpcBackend` **不实现** `AgentBackend` trait，因此**没有** `send_input` 方法。

- `PiRpcBackend::start()` (行 65) → 返回 `Result<PiRpcClient>`
- `PiRpcBackend::stop()` (行 162)
- `PiRpcBackend::check_installed()` (行 44)
- `PiRpcBackend::is_running()` (行 172)

用户输入通过 `PiRpcClient::notify()` 发送 type="prompt" 的 JSONL 消息，而非 `send_input`。

`AgentBackend` trait 中的 `send_input` 定义在 `backend/mod.rs` 行 24，但仅 PTY 模式（`start` 接收 `mpsc::Sender<Action>` 的旧模式）使用。

---

**结论**: 以上为所有请求的文件关键结构侦察。如需更深层的代码分析或修改计划，[建议升级到 T2 agent]。
