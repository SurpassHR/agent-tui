# 会话栏交互设计 — 块折叠 / Diff 视图 / 输入补全

> 日期: 2026-06-24 | 关联: Phase 4 RPC 架构的 UI 交互增强

## 1. 背景与动机

当前 agent-tui 的 MainView 渲染为扁平化消息列表，thinking 和 tool_call 仅作简单文本展示，无折叠/展开交互。pi 后端实际发出的 `AssistantMessage.content` 是一个内容块数组（Thinking / Text / ToolCall 可交错），但当前 `ChatMessage` 模型将其拍平为独立字段，丢失了块顺序。

本次设计引入三层交互模型（折叠 → 展开 → 进入），让用户能逐块查看思考过程、工具参数、Diff 变更和 Subagent 对话。

## 2. 数据模型变更

### 2.1 ChatMessage

`ChatMessage` 当前用独立字段 `thinking` / `text` / `tool_call` 拍平内容块。改为直接存储 `content: Vec<ContentBlock>`，旧字段保留为兼容读。

```rust
pub struct ChatMessage {
    pub id: String,
    pub agent_id: String,
    pub role: ChatRole,
    /// 内容块数组（保持 pi AssistantMessage.content 的原始顺序）
    pub content: Vec<ContentBlock>,
    pub timestamp: u64,
    pub meta: Option<HashMap<String, Value>>,
    // 旧字段保留为兼容层（用于 Tool/User/System/Error 消息）
    pub text: String,
    pub thinking: Option<String>,
    pub tool_call: Option<ToolCallInfo>,
}
```

消息类型与字段使用规则：

| ChatRole | content | text / thinking / tool_call |
|----------|---------|-----------------------------|
| Assistant | ✅ 使用（内容块数组） | 兼容只读派生 |
| User      | 空       | text 存用户输入 |
| Tool      | 空       | tool_call 存调用信息 |
| System    | 空       | text 存状态文本 |
| Error     | 空       | text 存错误文本 |

### 2.2 块索引模型

`block_cursor: usize` 是**所有消息中所有可交互块的扁平全局索引**。渲染前遍历所有消息收集可交互块列表 `Vec<BlockRef>`：

```rust
struct BlockRef {
    msg_index: usize,   // 原消息索引
    msg_id: String,     // 消息 UUID
    block_index: usize, // 消息内 content 索引
    kind: BlockKind,    // Thinking | ToolCall
}
```

`block_cursor` 指向此列表索引。消息增删时需重新构建列表，`block_cursor` 钳位到 `0..len()`。

### 2.3 ContentBlock 扩展

```rust
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
        /// 执行结果（ToolExecutionEnd 后回填）
        result: Option<Value>,
        is_error: bool,
    },
    Image { data: String, mime_type: String },
}
```

`ToolCall` 加入 `result` 和 `is_error`：`ToolExecutionEnd` 事件的 `toolCallId` 匹配 content 数组中的 ToolCall 块，回填结果。

### 2.3 MainView 新增字段

```rust
pub struct MainView {
    // ... 现有字段保持不变 ...
    /// 块光标 — 指向当前消息内的第 N 个内容块（仅可交互块）
    pub block_cursor: usize,
    /// 每个块的展开状态，key = "{msg_id}:{block_index}"
    pub block_states: HashMap<String, BlockExpanded>,
    /// 详情/进入视图
    pub entered_view: Option<EnteredView>,
    /// 输入补全 popup
    pub completion_popup: Option<CompletionPopup>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockExpanded {
    Collapsed,  // 默认折叠，只显示摘要行
    Expanded,   // 展开，显示完整内容
}

pub enum EnteredView {
    Diff {
        path: String,
        diff_lines: Vec<DiffLine>,
    },
    FullOutput {
        title: String,
        content: String,
        scroll: usize,
    },
    Subagent {
        agent_id: String,
        messages: Vec<ChatMessage>,
    },
}
```

**Subagent 消息来源**：pi 的 `subagent` 工具执行完毕后，`ToolExecutionEnd` 事件的 `result` 字段包含子代理的完整对话结果。从 `ToolResultMessage` 的 content 中解析子代理消息列表，填充到 `EnteredView::Subagent.messages`。若子代理尚在运行中（`ToolExecutionStart` 已发出但 `ToolExecutionEnd` 未到达），Enter 应无效果或提示等待。

## 3. 事件流改造

### 3.1 AssistantEventType 补全

`src/backend/event.rs` 缺失 5 个流式事件类型：

```rust
pub enum AssistantEventType {
    // 已有的
    TextDelta, TextStart, TextEnd,
    ThinkingDelta, ThinkingEnd,
    MessageStart, MessageEnd,
    Done, Error,
    // 新增
    ThinkingStart,     // 思考块开始
    ToolCallStart,     // 工具调用块开始（含 toolName）
    ToolCallDelta,     // 工具调用参数流式追加
    ToolCallEnd,       // 工具调用块完成（含完整 toolCall）
}
```

### 3.2 新增 Action

```rust
/// 内容块数组更新（来自 MessageUpdate.message.content 快照）
ContentUpdate {
    agent_id: String,
    content: Vec<ContentBlock>,
},
/// 切换块的展开/折叠
ToggleBlock {
    agent_id: String,
    msg_id: String,
    block_index: usize,
},
/// 进入详情视图
EnterBlock {
    agent_id: String,
    msg_id: String,
    block_index: usize,
},
/// 退出详情视图
ExitBlock,
```

### 3.3 translate_pi_event 修改

采用**共存策略**：delta 事件保留用于实时打字机效果，同时使用 `message.content` 快照维护内容块结构。

- **TextDelta/ThinkingDelta** → 继续发出 `MessageAppend`/`ThinkingAppend`（驱动流式打字机效果，用于用户看到的逐字输出）
- **MessageUpdate.message.content** → 每次事件到达时，若 `message` 字段非空，同时发出 `ContentUpdate`（更新内容块数组，用于折叠/展开结构渲染）
- **ToolCallDelta** → 仅在 `ContentUpdate` 的 content 数组中体现（不单独发流式 Action）

即两种 Action 并行：`MessageAppend` 驱动文本显示，`ContentUpdate` 更新底层块数组。`ToolCallStart/ToolCallEnd/ToolCallDelta/ThinkingStart` 事件类型补全后，用于 contentIndex 级别的块边界判断，但主要靠 `message.content` 快照驱动 UI 结构。

```
MessageUpdate { message: Some(data), assistant_event } → {
    // 同时发出两个 Action
    1. 按 delta 类型发 MessageAppend / ThinkingAppend（保留打字机效果）
    2. ContentUpdate { agent_id, content: data.content }
}
```

## 4. 渲染逻辑

### 4.1 渲染流程（MainView::render）

```
MainView::render()
  ├─ entered_view == Some → render_entered()  全面屏 Detail 视图
  │   ├─ Diff        → 统一 diff（+绿/-红，行号对齐）
  │   ├─ FullOutput  → 可滚动纯文本
  │   └─ Subagent    → 独立消息列表（递归渲染）
  │
  └─ entered_view == None → 正常消息列表
      ├─ 遍历 messages
      │   ├─ Assistant → 遍历 msg.content，按 BlockExpanded 渲染
      │   │   ├─ Thinking → Collapsed("▶ [思考] N字") 或 Expanded(显示全文)
      │   │   ├─ Text     → 始终展开（无折叠）
      │   │   └─ ToolCall → Collapsed("▶ ✓ edit path") 或 Expanded(显示 args+result)
      │   ├─ Tool   → "✓ tool_name" 行（折叠/展开同 ContentBlock::ToolCall）
      │   ├─ User   → "你" 标签 + 文本
      │   └─ System/Error → 维持现有样式
      ├─ 高亮当前 block_cursor 所在块
      ├─ 补全 popup（如有）
      └─ 底部输入框
```

### 4.2 块高亮

- `block_cursor` 指向当前可交互块
- 用 `theme.highlight_bg` 背景高亮该行
- `block_cursor` 跨消息移动：Alt+↑/↓ 遍历所有消息的可交互块

### 4.3 可交互块判断

- `ContentBlock::Thinking` — 可折叠，可 Enter（长内容时）
- `ContentBlock::ToolCall` — 可折叠，可 Enter → Diff/FullOutput/Subagent
- `ContentBlock::Text` — **不可交互**，Alt+↑/↓ 跳过

### 4.4 补全 Popup 渲染

```
┌─────────────────────────────────────────┐
│ > 帮我分析 @user█                        │  ← 输入框
├─────────────────────────────────────────┤
│ 📁 src/user/mod.rs            ← 高亮    │  ← Popup
│ 📁 src/user/auth.rs                    │
│ 📁 src/user/profile.rs                 │
└─────────────────────────────────────────┘
```

- 位置：输入框**上方**
- 宽度：最长候选 + 2，最多到 region 宽度
- 高度：最多 8 行
- `/` → 调用 `get_commands` 获取 skills + commands
- `@` → `fd` 模糊搜索项目文件
- 继续打字缩小范围，Tab 选择，Enter 确认填入，Esc 关闭

### 4.5 Diff 视图

```
┌─ Diff: src/user/mod.rs (+2 / -1) ──────┐
│     1 │ pub fn process_user(...)        │
│  -  2 │ let name = user.name.clone();  │  ← 红底
│  +  3 │ let name = user.display_name();│  ← 绿底
│     4 │ save_user(user.id, &name);     │
└─────────────────────────────────────────┘
```

- 统一 diff（单列），不并排
- 使用 `similar` crate 计算文件 diff
- `-` 行红色背景，`+` 行绿色背景
- Esc 退出，回到 Expanded

## 5. 键盘交互

### 5.1 块导航（entered_view == None）

| 按键 | 行为 |
|------|------|
| `Alt+↑` | block_cursor 跳到上一个可交互块 |
| `Alt+↓` | block_cursor 跳到下一个可交互块 |
| `Space` | 当前块 Collapsed ↔ Expanded |
| `Enter` | 进入 EnteredView（有详情时）

### 5.2 进入详情（EnteredView）

| 触发的块类型 | Enter 后行为 |
|-------------|-------------|
| `ToolCall(edit/write)` | → `Diff` 视图 |
| `ToolCall(read/bash)` | → `FullOutput` 视图 |
| `ToolCall(subagent)` | → `Subagent` 递归消息列表 |
| `Thinking`（内容 > 5 行） | → `FullOutput` 视图 |
| `Thinking`（内容 ≤ 5 行） | 无效果（已展开时），或先展开（折叠时） |

### 5.3 EnteredView 内

| 按键 | 行为 |
|------|------|
| `↑/↓` | 滚动 |
| `Esc` | 退出 → Expanded（保留展开态） |

### 5.4 输入框补全 Popup

| 按键 | 行为 |
|------|------|
| `/`（输入框首字符） | 触发命令补全（skills + commands） |
| `@`（输入框中） | 触发文件补全 |
| `Tab` | 选中下一项 |
| `Enter` | 确认选项，填入输入框 |
| `Esc` | 关闭 popup |

### 5.5 焦点切换

当 `entered_view == None`，跨面板焦点切换维持现有 `Alt+←/→` 逻辑不变。

## 6. Bug 修复：Tool Start/End 合并

当前 `handle_action` 中 `ToolEvent(Running)` 和 `ToolEvent(Done)` 各自创建一条 `ChatMessage::tool`，导致一个工具调用占两条消息。

**修复**：
- `ToolEvent(Running)` → 创建 Tool ChatMessage
- `ToolEvent(Done/Error)` → 按 `tool_call_id` 找到相同消息，更新 `status` 和 `result`
- 新增 `update_tool_message(agent_id, tool_call_id, status, result)` 方法

## 7. 文件变更清单

| 文件 | 变更 |
|------|------|
| `src/message.rs` | ChatMessage 加 `content: Vec<ContentBlock>`；ContentBlock::ToolCall 加 `result` / `is_error` |
| `src/backend/event.rs` | AssistantEventType 补全 4 个类型（ThinkingStart, ToolCallStart, ToolCallDelta, ToolCallEnd）；MessageUpdate 解析增加 message.content 快照 |
| `src/action.rs` | 新增 `ContentUpdate`/`ToggleBlock`/`EnterBlock`/`ExitBlock` |
| `src/app.rs` | 新增 `block_states`/`entered_view` 管理；`content_update`/`toggle_block`/`enter_block`/`exit_block` 方法；修复 ToolEvent 合并 |
| `src/tui.rs` | translate_pi_event 改为 `ContentUpdate`；新增 Alt+↑/↓/Space/Enter/Esc 键盘处理 |
| `src/components/main_view.rs` | 新增 `block_cursor`/`block_states`/`entered_view`/`completion_popup`；渲染分支；render_entered / render_diff / render_full_output / render_completion |
| `Cargo.toml` | 新增 `similar` diff 依赖 |
| `src/components/popup.rs` | 补充补全 popup 渲染（如需要复用已有 popup） |

## 8. 测试策略

- `ChatMessage` 序列化/反序列化 roundtrip（含 content 数组）
- `ContentBlock` ToolCall 带 result 的序列化
- `translate_pi_event` — MessageUpdate 携带 message.content 解析为 ContentUpdate
- `handle_action` — ContentUpdate / ToggleBlock / EnterBlock / ExitBlock
- `MainView::render` — 各状态渲染不 panic（TestBackend）
- Diff `similar` 计算 `edit` 参数提取 oldText/newText
- 补全 popup 过滤逻辑
- Tool Start/End 合并：确保同一 toolCallId 只产生一条消息

## 9. 渲染补充说明

- **EnteredView 期间布局**：Diff/FullOutput/Subagent 视图占满 MainView 区域（消息区 + 输入框区），隐藏输入框和分割线。侧边栏和 Agent 面板保持可见。Esc 退出后恢复。
- **ContentBlock::Image**：保留此 variant 但不实现渲染（终端不支持图片显示），折叠/展开对它不适用。

## 10. 局限

- 不实现 ANSI 高亮渲染
- 不实现鼠标点击折叠/展开（纯键盘）
- 不实现多 agent 同时运行（阶段五）
- 不实现消息编辑/删除/重新生成
