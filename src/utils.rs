use crate::message::{ChatMessage, ChatRole, ContentBlock, DiffLine, ToolCallInfo, ToolStatus};
use crate::state::{SkillInfo, SubAgentInfo, WorkspaceNode};

/// 解析 agent .md 文件的 YAML frontmatter
pub(crate) fn parse_agent_md(content: &str) -> Option<SubAgentInfo> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start();
    let end = rest.find("---")?;
    let yaml_text = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();
    let mut model = String::new();

    for line in yaml_text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("model:") {
            model = val.trim().to_string();
        }
    }

    if name.is_empty() {
        return None;
    }
    Some(SubAgentInfo {
        name,
        description,
        model,
    })
}

/// 解析 skill SKILL.md 文件的 YAML frontmatter
pub(crate) fn parse_skill_md(content: &str) -> Option<SkillInfo> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start();
    let end = rest.find("---")?;
    let yaml_text = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();

    for line in yaml_text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        }
    }

    if name.is_empty() {
        return None;
    }
    Some(SkillInfo { name, description })
}

/// 获取 pi 相关目录
pub(crate) fn dirs_for(subdir: &str, user: bool) -> std::path::PathBuf {
    let base = if user {
        std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(".pi").join("agent")
            })
    } else {
        std::env::var("PI_CODING_AGENT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(".pi").join("agent")
            })
            .join("npm")
            .join("node_modules")
            .join("pi-subagents")
    };
    base.join(subdir)
}

/// 将 pi 编码的工作区路径名解码为可读的目录名
///
/// `--media-hr-Data-Codes-agent-tui--` → `agent-tui`
/// `--home-hr-.pi-agent--` → `.pi-agent`
/// `--tmp--` → `tmp`
/// 从 JSONL 文件第一行提取 `cwd` 字段
pub(crate) fn extract_workspace_cwd(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?;
    let val: serde_json::Value = serde_json::from_str(first_line).ok()?;
    val.get("cwd")?.as_str().map(String::from)
}

/// 为一组工作区计算去重后的 display_name
pub(crate) fn compute_display_names(workspaces: &mut [WorkspaceNode]) {
    let mut names: Vec<String> = workspaces
        .iter()
        .map(|ws| {
            std::path::Path::new(&ws.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| ws.cwd.clone())
        })
        .collect();

    loop {
        let mut dup_set = std::collections::HashSet::new();
        let mut has_dup = false;
        for n in &names {
            if !dup_set.insert(n.clone()) {
                has_dup = true;
                break;
            }
        }
        if !has_dup {
            break;
        }
        let mut count_map: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for n in &names {
            *count_map.entry(n.clone()).or_insert(0) += 1;
        }
        for (i, ws) in workspaces.iter().enumerate() {
            if *count_map.get(&names[i]).unwrap_or(&0) > 1 {
                let path = std::path::Path::new(&ws.cwd);
                if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                    names[i] = format!("{}/{}", parent.to_string_lossy(), names[i]);
                }
            }
        }
    }

    for (i, ws) in workspaces.iter_mut().enumerate() {
        ws.display_name = names[i].clone();
    }
}

/// 从 JSONL 文件中提取可读的会话名称
///
/// 扫描前 20 行：优先找 `"type":"session_name"` 行取 `name` 字段，
/// 否则取第一条 `user` 消息的文本内容（截取 35 字符）。
pub(crate) fn extract_session_name(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines().take(20) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let val: serde_json::Value = serde_json::from_str(line).ok()?;
        let ty = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "session_name" => {
                return val.get("name").and_then(|v| v.as_str()).map(|s| {
                    let trimmed = s.trim();
                    if trimmed.chars().count() > 35 {
                        let end: usize = trimmed.chars().take(35).map(|c| c.len_utf8()).sum();
                        format!("{}…", &trimmed[..end])
                    } else {
                        trimmed.to_string()
                    }
                });
            }
            "message" => {
                let msg = val.get("message")?;
                if msg.get("role").and_then(|v| v.as_str()) != Some("user") {
                    continue;
                }
                let text = msg
                    .get("content")?
                    .as_array()?
                    .first()?
                    .get("text")?
                    .as_str()?;
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.chars().count() > 35 {
                    let end: usize = trimmed.chars().take(35).map(|c| c.len_utf8()).sum();
                    return Some(format!("{}…", &trimmed[..end]));
                }
                return Some(trimmed.to_string());
            }
            _ => continue,
        }
    }
    None
}

/// 获取 pi sessions 目录
pub(crate) fn pi_sessions_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("sessions")
}

/// 从 JSONL 文件加载 ChatMessage 列表
pub(crate) fn load_session_messages(path: &str) -> Result<Vec<ChatMessage>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {}", e))?;
    let mut messages = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("JSON 解析失败: {}", e))?;

        // pi 消息：{ "session_meta": { ... } } 或 { "context": ..., "message": { ... }, "turn_context": ... }
        let message_obj = msg
            .get("message")
            .or_else(|| msg.get("turn_context"))
            .or_else(|| msg.get("context"));

        // 如果是 session_meta 行，跳过
        if msg.get("session_meta").is_some() && message_obj.is_none() {
            continue;
        }

        let msg_data = message_obj.unwrap_or(&msg);

        let role_str = msg_data
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let chat_role = match role_str {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            "system" => ChatRole::System,
            "tool" => ChatRole::Tool,
            _ => ChatRole::User,
        };

        let text = msg_data
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let mut thinking: Option<String> = None;
        let mut tool_call: Option<ToolCallInfo> = None;
        let mut content_blocks: Vec<ContentBlock> = Vec::new();

        // 尝试从 content 数组中提取内容块
        if let Some(content_arr) = msg_data.get("content").and_then(|c| c.as_array()) {
            for block in content_arr {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            content_blocks.push(ContentBlock::Text {
                                text: t.to_string(),
                            });
                        }
                    }
                    "thinking" => {
                        if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                            thinking = Some(t.to_string());
                            content_blocks.push(ContentBlock::Thinking {
                                thinking: t.to_string(),
                            });
                        }
                    }
                    "tool_use" | "toolCall" => {
                        let tool_name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tool_id = block
                            .get("id")
                            .or_else(|| block.get("tool_call_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tool_args = block.get("input").or_else(|| block.get("arguments"));
                        tool_call = Some(ToolCallInfo {
                            tool_call_id: tool_id.clone(),
                            tool_name: tool_name.clone(),
                            status: ToolStatus::Running,
                            args: tool_args.cloned().unwrap_or(serde_json::Value::Null),
                            result: None,
                            detail_text: String::new(),
                        });
                        content_blocks.push(ContentBlock::ToolCall {
                            id: tool_id,
                            name: tool_name,
                            arguments: tool_args.cloned().unwrap_or(serde_json::Value::Null),
                            result: None,
                            is_error: false,
                        });
                    }
                    "tool_result" | "toolCallResult" | "toolExecutionEnd" => {
                        let _tool_name = block
                            .get("name")
                            .or_else(|| block.get("tool_name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tool_id = block
                            .get("id")
                            .or_else(|| block.get("tool_call_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let result_val = block
                            .get("result")
                            .or_else(|| block.get("content"))
                            .or_else(|| block.get("output"));
                        // 尝试找到匹配的 tool_use block 并回填 result
                        if let Some(ContentBlock::ToolCall {
                            ref mut result,
                            ref mut is_error,
                            ..
                        }) = content_blocks.iter_mut().rev().find(
                            |cb| matches!(cb, ContentBlock::ToolCall { id, .. } if id == &tool_id),
                        ) {
                            let result_content = result_val
                                .and_then(|v| v.as_str())
                                .map(|s| serde_json::json!({"content": s}))
                                .or_else(|| result_val.cloned());
                            *result = result_content;
                            *is_error = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                        }
                    }
                    _ => {}
                }
            }
        }

        // 提取 tool_calls（OpenAI-compat 格式）
        if let Some(tcs) = msg_data.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let func = tc.get("function");
                let tool_name = func
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_args = func
                    .and_then(|f| f.get("arguments"))
                    .or_else(|| tc.get("input"));
                tool_call = Some(ToolCallInfo {
                    tool_call_id: tool_id.clone(),
                    tool_name: tool_name.clone(),
                    status: ToolStatus::Running,
                    args: tool_args.cloned().unwrap_or(serde_json::Value::Null),
                    result: None,
                    detail_text: String::new(),
                });
            }
        }

        // 如果 content 不是数组而是直接文本，且无 content_blocks
        if content_blocks.is_empty() && !text.is_empty() {
            content_blocks.push(ContentBlock::Text { text: text.clone() });
        }

        // 尝试提图片 URI 的 detail 字段
        let mut _detail_text = String::new();
        if let Some(content_arr) = msg_data.get("content").and_then(|c| c.as_array()) {
            for block in content_arr {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if block_type == "image" {
                    let source = block.get("source").and_then(|s| s.as_object());
                    let image_url = source
                        .and_then(|s| s.get("url"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            block
                                .get("image_url")
                                .and_then(|u| u.get("url"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("[image]");
                    _detail_text = image_url.to_string();
                }
            }
        }

        // tool 消息的 result 文本
        let _result_content = msg_data
            .get("content")
            .and_then(|c| c.as_str())
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            });

        let mut msg_id = msg_data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if msg_id.is_empty() {
            msg_id = format!("msg_{}", messages.len());
        }

        messages.push(ChatMessage {
            id: msg_id,
            content: if chat_role == ChatRole::Tool {
                vec![]
            } else {
                content_blocks
            },
            agent_id: String::new(),
            role: chat_role,
            text,
            thinking,
            tool_call,
            timestamp: msg_data
                .get("timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            meta: None,
        });
    }

    Ok(messages)
}

/// 使用 similar crate 计算统一 diff
pub(crate) fn compute_diff_lines(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let (kind, old_line, new_line) = match change.tag() {
            ChangeTag::Delete => ('-', Some(change.old_index().unwrap_or(0) + 1), None),
            ChangeTag::Insert => ('+', None, Some(change.new_index().unwrap_or(0) + 1)),
            ChangeTag::Equal => (
                ' ',
                Some(change.old_index().unwrap_or(0) + 1),
                Some(change.new_index().unwrap_or(0) + 1),
            ),
        };
        lines.push(DiffLine {
            kind,
            old_line,
            new_line,
            text: change.value().to_string(),
        });
    }
    lines
}

/// 将用户友好的工作区名称编码为 pi 目录格式
/// 将文件系统路径编码为 pi sessions 目录名
///
/// pi 编码规则：`/` → `-`，前后加 `--`。
/// 例如 `/home/hr/Projects/agent-tui` → `--home-hr-Projects-agent-tui--`
pub(crate) fn encode_workspace_name(path: &str) -> String {
    // 与 pi 行为一致：剥离开头的 / 后再编码
    let trimmed = path.trim_start_matches('/');
    format!("--{}--", trimmed.replace('/', "-"))
}

/// 将 cwd 与 sessions 目录名匹配
///
/// 检查 `dir_name` 是否对应给定的 `cwd` 路径。
pub(crate) fn dir_matches_cwd(dir_name: &str, cwd: &str) -> bool {
    encode_workspace_name(cwd) == dir_name
}

/// 生成简单的 UUID（8 位 hex）
pub(crate) fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", ts % 0xFFFFFFFFFFFFFFFFu128)
}

/// 构建 AI 重命名 prompt
///
/// 从会话 JSONL 内容中提取最近的消息，构造分析 prompt
pub(crate) fn build_ai_rename_prompt(session_content: &str, current_name: &str) -> String {
    let lines: Vec<&str> = session_content.lines().collect();
    // 取最近 30 行
    let recent: Vec<&str> = if lines.len() > 30 {
        lines[lines.len() - 30..].to_vec()
    } else {
        lines
    };

    // 提取用户问题和助手回复的关键信息
    let mut user_texts: Vec<String> = Vec::new();
    for line in &recent {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(msg) = v.get("message") {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" {
                    if let Some(text) = msg
                        .get("content")
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|block| block.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        let t = text.trim();
                        if t.len() > 80 {
                            user_texts.push(format!("{}…", &t[..80]));
                        } else {
                            user_texts.push(t.to_string());
                        }
                    }
                }
            }
        }
    }

    let user_summary = user_texts.join("\n  ");

    format!(
        "请为以下对话生成 3 个候选会话名称（简短、描述性的英文名，用连字符连接）。当前名称为「{current_name}」。\n\n对话概要:\n  {user_summary}\n\n请只输出 3 个候选名称，每行一个，不要编号或其他文字。"
    )
}
