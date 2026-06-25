//! UI 状态持久化模块
//!
//! 负责将侧边栏展开状态和活跃会话 ID 持久化到
//! `~/.config/agent-tui/state.json`，在 TUI 重启时恢复。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// UI 持久化状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiPersistState {
    /// 上次活跃的会话 ID（文件名去 .jsonl 后缀）
    pub active_session_id: Option<String>,
    /// 活跃 agent 会话 ID 列表（启动后自动重新 spawn）
    #[serde(default)]
    pub active_agent_sessions: Vec<String>,
    /// 已展开的工作区名称列表（WorkspaceNode.name，显示名）
    pub expanded_workspaces: Vec<String>,
    /// 手动创建 session 的名称映射（session_id → name）
    #[serde(default)]
    pub session_names: std::collections::HashMap<String, String>,
}

/// 获取 state.json 持久化文件路径
pub fn state_path() -> PathBuf {
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(val).join("agent-tui").join("state.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("agent-tui")
            .join("state.json")
    } else {
        PathBuf::from(".agent-tui").join("state.json")
    }
}

/// 从 state.json 加载持久化状态
///
/// 文件不存在或 JSON 解析失败时返回默认值（空状态），
/// 不中断程序运行。
pub fn load() -> UiPersistState {
    let path = state_path();
    if !path.exists() {
        return UiPersistState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("state.json 解析失败: {} — 使用默认状态", e);
                UiPersistState::default()
            }
        },
        Err(e) => {
            tracing::warn!("state.json 读取失败: {} — 使用默认状态", e);
            UiPersistState::default()
        }
    }
}

/// 保存持久化状态到 state.json（原子写）
///
/// 先写 `.tmp` 临时文件，成功后再 `rename` 覆盖目标文件，
/// 避免写入中途崩溃导致文件损坏。
pub fn save(state: &UiPersistState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    match serde_json::to_string_pretty(state) {
        Ok(content) => {
            if std::fs::write(&tmp, &content).is_err() {
                tracing::error!("写入 state.json.tmp 失败");
                return;
            }
            if std::fs::rename(&tmp, &path).is_err() {
                tracing::error!("重命名 state.json.tmp 失败");
            }
        }
        Err(e) => tracing::error!("序列化 UiPersistState 失败: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_roundtrip() {
        let state = UiPersistState {
            active_session_id: Some("2026-06-24T13-52-35-984Z_019ef9e7".into()),
            active_agent_sessions: vec!["sess-a".into(), "sess-b".into()],
            expanded_workspaces: vec![
                "--home-hr-Projects-agent-tui--".into(),
                "--media-hr-Data-Codes-ideogram4-editor--".into(),
            ],
            session_names: std::collections::HashMap::new(),
        };
        let json = serde_json::to_string(&state).expect("序列化失败");
        let restored: UiPersistState = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(restored.active_session_id, state.active_session_id);
        assert_eq!(restored.active_agent_sessions, state.active_agent_sessions);
        assert_eq!(restored.expanded_workspaces, state.expanded_workspaces);
    }

    #[test]
    fn test_default_is_empty() {
        let state = UiPersistState::default();
        assert!(state.active_session_id.is_none());
        assert!(state.active_agent_sessions.is_empty());
        assert!(state.expanded_workspaces.is_empty());
        assert!(state.session_names.is_empty());
    }

    #[test]
    fn test_backward_compat_no_agent_sessions() {
        // 旧版 state.json 没有 active_agent_sessions 和 session_names 字段
        let old = r#"{"active_session_id":"sess-1","expanded_workspaces":[]}"#;
        let restored: UiPersistState = serde_json::from_str(old).expect("应兼容旧格式");
        assert_eq!(restored.active_session_id.unwrap(), "sess-1");
        assert!(restored.active_agent_sessions.is_empty());
        assert!(restored.session_names.is_empty());
    }
}
