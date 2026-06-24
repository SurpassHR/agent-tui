# PROVIDER 区块侦察报告

## 1. 项目整体结构

```
src/
├── main.rs              # 入口 + --dry-run 配置输出（含 provider/model 信息）
├── lib.rs               # 公共 API 库根，声明 pub mod provider
├── app.rs               # App 状态机 + TuiState 数据模型（核心）
├── tui.rs               # 事件循环（键盘/鼠标/剪贴板）、RPC 事件处理
├── action.rs            # Action 枚举（全局通信骨架）
├── config.rs            # 配置读取
├── errors.rs            # 错误类型
├── message.rs           # ChatMessage 消息模型
├── selection.rs         # 鼠标拖拽选中逻辑
├── theme.rs             # 主题色定义
├── logging.rs           # 日志配置
├── backend/             # Agent 后端（trait + RPC 实现）
│   ├── mod.rs
│   ├── rpc.rs
│   ├── rpc_client.rs
│   └── event.rs
├── components/          # TUI 组件
│   ├── mod.rs
│   ├── sidebar.rs       # 左侧栏 — PROVIDER 区块在这里渲染
│   ├── main_view.rs     # 中央对话区
│   ├── agent_panel.rs   # 右侧 AGENTS 面板
│   ├── bottom_bar.rs    # 底部状态栏
│   ├── top_bar.rs       # 顶栏
│   └── popup.rs         # 通用 Popup 浮层（Provider 详情弹窗不在这里）
└── provider/            # Provider 路由系统
    ├── mod.rs           # ProviderConfig / ProviderInfo / ModelInfo 数据模型
    └── router.rs        # HTTP 路由器
```

## 2. PROVIDER 区块在哪里渲染？

**文件：`src/components/sidebar.rs`**

PROVIDER 区块在 `Sidebar::render_footer()` 方法中渲染（第 204–291 行）。

### 渲染内容

- 全宽分隔线
- **PROVIDER 标题行** — `PROVIDER (N)` + 状态（`◈ :8001` / `◇ no providers` / `◇ offline`）
- **Provider 列表**（如果有）— 每个显示 `◆/○ name (N models)`，bridge 额外显示 🔗
- **空状态**（providers 为空时）— 显示一行 `◆ [+] add provider`（聚焦时）/ `○ [+] add provider`（未聚焦）
- **提示行**（聚焦时）— `  + add provider  [Space]`
- 分隔线
- **MODEL 区块** — 跟随 active provider 显示模型列表

### 同步机制

在 `app.rs` 的 `sync_components()` 中（第 792 行）：
```rust
self.tui.sidebar.providers.clone_from(&self.tui.providers);
self.tui.sidebar.router_running = self.tui.router_running;
self.tui.sidebar.current_model.clone_from(&self.tui.current_model);
```

## 3. "add provider" 交互在哪里定义？

**`handle_provider_key` 方法 — `src/app.rs` 第 283 行**

### 添加 provider 的键位：`+` 键

```rust
// src/app.rs:357
KeyCode::Char('+') => {
    if !self.selecting_model {
        // 创建一个默认的 ProviderInfo 并 push 到 providers 列表
        let default = crate::provider::ProviderInfo { ... };
        self.providers.push(default);
        // 立即保存到 providers.json
        let path = crate::provider::config_path();
        let cfg = crate::provider::ProviderConfig { ... };
        crate::provider::ProviderConfig::save(&path, &cfg);
    }
    true
}
```

### 界面上的提示不准确

sidebar 渲染的提示文字是 `"+ add provider [Space]"`，但实际代码中 **Space 键在 Provider 子区根本不处理**——Space 只在 Workspace 子区（第 512 行）处理，用于切换工作区展开/折叠。用户按 Space 不会触发任何操作。

实际有效的键是 **`+` 键**。

## 4. 回车键（Enter）事件处理链路

### 事件路由路径

```
crossterm Event::Key(Enter)
    → tui.rs 事件循环（第 460+ 行）
    → 检查 focus 和 sidebar_subsection

    ┌──────────────────────────────────────────────────────────────┐
    │ 条件：focus == Sidebar && subsection == Provider             │
    │ ⇒ app.tui.handle_provider_key(key.code);                   │
    └──────────────────────────────────────────────────────────────┘
    → app.rs handle_provider_key()（第 283 行）

```

### handle_provider_key 中 Enter 的处理（两个分支）

**分支 1 — Popup 已打开时（`provider_popup = Some(idx)`）：**
```rust
KeyCode::Enter => {
    // 激活所选 provider 的第一个模型
    if let Some(p) = self.providers.get(popup_idx) {
        if let Some(first) = p.models.first() {
            self.current_model = first.id.clone();
        }
    }
    self.provider_popup = None;  // 关闭 popup
    true
}
```

**分支 2 — Popup 未打开时（正常导航）：**
```rust
KeyCode::Enter => {
    if self.selecting_model {
        // 在模型列表中：选择模型 + 打开 provider popup
        if let Some(p) = self.providers.get(self.provider_cursor) {
            if let Some(m) = p.models.get(self.model_cursor) {
                self.current_model = m.id.clone();
                self.provider_popup = Some(self.provider_cursor);
            }
        }
    } else if !self.providers.is_empty() {
        // 在 provider 列表中：打开 provider 详情 popup
        self.provider_popup = Some(self.provider_cursor);
    }
    true
}
```

### Enter 不能触发弹窗的场景

**场景 A：providers 列表为空**
- 分支 2 的条件 `!self.providers.is_empty()` 为 false
- Enter 被设为 true（已消费），但什么都不做
- 用户看到的 UI 显示 "add provider" 提示，但 Enter 没有反应

**场景 B：选中的 provider 没有 models**
- 分支 1 中 `if let Some(first) = p.models.first()` 为 None
- `current_model` 不会变，popup 关闭
- 用户看到弹出又立刻消失或没反应

**场景 C：键盘焦点不在 Provider 子区**
- `tui.rs` 第 542-544 行的条件不匹配
- Enter 会被其他子区处理或忽略

## 5. Provider 详情弹窗

### 定义位置

**`src/app.rs` 第 879–917 行**，在 `App::render()` 方法中最后渲染。

这不是用 `popup.rs` 组件，而是直接内联渲染：
```rust
if let Some(idx) = self.tui.provider_popup {
    // ...
    let popup_area = crate::components::popup::centered_rect(65, 55, f.area());
    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(format!(" {} {} ", ...))
        .borders(Borders::ALL)
        .border_style(theme.border);
    // 显示 provider ID、Base URL、模式、模型列表
    // 提示: [Esc] close  [Enter] activate
}
```

### 关闭逻辑

- **Esc 键**：`tui.rs` 第 492 行 — `provider_popup = None`
- **Enter 键**：`handle_provider_key` 第 293 行 — 激活模型 + `provider_popup = None`

### `popup.rs` 通用组件

`src/components/popup.rs` — 是另一个独立的通用浮层（Popup struct），用于消息详情等场景（由 `Action::TogglePopup` 控制），**不是** Provider 详情弹窗。

### `provider_popup` 数据模型

```rust
// src/app.rs:187
pub provider_popup: Option<usize>,
// None = 关闭, Some(idx) = 显示第 idx 个 provider 的详情
```

初始化值：`None`

## 6. 焦点和子区导航

### 面板切换
- `Alt+←/→` — 在 Sidebar / MainView / AgentPanel 三面板间循环

### 子区切换（Sidebar 内）
- `Alt+↑/↓` — 在 ActiveSession ↔ Workspace ↔ Provider 三子区间循环

### Provider 子区内的键位

| 键 | 效果 |
|---|---|
| `↑/↓` | 移动 provider/model 光标 |
| `→` | 进入 model 选择模式（展开模型的子列表） |
| `←` / `Esc` | 退出 model 选择模式 |
| `Enter` | 打开 provider 详情弹窗（或有 model 时激活 model） |
| `+` | 添加默认 provider（立即保存到 providers.json） |
| **Space** | **无效**（UI 提示说可用，但代码中该键只在 Workspace 子区处理） |

## 7. 关键发现总结

| # | 发现 | 文件 | 严重性 |
|---|---|---|---|
| 1 | UI 提示 `[Space]` 但 Space 键在 Provider 子区未被处理 | `sidebar.rs:280` 渲染文本 vs `tui.rs` 事件分发 | ⚠️ 中等 — 误导用户 |
| 2 | providers 为空时按 Enter 无反应（`handle_provider_key` 分支不处理空列表） | `app.rs:344-346` | ⚠️ 中等 — 用户期待弹窗但无响应 |
| 3 | Provider 详情弹窗直接内联在 `App::render()` 中，未复用 `Popup` 组件 | `app.rs:879-917` | ℹ️ 信息 |
| 4 | "add provider" 实际靠 `+` 键，UI 文本写的是 Space | `app.rs:357` vs `sidebar.rs:280` | ⚠️ 中等 — 文档/实现不一致 |
| 5 | 添加 provider 后自动保存到 `providers.json`，但添加入口缺少确认弹窗 | `app.rs:357-375` | ℹ️ 信息 |

[建议升级到其他更高级 agent] — 如果需要修复 Enter 无法弹出配置弹窗的问题，或对齐 UI 提示与实际键位，建议升级到 T2 或 T3 agent 进行实现。

---

## Acceptance Report

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Returned concrete findings with file paths (sidebar.rs, app.rs, tui.rs, popup.rs) and severity levels. All 5 findings have explicit file paths and line numbers."
    }
  ],
  "changedFiles": [
    "/home/hr/Projects/agent-tui/context.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "ls /home/hr/Projects/agent-tui/src/",
      "result": "passed",
      "summary": "Listed src directory structure"
    },
    {
      "command": "grep -i 'provider' src/ -r",
      "result": "passed",
      "summary": "Found all provider-related code locations"
    },
    {
      "command": "grep 'provider_popup' src/ -r",
      "result": "passed",
      "summary": "Found all provider_popup references"
    },
    {
      "command": "grep 'handle_provider_key' src/ -r",
      "result": "passed",
      "summary": "Found handle_provider_key definition and all call sites"
    },
    {
      "command": "grep 'SidebarSubsection' src/ -r",
      "result": "passed",
      "summary": "Found SidebarSubsection enum and usage"
    },
    {
      "command": "grep 'Char.*space' src/ -r",
      "result": "passed",
      "summary": "Found Space key handling code"
    }
  ],
  "validationOutput": [],
  "residualRisks": [
    "Space key UI hint mismatch might confuse users trying to add providers",
    "Empty provider list + Enter does nothing — no feedback to user",
    "Provider popup rendering is inline in app.rs (879-917), not using the reusable popup.rs component — potential duplication of popup logic"
  ],
  "noStagedFiles": true,
  "notes": "context.md 已写入项目根目录。包含完整的 PROVIDER 区块侦察结果，涵盖渲染位置、键盘事件、弹窗逻辑、5 项具体发现及严重性评估。"
}
```
