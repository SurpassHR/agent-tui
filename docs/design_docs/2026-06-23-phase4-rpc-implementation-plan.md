# Phase 4: RPC 架构改造 — 实施计划

> 基于设计稿：`docs/superpowers/specs/2026-06-23-phase4-rpc-architecture-design.md`
> 日期：2026-06-23

---

## 总览

三步迁移，每步完成后可编译、可测试、可运行。

```
Step 1: 核心通信层  (纯新增，不删旧代码)   →   JSONL RPC 可通信
Step 2: 渲染+状态机  (修改现有文件)       →   完整 TUI 可对话
Step 3: 侧面板+清理   (删除旧代码)        →   全功能就绪
```

---

## Step 1: 核心通信层

### 目标

新增 RPC 通信基础设施，**不修改任何现有代码**。

### 文件清单

| 文件 | 操作 | 内容 |
|------|------|------|
| `Cargo.toml` | 修改 | 添加依赖 |
| `src/message.rs` | 新建 | ChatMessage 模型 |
| `src/backend/event.rs` | 新建 | PiEvent + parse_pi_event |
| `src/backend/rpc_client.rs` | 新建 | PiRpcClient + RpcReadState |
| `src/backend/rpc.rs` | 新建 | PiRpcBackend |
| `src/backend/mod.rs` | 修改 | 导出新模块 |
| `src/lib.rs` | 修改 | 导出 `pub mod message` |
| `tests/rpc_test.rs` | 新建 | RPC 单元测试 |

### Task 1.1: Cargo.toml 添加依赖

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
tokio-util = "0.7"  # tokio::io::BufReader/BufWriter
```

### Task 1.2: src/message.rs

ChatMessage 模型（纯数据，无逻辑）：

- `ChatRole` enum: User, Assistant, Tool, System, Error
- `ToolStatus` enum: Running, Done, Error
- `ToolCallInfo` struct: tool_name, tool_call_id, status, args, result, detail_text
- `ChatMessage` struct: id, agent_id, role, text, thinking, tool_call, timestamp, meta
- `ContentBlock` enum: Text, Thinking, ToolCall, Image (Image 标注 `#[allow(unused)]`)
- `MessageData` struct: role, content(Vec\<ContentBlock\>), stop_reason, error_message
- 构造函数: `ChatMessage::user()`, `::assistant()`, `::tool()`, `::system()`, `::error()`
- `#[derive(Debug, Clone, Serialize, Deserialize)]`

验证：`cargo build` 通过，`cargo test` 新增的消息序列化测试通过。

### Task 1.3: src/backend/event.rs

pi 事件类型和解析逻辑：

- `AssistantEventType` enum: TextDelta, TextStart, TextEnd, ThinkingDelta, ThinkingEnd, MessageStart, MessageEnd, Done, Error
- `PiEvent` enum (见设计稿 3.3 节)
- `pub fn parse_pi_event(raw: &Value) -> Result<Option<PiEvent>>` — 将原始 JSON 解析为 PiEvent
- `#[derive(Debug, Clone, Serialize, Deserialize)]` on all types

验证：单元测试覆盖所有 PiEvent 变体的 JSON 解析。

### Task 1.4: src/backend/rpc_client.rs

JSONL 协议客户端：

- `PendingRequest` struct: responder(oneshot::Sender), timer(Delay)
- `RpcReadState` struct: pending(HashMap), event_tx(UnboundedSender), protocol_err_tx(UnboundedSender) — 所有字段用 Arc\<Mutex\> 包裹
- `PiRpcClient` struct: writer(BufWriter), event_rx(UnboundedReceiver)
  - `fn new(stdin: ChildStdin, stdout: ChildStdout) -> (Self, Arc<Mutex<RpcReadState>>, JoinHandle<()>)` — 创建 client、启动读取循环
  - `async fn request(&mut self, cmd: Value, timeout: Duration, state: &Arc<Mutex<RpcReadState>>) -> Result<RpcResponse>` — 写入 stdin，await oneshot
  - `async fn notify(&mut self, cmd: Value) -> Result<()>` — 只写不等待
  - `fn event_rx(&mut self) -> &mut UnboundedReceiver<PiEvent>` — 获取事件接收端
- 读取循环函数 `fn read_loop(reader: ChildStdout, state: Arc<Mutex<RpcReadState>>)`
  - BufReader::read_line 循环
  - serde_json 解析
  - response → resolve pending
  - event → event_tx.send(PiEvent)
  - 解析失败 → protocol_err_tx.send(raw_line)

验证：
- `test_rpc_request_response` — 用管道模拟 pi 进程，验证请求/响应往返
- `test_rpc_event_stream` — 验证事件被正确发送到 event_rx
- `test_rpc_timeout` — 验证超时机制

### Task 1.5: src/backend/rpc.rs

进程管理器：

- `PiRpcBackend` struct: command(String), cwd(PathBuf), child(Mutex\<Option\<Child\>\>)
  - `fn new(command: &str, cwd: &Path) -> Self`
  - `async fn start(&mut self, session_path: Option<&str>) -> Result<PiRpcClient>` — spawn、移交 stdin/stdout、启动读取循环、返回 client
  - `async fn stop(&mut self) -> Result<()>` — kill + wait
  - `fn is_running(&self) -> bool`
- 启动 pi 的 args: `["--mode", "rpc", "--no-session"]` 或 `["--mode", "rpc", "--session", path]`

### Task 1.6: 模块注册

- `src/backend/mod.rs` — 添加 `pub mod rpc; pub mod rpc_client; pub mod event;`
- `src/lib.rs` — 添加 `pub mod message;`

### Task 1.7: 验证

```bash
cargo build                     # 编译通过
cargo test                      # 所有现有测试 + 新增 RPC 测试通过
cargo clippy -- -D warnings     # 零警告
cargo fmt --check               # 格式合规
```

---

## Step 2: 渲染 + 状态机

### 目标

修改现有文件，使 TUI 模式使用 PiRpcBackend 运行。此时 TUI 可以：
1. 用户输入文字 → 发送 prompt RPC → pi 返回事件流 → TUI 渲染消息列表

### 文件清单

| 文件 | 操作 | 内容 |
|------|------|------|
| `src/action.rs` | 修改 | 新增 RPC Action，旧 Action 标 `#[deprecated]` |
| `src/app.rs` | 重写 | RPC 事件处理 + 消息管理 |
| `src/components/main_view.rs` | 重写 | 消息列表渲染 + 输入框 |
| `src/tui.rs` | 修改 | 事件循环适配 PiRpcClient |
| `src/main.rs` | 修改 | TUI 模式走 PiRpcBackend |
| `src/config.rs` | 修改 | 新增 `--pi-path` CLI 参数 |
| `src/components/mod.rs` | 修改 | 可能需要的 trait 调整 |
| `tests/rpc_integration_test.rs` | 新建 | 集成测试 |

### Task 2.1: config.rs — 新增 CLI 参数

```rust
/// pi CLI 命令路径（默认 "pi"）
#[arg(long = "pi-path", default_value = "pi")]
pub pi_command: String,
```

### Task 2.2: action.rs — 新增 RPC Action

保留旧 PTY Action（标注 `#[deprecated]`），新增：

```rust
MessageAppend { agent_id: String, text: String },
MessageFinalize { agent_id: String },
ThinkingAppend { agent_id: String, text: String },
ThinkingFinalize { agent_id: String, text: String },
ToolEvent { agent_id, tool_name, tool_call_id, status: ToolStatus, args, result, is_error },
AgentStatusChange { agent_id: String, status: AgentStatus },
RuntimeStateUpdate(AgentRuntimeState),
AgentMessagesLoaded { agent_id: String, messages: Vec<ChatMessage> },
AutoRetryStatus { agent_id: String, text: String },
```

### Task 2.3: app.rs — 状态机重写

关键变化：
- 移除 `backend: Option<Box<dyn AgentBackend>>`
- 新增 `backend: PiRpcBackend`, `client: PiRpcClient`, `event_rx`
- 新增 `messages: HashMap<String, Vec<ChatMessage>>`
- 新增 `active_agent: Option<String>`, `runtime: AgentRuntimeState`
- `handle_action()` 新增 RPC 事件分支

**Agent 状态定义：**
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Starting,
    Idle,
    Running,
    Error,
    Closed,
}
```

**handle_action 核心分支：**

```rust
// 用户输入
Action::UserSubmitInput(msg) => {
    let user_msg = ChatMessage::user(&agent_id, &msg);
    self.push_message(&agent_id, user_msg);
    // 通过 RPC 发送
    let cmd = serde_json::json!({"type": "prompt", "message": msg});
    self.client.request(cmd, timeout, &self.read_state).await?;
}

// pi 事件翻译后的 Action
Action::MessageAppend { agent_id, text } => {
    // 追加到当前 assistant 消息（打字机效果）
    self.append_to_last_assistant(&agent_id, &text);
}
Action::AgentStatusChange { agent_id, status } => {
    self.set_agent_status(&agent_id, status);
}
// ... 其他事件
```

### Task 2.4: main_view.rs — 消息渲染

重构 MainView：

```rust
pub struct MainView {
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: usize,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub show_thinking: bool,
}

pub enum InputMode {
    Normal,
    Editing,
}
```

**渲染策略：**

- 消息区域：从 `messages` 列表遍历，每条消息按其 `role` 选择渲染方式
- 输入框：嵌入底部，使用 Ratatui `TextInput` 或手动处理光标

```
┌─────────────────────────────────────────┐
│ 你 (10:00)                               │
│ 写一个斐波那契函数                        │
│                                          │
│ pi (10:03)                               │
│ ┌─ 思考过程 ──┐  ← 可折叠               │
│ │ 用户需要...  │                          │
│ └─────────────┘                          │
│ 以下是 Rust 实现：                        │
│ fn fib(n: u64) -> u64 {                  │
│     match n { 0 => 0, 1 => 1, ... }      │
│ }                                        │
│                                          │
│ ✓ bash "cargo test"  ← 可展开           │
│                                          │
│ ┌──────────────────────────────────┐     │
│ │ _ 输入你的问题...                │     │
│ │ [Ctrl+Enter 发送]                │     │
│ └──────────────────────────────────┘     │
└─────────────────────────────────────────┘
```

- **输入按键处理**：`Enter` / `Ctrl+Enter` 发送（取决 InputMode），发送后清空输入
- **滚动**：`scroll_offset` + 翻页键（`PgUp`/`PgDn`）
- **思考折叠**：`show_thinking` toggle

### Task 2.5: tui.rs — 事件循环适配

`run_tui()` 从「PTY action_rx + ticker」改为「PiRpcClient event_rx + 用户输入 + ticker」：

```rust
loop {
    tokio::select! {
        // 定时刷新
        _ = ticker.tick() => {
            terminal.try_draw(|f| app.render_tui(f))?;
        }

        // pi 事件
        Some(event) = app.client.event_rx().recv() => {
            let action = translate_pi_event(event);
            app.handle_action(action).await?;
        }

        // 用户键盘输入（包括输入框内容、q 退出等）
        key_event = input_handler.next() => {
            app.handle_key_event(key_event).await?;
        }

        else => break,
    }
}
```

- 需要新增 `input_handler` 检测键盘事件（替代 `check_quit_key`）
- crossterm `event::poll` + `event::read` 处理输入

### Task 2.6: main.rs — 启动逻辑

```rust
if config.tui_mode {
    let backend = PiRpcBackend::new(&config.pi_command, &cwd);
    let mut client = backend.start(None).await?;
    let event_rx = client.event_rx(/* ... */);
    let app = App::new(backend, client, event_rx, action_tx);
    app.start().await?;
    run_tui(app, action_rx).await?;
}
```

**双模式共存：**
- TUI 模式 → 使用 PiRpcBackend（固定 `pi --mode rpc`）
- Headless 模式 → 保持 PiPtyBackend（`--target /bin/sh` 不变）
- `--target` 在 TUI 模式下不生效

### Task 2.7: 验证

```bash
cargo build
cargo test

# TUI 模式测试
cargo run -- --tui
# 输入文字，观察 pi 回复是否流式渲染到 MainView

cargo clippy -- -D warnings
cargo fmt --check
```

---

## Step 3: 侧面板 + 清理

### 目标

- 侧面板数据从 RPC 获取真实数据
- 删除所有 PTY/ANSI 旧代码
- 更新测试

### 文件清单

| 文件 | 操作 | 内容 |
|------|------|------|
| `src/components/top_bar.rs` | 修改 | 显示模型名、agent 状态 |
| `src/components/agent_panel.rs` | 修改 | 显示 runtime state |
| `src/components/sidebar.rs` | 修改 | 显示 token 统计、session 信息 |
| `src/backend/pi.rs` | **删除** | PiPtyBackend |
| `src/backend/mod.rs` | 修改 | 移除 pi 模块 |
| `src/ansi.rs` | **删除** | ANSI 解析器 |
| `src/interceptor/mod.rs` | **删除** | OutputInterceptor + NoopInterceptor |
| `src/lib.rs` | 修改 | 移除 ansi、interceptor 模块导出 |
| `tests/integration_test.rs` | 修改 | 移除 PTY 测试，保留 headless 测试 |
| `tests/rpc_integration_test.rs` | 修改 | 补充更多的 RPC 集成测试 |

### Task 3.1: AgentPanel 接入 RuntimeState

定期（每 30s 或 agent_end 时）调用 `get_session_stats`，更新：
- 当前模型名
- 上下文使用量 / 窗口大小 / 百分比
- Cache read/write
- Cost 累计

数据来源：`Action::RuntimeStateUpdate(AgentRuntimeState)`。

### Task 3.2: TopBar 显示状态

- 当前 agent 名称
- 模型名（`model_name@provider`）
- 状态指示器（`●` idle 绿 / running 黄 / error 红）
- 思考等级指示

### Task 3.3: Sidebar 更新

- Session 信息（从 get_state 获取 session_id）
- Token 用量
- 消息计数

### Task 3.4: 清理 PTY/ANSI 代码

删除文件：
- `src/backend/pi.rs`
- `src/ansi.rs`
- `src/interceptor/mod.rs`

从 `src/lib.rs` 移除：`pub mod ansi;` `pub mod interceptor;`

从 `src/backend/mod.rs` 移除：`pub mod pi;`

### Task 3.5: 更新测试

- 移除 `tests/integration_test.rs` 中依赖 `PiPtyBackend` 的用例
  - `test_pty_spawn_shell_output`
  - `test_pty_input_output_roundtrip`
  - `test_pty_exit_signal`
  - `test_terminate_cleanup`
  - `test_app_lifecycle`（保留，但改为使用 PiRpcBackend mock）
- 新增 RPC 集成测试：
  - `test_rpc_spawn_and_prompt` — spawn pi --mode rpc，发送 prompt，接收消息
  - `test_rpc_backend_lifecycle` — start → stop → 进程回收
  - `test_rpc_runtime_state` — get_state + get_session_stats

### Task 3.6: 最终验证

```bash
cargo build
cargo test                          # 所有测试通过
cargo clippy -- -D warnings         # 零警告
cargo fmt --check                   # 格式合规

# TUI 模式
cargo run -- --tui
# → 输入文字，pi 回复显示在 MainView
# → 侧面板显示模型/token/cost
# → q 退出

# Headless 模式（仍可用）
cargo run -- --target /bin/sh
# → 保持旧行为

# 验证无 PTY/ANSI 代码残留
grep -r "PiPtyBackend\|parse_to_lines\|OutputInterceptor\|AnsiStyleState" src/ || echo "cleaned"
```

---

## 实施顺序

```
Step 1.1 ─→ Step 1.2 ─→ Step 1.3 ─→ Step 1.4 ─→ Step 1.5 ─→ Step 1.6 ─→ Step 1.7 (验证)
                                                                                │
Step 2.1 ─→ Step 2.2 ─→ Step 2.3 ─→ Step 2.4 ─→ Step 2.5 ─→ Step 2.6 ─→ Step 2.7 (验证)
                                                                                │
Step 3.1 ─→ Step 3.2 ─→ Step 3.3 ─→ Step 3.4 ─→ Step 3.5 ─→ Step 3.6 (最终验证)
```

每步之间有依赖关系，必须按序执行。Step 1 所有 Task 可在实施计划批准后立即开始。
