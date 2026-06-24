use std::io::Write;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, MouseButton, MouseEventKind};
use futures::StreamExt;
use ratatui::layout::Size;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::action::Action;
use crate::app::{AgentStatus, App, SelectionPanel};
use crate::backend::event::{AssistantEventType, PiEvent};
use crate::backend::rpc::PiRpcBackend;
use crate::errors::Result;
use crate::message::ToolStatus;

/// 将 PiEvent 翻译为一个或多个 Action
///
/// 对于 MessageUpdate，同时发出 delta 事件（打字机效果）和 ContentUpdate（块数组快照）。
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
                    tracing::debug!("MSG: text_delta");
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
                ref other => {
                    tracing::debug!("MSG: unhandled delta type {:?}", other);
                }
            }
            // 2. ContentUpdate（块数组快照）
            if let Some(ref data) = message {
                tracing::debug!("MSG: content_update {} blocks", data.content.len());
                actions.push(Action::ContentUpdate {
                    agent_id: agent_id.into(),
                    content: data.content.clone(),
                });
                // 若有错误信息，同时发出状态消息
                if let Some(ref err) = data.error_message {
                    actions.push(Action::AutoRetryStatus {
                        agent_id: agent_id.into(),
                        text: format!("错误: {}", err),
                    });
                }
            }
        }
        _ => {
            if let Some(a) = translate_single_action(event, agent_id) {
                actions.push(a);
            }
        }
    }
    actions
}

/// 处理非 MessageUpdate 事件的单 Action 翻译（保留原 translate_pi_event 逻辑）
fn translate_single_action(event: PiEvent, agent_id: &str) -> Option<Action> {
    match event {
        PiEvent::AgentStart => Some(Action::AgentStatusChange {
            agent_id: agent_id.into(),
            status: AgentStatus::Running,
        }),

        PiEvent::AgentEnd { .. } => Some(Action::AgentStatusChange {
            agent_id: agent_id.into(),
            status: AgentStatus::Idle,
        }),

        PiEvent::ToolExecutionStart {
            tool_name,
            tool_call_id,
            args,
        } => Some(Action::ToolEvent {
            agent_id: agent_id.into(),
            tool_name,
            tool_call_id,
            status: ToolStatus::Running,
            args: Some(args),
            result: None,
            is_error: false,
        }),

        PiEvent::ToolExecutionEnd {
            tool_name,
            tool_call_id,
            result,
            is_error,
            ..
        } => Some(Action::ToolEvent {
            agent_id: agent_id.into(),
            tool_name,
            tool_call_id,
            status: if is_error {
                ToolStatus::Error
            } else {
                ToolStatus::Done
            },
            args: None,
            result: Some(result),
            is_error,
        }),

        PiEvent::ExtensionError { error } => {
            let agent_id = agent_id.to_string();
            Some(Action::AutoRetryStatus {
                agent_id,
                text: format!("Extension error: {}", error),
            })
        }

        PiEvent::MessageEnd { message } => {
            let agent_id = agent_id.to_string();
            let text = message
                .as_ref()
                .and_then(|m| m.error_message.as_deref())
                .unwrap_or("(message end)");
            Some(Action::AutoRetryStatus {
                agent_id,
                text: format!("Message end: {}", text),
            })
        }

        PiEvent::AutoRetryStart {
            attempt,
            max_attempts,
            delay_ms,
            error_message,
        } => {
            let agent_id = agent_id.to_string();
            let err = error_message.as_deref().unwrap_or("unknown error");
            Some(Action::AutoRetryStatus {
                agent_id,
                text: format!(
                    "自动重试 {}/{} ({}ms 后): {}",
                    attempt, max_attempts, delay_ms, err
                ),
            })
        }

        PiEvent::AutoRetryEnd {
            success,
            final_error,
        } => {
            let agent_id = agent_id.to_string();
            if success {
                Some(Action::AutoRetryStatus {
                    agent_id,
                    text: "重试成功".to_string(),
                })
            } else {
                let err = final_error.as_deref().unwrap_or("unknown");
                Some(Action::AutoRetryStatus {
                    agent_id,
                    text: format!("全部重试失败: {}", err),
                })
            }
        }

        PiEvent::MessageStart { role } => {
            let agent_id = agent_id.to_string();
            Some(Action::AutoRetryStatus {
                agent_id,
                text: format!("消息开始: role={}", role),
            })
        }

        ref other => {
            tracing::debug!("EVENT: unhandled {:?}", std::mem::discriminant(other));
            None
        }
    }
}

/// Headless 主循环（不变，保持 PTY 模式）
#[allow(deprecated)]
pub async fn run_headless(mut app: App, mut action_rx: mpsc::Receiver<Action>) -> Result<()> {
    let mut stdout = std::io::stdout();
    let stdin_reader = BufReader::new(tokio::io::stdin());
    let mut lines = stdin_reader.lines();

    writeln!(
        stdout,
        "[Meta-TUI] Headless 模式已启动。输入指令，输入 exit 退出。"
    )?;
    stdout.flush()?;

    loop {
        tokio::select! {
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        let trimmed = line.trim().to_string();
                        if trimmed == "exit" || trimmed == "quit" {
                            writeln!(stdout, "[Meta-TUI] 正在退出...")?;
                            stdout.flush()?;
                            app.terminate().await?;
                            break;
                        }
                        app.handle_action(Action::UserSubmitInput(trimmed)).await?;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        writeln!(stdout, "[Meta-TUI] 读取输入错误: {}", e)?;
                        stdout.flush()?;
                        break;
                    }
                }
            }

            Some(action) = action_rx.recv() => {
                match action {
                    Action::PtyStdout(text) => {
                        write!(stdout, "{}", text)?;
                        stdout.flush()?;
                    }
                    Action::PtyExit => {
                        writeln!(stdout, "\n[Meta-TUI] 子进程已退出。")?;
                        stdout.flush()?;
                        break;
                    }
                    _ => {
                        tracing::debug!("主循环收到未处理的 action: {:?}", action);
                    }
                }
            }

            else => break,
        }
    }

    let _ = app.terminate().await;
    writeln!(stdout, "[Meta-TUI] Headless 模式已退出。")?;
    stdout.flush()?;
    Ok(())
}

/// TUI 主循环（RPC 模式）
///
/// 使用 PiRpcBackend + PiRpcClient + crossterm EventStream。
pub async fn run_tui(mut app: App, _action_rx: mpsc::Receiver<Action>) -> Result<()> {
    // 先检测 pi 是否安装（在 ratatui::init 之前，不干扰终端）
    let (pi_installed, pi_version) = PiRpcBackend::check_installed().await;
    let mut terminal = ratatui::init();
    // 启用鼠标事件捕获（选区拖拽）
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);

    if !pi_installed {
        tracing::error!("pi CLI 未安装或不在 PATH 中");
        app.agent_status = AgentStatus::Error;
        app.push_message("default", crate::message::ChatMessage::system(
            "default",
            "❌ pi CLI 未安装或不在 PATH 中\n\n请先安装 pi:\n  npm install -g @earendil-works/pi-coding-agent\n\n或使用 --pi-path 指定路径",
        ));
        while !check_ctrl_c().await {
            let _ = terminal.try_draw(|f| {
                app.render_tui(f);
                Ok::<_, std::io::Error>(())
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        let _ = ratatui::try_restore();
        return Ok(());
    }

    tracing::info!("pi 已安装: {:?}", pi_version);

    // ============================================================
    // 先启动 Provider Router + 生成 local-provider.ts
    // 必须在 pi 启动之前完成，否则 pi 加载扩展时找不到 local provider
    // ============================================================
    let (router_port, shared_config) = {
        let config_path = crate::provider::config_path();
        let provider_cfg = crate::provider::ProviderConfig::load(&config_path);
        app.tui.providers.clone_from(&provider_cfg.providers);
        if let Some(ref m) = provider_cfg.current_model {
            app.tui.current_model.clone_from(m);
        }
        let initial_port = provider_cfg.port;
        let shared = crate::provider::SharedConfig::new(tokio::sync::RwLock::new(provider_cfg));

        // 生成 local-provider.ts（必须在 pi 启动前写入）
        let ts_content = crate::provider::generate_local_provider_ts(
            &*shared.read().await,
            shared.read().await.port,
        );
        let pi_home = std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".pi").join("agent"))
                    .unwrap_or_default()
            });
        let ext_dir = pi_home.join("extensions");
        let _ = std::fs::create_dir_all(&ext_dir);
        let _ = std::fs::write(ext_dir.join("local-provider.ts"), &ts_content);
        let model_count: usize = shared
            .read()
            .await
            .providers
            .iter()
            .map(|p| p.models.len())
            .sum();
        tracing::info!("local-provider.ts generated with {} models", model_count);

        // 启动 axum router
        let router_shared = shared.clone();
        let (port_tx, mut port_rx) = tokio::sync::oneshot::channel::<u16>();
        app.tui.router_running = true;
        tokio::spawn(async move {
            match crate::provider::router::start_router(router_shared).await {
                Ok(actual_port) => {
                    tracing::info!("Provider router started on port {}", actual_port);
                    let _ = port_tx.send(actual_port);
                }
                Err(e) => {
                    tracing::error!("Provider router failed: {}", e);
                }
            }
        });
        // 给 axum 一点时间绑定端口
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let actual_port = match port_rx.try_recv() {
            Ok(p) => p,
            _ => initial_port,
        };
        (actual_port, shared)
    };

    app.tui.router_port = router_port;
    app.tui.shared_config = Some(shared_config);

    // 自检：验证 router 是否可达
    {
        let test_url = format!("http://127.0.0.1:{}/v1/models", router_port);
        match reqwest::get(&test_url).await {
            Ok(resp) => {
                tracing::info!(
                    "Router 自检通过: GET /v1/models → status={}",
                    resp.status().as_u16()
                );
            }
            Err(e) => {
                tracing::error!("Router 自检失败: {} — pi 将无法使用 TUI 路由", e);
            }
        }
    }

    // ============================================================
    // 启动 pi（此时 local-provider.ts 已就绪）
    // ============================================================
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut backend = PiRpcBackend::new("pi", &cwd);
    let mut client = match backend.start(None).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to start pi: {}", e);
            app.agent_status = AgentStatus::Error;
            app.push_message(
                "default",
                crate::message::ChatMessage::system(
                    "default",
                    &format!("❌ pi RPC 后端启动失败: {}", e),
                ),
            );
            while !check_ctrl_c().await {
                let _ = terminal.try_draw(|f| {
                    app.render_tui(f);
                    Ok::<_, std::io::Error>(())
                });
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            let _ = ratatui::try_restore();
            return Ok(());
        }
    };

    app.agent_status = AgentStatus::Idle;

    // 启动后立即发送初始模型（否则 pi 不知道用哪个模型）
    let initial_model = app.tui.current_model.clone();
    if !initial_model.is_empty() {
        tracing::info!("发送初始模型: {}", initial_model);
        let _ = client
            .notify(serde_json::json!({
                "type": "set_model",
                "provider": "local",
                "modelId": initial_model,
            }))
            .await;
    } else {
        tracing::warn!("无初始模型 — 请在 MODEL 区选择一个模型");
    }

    // 扫描 session 目录，填充工作区数据
    app.populate_workspaces();
    // 扫描 agents 目录，填充 subagent 列表
    app.populate_subagents();
    // 扫描 skills 目录，填充 skill 列表
    app.populate_skills();
    // 扫描 mcp.json，填充 MCP server 列表
    app.populate_mcps();

    let agent_id = app.active_agent.clone().unwrap_or_default();
    tracing::info!("TUI 主循环开始，active_agent={:?}", agent_id);
    let mut input_buffer = String::new();

    // 焦点状态由 App 通过 focus_panel 管理
    app.tui.main_view.has_focus = true;
    app.tui.focus_panel = crate::app::FocusPanel::MainView;

    // crossterm 异步事件流（不依赖 spawn_blocking，窗口切换后仍可靠）
    let mut event_stream = crossterm::event::EventStream::new();

    // 定时刷新 + 状态轮询
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    let mut state_poll_ticker = tokio::time::interval(Duration::from_secs(5));

    tracing::debug!("TUI 主循环开始");

    loop {
        tokio::select! {
            // 定时刷新
            _ = ticker.tick() => {
                // 处理 pi 事件（支持多 Action 返回，如 ContentUpdate + delta）
                while let Ok(event) = client.event_rx.try_recv() {
                    tracing::debug!("EVENT: {:?}", std::mem::discriminant(&event));
                    let actions = translate_pi_events(event, &agent_id);
                    tracing::debug!("  -> {} actions", actions.len());
                    for (i, action) in actions.into_iter().enumerate() {
                        tracing::debug!("  ACTION[{}]: {:?}", i, std::mem::discriminant(&action));
                        if let Err(e) = app.handle_action(action).await {
                            tracing::error!("handle_action error: {}", e);
                        }
                    }
                }
                // 同步输入缓冲区到 main_view
                app.tui.main_view.input_buffer.clone_from(&input_buffer);
                // 检查模型列表拉取结果
                if let Some(mut rx) = app.tui.models_fetch_rx.take() {
                    match rx.try_recv() {
                        Ok(Some(models_text)) => {
                            if let Some(ref mut editor) = app.tui.provider_editor {
                                editor.models_text = models_text;
                                editor.models_fetching = false;
                            }
                        }
                        Ok(None) => {
                            if let Some(ref mut editor) = app.tui.provider_editor {
                                editor.models_fetching = false;
                            }
                        }
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                            app.tui.models_fetch_rx = Some(rx);
                        }
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            if let Some(ref mut editor) = app.tui.provider_editor {
                                editor.models_fetching = false;
                            }
                        }
                    }
                }
                // 渲染
                let _ = terminal.try_draw(|f| {
                    app.render_tui(f);
                    Ok::<_, std::io::Error>(())
                });
            }

            // 状态轮询（5 秒一次）
            _ = state_poll_ticker.tick() => {
                tracing::debug!("状态轮询触发");
                let _ = client.request(
                    serde_json::json!({"type": "get_state"}), Duration::from_secs(5),
                ).await.map(|resp| {
                    if let Some(data) = &resp.data {
                        if let Some(model) = data.get("model") {
                            app.runtime.model_name = model.get("name").and_then(|v| v.as_str()).map(String::from);
                            app.runtime.provider = model.get("provider").and_then(|v| v.as_str()).map(String::from);
                        }
                        app.session.id = data.get("sessionId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        app.session.file_path = data.get("sessionFile").and_then(|v| v.as_str()).map(String::from);
                        app.session.name = data.get("sessionName").and_then(|v| v.as_str()).map(String::from);
                    }
                });
                let _ = client.request(
                    serde_json::json!({"type": "get_session_stats"}), Duration::from_secs(5),
                ).await.map(|resp| {
                    if let Some(data) = &resp.data {
                        if let Some(tokens) = data.get("tokens") {
                            app.runtime.cache_read = tokens.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            app.runtime.cache_write = tokens.get("cacheWrite").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            app.runtime.cost = tokens.get("cost").and_then(|v| v.get("total")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        }
                        if let Some(context) = data.get("contextUsage") {
                            app.runtime.context_tokens = context.get("tokens").and_then(|v| v.as_u64()).map(|v| v as u32);
                            app.runtime.context_window = context.get("contextWindow").and_then(|v| v.as_u64()).map(|v| v as u32);
                            app.runtime.context_percent = context.get("percent").and_then(|v| v.as_f64()).map(|v| v as f32);
                        }
                    }
                });
                let _ = app.handle_action(Action::RuntimeStateUpdate(app.runtime.clone())).await;
            }

            // 键盘 & 鼠标事件（EventStream 是异步的，不阻塞）
            Some(Ok(event)) = event_stream.next() => {
                tracing::debug!("收到事件: {:?}", event);

                // ── 鼠标事件处理（拖拽选中） ──
                if let crossterm::event::Event::Mouse(mouse) = event {
                    let sel = &mut app.tui.selection;
                    let term_size = terminal.size().unwrap_or(Size::new(80, 24));
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let bounds = (
                                // sidebar 右边界固定 40
                                39u16,
                                // agent 左边界 = terminal_width - 40
                                term_size.width.saturating_sub(40),
                            );
                            let panel = if mouse.column <= bounds.0 {
                                SelectionPanel::Sidebar
                            } else if mouse.column >= bounds.1 {
                                SelectionPanel::AgentPanel
                            } else {
                                SelectionPanel::Content
                            };
                            *sel = crate::app::SelectionState {
                                panel,
                                active: true,
                                anchor: Some((mouse.column, mouse.row)),
                                focus: Some((mouse.column, mouse.row)),
                                selected_text: String::new(),
                            };
                            app.tui.bottom_bar.status = "选中中...".into();
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if sel.active {
                                // 限制不超出所选栏的边界
                                let bounds = (
                                    39u16,
                                    term_size.width.saturating_sub(40),
                                );
                                let col = match sel.panel {
                                    SelectionPanel::Sidebar => {
                                        mouse.column.min(bounds.0)
                                    }
                                    SelectionPanel::Content => {
                                        mouse.column.max(bounds.0.saturating_add(1)).min(bounds.1.saturating_sub(1))
                                    }
                                    SelectionPanel::AgentPanel => {
                                        mouse.column.max(bounds.1)
                                    }
                                    SelectionPanel::None => mouse.column,
                                };
                                sel.focus = Some((col, mouse.row));
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            sel.active = false;
                            let term_w = terminal.size().unwrap_or(Size::new(80, 24)).width;
                            let (anchor, focus) = (sel.anchor, sel.focus);

                            // 优先用渲染时收集的文本，否则直接从 buffer 读取
                            let text = if !sel.selected_text.is_empty() {
                                sel.selected_text.clone()
                            } else if let (Some(a), Some(f)) = (anchor, focus) {
                                if a != f {
                                    let (y1, y2) = (a.1.min(f.1), a.1.max(f.1));
                                    let (x1, x2) = (a.0.min(f.0), a.0.max(f.0));
                                    let mut buf_text = String::new();
                                    for y in y1..=y2 {
                                        if !buf_text.is_empty() { buf_text.push('\n'); }
                                        let buf = terminal.current_buffer_mut();
                                        for x in x1..=x2.min(term_w.saturating_sub(1)) {
                                            buf_text.push_str(buf[(x, y)].symbol());
                                        }
                                    }
                                    buf_text
                                } else {
                                    String::new()
                                }
                            } else {
                                String::new()
                            };

                            if text.trim().is_empty() {
                                app.tui.bottom_bar.status = "⚠ 选中区域无文字".into();
                            } else {
                                write_clipboard(&text);
                                let pasted = std::process::Command::new("wl-paste")
                                    .output()
                                    .ok()
                                    .and_then(|o| String::from_utf8(o.stdout).ok())
                                    .unwrap_or_default();
                                if !pasted.trim().is_empty() && (pasted.trim() == text.trim() || pasted.contains(text.trim())) {
                                    app.tui.bottom_bar.status = format!("✓ 已复制 ({}B)", text.len());
                                } else if pasted.trim().is_empty() {
                                    app.tui.bottom_bar.status = "✗ 剪贴板为空".into();
                                } else {
                                    app.tui.bottom_bar.status = format!("✗ 不匹配: 写{}读{}", text.len(), pasted.len());
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── 键盘事件 ──
                if let crossterm::event::Event::Key(key) = event {
                    let focus = &app.tui.focus_panel;

                    match key.code {
                        // ── Alt+方向键：面板切换 ──
                        crossterm::event::KeyCode::Left
                            if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
                        {
                            app.handle_action(Action::CycleFocusPanel(-1)).await.ok();
                        }
                        crossterm::event::KeyCode::Right
                            if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
                        {
                            app.handle_action(Action::CycleFocusPanel(1)).await.ok();
                        }
                        crossterm::event::KeyCode::Up
                            if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
                        {
                            // MainView + Messages 子区中：块导航
                            // 其他面板：焦点子区循环
                            if *focus == crate::app::FocusPanel::MainView
                                && app.tui.main_view_subsection
                                    == crate::app::MainViewSubsection::Messages
                            {
                                let blocks =
                                    crate::message::build_block_refs(&app.tui.main_view.messages);
                                let cur = app.tui.main_view.block_cursor;
                                if cur > 0 && !blocks.is_empty() {
                                    app.tui.main_view.block_cursor = cur - 1;
                                    if let Some(bref) =
                                        blocks.get(app.tui.main_view.block_cursor)
                                    {
                                        app.tui.message_cursor = bref.msg_index;
                                        app.tui.scroll_mode =
                                            crate::app::ScrollMode::Pinned;
                                    }
                                }
                            } else {
                                app.handle_action(Action::CycleFocusSubsection(-1))
                                    .await
                                    .ok();
                            }
                        }
                        crossterm::event::KeyCode::Down
                            if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
                        {
                            // MainView + Messages 子区中：块导航
                            // 其他面板：焦点子区循环
                            if *focus == crate::app::FocusPanel::MainView
                                && app.tui.main_view_subsection
                                    == crate::app::MainViewSubsection::Messages
                            {
                                let blocks =
                                    crate::message::build_block_refs(&app.tui.main_view.messages);
                                let cur = app.tui.main_view.block_cursor;
                                if cur + 1 < blocks.len() {
                                    app.tui.main_view.block_cursor = cur + 1;
                                    if let Some(bref) =
                                        blocks.get(app.tui.main_view.block_cursor)
                                    {
                                        app.tui.message_cursor = bref.msg_index;
                                        app.tui.scroll_mode =
                                            crate::app::ScrollMode::Pinned;
                                    }
                                }
                            } else {
                                app.handle_action(Action::CycleFocusSubsection(1))
                                    .await
                                    .ok();
                            }
                        }

                        // Ctrl+C: 退出
                        crossterm::event::KeyCode::Char('c')
                            if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            tracing::info!("Ctrl+C 退出");
                            break;
                        }

                        // Ctrl+Z: 终止
                        crossterm::event::KeyCode::Char('z')
                            if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            tracing::debug!("发送 abort");
                            let cmd = serde_json::json!({"type": "abort"});
                            let _ = client.request(cmd, Duration::from_secs(5)).await;
                            app.agent_status = AgentStatus::Idle;
                        }

                        // Esc: 退出详情视图 / 关闭 Popup / 编辑表单
                        crossterm::event::KeyCode::Esc => {
                            if app.tui.main_view.completion_popup.is_some() {
                                app.tui.main_view.completion_popup = None;
                            } else if app.tui.main_view.entered_view.is_some() {
                                app.handle_action(Action::ExitBlock).await.ok();
                            } else if app.tui.provider_editor.is_some() {
                                let has_mgr = app
                                    .tui
                                    .provider_editor
                                    .as_ref()
                                    .map(|e| e.model_mgr.is_some())
                                    .unwrap_or(false);
                                if has_mgr {
                                    // model_mgr 打开时，Esc 先关闭它
                                    app.tui.handle_provider_key(key.code);
                                } else {
                                    app.tui.provider_editor = None;
                                }
                            } else if app.tui.provider_popup.is_some() {
                                app.tui.provider_popup = None;
                            } else if app.tui.popup.visible {
                                app.tui.popup.visible = false;
                            }
                        }

                        // ── Sidebar + Workspace 子区：↑/↓/Enter ──
                        _ if *focus == crate::app::FocusPanel::Sidebar
                            && app.tui.sidebar_subsection
                                == crate::app::SidebarSubsection::Workspace =>
                        {
                            match key.code {
                                crossterm::event::KeyCode::Up => {
                                    app.handle_action(Action::SidebarMove(-1)).await.ok();
                                }
                                crossterm::event::KeyCode::Down => {
                                    app.handle_action(Action::SidebarMove(1)).await.ok();
                                }
                                crossterm::event::KeyCode::Enter
                                | crossterm::event::KeyCode::Char(' ') => {
                                    let cursor = app.tui.sidebar_cursor;
                                    if let Some((is_ws, ws_idx, sess_idx)) =
                                        app.tui.sidebar_item_at(cursor)
                                    {
                                        if is_ws {
                                            app.handle_action(Action::ToggleWorkspace(ws_idx))
                                                .await
                                                .ok();
                                        } else if let Some(si) = sess_idx {
                                            if let Some(session) = app
                                                .tui
                                                .workspaces
                                                .get(ws_idx)
                                                .and_then(|ws| ws.sessions.get(si))
                                            {
                                                app.handle_action(
                                                    Action::SelectSession(session.id.clone()),
                                                )
                                                .await
                                                .ok();
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        // Sidebar + Provider: 委托给 TuiState 处理
                        _ if *focus == crate::app::FocusPanel::Sidebar
                            && app.tui.sidebar_subsection
                                == crate::app::SidebarSubsection::Provider =>
                        {
                            app.tui.handle_provider_key(key.code);
                            // 检查是否需要自动拉取模型列表
                            if let Some(ref editor) = app.tui.provider_editor {
                                if editor.models_fetching && app.tui.models_fetch_rx.is_none() {
                                    let base_url = editor.draft.base_url.trim().to_string();
                                    let api_key = editor.draft.api_key.trim().to_string();
                                    if !base_url.is_empty() && !api_key.is_empty() {
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        app.tui.models_fetch_rx = Some(rx);
                                        tokio::spawn(async move {
                                            let result =
                                                crate::provider::fetch_models_list(&base_url, &api_key)
                                                    .await;
                                            let _ = tx.send(result);
                                        });
                                    }
                                }
                            }
                        }

                        // Sidebar + Model: 委托给 TuiState 处理
                        _ if *focus == crate::app::FocusPanel::Sidebar
                            && app.tui.sidebar_subsection
                                == crate::app::SidebarSubsection::Model =>
                        {
                            app.tui.handle_model_key(key.code);
                            // 同步模型选择到 pi（通过 RPC set_model）
                            if app.tui.model_just_switched {
                                app.tui.model_just_switched = false;
                                let model_id = app.tui.current_model.clone();
                                if !model_id.is_empty() {
                                    // 保存到磁盘并同步到 router 共享内存配置
                                    app.tui.sync_provider_config();
                                    // 通知 pi 切换模型
                                    let _ = client.request(
                                        serde_json::json!({
                                            "type": "set_model",
                                            "provider": "local",
                                            "modelId": model_id,
                                        }),
                                        std::time::Duration::from_secs(5),
                                    ).await;
                                }
                            }
                        }

                        _ if *focus == crate::app::FocusPanel::MainView
                            && app.tui.main_view_subsection
                                == crate::app::MainViewSubsection::Messages =>
                        {
                            match key.code {
                                // EnteredView 内 ↑/↓ 滚动
                                crossterm::event::KeyCode::Up
                                    if app.tui.main_view.entered_view.is_some() =>
                                {
                                    if let Some(crate::message::EnteredView::FullOutput {
                                        ref mut scroll,
                                        ..
                                    }) = app.tui.main_view.entered_view
                                    {
                                        *scroll = scroll.saturating_sub(1);
                                    }
                                }
                                crossterm::event::KeyCode::Down
                                    if app.tui.main_view.entered_view.is_some() =>
                                {
                                    if let Some(crate::message::EnteredView::FullOutput {
                                        ref mut scroll,
                                        ..
                                    }) = app.tui.main_view.entered_view
                                    {
                                        *scroll += 1;
                                    }
                                }
                                crossterm::event::KeyCode::Up => {
                                    let cur = app.tui.message_cursor;
                                    if cur > 0 {
                                        app.tui.message_cursor = cur - 1;
                                        app.tui.scroll_mode = crate::app::ScrollMode::Pinned;
                                    }
                                }
                                crossterm::event::KeyCode::Down => {
                                    let total = app.tui.main_view.messages.len();
                                    let cur = app.tui.message_cursor;
                                    if cur + 1 < total {
                                        app.tui.message_cursor = cur + 1;
                                    }
                                    if cur + 1 >= total.saturating_sub(1) {
                                        app.tui.scroll_mode =
                                            crate::app::ScrollMode::TailFollow;
                                    }
                                }
                                crossterm::event::KeyCode::Enter => {
                                    // 块选中时进入详情视图
                                    let blocks = crate::message::build_block_refs(
                                        &app.tui.main_view.messages,
                                    );
                                    if !blocks.is_empty()
                                        && app.tui.main_view.block_cursor < blocks.len()
                                        && app.tui.main_view.entered_view.is_none()
                                    {
                                        let bref = &blocks[app.tui.main_view.block_cursor];
                                        app.handle_action(Action::EnterBlock {
                                            agent_id: agent_id.clone(),
                                            msg_id: bref.msg_id.clone(),
                                            block_index: bref.block_index,
                                        })
                                        .await
                                        .ok();
                                    } else if app.tui.main_view.entered_view.is_none() {
                                        // 无块选中时打开消息弹出（原行为）
                                        if let Some(msg) = app
                                            .tui
                                            .main_view
                                            .messages
                                            .get(app.tui.message_cursor)
                                        {
                                            let title = match msg.role {
                                                crate::message::ChatRole::User => {
                                                    "你".into()
                                                }
                                                crate::message::ChatRole::Assistant => {
                                                    "pi".into()
                                                }
                                                crate::message::ChatRole::Tool => {
                                                    "工具".into()
                                                }
                                                crate::message::ChatRole::System => {
                                                    "系统".into()
                                                }
                                                crate::message::ChatRole::Error => {
                                                    "错误".into()
                                                }
                                            };
                                            app.tui.popup.title = title;
                                            app.tui.popup.description = msg.text.clone();
                                            app.tui.popup.visible = true;
                                        }
                                    }
                                }
                                crossterm::event::KeyCode::Char(' ') => {
                                    // Space 通过 Action 切换块的折叠/展开
                                    let blocks = crate::message::build_block_refs(
                                        &app.tui.main_view.messages,
                                    );
                                    if app.tui.main_view.block_cursor < blocks.len() {
                                        let bref = &blocks[app.tui.main_view.block_cursor];
                                        app.handle_action(Action::ToggleBlock {
                                            agent_id: agent_id.clone(),
                                            msg_id: bref.msg_id.clone(),
                                            block_index: bref.block_index,
                                        })
                                        .await
                                        .ok();
                                    }
                                }
                                _ => {}
                            }
                        }

                        // ── MainView + Input 子区：↑ 切换到 Messages ──
                        _ if *focus == crate::app::FocusPanel::MainView
                            && app.tui.main_view_subsection
                                == crate::app::MainViewSubsection::Input =>
                        {
                            match key.code {
                                crossterm::event::KeyCode::Up => {
                                    app.tui.main_view_subsection =
                                        crate::app::MainViewSubsection::Messages;
                                }
                                // / 触发命令补全（仅输入框为空时）
                                crossterm::event::KeyCode::Char('/')
                                    if input_buffer.is_empty() =>
                                {
                                    input_buffer.push('/');
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
                                }
                                // @ 触发文件补全
                                crossterm::event::KeyCode::Char('@') => {
                                    input_buffer.push('@');
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
                                // Tab: 补全 popup 下移
                                crossterm::event::KeyCode::Tab
                                    if app.tui.main_view.completion_popup.is_some() =>
                                {
                                    if let Some(ref mut popup) =
                                        app.tui.main_view.completion_popup
                                    {
                                        if popup.cursor + 1 < popup.items.len() {
                                            popup.cursor += 1;
                                        }
                                    }
                                }
                                // Enter: 补全 popup 选中 / 发送消息
                                crossterm::event::KeyCode::Enter => {
                                    if let Some(popup) =
                                        app.tui.main_view.completion_popup.take()
                                    {
                                        if let Some(item) = popup.items.get(popup.cursor) {
                                            input_buffer = item.value.clone();
                                        }
                                    } else {
                                        let trimmed = input_buffer.trim().to_string();
                                        if !trimmed.is_empty() {
                                            input_buffer.clear();
                                            app.handle_action(Action::UserSubmitInput(
                                                trimmed.clone(),
                                            ))
                                            .await
                                            .ok();
                                            let _ = client
                                                .notify(serde_json::json!({
                                                    "type": "prompt",
                                                    "message": trimmed,
                                                }))
                                                .await;
                                        }
                                    }
                                }
                                // Backspace: 删除字符，清空时关闭 popup
                                crossterm::event::KeyCode::Backspace => {
                                    input_buffer.pop();
                                    if input_buffer.is_empty() {
                                        app.tui.main_view.completion_popup = None;
                                    }
                                }
                                // 通用字符输入
                                crossterm::event::KeyCode::Char(c) => {
                                    input_buffer.push(c);
                                }
                                _ => {}
                            }
                        }

                        // ── AgentPanel + Agents 子区：↑/↓ 选择 subagent ──
                        _ if *focus == crate::app::FocusPanel::AgentPanel
                            && app.tui.agent_panel_subsection
                                == crate::app::AgentPanelSubsection::Agents =>
                        {
                            let total = app.tui.subagents.len();
                            match key.code {
                                crossterm::event::KeyCode::Up if total > 0 => {
                                    let cur = app.tui.agent_cursor;
                                    if cur > 0 {
                                        app.tui.agent_cursor = cur - 1;
                                    }
                                }
                                crossterm::event::KeyCode::Down if total > 0 => {
                                    let cur = app.tui.agent_cursor;
                                    if cur + 1 < total {
                                        app.tui.agent_cursor = cur + 1;
                                    }
                                }
                                _ => {}
                            }
                        }

                        // ── AgentPanel + Skills 子区：↑/↓ 选择 skill ──
                        _ if *focus == crate::app::FocusPanel::AgentPanel
                            && app.tui.agent_panel_subsection
                                == crate::app::AgentPanelSubsection::Skills =>
                        {
                            let total = app.tui.skills.len();
                            let display_max = 6usize;
                            match key.code {
                                crossterm::event::KeyCode::Up if total > 0 => {
                                    let cur = app.tui.skill_cursor;
                                    if cur > 0 {
                                        app.tui.skill_cursor = cur - 1;
                                    }
                                }
                                crossterm::event::KeyCode::Down if total > 0 => {
                                    let cur = app.tui.skill_cursor;
                                    if cur + 1 < total && cur + 1 < display_max {
                                        app.tui.skill_cursor = cur + 1;
                                    }
                                }
                                _ => {}
                            }
                        }

                        // ── AgentPanel + Mcps 子区：↑/↓ 选择 MCP server ──
                        _ if *focus == crate::app::FocusPanel::AgentPanel
                            && app.tui.agent_panel_subsection
                                == crate::app::AgentPanelSubsection::Mcps =>
                        {
                            let total = app.tui.mcps.len();
                            match key.code {
                                crossterm::event::KeyCode::Up if total > 0 => {
                                    let cur = app.tui.mcp_cursor;
                                    if cur > 0 {
                                        app.tui.mcp_cursor = cur - 1;
                                    }
                                }
                                crossterm::event::KeyCode::Down if total > 0 => {
                                    let cur = app.tui.mcp_cursor;
                                    if cur + 1 < total {
                                        app.tui.mcp_cursor = cur + 1;
                                    }
                                }
                                _ => {}
                            }
                        }

                        _ => {}
                    }
                }
            }

            // 事件流结束或出错
            else => {
                tracing::warn!("事件流结束");
                break;
            }
        }
    }

    tracing::debug!("TUI 主循环结束");
    backend.stop().await.ok();
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    let _ = ratatui::try_restore();
    Ok(())
}

#[cfg(test)]
mod clipboard_tests {
    #[test]
    fn test_wl_copy_roundtrip() {
        let test_data = "meta-tui wl-copy test: 你好 123 !@#";

        // 写入：echo -n test_data | wl-copy
        {
            let mut echo = std::process::Command::new("echo")
                .arg(test_data)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("echo 启动失败");

            let mut wl_copy = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("wl-copy 启动失败");

            if let (Some(mut w_stdin), Some(e_stdout)) = (wl_copy.stdin.take(), echo.stdout.take())
            {
                use std::io::{BufReader, Read, Write};
                let mut reader = BufReader::new(e_stdout);
                let mut buf = [0u8; 4096];
                loop {
                    let n = reader.read(&mut buf).expect("读 echo 输出失败");
                    if n == 0 {
                        break;
                    }
                    w_stdin.write_all(&buf[..n]).expect("写 wl-copy 输入失败");
                }
                drop(w_stdin);
            }

            let status = wl_copy.wait().expect("等待 wl-copy 完成");
            assert!(status.success(), "wl-copy 退出码非零");
        }

        // 读出验证
        let output = std::process::Command::new("wl-paste")
            .output()
            .expect("wl-paste 启动失败");
        assert!(output.status.success(), "wl-paste 退出码非零");
        let result = String::from_utf8_lossy(&output.stdout);
        assert_eq!(result.trim(), test_data, "剪贴板内容不匹配");
    }

    #[test]
    fn test_xclip_roundtrip() {
        let test_data = "meta-tui clipboard test: 你好 123 !@#";

        // 使用管道：echo -n test_data | xclip -selection clipboard
        let mut echo = std::process::Command::new("echo")
            .arg(test_data)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("echo 启动失败");

        let mut xclip = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("xclip 启动失败");

        // 将 echo 的 stdout 传给 xclip 的 stdin
        if let (Some(mut x_stdin), Some(e_stdout)) = (xclip.stdin.take(), echo.stdout.take()) {
            use std::io::{BufReader, Read, Write};
            let mut reader = BufReader::new(e_stdout);
            let mut buf = [0u8; 4096];
            loop {
                let n = reader.read(&mut buf).expect("读 echo 输出失败");
                if n == 0 {
                    break;
                }
                x_stdin.write_all(&buf[..n]).expect("写 xclip 输入失败");
            }
            drop(x_stdin); // 关闭 stdin 发 EOF
        }

        let status = xclip.wait().expect("等待 xclip 完成");
        assert!(status.success(), "xclip 退出码非零");

        // 读出验证
        let output = std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
            .expect("xclip 读取失败");
        assert!(output.status.success());
        let result = String::from_utf8_lossy(&output.stdout);
        assert_eq!(result.trim(), test_data, "剪贴板内容不匹配");
    }
}

/// 将文本写入系统剪贴板（跨平台兼容）
///
/// 按优先级依次尝试：
/// - Wayland: `wl-copy`
/// - X11: `xclip` 或 `xsel`
/// - macOS: `pbcopy`
/// - Windows: `clip`
fn write_clipboard(text: &str) {
    use std::io::Write;

    #[cfg(target_os = "linux")]
    {
        // Wayland
        if std::env::var("WAYLAND_DISPLAY").is_ok()
            && std::process::Command::new("wl-copy")
                .arg("--version")
                .output()
                .is_ok()
        {
            if let Ok(mut child) = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    drop(stdin);
                }
                let _ = child.wait();
                return;
            }
        }
        // X11 via xclip
        if std::process::Command::new("xclip")
            .arg("--version")
            .output()
            .is_ok()
        {
            if let Ok(mut child) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    drop(stdin);
                }
                let _ = child.wait();
                return;
            }
        }
        // X11 via xsel
        if std::process::Command::new("xsel")
            .arg("--version")
            .output()
            .is_ok()
        {
            if let Ok(mut child) = std::process::Command::new("xsel")
                .args(["-b", "-i"])
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    drop(stdin);
                }
                let _ = child.wait();
                return;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin);
            }
            let _ = child.wait();
            return;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(mut child) = std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin);
            }
            let _ = child.wait();
            return;
        }
    }

    tracing::warn!("未找到可用的剪贴板工具 (wl-copy/xclip/xsel/pbcopy/clip)");
}

/// 检查 Ctrl+C（用于错误状态的简单轮询）
async fn check_ctrl_c() -> bool {
    // 非阻塞检查 stdin 是否有 Ctrl+C
    tokio::task::spawn_blocking(|| {
        if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let crossterm::event::Event::Key(key) = crossterm::event::read().unwrap() {
                return key.code == crossterm::event::KeyCode::Char('c')
                    && key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL);
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}
