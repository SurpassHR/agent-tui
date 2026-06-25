# 多 Agent 进程并行支持 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使 agent-tui 支持同时运行多个 pi agent 进程，双击 Enter 连接/断开会话，退出时清理所有子进程，重启时恢复活跃会话。

**Architecture:** 新增 `AgentManager` 管理多个 `PiRpcBackend` 实例。每个 agent 的 `event_rx` 通过 channel 合并到主循环。`App::active_sessions: HashSet<String>` 替代单值 `agent_session_id`。持久化扩展为保存活跃会话列表。

**Tech Stack:** Rust, tokio, portable-pty, existing `PiRpcBackend`/`PiRpcClient`

## Global Constraints

- Rust edition 2024，MSRV 1.85
- 所有 `pub` 接口须有 `///` 文档注释
- 禁止 `unwrap()`，使用 `?` 操作符
- 异步：使用 `tokio`，禁止 `std::thread::spawn`
- Action 新增 variant 时同步更新所有 `match` 分支
- `cargo build`, `cargo test` (118 tests), `cargo clippy -- -D warnings` 全部通过

---

### Task 1: 数据结构准备 — App 和 Action

**Files:**
- Modify: `src/app.rs:1140-1168` (App struct)
- Modify: `src/action.rs:96-100` (Action enum)

**Interfaces:**
- Consumes: existing `App` struct, `Action` enum, `HashSet`
- Produces: `App.active_sessions: HashSet<String>`, `App.agent_manager: AgentManager`, `Action::ConnectSession{..}`, `Action::DisconnectSession(String)`

- [ ] **Step 1: App 结构体替换字段**

```rust
// src/app.rs — App struct
pub struct App {
    // ... 保持前面的字段不变 ...

    // ── 删除以下两行 ──
    // pub agent_session_id: Option<String>,
    // pub agent_session_file: Option<String>,

    // ── 新增 ──
    /// 活跃会话 ID 集合（is_online 绿点依据，替代原 agent_session_id 单值）
    pub active_sessions: std::collections::HashSet<String>,
    /// 多 agent 进程管理器
    pub agent_manager: crate::backend::agent_manager::AgentManager,

    // ── 以下不变 ──
    pub use_rpc: bool,
    pub messages: HashMap<String, Vec<ChatMessage>>,
    pub active_agent: Option<String>,
    pub runtime: AgentRuntimeState,
    pub agent_status: AgentStatus,
    pub session: SessionInfo,
    pub tui: TuiState,
}
```

- [ ] **Step 2: 初始化 active_sessions 和 agent_manager**

```rust
// src/app.rs — App::new() 和 App::new_rpc()
// 在两个构造函数的 Self { ... } 中添加：

// new() 中：
Self {
    // ... 前面字段 ...
    active_sessions: std::collections::HashSet::new(),
    agent_manager: crate::backend::agent_manager::AgentManager::new(),
    // ... 后面字段 ...
}

// new_rpc() 中同理添加这两行
```

- [ ] **Step 3: Action 枚举添加新 variant**

```rust
// src/action.rs
    /// 浏览会话（仅加载消息，不改变活动状态）
    SelectSession(String),
    /// 为指定会话启动 pi agent 进程
    ConnectSession {
        session_id: String,
        file_path: String,
    },
    /// 停止指定会话的 pi agent 进程
    DisconnectSession(String),
    /// 侧边栏焦点移动
```

- [ ] **Step 4: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

预期: 编译失败 — `agent_manager` 模块尚未创建，`agent_session_id`/`agent_session_file` 被删除导致其他文件引用失败。

- [ ] **Step 5: 修复删除 agent_session_id/file 导致的编译错误**

在 `src/tui.rs` 和 `src/app.rs` 中搜索 `agent_session_id` 和 `agent_session_file`，删除所有相关代码。具体位置：

`src/tui.rs:560-571` — 状态轮询中的 agent_session_file 设置 → 删除这些行：

```rust
// 删除以下代码块：
// app.agent_session_file = sfile.clone();
```

`src/app.rs` — sync_components 中的 is_online 更新 → 改为使用 active_sessions：

```rust
// 旧代码（删除）：
// for ws in &mut self.tui.workspaces {
//     for s in &mut ws.sessions {
//         s.is_online = self.agent_session_file.as_ref()
//             .is_some_and(|af| s.file_path.as_ref().is_some_and(|fp| fp == af));
//     }
// }

// 新代码（替换为）：
for ws in &mut self.tui.workspaces {
    for s in &mut ws.sessions {
        s.is_online = self.active_sessions.contains(&s.id);
    }
}
```

- [ ] **Step 6: 再次编译**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

预期: 仍编译失败 — AgentManager 模块尚未创建，但 agent_session_id 相关错误已消除。

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/action.rs src/tui.rs
git commit -m "refactor(session): 用 active_sessions 替代 agent_session_id 单值"
```

---

### Task 2: AgentManager 模块

**Files:**
- Create: `src/backend/agent_manager.rs`
- Modify: `src/backend/mod.rs`

**Interfaces:**
- Consumes: `PiRpcBackend`, `PiRpcClient`, `PiEvent`, `tokio::sync::mpsc`
- Produces: `AgentManager::new()`, `spawn()`, `kill()`, `kill_all()`, `is_active()`

- [ ] **Step 1: 创建 agent_manager.rs**

```rust
// src/backend/agent_manager.rs
//! 多 Agent 进程管理器
//!
//! 管理多个 pi agent 子进程的生命周期：spawn、kill、事件转发。

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc;

use super::event::PiEvent;
use super::rpc::PiRpcBackend;
use super::rpc_client::PiRpcClient;
use crate::errors::Result;

/// 单个 agent 进程的完整句柄
pub struct AgentProcess {
    /// 会话 ID（工作区树中的 session id）
    pub session_id: String,
    /// JSONL 会话文件路径
    pub session_path: PathBuf,
    /// pi 子进程后端
    pub backend: PiRpcBackend,
    /// RPC 通信客户端
    pub client: PiRpcClient,
}

/// 多 agent 进程管理器
///
/// 生命周期：
/// - `spawn()` → 启动 pi 子进程，注册事件转发
/// - `kill()` → 停止进程并移除
/// - `kill_all()` → 退出时停止所有进程
pub struct AgentManager {
    agents: HashMap<String, AgentProcess>,
}

impl AgentManager {
    /// 创建空的管理器实例
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// 为指定会话启动 pi agent 进程
    ///
    /// `event_tx` 用于将 agent 的 RPC 事件转发到主循环的事件流。
    pub async fn spawn(
        &mut self,
        session_id: String,
        session_path: PathBuf,
        cwd: PathBuf,
        event_tx: mpsc::UnboundedSender<(String, PiEvent)>,
    ) -> Result<()> {
        let mut backend = PiRpcBackend::new("pi", &cwd);
        let session_path_str = session_path.to_string_lossy().to_string();
        let client = backend.start(Some(&session_path_str)).await?;

        // 将 agent 的 event_rx 转发到统一 event stream
        let rx = client.event_rx.resubscribe();
        let sid = session_id.clone();
        tokio::spawn(async move {
            let mut rx = rx;
            while let Some(event) = rx.recv().await {
                if event_tx.send((sid.clone(), event)).is_err() {
                    break;
                }
            }
        });

        self.agents.insert(
            session_id.clone(),
            AgentProcess {
                session_id,
                session_path,
                backend,
                client,
            },
        );

        Ok(())
    }

    /// 停止指定会话的 pi agent 进程
    pub async fn kill(&mut self, session_id: &str) {
        if let Some(mut proc) = self.agents.remove(session_id) {
            let _ = proc.backend.stop().await;
        }
    }

    /// 退出时停止所有 agent 进程
    pub async fn kill_all(&mut self) {
        for (_, mut proc) in self.agents.drain() {
            let _ = proc.backend.stop().await;
        }
    }

    /// 检查指定会话是否有活跃 agent
    pub fn is_active(&self, session_id: &str) -> bool {
        self.agents.contains_key(session_id)
    }

    /// 获取指定 agent 的 RPC client（用于状态轮询等）
    pub fn client_mut(&mut self, session_id: &str) -> Option<&mut PiRpcClient> {
        self.agents.get_mut(session_id).map(|p| &mut p.client)
    }

    /// 返回所有活跃 agent 的引用（用于状态轮询遍历）
    pub fn agents_mut(&mut self) -> impl Iterator<Item = &mut AgentProcess> {
        self.agents.values_mut()
    }
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 更新 backend/mod.rs 导出**

```rust
// src/backend/mod.rs
pub mod agent_manager;
pub mod event;
pub mod rpc;
pub mod rpc_client;
// ... 其余不变
```

- [ ] **Step 3: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

预期: PiRpcClient 可能没有 `event_rx` 的 `resubscribe()` 方法，需要检查 PiRpcClient 结构。

- [ ] **Step 4: 检查并修复 PiRpcClient 的 event_rx 访问**

```bash
# 检查 PiRpcClient 结构
cd /home/hr/Projects/agent-tui && cargo build 2>&1 | grep "event_rx\|resubscribe\|no field"
```

如果 `event_rx` 是私有字段，需要为 `PiRpcClient` 添加公开方法。确认后修复。

- [ ] **Step 5: Commit**

```bash
git add src/backend/agent_manager.rs src/backend/mod.rs
git commit -m "feat(backend): 新增 AgentManager 多进程管理器"
```

---

### Task 3: 事件流合并

**Files:**
- Modify: `src/tui.rs:305-475` (run_tui 初始化 + 主循环)

**Interfaces:**
- Consumes: `AgentManager::spawn(event_tx)`, `mpsc::unbounded_channel`
- Produces: 统一的 `agent_event_rx` 在 `tokio::select!` 中使用

- [ ] **Step 1: 添加 agent_event channel**

在 `run_tui()` 中，pi agent 启动成功之后、主循环之前添加：

```rust
// src/tui.rs run_tui() 中

// 统一事件接收 channel
let (agent_event_tx, mut agent_event_rx) =
    tokio::sync::mpsc::unbounded_channel::<(String, crate::backend::event::PiEvent)>();
```

- [ ] **Step 2: 修改主循环 select!**

将原 `client.event_rx.try_recv()` 从 `ticker.tick()` 分支移除，改为独立分支：

```rust
loop {
    tokio::select! {
        _ = ticker.tick() => {
            // 移除原有的 client.event_rx.try_recv() 代码块
            // 只保留：同步输入缓冲区、检查模型拉取结果、渲染
            app.tui.main_view.input_buffer.clone_from(&input_buffer);
            // ... 模型拉取检查 ...
            let _ = terminal.try_draw(|f| {
                app.render_tui(f);
                Ok::<_, std::io::Error>(())
            });
        }

        _ = state_poll_ticker.tick() => {
            // 状态轮询 — 遍历所有 agent
            for proc in app.agent_manager.agents_mut() {
                if let Ok(resp) = proc.client.request(
                    serde_json::json!({"type": "get_state"}),
                    Duration::from_secs(5),
                ).await {
                    if let Some(data) = &resp.data {
                        if let Some(model) = data.get("model") {
                            app.runtime.model_name = model.get("name")
                                .and_then(|v| v.as_str()).map(String::from);
                            app.runtime.provider = model.get("provider")
                                .and_then(|v| v.as_str()).map(String::from);
                        }
                    }
                }
            }
            let _ = app.handle_action(Action::RuntimeStateUpdate(app.runtime.clone())).await;
        }

        Some((agent_id, event)) = agent_event_rx.recv() => {
            // 来自任意 agent 的 RPC 事件
            tracing::debug!("EVENT from {}: {:?}", agent_id, std::mem::discriminant(&event));
            let actions = translate_pi_events(event, &agent_id);
            for action in actions {
                if let Err(e) = app.handle_action(action).await {
                    tracing::error!("handle_action error: {}", e);
                }
            }
        }

        Some(Ok(event)) = crossterm_event_stream.next() => {
            // 键盘/鼠标事件处理（不变）
            // ... 现有代码 ...
        }

        Some(action) = action_rx.recv() => {
            // channel action（不变）
            if let Err(e) = app.handle_action(action).await {
                tracing::error!("handle_action error: {}", e);
            }
        }
    }
}
```

- [ ] **Step 3: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): 多 agent 事件流合并到主循环"
```

---

### Task 4: ConnectSession / DisconnectSession 处理

**Files:**
- Modify: `src/app.rs:1575-1600` (handle_action — ConnectSession/DisconnectSession)

**Interfaces:**
- Consumes: `AgentManager::spawn()`, `active_sessions`, `load_session_messages()`
- Produces: ConnectSession 和 DisconnectSession 的完整处理逻辑

- [ ] **Step 1: 实现 ConnectSession handler**

在 `handle_action` 的 `SelectSession` 分支之后添加：

```rust
Action::ConnectSession {
    session_id,
    file_path,
} => {
    let cwd = std::env::current_dir().unwrap_or_default();
    let event_tx = self.event_tx.clone(); // 需要从 run_tui 传入
    // 注意: event_tx 需要添加为 App 字段，见 Step 2
    match self.agent_manager.spawn(
        session_id.clone(),
        std::path::PathBuf::from(&file_path),
        cwd,
        event_tx,
    ).await {
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
            // 从工作区树获取会话名称
            let name = self.tui.workspaces.iter()
                .find_map(|ws| ws.sessions.iter().find(|s| s.id == session_id))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| session_id.clone());
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
    // 从工作区树获取会话名称
    let name = self.tui.workspaces.iter()
        .find_map(|ws| ws.sessions.iter().find(|s| s.id == session_id))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| session_id.clone());
    self.tui.bottom_bar.status = format!("已断开: {}", name);
    crate::persistence::save(&self.build_persist_state());
}
```

- [ ] **Step 2: 添加 event_tx 字段到 App**

由于 `ConnectSession` 需要 `event_tx` 来创建新的 agent 进程，需要将此 channel 的 sender 存入 App：

```rust
// src/app.rs — App struct 新增字段
/// agent 事件转发 channel sender（由 run_tui 传入，供 ConnectSession 使用）
event_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, crate::backend::event::PiEvent)>>,
```

在 `new()` 和 `new_rpc()` 中初始化为 `None`。在 `run_tui()` 创建 channel 后设置：

```rust
app.event_tx = Some(agent_event_tx);
```

- [ ] **Step 3: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat(session): 实现 ConnectSession / DisconnectSession 处理"
```

---

### Task 5: 双击 Enter 触发 ConnectSession

**Files:**
- Modify: `src/tui.rs:1200-1270` (Workspace Enter 处理)

**Interfaces:**
- Consumes: `SessionNode.file_path`, `TuiState.last_enter_session`
- Produces: 双击检测 → `ConnectSession`, 单击 → `SelectSession`

- [ ] **Step 1: 查找现有 Enter 处理代码**

目前 Workspace 子区的 Enter 已经有双击检测（`last_enter_session`），但触发的是旧版 `ConnectSession(session_id)`。需要改为 `ConnectSession { session_id, file_path }`。

```rust
// src/tui.rs — Workspace 子区 Enter 处理（替换 is_double 分支）
if is_double {
    app.tui.last_enter_session = None;
    if let Some(file_path) = session.file_path.clone() {
        app.handle_action(Action::ConnectSession {
            session_id: session.id.clone(),
            file_path,
        })
        .await
        .ok();
    }
} else {
    app.tui.last_enter_session = Some((session.id.clone(), now));
    app.handle_action(Action::SelectSession(session.id.clone()))
        .await
        .ok();
}
```

- [ ] **Step 2: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

- [ ] **Step 3: 运行测试**

```bash
cd /home/hr/Projects/agent-tui && cargo test 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add src/tui.rs
git commit -m "feat(session): 双击 Enter 触发 ConnectSession"
```

---

### Task 6: 退出清理 + 启动恢复

**Files:**
- Modify: `src/tui.rs:1676` (退出前清理)
- Modify: `src/tui.rs:464` (启动后恢复)
- Modify: `src/persistence.rs`

**Interfaces:**
- Consumes: `AgentManager::kill_all()`, `UiPersistState`
- Produces: 清理 + 恢复逻辑

- [ ] **Step 1: 退出时 kill_all**

在 `run_tui()` 退出点（Ctrl+C 或正常退出）前添加：

```rust
// src/tui.rs — run_tui() 退出前
tracing::info!("TUI 主循环结束，清理所有 agent 进程");
app.agent_manager.kill_all().await;
let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
let _ = ratatui::try_restore();
```

- [ ] **Step 2: 持久化扩展**

```rust
// src/persistence.rs — UiPersistState
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPersistState {
    pub active_session_id: Option<String>,
    /// 新增：活跃 agent 会话 ID 列表
    #[serde(default)]
    pub active_agent_sessions: Vec<String>,
    pub expanded_workspaces: Vec<String>,
}

// build_persist_state() 更新：
pub fn build_persist_state(&self) -> UiPersistState {
    UiPersistState {
        active_session_id: self.active_agent.clone(),
        active_agent_sessions: self.active_sessions.iter().cloned().collect(),
        expanded_workspaces: self.tui.workspaces.iter()
            .filter(|ws| ws.expanded)
            .map(|ws| ws.cwd.clone())
            .collect(),
    }
}
```

- [ ] **Step 3: 启动时恢复**

在 `run_tui()` 中 `populate_workspaces()` 之后、主循环之前添加：

```rust
// src/tui.rs — 恢复活跃会话
if let Some(state) = crate::persistence::load() {
    for session_id in &state.active_agent_sessions {
        if let Some(file_path) = app.tui.workspaces.iter()
            .find_map(|ws| ws.sessions.iter().find(|s| s.id == *session_id))
            .and_then(|s| s.file_path.clone())
        {
            let cwd = std::env::current_dir().unwrap_or_default();
            let _ = app.agent_manager.spawn(
                session_id.clone(),
                std::path::PathBuf::from(&file_path),
                cwd,
                agent_event_tx.clone(),
            ).await;
            app.active_sessions.insert(session_id.clone());
        }
    }
    app.sync_components();
}
```

- [ ] **Step 4: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add src/tui.rs src/persistence.rs
git commit -m "feat(session): 退出清理所有 agent + 启动恢复活跃会话"
```

---

### Task 7: 清理旧代码 + 联调

**Files:**
- Modify: `src/app.rs` (sync_components is_online 更新)
- Modify: `src/tui.rs` (状态轮询清理)

- [ ] **Step 1: sync_components 中 is_online 更新**

确认 `sync_components` 中 is_online 更新使用 `active_sessions`（已在 Task 1 Step 5 完成）。

- [ ] **Step 2: 清理状态轮询中残留的 agent_session 代码**

在 `state_poll_ticker` 分支中移除残留的 `app.session.id = ...` 等设定：

```rust
// 删除残留代码块（如果还存在）：
// app.session.id = sid.unwrap_or_default();
// app.session.file_path = sfile;
// app.session.name = sname;
```

- [ ] **Step 3: 全局搜索 agent_session_id / agent_session_file 残留**

```bash
cd /home/hr/Projects/agent-tui && grep -r "agent_session_id\|agent_session_file" src/ 2>&1
```

预期: 无输出（无残留引用）。

- [ ] **Step 4: 全量编译 + 测试 + Lint**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1 && cargo test 2>&1 | tail -5 && cargo clippy -- -D warnings 2>&1
```

预期: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: 清理旧 agent_session 代码残留"
```

---

### Task 8: 单元测试

**Files:**
- Create: 测试代码追加到 `src/app.rs` tests 模块末尾

- [ ] **Step 1: 编写 AgentManager 单测**

```rust
// src/app.rs tests 模块末尾
#[cfg(test)]
mod agent_manager_tests {
    use super::*;
    use crate::backend::agent_manager::AgentManager;

    #[test]
    fn test_agent_manager_new_is_empty() {
        let mgr = AgentManager::new();
        assert!(!mgr.is_active("any-session"));
    }
}
```

- [ ] **Step 2: 编写 active_sessions 逻辑单测**

```rust
#[test]
fn test_is_online_from_active_sessions() {
    let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
    app.tui.workspaces.push(WorkspaceNode {
        cwd: "/test".into(),
        display_name: "test".into(),
        sessions: vec![SessionNode {
            id: "sess-1".into(),
            name: "Test".into(),
            file_path: None,
            message_count: 0,
            is_online: false,
        }],
        expanded: true,
    });
    
    // 未连接时 is_online 为 false
    app.sync_components();
    assert!(!app.tui.workspaces[0].sessions[0].is_online);
    
    // 连接后 is_online 为 true
    app.active_sessions.insert("sess-1".into());
    app.sync_components();
    assert!(app.tui.workspaces[0].sessions[0].is_online);
}
```

- [ ] **Step 3: 编写持久化单测**

```rust
#[test]
fn test_persist_active_agent_sessions() {
    let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
    app.active_sessions.insert("sess-a".into());
    app.active_sessions.insert("sess-b".into());
    
    let state = app.build_persist_state();
    assert_eq!(state.active_agent_sessions.len(), 2);
    assert!(state.active_agent_sessions.contains(&"sess-a".to_string()));
    assert!(state.active_agent_sessions.contains(&"sess-b".to_string()));
}
```

- [ ] **Step 4: 运行测试**

```bash
cd /home/hr/Projects/agent-tui && cargo test agent_manager_tests 2>&1 | tail -5
cargo test test_is_online 2>&1 | tail -5
cargo test test_persist 2>&1 | tail -5
```

- [ ] **Step 5: 全量测试 + Lint**

```bash
cd /home/hr/Projects/agent-tui && cargo test 2>&1 | tail -5 && cargo clippy -- -D warnings 2>&1
```

预期: 全部通过（新增 3 个测试，总计 121 测试）。

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "test(agent): AgentManager + active_sessions 单元测试"
```

---

### Task 9: ACTIVE SESSION 区显示多活跃会话

**Files:**
- Modify: `src/components/sidebar.rs:147-170` (render_active_session)
- Modify: `src/app.rs:1975-1990` (sync_components active_session 同步)

**Interfaces:**
- Consumes: `App.active_sessions`, 工作区树
- Produces: 多行 ACTIVE SESSION 渲染

- [ ] **Step 1: 更新 Sidebar 渲染方法**

```rust
// src/components/sidebar.rs — render_active_session → render_active_sessions
fn render_active_sessions(&self, theme: &Theme) -> Vec<Line<'static>> {
    let has_focus = self.has_focus && self.subsection == SidebarSubsection::ActiveSession;
    
    // 标题
    let title = if has_focus && self.active_session_cursor == 0 {
        Line::from(vec![
            "▎".fg(theme.accent),
            "ACTIVE SESSION".fg(theme.accent).bold(),
        ])
        .style(Style::default().bg(theme.highlight_bg))
    } else if has_focus {
        Line::from(vec![
            "▎".fg(theme.accent),
            "ACTIVE SESSION".fg(theme.accent).bold(),
        ])
    } else {
        Line::from(vec![
            Span::from(" "),
            "ACTIVE SESSION".fg(theme.heading).bold(),
        ])
    };
    let mut lines = vec![title];
    
    // 每个活跃会话一行
    let mut idx = 1; // cursor 偏移（0=title）
    for ws in &self.workspaces {
        for s in &ws.sessions {
            if s.is_online {
                let is_selected = has_focus && self.active_session_cursor == idx;
                let (fg, bg) = if is_selected {
                    (theme.selection_fg, theme.highlight_bg)
                } else {
                    (theme.text, theme.bg)
                };
                let sid_short = &s.id[..s.id.len().min(12)];
                lines.push(
                    Line::from(vec![
                        "● ".fg(theme.success),
                        Span::from(s.name.clone()).fg(fg),
                        Span::from(format!("  {}", sid_short)).fg(theme.border_dim),
                    ])
                    .style(Style::default().bg(bg)),
                );
                idx += 1;
            }
        }
    }
    
    if idx == 1 {
        // 无活跃会话
        lines.push(Line::from("○ 暂无活跃会话".fg(theme.text_dim)));
    }
    
    lines
}
```

- [ ] **Step 2: 更新 render 方法调用**

将原 `render_active_session` 调用替换为 `render_active_sessions`，返回多行后追加到 `lines` 列表：

```rust
// sidebar.rs render() 方法中
let active_lines = self.render_active_sessions(theme);
lines.extend(active_lines);
lines.push(Line::from("")); // 空行分隔
```

- [ ] **Step 3: 编译验证**

```bash
cd /home/hr/Projects/agent-tui && cargo build 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/components/sidebar.rs
git commit -m "feat(sidebar): ACTIVE SESSION 区显示多活跃会话"
```

---

## 执行顺序

Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6 → Task 7 → Task 8 → Task 9

每个 Task 独立提交，Task 1 和 Task 2 可并行（不同文件），其余顺序依赖。
