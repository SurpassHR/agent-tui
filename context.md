# 项目侦察报告 — agent-tui

> 日期: 2026-06-24
> 目标: 深入理解消息模型、渲染流程、Action 通信、PiEvent 事件流和当前功能边界

---

## 1. 消息模型 — `src/message.rs`

### 1.1 ChatRole（角色枚举）

| Variant | 说明 |
|---------|------|
| `User` | 用户发送的消息 |
| `Assistant` | 助手（pi）的回复 |
| `Tool` | 工具执行结果 |
| `System` | 系统状态消息 |
| `Error` | 错误消息 |

### 1.2 ChatMessage（结构化消息单元）

```rust
pub struct ChatMessage {
    pub id: String,          // UUID v4 唯一标识
    pub agent_id: String,    // 所属 agent ID
    pub role: ChatRole,      // 角色
    pub text: String,        // 文本内容
    pub thinking: Option<String>,       // 模型思考过程 ✅ 已有
    pub tool_call: Option<ToolCallInfo>, // 工具调用信息 ✅ 已有
    pub timestamp: u64,                 // 毫秒时间戳
    pub meta: Option<HashMap<String, Value>>, // 额外元数据
}
```

**关键发现**: 已有 `thinking` 和 `tool_call` 字段。`ToolCallInfo` 包含 `tool_name`, `tool_call_id`, `status`(Running/Done/Error), `args`, `result`, `detail_text`。

### 1.3 ContentBlock（内容块枚举，用于序列化）

```rust
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String },
    ToolCall { id: String, name: String, arguments: Value },
    Image { data: String, mime_type: String }, // 预留，TUI 暂不支持
}
```

### 1.4 ToolStatus

```rust
pub enum ToolStatus {
    Running,  // 执行中
    Done,     // 已完成
    Error,    // 出错
}
```

### 1.5 工厂方法

- `ChatMessage::user(agent_id, text)`
- `ChatMessage::assistant(agent_id, text)`
- `ChatMessage::tool(agent_id, tool_call)` — 自动生成文本如 "✓ read"/"✗ read"
- `ChatMessage::system(agent_id, text)`
- `ChatMessage::error(agent_id, text)`

**结论**: 消息模型已有 thinking 和 tool_call。**没有"折叠/展开"相关的状态字段**（如 `collapsed: bool`）。

---

## 2. 主视图渲染逻辑 — `src/components/main_view.rs`

### 2.1 MainView 状态结构

```rust
pub struct MainView {
    pub messages: Vec<ChatMessage>,   // 消息列表
    pub scroll_offset: usize,         // 滚动偏移
    pub input_buffer: String,         // 输入缓冲区
    pub show_thinking: bool,          // 是否显示思考过程
    pub has_focus: bool,              // 输入框是否获得焦点
    pub subsection: MainViewSubsection, // Messages 或 Input
    pub message_cursor: usize,        // 消息光标索引
    pub scroll_mode: ScrollMode,      // TailFollow 或 Pinned
    pub selection: SelectionState,    // 选区状态
}
```

### 2.2 消息渲染（render_message 方法）

每个 `ChatRole` 有不同的渲染样式：

- **User**: `" 你 "` (橙色粗体) → `" ─"` 分隔线 → 逐行文本
- **Assistant**: `" pi "` (橙色粗体) → `" ─"` 分隔线 → thinking 预览(取前3行, 灰色斜体) → 消息文本
- **Tool**: `icon + tool_name` (如 `✓ read` / `▶ read` / `✗ read`)，灰色
- **System**: 灰色斜体文本
- **Error**: `" ⚠ "` + 橙色文本

**ANSI 支持**: **不支持 ANSI 解析**。所有文本直接用 `fg(theme.text)` 纯色渲染。原阶段三的 `ansi::parse_to_lines` 已被替换为纯文本方案。

**选中高亮**: 当 `is_selected == true` 时（消息光标所在 + Messages 子区有焦点），整条消息应用 `bg(theme.highlight_bg)` 背景。

### 2.3 布局 (render 方法)

```
┌─────────────────────────────────────────┐
│ 消息区域 (msg_area)                     │
│  你                                      │
│  ─                                       │
│  hello                                   │
│                                         │
│  pi                                      │
│  ─                                       │
│   [思考] I need to...                    │
│   Hi!                                    │
│                                         │
│  ✓ read                                  │
├─────────────────────────────────────────┤
│ ───── 分割线 (sep_area) ──────────────  │
├─────────────────────────────────────────┤
│ > 输入框 (input_area)                   │
└─────────────────────────────────────────┘
```

### 2.4 滚动

- **TailFollow**（默认）: 始终显示最新消息，`start = all_lines.len() - msg_area_height`
- **Pinned**: 将消息光标所在行固定在视窗上 1/3 处

**局限性**: 没有基于行的手动滚动（PageUp/PageDown），没有无限历史回溯。

---

## 3. 输入框 — MainView 底部输入区

### 3.1 当前状态

```rust
// MainView 字段
pub input_buffer: String,   // 缓存用户输入的字符串
pub subsection: MainViewSubsection, // Input 或 Messages
pub has_focus: bool,
```

### 3.2 渲染 (render_input_inner)

- 无焦点时: `> █ 输入你的问题...`（灰色占位符）
- 有焦点时: `> Hello█`（橙色 >，白色文本，█ 光标）
- 光标字符是硬编码的 `"█"`，不是真正的终端 cursor

### 3.3 键盘处理 (tui.rs 中)

位于 `FocusPanel::MainView + MainViewSubsection::Input` 时：

| 按键 | 行为 |
|------|------|
| `↑` | 切换到 Messages 子区 |
| 字符键 | `input_buffer.push(c)` |
| Backspace | `input_buffer.pop()` |
| Enter | 发送输入 → `Action::UserSubmitInput` + RPC `{"type":"prompt","message":...}` |

### 3.4 缺少的功能

- ❌ **无 `/command` 支持** — 输入框是纯文本，没有命令解析
- ❌ **无 `@file` 补全** — 没有文件路径自动补全逻辑
- ❌ **无历史记录** — 没有 ↑ 键回看历史输入

---

## 4. 中央对话区渲染流程

### 4.1 完整渲染流程

```
tui.rs 主循环 (每 50ms tick)
  │
  ├─ 从 PiRpcClient.event_rx 读取事件
  │   └─ translate_pi_event() → Action
  │       └─ app.handle_action(action)
  │           ├─ Action::MessageAppend → append_to_last_assistant()
  │           ├─ Action::ThinkingAppend → append_to_last_assistant_thinking()
  │           ├─ Action::ThinkingFinalize → finalize_thinking()
  │           ├─ Action::ToolEvent → 创建/更新 Tool ChatMessage
  │           └─ Action::UserSubmitInput → 创建 User ChatMessage
  │
  ├─ 同步 input_buffer 到 main_view
  │
  └─ terminal.try_draw(|f| app.render_tui(f))
       └─ main_view.render(f, area, theme)
            ├─ render_empty() — 无消息时
            └─ render() — 有消息时
                 ├─ 遍历 messages → render_message() 生成 Lines
                 ├─ 滚动裁剪 (TailFollow/Pinned)
                 ├─ selection::apply_selection() 应用选区高亮
                 ├─ 渲染消息 Paragraph
                 ├─ 渲染分割线 ─────
                 └─ 渲染输入框
```

### 4.2 消息数据流

```
pi agent → stdout JSONL → PiRpcClient.event_rx (tokio channel)
         → tui.rs event loop → translate_pi_event() → Action enum
         → app.handle_action() → App.messages HashMap<String, Vec<ChatMessage>>
         → sync_messages_to_main_view() → MainView.messages
         → render() → ratatui Lines → 终端
```

**App.messages** 是按 agent_id 分组的 HashMap: `HashMap<String, Vec<ChatMessage>>`，每次修改后 `sync_messages_to_main_view` 会把当前 active_agent 的消息列表 clone 到 `MainView.messages`。

### 4.3 滚动局限

- 只支持 TailFollow（尾部跟随）和 Pinned（固定到光标）两种模式
- 没有基于行的 PageUp/PageDown 手动滚动
- 没有 scrollbar 指示器

---

## 5. Action 枚举 — `src/action.rs`

### 5.1 完整 Action 列表

| Action Variant | 分类 | 说明 |
|---|---|---|
| `PtyStdout(String)` | PTY（已废弃） | 子进程标准输出 |
| `PtyExit` | PTY（已废弃） | 子进程退出 |
| `UserSubmitInput(String)` | 用户交互 | 用户提交输入 |
| `MessageAppend{agent_id, text}` | RPC 事件 | 流式追加助手文本 |
| `MessageFinalize{agent_id}` | RPC 事件 | 消息定型完成 |
| `ThinkingAppend{agent_id, text}` | RPC 事件 | 流式追加思考过程 |
| `ThinkingFinalize{agent_id, text}` | RPC 事件 | 思考过程完成 |
| `ToolEvent{agent_id, tool_name, tool_call_id, status, args, result, is_error}` | RPC 事件 | 工具执行事件 |
| `AgentStatusChange{agent_id, status}` | RPC 事件 | Agent 状态变更 |
| `RuntimeStateUpdate(AgentRuntimeState)` | RPC 事件 | 运行时状态更新 |
| `AgentMessagesLoaded{agent_id, messages}` | RPC 事件 | 历史消息加载 |
| `AutoRetryStatus{agent_id, text}` | RPC 事件 | 自动重试状态 |
| `TogglePopup` | UI 控制 | 切换浮窗显示 |
| `SwitchSession(String)` | UI 控制 | 切换左侧会话 |
| `ToggleWorkspace(usize)` | 侧边栏 | 展开/折叠工作区 |
| `SelectSession(String)` | 侧边栏 | 选择会话 |
| `SidebarMove(i32)` | 侧边栏 | 焦点移动 |
| `CycleFocusPanel(i32)` | 面板 | 面板焦点循环 |
| `CycleFocusSubsection(i32)` | 面板 | 子区焦点切换 |

### 5.2 关键发现

- **无消息折叠/展开 Action** — 没有类似 `ToggleMessageCollapse` 或 `ToggleThinking` 的 Action
- **无消息编辑/删除 Action** — 不支持编辑或删除已发送的消息
- **无重新生成/重试 Action** — 没有重新生成助手回复的 Action
- **`MessageFinalize` 目前是空操作** — handle_action 不执行任何操作
- PTY Action (`PtyStdout`, `PtyExit`, `Intercepted*`, `UpdateTokenUsage`) 都标记为 `#[deprecated]`，计划迁移到 RPC 事件

---

## 6. 最近 Git Log（最近 20 条）

```
c8b161e fix(provider): fix pi local provider routing and add request diagnostics
e111532 fix(main): 绑定 LogGuard 防止日志丢失
779bf89 feat(logging): add file logging with JSON daily rolling rotation
dcd5a83 feat(tui): 切换模型时通过 RPC 同步到 pi agent
3645a21 fix(sidebar): MODEL 子区搜索/滚动/禁显，统一 footer 高度防跳动
0e1bd0a refactor(sidebar): MODEL 拆分为独立子区，支持 Alt+↑/↓ 切换
b1d9769 fix(provider): 使用动态端口替代硬编码 8001，统一缩进为 2 空格
2746f94 feat(provider): 统一编辑/查看表单、模型管理、自动拉取
6b3b920 fix: 提取 handle_provider_key 并添加 TDD 测试修复 Enter popup 和空列表保护
8cc73fa fix: 重构 provider popup 处理，用 if-else 替代 continue
7a7f1ba fix: 增加 Enter 打开 popup 的 tracing 日志便于调试
0e97cce fix: 空 PROVIDER 时显示 ◆ [+] add provider 高亮选中项
6fc7b92 fix: 对齐 AGENTS 高亮模式，使用 vec![Span] + Style(fg+bg)
37c1e62 fix: 无配置时显示 [+] add provider 提示，provider_cursor 自动钳位
b7da7d5 fix: 修复 PROVIDER 区块三个问题
2622655 feat: 添加 provider 入口提示和 + 键添加 provider
a3c21bf feat: Provider 详情 Popup + 键盘交互
796b427 feat: 实现 Provider 路由系统
b13e52f docs: 新增 Bridge 桥接模式，cc-switch 等本地代理纯透传
e3ad6a2 docs: PROVIDER 区块替换左侧栏 MODEL 区块
```

**近期改动方向**: Provider 路由系统（最近 10 条 commit 主要集中在 provider 侧边栏和模型切换），日志系统，小 bug 修复。**没有消息展示/滚动/折叠相关的改动**。

---

## 7. PiEvent 枚举 — `src/backend/event.rs`

### 7.1 完整 PiEvent 列表

| PiEvent Variant | 触发条件 | 翻译为 Action |
|---|---|---|
| `AgentStart` | agent 开始处理 | `AgentStatusChange(Running)` |
| `AgentEnd{stop_reason, error, will_retry}` | agent 结束处理 | `AgentStatusChange(Idle)` |
| `MessageStart{role}` | 新消息开始 | 未翻译 |
| `MessageUpdate{assistant_event, delta, message}` | 消息内容更新 | 见下表 |
| `MessageEnd{message}` | 消息结束定型 | `MessageFinalize` |
| `ToolExecutionStart{tool_name, tool_call_id, args}` | 工具调用开始 | `ToolEvent(Running)` |
| `ToolExecutionUpdate{tool_name, tool_call_id, partial_result}` | 工具执行进度更新 | 未翻译（忽略） |
| `ToolExecutionEnd{tool_name, tool_call_id, result, is_error}` | 工具调用结束 | `ToolEvent(Done/Error)` |
| `ExtensionError{error}` | 扩展错误 | `AutoRetryStatus` |
| `AutoRetryStart{attempt, max_attempts, delay_ms, error_message}` | 自动重试开始 | 未翻译 |
| `AutoRetryEnd{success, final_error}` | 自动重试结束 | 未翻译 |
| `ExtensionUiRequest{id, method, params}` | 扩展 UI 请求 | 未翻译 |

### 7.2 AssistantEventType（MessageUpdate 的子事件类型）

| 类型 | 翻译为 Action |
|---|---|
| `TextDelta` | `MessageAppend` |
| `ThinkingDelta` | `ThinkingAppend` |
| `ThinkingEnd` | `ThinkingFinalize` |
| `MessageEnd` | `MessageFinalize` |
| `Done` | `MessageFinalize` |
| `TextStart`, `Error`, `MessageStart` | 忽略（None） |

### 7.3 关键发现

- **thinking 事件已完整支持**: `ThinkingDelta` → `ThinkingAppend`, `ThinkingEnd` → `ThinkingFinalize` ✅
- **tool_call 事件已完整支持**: `ToolExecutionStart/Update/End` → `ToolEvent` ✅
- **`ToolExecutionUpdate` 被忽略**: 进度更新事件(`partialResult`)未传递给 UI，这是设计选择还是遗漏待确认
- **`AutoRetryStart/End` 被忽略**: 用户看不到自动重试的进度和结果
- **`MessageStart` 被忽略**: 新消息开始时没有任何 UI 反馈
- **`MessageEnd` 的 `message` 字段未使用**: 包含完整的 `MessageData` (含 ContentBlock 列表)，但 translate_pi_event 只发了 `MessageFinalize` 空信号

### 7.4 PiEvent 序列化约定

- `serde(tag = "type", rename_all = "snake_case")`
- 事件类型使用 snake_case: `agent_start`, `agent_end`, `message_update`, `tool_execution_start`, `extension_error`
- `response` 类型由 `PiRpcClient` 的 pending 机制处理，不进入事件流
- `AssistantMessageEvent` 使用 `serde(rename_all = "snake_case")`: `text_delta`, `thinking_delta`, `thinking_end`, `message_end`

---

## 阶段四亮点汇总

### ✅ 已实现的
- 结构化 ChatMessage 模型（含 thinking + tool_call）
- 流式打字机效果（MessageAppend / ThinkingAppend）
- 思考过程暂存和定型（ThinkingFinalize）
- 工具执行状态追踪（Running/Done/Error）
- RPC 事件 → Action → handle_action 完整链路
- 消息列表 TailFollow + Pinned 两种滚动模式
- 鼠标拖拽选中 + 系统剪贴板写入
- 三条布局：侧边栏(40) | 主视图(flex) | Agent面板(40)
- Provider 路由系统 + 模型切换 RPC 同步
- 文件日志轮转

### ❌ 未实现的
- 消息折叠/展开功能（thinking 可折叠显示前 3 行硬编码）
- 消息手动滚动（PageUp/PageDown）
- `/command` 支持
- `@file` 自动补全
- 消息编辑/删除/重新生成
- ANSI 解析渲染
- Talkback / Popup 消息详情查看（Enter 弹窗展示消息文本，但功能很基础）
- 键盘输入 → PTY 子进程（阶段四补）
- 多 agent 同时运行（阶段五补）
