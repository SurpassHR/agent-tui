use crate::app::App;
use crate::state::{McpInfo, SessionNode, SubAgentInfo, WorkspaceNode};
use crate::utils::{
    compute_display_names, dirs_for, extract_session_name, extract_workspace_cwd,
    load_session_messages, parse_agent_md, parse_skill_md, pi_sessions_dir,
};

impl App {
    /// 从当前 TUI 状态构建持久化数据结构
    pub fn build_persist_state(&self) -> crate::persistence::UiPersistState {
        crate::persistence::UiPersistState {
            active_session_id: self.active_agent.clone(),
            active_agent_sessions: self.active_sessions.iter().cloned().collect(),
            expanded_workspaces: self
                .tui
                .workspaces
                .iter()
                .filter(|ws| ws.expanded)
                .map(|ws| ws.cwd.clone())
                .collect(),
            session_names: self.tui.session_names.clone(),
        }
    }

    /// 从 state.json 恢复工作区展开状态和活跃会话
    ///
    /// 应在 `populate_workspaces()` 之后调用。
    pub fn restore_persisted_state(&mut self) {
        let state = crate::persistence::load();

        // 恢复工作区展开状态
        for ws in &mut self.tui.workspaces {
            ws.expanded = state.expanded_workspaces.contains(&ws.cwd);
        }

        // 恢复手动创建 session 的名称映射
        self.tui.session_names = state.session_names;

        // 恢复活跃会话
        if let Some(ref session_id) = state.active_session_id {
            // 验证会话是否仍然存在
            let session_exists = self
                .tui
                .workspaces
                .iter()
                .any(|ws| ws.sessions.iter().any(|s| s.id == *session_id));
            if session_exists {
                tracing::info!("恢复上次活跃会话: {}", session_id);
                self.tui.active_session = session_id.clone();
                // 查找 file_path 并加载消息
                if let Some(path) = self.tui.workspaces.iter().find_map(|ws| {
                    ws.sessions
                        .iter()
                        .find(|s| s.id == *session_id)
                        .and_then(|s| s.file_path.clone())
                }) {
                    match load_session_messages(&path) {
                        Ok(messages) => {
                            tracing::info!(
                                "恢复会话消息: {} ({} 条消息)",
                                session_id,
                                messages.len()
                            );
                            self.messages.insert(session_id.clone(), messages);
                            self.active_agent = Some(session_id.clone());
                            self.session.id = session_id.clone();
                            self.session.file_path = Some(path);
                            if let Some(ws) = self
                                .tui
                                .workspaces
                                .iter()
                                .find(|ws| ws.sessions.iter().any(|s| s.id == *session_id))
                            {
                                if let Some(sess) = ws.sessions.iter().find(|s| s.id == *session_id)
                                {
                                    self.session.name = Some(sess.name.clone());
                                }
                            }
                            self.sync_messages_to_main_view(session_id);
                            self.sync_components();
                        }
                        Err(e) => {
                            tracing::warn!("恢复会话消息失败: {} — {}", path, e);
                            // 加载失败时仍设置 active_agent 和同步组件以保持 UI 一致
                            self.active_agent = Some(session_id.clone());
                            self.session.id = session_id.clone();
                            self.sync_components();
                        }
                    }
                }
            } else {
                tracing::info!("上次活跃会话已不存在，不恢复");
            }
        }
    }

    /// 从 session 目录扫描并填充工作区数据
    ///
    /// 从每个目录下的第一个 JSONL 文件提取 `cwd` 字段作为工作区标识，
    /// 无 JSONL 或无法提取 cwd 的目录不显示。同名末端目录自动加父级区分。
    /// 已有工作区的展开状态在重建时保留。
    pub fn populate_workspaces(&mut self) {
        // 先从 state.json 加载自定义 session 名称映射，
        // 确保上次改名后的名称在重建工作区时生效
        // （restore_persisted_state 在此之后调用，但 populate_workspaces
        // 需要 session_names 来决定每个 session 的显示名）
        if self.tui.session_names.is_empty() {
            let state = crate::persistence::load();
            self.tui.session_names = state.session_names;
        }

        let sessions_dir = pi_sessions_dir();
        let mut ws_map: std::collections::HashMap<String, WorkspaceNode> =
            std::collections::HashMap::new();

        // 保存当前工作区的展开状态（按 cwd 索引）
        let old_expanded: std::collections::HashMap<String, bool> = self
            .tui
            .workspaces
            .iter()
            .map(|ws| (ws.cwd.clone(), ws.expanded))
            .collect();

        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                // 扫描目录中的 JSONL 文件
                let mut jsonl_files: Vec<std::path::PathBuf> = Vec::new();
                if let Ok(file_entries) = std::fs::read_dir(&path) {
                    for fe in file_entries.flatten() {
                        let fp = fe.path();
                        if fp.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            jsonl_files.push(fp);
                        }
                    }
                }
                // 无 JSONL 的目录不显示
                if jsonl_files.is_empty() {
                    continue;
                }

                // 从第一个 JSONL 文件提取 cwd
                let cwd = jsonl_files
                    .first()
                    .and_then(|fp| extract_workspace_cwd(fp))
                    .unwrap_or_default();
                if cwd.is_empty() {
                    continue;
                }

                // 构建会话列表
                let mut sessions = Vec::new();
                for fp in &jsonl_files {
                    let id = fp
                        .file_stem()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let name = self.tui.session_names.get(&id).cloned().unwrap_or_else(|| {
                        extract_session_name(fp).unwrap_or_else(|| "New Session".to_string())
                    });
                    let message_count = if let Ok(content) = std::fs::read_to_string(fp) {
                        content.lines().count()
                    } else {
                        0
                    };
                    sessions.push(SessionNode {
                        id: id.clone(),
                        name,
                        file_path: Some(fp.to_string_lossy().to_string()),
                        message_count,
                        is_online: false, // sync_components 中会根据 active_sessions 更新
                    });
                }
                sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));

                // 按 cwd 去重合并：同一 cwd 的多个目录合并会话
                if let Some(existing) = ws_map.get_mut(&cwd) {
                    existing.sessions.extend(sessions);
                    existing
                        .sessions
                        .sort_by(|a, b| b.message_count.cmp(&a.message_count));
                } else {
                    ws_map.insert(
                        cwd.clone(),
                        WorkspaceNode {
                            cwd: cwd.clone(),
                            display_name: String::new(), // 稍后计算
                            sessions,
                            // 已有工作区保留原展开状态，新工作区默认展开
                            expanded: *old_expanded.get(&cwd).unwrap_or(&true),
                        },
                    );
                }
            }
        }

        // 转换为 Vec 并计算 display_name
        let mut workspaces: Vec<WorkspaceNode> = ws_map.into_values().collect();
        compute_display_names(&mut workspaces);
        workspaces.sort_by(|a, b| a.display_name.cmp(&b.display_name));

        self.tui.workspaces = workspaces;
    }
}

impl App {
    /// 扫描 agents/*.md 文件解析 subagent 列表
    pub fn populate_subagents(&mut self) {
        let mut agents = Vec::new();

        // 数据源优先级：用户目录 > 内置
        let builtin_dir = dirs_for("agents", false);
        let user_dir = dirs_for("agents", true);

        // 先加载内置，用户同名文件覆盖
        for dir in [&builtin_dir, &user_dir] {
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let fpath = entry.path();
                    if fpath.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&fpath) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    if let Some(info) = parse_agent_md(&content) {
                        // 用户目录覆盖内置
                        if dir == &user_dir {
                            if let Some(pos) = agents
                                .iter()
                                .position(|a: &SubAgentInfo| a.name == info.name)
                            {
                                agents[pos] = info;
                                continue;
                            }
                        }
                        agents.push(info);
                    }
                }
            }
        }

        self.tui.subagents = agents;
    }

    /// 扫描 skills 目录解析 skill 列表
    pub fn populate_skills(&mut self) {
        let mut skills = Vec::new();
        let skills_dir = std::path::PathBuf::from("/home/hr/.agents/skills");

        if skills_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let skill_md = path.join("SKILL.md");
                    if !skill_md.exists() {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&skill_md) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    if let Some(info) = parse_skill_md(&content) {
                        skills.push(info);
                    }
                }
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        self.tui.skills = skills;
    }

    /// 扫描 mcp.json + mcp-cache.json 解析 MCP server 列表
    pub fn populate_mcps(&mut self) {
        let mut mcps = Vec::new();

        // 定位 pi 配置目录（PI_CODING_AGENT_DIR 或 ~/.pi/agent）
        let pi_home = std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .map(|h| std::path::PathBuf::from(h).join(".pi").join("agent"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

        // 来源 1：mcp.json（用户显式配置的 server）
        let config_path = pi_home.join("mcp.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, info) in servers {
                            let command = info
                                .get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .to_string();
                            mcps.push(McpInfo {
                                name: name.clone(),
                                command,
                                tool_count: 0,
                            });
                        }
                    }
                }
            }
        }

        // 来源 2：mcp-cache.json（已连接的 server 缓存）
        let cache_path = pi_home.join("mcp-cache.json");
        if cache_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&cache_path) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("servers").and_then(|v| v.as_object()) {
                        for (name, info) in servers {
                            // 跳过已在 mcp.json 中定义的 server
                            if mcps.iter().any(|m: &McpInfo| m.name == *name) {
                                continue;
                            }
                            let tool_count = info
                                .get("tools")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            mcps.push(McpInfo {
                                name: name.clone(),
                                command: String::new(),
                                tool_count,
                            });
                        }
                    }
                }
            }
        }

        mcps.sort_by(|a, b| a.name.cmp(&b.name));
        self.tui.mcps = mcps;
    }
}
