# Phase 4: RPC 驱动的 TUI 架构改造

> 把 pi 从「终端程序」降级为「后端 RPC 服务」，TUI 升级为完整控制层
> 日期：2026-06-23 | 状态：设计稿

---

## 1. 动机与目标

### 1.1 当前问题

目前 Meta-TUI 使用 PTY（伪终端）运行 pi agent：

```
用户 → TUI(终端模拟器) → PTY → pi(交互式终端)
                            ↓
                        MainView 直接显示 pi 的原始终端 ANSI 输出
```

TUI 本质上只是一个「带边框的终端模拟器」，存在以下根本性缺陷：

- **无结构化数据**：pi 的回答、思维链、工具调用全部混杂在 ANSI 流中
- **解析脆弱**：`OutputInterceptor` 需要用正则从终端输出中猜测结构化信息
- **交互受限**：TUI 无法自主控制交互，用户输入必须转发给 pi 的 PTY STDIN
- **侧面板空洞**：所有侧面板（任务状态、Token 统计）的"待接入"是因为数据根本无法从终端流中可靠提取

### 1.2 目标架构

```
用户 ←→ TUI(完整控制层) ──JSONL RPC──→ pi(纯后端推理引擎)
            ↑                               ↑
       结构化消息渲染                   结构化事件推送
       (ChatMessage[])                  (agent_start, message_update...)
```

核心转变：

| 维度 | 当前 (PTY) | 目标 (RPC) |
|------|-----------|-----------|
| 通信协议 | PTY (伪终端) | JSONL over stdio |
| 输出格式 | ANSI 转义码 | 结构化 JSON 事件 |
| 用户输入 | PTY stdin 转发 | RPC prompt 命令 |
| 消息渲染 | ANSI 文本直出 (`ansi::parse_to_lines`) | 消息列表 (`ChatMessage` 列表) |
| 工具调用 | 不可见 | `tool_execution_start/end` 事件 |
| 侧面板数据 | 静态占位，无真实数据 | `get_state` / `get_session_stats` RPC |

### 1.3 参考实现

PiDeck (`/media/hr/Data/Codes/PiDeck`) 使用 `pi --mode rpc` 实现了相同的架构：

- `PiRpcClient.ts` — JSONL RPC 协议客户端（stdin/stdout）
- `AgentManager.ts` — 事件状态机 + 消息管理
- `PiProcess.ts` — spawn `pi --mode rpc` + 进程生命周期

本设计受 PiDeck 启发，但用 Rust 重新实现，保持零额外运行时依赖。

---

## 2. 整体架构

```text
┌──────────────────────────────────────────────────────────────────┐
│                        Ratatui TUI                              │
│  ┌────────┐  ┌──────────────────────────┐  ┌────────────────┐   │
│  │ TopBar │  │   MainView               │  │ AgentPanel     │   │
│  │ model  │  │   ├─ ChatMessage[]       │  │ ├─ agent list  │   │
│  │ status │  │   ├─ MessageRenderer     │  │ ├─ tasks       │   │
│  ├────────┤  │   ├─ scroll / input      │  │ ├─ token usage │   │
│  │ Sidebar│  │   └──────────────────────┘  │ ├─ session     │   │
│  │ sess.  │  │                              │   stats       │   │
│  └────────┘  └──────────────────────────────┘ └────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │ BottomBar / InputArea: [输入框] [发送] [快捷键提示]          │  │
│  └──────────────────────────────────────────────────────────────┘  │
└────────────────────────────┬───────────────────────────────────────┘
                             │ Action MPSC Channel
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│  App (状态机)                                                    │
│  ├─ agents: HashMap<AgentId, AgentInfo>                          │
│  ├─ messages: HashMap<AgentId, Vec<ChatMessage>>                 │
│  ├─ active_agent: Option<AgentId>                                │
│  ├─ runtime: AgentRuntimeState                                   │
│  └─ handle_action(action) → 更新状态 → 触发重渲染                │
└────────────────────────────┬──────────────────────────────────────┘
                             │ JSONL RPC over stdio
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│  PiRpcBackend (替换 PiPtyBackend)                                │
│  ├─ spawn("pi", ["--mode", "rpc", ...session_args])              │
│  ├─ PiRpcClient                                                  │
│  │   ├─ request(cmd) → Promise<Response>  (同步回复)             │
│  │   └─ event stream → Action  (异步推送)                        │
│  └─ PiEventProcessor                                             │
│       ├─ agent_start       → Action::AgentStatusChange(running)  │
│       ├─ message_update    → Action::MessageAppend / Thinking    │
│       ├─ tool_execution_*  → Action::ToolEvent                   │
│       └─ agent_end         → Action::AgentStatusChange(idle)     │
└──────────────────────────────────────────────────────────────────┘
```

---

## 3. 模块详细设计

### 3.1 PiRpcBackend — 进程管理（替换 `PiPtyBackend`）

**文件：** `src/backend/rpc.rs`

> **不实现旧的 `AgentBackend` trait** — RPC 后端接口完全不同（request/response + event stream 而非 start/send_input/terminate）。App 状态机直接持有 `PiRpcBackend` 实例。旧的 `AgentBackend` trait 在 Step 3 随 `PiPtyBackend` 一并删除。

- 使用 `std::process::Command` spawn `pi --mode rpc`
- stdin: `Stdio::piped()` — 写入 JSONL 请求
- stdout: `Stdio::piped()` — 读取 JSONL 响应/事件
- stderr: `Stdio::piped()` — 仅日志输出，不参与协议
- 进程生命周期管理：start / stop / is_running

```rust
pub struct PiRpcBackend {
    command: String,          // "pi"
    cwd: PathBuf,             // 工作目录
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<BufWriter<ChildStdin>>>,
}

impl PiRpcBackend {
    pub fn new(command: &str, cwd: &Path) -> Self;

    /// 启动进程并返回 PiRpcClient
    /// 在 spawn_blocking 中启动 stdout 读取循环
    pub async fn start(&mut self, session_path: Option<&str>) -> Result<PiRpcClient>;

    /// 停止进程
    pub async fn stop(&mut self) -> Result<()>;

    /// 进程是否存活
    pub fn is_running(&self) -> bool;
}
```

### 3.2 PiRpcClient — JSONL 协议

**文件：** `src/backend/rpc_client.rs`

- 向 stdin 写入 JSON Lines
- 从 stdout 读取 JSON Lines
- 区分 `response`（同步回复）和 `event`（异步推送）
- 带超时的 pending 请求管理

**请求格式（stdin）：**
```json
{"id":"uuid","type":"prompt","message":"hello"}
{"id":"uuid","type":"get_state"}
{"id":"uuid","type":"bash","command":"ls -la"}
{"id":"uuid","type":"abort"}
```

**响应格式（stdout）：**
```json
{"id":"uuid","type":"response","command":"get_state","success":true,"data":{...}}
```

**事件格式（stdout）：**
```json
{"type":"agent_start"}
{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hello"}}
{"type":"tool_execution_start","toolName":"read","toolCallId":"...","args":{...}}
{"type":"agent_end","stopReason":"endTurn","messages":[...]}
```

核心类型：

```rust
pub struct PiRpcClient {
    // stdin writer — App 通过它发送 JSONL 请求
    writer: BufWriter<ChildStdin>,
    // App 从 event_rx 接收解析后的 PiEvent
    event_rx: mpsc::UnboundedReceiver<PiEvent>,
}

/// 读取循环内部状态（Arc<Mutex<>> 包裹，在 spawn_blocking 中共享）
pub(crate) struct RpcReadState {
    pub pending: HashMap<String, PendingRequest>,
    pub event_tx: mpsc::UnboundedSender<PiEvent>,
    pub protocol_err_tx: mpsc::UnboundedSender<String>,
}

impl PiRpcClient {
    /// 发送请求，等待响应
    pub async fn request(&mut self, cmd: Value, timeout: Duration) -> Result<RpcResponse>;

    /// 发送通知（不需要响应）
    pub async fn notify(&mut self, cmd: Value) -> Result<()>;

    /// 获取事件接收端引用
    pub fn event_rx(&mut self) -> &mut mpsc::UnboundedReceiver<PiEvent>;
}

pub(crate) struct PendingRequest {
    pub responder: oneshot::Sender<RpcResponse>,
    pub timer: tokio::time::Sleep,
}

pub struct RpcResponse {
    pub id: String,
    pub command: String,
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}
```

**通信过程详解：**

```
PiRpcClient 向 pi 发送请求：
  1. 生成 uuid
  2. 构造 {id, type, ...} 并通过 writer 写入 stdin（JSONL）
  3. 创建 oneshot::channel，将 id + responder 存入 RpcReadState.pending
  4. await oneshot::Receiver（带超时）

读取循环接收消息（spawn_blocking 中，Arc<Mutex<RpcReadState>>）：
  1. BufReader::read_line() 从 stdout 获取一行 JSON
  2. serde_json::from_str() 解析
  3. 如果有 id 且 match RpcReadState.pending
     → 通过 oneshot::Sender 发送响应，解除 request() 的 await
  4. 否则 → 通过 event_tx 发送 PiEvent
     → App 的 select! 收到 event_rx 消息

App 消费事件：
  loop {
      select! {
          event = client.event_rx().recv() => {
              let action = translate_pi_event(event);
              app.handle_action(action).await?;
          }
          _ = ticker.tick() => { render(); }
          _ = input_handler => { app.handle_action(UserSubmitInput).await?; }
          quit = check_quit_key() => { break; }
      }
  }
```

**支持的 RPC 命令：**

| 命令 | 用途 | 建议超时 |
|------|------|---------|
| `prompt` | 发送用户消息 | 600s (可配置) |
| `get_state` | 获取 session/模型状态 | 10s |
| `get_session_stats` | 获取 context/tokens/cost | 10s |
| `get_messages` | 加载历史消息 | 30s |
| `bash` | 执行 bash 命令 | 60s |
| `abort` | 停止当前响应 | 10s |
| `compact` | 触发上下文压缩 | 120s |
| `set_model` | 切换模型 | 60s |
| `get_available_models` | 获取可用模型列表 | 60s |
| `set_session_name` | 重命名会话 | 20s |
| `clone` | 克隆会话 | 120s |
| `fork` | 分叉会话 | 120s |
| `switch_session` | 切换会话文件 | 120s |

### 3.3 PiEventProcessor — 事件翻译

**文件：** `src/backend/event.rs`

将 pi 原始 JSON 事件转换为类型化的 Action，供 App 状态机消费。

```rust
/// pi 消息中的 content 块（对应 assistant 消息的复杂内容结构）
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String },
    ToolCall { id: String, name: String, arguments: Value },
    Image { data: String, mime_type: String },
}

/// pi 消息原始数据摘要（从 JSON 事件中提取的公共字段）
#[derive(Debug, Clone)]
pub struct MessageData {
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub enum PiEvent {
    AgentStart,

    AgentEnd {
        stop_reason: String,
        error: Option<String>,
        will_retry: Option<bool>,
    },

    MessageStart {
        role: String,
    },

    MessageUpdate {
        event_type: AssistantEventType,
        delta: Option<String>,
        message: Option<MessageData>,
    },

    MessageEnd {
        message: Option<MessageData>,
    },

    ToolExecutionStart {
        tool_name: String,
        tool_call_id: String,
        args: Value,
    },

    ToolExecutionUpdate {
        tool_name: String,
        tool_call_id: String,
        partial_result: Option<Value>,
    },

    ToolExecutionEnd {
        tool_name: String,
        tool_call_id: String,
        result: Value,
        is_error: bool,
    },

    ExtensionError {
        error: String,
    },

    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
    },

    AutoRetryEnd {
        success: bool,
        final_error: Option<String>,
    },
}

#[derive(Debug)]
pub enum AssistantEventType {
    TextDelta,
    TextStart,
    TextEnd,
    ThinkingDelta,
    ThinkingEnd,
    MessageStart,
    MessageEnd,
    Done,
    Error,
}

/// 将原始 JSON Value 解析为 PiEvent
pub fn parse_pi_event(raw: &Value) -> Result<Option<PiEvent>>;
```

事件→Action 映射表：

| pi 事件 | → Action | 效果 |
|---------|---------|------|
| agent_start | AgentStatusChange(running) | TopBar 状态变绿 |
| message_update(text_delta) | MessageAppend { agent_id, delta } | MainView 打字机效果 |
| message_update(thinking_delta) | ThinkingAppend { agent_id, delta } | 思考过程展示 |
| message_update(thinking_end) | ThinkingFinalize { agent_id, text } | 思考完成 |
| message_end | MessageFinalize { agent_id } | 消息定型 |
| tool_execution_start | ToolEvent { status: Running, ... } | 工具调用指示器 |
| tool_execution_update | ToolEvent { status: Update, ... } | 进度更新 |
| tool_execution_end | ToolEvent { status: Done/Error, ... } | 工具完成 |
| agent_end | AgentStatusChange(idle) + 加载 runtime state | 恢复空闲状态 |
| agent_end(will_retry) | AutoRetryStatus { ... } | 显示自动重试进度 |

### 3.4 ChatMessage 模型

**文件：** `src/message.rs`

```rust
#[derive(Debug, Clone)]
pub enum ChatRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

#[derive(Debug, Clone)]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub tool_name: String,
    pub tool_call_id: String,
    pub status: ToolStatus,
    pub args: Value,
    pub result: Option<Value>,
    pub detail_text: String,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub agent_id: String,
    pub role: ChatRole,
    pub text: String,
    pub thinking: Option<String>,
    pub tool_call: Option<ToolCallInfo>,
    pub timestamp: u64,
    pub meta: Option<HashMap<String, Value>>,
}

impl ChatMessage {
    pub fn user(agent_id: &str, text: &str) -> Self;
    pub fn assistant(agent_id: &str, text: &str) -> Self;
    pub fn tool(agent_id: &str, tool_call: ToolCallInfo) -> Self;
    pub fn system(agent_id: &str, text: &str) -> Self;
    pub fn error(agent_id: &str, text: &str) -> Self;
}
```

### 3.5 Action 枚举重构

**文件：** `src/action.rs`

```rust
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    // --- PTY 遗留（Step 3 删除） ---
    #[deprecated(note = "将迁移到 MessageAppend")]
    PtyStdout(String),
    #[deprecated(note = "将迁移到 AgentStatusChange")]
    PtyExit,

    // --- 用户交互 ---
    UserSubmitInput(String),

    // --- RPC 事件 (新增) ---
    MessageAppend { agent_id: String, text: String },
    MessageFinalize { agent_id: String },
    ThinkingAppend { agent_id: String, text: String },
    ThinkingFinalize { agent_id: String, text: String },
    ToolEvent {
        agent_id: String,
        tool_name: String,
        tool_call_id: String,
        status: ToolStatus,
        args: Option<Value>,
        result: Option<Value>,
        is_error: bool,
    },
    AgentStatusChange { agent_id: String, status: AgentStatus },
    RuntimeStateUpdate(AgentRuntimeState),
    AgentMessagesLoaded { agent_id: String, messages: Vec<ChatMessage> },
    AutoRetryStatus { agent_id: String, text: String },

    // --- UI 控制 ---
    TogglePopup,
    SwitchSession(String),
}
```

### 3.6 MainView — 消息渲染

**文件：** `src/components/main_view.rs`

从当前「buffer + ANSI 解析」改为「消息列表 + 结构化渲染」。

```rust
pub struct MainView {
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: usize,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub show_thinking: bool,
}
```

**渲染策略：**

| 消息类型 | 渲染方式 |
|---------|---------|
| user | `你 [时间]` 标签 + 文本（可 Markdown 高亮） |
| assistant | `pi [时间]` 标签 + 文本；有 thinking 时先展开/折叠思考块 |
| tool | 工具名 + 状态图标(▶/✓/✗) + 结果（可折叠） |
| system | 灰色标签 + 信息文本 |
| error | 红色标签 + 错误文本 |

**输入框：** 嵌入 MainView 底部区域，使用 Ratatui 原生输入处理。
- Enter: 发送（或 Ctrl+Enter，可配置）
- 发送后清空输入框，添加用户消息到 messages
- 通过 RPC prompt 命令发送给 pi

### 3.7 App 状态机

**文件：** `src/app.rs`

```rust
pub struct App {
    // Agent 管理
    agents: HashMap<String, AgentInfo>,
    messages: HashMap<String, Vec<ChatMessage>>,
    active_agent: Option<String>,

    // 运行时状态
    runtime: AgentRuntimeState,

    // RPC 后端
    backend: PiRpcBackend,
    event_rx: mpsc::UnboundedReceiver<PiEvent>,

    // 通信
    action_tx: mpsc::Sender<Action>,

    // TUI 状态
    tui: TuiState,
}

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
```

**核心状态循环：**

```rust
loop {
    tokio::select! {
        // 定时刷新
        _ = ticker.tick() => { render(); }

        // pi 事件
        Some(event) = event_rx.recv() => {
            let action = translate_event_to_action(event);
            app.handle_action(action).await?;
        }

        // 用户输入
        input = input_handler.next() => {
            app.handle_action(Action::UserSubmitInput(input)).await?;
        }

        // q 键退出
        quit = check_quit_key() => { break; }

        else => break,
    }
}
```

---

## 4. 迁移策略

### Step 1: 核心通信层（纯新增，不删旧代码）

**文件变化：**
- `src/backend/rpc_client.rs` — 新建
- `src/backend/rpc.rs` — 新建（PiRpcBackend）
- `src/backend/event.rs` — 新建（PiEvent + parse_pi_event）
- `src/message.rs` — 新建（ChatMessage 模型）
- `src/backend/mod.rs` — `pub mod rpc; pub mod rpc_client; pub mod event;`
- `src/lib.rs` — `pub mod message;`
- `Cargo.toml` — 添加 `serde` (features = ["derive"]), `serde_json`, `uuid` (features = ["v4"])

**产出：** JSONL RPC 可通信，事件可解析。
**验证：** `cargo build && cargo test` 保持绿色。
**新旧共存：** ✅ 是，PiPtyBackend 继续可用。

### Step 2: 渲染 + 状态机（修改现有文件）

**文件变化：**
- `src/action.rs` — 新增 RPC Action，标注旧 Action 为 `#[deprecated]`
- `src/app.rs` — 增加 RPC 事件处理分支
- `src/components/main_view.rs` — 重构为消息列表渲染
- `src/main.rs` — TUI 模式启动 PiRpcBackend

**产出：** 完整 TUI 可以与 pi 对话（使用 `pi --mode rpc`）。
**验证：** `cargo run -- --tui` — `--target` 参数在 RPC 模式下不再生效。TUI 模式固定使用 `pi --mode rpc`（忽略 `--target` 值）。Headless 模式仍可使用旧 `--target`（走 PTY）以保持向后兼容。

> **注意：** `--target` 在 Phase 4 中的角色变更：
> - TUI 模式：忽略 `--target`，固定 `pi --mode rpc`
> - Headless 模式：保持旧行为（--target /bin/sh 仍走 PTY）
> - 这也意味着 `PiPtyBackend` 在 Step 1-2 期间仍然需要存在（为 headless 模式服务），Step 3 再一并清理
**新旧共存：** ✅ 是，旧代码暂不删除。

### Step 3: 侧面板 + 清理

**文件变化：**
- `src/components/agent_panel.rs` — 接入真实 runtime state
- `src/components/sidebar.rs` — 接入 token 统计
- `src/components/top_bar.rs` — 显示模型/状态
- 删除 `src/backend/pi.rs`（PiPtyBackend）
- 删除 `src/ansi.rs`（ANSI 解析器）
- 删除 `src/interceptor/mod.rs`（OutputInterceptor + NoopInterceptor）
- 更新集成测试

**产出：** 全功能就绪，PTY/ANSI 代码清理干净。
**验证：** `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check`。
**新旧共存：** ❌ Step 3 完成后，旧代码不再留存。

---

## 5. 边界情况处理

| 场景 | 处理方式 |
|------|---------|
| pi 未安装 | `PiRpcBackend::start()` 返回错误，TUI 显示 "未检测到 pi CLI" |
| RPC 超时 | `request()` 返回 `Timeout` 错误，App 显示 "响应超时" |
| pi 进程崩溃 | stdout 关闭 → 读取循环退出 → `AgentExited` Action |
| 非 JSON 输出 | `PiRpcClient` 通过 `protocol-error` 事件暴露原始文本 |
| 自动重试 | `auto_retry_start/end` 事件 → 系统消息显示重试进度 |
| 多 Agent | Phase 5 范围，当前仅单 Agent；数据结构预留 `agent_id: String` |

---

## 6. 测试计划

| 测试类型 | 测试内容 | 阶段 |
|---------|---------|------|
| 单元测试 | `PiRpcClient::request()` 请求/响应匹配 | Step 1 |
| 单元测试 | `parse_pi_event()` 各种事件类型解析 | Step 1 |
| 单元测试 | `ChatMessage` 构建和序列化 | Step 1 |
| 集成测试 | spawn pi + 发送 prompt + 接收响应 | Step 2 |
| 集成测试 | 进程崩溃后 App 状态转换 | Step 2 |
| 集成测试 | 消息列表渲染（mock 数据） | Step 2 |
| E2E 测试 | `cargo run -- --tui` 完整流程 | Step 3 |

---

## 7. 不涉及的范围 (Out of Scope)

- **多 Agent 编排**：Phase 5 范围，当前只管理单 Agent
- **会话持久化**：由 pi 的 `--session` 机制负责，TUI 暂不接管
- **Markdown/代码高亮渲染**：后续增量优化，Step 2 先展示原始文本
- **编辑器集成**（Monaco 等）：远期的阶段
- **飞书/外部集成**：PiDeck 的 FeishuBridge 不在本设计范围内
