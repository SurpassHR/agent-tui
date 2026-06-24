# UI 状态持久化设计

> 状态：设计定稿 | 日期：2026-06-24

## 目标

关闭 TUI 后，侧边栏工作区展开/折叠状态和当前活跃会话不丢失，重启时自动恢复。

## 需求

| 维度 | 决策 |
|------|------|
| 持久化内容 | 工作区展开状态 + 活跃会话 ID |
| 存储文件 | `~/.config/agent-tui/state.json`（与 `providers.json` 同目录） |
| 格式 | JSON |
| 保存时机 | 工作区展开/折叠时、会话切换时、退出时 |
| 恢复时机 | 启动时 `populate_workspaces()` 之后 |
| 容错 | 文件不存在/JSON 解析失败 → 全部默认；会话已删除 → 静默跳过 |

## 数据结构

```json
{
  "active_session_id": "2026-06-24T13-52-35-984Z_019ef9e7-7490-70f2-969c-e805fd720bba",
  "expanded_workspaces": [
    "--home-hr-Projects-agent-tui--",
    "--media-hr-Data-Codes-ideogram4-editor--"
  ]
}
```

- `active_session_id` — 文件名去 `.jsonl` 后缀（即 SessionNode.id）
- `expanded_workspaces` — 已展开的工作区名称列表（WorkspaceNode.name，显示名）

### Rust 结构体

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiPersistState {
    pub active_session_id: Option<String>,
    pub expanded_workspaces: Vec<String>,
}
```

## 架构

```
App 启动
  └─ populate_workspaces()      // 扫描 sessions 目录
  └─ persistence::load()        // 读取 state.json
       ├─ 恢复 expanded_workspaces → 设置 WorkspaceNode.expanded
       └─ 恢复 active_session_id → 调用 SelectSession 加载消息

运行时
  ├─ ToggleWorkspace → 保存 state.json
  └─ SelectSession   → 保存 state.json

App 退出
  └─ persistence::save()        // 最终保存
```

## 改动清单

### `src/persistence.rs` — 新建

- `UiPersistState` 结构体（serde 序列化）
- `fn load() -> UiPersistState` — 从 `state.json` 读取
- `fn save(state: &UiPersistState)` — 写入 `state.json`（原子写，先写 `.tmp` 再 `rename`）
- `fn state_path() -> PathBuf` — 返回 `~/.config/agent-tui/state.json`

### `src/app.rs` — 修改

- `ToggleWorkspace` action：处理后调用 `persistence::save()`
- `SelectSession` action：处理后调用 `persistence::save()`
- 新增 `restore_persisted_state()` 方法：加载 `state.json`，恢复展开状态和活跃会话

### `src/tui.rs` — 修改

- `run_tui` 中：`populate_workspaces()` 后调用 `restore_persisted_state()`
- `run_tui` 退出前：调用 `persistence::save()`

### `Cargo.toml` — 无改动

`serde` / `serde_json` 已在依赖中。

## 容错策略

| 场景 | 行为 |
|------|------|
| `state.json` 不存在 | 返回 `UiPersistState::default()`，首次启动 |
| JSON 解析失败 | 记录 warn 日志，返回默认值 |
| `active_session_id` 指向已删除的会话 | 静默跳过，不报错 |
| `expanded_workspaces` 中包含已删除的工作区 | 忽略不存在的项 |
| 写文件失败（权限等） | 记录 error 日志，不影响正常运行 |
