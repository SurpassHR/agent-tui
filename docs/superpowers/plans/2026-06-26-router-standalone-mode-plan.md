# Router 独立启动模式（含配置 TUI）— 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development or executing-plans to implement this plan task-by-task.

**Goal:** 新增 `cargo run -- --router` 模式，启动轻量 Ratatui TUI 管理 Provider/Model 配置，Router 在背景运行。

**Architecture:** 新增 `src/router_tui.rs` 自包含的 TUI 应用，复用 `src/provider/` 的数据模型和 `start_router()`；`start_router()` 增加可选 shutdown 信号参数；`main.rs` 检测 `--router` 标志后走新分支。

**Tech Stack:** Ratatui 0.30 + crossterm 0.29 + tokio

## Global Constraints

- Rust edition 2024, MSRV 1.85
- 所有 pub fn/struct 必须有文档注释
- 禁止 unwrap()，使用 `?` 操作符
- 新代码必须通过 `cargo clippy -- -D warnings`
- 不需要新依赖（现有 ratatui 0.30 + crossterm 0.29 + tokio 1.x 已足够）

---

### Task 1: CLI flag + router shutdown signal + main.rs 分支

**Files:**
- Modify: `src/config.rs` — 新增 `--router` CLI 标志
- Modify: `src/provider/router.rs` — `start_router()` 增加可选 shutdown 参数
- Modify: `src/tui.rs` — 更新 `start_router()` 调用处传 `None`
- Modify: `src/main.rs` — 新增 `config.router` 分支

- [ ] **Step 1.1: config.rs — 新增 `--router` flag**

在 `src/config.rs` 的 `Config` 结构体中新增字段：

```rust
/// 独立启动 Provider Router（不启动 TUI 主界面，仅启动配置管理 TUI）
#[arg(long = "router", default_value_t = false)]
pub router: bool,
```

- [ ] **Step 1.2: router.rs — 增加 shutdown 信号参数**

```rust
use std::pin::Pin;
use std::future::Future;

pub async fn start_router(
    config: SharedConfig,
    shutdown: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
) -> Result<u16, crate::errors::Error> {
    // ... 原有代码在 Ok(listener) 分支改为:
    Ok(listener) => {
        let serve = axum::serve(listener, app);
        if let Some(signal) = shutdown {
            serve.with_graceful_shutdown(signal).await
        } else {
            serve.await
        }
        .map_err(|e| crate::errors::Error::Config(format!("axum serve: {}", e)))?;
        return Ok(try_port);
    }
```

- [ ] **Step 1.3: tui.rs — 更新调用处传 None**

定位 `start_router(router_shared)` 调用处（目前是 `start_router(router_shared).await`），改为：

```rust
start_router(router_shared, None).await
```

- [ ] **Step 1.4: main.rs — 新增 router 分支**

在 `main()` 函数中，在 `dry_run`、`diagnose` 之后，TUI 分支之前新增：

```rust
if config.router {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(meta_tui::router_tui::run_router_tui())?;
    return Ok(());
}
```

- [ ] **Step 1.5: 验证编译**

```bash
cargo build 2>&1 | head -20
```

预期：编译通过无错误。TUI 调用的 `start_router(shared, None).await` 匹配新签名。

---

### Task 2: `src/router_tui.rs` — 状态定义 + 终端事件循环

**Files:**
- Create: `src/router_tui.rs`
- Modify: `src/lib.rs` — 声明 `pub mod router_tui`

**Interfaces:**
- Produce: `pub async fn run_router_tui() -> Result<()>`

- [ ] **Step 2.1: lib.rs 注册模块**

```rust
pub mod router_tui;
```

- [ ] **Step 2.2: 创建 router_tui.rs 基本骨架**

```rust
//! Router 配置管理 TUI
//!
//! 轻量 Ratatui 应用，管理 Provider/Model 配置，Router 在背景运行。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use tokio::sync::RwLock;

use crate::provider::{
    self, config_path, ProviderConfig, ProviderInfo, SharedConfig, ModelInfo,
};

/// 视图模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum View {
    List,
    Form,
}

/// 表单模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum FormMode {
    Add,
    Edit(usize),
}

/// Endpoint 类型选项
const ENDPOINT_OPTIONS: &[&str] = &[
    "openai_compat",
    "openai_responses",
    "anthropic_messages",
    "gemini",
];

/// Router TUI 主状态
struct RouterTuiState {
    providers: Vec<ProviderInfo>,
    selected: usize,
    expanded: Option<usize>,

    view: View,

    // 表单字段
    form_id: String,
    form_name: String,
    form_url: String,
    form_key: String,
    form_ep_idx: usize,
    form_ep_open: bool,
    form_enabled: bool,
    form_models: String,
    form_focus: usize,
    form_mode: FormMode,

    // Router 背景任务
    router_running: bool,
    router_port: u16,
    router_shutdown: Option<tokio::sync::oneshot::Sender<()>>,

    // 配置
    config_path: PathBuf,
    shared_config: SharedConfig,

    request_count: u64,
    dirty: bool,
}
```

- [ ] **Step 2.3: 实现 `run_router_tui()`** 入口函数

```rust
/// 运行 Router 配置 TUI
pub async fn run_router_tui() -> Result<()> {
    let config_path = provider::config_path();
    let provider_cfg = ProviderConfig::load(&config_path);
    let port = provider_cfg.port;
    let shared_config = Arc::new(RwLock::new(provider_cfg.clone()));

    let mut state = RouterTuiState {
        providers: provider_cfg.providers.clone(),
        selected: 0,
        expanded: None,
        view: View::List,
        form_id: String::new(),
        form_name: String::new(),
        form_url: String::new(),
        form_key: String::new(),
        form_ep_idx: 0,
        form_ep_open: false,
        form_enabled: true,
        form_models: String::new(),
        form_focus: 0,
        form_mode: FormMode::Add,
        router_running: false,
        router_port: port,
        router_shutdown: None,
        config_path,
        shared_config: shared_config.clone(),
        request_count: 0,
        dirty: false,
    };

    // 启动 Router 背景任务
    start_router_background(&mut state).await;

    // 初始化终端
    let mut terminal = ratatui::init();

    // 事件循环
    let res = run_event_loop(&mut terminal, &mut state).await;

    // 停止 Router
    stop_router(&mut state).await;

    // 恢复终端
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = ratatui::try_restore();

    res
}
```

- [ ] **Step 2.4: 实现 run_event_loop**

```rust
async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    state: &mut RouterTuiState,
) -> Result<()> {
    loop {
        // 渲染
        terminal.draw(|f| render(f, state))?;

        // 等待键盘事件（100ms 超时以定期重绘）
        if !crossterm::event::poll(Duration::from_millis(100))? {
            continue;
        }

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_key(key, state) {
                break; // 返回 true 表示退出
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2.5: 实现渲染入口 `render()` + 键盘入口 `handle_key()`**

```rust
fn render(frame: &mut Frame, state: &mut RouterTuiState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // 顶部状态栏
            Constraint::Min(0),     // 内容区
            Constraint::Length(1),  // 底部快捷键栏
        ])
        .split(area);

    render_top_bar(frame, chunks[0], state);
    match state.view {
        View::List => render_list_view(frame, chunks[1], state),
        View::Form => render_form_view(frame, chunks[1], state),
    }
    render_bottom_bar(frame, chunks[2], state);
}

fn handle_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    match state.view {
        View::List => handle_list_key(key, state),
        View::Form => handle_form_key(key, state),
    }
}
```

- [ ] **Step 2.6: 空实现所有子函数 + 验证编译**

所有 `render_*()` `handle_*()` 函数先返回空内容 / 不做操作：

```rust
fn render_top_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {}
fn render_list_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {}
fn render_form_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {}
fn render_bottom_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {}
fn handle_list_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool { false }
fn handle_form_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool { false }
fn start_router_background(state: &mut RouterTuiState) -> impl std::future::Future<Output = ()> { async {} }
fn stop_router(state: &mut RouterTuiState) -> impl std::future::Future<Output = ()> { async {} }
```

验证编译：`cargo build`

---

### Task 3: `src/router_tui.rs` — 列表视图

- [ ] **Step 3.1: 渲染顶部状态栏**

```rust
fn render_top_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let status = if state.router_running {
        Span::styled(" ● Running", Style::new().fg(Color::Green))
    } else {
        Span::styled(" ● Stopped", Style::new().fg(Color::Red))
    };
    let port = format!(" 127.0.0.1:{}", state.router_port);
    let view_mode = match state.view {
        View::List => "LIST",
        View::Form => "EDIT",
    };

    let line = Line::from(vec![
        Span::styled(" Provider Router", Style::new().bold()),
        status,
        Span::styled(port, Style::new().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            format!("{} mode", view_mode),
            Style::new().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(
            format!("proxied: {}", state.request_count),
            Style::new().fg(Color::DarkGray),
        ),
    ]);

    let p = Paragraph::new(line).style(Style::new().bg(Color::Black).fg(Color::White));
    frame.render_widget(p, area);
}
```

- [ ] **Step 3.2: 渲染底部快捷键栏**

```rust
fn render_bottom_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let (keys, right) = match state.view {
        View::List => (
            "  ↑↓ Navigate  Enter Expand  a Add  e Edit  d Delete  q Quit",
            &format!("provider mode · {} providers", state.providers.len()),
        ),
        View::Form => (
            "  Tab Next  Shift+Tab Prev  Enter Dropdown  Esc Cancel  F2 Save",
            "INSERT mode",
        ),
    };

    let p = Paragraph::new(format!("{}{:>width$}", keys, right, width = area.width as usize - keys.len().saturating_sub(2)))
        .style(Style::new().bg(Color::Black).fg(Color::DarkGray));
    frame.render_widget(p, area);
}
```

- [ ] **Step 3.3: 渲染 provider 列表**

```rust
fn render_list_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // 标题行
    let title = format!(
        " Providers ({})",
        state.providers.len()
    );
    let title_p = Paragraph::new(Span::styled(
        &title,
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(title_p, chunks[0]);

    if state.providers.is_empty() {
        let empty = Paragraph::new("  No providers configured. Press 'a' to add one.")
            .style(Style::new().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    // 修正选中索引
    if state.selected >= state.providers.len() {
        state.selected = state.providers.len().saturating_sub(1);
    }

    let mut items: Vec<ListItem> = Vec::new();

    for (i, p) in state.providers.iter().enumerate() {
        let selected = i == state.selected;
        let expanded = state.expanded == Some(i);

        let prefix = if selected { " ● " } else { " ○ " };
        let enabled_badge = if p.enabled {
            Span::styled(" ENABLED ", Style::new().fg(Color::Green).bg(Color::Black))
        } else {
            Span::styled(" DISABLED ", Style::new().fg(Color::Red).bg(Color::Black))
        };

        let model_count = format!(" {} model{}", p.models.len(), if p.models.len() == 1 { "" } else { "s" });

        let header = Line::from(vec![
            Span::raw(prefix),
            Span::styled(&p.name, Style::new().bold()),
            Span::raw("  "),
            Span::styled(&p.endpoint_type, Style::new().fg(Color::DarkGray)),
            Span::raw("  "),
            enabled_badge,
            Span::raw("  "),
            Span::styled(model_count, Style::new().fg(Color::DarkGray)),
        ]);

        items.push(ListItem::new(header).style(if selected {
            Style::new().bg(Color::Rgb(29, 30, 56))
        } else {
            Style::default()
        }));

        // 展开的 model 子列表
        if expanded {
            for m in &p.models {
                let tier = Span::styled(
                    format!(" {} ", m.tier),
                    Style::new().fg(match m.tier.as_str() {
                        "T1" => Color::Rgb(167, 139, 250),
                        "T2" => Color::Rgb(96, 165, 250),
                        _ => Color::DarkGray,
                    })
                    .bg(Color::Rgb(26, 27, 46)),
                );
                let ctx = if m.context_window >= 1_000_000 {
                    format!(" {:.1}M ctx", m.context_window as f64 / 1_000_000.0)
                } else if m.context_window >= 1000 {
                    format!(" {}K ctx", m.context_window / 1000)
                } else {
                    format!(" {} ctx", m.context_window)
                };
                let reasoning = if m.reasoning {
                    Span::styled(" reasoning", Style::new().fg(Color::Yellow))
                } else {
                    Span::raw("")
                };

                let model_line = Line::from(vec![
                    Span::raw("     "),
                    Span::styled(&m.id, Style::new().fg(Color::White)),
                    Span::raw("  "),
                    tier,
                    Span::styled(ctx, Style::new().fg(Color::DarkGray)),
                    reasoning,
                ]);
                items.push(ListItem::new(model_line));
            }
        }
    }

    let list = List::new(items).highlight_style(Style::new().bg(Color::Rgb(29, 30, 56)));
    frame.render_widget(list, chunks[1]);
}
```

- [ ] **Step 3.4: 列表键盘处理**

```rust
fn handle_list_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.providers.is_empty() {
                state.selected = (state.selected + 1).min(state.providers.len() - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Right | KeyCode::Enter => {
            if state.expanded == Some(state.selected) {
                state.expanded = None;
            } else {
                state.expanded = Some(state.selected);
            }
        }
        KeyCode::Left => {
            if state.expanded == Some(state.selected) {
                state.expanded = None;
            }
        }
        KeyCode::Char('a') => {
            open_form_add(state);
        }
        KeyCode::Char('e') => {
            if !state.providers.is_empty() {
                open_form_edit(state, state.selected);
            }
        }
        KeyCode::Char('d') => {
            if !state.providers.is_empty() {
                delete_provider(state);
            }
        }
        KeyCode::Char('q') => {
            return true; // 退出
        }
        KeyCode::Esc => {
            return true;
        }
        _ => {}
    }
    false
}

fn open_form_add(state: &mut RouterTuiState) {
    state.view = View::Form;
    state.form_mode = FormMode::Add;
    state.form_id.clear();
    state.form_name.clear();
    state.form_url = "https://".to_string();
    state.form_key.clear();
    state.form_ep_idx = 0;
    state.form_ep_open = false;
    state.form_enabled = true;
    state.form_models.clear();
    state.form_focus = 0;
}

fn open_form_edit(state: &mut RouterTuiState, idx: usize) {
    let p = &state.providers[idx];
    state.view = View::Form;
    state.form_mode = FormMode::Edit(idx);
    state.form_id = p.id.clone();
    state.form_name = p.name.clone();
    state.form_url = p.base_url.clone();
    state.form_key.clear(); // 编辑时不填充 key（安全）
    state.form_ep_idx = ENDPOINT_OPTIONS
        .iter()
        .position(|&ep| ep == p.endpoint_type)
        .unwrap_or(0);
    state.form_ep_open = false;
    state.form_enabled = p.enabled;
    state.form_models = p
        .models
        .iter()
        .map(|m| {
            let mut s = format!("{}:{}:{}", m.id, m.tier, m.context_window);
            if m.reasoning {
                s.push_str(":reasoning");
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n");
    state.form_focus = 0;
}
```

- [ ] **Step 3.5: 编译验证**

```bash
cargo build 2>&1
```

---

### Task 4: `src/router_tui.rs` — 表单视图 + 增删改 + 持久化

- [ ] **Step 4.1: 渲染表单视图**

```rust
fn render_form_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    // 标题
    let title = match state.form_mode {
        FormMode::Add => "Add Provider",
        FormMode::Edit(_) => "Edit Provider",
    };

    // 计算表单可用高度
    let form_area = Rect::new(area.x, area.y, area.width, area.height);

    // 简单垂直列表：每个字段一行
    let constraints = [
        Constraint::Length(3),  // 标题
        Constraint::Length(3),  // ID
        Constraint::Length(3),  // Name
        Constraint::Length(3),  // Base URL
        Constraint::Length(3),  // API Key
        Constraint::Length(3),  // Endpoint
        if state.form_ep_open {
            Constraint::Length((ENDPOINT_OPTIONS.len() + 1) as u16)
        } else {
            Constraint::Length(0)
        },
        Constraint::Length(3),  // Enabled
        Constraint::Length(5),  // Models
        Constraint::Length(3),  // Buttons
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(form_area);

    let mut idx = 0;

    // 标题
    let title_p = Paragraph::new(Span::styled(title, Style::new().bold().fg(Color::White)))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title_p, chunks[idx]); idx += 1;

    // ID 字段
    render_text_field(frame, chunks[idx], "ID", &state.form_id, state.form_focus == 0, false);
    idx += 1;

    // Name 字段
    render_text_field(frame, chunks[idx], "Name", &state.form_name, state.form_focus == 1, false);
    idx += 1;

    // Base URL 字段
    render_text_field(frame, chunks[idx], "Base URL", &state.form_url, state.form_focus == 2, false);
    idx += 1;

    // API Key 字段
    let masked = if state.form_key.is_empty() { String::new() } else { "********".to_string() };
    render_text_field(frame, chunks[idx], "API Key", &masked, state.form_focus == 3, false);
    idx += 1;

    // Endpoint 字段
    render_dropdown_field(frame, chunks[idx], "Endpoint", ENDPOINT_OPTIONS[state.form_ep_idx], state.form_focus == 4);
    idx += 1;

    // Endpoint 下拉展开项
    if state.form_ep_open {
        render_dropdown_options(frame, chunks[idx], state);
        idx += 1;
    }

    // Enabled 字段
    render_toggle_field(frame, chunks[idx], "Enabled", state.form_enabled, state.form_focus == if state.form_ep_open { 6 } else { 5 });
    idx += 1;

    // Models 字段
    render_multiline_field(frame, chunks[idx], "Models", &state.form_models, state.form_focus == if state.form_ep_open { 7 } else { 6 });
    idx += 1;

    // 按钮行
    render_form_buttons(frame, chunks[idx], state);
}
```

- [ ] **Step 4.2: 各字段渲染辅助函数**

```rust
fn render_text_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    _masked: bool,
) {
    let block = Block::default()
        .title(format!(" {} ", label))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(if focused {
            Style::new().fg(Color::Rgb(108, 106, 255))
        } else {
            Style::new().fg(Color::DarkGray)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let p = Paragraph::new(Text::from(value.clone()))
        .style(if focused {
            Style::new().fg(Color::White).bg(Color::Rgb(14, 15, 30))
        } else {
            Style::default()
        });
    frame.render_widget(p, inner);
}

fn render_dropdown_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
) {
    let block = Block::default()
        .title(format!(" {} ", label))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(if focused {
            Style::new().fg(Color::Rgb(108, 106, 255))
        } else {
            Style::new().fg(Color::DarkGray)
        });
    let inner = block.inner(area);

    let text = Line::from(vec![
        Span::styled(value, Style::new().fg(Color::White)),
        Span::raw("  "),
        Span::styled("▶", Style::new().fg(Color::DarkGray)),
    ]);

    frame.render_widget(block, area);
    let p = Paragraph::new(text);
    frame.render_widget(p, inner);
}

fn render_dropdown_options(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let items: Vec<ListItem> = ENDPOINT_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let prefix = if i == state.form_ep_idx { " ● " } else { "   " };
            ListItem::new(Line::from(Span::raw(format!("{}{}", prefix, opt))))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Rgb(108, 106, 255))),
        )
        .highlight_style(Style::new().bg(Color::Rgb(108, 106, 255)).fg(Color::White));

    frame.render_widget(list, area);
}

fn render_toggle_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: bool,
    focused: bool,
) {
    let block = Block::default()
        .title(format!(" {} ", label))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(if focused {
            Style::new().fg(Color::Rgb(108, 106, 255))
        } else {
            Style::new().fg(Color::DarkGray)
        });
    let inner = block.inner(area);

    let text = if value {
        Line::from(Span::raw("  [x] Yes"))
    } else {
        Line::from(Span::raw("  [ ] No"))
    };

    frame.render_widget(block, area);
    let p = Paragraph::new(text);
    frame.render_widget(p, inner);
}

fn render_multiline_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
) {
    let block = Block::default()
        .title(format!(" {} (id:tier:ctx) ", label))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(if focused {
            Style::new().fg(Color::Rgb(108, 106, 255))
        } else {
            Style::new().fg(Color::DarkGray)
        });
    let inner = block.inner(area);

    let text = if value.is_empty() {
        Text::from(Line::from(Span::styled(
            "  model_id:T1:128000",
            Style::new().fg(Color::DarkGray),
        )))
    } else {
        Text::from(value.clone())
    };

    frame.render_widget(block, area);
    let p = Paragraph::new(text);
    frame.render_widget(p, inner);
}

fn render_form_buttons(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let cancel_block = Block::default()
        .title(" Cancel ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::DarkGray));
    frame.render_widget(cancel_block, chunks[0]);

    let save_block = Block::default()
        .title(" Save ")
        .borders(Borders::ALL)
        .border_style(if state.form_focus == get_last_focus(state) {
            Style::new().fg(Color::Rgb(108, 106, 255))
        } else {
            Style::new().fg(Color::DarkGray)
        });
    frame.render_widget(save_block, chunks[1]);
}

fn get_last_focus(state: &RouterTuiState) -> usize {
    if state.form_ep_open { 8 } else { 7 }
}
```

- [ ] **Step 4.3: 表单键盘处理**

```rust
fn handle_form_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    // 下拉展开时特殊处理
    if state.form_ep_open {
        match key.code {
            KeyCode::Esc => {
                state.form_ep_open = false;
                return false;
            }
            KeyCode::Enter => {
                state.form_ep_open = false;
                return false;
            }
            KeyCode::Down => {
                state.form_ep_idx = (state.form_ep_idx + 1).min(ENDPOINT_OPTIONS.len() - 1);
                return false;
            }
            KeyCode::Up => {
                state.form_ep_idx = state.form_ep_idx.saturating_sub(1);
                return false;
            }
            _ => return false,
        }
    }

    let max_focus = 7; // 0-based, last is Models field

    match key.code {
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                state.form_focus = state.form_focus.saturating_sub(1);
            } else {
                state.form_focus = (state.form_focus + 1).min(max_focus);
                // 如果跳过 endpoint 下拉区域，加 1
                if state.form_ep_open && state.form_focus == 5 {
                    state.form_focus = 6;
                }
            }
        }
        KeyCode::Char(c) => {
            // 文本输入
            match state.form_focus {
                0 => state.form_id.push(c),       // ID
                1 => state.form_name.push(c),     // Name
                2 => state.form_url.push(c),      // Base URL
                3 => if !c.is_control() { state.form_key.push(c) }, // API Key
                _ => {}
            }
        }
        KeyCode::Backspace => {
            match state.form_focus {
                0 => { state.form_id.pop(); }
                1 => { state.form_name.pop(); }
                2 => { state.form_url.pop(); }
                3 => { state.form_key.pop(); }
                _ => {}
            }
        }
        KeyCode::Enter => {
            match state.form_focus {
                4 => state.form_ep_open = !state.form_ep_open, // 切换下拉
                6 => state.form_models.push('\n'), // Models 换行
                7 => {
                    // Save 按钮
                    save_form(state);
                    state.view = View::List;
                }
                _ => {}
            }
        }
        KeyCode::Esc => {
            state.view = View::List;
        }
        KeyCode::F(2) => {
            save_form(state);
            state.view = View::List;
        }
        KeyCode::Char(' ') => {
            if state.form_focus == 5 {
                state.form_enabled = !state.form_enabled;
            }
        }
        KeyCode::Up => {
            if state.form_focus == 5 && state.form_ep_open {
                state.form_ep_idx = state.form_ep_idx.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if state.form_focus == 4 && state.form_ep_open {
                state.form_ep_idx = (state.form_ep_idx + 1).min(ENDPOINT_OPTIONS.len() - 1);
            }
        }
        _ => {}
    }
    false
}
```

- [ ] **Step 4.4: 保存 / 删除 / 持久化**

```rust
fn save_form(state: &mut RouterTuiState) {
    let id = state.form_id.trim().to_string();
    let name = state.form_name.trim().to_string();
    let url = state.form_url.trim().to_string();
    let ep = ENDPOINT_OPTIONS[state.form_ep_idx].to_string();
    let key = state.form_key.clone();
    let enabled = state.form_enabled;

    if id.is_empty() || name.is_empty() || url.is_empty() {
        return; // 简单校验
    }

    let models: Vec<ModelInfo> = state
        .form_models
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.trim().split(':').collect();
            ModelInfo {
                id: parts.first().unwrap_or(&"model").to_string(),
                name: String::new(),
                context_window: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(128000),
                reasoning: parts.get(3).map(|&s| s == "reasoning").unwrap_or(false),
                tier: parts.get(1).unwrap_or(&"T2").to_string().to_uppercase(),
                enabled: true,
                thinking_level_map: None,
            }
        })
        .collect();

    match state.form_mode {
        FormMode::Add => {
            let p = ProviderInfo {
                id: id.clone(),
                name: name.clone(),
                base_url: url.clone(),
                api_key: key,
                endpoint_type: ep,
                enabled,
                models,
            };
            state.providers.push(p);
            state.selected = state.providers.len() - 1;
        }
        FormMode::Edit(idx) => {
            if let Some(p) = state.providers.get_mut(idx) {
                p.id = id;
                p.name = name;
                p.base_url = url;
                if !key.is_empty() {
                    p.api_key = key;
                }
                p.endpoint_type = ep;
                p.enabled = enabled;
                p.models = models;
            }
        }
    }

    // 同步到共享配置 + 写磁盘
    save_config_sync(state);
}

fn delete_provider(state: &mut RouterTuiState) {
    if state.providers.is_empty() {
        return;
    }
    state.providers.remove(state.selected);
    if state.selected >= state.providers.len() {
        state.selected = state.providers.len().saturating_sub(1);
    }
    state.expanded = None;
    save_config_sync(state);
}

fn save_config_sync(state: &mut RouterTuiState) {
    let config = ProviderConfig {
        port: state.router_port,
        current_model: None,
        current_provider: None,
        providers: state.providers.clone(),
    };

    // 写磁盘
    ProviderConfig::save(&state.config_path, &config);

    // 同步到共享配置
    let shared = state.shared_config.clone();
    let providers = state.providers.clone();
    let port = state.router_port;
    tokio::spawn(async move {
        let mut cfg = shared.write().await;
        cfg.providers = providers;
        cfg.port = port;
    });
}
```

- [ ] **Step 4.5: 编译验证**

```bash
cargo build 2>&1
```

---

### Task 5: Router 背景任务管理

- [ ] **Step 5.1: 启动 / 停止 Router 的函数**

```rust
use futures::FutureExt;

async fn start_router_background(state: &mut RouterTuiState) {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    state.router_shutdown = Some(tx);

    let config = state.shared_config.clone();
    let port = state.router_port;

    state.router_running = true;

    tokio::spawn(async move {
        let shutdown_signal = rx.map(|_| ());
        match crate::provider::router::start_router(
            config,
            Some(Box::pin(shutdown_signal)),
        )
        .await
        {
            Ok(actual_port) => {
                tracing::info!("Router started on port {}", actual_port);
            }
            Err(e) => {
                tracing::error!("Router failed: {}", e);
            }
        }
    });
}

async fn stop_router(state: &mut RouterTuiState) {
    if let Some(tx) = state.router_shutdown.take() {
        let _ = tx.send(());
    }
    state.router_running = false;
}
```

- [ ] **Step 5.2: 编译 + 运行验证**

```bash
cargo build 2>&1
# dry-run 验证
cargo run -- --dry-run 2>/dev/null | head -5
```
