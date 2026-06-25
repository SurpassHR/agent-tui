# 鼠标滚轮滚动支持 — 设计文档

> 日期：2026-06-25 | 状态：已确认（经审查修订）

## 概述

为 Agent TUI 的所有内容溢出区域添加鼠标滚轮（`ScrollDown`/`ScrollUp`）滚动支持。

## 目标范围

所有存在内容溢出可能的区域：

| 区域 | 当前滚动方式 | 新增 |
|------|-------------|------|
| MainView 消息列表 | 键盘 ↑↓ + ScrollMode | 滚轮移动 message_cursor + Pinned |
| MainView EnteredView::FullOutput | scroll 字段存在但渲染忽略 | 滚轮调整 scroll，渲染正确使用 + 渲染时 clamp |
| MainView EnteredView::Diff | 无滚动 | 新增 scroll 字段，滚轮调整 |
| MainView EnteredView::Subagent | 硬编码 scroll: 0 | 新增 scroll 字段，同 FullOutput |
| Sidebar 工作区树 | 键盘 ↑↓ 驱动 cursor + compute_ws_scroll | 滚轮移动 cursor |
| Sidebar PROVIDER 列表 | 键盘 ↑↓ 驱动 provider_cursor | 滚轮移动 provider_cursor |
| Sidebar MODEL 列表 | 键盘 ↑↓ 驱动 model_cursor | 滚轮移动 model_cursor |
| Agent Panel | 无 | 后续补充（当前无溢出内容） |

## 架构

### 事件路由

```
crossterm ScrollDown/ScrollUp
        │
        ▼
tui.rs MouseEventKind::ScrollDown | ScrollUp 分支
        │
        ├─ mouse.column ≤ 39  ──▶ Sidebar hit-test → 移动对应 cursor
        ├─ mouse.column ≥ term_w-40 ──▶ Agent Panel（暂忽略）
        └─ 其他 ──▶ MainView hit-test
                     ├─ 鼠标在输入区 → 忽略（输入区无可滚动内容）
                     └─ 鼠标在消息区 → 移动 message_cursor | entered_view.scroll
```

面板边界复用已有选区判定逻辑（39 / term_w - 40）。

### 子区命中测试

各组件在 `render(&mut self, ...)` 末尾将计算出的子区布局 `Rect` 存入组件状态字段，事件处理时用鼠标 `row` 坐标匹配。

- **Sidebar**：存储 `layout_sections: [Rect; 4]`
  - `[0]` 活跃会话区
  - `[1]` 工作区树
  - `[2]` 分割线
  - `[3]` PROVIDER+MODEL 区（含搜索栏）
  - PROVIDER 列表和 MODEL 列表位于同一 footer 区，通过二级偏移区分：provider 区域为 footer 区顶部行，model 列表从 provider 行数之后开始
- **MainView**：存储 `messages_rect: Rect` 和 `input_rect: Rect`
  - 滚轮仅在 `messages_rect` 内生效，`input_rect` 内滚轮忽略
  - EnteredView 模式覆盖整个面板，滚轮直接路由到 entered_view 内部

### 数据流

```
render_tui() → Layout::split() → components[].render()
    │                                   │
    └── 计算顶层 Rect                    └── 计算子区 Rect，存入组件状态
                                            │
事件循环 ← crossterm ScrollDown/ScrollUp ──┘
    │
    ├── 命中 Sidebar 子区 → 修改 sidebar.cursor / provider_cursor / model_cursor
    │       └── 下次 render 时 compute_ws_scroll 自动跟随
    │
    └── 命中 MainView
            ├── entered_view 存在 → 调整 entered_view.scroll
            └── 否则 → 调整 message_cursor + ScrollMode
```

## 行为规格

### MainView 消息列表

- **向上滚**：自动切换到 `ScrollMode::Pinned`，每刻度移动 `message_cursor -= 1`（与键盘 ↑↓ 行为一致）
- **向下滚**：移动 `message_cursor += 1`，到达最后一条消息时切回 `TailFollow`
- 滚动粒度：1 条消息 / 刻度
- 鼠标在输入框区域时不触发消息滚动

### EnteredView（FullOutput / Diff / Subagent）

- 滚轮调整 `scroll` 字段，每刻度 ±1 行
- **渲染时 clamp**：`scroll` 取 `min(scroll, total_lines.saturating_sub(visible_lines))`
  - 这同时处理了终端 resize 后的偏移越界
- FullOutput / Subagent：`total_lines` = header(1) + markdown 渲染行数 + footer(1)
- Diff：`total_lines` = header(1) + diff_lines.len() + footer(1)

### Sidebar 工作区树 / PROVIDER / MODEL

- 滚轮移动对应 `cursor`/`provider_cursor`/`model_cursor`，每刻度 ±1 项
- 光标变化后渲染时 `compute_ws_scroll` / model_scroll 自动重新计算偏移
- 键盘 ↑↓ 和滚轮共享同一光标状态
- 鼠标 Y 坐标用于区分命中哪个子区（活跃会话区忽略滚轮）

## 实现细节

### EnteredView 滚动 clamp

`render_full_output` 和 `render_diff` 在渲染首行处 clamp scroll 偏移：

```rust
let max_scroll = total_lines.saturating_sub(visible_height as usize);
let scroll = (*scroll).min(max_scroll);
```

### Sidebar footer 区二级命中

Footer `Rect` 内部：
- 前 `provider_item_count` 行为 PROVIDER 项
- 之后为 MODEL 标题 + MODEL 搜索栏 + MODEL 项
- 通过 `mouse_row - footer_rect.y` 与 provider 行数比较来区分

### layout_sections 存储

Sidebar 在 `render()` 末尾写入：
```rust
self.layout_sections = [top_rect, ws_rect, divider_rect, footer_rect];
```

MainView 在 `render()` 末尾写入：
```rust
self.messages_rect = msg_area;
self.input_rect = input_area;
```

## 需要修改的文件

| 文件 | 变更 |
|------|------|
| `src/tui.rs` | 新增 `MouseEventKind::ScrollDown\|ScrollUp` 处理分支 + 辅助函数 |
| `src/components/main_view.rs` | 新增 `messages_rect`/`input_rect`；`render_full_output` 使用 scroll + clamp；`render_diff` 新增 scroll + clamp |
| `src/components/sidebar.rs` | 新增 `layout_sections: [Rect; 4]`；render 末尾写入 |
| `src/message.rs` | `EnteredView::Diff` 新增 `scroll: usize`；`EnteredView::Subagent` 新增 `scroll: usize` |
| `src/app.rs` | 删除 `MainView.scroll_offset` 死字段（可选清理）；更新 Subagent 构造处传递 scroll |

## 后续扩展

- Agent Panel 内容溢出时的滚轮支持
- 滚动粒度配置（当前硬编码 1 项/刻度）
- 平滑滚动动画（当前瞬时跳转）
