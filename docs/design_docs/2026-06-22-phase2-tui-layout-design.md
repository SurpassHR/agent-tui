# Phase 2: Ratatui 工业风三栏网格静态框架

> 日期: 2026-06-22 | 状态: 设计评审 | 阶段: 二

## 1. 目标

在阶段一（Headless PTY I/O 隧道）的基础上，引入 Ratatui TUI 渲染层，
实现工业风三栏网格的静态布局框架，为后续阶段提供视觉基础。

### 1.1 范围（Scope）

**包含：**
- Raw 模式切换与终端大小变更监听
- 三栏布局（20% / 60% / 20%）
- 顶栏（TopBar）、底栏（BottomBar）
- Popup 浮层组件（Z-index 叠加，可开关）
- `run_tui()` 渲染循环与 `run_headless()` 共存

**不包含（后续阶段）：**
- 真实数据流接入（阶段三）
- 会话切换交互（阶段三）
- CLI 终端面板（阶段三）
- 主题/CRT 切换（阶段三）
- OutputInterceptor 数据联动（阶段四）

### 1.2 验收标准

- `cargo run -- --tui` 启动 TUI 模式，显示完整三栏布局
- `cargo run`（无参数）仍为 headless 模式，功能不变
- 终端 resize 时布局自动适配
- Popup 浮层可通过状态控制显示/隐藏
- 6 个已有集成测试全部通过
- 无 clippy 警告、格式正确

## 2. 架构设计

### 2.1 组件架构

遵循 Ratatui 组件化模式，每个面板实现统一的 `Component` trait：

```rust
pub trait Component {
    /// 渲染组件到指定的 Rect 区域
    fn render(&self, f: &mut Frame, area: Rect);
}
```

> 注意：阶段二为纯静态渲染，`Component` trait 暂不包含 `handle_event` 方法。
> 阶段三起补充事件处理。

**组件树：**

```
App (状态容器)
├── TopBar          — 顶部状态栏
├── Sidebar         — 左侧面板（SESSIONS）
├── MainView        — 中央面板（文档内容）
├── AgentPanel      — 右侧面板（AGENTS）
├── BottomBar       — 底部快捷键栏
└── Popup           — 浮层覆盖层（可选显示）
```

### 2.2 布局拓扑

```
┌──────────────────────────────────────────────────────┐
│ TopBar: ● CORE NODE ONLINE | 时间 | 会话ID | 轮次    │  Height=3
├──────────┬────────────────────────┬──────────────────┤
│ Sidebar  │      MainView          │   AgentPanel     │
│ (20%)    │       (60%)            │     (20%)        │
│          │                        │                  │
│ Sessions │  [Popup overlay]       │  AGENTS          │
│ 树形分类  │  文档内容区            │  TASKS           │
│          │                        │  控制台信息       │
│ 模型规格  │                        │                  │
│          │                        │                  │
├──────────┴────────────────────────┴──────────────────┤
│ BottomBar: Ctrl+Q  F2  /agent  /memory  Ctrl+N       │  Height=2
└──────────────────────────────────────────────────────┘
```

Ratatui 布局代码：

```rust
// 垂直切分
let main_layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
    .split(frame.area());

// 水平切分中间区域
let columns = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([
        Constraint::Ratio(1, 5),  // 20%
        Constraint::Ratio(3, 5),  // 60%
        Constraint::Ratio(1, 5),  // 20%
    ])
    .split(main_layout[1]);
```

### 2.3 Popup 浮层渲染机制

Popup 作为最高 Z-index 层级，使用 Ratatui 的 `Clear` widget 清除目标区域后渲染：

```rust
if app.state.show_popup {
    // 计算 popup 区域（居中于 MainView）
    let popup_area = centered_rect(70, 40, columns[1]);
    
    // Clear 清除该区域（产生叠加效果）
    f.render_widget(Clear, popup_area);
    
    // 渲染 Popup 内容
    popup.render(f, popup_area);
}
```

`centered_rect()` 辅助函数计算居中矩形：

```rust
/// 计算居中矩形（相对指定区域的居中百分比）
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v_layout = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Percentage(percent_y),
        Constraint::Fill(1),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(percent_x),
        Constraint::Fill(1),
    ])
    .split(v_layout[1])[1]
}
```

### 2.4 颜色方案

定义 `Theme` 结构体，初始仅 cyan 主题：

```rust
pub struct Theme {
    pub bg: Color,
    pub text: Color,
    pub text_dim: Color,
    pub border: Color,
    pub border_dim: Color,
    pub highlight_bg: Color,
}
```

Cyan 主题色值（对应参考实现）：

| Token | Color | RGB |
|-------|-------|-----|
| `bg` | 深色背景 | `Color::Rgb(11, 15, 22)` |
| `text` | 青色主色 | `Color::Rgb(92, 225, 230)` |
| `text_dim` | 暗青色 | `Color::Rgb(65, 163, 166)` |
| `border` | 青色边框 | `Color::Rgb(92, 225, 230)` |
| `border_dim` | 暗青边框 | `Color::Rgb(31, 86, 88)` |
| `highlight_bg` | 高亮背景 | `Color::Rgb(21, 52, 54)` |

## 3. 组件详细设计

### 3.1 TopBar

```
● CORE NODE ONLINE | 当前时间: 2026-06-09 21:34:06 | 会话ID: xxx.txt | 轮次: 3/4
```

- 左对齐：状态指示（绿色圆点 + "CORE NODE ONLINE"）
- 居中/分散：时间戳、会话ID、当前轮次
- 使用 `Paragraph` + `Line` + `Span` 组合实现
- 分割线：底部 `Block` 带下边框

### 3.2 Sidebar

```
● CURRENT SESSIONS (3)
  • 深度教程工作流构建
  • 调研MaaS语音API
  • Headless独立桌面构建

▼ SESSIONS (94)
  ▼ Recent (5)
    S01 Xvfb Chromium...
    S02 人机协作研究...
    ...
  ▼ API调研 (4)
    ...

CURRENT MODEL
  provider: deepseek-v4-flash
  model: deepseek-v4-flash

TOKEN USAGE
  total: 2.12M   in: 2.09M  out: 29.2k
  cache: 1.89M   hit: 91%
```

- 使用 `Paragraph` 配合 `Line` + `Span` 实现树形结构
- 缩进用空格模拟
- 底部固定为模型信息和 Token 统计
- 注意：阶段二为纯静态显示，所有数据硬编码

### 3.3 MainView

```
█ Part 3: 关键范式与架构

3.1 Human-in-the-Loop (HITL | 人类在回路中)
  人类是反馈循环的活跃部分...

3.2 Human-on-the-loop (HOTL | 人类在回路上)
  AI自主运行，人类监控并在异常时介入...

DATA PIPELINE FLOW
  ┌── code ────────────────────────────────┐
  Create New          ──➔ 创造新内容
  Extend              ──➔ 扩展现有内容
  ...
  └────────────────────────────────────────┘

机制分配与场景矩阵
  机制   人类主导    AI主导     典型场景
  ...
```

- 使用 `Paragraph` 渲染静态文档内容
- 使用 `Gauge` 或自定义 border 实现 ASCII 风格管线图
- 标题黄色高亮（`Color::Yellow`）
- 内容青色/白色层次分明

### 3.4 AgentPanel

```
● AGENTS (5)
  ● 主 agent          idle

👥 持久 (4)
  ○ Obsidian知识库管家  idle
  ○ 搜索代理            idle
  ○ ...

⌛ 临时会话 (0)
  ○ 暂无活动连接

📋 ACTIVE TASKS
  ● 调研MaaS接口延迟数据...
  [████████████░░░░░] 68%

SELECTED AGENT CONSOLE
  [主 agent]
  角色配置: 系统协同多智能体
  自适应率: 98.4%
  状态通道: READY_TO_EXEC
  可用槽位: 4/4
```

- 三段式结构：AGENTS 列表 / TASKS 进度 / 选中 Agent 信息
- 进度条用 Ratatui 的 `Gauge` widget
- 底部固定为 Agent Console 信息

### 3.5 BottomBar

```
Ctrl+Q 清空控制面板  │  F2 切换翠绿矩阵主题  │  /agent 唤醒搜索代理  │  ...
```

- 使用 `Paragraph` 渲染
- 快捷键按键用反白/加粗样式
- 分散布局

### 3.6 Popup

```
┌──────────────────────────────────────────────┐
│ [会话简介] ID: 081782366404007 (智能体)  [X] │
│                                              │
│  用户问询是否为其建立一套自我优化积累的...    │
│                                              │
│  提示: 输入 /switch 可挂载其他会话            │
└──────────────────────────────────────────────┘
```

- 使用 `Clear` widget 叠加渲染
- `Block` 带青色边框 + 标题
- 内容为静态文本
- 显示状态由 `app.state.show_popup` 控制

## 4. 与现有代码的集成

### 4.1 新增 `TuiState`

```rust
// src/app.rs
pub struct TuiState {
    pub show_popup: bool,
    pub active_session: String,
    pub current_time: String,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            show_popup: true,
            active_session: "S03".to_string(),
            current_time: String::new(),
        }
    }
}
```

### 4.2 `main.rs` 分派逻辑

```rust
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    logging::init()?;
    let config = Config::from_args();
    let (action_tx, action_rx) = mpsc::channel(1024);
    let backend = Box::new(PiPtyBackend::new(...));
    let app = App::new(backend, action_tx);

    let rt = Runtime::new()?;
    if config.tui_mode {
        rt.block_on(tui::run_tui(app, action_rx))?;
    } else {
        rt.block_on(async {
            let mut app = app;
            app.start().await?;
            tui::run_headless(app, action_rx).await
        })?;
    }
    Ok(())
}
```

### 4.3 `tui.rs` 新增 `run_tui()`

```rust
pub async fn run_tui(mut app: App, action_rx: Receiver<Action>) -> Result<()> {
    let mut terminal = ratatui::init();  // 初始化 crossterm raw mode + terminal
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    
    let result = AppResult::Ok(());
    
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // 定时刷新（更新时钟等）
                terminal.draw(|f| app.render_tui(f))?;
            }
            Some(action) = action_rx.recv() => {
                match action {
                    Action::PtyExit => break,
                    _ => {} // 暂不处理
                }
            }
            else => break,
        }
    }
    
    ratatui::restore();
    Ok(())
}
```

## 5. 测试策略

### 5.1 集成测试

- 无新增集成测试（阶段二为纯视觉框架，难以自动化测试）
- 6 个已有 PTY 集成测试必须全部通过
- 编译验证：`cargo build --features tui`

### 5.2 手动测试

```bash
# TUI 模式启动
cargo run -- --tui

# 验证：三栏布局正确显示
# 验证：Popup 可见
# 验证：resize 终端窗口，布局自适应
# 验证：headless 模式不受影响
cargo run -- --target /bin/sh
```

## 6. 风险和注意事项

| 风险 | 缓解措施 |
|------|---------|
| `ratatui::init()`/`restore()` 与 `crossterm` 状态冲突 | 确保 panic 时始终 restore；使用 `std::panic::set_hook` |
| 终端 resize 事件处理不当 | Ratatui 的 `draw()` 自动处理 resize，无需手动监听 SIGWINCH |
| Popup 渲染顺序错误 | 先渲染底层组件，再渲染 Popup（`Clear` + 内容） |
| headless 模式被破坏 | `run_headless()` 完全不变，仅新增 `run_tui()` |

## 7. 依赖变更

无需新增 Cargo 依赖。`ratatui` + `crossterm` 已在 Cargo.toml 中存在。

## 8. 实施顺序

```
Step 1: 更新 Component trait（components/mod.rs）
Step 2: 创建 TopBar 组件
Step 3: 创建 Sidebar 组件
Step 4: 创建 MainView 组件
Step 5: 创建 AgentPanel 组件
Step 6: 创建 BottomBar 组件
Step 7: 创建 Popup 组件
Step 8: 更新 App（新增 TuiState、tui 数据、render_tui 方法）
Step 9: 新增 run_tui() 渲染循环（tui.rs）
Step 10: 更新 main.rs（tui/headless 分派）
Step 11: 更新 lib.rs（导出新模块）
Step 12: cargo build + cargo test + cargo clippy
```
