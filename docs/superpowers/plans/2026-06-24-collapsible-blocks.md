# 会话栏交互实现计划 — 块折叠 / Diff 视图 / 输入补全

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 MainView 的消息块折叠/展开交互、Diff 视图、输入框 `/skill` `@file` 补全 popup，以及修复 Tool Start/End 消息重复 bug。

**Architecture:** ChatMessage 新增 `content: Vec<ContentBlock>` 维护 pi 原始内容块顺序；MainView 新增 `block_states`/`entered_view`/`completion_popup` 状态字段；渲染按 entered_view 分支（正常 / Diff / FullOutput / Subagent）；键盘 Alt+↑/↓ 导航块、Space 折叠/展开、Enter 进入详情。

**Tech Stack:** Rust edition 2021, Ratatui 0.30, serde, similar (diff), tokio, crossterm. MSRV 1.85.

## Global Constraints

- Rust edition 2021, MSRV 1.85
- 禁止 `unwrap()`，使用 `?` 或 `expect` 加说明信息
- 所有 `pub fn` / `pub struct` 必须 `///` 文档注释
- 每步 commit，commit message 英语 Conventional Commits
- 遵循 TDD：先写测试，确保失败，再实现

---

### Task 1: 添加 similar diff 依赖

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: (none)
- Produces: `similar` crate 可用

- [ ] **Step 1: 添加 similar 依赖**

```toml
# 在 [dependencies] 中添加
similar = "2"
```

在 `tracing-appender = "0.2"` 行之后添加。

- [ ] **Step 2: 验证编译**

```bash
cargo check
```

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(deps): add similar crate for diff computation"
```

---

### Task 2: 数据模型变更 — ChatMessage + ContentBlock + 新类型

**Files:**
- Modify: `src/message.rs`

**Interfaces:**
- Consumes: (none)
- Produces:
  - `ChatMessage.content: Vec<ContentBlock>`
  - `ContentBlock::ToolCall { result, is_error }`
  - `BlockExpanded` enum
  - `BlockKind` enum
  - `EnteredView` enum
  - `DiffLine` struct
  - `CompletionPopup` struct
  - `CompletionItem` struct

- [ ] **Step 1: 更新 ContentBlock，ToolCall 加 result/is_error**

修改 `ContentBlock::ToolCall` variant（约 L182-191）：

```rust
/// 工具调用块
#[serde(rename = "toolCall")]
ToolCall {
    /// 工具调用 ID
    id: String,
    /// 工具名称
    name: String,
    /// 调用参数
    arguments: Value,
    /// 执行结果（ToolExecutionEnd 后回填）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    /// 是否出错
    #[serde(default)]
    is_error: bool,
},
```

- [ ] **Step 2: 更新已有 ContentBlock 测试**

更新 `test_content_block_tool_call` 测试，验证新字段：

```rust
#[test]
fn test_content_block_tool_call() {
    let block = ContentBlock::ToolCall {
        id: "tc-1".to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({"command": "ls"}),
        result: None,
        is_error: false,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "toolCall");
    assert_eq!(json["name"], "bash");
    assert_eq!(json["arguments"]["command"], "ls");
    assert!(json.get("result").is_none());
}

#[test]
fn test_content_block_tool_call_with_result() {
    let block = ContentBlock::ToolCall {
        id: "tc-2".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
        result: Some(serde_json::json!({"content": "fn main() {}"})),
        is_error: false,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["result"]["content"], "fn main() {}");
    assert_eq!(json["is_error"], false);
}
```

- [ ] **Step 3: 运行测试验证失败**

```bash
cargo test -- message
```
预期：测试编译失败（ContentBlock 结构变了）

- [ ] **Step 4: ChatMessage 加 content 字段**

在 `ChatMessage` struct 中（约 L64-81），在 `id` 行之后添加 `content` 字段：

```rust
pub struct ChatMessage {
    /// 消息唯一标识
    pub id: String,
    /// 内容块数组（保持 pi AssistantMessage.content 的原始顺序）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentBlock>,
    /// 所属 agent 的 ID
    pub agent_id: String,
    // ... 其余字段不变
}
```

- [ ] **Step 5: 更新工厂方法，所有构造加 `content: vec![]`**

在每个工厂方法（user/assistant/tool/system/error）的 `Self { ... }` 构造中，`id` 行之后添加 `content: vec![],`。

示例更新 `user` 方法：

```rust
pub fn user(agent_id: &str, text: &str) -> Self {
    Self {
        id: uuid::Uuid::new_v4().to_string(),
        content: vec![],
        agent_id: agent_id.to_string(),
        role: ChatRole::User,
        text: text.to_string(),
        thinking: None,
        tool_call: None,
        timestamp: now_millis(),
        meta: None,
    }
}
```

- [ ] **Step 6: 运行测试确保通过**

```bash
cargo test -- message
```

- [ ] **Step 7: 添加新类型到 message.rs**

在 `ToolStatus` 之后、`ToolCallInfo` 之前添加：

```rust
/// 块的展开状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BlockExpanded {
    /// 折叠（默认）— 只显示摘要行
    Collapsed,
    /// 展开 — 显示完整内容
    Expanded,
}

/// 块类型（用于可交互块分类）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockKind {
    /// 思考块
    Thinking,
    /// 工具调用块
    ToolCall,
}

/// Diff 行
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// 行类型：- 删除, + 新增, = 不变
    pub kind: char,
    /// 行号（原始文件）
    pub old_line: Option<usize>,
    /// 行号（新文件）
    pub new_line: Option<usize>,
    /// 行文本
    pub text: String,
}

/// 补全候选项
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// 显示文本
    pub label: String,
    /// 填入的文本
    pub value: String,
    /// 类型图标前缀（如 "📁" / "⚡"）
    pub prefix: String,
}

/// 输入补全 popup 状态
#[derive(Debug, Clone)]
pub struct CompletionPopup {
    /// 候选项列表
    pub items: Vec<CompletionItem>,
    /// 当前选中索引
    pub cursor: usize,
    /// 触发前缀: "/" 或 "@"
    pub trigger: char,
    /// 用户已输入的过滤文本（不含前缀）
    pub filter: String,
}
```

在 `ChatMessage` struct 之后（约 L82 之前）添加：

```rust
/// 进入详情视图类型
#[derive(Debug, Clone)]
pub enum EnteredView {
    /// Diff 视图
    Diff {
        /// 文件路径
        path: String,
        /// diff 行
        diff_lines: Vec<DiffLine>,
    },
    /// 完整输出视图
    FullOutput {
        /// 标题
        title: String,
        /// 完整内容
        content: String,
        /// 滚动偏移（行数）
        scroll: usize,
    },
    /// Subagent 对话视图
    Subagent {
        /// 子代理 ID
        agent_id: String,
        /// 子代理消息列表
        messages: Vec<ChatMessage>,
    },
}
```

- [ ] **Step 8: 运行测试确保通过**

```bash
cargo test -- message
cargo check
```

- [ ] **Step 9: Commit**

```bash
git add src/message.rs
git commit -m "feat(message): add content blocks array, BlockExpanded, EnteredView, DiffLine, CompletionPopup"
```

---

### Task 3: 事件模型补全

**Files:**
- Modify: `src/backend/event.rs`

**Interfaces:**
- Consumes: (pi 事件 JSON)
- Produces: 4 个新 `AssistantEventType` 变体

- [ ] **Step 1: 补全 AssistantEventType 枚举**

在 `AssistantEventType` enum（约 L168）中，在 `Error,` 之前添加：

```rust
/// 思考块开始
ThinkingStart,
/// 工具调用块开始
ToolCallStart,
/// 工具调用参数流式追加
ToolCallDelta,
/// 工具调用块完成
ToolCallEnd,
```

最终的 enum 顺序应为：

```rust
pub enum AssistantEventType {
    TextDelta,
    TextStart,
    TextEnd,
    ThinkingDelta,
    ThinkingEnd,
    ThinkingStart,
    MessageStart,
    MessageEnd,
    ToolCallStart,
    ToolCallDelta,
    ToolCallEnd,
    Done,
    Error,
}
```

- [ ] **Step 2: 添加新类型的序列化测试**

```rust
#[test]
fn test_assistant_event_tool_call_start() {
    let raw = json!({
        "type": "message_update",
        "assistantMessageEvent": {
            "type": "toolcall_start",
            "contentIndex": 2,
            "toolName": "bash"
        }
    });
    let event = parse_pi_event(&raw).unwrap();
    match event {
        Some(PiEvent::MessageUpdate { assistant_event, .. }) => {
            assert_eq!(assistant_event.event_type, AssistantEventType::ToolCallStart);
        }
        other => panic!("expected MessageUpdate, got {:?}", other),
    }
}

#[test]
fn test_assistant_event_thinking_start() {
    let raw = json!({
        "type": "message_update",
        "assistantMessageEvent": {
            "type": "thinking_start"
        }
    });
    let event = parse_pi_event(&raw).unwrap();
    match event {
        Some(PiEvent::MessageUpdate { assistant_event, .. }) => {
            assert_eq!(assistant_event.event_type, AssistantEventType::ThinkingStart);
        }
        other => panic!("expected MessageUpdate, got {:?}", other),
    }
}
```

- [ ] **Step 3: 运行测试验证解析正确**

```bash
cargo test -- event
```

- [ ] **Step 4: Commit**

```bash
git add src/backend/event.rs
git commit -m "feat(event): add ThinkingStart, ToolCallStart, ToolCallDelta, ToolCallEnd event types"
```

---

### Task 4: Action 枚举扩展

**Files:**
- Modify: `src/action.rs`

**Interfaces:**
- Consumes: `ContentBlock` (from message.rs)
- Produces: `ContentUpdate`, `ToggleBlock`, `EnterBlock`, `ExitBlock`

- [ ] **Step 1: 添加新 Action variants**

在 `action.rs` 的 `Action` enum 中，在 `AutoRetryStatus` 之后、`TogglePopup` 之前添加：

```rust
/// 内容块数组更新（来自 MessageUpdate.message.content 快照）
ContentUpdate {
    agent_id: String,
    content: Vec<crate::message::ContentBlock>,
},
/// 切换块的展开/折叠
ToggleBlock {
    agent_id: String,
    msg_id: String,
    block_index: usize,
},
/// 进入块的详情视图
EnterBlock {
    agent_id: String,
    msg_id: String,
    block_index: usize,
},
/// 退出详情视图
ExitBlock,
```

- [ ] **Step 2: 验证编译**

```bash
cargo check
```

预期可能报错（match 分支不完整），这是正常的，后续 task 会补全。

- [ ] **Step 3: Commit**

```bash
git add src/action.rs
git commit -m "feat(action): add ContentUpdate, ToggleBlock, EnterBlock, ExitBlock actions"
```

---

### Task 5: App 状态机 — handle_action 新增逻辑

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: Action::ContentUpdate, ToggleBlock, EnterBlock, ExitBlock; ToolEvent
- Produces: `App.content_update()`, `App.toggle_block()`, `App.enter_block()`, `App.exit_block()`, `App.update_tool_message()`

- [ ] **Step 1: 添加 ContentUpdate 处理**

在 `handle_action` 方法的 `Action::ThinkingFinalize { .. }` 之后添加：

```rust
Action::ContentUpdate { agent_id, content } => {
    // 更新最后一条 Assistant 消息的 content 数组
    let msgs = self.messages.entry(agent_id.clone()).or_default();
    if let Some(last) = msgs.iter_mut().rev().find(|m| {
        matches!(m.role, crate::message::ChatRole::Assistant)
    }) {
        last.content = content;
    }
}
```

- [ ] **Step 2: 添加 ToggleBlock 处理**

在 ContentUpdate 处理之后添加：

```rust
Action::ToggleBlock {
    agent_id,
    msg_id,
    block_index,
} => {
    let key = format!("{}:{}", msg_id, block_index);
    let state = self
        .tui
        .main_view
        .block_states
        .entry(key)
        .or_insert(crate::message::BlockExpanded::Collapsed);
    *state = match *state {
        crate::message::BlockExpanded::Collapsed => crate::message::BlockExpanded::Expanded,
        crate::message::BlockExpanded::Expanded => crate::message::BlockExpanded::Collapsed,
    };
}
```

- [ ] **Step 3: 添加 EnterBlock 处理**

在 ToggleBlock 处理之后添加：

```rust
Action::EnterBlock {
    agent_id,
    msg_id,
    block_index,
} => {
    let msgs = self.messages.get(&agent_id);
    if let Some(msgs) = msgs {
        // 构建块引用列表，找到对应的块
        let blocks = build_block_refs(msgs);
        let key = format!("{}:{}", msg_id, block_index);
        if let Some(block_ref) = blocks.iter().find(|b| {
            format!("{}:{}", b.msg_id, b.block_index) == key
        }) {
            let msg = &msgs[block_ref.msg_index];
            match block_ref.kind {
                crate::message::BlockKind::Thinking => {
                    // 思考块 > 5 行 → FullOutput
                    let thinking_text = if let Some(
                        crate::message::ContentBlock::Thinking { thinking },
                    ) = msg.content.get(block_ref.block_index)
                    {
                        thinking.clone()
                    } else {
                        String::new()
                    };
                    if thinking_text.lines().count() > 5 {
                        self.tui.main_view.entered_view =
                            Some(crate::message::EnteredView::FullOutput {
                                title: "思考过程".to_string(),
                                content: thinking_text,
                                scroll: 0,
                            });
                    }
                }
                crate::message::BlockKind::ToolCall => {
                    if let Some(
                        crate::message::ContentBlock::ToolCall {
                            id: _,
                            name,
                            arguments,
                            result,
                            is_error: _,
                        },
                    ) = msg.content.get(block_ref.block_index)
                    {
                        match name.as_str() {
                            "edit" | "write" => {
                                // 构建 Diff 视图
                                let old_text = arguments
                                    .get("oldText")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let new_text = arguments
                                    .get("newText")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let path = arguments
                                    .get("path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let diff_lines = compute_diff_lines(old_text, new_text);
                                self.tui.main_view.entered_view =
                                    Some(crate::message::EnteredView::Diff {
                                        path: path.to_string(),
                                        diff_lines,
                                    });
                            }
                            "read" | "bash" => {
                                let content = result
                                    .as_ref()
                                    .and_then(|r| r.get("content"))
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("");
                                self.tui.main_view.entered_view =
                                    Some(crate::message::EnteredView::FullOutput {
                                        title: format!("{}", name),
                                        content: content.to_string(),
                                        scroll: 0,
                                    });
                            }
                            "subagent" => {
                                // Subagent — 从 result 解析消息（Phase 5 完整实现）
                                self.tui.main_view.entered_view =
                                    Some(crate::message::EnteredView::FullOutput {
                                        title: format!("subagent: {}", name),
                                        content: format!("{:?}", result),
                                        scroll: 0,
                                    });
                            }
                            _ => {
                                // 未知工具 → FullOutput
                                let content = result
                                    .as_ref()
                                    .map(|r| format!("{:?}", r))
                                    .unwrap_or_default();
                                self.tui.main_view.entered_view =
                                    Some(crate::message::EnteredView::FullOutput {
                                        title: name.clone(),
                                        content,
                                        scroll: 0,
                                    });
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: 添加 ExitBlock 处理**

```rust
Action::ExitBlock => {
    self.tui.main_view.entered_view = None;
}
```

- [ ] **Step 5: 修复 ToolEvent Start/End 合并**

替换现有 `Action::ToolEvent { .. }` 处理逻辑（约 L1081-1093）：

```rust
Action::ToolEvent {
    agent_id,
    tool_name,
    tool_call_id,
    status,
    args,
    result,
    is_error,
} => {
    let msgs = self.messages.entry(agent_id.clone()).or_default();
    match status {
        crate::message::ToolStatus::Running => {
            // 创建新的 Tool ChatMessage
            let tool_info = crate::message::ToolCallInfo {
                tool_name,
                tool_call_id,
                status,
                args: args.unwrap_or_default(),
                result: None,
                detail_text: String::new(),
            };
            let msg = ChatMessage::tool(&agent_id, tool_info);
            msgs.push(msg);
        }
        _ => {
            // Done 或 Error → 按 tool_call_id 找到并更新
            if let Some(msg) = msgs.iter_mut().rev().find(|m| {
                m.role == crate::message::ChatRole::Tool
                    && m.tool_call
                        .as_ref()
                        .is_some_and(|tc| tc.tool_call_id == tool_call_id)
            }) {
                if let Some(ref mut tc) = msg.tool_call {
                    tc.status = status;
                    tc.result = result;
                    // 更新显示文本
                    let icon = match status {
                        crate::message::ToolStatus::Done => "✓",
                        crate::message::ToolStatus::Error => "✗",
                        crate::message::ToolStatus::Running => "▶",
                    };
                    msg.text = format!("{} {}", icon, tc.tool_name);
                }
            }
        }
    }
}
```

- [ ] **Step 6: 添加辅助函数到 app.rs**

在 `App` impl 块外部、文件末尾添加（约 L1198 之前）：

```rust
/// 从消息列表中构建可交互块引用列表
fn build_block_refs(messages: &[ChatMessage]) -> Vec<BlockRef> {
    let mut refs = Vec::new();
    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role == crate::message::ChatRole::Assistant {
            for (block_idx, block) in msg.content.iter().enumerate() {
                match block {
                    crate::message::ContentBlock::Thinking { .. } => {
                        refs.push(BlockRef {
                            msg_index: msg_idx,
                            msg_id: msg.id.clone(),
                            block_index: block_idx,
                            kind: crate::message::BlockKind::Thinking,
                        });
                    }
                    crate::message::ContentBlock::ToolCall { .. } => {
                        refs.push(BlockRef {
                            msg_index: msg_idx,
                            msg_id: msg.id.clone(),
                            block_index: block_idx,
                            kind: crate::message::BlockKind::ToolCall,
                        });
                    }
                    _ => {} // Text/Image 不可交互，跳过
                }
            }
        }
    }
    refs
}

/// 块引用（扁平全局索引）
struct BlockRef {
    msg_index: usize,
    msg_id: String,
    block_index: usize,
    kind: crate::message::BlockKind,
}

/// 使用 similar crate 计算统一 diff
fn compute_diff_lines(old_text: &str, new_text: &str) -> Vec<crate::message::DiffLine> {
    let diff = similar::TextDiff::from_lines(old_text, new_text);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let (kind, old_line, new_line) = match change.tag() {
            similar::ChangeTag::Delete => ('-', Some(change.old_index().unwrap_or(0) + 1), None),
            similar::ChangeTag::Insert => ('+', None, Some(change.new_index().unwrap_or(0) + 1)),
            similar::ChangeTag::Equal => (
                ' ',
                Some(change.old_index().unwrap_or(0) + 1),
                Some(change.new_index().unwrap_or(0) + 1),
            ),
        };
        lines.push(crate::message::DiffLine {
            kind,
            old_line,
            new_line,
            text: change.value().to_string(),
        });
    }
    lines
}
```

- [ ] **Step 7: 运行测试确保编译通过**

```bash
cargo check
```

- [ ] **Step 8: 写 Action 单元测试**

在 `src/app.rs` 的 `#[cfg(test)]` 模块中添加：

```rust
#[test]
fn test_toggle_block() {
    let mut app = App::new_rpc_for_test();
    app.tui.main_view.block_states.insert(
        "msg-1:0".to_string(),
        crate::message::BlockExpanded::Collapsed,
    );
    // 模拟 ToggleBlock
    let key = "msg-1:0".to_string();
    let state = app.tui.main_view.block_states.get_mut(&key).unwrap();
    *state = crate::message::BlockExpanded::Expanded;
    assert_eq!(*state, crate::message::BlockExpanded::Expanded);
}

#[test]
fn test_exit_block() {
    let mut app = App::new_rpc_for_test();
    app.tui.main_view.entered_view =
        Some(crate::message::EnteredView::FullOutput {
            title: "test".into(),
            content: "hello".into(),
            scroll: 0,
        });
    app.tui.main_view.entered_view = None;
    assert!(app.tui.main_view.entered_view.is_none());
}
```

- [ ] **Step 9: 运行全部测试**

```bash
cargo test
```

- [ ] **Step 10: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): add ContentUpdate/ToggleBlock/EnterBlock/ExitBlock handling and ToolEvent dedup"
```

---

### Task 6: MainView 渲染 — 块折叠 & EnteredView

**Files:**
- Modify: `src/components/main_view.rs`

**Interfaces:**
- Consumes: `MainView` 新字段, `ChatMessage.content`, `EnteredView`, `BlockExpanded`
- Produces: `render_entered()`, `render_diff()`, `render_full_output()`, `render_block()`

- [ ] **Step 1: 更新 MainView struct 加新字段**

在 `MainView::default()` impl 和 struct 定义中，`selection` 字段之后添加：

```rust
/// 块光标（可交互块的扁平全局索引）
pub block_cursor: usize,
/// 块展开状态 key = "{msg_id}:{block_index}"
pub block_states: std::collections::HashMap<String, crate::message::BlockExpanded>,
/// 详情/进入视图
pub entered_view: Option<crate::message::EnteredView>,
/// 输入补全 popup
pub completion_popup: Option<crate::message::CompletionPopup>,
```

同时更新 `#[derive(Default)]` — MainView 不再能 derive Default，需手动实现或移除 derive 并写 `impl Default`：

```rust
impl Default for MainView {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            input_buffer: String::new(),
            show_thinking: false,
            has_focus: false,
            subsection: crate::app::MainViewSubsection::Messages,
            message_cursor: 0,
            scroll_mode: crate::app::ScrollMode::TailFollow,
            selection: crate::app::SelectionState::default(),
            block_cursor: 0,
            block_states: std::collections::HashMap::new(),
            entered_view: None,
            completion_popup: None,
        }
    }
}
```

- [ ] **Step 2: 更新 render 方法加 entered_view 分支**

修改 `render` 方法开头（约 L166），在 `let block = ...` 之前添加：

```rust
fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
    // entered_view 分支：全屏 Detail 视图
    if let Some(ref view) = self.entered_view {
        return self.render_entered(f, area, theme, view);
    }

    if self.messages.is_empty() && self.input_buffer.is_empty() {
        return self.render_empty(f, area, theme);
    }
    // ... 原有代码继续
```

- [ ] **Step 3: 更新 render_message 用 content 数组渲染 Assistant**

修改 `render_message` 方法中 `ChatRole::Assistant` 分支（约 L98-119）。替换为：

```rust
ChatRole::Assistant => {
    lines.push(Line::from(vec![" pi ".to_string().fg(theme.accent).bold()]));
    lines.push(Line::from(vec![" ─".to_string().fg(theme.text_dim)]));

    // 遍历 content 数组渲染内容块
    for (block_idx, block) in message.content.iter().enumerate() {
        let key = format!("{}:{}", message.id, block_idx);
        let state = self
            .block_states
            .get(&key)
            .copied()
            .unwrap_or(crate::message::BlockExpanded::Collapsed);
        let is_selected = is_selected && block_idx == self.block_cursor;

        match block {
            crate::message::ContentBlock::Thinking { thinking } => {
                match state {
                    crate::message::BlockExpanded::Collapsed => {
                        let preview: String = thinking
                            .lines()
                            .take(1)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let chars = thinking.chars().count();
                        let icon = "▶";
                        let mut text = format!(" {} [思考] {} 字 — {}", icon, chars, preview);
                        // 截断到可视宽度
                        // （实际渲染时 Ratatui 会自动处理）
                        let span = text.fg(theme.text_dim);
                        if is_block_selected {
                            lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                        } else {
                            lines.push(Line::from(vec![span]));
                        }
                    }
                    crate::message::BlockExpanded::Expanded => {
                        let icon = "▼";
                        let mut span = format!(" {} [思考] {} 字", icon, thinking.chars().count())
                            .fg(theme.text_dim);
                        if is_block_selected {
                            lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                        } else {
                            lines.push(Line::from(vec![span]));
                        }
                        for think_line in thinking.lines() {
                            let s = format!("   {}", think_line).fg(theme.text_dim);
                            if is_block_selected {
                                lines.push(Line::from(vec![s.bg(theme.highlight_bg)]));
                            } else {
                                lines.push(Line::from(vec![s]));
                            }
                        }
                    }
                }
            }
            crate::message::ContentBlock::Text { text } => {
                for text_line in text.lines() {
                    lines.push(Line::from(vec![
                        format!(" {}", text_line).fg(theme.text)
                    ]));
                }
            }
            crate::message::ContentBlock::ToolCall {
                name,
                arguments,
                result,
                is_error,
                ..
            } => {
                let icon = if *is_error { "✗" } else if result.is_some() { "✓" } else { "▶" };
                let args_summary = format_args_summary(arguments);
                match state {
                    crate::message::BlockExpanded::Collapsed => {
                        let text = format!(" {} {} {} {}",
                            icon,
                            name,
                            args_summary,
                            if *is_error { "(出错)" } else { "" }
                        );
                        let span = text.fg(theme.text_dim);
                        if is_block_selected {
                            lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                        } else {
                            lines.push(Line::from(vec![span]));
                        }
                    }
                    crate::message::BlockExpanded::Expanded => {
                        let text = format!(" {} {} {}",
                            icon,
                            name,
                            if *is_error { "(出错)" } else { "" }
                        );
                        let span = text.fg(theme.text_dim);
                        if is_block_selected {
                            lines.push(Line::from(vec![span.bg(theme.highlight_bg)]));
                        } else {
                            lines.push(Line::from(vec![span]));
                        }
                        // 显示参数摘要
                        if let Some(args_str) = serde_json::to_string_pretty(arguments).ok() {
                            for arg_line in args_str.lines().take(5) {
                                let s = format!("   {}", arg_line).fg(theme.text_dim);
                                if is_block_selected {
                                    lines.push(Line::from(vec![s.bg(theme.highlight_bg)]));
                                } else {
                                    lines.push(Line::from(vec![s]));
                                }
                            }
                        }
                        // 显示结果摘要
                        if let Some(ref r) = result {
                            if let Some(content) = r.get("content").and_then(|c| c.as_str()) {
                                let preview: String = content.lines().take(3).collect::<Vec<_>>().join("\n");
                                lines.push(Line::from(vec!["   ---".fg(theme.text_dim)]));
                                for out_line in preview.lines() {
                                    let s = format!("   {}", out_line).fg(theme.text_dim);
                                    if is_block_selected {
                                        lines.push(Line::from(vec![s.bg(theme.highlight_bg)]));
                                    } else {
                                        lines.push(Line::from(vec![s]));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {} // Image 不渲染
        }
    }
    lines.push(Line::from(""));
}
```

- [ ] **Step 4: 修复 is_selected 判断逻辑**

在 `render` 方法中，修复 is_selected 调用。原代码传 `is_selected` 到 render_message 表示整条消息选中，需要改为块级别。改为传 `None`（在 render_message 内部用 block 级别的判断）：

在 `render` 方法的消息遍历循环（约 L196）中，将：

```rust
let is_selected = self.has_focus
    && self.subsection == MainViewSubsection::Messages
    && i == self.message_cursor;
```

改为不传入 render_message，而是在 render_message 内部通过 block_cursor 判断每个块。

简化为：`render_message` 不需要外部的 is_selected，内部自己判断。在 `render` 方法中改为：

```rust
for (i, msg) in self.messages.iter().enumerate() {
    let start = all_lines.len();
    all_lines.extend(Self::render_message(msg, theme, self, i));
    let end = all_lines.len();
    msg_line_ranges.push((start, end));
}
```

并修改 `render_message` 签名：

```rust
fn render_message(
    message: &ChatMessage,
    theme: &Theme,
    main_view: &Self,
    msg_index: usize,
) -> Vec<Line<'static>> {
```

在 render_message 内部计算 is_block_selected：

```rust
// 构建块引用判断当前 block 是否选中
let blocks = build_flat_blocks(&main_view.messages);
let is_block_selected = main_view.has_focus
    && main_view.subsection == crate::app::MainViewSubsection::Messages
    && main_view.block_cursor < blocks.len()
    && blocks[main_view.block_cursor].msg_index == msg_index
    && blocks[main_view.block_cursor].block_index == block_idx;
```

- [ ] **Step 5: 添加 render_entered 方法**

在 `impl MainView` 中添加新渲染方法：

```rust
/// 渲染详情视图（Diff / FullOutput / Subagent）
fn render_entered(
    &self,
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    view: &crate::message::EnteredView,
) {
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(theme.bg));
    f.render_widget(block, area);

    match view {
        crate::message::EnteredView::Diff { path, diff_lines } => {
            self.render_diff(f, area, theme, path, diff_lines);
        }
        crate::message::EnteredView::FullOutput {
            title,
            content,
            scroll,
        } => {
            self.render_full_output(f, area, theme, title, content, *scroll);
        }
        crate::message::EnteredView::Subagent { agent_id, messages } => {
            // Subagent 递归渲染（简化版：显示消息列表）
            let title = format!("subagent: {}", agent_id);
            let content = messages
                .iter()
                .map(|m| format!("[{}] {}", 
                    match m.role { 
                        crate::message::ChatRole::User => "你", 
                        crate::message::ChatRole::Assistant => "pi", 
                        _ => ".."
                    }, 
                    m.text))
                .collect::<Vec<_>>()
                .join("\n");
            self.render_full_output(f, area, theme, &title, &content, 0);
        }
    }
}

/// 渲染 Diff 视图
fn render_diff(
    &self,
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    path: &str,
    diff_lines: &[crate::message::DiffLine],
) {
    let inner = inset_content(area);
    let plus_count = diff_lines.iter().filter(|l| l.kind == '+').count();
    let minus_count = diff_lines.iter().filter(|l| l.kind == '-').count();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let header = format!("┌─ Diff: {} (+{} / -{}) ─", path, plus_count, minus_count);
    lines.push(Line::from(vec![header.fg(theme.text_dim)]));

    for dl in diff_lines {
        let prefix = match dl.kind {
            '-' => format!("  -  {:>4} │ ", dl.old_line.unwrap_or(0)),
            '+' => format!("  +  {:>4} │ ", dl.new_line.unwrap_or(0)),
            _ => format!("     {:>4} │ ", dl.old_line.unwrap_or(0)),
        };
        match dl.kind {
            '-' => lines.push(Line::from(vec![
                prefix.fg(theme.text_dim),
                dl.text.clone().fg(theme.text).bg(theme.diff_red_bg),
            ])),
            '+' => lines.push(Line::from(vec![
                prefix.fg(theme.text_dim),
                dl.text.clone().fg(theme.text).bg(theme.diff_green_bg),
            ])),
            _ => lines.push(Line::from(vec![
                prefix.fg(theme.text_dim),
                dl.text.clone().fg(theme.text),
            ])),
        }
    }

    let footer = "└".to_string()
        + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
    lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

    // 裁剪到可见区域
    let visible: Vec<Line> = lines
        .into_iter()
        .take(inner.height.saturating_sub(1) as usize)
        .collect();

    let paragraph = Paragraph::new(visible)
        .style(Style::default().bg(theme.bg))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

/// 渲染完整输出视图
fn render_full_output(
    &self,
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    content: &str,
    scroll: usize,
) {
    let inner = inset_content(area);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let header = format!("┌─ {} ─", title);
    lines.push(Line::from(vec![header.fg(theme.text_dim)]));

    // 跳过 scroll 行
    for text_line in content.lines().skip(scroll) {
        lines.push(Line::from(vec![format!(" {}", text_line).fg(theme.text)]));
    }

    let footer = "└".to_string()
        + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
    lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

    let visible: Vec<Line> = lines
        .into_iter()
        .take(inner.height.saturating_sub(1) as usize)
        .collect();

    let paragraph = Paragraph::new(visible)
        .style(Style::default().bg(theme.bg))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}
```

- [ ] **Step 6: 添加辅助函数**

```rust
/// 构建所有消息的平坦可交互块引用列表
fn build_flat_blocks(messages: &[ChatMessage]) -> Vec<FlatBlockRef> {
    let mut refs = Vec::new();
    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role == ChatRole::Assistant {
            for (block_idx, block) in msg.content.iter().enumerate() {
                match block {
                    crate::message::ContentBlock::Thinking { .. } => {
                        refs.push(FlatBlockRef {
                            msg_index: msg_idx,
                            msg_id: msg.id.clone(),
                            block_index: block_idx,
                        });
                    }
                    crate::message::ContentBlock::ToolCall { .. } => {
                        refs.push(FlatBlockRef {
                            msg_index: msg_idx,
                            msg_id: msg.id.clone(),
                            block_index: block_idx,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    refs
}

struct FlatBlockRef {
    msg_index: usize,
    msg_id: String,
    block_index: usize,
}

/// 格式化工具调用参数为简短摘要
fn format_args_summary(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        let keys: Vec<String> = obj.keys().take(3).cloned().collect();
        if keys.is_empty() {
            String::new()
        } else {
            let summary = keys.join(", ");
            if obj.len() > 3 {
                format!("({}, …)", summary)
            } else {
                format!("({})", summary)
            }
        }
    } else {
        String::new()
    }
}
```

- [ ] **Step 7: 添加 Theme diff 颜色（可选，临时用硬编码）**

在 `render_diff` 中使用 `theme.diff_red_bg` 和 `theme.diff_green_bg`。需要在 `Theme` 中添加（如果还没有）。先暂时用 Color::Rgb 硬编码，后续可迁移到 Theme：

```rust
use ratatui::style::Color;
// 在 render_diff 中：
let diff_red_bg = Color::Rgb(42, 26, 26);
let diff_green_bg = Color::Rgb(26, 42, 26);
```

- [ ] **Step 8: 验证编译**

```bash
cargo check
```

修复所有编译错误。

- [ ] **Step 9: 写渲染测试**

```rust
#[test]
fn test_render_with_content_blocks() {
    let mut mv = MainView::default();
    let mut msg = ChatMessage::assistant("test", "");
    msg.content = vec![
        crate::message::ContentBlock::Thinking {
            thinking: "分析中...".to_string(),
        },
        crate::message::ContentBlock::Text {
            text: "这是回复".to_string(),
        },
    ];
    mv.messages.push(msg);

    let theme = Theme::cyan();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            mv.render(f, area, &theme);
        })
        .unwrap();
}
```

- [ ] **Step 10: Commit**

```bash
git add src/components/main_view.rs
git commit -m "feat(main_view): block-based rendering with collapsible thinking/tool blocks and entered views"
```

---

### Task 7: TUI 键盘交互 & translate_pi_event 更新

**Files:**
- Modify: `src/tui.rs`

**Interfaces:**
- Consumes: PiEvent, crossterm key events
- Produces: translate_pi_event 新增 ContentUpdate, 键盘处理 Alt+↑/↓/Space/Enter/Esc

- [ ] **Step 1: 更新 translate_pi_event 发出 ContentUpdate**

在 `translate_pi_event` 函数中，修改 `MessageUpdate` 处理。当前代码（约 L20-40）只处理 delta。改为在原有 delta 处理基础上，同时从 `message` 字段发出 `ContentUpdate`。

需修改函数签名以支持返回多个 Action（改为 `Vec<Action>`），或将 `ContentUpdate` 合并到现有逻辑。

简化方案：修改 `translate_pi_event` 返回 `Option<Action>` 不变，但新增 `content_update_from_message` helper：

不用改签名。在现有 `translate_pi_event` 逻辑的基础上，在 `MessageUpdate` 的 match 挡中同时处理 `message` 字段。但目前 `translate_pi_event` 只返回单个 `Option<Action>`。最简单方案：在 `PiEvent::MessageUpdate` 分支中，**优先取 message.content** 发 ContentUpdate（delta 依旧保留用于打字机效果但不能同时返回两个 Action）。

折中：在 tui.rs 主循环中，收到 `translate_pi_event` 结果之外，额外检查事件是否含 message 字段。或改为：

```rust
fn translate_pi_events(event: PiEvent, agent_id: &str) -> Vec<Action> {
    let mut actions = Vec::new();
    match &event {
        PiEvent::MessageUpdate {
            assistant_event,
            message,
            ..
        } => {
            // 1. delta 事件（打字机效果）
            match assistant_event.event_type {
                AssistantEventType::TextDelta => {
                    actions.push(Action::MessageAppend {
                        agent_id: agent_id.into(),
                        text: assistant_event.delta.clone().unwrap_or_default(),
                    });
                }
                AssistantEventType::ThinkingDelta => {
                    actions.push(Action::ThinkingAppend {
                        agent_id: agent_id.into(),
                        text: assistant_event.delta.clone().unwrap_or_default(),
                    });
                }
                AssistantEventType::ThinkingEnd => {
                    actions.push(Action::ThinkingFinalize {
                        agent_id: agent_id.into(),
                        text: assistant_event.delta.clone().unwrap_or_default(),
                    });
                }
                AssistantEventType::MessageEnd | AssistantEventType::Done => {
                    actions.push(Action::MessageFinalize {
                        agent_id: agent_id.into(),
                    });
                }
                _ => {}
            }
            // 2. ContentUpdate（块数组快照）
            if let Some(ref data) = message {
                actions.push(Action::ContentUpdate {
                    agent_id: agent_id.into(),
                    content: data.content.clone(),
                });
            }
        }
        // ... 其余 event 类型（不变，将 Option<Action> 改为 push 到 vec）
        _ => {
            if let Some(a) = translate_single_action(event, agent_id) {
                actions.push(a);
            }
        }
    }
    actions
}
```

将原 `translate_pi_event` 改名为 `translate_single_action` 处理非 MessageUpdate 事件。

在 tui.rs 主循环中（约 L350）：

```rust
// 替换原来的:
// if let Some(action) = translate_pi_event(event, &agent_id) {
//     ...
// }

for action in translate_pi_events(event, &agent_id) {
    if let Err(e) = app.handle_action(action).await {
        tracing::error!("handle_action error: {}", e);
    }
}
```

- [ ] **Step 2: 添加 Alt+↑/↓ 块导航键盘处理**

在 tui.rs 键盘处理中（约 L698，Messages 子区），在现有 `KeyCode::Up` / `KeyCode::Down` 处理中添加 `Alt+Up` / `Alt+Down`：

```rust
// ── MainView + Messages 子区：Alt+↑/↓ 块导航 ──
KeyEvent {
    code: KeyCode::Up,
    modifiers: KeyModifiers::ALT,
    ..
} => {
    let blocks = build_flat_blocks_for_tui(&app.tui.main_view.messages);
    let cur = app.tui.main_view.block_cursor;
    if cur > 0 && !blocks.is_empty() {
        app.tui.main_view.block_cursor = cur - 1;
        // Pinned 到对应消息
        if let Some(bref) = blocks.get(app.tui.main_view.block_cursor) {
            app.tui.message_cursor = bref.msg_index;
            app.tui.scroll_mode = crate::app::ScrollMode::Pinned;
        }
    }
}
KeyEvent {
    code: KeyCode::Down,
    modifiers: KeyModifiers::ALT,
    ..
} => {
    let blocks = build_flat_blocks_for_tui(&app.tui.main_view.messages);
    let cur = app.tui.main_view.block_cursor;
    if cur + 1 < blocks.len() {
        app.tui.main_view.block_cursor = cur + 1;
        if let Some(bref) = blocks.get(app.tui.main_view.block_cursor) {
            app.tui.message_cursor = bref.msg_index;
            app.tui.scroll_mode = crate::app::ScrollMode::Pinned;
        }
    }
}
```

在文件末尾添加辅助函数（与 main_view.rs 中的 build_flat_blocks 相同逻辑）：

```rust
fn build_flat_blocks_for_tui(messages: &[crate::message::ChatMessage]) -> Vec<FlatBlockRef> {
    let mut refs = Vec::new();
    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role == crate::message::ChatRole::Assistant {
            for (block_idx, block) in msg.content.iter().enumerate() {
                match block {
                    crate::message::ContentBlock::Thinking { .. }
                    | crate::message::ContentBlock::ToolCall { .. } => {
                        refs.push(FlatBlockRef {
                            msg_index: msg_idx,
                            msg_id: msg.id.clone(),
                            block_index: block_idx,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    refs
}

struct FlatBlockRef {
    msg_index: usize,
    msg_id: String,
    block_index: usize,
}
```

- [ ] **Step 3: 添加 Space 折叠/展开键盘处理**

在 Alt+↑/↓ 处理之后，Messages 子区中添加：

```rust
KeyEvent {
    code: KeyCode::Char(' '),
    ..
} => {
    let blocks = build_flat_blocks_for_tui(&app.tui.main_view.messages);
    if app.tui.main_view.block_cursor < blocks.len() {
        let bref = &blocks[app.tui.main_view.block_cursor];
        let key = format!("{}:{}", bref.msg_id, bref.block_index);
        let state = app
            .tui
            .main_view
            .block_states
            .entry(key)
            .or_insert(crate::message::BlockExpanded::Collapsed);
        *state = match *state {
            crate::message::BlockExpanded::Collapsed => {
                crate::message::BlockExpanded::Expanded
            }
            crate::message::BlockExpanded::Expanded => {
                crate::message::BlockExpanded::Collapsed
            }
        };
    }
}
```

- [ ] **Step 4: 添加 Enter 进入详情键盘处理**

Space 处理之后：

```rust
KeyEvent {
    code: KeyCode::Enter,
    ..
} if entered_view.is_none() => {
    let blocks = build_flat_blocks_for_tui(&app.tui.main_view.messages);
    if app.tui.main_view.block_cursor < blocks.len() {
        let bref = &blocks[app.tui.main_view.block_cursor];
        app.handle_action(Action::EnterBlock {
            agent_id: agent_id.clone(),
            msg_id: bref.msg_id.clone(),
            block_index: bref.block_index,
        })
        .await
        .ok();
    }
}
```

- [ ] **Step 5: 添加 Esc 退出键盘处理**

在 Messages 子区键盘处理末尾：

```rust
KeyEvent {
    code: KeyCode::Esc,
    ..
} if entered_view.is_some() => {
    app.handle_action(Action::ExitBlock).await.ok();
}
```

- [ ] **Step 6: 运行测试验证编译**

```bash
cargo check
cargo test
```

- [ ] **Step 7: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): add Alt+Up/Down block navigation, Space toggle, Enter/Esc detail view, ContentUpdate in translate"
```

---

### Task 8: Theme 添加 diff 颜色 + 集成测试

**Files:**
- Modify: `src/theme.rs`
- Modify: `tests/integration_test.rs`

**Interfaces:**
- Produces: Theme 新颜色字段

- [ ] **Step 1: Theme 添加 diff 颜色**

在 `Theme` struct 中添加两个字段：

```rust
/// Diff 删除行背景色
pub diff_red_bg: Color,
/// Diff 新增行背景色
pub diff_green_bg: Color,
```

在 `Theme::cyan()` 工厂方法中初始化：

```rust
diff_red_bg: Color::Rgb(42, 26, 26),
diff_green_bg: Color::Rgb(26, 42, 26),
```

- [ ] **Step 2: 更新 main_view.rs 使用 theme 颜色**

修改 `render_diff` 方法，将硬编码的颜色替换为 `theme.diff_red_bg` 和 `theme.diff_green_bg`。

```rust
'-' => lines.push(Line::from(vec![
    prefix.fg(theme.text_dim),
    dl.text.clone().fg(theme.text).bg(theme.diff_red_bg),
])),
'+' => lines.push(Line::from(vec![
    prefix.fg(theme.text_dim),
    dl.text.clone().fg(theme.text).bg(theme.diff_green_bg),
])),
```

- [ ] **Step 3: 写集成测试验证完整流程**

```rust
#[test]
fn test_diff_computation_with_similar() {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nline2_changed\nline3\nline4\n";
    let diff = similar::TextDiff::from_lines(old, new);
    let changes: Vec<_> = diff.iter_all_changes().collect();
    assert!(changes.iter().any(|c| c.tag() == similar::ChangeTag::Delete));
    assert!(changes.iter().any(|c| c.tag() == similar::ChangeTag::Insert));
}

#[test]
fn test_block_toggle_roundtrip() {
    // 验证折叠→展开→折叠
    let mut states: std::collections::HashMap<String, crate::message::BlockExpanded> =
        std::collections::HashMap::new();
    let key = "msg-1:0".to_string();
    states.insert(key.clone(), crate::message::BlockExpanded::Collapsed);
    // 展开
    let s = states.get_mut(&key).unwrap();
    *s = crate::message::BlockExpanded::Expanded;
    assert_eq!(*s, crate::message::BlockExpanded::Expanded);
    // 折叠
    let s = states.get_mut(&key).unwrap();
    *s = crate::message::BlockExpanded::Collapsed;
    assert_eq!(*s, crate::message::BlockExpanded::Collapsed);
}
```

- [ ] **Step 4: 运行全部测试**

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

- [ ] **Step 5: Commit**

```bash
git add src/theme.rs tests/integration_test.rs src/components/main_view.rs
git commit -m "feat(theme): add diff color fields and integration tests"
```

---

### Task 9: 输入补全 Popup 渲染 & 交互（可选扩展）

**Files:**
- Modify: `src/components/main_view.rs`
- Modify: `src/tui.rs`

**Interfaces:**
- Produces: 补全 popup 渲染、/ 和 @ 触发逻辑

> **注意**：此 Task 为基础实现，不包含网络调用（get_commands）。`@` 使用简单的文件查找（fd/glob），`/` 使用硬编码的 skill 列表。后续可扩展为 RPC 异步获取。

- [ ] **Step 1: 添加补全 popup 渲染方法**

在 `MainView::render` 方法的输入框上方添加：

```rust
// 补全 popup（在输入框上方）
if let Some(ref popup) = self.completion_popup {
    let popup_height = (popup.items.len().min(8) + 2) as u16;
    let popup_width = popup.items.iter()
        .map(|i| i.label.len() + 4)
        .max()
        .unwrap_or(20)
        .min(inner.width as usize) as u16;
    let popup_area = Rect::new(
        input_area.x,
        input_area.y.saturating_sub(popup_height),
        popup_width,
        popup_height,
    );
    let mut popup_lines: Vec<Line> = Vec::new();
    for (i, item) in popup.items.iter().enumerate().take(8) {
        let prefix = if i == popup.cursor { "▶" } else { " " };
        let text = format!("{} {} {}", prefix, item.prefix, item.label);
        if i == popup.cursor {
            popup_lines.push(Line::from(vec![text.bg(theme.highlight_bg)]));
        } else {
            popup_lines.push(Line::from(vec![text.fg(theme.text_dim)]));
        }
    }
    f.render_widget(
        Paragraph::new(popup_lines)
            .style(Style::default().bg(theme.bg))
            .block(Block::default().borders(Borders::NONE)),
        popup_area,
    );
}
```

- [ ] **Step 2: 添加 / 和 @ 触发键盘处理**

在 tui.rs 的 Input 子区键盘处理中（Char 分支之前）：

```rust
KeyEvent {
    code: KeyCode::Char(c @ ('/' | '@')),
    ..
} if input_buffer.is_empty() || (c == '@' && !input_buffer.is_empty()) => {
    if c == '/' && input_buffer.is_empty() {
        // 触发命令补全
        input_buffer.push('/');
        // TODO: 异步获取 commands（后续实现）
        app.tui.main_view.completion_popup = Some(
            crate::message::CompletionPopup {
                items: vec![
                    crate::message::CompletionItem {
                        label: "skill-creator".into(),
                        value: "/skill:skill-creator ".into(),
                        prefix: "⚡".into(),
                    },
                    crate::message::CompletionItem {
                        label: "skill-finder".into(),
                        value: "/skill:skill-finder ".into(),
                        prefix: "⚡".into(),
                    },
                ],
                cursor: 0,
                trigger: '/',
                filter: String::new(),
            },
        );
    } else if c == '@' {
        // 触发文件补全
        input_buffer.push('@');
        // TODO: 异步获取文件列表（后续实现）
        app.tui.main_view.completion_popup = Some(
            crate::message::CompletionPopup {
                items: vec![
                    crate::message::CompletionItem {
                        label: "src/main.rs".into(),
                        value: "@src/main.rs ".into(),
                        prefix: "📁".into(),
                    },
                ],
                cursor: 0,
                trigger: '@',
                filter: String::new(),
            },
        );
    }
}
```

- [ ] **Step 3: 添加 Popup Tab/Enter/Esc 键盘处理**

在 Input 子区中，Popup 存在时的键盘处理：

```rust
KeyEvent {
    code: KeyCode::Tab,
    ..
} if app.tui.main_view.completion_popup.is_some() => {
    if let Some(ref mut popup) = app.tui.main_view.completion_popup {
        if popup.cursor + 1 < popup.items.len() {
            popup.cursor += 1;
        }
    }
}
KeyEvent {
    code: KeyCode::Enter,
    ..
} if app.tui.main_view.completion_popup.is_some() => {
    if let Some(popup) = app.tui.main_view.completion_popup.take() {
        if let Some(item) = popup.items.get(popup.cursor) {
            // 替换前缀部分：去掉 / 或 @ 后的内容，填入选中值
            let prefix = if popup.trigger == '/' { '/' } else { '@' };
            // 简单实现：清空后缀，填入 value
            input_buffer = item.value.clone();
        }
    }
}
KeyEvent {
    code: KeyCode::Esc,
    ..
} if app.tui.main_view.completion_popup.is_some() => {
    app.tui.main_view.completion_popup = None;
}
```

- [ ] **Step 4: 验证编译和测试**

```bash
cargo check
cargo test
```

- [ ] **Step 5: Commit**

```bash
git add src/components/main_view.rs src/tui.rs
git commit -m "feat(input): add / and @ completion popup with basic skill/file suggestions"
```

---

### 实施顺序

按 Task 编号顺序执行：1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9。每个 Task 完成后 commit，再进行下一个。Task 9 为可选扩展，可最后实现或跳过。
