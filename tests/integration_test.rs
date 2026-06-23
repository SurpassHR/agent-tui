use meta_tui::action::Action;
use meta_tui::message::{ChatMessage, ChatRole, ToolCallInfo, ToolStatus};
use serde_json::json;
use tokio::sync::mpsc;

/// 测试 1：ChatMessage 构建和序列化
#[test]
fn test_chat_message_user() {
    let msg = ChatMessage::user("agent-1", "Hello, world!");
    assert_eq!(msg.agent_id, "agent-1");
    assert_eq!(msg.text, "Hello, world!");
    assert_eq!(msg.role, ChatRole::User);
    assert!(msg.thinking.is_none());
    assert!(msg.tool_call.is_none());
}

#[test]
fn test_chat_message_assistant() {
    let msg = ChatMessage::assistant("agent-1", "Hi there!");
    assert_eq!(msg.role, ChatRole::Assistant);
    assert_eq!(msg.text, "Hi there!");
}

#[test]
fn test_chat_message_tool() {
    let tool = ToolCallInfo {
        tool_name: "bash".into(),
        tool_call_id: "call-1".into(),
        status: ToolStatus::Done,
        args: json!({"command": "ls"}),
        result: Some(json!({"output": "file.txt"})),
        detail_text: String::new(),
    };
    let msg = ChatMessage::tool("agent-1", tool);
    assert_eq!(msg.role, ChatRole::Tool);
    assert!(msg.tool_call.is_some());
    let tc = msg.tool_call.unwrap();
    assert_eq!(tc.tool_name, "bash");
    assert_eq!(tc.tool_call_id, "call-1");
    assert_eq!(tc.status, ToolStatus::Done);
}

#[test]
fn test_chat_message_system() {
    let msg = ChatMessage::system("agent-1", "System message");
    assert_eq!(msg.role, ChatRole::System);
    assert_eq!(msg.text, "System message");
}

#[test]
fn test_chat_message_error() {
    let msg = ChatMessage::error("agent-1", "Something went wrong");
    assert_eq!(msg.role, ChatRole::Error);
    assert_eq!(msg.text, "Something went wrong");
}

#[test]
fn test_chat_message_serialization() {
    let msg = ChatMessage::user("agent-1", "Test");
    let json_str = serde_json::to_string(&msg).expect("序列化失败");
    let deserialized: ChatMessage = serde_json::from_str(&json_str).expect("反序列化失败");
    assert_eq!(deserialized.text, "Test");
    assert_eq!(deserialized.role, ChatRole::User);
}

/// 测试 2：PiEvent JSON 解析（不需要实际 RPC 连接）
#[test]
fn test_parse_pi_agent_start() {
    // PiEvent with serde tag - test AgentStart
    let json_str = r#"{"type":"agent_start"}"#;
    let event: meta_tui::backend::event::PiEvent =
        serde_json::from_str(json_str).expect("解析 PiEvent 失败");
    assert!(matches!(
        event,
        meta_tui::backend::event::PiEvent::AgentStart
    ));
}

#[test]
fn test_parse_pi_message_update() {
    let json_str = r#"{
        "type":"message_update",
        "assistantMessageEvent":{"type":"text_delta","delta":"Hello World"},
        "delta":"Hello World"
    }"#;
    let event: meta_tui::backend::event::PiEvent =
        serde_json::from_str(json_str).expect("解析 PiEvent 失败");
    if let meta_tui::backend::event::PiEvent::MessageUpdate {
        assistant_event,
        delta,
        ..
    } = event
    {
        assert!(matches!(
            assistant_event.event_type,
            meta_tui::backend::event::AssistantEventType::TextDelta
        ));
        assert_eq!(delta, Some("Hello World".to_string()));
    } else {
        panic!("期望 MessageUpdate");
    }
}

#[test]
fn test_parse_pi_tool_execution_start() {
    let json_str = r#"{
        "type":"tool_execution_start",
        "toolName":"read",
        "toolCallId":"call-123",
        "args":{"path":"/tmp/test.txt"}
    }"#;
    let event: meta_tui::backend::event::PiEvent =
        serde_json::from_str(json_str).expect("解析 PiEvent 失败");
    if let meta_tui::backend::event::PiEvent::ToolExecutionStart {
        tool_name,
        tool_call_id,
        ..
    } = event
    {
        assert_eq!(tool_name, "read");
        assert_eq!(tool_call_id, "call-123");
    } else {
        panic!("期望 ToolExecutionStart");
    }
}

/// 测试 3：Action channel 在有界缓冲下不阻塞
#[tokio::test]
async fn test_channel_bounded_buffer() {
    let (tx, mut rx) = mpsc::channel::<Action>(16);

    for i in 0..16 {
        tx.send(Action::UserSubmitInput(format!("msg-{}", i)))
            .await
            .unwrap();
    }

    let mut count = 0;
    while let Ok(action) = rx.try_recv() {
        if matches!(action, Action::UserSubmitInput(_)) {
            count += 1;
        }
    }
    assert_eq!(count, 16, "应收到 16 条消息");
}

/// 测试 4：App RPC 模式的基本状态
#[tokio::test]
async fn test_app_rpc_lifecycle() {
    let (action_tx, _action_rx) = mpsc::channel::<Action>(1024);
    let app = meta_tui::App::new_rpc(action_tx);

    assert!(app.use_rpc, "RPC 模式应启用");
    assert_eq!(app.agent_status, meta_tui::app::AgentStatus::Starting);
    assert!(app.active_agent.is_some());
}

/// 测试 5：App handle_action 处理 RPC 事件
#[tokio::test]
async fn test_app_handle_rpc_actions() {
    let (action_tx, _action_rx) = mpsc::channel::<Action>(1024);
    let mut app = meta_tui::App::new_rpc(action_tx);

    // 测试 UserSubmitInput 添加消息
    app.handle_action(Action::UserSubmitInput("test input".into()))
        .await
        .unwrap();
    let agent_id = app.active_agent.clone().unwrap();
    let msgs = app.messages.get(&agent_id);
    assert!(msgs.is_some(), "消息应被添加");
    if let Some(msgs) = msgs {
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "test input");
        assert_eq!(msgs[0].role, ChatRole::User);
    }

    // 测试 MessageAppend
    app.handle_action(Action::MessageAppend {
        agent_id: agent_id.clone(),
        text: "Hello ".into(),
    })
    .await
    .unwrap();
    app.handle_action(Action::MessageAppend {
        agent_id: agent_id.clone(),
        text: "World".into(),
    })
    .await
    .unwrap();
    let msgs = app.messages.get(&agent_id).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1].text, "Hello World");

    // 测试 AgentStatusChange
    app.handle_action(Action::AgentStatusChange {
        agent_id: agent_id.clone(),
        status: meta_tui::app::AgentStatus::Running,
    })
    .await
    .unwrap();
    assert_eq!(app.agent_status, meta_tui::app::AgentStatus::Running);
}

/// 测试 6：App RPC 模式消息管理
#[tokio::test]
async fn test_app_rpc_thinking() {
    let (action_tx, _action_rx) = mpsc::channel::<Action>(1024);
    let mut app = meta_tui::App::new_rpc(action_tx);
    let agent_id = app.active_agent.clone().unwrap();

    // 先有 assistant 消息，再追加 thinking
    app.handle_action(Action::MessageAppend {
        agent_id: agent_id.clone(),
        text: "Response text".into(),
    })
    .await
    .unwrap();
    app.handle_action(Action::ThinkingAppend {
        agent_id: agent_id.clone(),
        text: "thinking...".into(),
    })
    .await
    .unwrap();

    let msgs = app.messages.get(&agent_id).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].thinking.as_deref(), Some("thinking..."));
}

/// 测试 7：PiRpcClient 使用 cat 进程验证 notify
#[tokio::test]
async fn test_rpc_client_notify_with_cat() {
    let mut child = tokio::process::Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("无法启动 cat");

    let stdin = child.stdin.take().expect("无法获取 stdin");
    let _stdout = child.stdout.take().expect("无法获取 stdout");

    let mut client = meta_tui::backend::rpc_client::PiRpcClient::from_stdin(stdin);

    let cmd = serde_json::json!({"type": "ping"});
    let r = client.notify(cmd).await;
    assert!(r.is_ok(), "notify 应成功: {:?}", r.err());

    drop(client);
    let _ = child.wait().await;
}

/// 测试 8：检测 pi CLI 可用性（不假定已安装）
#[tokio::test]
async fn test_pi_check_installed() {
    let (installed, version) = meta_tui::backend::rpc::PiRpcBackend::check_installed().await;
    // 仅验证函数不 panic、返回一致的结果
    if installed {
        assert!(version.is_some(), "已安装时有版本号");
    } else {
        assert!(version.is_none(), "未安装时版本号为 None");
    }
}

/// 测试 9：PiRpcBackend 启动和停止（需要 pi CLI 已安装）
#[tokio::test]
#[ignore = "需要 pi CLI 已安装并配置"]
async fn test_rpc_backend_spawn_pi() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut backend = meta_tui::backend::rpc::PiRpcBackend::new("pi", &cwd);

    let _client = backend
        .start(None)
        .await
        .expect("pi --mode rpc 启动失败 — 请确认 pi 已安装并配置");

    assert!(backend.is_running(), "pi 进程应存活");

    backend.stop().await.expect("停止 pi 进程失败");
    assert!(!backend.is_running(), "pi 进程应已停止");
}
