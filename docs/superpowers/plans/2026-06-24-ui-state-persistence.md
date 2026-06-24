# UI 状态持久化实现计划

> **For agentic workers:** 使用 subagent-driven-development 按任务逐个实现。步骤使用 checkbox (`- [ ]`) 语法跟踪。

**目标:** 关闭 TUI 后工作区展开状态和活跃会话不丢失，重启时自动恢复。

**架构:** 新建 `src/persistence.rs` 提供 `UiPersistState` 的 serde 序列化/反序列化和原子文件读写；在 `app.rs` 的 `ToggleWorkspace` / `SelectSession` 处理后触发保存；在 `tui.rs` 启动时恢复状态、退出时最终保存。

**技术栈:** Rust + serde_json（已在依赖中），无新增依赖。

## 全局约束

- Rust edition 2024，MSRV 1.85
- `cargo clippy -- -D warnings` 零警告
- 所有 `pub fn` / `pub struct` 必须有 `///` 文档注释
- 错误处理：使用 `?` 操作符，禁止 `unwrap()`
- 禁止修改 `Cargo.lock`、`target/`、`.codegraph/`

---

### Task 1: 创建 `src/persistence.rs` 持久化模块

**文件:**
- Create: `src/persistence.rs`
- Modify: `src/lib.rs`（添加 `pub mod persistence;`）

**接口:**
- 产出: `UiPersistState` struct, `state_path()`, `load()`, `save()`

- [ ] **Step 1: 创建 `src/persistence.rs`**

```rust
//! UI 状态持久化模块
//!
//! 负责将侧边栏展开状态和活跃会话 ID 持久化到
//! `~/.config/agent-tui/state.json`，在 TUI 重启时恢复。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// UI 持久化状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiPersistState {
    /// 上次活跃的会话 ID（文件名去 .jsonl 后缀）
    pub active_session_id: Option<String>,
    /// 已展开的工作区名称列表（WorkspaceNode.name，显示名）
    pub expanded_workspaces: Vec<String>,
}

/// 获取 state.json 持久化文件路径
pub fn state_path() -> PathBuf {
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(val).join("agent-tui").join("state.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("agent-tui")
            .join("state.json")
    } else {
        PathBuf::from(".agent-tui").join("state.json")
    }
}

/// 从 state.json 加载持久化状态
///
/// 文件不存在或 JSON 解析失败时返回默认值（空状态），
/// 不中断程序运行。
pub fn load() -> UiPersistState {
    let path = state_path();
    if !path.exists() {
        return UiPersistState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("state.json 解析失败: {} — 使用默认状态", e);
                UiPersistState::default()
            }
        },
        Err(e) => {
            tracing::warn!("state.json 读取失败: {} — 使用默认状态", e);
            UiPersistState::default()
        }
    }
}

/// 保存持久化状态到 state.json（原子写）
///
/// 先写 `.tmp` 临时文件，成功后再 `rename` 覆盖目标文件，
/// 避免写入中途崩溃导致文件损坏。
pub fn save(state: &UiPersistState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    match serde_json::to_string_pretty(state) {
        Ok(content) => {
            if std::fs::write(&tmp, &content).is_err() {
                tracing::error!("写入 state.json.tmp 失败");
                return;
            }
            if std::fs::rename(&tmp, &path).is_err() {
                tracing::error!("重命名 state.json.tmp 失败");
            }
        }
        Err(e) => tracing::error!("序列化 UiPersistState 失败: {}", e),
    }
}
```

- [ ] **Step 2: 在 `src/lib.rs` 中添加模块声明**

在 `pub mod message;` 之后添加：

```rust
pub mod persistence;
```

- [ ] **Step 3: 编译检查**

```bash
cargo build
```

预期：编译通过，无错误。

- [ ] **Step 4: 运行 clippy**

```bash
cargo clippy -- -D warnings
```

预期：零警告。

- [ ] **Step 5: 编写单元测试 — 序列化往返**

在 `src/persistence.rs` 末尾添加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_roundtrip() {
        let state = UiPersistState {
            active_session_id: Some("2026-06-24T13-52-35-984Z_019ef9e7".into()),
            expanded_workspaces: vec![
                "--home-hr-Projects-agent-tui--".into(),
                "--media-hr-Data-Codes-ideogram4-editor--".into(),
            ],
        };
        let json = serde_json::to_string(&state).expect("序列化失败");
        let restored: UiPersistState = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(restored.active_session_id, state.active_session_id);
        assert_eq!(restored.expanded_workspaces, state.expanded_workspaces);
    }

    #[test]
    fn test_default_is_empty() {
        let state = UiPersistState::default();
        assert!(state.active_session_id.is_none());
        assert!(state.expanded_workspaces.is_empty());
    }
}
```

- [ ] **Step 6: 运行测试**

```bash
cargo test persistence
```

预期：2 tests passed。

- [ ] **Step 7: Commit**

```bash
git add src/persistence.rs src/lib.rs
git commit -m "feat(persistence): 添加 UI 状态持久化模块"
```

---

### Task 2: 在 `app.rs` 中添加保存触发和恢复方法

**文件:**
- Modify: `src/app.rs`

**接口:**
- 消费: `persistence::save()`, `persistence::load()`, `UiPersistState`
- 产出: `App::build_persist_state()` 方法, `App::restore_persisted_state()` 方法
- 修改: `ToggleWorkspace` / `SelectSession` action 处理

- [ ] **Step 1: 添加 `build_persist_state` 方法**

在 `impl App` 块中添加（放在 `populate_workspaces` 方法附近）：

```rust
/// 从当前 TUI 状态构建持久化数据结构
pub fn build_persist_state(&self) -> crate::persistence::UiPersistState {
    crate::persistence::UiPersistState {
        active_session_id: self.active_agent.clone(),
        expanded_workspaces: self
            .tui
            .workspaces
            .iter()
            .filter(|ws| ws.expanded)
            .map(|ws| ws.name.clone())
            .collect(),
    }
}
```

- [ ] **Step 2: 添加 `restore_persisted_state` 方法**

在 `impl App` 块中添加：

```rust
/// 从 state.json 恢复工作区展开状态和活跃会话
///
/// 应在 `populate_workspaces()` 之后调用。
pub fn restore_persisted_state(&mut self) {
    let state = crate::persistence::load();

    // 恢复工作区展开状态
    for ws in &mut self.tui.workspaces {
        ws.expanded = state.expanded_workspaces.contains(&ws.name);
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
            // 注意：这里不直接调用 handle_action 因为需要 &mut self，
            // 直接内联 SelectSession 的核心逻辑
            self.tui.active_session = session_id.clone();
            // 查找 file_path 并加载消息
            if let Some(path) = self
                .tui
                .workspaces
                .iter()
                .find_map(|ws| {
                    ws.sessions
                        .iter()
                        .find(|s| s.id == *session_id)
                        .and_then(|s| s.file_path.clone())
                })
            {
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
                            if let Some(sess) =
                                ws.sessions.iter().find(|s| s.id == *session_id)
                            {
                                self.session.name = Some(sess.name.clone());
                            }
                        }
                        self.sync_messages_to_main_view(session_id);
                        self.sync_components();
                    }
                    Err(e) => {
                        tracing::warn!("恢复会话消息失败: {} — {}", path, e);
                    }
                }
            }
        } else {
            tracing::info!("上次活跃会话已不存在，不恢复");
        }
    }
}
```

- [ ] **Step 3: 在 `ToggleWorkspace` action 处理后添加保存**

找到 `Action::ToggleWorkspace(index)` 的处理，在 `ws.expanded = !ws.expanded;` 之后添加：

```rust
// 保存 UI 状态到 state.json
crate::persistence::save(&self.build_persist_state());
```

- [ ] **Step 4: 在 `SelectSession` action 处理后添加保存**

找到 `Action::SelectSession` 的最后（`sync_components()` 之后），在分支结束前添加：

```rust
// 保存 UI 状态到 state.json
crate::persistence::save(&self.build_persist_state());
```

- [ ] **Step 5: 编译检查**

```bash
cargo build
```

预期：编译通过。

- [ ] **Step 6: 运行 clippy + 全部测试**

```bash
cargo clippy -- -D warnings && cargo test
```

预期：零警告，全部测试通过。

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "feat(persistence): 在工作区展开/会话切换时自动保存状态"
```

---

### Task 3: 在 `tui.rs` 中集成状态恢复和退出保存

**文件:**
- Modify: `src/tui.rs`

**接口:**
- 消费: `App::restore_persisted_state()`, `App::build_persist_state()`, `persistence::save()`

- [ ] **Step 1: 在启动时恢复状态**

在 `populate_workspaces()` 调用之后，`populate_subagents()` 调用之前插入：

```rust
// 恢复持久化的 UI 状态（工作区展开 + 活跃会话）
app.restore_persisted_state();
```

具体位置（约第 464 行后）：

```diff
     // 扫描 session 目录，填充工作区数据
     app.populate_workspaces();
+    // 恢复持久化的 UI 状态（工作区展开 + 活跃会话）
+    app.restore_persisted_state();
     // 扫描 agents 目录，填充 subagent 列表
     app.populate_subagents();
```

- [ ] **Step 2: 在退出前保存状态**

在 `run_tui` 函数末尾，`tracing::debug!("TUI 主循环结束");` 之前添加：

```rust
// 最终保存 UI 状态
crate::persistence::save(&app.build_persist_state());
```

具体位置（约第 1436 行）：

```diff
+    // 最终保存 UI 状态
+    crate::persistence::save(&app.build_persist_state());
+
     tracing::debug!("TUI 主循环结束");
     backend.stop().await.ok();
```

- [ ] **Step 3: 编译检查**

```bash
cargo build
```

预期：编译通过。

- [ ] **Step 4: 运行 clippy + 全部测试**

```bash
cargo clippy -- -D warnings && cargo test
```

预期：零警告，全部测试通过。

- [ ] **Step 5: 手动集成测试**

```bash
cargo run -- --tui
```

测试步骤：
1. 展开几个工作区，折叠几个
2. 选择一个会话
3. Ctrl+C 退出
4. 检查 `~/.config/agent-tui/state.json` 是否生成
5. 重新启动 TUI，验证工作区展开状态和活跃会话是否恢复

- [ ] **Step 6: Commit**

```bash
git add src/tui.rs
git commit -m "feat(persistence): 启动时恢复状态，退出时保存状态"
```

---

### 自检清单

- [ ] Task 1: 创建 persistence 模块
- [ ] Task 2: 添加保存触发和恢复方法
- [ ] Task 3: 集成到 tui.rs
- [ ] 全部 clippy 零警告
- [ ] 全部测试通过
- [ ] 手动测试通过
