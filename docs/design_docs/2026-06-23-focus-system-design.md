# 多面板焦点切换系统设计

> 使用 Alt+方向键在侧边栏、主视图、Agent 面板之间切换焦点，并支持面板内部子区导航。

## 动机

当前 TUI 只有二元焦点（sidebar ↔ main_view），Agent 面板完全不可交互。随着 subagent 功能成熟，用户需要在三个面板之间自由导航，并操作每个面板的各个子区域。

## 焦点架构

### 三级焦点模型

```
面板层 (Alt+←/→)
│
├── 侧边栏 (Sidebar) ──────── 子区 (Alt+↑/↓)
│   ├── ACTIVE SESSION       显示信息，预留交互
│   ├── 工作区 (workspace)   现有 ↑/↓ 导航，Enter 选择/展开
│   └── MODEL                显示信息，预留交互
│
├── 主视图 (MainView) ─────── 子区 (Alt+↑/↓)
│   ├── 消息列表             新增 ↑/↓ 按消息级切换，Enter 弹出 Popup
│   └── 输入框               现有文本输入行为（默认焦点子区）
│
└── Agent 面板 (AgentPanel) ─ 子区 (Alt+↑/↓)
    ├── AGENTS               新增 ↑/↓ 选择 subagent（光标跳过 pi agent 行）
    └── TASKS                预留展示（当前占位）
```

### 快捷键映射

| 快捷键 | 作用域 | 行为 |
|--------|--------|------|
| Alt+← | 所有面板 | 焦点向左移动一个面板（循环） |
| Alt+→ | 所有面板 | 焦点向右移动一个面板（循环） |
| Alt+↑ | 当前面板内 | 切换到上一个子区 |
| Alt+↓ | 当前面板内 | 切换到下一个子区 |
| ↑/↓ | 子区内 | 子区内部导航 |
| Enter | 子区内 | 激活选中项 |
| Esc | Popup 可见时 | 关闭 Popup（不退出 TUI） |

**注意**：Alt+箭头依赖于终端对 Alt 修饰键的传递策略。某些终端（tmux、部分 Wayland 终端）可能将 `Alt+←` 解析为 `ESC ←` 序列。如果 Alt+箭头不可用，可使用 `Ctrl+[` 模拟 Alt。

### 面板焦点环顺序

```
Sidebar ←→ MainView ←→ AgentPanel
    ↑                        │
    └──────── 循环 ──────────┘
```

使用 `FocusPanel` 枚举而非 usize 索引，`CycleFocusPanel` 用 match 匹配方向跳跃。

### 子区定义

每个面板维护独立的子区状态（面板切换不重置子区）：

**SidebarSubsection**（3 个，循环：0→1→2→0）
- `0`: ActiveSession — 顶部活跃会话信息（只读展示，Enter 预留）
- `1`: Workspace — 中间工作区树（已有 ↑/↓ 导航，Enter 选择/展开）
- `2`: Model — 底部模型/Token 信息（只读展示，Enter 预留）

**MainViewSubsection**（2 个，循环：0→1→0）
- `0`: Messages — 消息列表（新增 ↑/↓ 按消息切换，Enter 弹出 Popup）
- `1`: Input — 输入框（现有行为。当此处聚焦时，↑/↓ 切换到 Messages 子区）

**AgentPanelSubsection**（2 个，循环：0→1→0）
- `0`: Agents — AGENTS 列表（新增 ↑/↓ 选择 subagent，光标仅遍历 subagent 项）
- `1`: Tasks — TASKS 占位（只读显示 "暂无任务"）

## 数据模型变更

### Action 枚举（`src/action.rs`）

新增两个 Action 变体。移除不再使用的 `CycleFocus`（原 Tab 快捷键用），保留 `SidebarMove`：

```rust
/// 面板焦点循环（1=向右，-1=向左）
CycleFocusPanel(i32),
/// 子区焦点切换（1=向下，-1=向上）
CycleFocusSubsection(i32),
```

**不新增** `SelectPrev`/`SelectNext`/`ActivateSelected`。原因：
- 侧边栏导航继续使用现有 `SidebarMove` Action
- 消息列表和 AGENTS 列表导航在 `tui.rs` 键盘处理中直接操作 `message_cursor`/`agent_cursor`
- Enter 激活在 `tui.rs` 中根据焦点上下文直接处理

### TuiState（`src/app.rs`）

```rust
/// 聚焦的面板（替代旧的 sidebar_has_focus）
pub focus_panel: FocusPanel,

/// 面板枚举
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusPanel {
    Sidebar = 0,
    MainView = 1,
    AgentPanel = 2,
}
```

`sidebar_has_focus` 和 `main_view.has_focus` 合并到 `focus_panel` 管理。`tui.rs` 和 `sync_components()` 从 `focus_panel` 推导各组件的 `has_focus`。

```rust
/// 各面板的当前子区（面板切换时保持独立）
pub sidebar_subsection: SidebarSubsection,
pub main_view_subsection: MainViewSubsection,
pub agent_panel_subsection: AgentPanelSubsection,

/// Agent 面板内 subagent 列表选中光标（仅索引 Vec<SubAgentInfo>，不含 pi agent 行）
pub agent_cursor: usize,
/// 消息列表中选择的消息索引（指向 messages 数组中的索引）
pub message_cursor: usize,
```

子区枚举：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarSubsection {
    ActiveSession,
    Workspace,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MainViewSubsection {
    Messages,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentPanelSubsection {
    Agents,
    Tasks,
}
```

### 旧字段替代映射

| 旧字段 | 被替代方式 |
|--------|-----------|
| `app.tui.sidebar_has_focus: bool` | `app.tui.focus_panel == FocusPanel::Sidebar` |
| `app.tui.main_view.has_focus: bool` | `app.tui.focus_panel == FocusPanel::MainView` |
| `tui.rs: sidebar_focused` 变量 | 检查 `focus_panel` |
| `tui.rs: app.tui.main_view.has_focus` 分支 | `focus_panel == MainView && subsection == Input` |
| `Action::CycleFocus` | 移除（原 Tab 快捷键，不再使用） |

## sync_components 变更

每帧渲染前 `sync_components()` 新增以下同步逻辑：

```rust
// 从 focus_panel 推导各组件 has_focus
self.tui.sidebar.has_focus = self.tui.focus_panel == FocusPanel::Sidebar;
self.tui.main_view.has_focus = self.tui.focus_panel == FocusPanel::MainView;
// AgentPanel 新增 has_focus 字段
self.tui.agent_panel.has_focus = self.tui.focus_panel == FocusPanel::AgentPanel;

// 同步子区到组件
self.tui.sidebar.subsection = self.tui.sidebar_subsection;
self.tui.main_view.subsection = self.tui.main_view_subsection;
self.tui.agent_panel.subsection = self.tui.agent_panel_subsection;
self.tui.agent_panel.subsection = self.tui.agent_panel_subsection;

// 同步光标
self.tui.agent_panel.cursor = self.tui.agent_cursor;
self.tui.main_view.message_cursor = self.tui.message_cursor;
```

## 键盘事件处理逻辑（`src/tui.rs`）

### 完整路由表

| 按键 | 焦点上下文 | 行为 |
|------|-----------|------|
| Alt+← | 任意 | `CycleFocusPanel(-1)` |
| Alt+→ | 任意 | `CycleFocusPanel(1)` |
| Alt+↑ | 任意 | `CycleFocusSubsection(-1)` |
| Alt+↓ | 任意 | `CycleFocusSubsection(1)` |
| ↑ | Sidebar + Workspace | `SidebarMove(-1)` |
| ↓ | Sidebar + Workspace | `SidebarMove(1)` |
| ↑ | MainView + Messages | `message_cursor -= 1`（clamp 到 0） |
| ↓ | MainView + Messages | `message_cursor += 1`（clamp 到 messages.len()-1） |
| ↑ | MainView + Input | 切换到 MainViewSubsection::Messages（"往上翻看消息"） |
| ↓ | MainView + Input | 无操作（已在最底部子区） |
| ↑ | AgentPanel + Agents | `agent_cursor -= 1`（clamp 到 0） |
| ↓ | AgentPanel + Agents | `agent_cursor += 1`（clamp 到 subagents.len()-1） |
| Enter | Sidebar + Workspace | 展开/选择会话 |
| Enter | MainView + Messages | 弹出 Popup 显示该消息完整内容 |
| Enter | MainView + Input | 提交输入 |
| Enter | AgentPanel + Agents | 高亮选中 subagent（仅视觉，不调用后端） |
| Esc | Popup 可见 | 关闭 Popup（`TogglePopup`） |
| Ctrl+C | 任意 | 退出 TUI |
| Ctrl+Z | 任意 | abort

### 消息光标与滚动策略

引入 `message_cursor` 后，MainView 需要支持两种滚动模式：

```rust
pub enum ScrollMode {
    /// 尾部跟随：新消息到达自动滚动到底部（默认）
    TailFollow,
    /// 固定到光标所在消息
    Pinned(usize),
}
```

行为规则：
- 初始状态为 `TailFollow`，光标在最后一条消息
- 用户 ↑ 离开尾部时 → 切换为 `Pinned(message_cursor)`，滚动窗口显示光标附近
- 用户 ↓ 回到最后一条消息 → 恢复 `TailFollow`
- 新消息到达时：`TailFollow` 模式自动更新光标到最新消息；`Pinned` 模式不移动

渲染算法：
```
1. 计算总消息行数（all_lines）
2. 如果 Pinned 模式，从 cursor 所在消息的开始行往上算 msg_area_height 行
3. 如果 TailFollow 模式，取最后 msg_area_height 行（现有行为）
```

`main_view.rs` 新增字段：
```rust
pub message_cursor: usize,
pub scroll_mode: ScrollMode,
```

## 组件变更

### Sidebar

- 新增 `pub subsection: SidebarSubsection` 字段
- ACTIVE SESSION / 工作区 / MODEL 三区分别根据 `subsection == Self` 渲染不同样式
- 聚焦子区：左侧显示指示条 `▎` + 标题使用 `accent` 色
- 非聚焦子区：现有样式（不变）
- 非聚焦面板（`has_focus == false`）：不显示子区指示条

### MainView

- 新增 `pub subsection: MainViewSubsection` 字段
- 新增 `pub message_cursor: usize` 和 `pub scroll_mode: ScrollMode`
- Messages 子区聚焦时：
  - 每条消息视为一个焦点项
  - `message_cursor` 索引指向 `self.messages` 中的一条消息
  - 选中的消息用 `highlight_bg` 背景高亮
  - Enter 弹出 Popup（设置 popup.title = 消息角色，popup.description = 消息内容）
- Input 子区聚焦时：现有行为不变
- 非聚焦面板时：不显示消息高亮，输入框用灰色（已有）

### AgentPanel

- 新增 `pub has_focus: bool`、`pub subsection: AgentPanelSubsection`、`pub cursor: usize`
- AGENTS 区聚焦时：
  - subagent 列表项可选中
  - `cursor` 只索引 `subagents` 数组（pi agent 行不参与光标计算）
  - 选中的 subagent 用 `highlight_bg` 背景 + `selection_fg` 白色高亮
  - Enter 选中高亮（不调用后端）
- TASKS 区聚焦时：占位显示，展示 "暂无任务"（未来展示运行中任务）
- 非聚焦面板时：现有样式

### Popup

- Enter 在 MainView+Messages 时：`popup.visible = true`，`popup.description = 消息内容`
- Popup 可见时处理 Esc 键关闭
- Popup 内导航不在此阶段实现（静态展示）

## 视觉样式

| 状态 | 样式 |
|------|------|
| 面板聚焦 | 边框使用 `accent` 色，不聚焦使用 `border` 色 |
| 子区聚焦 | 左侧 `▎` 指示条 + 标题 `accent` 色 + bold |
| 子区未聚焦 | 无指示条，标题 `heading` 色 |
| 项选中 | 现有高亮（`selection_fg` 白色 + `highlight_bg`） |

侧边栏边框 `Borders::RIGHT` 在聚焦时使用 `accent` 色，不聚焦时使用 `border` 色。
Agent 面板边框 `Borders::LEFT` 同样改为聚焦感知。

## handle_action 实现细节

### CycleFocusPanel(direction)

```rust
Action::CycleFocusPanel(dir) => {
    let panels = [FocusPanel::Sidebar, FocusPanel::MainView, FocusPanel::AgentPanel];
    let current = self.tui.focus_panel;
    let idx = panels.iter().position(|p| *p == current).unwrap_or(1);
    let next = ((idx as i32 + dir).rem_euclid(3)) as usize;
    self.tui.focus_panel = panels[next];
    // 如果 AgentPanel 被聚焦且其子区对应的组件无焦点字段，初始化
}
```

### CycleFocusSubsection(direction)

```rust
Action::CycleFocusSubsection(dir) => {
    match self.tui.focus_panel {
        FocusPanel::Sidebar => {
            let subs = [SidebarSubsection::ActiveSession, SidebarSubsection::Workspace, SidebarSubsection::Model];
            let current = self.tui.sidebar_subsection;
            let idx = subs.iter().position(|s| *s == current).unwrap_or(1);
            let next = ((idx as i32 + dir).rem_euclid(3)) as usize;
            self.tui.sidebar_subsection = subs[next];
        }
        FocusPanel::MainView => {
            let subs = [MainViewSubsection::Messages, MainViewSubsection::Input];
            let current = self.tui.main_view_subsection;
            let idx = subs.iter().position(|s| *s == current).unwrap_or(1);
            let next = ((idx as i32 + dir).rem_euclid(2)) as usize;
            self.tui.main_view_subsection = subs[next];
        }
        FocusPanel::AgentPanel => {
            let subs = [AgentPanelSubsection::Agents, AgentPanelSubsection::Tasks];
            let current = self.tui.agent_panel_subsection;
            let idx = subs.iter().position(|s| *s == current).unwrap_or(0);
            let next = ((idx as i32 + dir).rem_euclid(2)) as usize;
            self.tui.agent_panel_subsection = subs[next];
        }
    }
}
```

## 错误处理

- 子区超出范围：`rem_euclid` 保证永远有效
- 面板切换：`rem_euclid` 保证永远有效
- 消息光标超出消息范围：clamp 到 `[0, messages.len().saturating_sub(1)]`
- agent 光标超出 subagent 列表：clamp 到 `[0, subagents.len().saturating_sub(1)]`
- `handle_action` 中的未匹配分支使用 `tracing::warn!` 记录

## 测试策略

- 单元测试验证 `CycleFocusPanel` 和 `CycleFocusSubsection` 状态转换
- 测试 `FocusPanel` 枚举的循环行为（wrap-around）
- 测试各面板的子区切换边界
- 测试 `message_cursor` 的 clamp 行为
- 集成测试验证按键分发后各组件的 `has_focus` 状态正确

## 不包含（此阶段不做）

- AgentPanel 中「唤起 subagent」的后端调用（阶段五）
- Popup 内键盘导航（方向键在 Popup 内翻页）
- 会话重命名（ACTIVE SESSION Enter 操作）
- 模型切换交互（MODEL 子区）
- MainView 消息折叠/展开（消息永久展开，仅是焦点可选中）
- Popup 焦点锁定（Popup 打开期间仍可切换到其他面板）
