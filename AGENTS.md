# Meta-TUI 智能体编排工作台

> Rust TUI 应用 — PTY 驱动的 AI Agent 多进程编排器。
> Ratatui 0.30.2 + Tokio + portable-pty | edition 2024 | MSRV 1.85

## 参考文档

- Ratatui API 文档（rustdoc-md 版）: `docs/api_docs/ratatui.md`（2853 行，涵盖所有公开 API）
- 设计文档: `docs/design_docs/`（架构设计、实现计划）
- 规格文档: `docs/superpowers/specs/`（Provider 路由等子系统规格）

## 构建与测试命令

```bash
cargo build                                           # 构建
cargo run -- --dry-run                                # 打印配置信息（不启动 pi）
	cargo run -- --tui                                    # 运行 TUI 模式（自动启动 pi RPC）
	cargo run -- --router                                 # 启动独立 Router 模式（Provider 配置管理 TUI）
cargo test                                            # 单元+集成测试
cargo test -- --nocapture                             # 测试（显示输出）
cargo clippy -- -D warnings                           # Lint（必须零警告）
cargo fmt --check                                     # 格式检查
```

## 提交策略

- **只在用户主动要求 commit 时才能执行 git commit**。任何时候都不要擅自提交。
- 完成阶段性改动后，提示用户是否需要 commit，但不要直接执行。
- 提示语简洁即可，如：「改动完成，是否需要 commit？」
- **Commit message 必须使用用户语言**，遵循 Conventional Commits 格式：`<type>(<scope>): <描述>`

## 编码约定

- **Rust edition 2024**，MSRV 1.85
- **错误处理**：公开 API 返回 `anyhow::Result`（通过 `errors::Result` 类型别名）；内部使用 `color-eyre`。**禁止 `unwrap()`**，使用 `?` 操作符。
- **文档**：所有 `pub trait`、`pub fn`、`pub struct` 必须有 `///` 文档注释。
- **异步**：使用 `tokio`，禁止 `std::thread::spawn`。
- **阻塞 I/O**：所有阻塞操作必须在 `tokio::task::spawn_blocking` 中执行。
- **PTY 读取**：使用 `MasterPty::try_clone_reader()` 获取读取端，在 `spawn_blocking` 中死循环读取。
- **僵尸进程**：`terminate()` 必须依次调用 `child.kill()` 和 `child.wait()`。
- **Action 枚举**：新增 variant 时同步更新所有 `match` 分支，或标注 `#[allow(unreachable_patterns)]`。
- **跨线程安全**：`AgentBackend` 实现必须是 `Send + Sync`，非 `Sync` 字段用 `Mutex` 包裹。

## 不要修改

- `target/` — 构建产物
- `.codegraph/` — CodeGraph 索引
- `Cargo.lock` — 由 cargo 管理
- `tests/common/` — 测试夹具，需与集成测试同步更新

## 项目结构要点

- `src/lib.rs` — 公共 API 库根
	- `src/main.rs` — 入口点 + CLI 解析（`--dry-run` / `--tui` / `--router`）
- `src/action.rs` — Action 枚举（全局通信骨架）
	- `src/app.rs` — App 状态机（持有 `TuiState`），包含 WorkspaceNode/SessionNode/SubAgentInfo 数据模型（渲染/状态操作/Drop 已拆分为 app_render / app_state_ops / app_drop 子模块）
- `src/persistence.rs` — UI 状态持久化（工作区展开 + 活跃会话 → state.json）
- `src/tui.rs` — TUI 渲染循环 + RPC 事件处理 + 鼠标/键盘事件 + 剪贴板
- `src/selection.rs` — 鼠标拖拽选中高亮和文本收集
- `src/theme.rs` — 主题色定义
- `src/backend/` — Agent 后端
  - `mod.rs` — `AgentBackend` trait
  - `rpc.rs` — `PiRpcBackend` 进程管理
  - `rpc_client.rs` — JSONL RPC 协议客户端
  - `event.rs` — PiEvent 枚举 + 解析
- `src/components/` — UI 组件
  - `sidebar.rs` — 左侧栏（ACTIVE SESSION / WORKSPACE 树 a/d/r 增删改 / PROVIDER + MODEL）
  - `main_view.rs` — 中央对话区 + 底部输入框 + 块折叠/进入视图
  - `markdown.rs` — Markdown → Ratatui Line 渲染器（GFM 表格 + 围栏代码块 + ANSI）
  - `agent_panel.rs` — 右侧栏（AGENTS / SKILLS / MCPS / TASKS 四区块）
  - `bottom_bar.rs` — 底部栏（左快捷键 / 中状态提示 / 右 Token 信息）
  - `top_bar.rs` — 顶部 Tab 栏
  - `popup.rs` — Popup 浮层组件
	- `src/message.rs` — ChatMessage 消息模型 + ContentBlock + EnteredView
	- `src/config.rs` — CLI 参数解析（--dry-run / --tui / --router）
	- `src/errors.rs` — 错误类型定义
	- `src/logging.rs` — tracing 日志初始化
	- `src/state.rs` — 会话状态和上下文管理
	- `src/tui_state.rs` — TUI 布局状态（滚动位置、折叠状态、输入光标）
	- `src/router_tui.rs` — Router 独立模式 TUI（Provider 配置管理界面）
	- `src/utils.rs` — 工具函数（JSON 解析、字符串处理等）
- `src/provider/` — Provider 路由系统
  - `mod.rs` — ProviderConfig 数据模型（含 endpoint_type 端点类型）+ 持久化 + TS 生成
  - `router.rs` — axum HTTP 反向代理（按 body.model 字段路由，根据 endpoint_type 构造目标路径）

## 当前架构

### 布局

```
┌──────────────┬─────────────────────────────────┬──────────────┐
│ LEFT SIDEBAR │          MAIN VIEW               │ AGENT PANEL  │
│ (40 cols)    │          (flex)                   │ (40 cols)    │
│              │                                   │              │
│ ● 会话名     │ ┌─ Diff: path (+n / -m) ───┐    │ AGENTS       │
│   sess_id    │ │ - 42 │ old line         │    │ ● pi  idle   │
│ 工作区       │ │ + 42 │ new line         │    │ ○ scout      │
│ ▼ 📁 proj   │ └────────────────────────┘    │ SKILLS       │
│   ├─ 会话    │                                   │ ■ skill-a    │
│   …          │ ┌──── code block ──────────┐    │ MCPS         │
│              │ │ │ 1 │ let x = 1;     │   │ ● server-a   │
│──────────────│ │ └───────────────────────┘    │              │
│ PROVIDER (2) │ ─────────── separator ───────    │              │
│ ● deepseek   │ > 输入框                          │              │
│ MODEL        │                                   │              │
│   v4-flash   │                                   │              │
├──────────────┴───────────────────────────────────┴──────────────┤
│ BOTTOM BAR: 快捷键 | 状态提示 | Context/Tokens/Cost             │
└─────────────────────────────────────────────────────────────────┘
```

### 数据流

```
pi agent (RPC) ──EventStream──→ tui.rs 事件循环
                                     ↓
                                handle_action()
                                     ↓
                                App (messages, runtime, session)
                                     ↓ sync_components()
                                TuiState (组件实例)
                                     ↓ render()
                                终端
```

- 三栏宽度一致（40 字符），中间区 Min(40) 自适应
- 鼠标拖拽选中三栏独立，选区不跨栏，松手自动复制到系统剪贴板
- 选中文本渲染时收集（selection.selected_text），不从 buffer 异步读取

## 协作原则

- **不确定就问，不要猜**：不理解用户说的术语时，先问清楚再动手。不要边猜边改。
- **先理解需求，再写代码**：每次改之前确认「你要的是什么」，而不是「我认为你要什么」。
- **一次只做一个改动**：改完先验证理解是否正确，再继续下一步。

### 反面案例

用户说「AGENTS 模块未对接成功，dry-run 实现成功输出所有 agents」：
1. ❌ 我没问「你说的 agents 具体指什么」
2. ❌ 我直接猜 agents = extensions，然后改代码
3. ❌ 用户纠正后我又猜 agents = pi agent 自身信息，继续改
4. ❌ 连续三次猜错，每次都在写代码而不是先确认

**正确做法：先问「什么是 agents？你期望 AGENTS 面板显示什么？」**

## 功能要点

- **工作区**：名称从 JSONL `cwd` 字段自动提取，显示末级目录（重名时加父级 `/` 区分）；手动添加需输入校验通过的项目路径
- **Provider 端点类型**：支持 openai_compat / openai_responses / anthropic_messages / gemini 四种端点，TS 动态生成 api 字段，配置变更时自动重新生成 local-provider.ts
- **持久化**：`~/.config/agent-tui/state.json` — 工作区展开状态 + 活跃会话 ID，退出时自动保存，启动时恢复；`~/.config/agent-tui/providers.json` — Provider 配置
	- **确认弹窗**：删除工作区/会话前弹出确认框（`d` 键触发，Enter 确认 / Esc 取消）
	- **思考级别控制**：Shift+Tab 循环切换 thinking level，从 `pi get_state` 读取并展示当前级别
	- **快照命令**：输入 `/snapshot` 将当前 TUI 画面保存为文本文件
	- **Markdown 宽度折行**：代码块和表格内容根据面板实际宽度自动折行
	- **输入区编辑**：支持左右光标移动、Ctrl/Alt 快速删除单词
	- **Router 独立模式**：`--router` 启动独立 Provider 配置管理 TUI，含列表视图和空格切换启用/禁用状态

## 当前已知局限

- **无分页/搜索**：消息区无 PageUp/Down 翻页或全文搜索
- **每帧全量重解析**：Markdown 渲染未缓存，100KB 量级实测可接受
- **多进程支持**：仅单 agent 实例（阶段五补）
	- **Diff 视图无滚动**：EnteredView::Diff 超出视窗内容不可见
	- **无会话改名 UI**：会话重命名依赖后端 RPC 接口，TUI 层无内联编辑控件
	- **Router TUI 无持久化**：独立 Router 模式的配置修改暂不写回 providers.json

