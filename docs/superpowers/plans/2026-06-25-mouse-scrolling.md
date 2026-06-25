# 鼠标滚轮滚动支持 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Agent TUI 的所有内容溢出区域添加鼠标滚轮滚动支持。

**Architecture:** 在现有 `tui.rs` 事件循环中新增 `MouseEventKind::ScrollDown|ScrollUp` 处理，复用已有面板边界判定（column ≤ 39 / ≥ term_w - 40）。各组件在 `render()` 末尾存储子区布局 `Rect`，事件处理时用鼠标 Y 坐标命中测试。滚轮修改组件状态变量（cursor / scroll），渲染时自动跟随。

**Tech Stack:** Rust edition 2024, Ratatui 0.30.2, crossterm, tokio

## Global Constraints

- Rust edition 2024, MSRV 1.85
- 公开 API 返回 `anyhow::Result`；内部使用 `color-eyre`；禁止 `unwrap()`
- 所有 `pub fn` / `pub struct` 必须有 `///` 文档注释
- `cargo clippy -- -D warnings` 必须零警告
- `cargo fmt --check` 必须通过
- 提交遵循 Conventional Commits，用户语言（中文）

---

### Task 1: EnteredView 变体添加 scroll 字段

**Files:**
- Modify: `src/message.rs` — `EnteredView::Diff` 和 `EnteredView::Subagent` 变体
- Modify: `src/app.rs` — 所有构造 `EnteredView::Diff` 和 `EnteredView::Subagent` 的位置
- Modify: `src/app.rs` — 测试代码中的构造

**Interfaces:**
- Consumes: 无（独立初始任务）
- Produces:
  - `EnteredView::Diff { path: String, diff_lines: Vec<DiffLine>, scroll: usize }`
  - `EnteredView::Subagent { agent_id: String, messages: Vec<ChatMessage>, scroll: usize }`
  - 所有构造处新增 `scroll: 0`

- [ ] **Step 1: 为 `EnteredView::Diff` 添加 `scroll` 字段**

编辑 `src/message.rs:231-237`：

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
        /// 滚动偏移（行数）
        scroll: usize,
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
        /// 滚动偏移（行数）
        scroll: usize,
    },
}
```

- [ ] **Step 2: 为 `EnteredView::Subagent` 添加 `scroll` 字段**

已在 Step 1 一并完成（同一 enum 定义）。

- [ ] **Step 3: 构建检查，找到编译错误**

Run: `cargo build 2>&1 | head -60`

Expected: 编译错误出现在 `EnteredView::Diff { path, diff_lines }` 和 `EnteredView::Subagent { agent_id, messages }` 缺少 `scroll` 字段的位置。

- [ ] **Step 4: 修复所有构造 `EnteredView::Diff` 的位置**

编辑 `src/app.rs:1339` — `Action::EnterBlock` 中的 edit/write tool 构造：

```rust
// 原代码 (~line 1339):
self.tui.main_view.entered_view =
    Some(crate::message::EnteredView::Diff {
        path: path.to_string(),
        diff_lines,
    });

// 改为:
self.tui.main_view.entered_view =
    Some(crate::message::EnteredView::Diff {
        path: path.to_string(),
        diff_lines,
        scroll: 0,
    });
```

- [ ] **Step 5: 修复所有构造 `EnteredView::Subagent` 的位置**

`EnteredView::Subagent` 当前只在 `render_entered()` 的 match 分支中解构，无构造位置。检查确认无遗漏：

Run: `cargo build 2>&1 | grep -i "error" | head -20`

Expected: 无 Subagent 相关错误（如果无构造位置则无需修改）。

- [ ] **Step 6: 验证编译通过**

Run: `cargo build 2>&1`

Expected: 编译成功，零错误。

- [ ] **Step 7: 运行现有测试确保无回归**

Run: `cargo test 2>&1`

Expected: 所有测试通过。

- [ ] **Step 8: Commit**

```bash
git add src/message.rs src/app.rs
git commit -m "feat(message): 为 EnteredView::Diff 和 Subagent 添加 scroll 字段"
```

---

### Task 2: Sidebar 存储布局矩形

**Files:**
- Modify: `src/components/sidebar.rs` — 新增 `layout_sections` 字段 + render 末尾写入

**Interfaces:**
- Consumes: 无
- Produces: `Sidebar::layout_sections: [Rect; 4]`（活跃会话区, 工作区树, 分割线, PROVIDER+MODEL 区）

- [ ] **Step 1: 添加 `layout_sections` 字段到 Sidebar 结构体**

编辑 `src/components/sidebar.rs:27-52`，在字段列表末尾添加：

```rust
    /// 最近一次渲染的子区布局 [活跃会话区, 工作区树, 分割线, PROVIDER+MODEL]
    pub layout_sections: [Rect; 4],
```

由于 `Rect` 不实现 `Default`，需要手动实现或使用 `Rect::ZERO`。修改 `Default` impl：

在 sidebar.rs 中，`#[derive(Default)]` 改为手动实现 Default：

```rust
impl Default for Sidebar {
    fn default() -> Self {
        Self {
            active_session: String::new(),
            session_id: String::new(),
            message_count: 0,
            workspaces: Vec::new(),
            has_focus: false,
            cursor: 0,
            subsection: SidebarSubsection::Workspace,
            selection: SelectionState::default(),
            providers: Vec::new(),
            router_running: false,
            current_model: String::new(),
            provider_cursor: 0,
            model_cursor: 0,
            model_search: String::new(),
            model_scroll: 0,
            port: 0,
            active_provider_idx: None,
            layout_sections: [Rect::ZERO; 4],
        }
    }
}
```

⚠️ 删除原有的 `#[derive(Default)]` 宏。

- [ ] **Step 2: 在 render() 末尾存储 section rects**

编辑 `src/components/sidebar.rs`，在 `render()` 函数末尾（`f.render_widget(footer_para, sections[2]);` 之后）添加：

```rust
        // 存储布局矩形供鼠标滚轮命中测试
        self.layout_sections = [
            sections[0], // 活跃会话区
            sections[1], // 工作区树
            Rect::default(), // 分割线（无滚动内容）
            sections[2], // PROVIDER+MODEL 区
        ];
```

`sections[2]` 是 footer 区（包含分割线 + PROVIDER + MODEL 的预渲染区域），命中测试时用 mouse_y 在此区域内做二级区分。

- [ ] **Step 3: 编译检查**

Run: `cargo build 2>&1`

Expected: 编译成功。需要确认 `Rect::ZERO` 常量可用（Ratatui 0.30.2 中为 `Rect::new(0,0,0,0)`）。

- [ ] **Step 4: 运行测试**

Run: `cargo test 2>&1`

Expected: 所有测试通过。

- [ ] **Step 5: Clippy 检查**

Run: `cargo clippy -- -D warnings`

Expected: 零警告。

- [ ] **Step 6: Commit**

```bash
git add src/components/sidebar.rs
git commit -m "feat(sidebar): 存储渲染子区布局矩形供鼠标滚轮命中测试"
```

---

### Task 3: MainView 存储布局矩形 + 实现滚动渲染

**Files:**
- Modify: `src/components/main_view.rs` — 新增 `messages_rect`/`input_rect` 字段；`render_full_output` 和 `render_diff` 使用 scroll 偏移 + clamp

**Interfaces:**
- Consumes:
  - `EnteredView::{Diff,FullOutput,Subagent}.scroll: usize`（Task 1）
- Produces:
  - `MainView::messages_rect: Rect`
  - `MainView::input_rect: Rect`
  - `render_full_output(f, area, theme, title, content, scroll)` — scroll 参数生效
  - `render_diff(f, area, theme, path, diff_lines, scroll)` — 新增 scroll 参数并生效

- [ ] **Step 1: 修改 render_full_output 使用 scroll 参数实现滚动**

编辑 `src/components/main_view.rs:565` 附近的 `render_full_output` 函数：

```rust
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
        let visible_height = inner.height.saturating_sub(1) as usize;
        let mut lines: Vec<Line<'static>> = Vec::new();

        let header = format!("┌─ {} ─", title);
        lines.push(Line::from(vec![header.fg(theme.text_dim)]));

        let clean = crate::strip_ansi(content);
        let md_lines = markdown::render(&clean, theme);
        for md_line in md_lines {
            let mut spans = vec![Span::from(" ")];
            spans.extend(md_line.spans.into_iter());
            lines.push(Line::from(spans));
        }

        let footer = "└".to_string() + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
        lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

        // Clamp scroll 偏移（防止终端 resize 后越界）
        let total_lines = lines.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = scroll.min(max_scroll);

        // 从 scroll 偏移开始取行
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(scroll)
            .take(visible_height)
            .collect();

        let paragraph = Paragraph::new(visible)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }
```

- [ ] **Step 2: 修改 render_diff 添加 scroll 参数并实现滚动**

编辑 `src/components/main_view.rs:497`：

```rust
    /// 渲染 Diff 视图
    fn render_diff(
        &self,
        f: &mut Frame,
        area: Rect,
        theme: &Theme,
        path: &str,
        diff_lines: &[crate::message::DiffLine],
        scroll: usize,
    ) {
        let inner = inset_content(area);
        let visible_height = inner.height.saturating_sub(1) as usize;
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
                '-' => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text).bg(theme.diff_red_bg),
                    ]));
                }
                '+' => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text).bg(theme.diff_green_bg),
                    ]));
                }
                _ => {
                    lines.push(Line::from(vec![
                        prefix.fg(theme.text_dim),
                        dl.text.clone().fg(theme.text),
                    ]));
                }
            }
        }

        let footer = "└".to_string() + &"─".repeat(inner.width.saturating_sub(2).max(1) as usize);
        lines.push(Line::from(vec![footer.fg(theme.text_dim)]));

        // Clamp scroll 偏移
        let total_lines = lines.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = scroll.min(max_scroll);

        // 从 scroll 偏移开始取行
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(scroll)
            .take(visible_height)
            .collect();

        let paragraph = Paragraph::new(visible)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }
```

- [ ] **Step 3: 更新 render_entered 中的调用传递 scroll**

编辑 `src/components/main_view.rs:460-493` 的 `render_entered` match 分支：

```rust
        match view {
            crate::message::EnteredView::Diff {
                path,
                diff_lines,
                scroll,
            } => {
                self.render_diff(f, area, theme, path, diff_lines, *scroll);
            }
            crate::message::EnteredView::FullOutput {
                title,
                content,
                scroll,
            } => {
                self.render_full_output(f, area, theme, title, content, *scroll);
            }
            crate::message::EnteredView::Subagent {
                agent_id,
                messages,
                scroll,
            } => {
                let title = format!("subagent: {}", agent_id);
                let content = messages
                    .iter()
                    .map(|m| {
                        format!(
                            "[{}] {}",
                            match m.role {
                                crate::message::ChatRole::User => "你",
                                crate::message::ChatRole::Assistant => "pi",
                                _ => "..",
                            },
                            m.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.render_full_output(f, area, theme, &title, &content, *scroll);
            }
        }
```

- [ ] **Step 4: 编译检查**

Run: `cargo build 2>&1`

Expected: 编译成功。

- [ ] **Step 5: 在 MainView 结构体添加 `messages_rect` 和 `input_rect` 字段**

编辑 `src/components/main_view.rs:18-34`，在字段列表末尾添加：

```rust
    /// 最近一次渲染的消息区域布局（供鼠标滚轮命中测试）
    pub messages_rect: Rect,
    /// 最近一次渲染的输入区域布局（滚轮忽略此区域）
    pub input_rect: Rect,
```

并在 `Default::default()` 中添加默认值：

```rust
            messages_rect: Rect::ZERO,
            input_rect: Rect::ZERO,
```

- [ ] **Step 6: 在 MainView::render() 末尾存储布局矩形**

编辑 `src/components/main_view.rs:445`（`render()` 末尾，`f.render_widget(Paragraph::new(input_line), input_area);` 之后），添加：

```rust
        // 存储布局矩形供鼠标滚轮命中测试
        self.messages_rect = msg_area;
        self.input_rect = input_area;
```

- [ ] **Step 7: 编译 + 测试**

Run: `cargo build && cargo test 2>&1`

Expected: 编译成功，所有测试通过。

- [ ] **Step 8: Clippy**

Run: `cargo clippy -- -D warnings`

Expected: 零警告。

- [ ] **Step 9: Commit**

```bash
git add src/components/main_view.rs
git commit -m "feat(main_view): 实现 EnteredView 滚动渲染 + 存储布局矩形"
```

---

### Task 4: 实现鼠标滚轮事件处理

**Files:**
- Modify: `src/tui.rs` — 在现有 `Event::Mouse` 匹配分支中新增 `ScrollDown|ScrollUp` 处理

**Interfaces:**
- Consumes:
  - `Sidebar::layout_sections: [Rect; 4]`（Task 2）
  - `MainView::messages_rect: Rect`（Task 3）
  - `MainView::input_rect: Rect`（Task 3）
  - `TuiState::scroll_mode`, `TuiState::message_cursor`, `TuiState::sidebar_cursor`, `TuiState::provider_cursor`, `TuiState::model_cursor`
  - `MainView::entered_view`
- Produces: 无新类型（直接修改 TuiState 字段）

- [ ] **Step 1: 在事件循环中新增 ScrollDown/ScrollUp 匹配分支**

编辑 `src/tui.rs`，在 `MouseEventKind::Up(MouseButton::Left) => { ... }` 分支之后（`sel.active = false; ... }` 结束的大括号后），新增：

```rust
                        // ── 鼠标滚轮事件（内容滚动） ──
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                            let delta: isize = match mouse.kind {
                                MouseEventKind::ScrollDown => 1,
                                MouseEventKind::ScrollUp => -1,
                                _ => 0,
                            };
                            let bounds = (
                                39u16,
                                term_size.width.saturating_sub(40),
                            );

                            if mouse.column <= bounds.0 {
                                // Sidebar 区域
                                let secs = &app.tui.sidebar.layout_sections;
                                let mouse_y = mouse.row;

                                // 工作区树
                                if mouse_y >= secs[1].y
                                    && mouse_y < secs[1].y.saturating_add(secs[1].height)
                                {
                                    if delta > 0 {
                                        app.tui.sidebar_cursor = app
                                            .tui
                                            .sidebar_cursor
                                            .saturating_add(delta as usize);
                                    } else {
                                        app.tui.sidebar_cursor = app
                                            .tui
                                            .sidebar_cursor
                                            .saturating_sub((-delta) as usize);
                                    }
                                }
                                // PROVIDER+MODEL 区（footer）
                                else if mouse_y >= secs[3].y
                                    && mouse_y < secs[3].y.saturating_add(secs[3].height)
                                {
                                    // 二级命中：footer 内区分 PROVIDER 和 MODEL
                                    let offset = mouse_y.saturating_sub(secs[3].y) as usize;
                                    // provider 区行数 = 1(分隔线) + 1(PROVIDER标题) + providers.len()
                                    //   + (如果 providers 非空且 provider_cursor == len) 1(add provider)
                                    let divider_h = 1usize;
                                    let provider_title_h = 1usize;
                                    let provider_items = app.tui.providers.len()
                                        + if app.tui.providers.is_empty() {
                                            0
                                        } else {
                                            1
                                        }; // add provider 行
                                    let provider_total = divider_h + provider_title_h + provider_items;
                                    let model_title_h = 1usize;
                                    let model_search_h = if app.tui.sidebar.model_search.is_empty() {
                                        0
                                    } else {
                                        1
                                    };

                                    if offset < provider_total {
                                        // PROVIDER 区
                                        if delta > 0 {
                                            let new = app
                                                .tui
                                                .provider_cursor
                                                .saturating_add(delta as usize);
                                            // provider_cursor 最大为 providers.len() （add provider 行）
                                            app.tui.provider_cursor =
                                                new.min(app.tui.providers.len());
                                        } else {
                                            app.tui.provider_cursor = app
                                                .tui
                                                .provider_cursor
                                                .saturating_sub((-delta) as usize);
                                        }
                                    } else {
                                        // MODEL 区
                                        let ap = app
                                            .tui
                                            .providers
                                            .get(app.tui.active_provider_idx.unwrap_or(0));
                                        let model_count = ap.map(|p| p.models.len()).unwrap_or(0);
                                        if delta > 0 {
                                            let new = app
                                                .tui
                                                .model_cursor
                                                .saturating_add(delta as usize);
                                            app.tui.model_cursor =
                                                new.min(model_count.saturating_sub(1));
                                        } else {
                                            app.tui.model_cursor = app
                                                .tui
                                                .model_cursor
                                                .saturating_sub((-delta) as usize);
                                        }
                                    }
                                }
                                // 活跃会话区 — 无滚动内容，忽略
                            } else if mouse.column >= bounds.1 {
                                // Agent Panel — 暂忽略（无溢出内容）
                            } else {
                                // MainView 区域 — 先读取需要的不可变数据
                                let messages_rect = app.tui.main_view.messages_rect;
                                let total_msgs = app.tui.main_view.messages.len();

                                if app.tui.main_view.entered_view.is_some() {
                                    // EnteredView 模式 — 调整内部 scroll
                                    match &mut app.tui.main_view.entered_view {
                                        Some(crate::message::EnteredView::FullOutput {
                                            ref mut scroll,
                                            ..
                                        }) => {
                                            if delta > 0 {
                                                *scroll = scroll.saturating_add(delta as usize);
                                            } else {
                                                *scroll =
                                                    scroll.saturating_sub((-delta) as usize);
                                            }
                                        }
                                        Some(crate::message::EnteredView::Diff {
                                            ref mut scroll,
                                            ..
                                        }) => {
                                            if delta > 0 {
                                                *scroll = scroll.saturating_add(delta as usize);
                                            } else {
                                                *scroll =
                                                    scroll.saturating_sub((-delta) as usize);
                                            }
                                        }
                                        Some(crate::message::EnteredView::Subagent {
                                            ref mut scroll,
                                            ..
                                        }) => {
                                            if delta > 0 {
                                                *scroll = scroll.saturating_add(delta as usize);
                                            } else {
                                                *scroll =
                                                    scroll.saturating_sub((-delta) as usize);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if mouse.row >= messages_rect.y
                                    && mouse.row
                                        < messages_rect
                                            .y
                                            .saturating_add(messages_rect.height)
                                {
                                    // 普通消息列表 — 仅在消息区域响应滚轮
                                    if delta > 0 {
                                        // 向下滚
                                        let new = app
                                            .tui
                                            .message_cursor
                                            .saturating_add(delta as usize);
                                        app.tui.message_cursor =
                                            new.min(total_msgs.saturating_sub(1));
                                        if app.tui.message_cursor
                                            >= total_msgs.saturating_sub(1)
                                        {
                                            app.tui.scroll_mode =
                                                crate::app::ScrollMode::TailFollow;
                                        }
                                    } else {
                                        // 向上滚 → Pinned
                                        app.tui.scroll_mode =
                                            crate::app::ScrollMode::Pinned;
                                        app.tui.message_cursor = app
                                            .tui
                                            .message_cursor
                                            .saturating_sub((-delta) as usize);
                                    }
                                }
                                // 输入区 — 忽略滚轮
                            }
                        }
```

注意：ScrollDown（向下滚）使内容上移，对应 `delta = 1`（crossterm 约定）。

- [ ] **Step 2: 编译检查**

Run: `cargo build 2>&1`

Expected: 编译成功。

- [ ] **Step 3: Clippy 检查**

Run: `cargo clippy -- -D warnings`

Expected: 零警告。

- [ ] **Step 4: 格式检查**

Run: `cargo fmt --check`

Expected: 格式正确。如有格式问题，运行 `cargo fmt` 修复。

- [ ] **Step 5: 运行所有测试**

Run: `cargo test 2>&1`

Expected: 所有测试通过。

- [ ] **Step 6: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): 实现鼠标滚轮滚动事件处理"
```

---

### Task 5: 清理死字段 `scroll_offset`

**Files:**
- Modify: `src/components/main_view.rs` — 删除 `scroll_offset` 字段及 Default 中的初始化

**Interfaces:**
- Consumes: 无
- Produces: 无（纯清理）

- [ ] **Step 1: 删除 `scroll_offset` 字段定义**

编辑 `src/components/main_view.rs:22`，删除：

```rust
    /// 滚动偏移
    pub scroll_offset: usize,
```

- [ ] **Step 2: 删除 Default 中的初始化**

编辑 `src/components/main_view.rs:51`，删除：

```rust
            scroll_offset: 0,
```

- [ ] **Step 3: 搜索确认无其他引用**

Run: `grep -rn "scroll_offset" src/`

Expected: 无输出。

- [ ] **Step 4: 编译 + 测试**

Run: `cargo build && cargo test 2>&1`

Expected: 编译成功，所有测试通过。

- [ ] **Step 5: Commit**

```bash
git add src/components/main_view.rs
git commit -m "chore(main_view): 移除未使用的 scroll_offset 死字段"
```

---

### Task 6: 集成验证

**Files:**
- 无新文件（手动验证为主）
- Modify: 可选 — `src/main.rs` 开启 mouse capture（如未开启）

- [ ] **Step 1: 确认 EnableMouseCapture 已启用**

Run: `grep -n "EnableMouseCapture" src/tui.rs`

Expected: 已有 `EnableMouseCapture` 调用（当前代码在 `tui.rs:4` import 且应有启用逻辑）。如果没有，需要在 TUI 初始化时添加。

- [ ] **Step 2: 构建并运行 TUI**

Run: `cargo run -- --tui`

Expected: TUI 正常启动。手动测试：
- 鼠标在消息区滚轮 → 消息列表上下滚动，向上滚切换 Pinned
- 鼠标在 Sidebar 工作区滚轮 → 光标移动
- 鼠标在 Sidebar PROVIDER/MODEL 区滚轮 → 对应光标移动
- 鼠标在输入框滚轮 → 无效果
- 进入 Diff/FullOutput EnteredView 后滚轮 → 内容滚动，终端 resize 后不越界

- [ ] **Step 3: 运行完整测试套件**

Run: `cargo test && cargo clippy -- -D warnings && cargo fmt --check`

Expected: 全部通过。

- [ ] **Step 4: Commit**

```bash
git commit -m "test: 集成验证鼠标滚轮功能"
```
