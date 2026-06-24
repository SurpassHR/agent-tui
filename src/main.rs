use std::time::Duration;

use meta_tui::backend::rpc::PiRpcBackend;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _log_guard = meta_tui::logging::init()?;
    let config = meta_tui::config::Config::from_args();

    if config.dry_run {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(dry_run())?;
        return Ok(());
    }

    if config.diagnose {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(diagnose())?;
        return Ok(());
    }

    // TUI 模式
    let (action_tx, action_rx) = tokio::sync::mpsc::channel(1024);
    let app = meta_tui::app::App::new_rpc(action_tx);
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { meta_tui::tui::run_tui(app, action_rx).await })?;
    Ok(())
}

/// pi 配置目录（优先 PI_CODING_AGENT_DIR 环境变量）
fn pi_home() -> std::path::PathBuf {
    std::env::var("PI_CODING_AGENT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".pi").join("agent")
        })
}

/// 全局 agent skills 目录
fn agent_skills_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/home/hr/.agents/skills")
}

/// 递归收集目录下所有 .jsonl 文件
fn collect_jsonl_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_jsonl_files(&path, files);
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
}

/// --dry-run：只打印配置信息，不启动 pi
async fn dry_run() -> color_eyre::Result<()> {
    println!("═══ Meta-TUI 信息 ═══\n");

    // pi 安装检测
    let (installed, version) = PiRpcBackend::check_installed().await;
    println!(
        "pi CLI: {}",
        if installed {
            format!("✅ v{}", version.unwrap_or_default())
        } else {
            "❌ 未安装".into()
        }
    );

    // Extensions (pi list)
    println!("\n── Extensions ──");
    match tokio::process::Command::new("pi")
        .arg("list")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                println!("  {}", line);
            }
            for line in String::from_utf8_lossy(&output.stderr).lines() {
                println!("  {}", line);
            }
        }
        Ok(output) => {
            println!(
                "  ❌ {}",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("unknown error")
            );
        }
        Err(e) => println!("  ❌ {}", e),
    }

    // Sessions（递归扫描项目子目录下的 .jsonl 文件）
    let base = pi_home();
    let session_dir = base.join("sessions");
    println!("\n── Sessions ({}) ──", session_dir.display());
    if session_dir.exists() {
        let mut total = 0;
        if let Ok(entries) = std::fs::read_dir(&session_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let project_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // 递归扫描该目录下的 .jsonl 文件
                    let mut files = Vec::new();
                    collect_jsonl_files(&path, &mut files);
                    if !files.is_empty() {
                        println!("  {} ({} 个会话)", project_name, files.len());
                        for f in files.iter().take(3) {
                            let name = f
                                .file_name()
                                .map(|n| n.to_string_lossy())
                                .unwrap_or_default();
                            let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                            println!("    {}  ({} bytes)", name, size);
                        }
                        if files.len() > 3 {
                            println!("    ... 还有 {} 个", files.len() - 3);
                        }
                        total += files.len();
                    }
                }
            }
        }
        if total == 0 {
            println!("  (空)");
        } else {
            println!("  📊 共 {} 个会话文件", total);
        }
    } else {
        println!("  (目录不存在)");
    }

    // Skills（只输出名字）
    println!("\n── Skills ──");
    let global_skills = agent_skills_dir();
    println!("  全局: {}", global_skills.display());
    if global_skills.exists() {
        if let Ok(entries) = std::fs::read_dir(&global_skills) {
            let mut names: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            if names.is_empty() {
                println!("    (空)");
            }
            for name in names {
                println!("    {}", name);
            }
        }
    } else {
        println!("    (目录不存在)");
    }
    let user_skills = base.join("skills");
    println!("  用户: {}", user_skills.display());
    if user_skills.exists() {
        if let Ok(entries) = std::fs::read_dir(&user_skills) {
            let mut names: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            if names.is_empty() {
                println!("    (空)");
            }
            for name in names {
                println!("    {}", name);
            }
        }
    } else {
        println!("    (目录不存在)");
    }

    // Agents（subagent 定义）
    println!("\n── Agents ──");
    let builtin_agents = base
        .join("npm")
        .join("node_modules")
        .join("pi-subagents")
        .join("agents");
    let user_agents = base.join("agents");
    for (label, dir) in [("内置", &builtin_agents), ("用户", &user_agents)] {
        println!("  {}: {}", label, dir.display());
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut names: Vec<_> = entries
                    .flatten()
                    .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
                    .map(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .trim_end_matches(".md")
                            .to_string()
                    })
                    .collect();
                names.sort();
                if names.is_empty() {
                    println!("    (空)");
                }
                for name in names {
                    println!("    {}", name);
                }
            }
        } else {
            println!("    (目录不存在)");
        }
    }

    // MCP 配置
    println!("\n── MCP Servers (从配置读取) ──");
    let mcp_json = base.join("mcp.json");
    if mcp_json.exists() {
        match std::fs::read_to_string(&mcp_json) {
            Ok(content) => {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, info) in servers {
                            let cmd = info.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                            let args: Vec<_> = info
                                .get("args")
                                .and_then(|v| v.as_array())
                                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                                .unwrap_or_default();
                            println!("  {}: {} {}", name, cmd, args.join(" "));
                            if let Some(tools) = info.get("directTools") {
                                println!("    directTools: {}", tools);
                            }
                        }
                    } else {
                        println!("  (没有配置 MCP servers)");
                    }
                }
            }
            Err(e) => println!("  ❌ 读取失败: {}", e),
        }
    } else {
        println!("  (mcp.json 不存在)");
    }

    // Provider & Model（从 settings.json 读取）
    println!("\n── Provider / Model ──");
    let settings_path = base.join("settings.json");
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                let provider = cfg
                    .get("defaultProvider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let model = cfg
                    .get("defaultModel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let thinking = cfg
                    .get("defaultThinkingLevel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                println!("  默认: {} / {}", provider, model);
                println!("  思考: {}", thinking);
                if let Some(enabled) = cfg.get("enabledModels").and_then(|v| v.as_array()) {
                    let names: Vec<_> = enabled.iter().filter_map(|v| v.as_str()).collect();
                    if !names.is_empty() {
                        println!("  已启用: {}", names.join(", "));
                    }
                }
                // 已配置的 provider（从 auth.json 读取）
                let auth_path = base.join("auth.json");
                if auth_path.exists() {
                    if let Ok(auth_content) = std::fs::read_to_string(&auth_path) {
                        if let Ok(auth) = serde_json::from_str::<serde_json::Value>(&auth_content) {
                            if let Some(providers) = auth.as_object() {
                                let names: Vec<_> = providers.keys().map(|k| k.as_str()).collect();
                                println!("  API keys: {}", names.join(", "));
                            }
                        }
                    }
                }
            }
        }
    }

    println!("\n═══ 信息输出完毕 ═══");
    Ok(())
}

/// --diagnose：启动 pi RPC 查询状态
async fn diagnose() -> color_eyre::Result<()> {
    println!("═══ Meta-TUI 诊断 ═══\n");

    // 1) 检测 pi 安装
    println!("── pi CLI ──");
    let (installed, version) = PiRpcBackend::check_installed().await;
    if !installed {
        println!("  ❌ 未安装\n");
        println!("  请运行: npm install -g @earendil-works/pi-coding-agent");
        return Ok(());
    }
    println!("  ✅ 已安装: v{}", version.unwrap_or_default());

    // 2) 启动 pi RPC
    println!("\n── 启动 pi --mode rpc --no-session ──");
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut backend = PiRpcBackend::new("pi", &cwd);
    let mut client = match backend.start(None).await {
        Ok(c) => c,
        Err(e) => {
            println!("  ❌ 启动失败: {}\n", e);
            return Ok(());
        }
    };
    println!("  ✅ pi RPC 进程已启动");

    // 3) get_state
    println!("\n── get_state ──");
    match client
        .request(
            serde_json::json!({"type": "get_state"}),
            Duration::from_secs(5),
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                if let Some(model) = data.get("model") {
                    let name = model.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let provider = model
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let cw = model
                        .get("contextWindow")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into());
                    println!("  模型: {} @{} (contextWindow: {})", name, provider, cw);
                }
                println!(
                    "  sessionId: {}",
                    data.get("sessionId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                );
                if let Some(path) = data.get("sessionFile").and_then(|v| v.as_str()) {
                    println!("  sessionFile: {}", path);
                }
                if let Some(name) = data.get("sessionName").and_then(|v| v.as_str()) {
                    println!("  sessionName: {}", name);
                }
                println!(
                    "  thinkingLevel: {}",
                    data.get("thinkingLevel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                );
                println!(
                    "  isStreaming: {}",
                    data.get("isStreaming")
                        .and_then(|v| v.as_bool())
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "?".into())
                );
                println!(
                    "  messageCount: {}",
                    data.get("messageCount")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into())
                );
            }
        }
        Ok(resp) => println!("  ❌ 请求失败: {}", resp.error.unwrap_or_default()),
        Err(e) => println!("  ❌ 请求错误: {}", e),
    }

    // 4) get_session_stats
    println!("\n── get_session_stats ──");
    match client
        .request(
            serde_json::json!({"type": "get_session_stats"}),
            Duration::from_secs(5),
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                if let Some(tokens) = data.get("tokens") {
                    println!("  tokens: {:?}", tokens);
                }
                if let Some(ctx) = data.get("contextUsage") {
                    println!("  contextUsage: {:?}", ctx);
                }
                if let Some(cost) = data.get("cost") {
                    println!("  cost: {:?}", cost);
                }
            }
        }
        Ok(resp) => println!("  ❌ 请求失败: {}", resp.error.unwrap_or_default()),
        Err(e) => println!("  ❌ 请求错误: {}", e),
    }

    // 5) get_commands
    println!("\n── get_commands ──");
    match client
        .request(
            serde_json::json!({"type": "get_commands"}),
            Duration::from_secs(5),
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                if let Some(commands) = data.get("commands").and_then(|v| v.as_array()) {
                    if commands.is_empty() {
                        println!("  (空)");
                    }
                    for cmd in commands {
                        let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let desc = cmd
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        println!("  {}  {}", name, desc);
                    }
                }
            }
        }
        Ok(resp) => println!("  ❌ 请求失败: {}", resp.error.unwrap_or_default()),
        Err(e) => println!("  ❌ 请求错误: {}", e),
    }

    // 6) 协议日志
    println!("\n── 协议日志 ──");
    let mut has_err = false;
    while let Ok(line) = client.protocol_err_rx.try_recv() {
        println!("  ⚠ {}", line);
        has_err = true;
    }
    if !has_err {
        println!("  (无)");
    }

    // 7) 清理
    println!("\n── 清理 ──");
    backend.stop().await.ok();
    println!("  ✅ pi 进程已停止");

    println!("\n═══ 诊断完成 ═══");
    Ok(())
}
