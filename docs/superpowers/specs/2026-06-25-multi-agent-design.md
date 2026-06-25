# 多 Agent 进程并行支持 — 设计文档

> 状态：草稿 | 日期：2026-06-25 | 作者：agent-tui

## 1. 概述

### 1.1 目标

使 agent-tui 支持同时运行多个 pi agent 进程，每个进程对应一个独立的会话。用户在 WORKSPACE 树中双击 Enter 即可为任意会话启动 pi agent，单 agent 的 ● 绿点变为多 agent 的 ● 绿点集合。退出 TUI 时自动杀掉所有子进程，重启后从 state.json 恢复活跃会话列表并重新 spawn agent。

### 1.2 非目标

- 退出后保持 agent 进程存活（后续阶段）
- 进程扫描/PID 文件发现（后续阶段）
- Agent 进程间通信

## 2. 数据模型

### 2.1 App 结构体变更

```rust
pub struct App {
    // ── 移除 ──
    // pub agent_session_id: Option<String>,
    // pub agent_session_file: Option<String>,

    // ── 新增 ──
    /// 多 agent 进程管理器
    pub agent_manager: AgentManager,
    /// 活跃会话 ID 集合（is_online 绿点的依据）
    pub active_sessions: HashSet<String>,

    // ── 保留（语义不变） ──
    pub active_agent: Option<String>,       // 当前浏览的会话 ID
    pub session: SessionInfo,               // 当前聚焦会话的元信息
    // ... 其余字段不变
}
```

### 2.2 action.rs 变更

```rust
// 移除
// ConnectSession(String),

// 新增
/// 为指定会话启动 pi agent 进程
ConnectSession {
    session_id: String,
    file_path: String,
},
/// 停止指定会话的 pi agent 进程
DisconnectSession(String),
```

### 2.3 TuiState 不变

`is_online` 由 `sync_components` 根据 `App.active_sessions` 动态计算，无需 TuiState 字段变更。

## 3. AgentManager

### 3.1 结构

新文件：`src/backend/agent_manager.rs`

```rust
pub struct AgentProcess {
    pub session_id: String,
    pub session_path: PathBuf,
    pub backend: PiRpcBackend,
    pub client: PiRpcClient,
}

pub struct AgentManager {
    agents: HashMap<String, AgentProcess>,
}
```

### 3.2 接口

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `fn new() -> Self` | 空管理器 |
| `spawn` | `async fn spawn(&mut self, session_id: String, session_path: PathBuf, cwd: PathBuf, event_tx: Sender) -> Result<()>` | 启动 pi 子进程，连接指定 session；将 event_rx 转发到 event_tx |
| `kill` | `async fn kill(&mut self, session_id: &str)` | 杀掉指定 agent 进程并移除 |
| `kill_all` | `async fn kill_all(&mut self)` | 杀掉所有 agent 进程 |
| `is_active` | `fn is_active(&self, session_id: &str) -> bool` | 是否存在活跃 agent |
| `client_mut` | `fn client_mut(&mut self, id: &str) -> Option<&mut PiRpcClient>` | 获取指定 agent 的 RPC client |

### 3.3 spawn 流程

1. `let mut backend = PiRpcBackend::new("pi", &cwd)`
2. `let client = backend.start(Some(&session_path.to_string_lossy())).await?`
3. 创建 event 转发 task：`tokio::spawn` 死循环从 `client.event_rx` 读取，包装为 `(session_id, event)` 发送到 `event_tx`
4. 包装为 `AgentProcess` 插入 `agents` HashMap

### 3.4 kill 流程

1. `backend.stop().await` — kill + wait 子进程
2. `agents.remove(session_id)` — 移除记录
3. event 转发 task 在 event_rx close 后自动退出

## 4. 事件流合并

### 4.1 架构

当前主循环直接 poll `client.event_rx`。多 agent 后改为：

```
┌─ Agent A event_rx ──┐
├─ Agent B event_rx ──┤  →  event_tx  →  agent_event_rx  →  主循环 select!
├─ Agent C event_rx ──┘
└─ crossterm EventStream ─────────────────────────────────→  主循环 select!
```

### 4.2 主循环 select! 结构

```rust
let (agent_event_tx, mut agent_event_rx) = mpsc::unbounded_channel();

loop {
    tokio::select! {
        _ = ticker.tick() => { /* 50ms 渲染 */ }
        _ = state_poll_ticker.tick() => { /* 5s 状态轮询 */ }
        Some((agent_id, event)) = agent_event_rx.recv() => {
            translate_pi_events(event, &agent_id)
                .into_iter()
                .for_each(|action| app.handle_action(action));
        }
        Some(Ok(event)) = crossterm_event_stream.next() => { /* 键盘/鼠标 */ }
        Some(action) = action_rx.recv() => { /* channel action */ }
    }
}
```

### 4.3 事件转发 task

定义在 `AgentManager` 内（`agent_manager.rs`）：

```rust
impl AgentManager {
    fn spawn_event_forwarder(
        mut rx: UnboundedReceiver<PiEvent>,
        session_id: String,
        tx: UnboundedSender<(String, PiEvent)>,
    ) {
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if tx.send((session_id.clone(), event)).is_err() {
                    break;
                }
            }
        });
    }
}
```

## 5. Action 处理

### 5.1 ConnectSession

```rust
Action::ConnectSession { session_id, file_path } => {
    let cwd = std::env::current_dir().unwrap_or_default();
    match self.agent_manager.spawn(session_id.clone(), file_path.into(), cwd, agent_event_tx.clone()).await {
        Ok(()) => {
            self.active_sessions.insert(session_id.clone());
            // 同时切换到浏览该会话
            self.active_agent = Some(session_id.clone());
            // 加载消息
            if let Ok(msgs) = load_session_messages(&file_path) {
                self.messages.insert(session_id.clone(), msgs);
                self.sync_messages_to_main_view(&session_id);
            }
            self.sync_components();
            self.tui.bottom_bar.status = format!("已连接: {}", session_id);
        }
        Err(e) => {
            self.tui.bottom_bar.status = format!("连接失败: {}", e);
        }
    }
}
```

### 5.2 DisconnectSession

```rust
Action::DisconnectSession(session_id) => {
    self.agent_manager.kill(&session_id).await;
    self.active_sessions.remove(&session_id);
    self.sync_components();
    self.tui.bottom_bar.status = format!("已断开: {}", session_id);
}
```

### 5.3 TUI 退出

```rust
// run_tui() 退出前
app.agent_manager.kill_all().await;
// 保存持久化状态
crate::persistence::save(&app.build_persist_state());
let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
let _ = ratatui::try_restore();
return Ok(());
```

### 5.4 sync_components 中 is_online 更新

```rust
for ws in &mut self.tui.workspaces {
    for s in &mut ws.sessions {
        s.is_online = self.active_sessions.contains(&s.id);
    }
}
```

### 5.5 ACTIVE SESSION 区显示

显示所有活跃会话，而非仅一个。从 `active_sessions` 集合获取会话 ID，从工作区树查找名称。

```rust
let active_names: Vec<String> = self.active_sessions.iter()
    .filter_map(|sid| {
        self.tui.workspaces.iter()
            .find_map(|ws| ws.sessions.iter().find(|s| s.id == *sid))
            .map(|s| s.name.clone())
    })
    .collect();
```

## 6. 持久化

### 6.1 UiPersistState 扩展

```rust
pub struct UiPersistState {
    pub active_session_id: Option<String>,       // 浏览会话（不变）
    pub active_agent_sessions: Vec<String>,      // 新增：活跃 agent 会话 ID 列表
    pub expanded_workspaces: Vec<String>,
}
```

### 6.2 保存时机

- `ConnectSession` / `DisconnectSession` 处理后
- 对应 `handle_action` 分支末尾调用 `crate::persistence::save()`

### 6.3 恢复流程

启动时 `run_tui()` 中：
1. 从 state.json 读取 `active_agent_sessions`
2. 对每个 session ID，从工作区树查找 `file_path`
3. 调用 `agent_manager.spawn(session_id, file_path, cwd, event_tx)`
4. 将 session ID 加入 `active_sessions`

## 7. 双击 Enter 触发 ConnectSession

### 7.1 tui.rs Workspace Enter 处理（已实现）

双击检测逻辑不变：同会话 + 500ms 内连续 Enter → `ConnectSession { session_id, file_path }`。

## 8. 测试策略

## 9. ACTIVE SESSION 区渲染变更

当前 `render_active_session` 只显示单行（会话名 + UUID）。多 agent 后改为列表形式：每个活跃会话一行，带 ● 前缀和 agent 运行状态。

```rust
fn render_active_sessions(&self, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // 标题行
    lines.push(Line::from("ACTIVE SESSION".fg(theme.heading).bold()));
    // 每个活跃会话一行
    for sid in &self.active_sessions {
        if let Some(sess) = find_session_in_workspaces(sid) {
            lines.push(Line::from(vec![
                "● ".fg(theme.success),
                sess.name.fg(theme.text),
                format!("  ({})", &sess.id[..sess.id.len().min(12)]).fg(theme.border_dim),
            ]));
        }
    }
    if self.active_sessions.is_empty() {
        lines.push(Line::from("○ No active agents".fg(theme.text_dim)));
    }
    lines
}
```

## 10. 状态轮询变更

当前 `get_state` 和 `get_session_stats` 使用 `client.request()`。多 agent 后改为：

- **`get_state`**：对每个活跃 agent 分别调用，收集各自的 model/token 信息
- **`get_session_stats`**：对当前浏览会话的 agent 调用（如存在）
- 移除对 `app.session.id/name/file_path` 的状态轮询覆盖——agent 会话信息全部由 `ConnectSession` 时从工作区树获取

```rust
// state_poll_ticker 内
for (sid, proc) in app.agent_manager.agents.iter_mut() {
    if let Ok(resp) = proc.client.request(
        serde_json::json!({"type": "get_state"}), Duration::from_secs(5)
    ).await {
        if let Some(data) = &resp.data {
            // 更新该 agent 的 model 信息
            // ...
        }
    }
}
```

## 11. 测试策略

| 测试层级 | 测试内容 |
|---------|---------|
| 单元测试 | `AgentManager::new/spawn/kill/kill_all` — mock PiRpcBackend |
| 单元测试 | `ConnectSession` / `DisconnectSession` action 处理 — 验证 active_sessions 变更 |
| 集成测试 | 多 agent 并发 spawn/kill — 验证子进程正确创建和清理 |
| 手动测试 | TUI 中双击 Enter 连接/断开 → 验证 ● 绿点切换、ACTIVE SESSION 更新 |

## 9. 文件变更清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src/backend/agent_manager.rs` | 新增 | AgentManager + AgentProcess |
| `src/backend/mod.rs` | 修改 | 导出 agent_manager 模块 |
| `src/app.rs` | 修改 | App 字段变更，handle_action 新增，sync_components 更新 |
| `src/action.rs` | 修改 | ConnectSession/DisconnectSession，删除旧 ConnectSession |
| `src/tui.rs` | 修改 | 事件流合并、退出 kill_all、恢复逻辑 |
| `src/persistence.rs` | 修改 | UiPersistState 扩展 |
