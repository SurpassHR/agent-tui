//! Router 配置管理 TUI
//!
//! 轻量 Ratatui 应用，管理 Provider/Model 配置，Router 在背景运行。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use futures::FutureExt;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;
use tokio::sync::RwLock;

use crate::provider::{
    self, ModelInfo, ProviderConfig, ProviderInfo, SharedConfig,
};

/// Endpoint 类型选项
const ENDPOINT_OPTIONS: &[&str] = &[
    "openai_compat",
    "openai_responses",
    "anthropic_messages",
    "gemini",
];


// ── 主题色 ──
const C_ACCENT: Color = Color::Rgb(108, 106, 255);
const C_GREEN: Color = Color::Rgb(52, 211, 153);
const C_RED: Color = Color::Rgb(239, 68, 68);
const C_PURPLE: Color = Color::Rgb(167, 139, 250);
const C_BLUE: Color = Color::Rgb(96, 165, 250);
const C_YELLOW: Color = Color::Rgb(251, 191, 36);
const C_FG: Color = Color::Rgb(226, 227, 240);
const C_FG_D: Color = Color::Rgb(84, 85, 110);
const C_BG: Color = Color::Rgb(20, 21, 36);
const C_BG_ALT: Color = Color::Rgb(26, 27, 46);
const C_BG_SEL: Color = Color::Rgb(29, 30, 56);
/// 视图模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum View {
    /// Provider 列表
    List,
    /// 新增/编辑表单
    Form,
}

/// 表单模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum FormMode {
    /// 新增
    Add,
    /// 编辑指定索引的 provider
    Edit(usize),
}

/// Router TUI 主状态
struct RouterTuiState {
    // ── Provider 数据 ──
    providers: Vec<ProviderInfo>,
    selected: usize,
    expanded: Option<usize>,

    // ── 视图切换 ──
    view: View,

    // ── 表单字段 ──
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

    // ── Router 背景任务 ──
    router_running: bool,
    router_port: u16,
    router_shutdown: Option<tokio::sync::oneshot::Sender<()>>,

    // ── 配置持久化 ──
    config_path: PathBuf,
    shared_config: SharedConfig,

    // ── 界面 ──
    request_count: u64,
}

impl RouterTuiState {
    fn new() -> Self {
        let config_path = provider::config_path();
        let provider_cfg = ProviderConfig::load(&config_path);
        let port = provider_cfg.port;
        let shared_config = Arc::new(RwLock::new(provider_cfg.clone()));

        Self {
            providers: provider_cfg.providers,
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
        }
    }
}

// ============================================================
//  公开 API
// ============================================================

/// 运行 Router 配置 TUI
///
/// 初始化终端 → 启动 Router 背景任务 → 事件循环 → 退出时停止 Router + 恢复终端
pub async fn run_router_tui() -> Result<(), crate::errors::Error> {
    let mut state = RouterTuiState::new();

    // 启动 Router 背景任务
    start_router_background(&mut state).await;

    // 初始化终端
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);

    // 事件循环
    let res = run_event_loop(&mut terminal, &mut state).await;

    // 停止 Router
    stop_router(&mut state).await;

    // 恢复终端
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = ratatui::try_restore();

    res
}

// ============================================================
//  事件循环
// ============================================================

async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    state: &mut RouterTuiState,
) -> Result<(), crate::errors::Error> {
    loop {
        terminal.draw(|f| render(f, state))?;

        if !crossterm::event::poll(Duration::from_millis(100))? {
            continue;
        }

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_key(key, state) {
                break;
            }
        }
    }
    Ok(())
}

// ============================================================
//  渲染入口
// ============================================================

fn render(frame: &mut Frame, state: &mut RouterTuiState) {
    let area = frame.area();
    let content_size = compute_content_size(state);

    let (h_fill, cw, h_area) = if let Some((cw, ch)) = content_size {
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Fill(1),
                Constraint::Max(cw.min(area.width)),
                Constraint::Fill(1),
            ])
            .split(area);
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Max(ch.min(area.height)),
                Constraint::Fill(1),
            ])
            .split(h[1]);
        (v[1].x, cw, v[1])
    } else {
        (area.x, area.width, area)
    };

    let outer_rect = Rect { x: h_fill, y: h_area.y, width: cw.min(area.width), height: h_area.height };
    let outer_block = Block::default()
        .title(" Provider Router ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(C_ACCENT));
    let inner_area = outer_block.inner(outer_rect);
    frame.render_widget(outer_block, outer_rect);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 顶部状态栏
            Constraint::Min(0),    // 内容区
            Constraint::Length(1), // 底部快捷键栏
        ])
        .split(inner_area);

    render_top_bar(frame, chunks[0], state);
    match state.view {
        View::List => render_list_view(frame, chunks[1], state),
        View::Form => render_form_view(frame, chunks[1], state),
    }
    render_bottom_bar(frame, chunks[2], state);
}

/// 根据当前视图和数据计算内容应有的宽高
fn compute_content_size(state: &RouterTuiState) -> Option<(u16, u16)> {
    match state.view {
        View::Form => None,
        View::List => {
            let mut max_w = 0u16;

            let top = format!(
                " Provider Router {} 127.0.0.1:{}  {} mode  proxied: {}",
                if state.router_running { "● Running" } else { "● Stopped" },
                state.router_port,
                "LIST",
                state.request_count,
            );
            max_w = max_w.max(top.chars().count() as u16);

            for p in &state.providers {
                let model_text = format!(
                    " {} model{}",
                    p.models.len(),
                    if p.models.len() == 1 { "" } else { "s" },
                );
                let line_len = 3 + 4 + p.name.chars().count() + 2
                    + p.endpoint_type.chars().count() + 2 + 9 + 2
                    + model_text.chars().count() + 2;
                max_w = max_w.max(line_len as u16);
            }

            let keys = "  ↑↓ Navigate  Enter Expand  Space Toggle  a Add  e Edit  d Delete  q Quit";
            max_w = max_w.max(keys.chars().count() as u16);

            if state.providers.is_empty() {
                max_w = max_w.max(
                    "  No providers configured. Press 'a' to add one.".chars().count() as u16
                );
            }

            let content_w = max_w + 4;

            let mut h = 0u16;
            h += 2; // 边框上下
            h += 1; // 顶部状态栏
            h += 1; // title Providers (N)
            if state.providers.is_empty() {
                h += 1;
            } else {
                for (i, p) in state.providers.iter().enumerate() {
                    h += 1;
                    if state.expanded == Some(i) {
                        h += p.models.len() as u16;
                    }
                }
            }
            h += 1; // bottom bar

            Some((content_w, h))
        }
    }
}

// ============================================================
//  键盘调度
// ============================================================

fn handle_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    match state.view {
        View::List => handle_list_key(key, state),
        View::Form => handle_form_key(key, state),
    }
}

// ============================================================
//  顶部状态栏
// ============================================================

fn render_top_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let status = if state.router_running {
        Span::styled(" ● Running", Style::new().fg(C_GREEN))
    } else {
        Span::styled(" ● Stopped", Style::new().fg(C_RED))
    };

    let view_mode = match state.view {
        View::List => "LIST",
        View::Form => "EDIT",
    };

    let proxied = format!("proxied: {}", state.request_count);

    let line = Line::from(vec![
        Span::styled(" Provider Router", Style::new().bold().fg(C_FG)),
        Span::raw(" "),
        status,
        Span::styled(format!(" 127.0.0.1:{}", state.router_port), Style::new().fg(C_FG_D)),
        Span::raw("  "),
        Span::styled(format!("{} mode", view_mode), Style::new().fg(C_FG_D)),
        Span::raw("  "),
        Span::styled(proxied, Style::new().fg(C_FG_D)),
    ]);

    let p = Paragraph::new(line).style(Style::new().bg(C_BG));
    frame.render_widget(p, area);
}

// ============================================================
//  底部快捷键栏
// ============================================================

fn render_bottom_bar(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let (keys, right) = match state.view {
        View::List => (
            format!("  {} Navigate  {} Expand  {} Add  {} Edit  {} Delete  {} Quit",
                "↑↓", "Enter", "a", "e", "d", "q"),
            format!("provider mode · {} provider{}", state.providers.len(),
                if state.providers.len() == 1 { "" } else { "s" }),
        ),
        View::Form => (
            "  Tab Next  Shift+Tab Prev  Enter Dropdown  Space Toggle  Esc Cancel  F2 Save".to_string(),
            "INSERT mode".to_string(),
        ),
    };

    let max_w = area.width as usize;
    let padding = max_w.saturating_sub(keys.len() + right.len() + 1);
    let full = format!("{}{:padding$}", keys, right, padding = padding);
    let p = Paragraph::new(full).style(Style::new().bg(C_BG).fg(C_FG_D));
    frame.render_widget(p, area);
}

// ============================================================
//  列表视图（TODO: Task 3 实现）
// ============================================================


/// 获取 provider 图标的标签和背景色
fn provider_icon(id: &str) -> (String, Color) {
    let label: String = id.chars().take(2).map(|c| c.to_ascii_uppercase()).collect();
    // Deterministic color from ID
    let hash: u32 = id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let colors = [
        Color::Rgb(26, 47, 74),   // blue
        Color::Rgb(13, 40, 24),   // green  
        Color::Rgb(30, 16, 46),   // purple
        Color::Rgb(14, 31, 46),   // teal
        Color::Rgb(46, 26, 26),   // maroon
    ];
    let bg = colors[hash as usize % colors.len()];
    (label, bg)
}
fn render_list_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // 标题行
    let title = format!(" Providers ({})", state.providers.len());
    let title_p = Paragraph::new(Span::styled(&title, Style::new().fg(C_FG_D).add_modifier(Modifier::BOLD)));
    frame.render_widget(title_p, chunks[0]);

    if state.providers.is_empty() {
        let empty = Paragraph::new("  No providers configured. Press 'a' to add one.")
            .style(Style::new().fg(C_FG_D))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    // 修正选中索引
    let sel = if state.selected >= state.providers.len() { 0 } else { state.selected };

    let mut items: Vec<ListItem> = Vec::new();

    for (i, p) in state.providers.iter().enumerate() {
        let selected = i == sel;
        let (icon_label, icon_bg) = provider_icon(&p.id);

        let prefix = if selected { " ● " } else { " ○ " };
        let sel_color = if selected { C_ACCENT } else { C_FG_D };

        let enabled_badge = if p.enabled {
            Span::styled(" ENABLED ", Style::new().fg(C_GREEN))
        } else {
            Span::styled(" DISABLED ", Style::new().fg(C_RED))
        };

        let model_text = format!(" {} model{}", p.models.len(), if p.models.len() == 1 { "" } else { "s" });
        let arrow = if state.expanded == Some(i) { " ▼" } else { " ▶" };

        let header = Line::from(vec![
            Span::styled(prefix, Style::new().fg(sel_color)),
            Span::styled(format!(" {} ", icon_label), Style::new().fg(C_FG).bg(icon_bg)),
            Span::raw(" "),
            Span::styled(&p.name, Style::new().bold().fg(C_FG)),
            Span::raw("  "),
            Span::styled(&p.endpoint_type, Style::new().fg(C_FG_D)),
            Span::raw("  "),
            enabled_badge,
            Span::raw("  "),
            Span::styled(model_text, Style::new().fg(C_FG_D)),
            Span::styled(arrow, Style::new().fg(C_FG_D)),
        ]);

        let item_style = if selected {
            Style::new().bg(C_BG_SEL)
        } else {
            Style::new().bg(C_BG)
        };

        items.push(ListItem::new(header).style(item_style));

        // Expanded models
        if state.expanded == Some(i) {
            for m in &p.models {
                let tier_style = match m.tier.as_str() {
                    "T1" => Style::new().fg(C_PURPLE).bg(C_BG_ALT),
                    "T2" => Style::new().fg(C_BLUE).bg(C_BG_ALT),
                    _ => Style::new().fg(C_FG_D).bg(C_BG_ALT),
                };
                let ctx = if m.context_window >= 1_000_000 {
                    format!(" {:.1}M ctx", m.context_window as f64 / 1_000_000.0)
                } else if m.context_window >= 1000 {
                    format!(" {}K ctx", m.context_window / 1000)
                } else {
                    format!(" {} ctx", m.context_window)
                };
                let reasoning = if m.reasoning {
                    Span::styled(" reasoning", Style::new().fg(C_YELLOW))
                } else {
                    Span::raw("")
                };
                let model_line = Line::from(vec![
                    Span::raw("     "),
                    Span::styled(&m.id, Style::new().fg(C_FG)),
                    Span::raw("  "),
                    Span::styled(format!(" {} ", m.tier), tier_style),
                    Span::styled(ctx, Style::new().fg(C_FG_D)),
                    reasoning,
                ]);
                items.push(ListItem::new(model_line));
            }
        }
    }

    let list = List::new(items);
    frame.render_widget(list, chunks[1]);
}

// ============================================================
//  表单视图（TODO: Task 4 实现）
// ============================================================


fn render_form_view(frame: &mut Frame, area: Rect, state: &RouterTuiState) {
    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(1), // ID
        Constraint::Length(1), // Name
        Constraint::Length(1), // URL
        Constraint::Length(1), // Key
        Constraint::Length(1), // EP button
    ];
    if state.form_ep_open {
        constraints.push(Constraint::Length(ENDPOINT_OPTIONS.len() as u16 + 1));
    }
    constraints.push(Constraint::Length(1)); // Enabled
    constraints.push(Constraint::Length(5)); // Models
    constraints.push(Constraint::Min(0));    // spacer
    constraints.push(Constraint::Length(3)); // Buttons

    // 计算弹窗尺寸
    let content_height: u16 = constraints.iter().map(|c| match c {
        Constraint::Length(h) => *h,
        _ => 0,
    }).sum();
    let window_height = content_height + 2; // 上下边框
    let content_width = 62u16;
    let window_width = (content_width + 2).min(area.width.saturating_sub(4));

    // 垂直居中
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(window_height), Constraint::Min(0)])
        .split(area);
    // 水平居中
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(window_width), Constraint::Min(0)])
        .split(v_chunks[1]);

    let window_area = h_chunks[1];

    // 带边框的弹窗
    let title = match state.form_mode {
        FormMode::Add => " Add Provider ",
        FormMode::Edit(_) => " Edit Provider ",
    };
    let window_block = ratatui::widgets::Block::default()
        .title(title)
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(C_ACCENT));
    let inner = window_block.inner(window_area);
    frame.render_widget(window_block, window_area);

    // 内容区域分割
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut ci = 0;

    // ID
    render_form_field_line(frame, chunks[ci], "ID", &state.form_id, state.form_focus == 0, false);
    ci += 1;

    // Name
    render_form_field_line(frame, chunks[ci], "Name", &state.form_name, state.form_focus == 1, false);
    ci += 1;

    // Base URL
    render_form_field_line(frame, chunks[ci], "Base URL", &state.form_url, state.form_focus == 2, false);
    ci += 1;

    // API Key
    let key_display = if state.form_key.is_empty() { String::new() } else { "********".to_string() };
    render_form_field_line(frame, chunks[ci], "API Key", &key_display, state.form_focus == 3, false);
    ci += 1;

    // Endpoint
    let ep_text = format!("{} ▶", ENDPOINT_OPTIONS[state.form_ep_idx]);
    render_form_field_line(frame, chunks[ci], "Endpoint", &ep_text, state.form_focus == 4, false);
    ci += 1;

    // Dropdown options (if open)
    if state.form_ep_open {
        let opts: Vec<ListItem> = ENDPOINT_OPTIONS.iter().enumerate().map(|(i, opt)| {
            let prefix = if i == state.form_ep_idx { " ● " } else { "   " };
            ListItem::new(Line::from(Span::raw(format!("{}{}", prefix, opt))))
                .style(if i == state.form_ep_idx {
                    Style::new().bg(C_ACCENT).fg(Color::White)
                } else {
                    Style::default()
                })
        }).collect();
        let list = List::new(opts).block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Style::new().fg(C_ACCENT)),
        );
        frame.render_widget(list, chunks[ci]);
        ci += 1;
    }

    // Enabled
    let enabled_text = if state.form_enabled { "[x] Yes" } else { "[ ] No" };
    render_form_field_line(frame, chunks[ci], "Enabled", enabled_text, state.form_focus == 5, false);
    ci += 1;

    // Models
    render_models_field(frame, chunks[ci], &state.form_models, state.form_focus == 6);
    ci += 1;

    // Spacer
    ci += 1;

    // Buttons
    let btn_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[ci]);

    let cancel_block = ratatui::widgets::Block::default()
        .title(" Cancel (Esc) ")
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(Color::DarkGray));
    frame.render_widget(cancel_block, btn_chunks[0]);

    let save_style = if state.form_focus == 7 {
        Style::new().fg(C_ACCENT)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let save_block = ratatui::widgets::Block::default()
        .title(" Save (F2) ")
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(save_style);
    frame.render_widget(save_block, btn_chunks[1]);
}

// ============================================================
//  列表键盘处理
// ============================================================

fn handle_list_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    if state.providers.is_empty() {
        if let KeyCode::Char('a') = key.code {
            open_form_add(state);
        }
        return false;
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(state.providers.len() - 1);
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
        KeyCode::Char('a') => open_form_add(state),
        KeyCode::Char('e') => {
            if !state.providers.is_empty() {
                open_form_edit(state, state.selected);
            }
        }
        KeyCode::Char('d') => delete_provider(state),
        KeyCode::Char(' ') => {
            if let Some(p) = state.providers.get_mut(state.selected) {
                p.enabled = !p.enabled;
                save_config_sync(state);
            }
        }
        KeyCode::Char('q') | KeyCode::Esc => return true,
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
    state.form_key.clear();
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
            if m.reasoning { s.push_str(":reasoning"); }
            s
        })
        .collect::<Vec<_>>()
        .join("
");
    state.form_focus = 0;
}

fn delete_provider(state: &mut RouterTuiState) {
    if state.providers.is_empty() { return; }
    state.providers.remove(state.selected);
    if state.selected >= state.providers.len() {
        state.selected = state.providers.len().saturating_sub(1);
    }
    state.expanded = None;
    save_config_sync(state);
}

// ============================================================
//  表单键盘处理
// ============================================================

fn handle_form_key(key: crossterm::event::KeyEvent, state: &mut RouterTuiState) -> bool {
    if state.form_ep_open {
        match key.code {
            KeyCode::Esc => state.form_ep_open = false,
            KeyCode::Enter => state.form_ep_open = false,
            KeyCode::Up => state.form_ep_idx = state.form_ep_idx.saturating_sub(1),
            KeyCode::Down => {
                state.form_ep_idx = (state.form_ep_idx + 1).min(ENDPOINT_OPTIONS.len() - 1);
            }
            _ => {}
        }
        return false;
    }

    // Abstract field indices: ID(0), Name(1), URL(2), Key(3), EP(4), Enabled(5), Models(6), Buttons(7)
    let ep_focus = 4;
    let enabled_focus = 5;
    let buttons_focus = 7;

    match key.code {
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                state.form_focus = state.form_focus.saturating_sub(1);
            } else {
                state.form_focus = (state.form_focus + 1).min(buttons_focus);
            }
        }
        KeyCode::Char(c) if !c.is_ascii_control() => {
            match state.form_focus {
                0 => state.form_id.push(c),
                1 => state.form_name.push(c),
                2 => state.form_url.push(c),
                3 => state.form_key.push(c),
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
            if state.form_focus == ep_focus {
                state.form_ep_open = true;
            } else if state.form_focus == buttons_focus {
                save_form(state);
                state.view = View::List;
            }
        }
        KeyCode::Esc => { state.view = View::List; }
        KeyCode::F(2) => {
            save_form(state);
            state.view = View::List;
        }
        KeyCode::Char(' ') => {
            if state.form_focus == enabled_focus {
                state.form_enabled = !state.form_enabled;
            }
        }
        _ => {}
    }
    false
}

// ============================================================
//  表单渲染辅助函数
// ============================================================

fn render_form_field_line(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    _masked: bool,
) {
    let label_w: usize = 11;
    let inner_w = area.width as usize;
    let bracket_style = if focused { Style::new().fg(C_ACCENT) } else { Style::new().fg(C_FG_D) };

    let avail = inner_w.saturating_sub(label_w + 4);
    let display = if value.len() > avail {
        format!("..{}", &value[value.len().saturating_sub(avail.saturating_sub(2))..])
    } else {
        value.to_string()
    };
    let pad = avail.saturating_sub(display.len());

    let mut spans: Vec<Span> = vec![
        Span::styled(format!("{:>width$}:", label, width = label_w - 1), Style::new().fg(C_FG_D)),
        Span::raw(" "),
        Span::styled("[".to_string(), bracket_style),
        Span::styled(display, Style::new().fg(C_FG)),
    ];
    if pad > 0 { spans.push(Span::raw(" ".repeat(pad))); }
    spans.push(Span::styled("]".to_string(), bracket_style));

    let p = Paragraph::new(Line::from(spans));
    frame.render_widget(p, area);
}

fn render_models_field(frame: &mut Frame, area: Rect, value: &str, focused: bool) {
    let border_style = if focused { Style::new().fg(C_ACCENT) } else { Style::new().fg(C_FG_D) };

    let block = ratatui::widgets::Block::default()
        .title(" Models (id:tier:ctx) ")
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = if value.is_empty() {
        Text::from(Line::from(Span::styled("  model_id:T1:128000", Style::new().fg(C_FG_D))))
    } else {
        Text::from(value.to_string())
    };

    let p = Paragraph::new(text);
    frame.render_widget(p, inner);
}

fn save_form(state: &mut RouterTuiState) {
    let id = state.form_id.trim().to_string();
    let name = state.form_name.trim().to_string();
    let url = state.form_url.trim().to_string();
    let ep = ENDPOINT_OPTIONS[state.form_ep_idx].to_string();
    let key = state.form_key.clone();
    let enabled = state.form_enabled;

    if id.is_empty() || name.is_empty() || url.is_empty() {
        return;
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
                id,
                name,
                base_url: url,
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

    save_config_sync(state);
}

// ============================================================

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

//  Router 背景任务管理（TODO: Task 5 实现）
// ============================================================

async fn start_router_background(state: &mut RouterTuiState) {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    state.router_shutdown = Some(tx);

    let config = state.shared_config.clone();
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
