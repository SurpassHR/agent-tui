# Meta-TUI：智能体编排工作台

[English](README.md) | 简体中文

> 基于 **Rust + Ratatui + Tokio** 的高密度终端（TUI）工作台，
> 通过 RPC 技术编排 CLI AI 智能体。

## 概述

Meta-TUI 通过 **RPC** 连接 `pi`（或其他 CLI 智能体），提供三栏 TUI 界面：

- **左侧栏**：活跃会话 + 工作区树（可展开会话列表）
- **中部**：对话/消息区 + 底部输入框
- **右侧栏**：subagent 列表（AGENTS）+ SKILLS + MCPS + TASKS（从 `~/.pi/agent/` 发现）
- **底栏**：键盘快捷键 + 状态提示 + Context/Token/Cost 信息

文字选中限制在每栏边界内，松手自动复制到系统剪贴板。

## 快速开始

```bash
# 构建
cargo build

# 打印配置信息（不启动 pi）
cargo run -- --dry-run

# 运行 TUI（自动启动 pi RPC 后端）
cargo run -- --tui

# 测试
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## 技术栈

| 组件 | 技术选型 |
|-----------|-----------|
| 语言 | Rust (edition 2024, MSRV 1.85) |
| TUI 框架 | Ratatui 0.30.2 + Crossterm 0.29 |
| 异步运行时 | Tokio |
| RPC | 自定义 JSONL RPC 协议 |
| 错误处理 | color-eyre + thiserror |
| 日志 | tracing + tracing-subscriber |

## 项目结构

```
meta-tui/
├── src/
│   ├── main.rs              # 入口 + --dry-run
│   ├── lib.rs               # 库根
│   ├── action.rs            # Action 枚举
│   ├── app.rs               # App 状态机 + TuiState
│   ├── tui.rs               # TUI 事件循环（RPC + 鼠标/键盘）
│   ├── selection.rs         # 鼠标选中高亮 + 文本收集
│   ├── config.rs            # CLI 参数解析
│   ├── errors.rs            # 错误类型
│   ├── logging.rs           # 日志初始化
│   ├── message.rs           # ChatMessage + ContentBlock 消息模型
│   ├── theme.rs             # Cyan 工业主题
│   ├── backend/
│   │   ├── mod.rs           # AgentBackend trait
│   │   ├── rpc.rs           # PiRpcBackend（进程管理）
│   │   ├── rpc_client.rs    # JSONL RPC 客户端
│   │   └── event.rs         # PiEvent 类型 + 解析
│   ├── provider/
│   │   ├── mod.rs           # ProviderConfig 数据模型 + 持久化
│   │   └── router.rs        # axum HTTP 反向代理
│   └── components/
│       ├── mod.rs           # Component trait (&mut self)
│       ├── sidebar.rs       # 左：会话 + 工作区树 + PROVIDER/MODEL
│       ├── main_view.rs     # 中：对话 + 输入框 + 块折叠
│       ├── markdown.rs      # Markdown → Ratatui Line 渲染器（GFM + 代码块）
│       ├── agent_panel.rs   # 右：AGENTS / SKILLS / MCPS / TASKS
│       ├── bottom_bar.rs    # 底：快捷键 + 状态 + Token
│       ├── top_bar.rs       # 顶部 Tab 栏
│       └── popup.rs         # 弹窗浮层
└── tests/
    └── integration_test.rs
```

## 许可证

MIT
