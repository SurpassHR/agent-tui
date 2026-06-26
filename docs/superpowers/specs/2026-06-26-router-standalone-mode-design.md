# Router 独立启动模式 — 设计文档

> 日期：2026-06-26 | 状态：草稿 | 修订：v2（增加配置 TUI）

## 概述

为 `agent-tui` 新增 `cargo run -- --router` 运行模式，提供一个轻量 TUI 用于管理 Provider/Model 配置，Router 在后台作为 HTTP 代理服务器运行。

当前 router 完全寄生在主 TUI 生命周期内——主 TUI 启动时加载 provider 配置、启动 axum 服务器，主 TUI 退出时 router 随之终止。`--router` 模式将其拆为独立的轻量 TUI，聚焦于 Provider/Model 配置管理和 Router 状态监控。

## 问题背景

TUI 模式下 router 与完整编排工作台耦合过紧，导致：
- 想单独用 router 做 API 代理时，必须启动完整的 pi RPC + 会话管理 + 右侧面板等不相关功能
- 调试网络路由问题/验证 provider 配置时，需要排除 TUI 渲染和 pi 交互的干扰
- 需要一个轻量工具专门管理 provider 配置，而非在主 TUI 侧边栏中操作

## 目标范围

| 场景 | 支持 |
|------|------|
| `cargo run -- --router` 启动 Router TUI | ✅ |
| TUI 中查看 Provider 列表 | ✅ |
| TUI 中新增/编辑/删除 Provider | ✅ |
| Provider 内 Model 管理（展开查看、添加、删除） | ✅ |
| Router 在后台自动运行，状态显示在顶部栏 | ✅ |
| 更新配置后 Router 自动同步（`SharedConfig`） | ✅ |
| `Ctrl+C` / `q` 优雅退出（停止 Router + 恢复终端） | ✅ |
| 日志输出到 stderr（复用现有 tracing subscriber） | ✅ |

| 不在此范围 | 原因 |
|---|---|
| 实时请求日志 / 请求统计 | HUD 消耗大，后期可加 |
| Provider 配置导入/导出 | YAGNI |
| 端口覆盖 CLI 参数 | 编辑 providers.json `port` 字段即可 |
| 多开 Router 实例冲突处理 | 继承既有端口递增行为 |

## 交互设计

### 布局

```
┌──────────────────────────────────────────────────────┐
│  Provider Router  ● 127.0.0.1:8001  🟢 Running      │  ← 顶部状态栏
├──────────────────────────────────────────────────────┤
│                                                      │
│  ── Providers ───────────────────────────── 2 ────  │
│                                                      │
│  ● DeepSeek  (openai_compat)   3 models  [ENABLED]  │  ← 选中态 ●
│    ├── deepseek-chat             T1  128K ctx        │  ← 展开后显示 model
│    ├── deepseek-reasoner         T1  128K ctx        │
│    └── … [1 more]                                    │
│                                                      │
│  ○ OpenAI  (openai_responses)  2 models  [ENABLED]  │  ← 未选中态 ○
│    ├── gpt-4o                    T1  128K ctx        │
│    └── gpt-4o-mini               T2  128K ctx        │
│                                                      │
│  [+]  [-]  [e]dit                                   │  ← 操作键提示行
│                                                      │
├──────────────────────────────────────────────────────┤
│  [a] Add Provider  [d] Delete  [e] Edit  [ENTER]    │  ← 底部快捷键提示
│  Expand/Collapse  [q] Quit                           │
└──────────────────────────────────────────────────────┘
```

### 快捷键

| 键 | 操作 |
|----|------|
| `↑` / `↓` | 在 Provider 中导航 |
| `←` / `→` 或 `Enter` | 展开/折叠当前 Provider 的 Model 列表 |
| `a` | 新增 Provider（弹出表单） |
| `d` | 删除选中的 Provider（确认弹窗） |
| `e` | 编辑选中 Provider（弹出表单，预填当前值） |
| `q` / `Ctrl+C` | 退出（停止 Router + 恢复终端） |

Provider 内的 Model：
- 折叠态仅显示 Model 数量
- 展开态显示每个 Model 的 id、tier、context_window
- Model 本身不支持独立增删（精简交互），通过编辑 Provider 统一管理

### 弹出表单

新增/编辑 Provider 时弹出表单弹窗：

```
┌─────────── Edit Provider ───────────┐
│  ID:        deepseek                 │
│  Name:      DeepSeek                 │
│  Base URL:  https://api.deepseek.com │
│  API Key:   ********                 │
│  Endpoint:  openai_compat  ▾         │
│  Enabled:   [x]                      │
│  Models:    deepseek-chat:T1:128000  │
│             deepseek-reasoner:T1:64000│
│                                      │
│  [Enter] Save  [Esc] Cancel          │
└──────────────────────────────────────┘
```

编辑时不直接编辑 models 字段，而是在 Provider 级别提供 models 文本域，每行格式 `model_id:tier:context_window`（与当前拉取模型列表的格式一致）。

## 技术设计

### 新文件：`src/router_tui.rs`

自包含的 Ratatui TUI 应用，不依赖 `src/tui.rs` 的框架。

```rust
pub async fn run_router_tui() -> Result<(), crate::errors::Error>
```

**内部结构**：

```
RouterTuiState {
    providers: Vec<ProviderInfo>,           // 当前 provider 列表
    selected: usize,                        // 选中索引
    expanded: Option<usize>,                // 展开的 provider 索引
    router_running: bool,                   // router 状态
    router_port: u16,                       // router 端口
    popup: Option<PopupKind>,               // 弹窗状态 (None/AddForm/EditForm/ConfirmDelete)
    dirty: bool,                            // 配置有变更需保存
    shared_config: SharedConfig,            // 与 router 共享的配置
}
```

**事件循环**：
```
初始化终端 (crossterm::terminal)
初始化 SharedConfig（从磁盘加载） 
启动 Router（后台 tokio::spawn）
循环:
    处理键盘事件
    更新状态
    保存配置（如有变更，同步到 SharedConfig + 写磁盘）
    渲染
退出:
    停止 Router（通过 stop_signal 通知）
    恢复终端
```

### 复用/引用

| 需求 | 来源 |
|------|------|
| Provider 数据模型 | `src/provider/mod.rs` (ProviderInfo, ProviderConfig) |
| SharedConfig 类型 | `src/provider/mod.rs` |
| Router 启动 | `src/provider/router.rs` — `start_router()` |
| 配置加载/保存 | `src/provider/mod.rs` — `load()`, `save()` |
| 确认弹窗 | `src/components/popup.rs` — 可提取通用弹窗，或直接复用 |
| Markdown 渲染 | 不需要（无消息渲染） |
| 颜色/主题 | 内联定义简单色值（不依赖 `src/theme.rs`），或轻量引用 |

### 配置同步

配置变更时：
1. 更新 `RouterTuiState.providers`
2. 写锁 `shared_config.write()` → 更新 `config.providers`
3. 写磁盘 `ProviderConfig::save(&config_path, &*shared_config.read().await)`

Router 的每个请求持有 `SharedConfig` 的读锁，因此配置更新立即对后续请求生效。

## 变更范围

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `src/config.rs` | 新增字段 | `--router` CLI flag |
| `src/main.rs` | 新增分支 | `config.router` → `run_router_tui()` |
| `src/provider/router.rs` | 签名变更 | `start_router()` 增加可选 `shutdown` 参数 |
| `src/router_tui.rs` | **新文件** | Router 配置 TUI 的全部逻辑 |

### 无改动的文件

- `src/tui.rs` — 签名兼容（`start_router(shared, None).await`）
- `src/provider/mod.rs` — 数据模型不变
- `src/components/` — 不依赖现有组件（自包含）
- `Cargo.toml` — 不需要新依赖

## 错误处理

| 场景 | 行为 |
|------|------|
| providers.json 不存在 | 空 Provider 列表，Router 使用默认配置启动 |
| 端口被占用 | start_router 自动递增端口 |
| Router 启动失败 | 打印错误日志，TUI 继续运行（显示 Router 错误状态） |
| 配置保存失败 | 打印警告日志，TUI 继续 |
| 终端 resize | crossterm 自动处理 |
| Ctrl+C 信号 | 停止 Router + 恢复终端 + 退出码 0 |

## 边界情况

- **空 Provider 列表**：Router 可启动但所有 /v1/* 请求返回 404
- **表单字段验证**：ID/base_url 不可为空，Endpoint 类型从预定义列表选择
- **API Key 显示**：编辑时显示 `****` 掩码（仅显示是否有值，不泄露密钥）
- **配置变更未保存退出**：不设自动保存提醒（每次变更即保存）
- **Provider 名包含特殊字符**：ID 仅允许字母/数字/下划线/连字符，提交时校验
- **终端过小**：最小宽 80 高 24，否则显示提示信息

## 测试

```bash
# 交互式测试
cargo run -- --router

# 验证 Router 状态
curl http://127.0.0.1:8001/v1/models
```
