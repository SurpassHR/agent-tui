//! 结构化消息模型 — RPC 模式下表示 pi agent 的完整对话状态
//!
//! 替换阶段三的 `MainView.buffer` + `ansi::parse_to_lines` 方案，
//! 从原始终端输出升级为结构化消息列表。
//!
//! # 设计
//!
//! - `ChatMessage` 是单一的消息单元，可表示用户/助手/工具调用等各种角色
//! - `ContentBlock` 描述助手消息内部的多种内容类型（文本、思考、工具调用）
//! - `MessageData` 是从 pi 事件中提取的原始消息摘要

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 聊天消息的角色类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChatRole {
    /// 用户发送的消息
    User,
    /// 助手（pi）的回复
    Assistant,
    /// 工具执行结果
    Tool,
    /// 系统状态消息
    System,
    /// 错误消息
    Error,
}

/// 工具执行状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolStatus {
    /// 工具正在执行中
    Running,
    /// 工具执行完成
    Done,
    /// 工具执行出错
    Error,
}

/// 块的展开状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BlockExpanded {
    /// 折叠（默认）— 只显示摘要行
    Collapsed,
    /// 展开 — 显示完整内容
    Expanded,
}

/// 块类型（用于可交互块分类）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockKind {
    /// 思考块
    Thinking,
    /// 工具调用块
    ToolCall,
}

/// Diff 行
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// 行类型：- 删除, + 新增, = 不变
    pub kind: char,
    /// 行号（原始文件）
    pub old_line: Option<usize>,
    /// 行号（新文件）
    pub new_line: Option<usize>,
    /// 行文本
    pub text: String,
}

/// 补全候选项
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// 显示文本
    pub label: String,
    /// 填入的文本
    pub value: String,
    /// 类型图标前缀（如 "📁" / "⚡"）
    pub prefix: String,
}

/// 输入补全 popup 状态
#[derive(Debug, Clone)]
pub struct CompletionPopup {
    /// 候选项列表
    pub items: Vec<CompletionItem>,
    /// 当前选中索引
    pub cursor: usize,
    /// 触发前缀: "/" 或 "@"
    pub trigger: char,
    /// 用户已输入的过滤文本（不含前缀）
    pub filter: String,
}

/// 工具调用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    /// 工具名称（如 read、bash、edit、write）
    pub tool_name: String,
    /// 工具调用唯一标识
    pub tool_call_id: String,
    /// 执行状态
    pub status: ToolStatus,
    /// 调用参数
    pub args: Value,
    /// 执行结果（完成/出错后填充）
    pub result: Option<Value>,
    /// 格式化后的详细描述文本，用于 UI 展示
    pub detail_text: String,
}

/// 结构化消息单元
///
/// 这是 TUI MainView 渲染的基本单位。
/// 每条消息对应一次用户输入、一次助手回复、或一次工具调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 消息唯一标识
    pub id: String,
    /// 内容块数组（保持 pi AssistantMessage.content 的原始顺序）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentBlock>,
    /// 所属 agent 的 ID
    pub agent_id: String,
    /// 角色
    pub role: ChatRole,
    /// 文本内容
    pub text: String,
    /// 模型思考过程（可选，助手消息可能有）
    pub thinking: Option<String>,
    /// 工具调用信息（可选，工具消息可能有）
    pub tool_call: Option<ToolCallInfo>,
    /// 时间戳
    pub timestamp: u64,
    /// 额外元数据
    pub meta: Option<HashMap<String, Value>>,
}

impl ChatMessage {
    /// 创建用户消息
    pub fn user(agent_id: &str, text: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![],
            agent_id: agent_id.to_string(),
            role: ChatRole::User,
            text: text.to_string(),
            thinking: None,
            tool_call: None,
            timestamp: now_millis(),
            meta: None,
        }
    }

    /// 创建助手消息
    pub fn assistant(agent_id: &str, text: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![],
            agent_id: agent_id.to_string(),
            role: ChatRole::Assistant,
            text: text.to_string(),
            thinking: None,
            tool_call: None,
            timestamp: now_millis(),
            meta: None,
        }
    }

    /// 创建工具消息
    pub fn tool(agent_id: &str, tool_call: ToolCallInfo) -> Self {
        let text = format!(
            "{} {}",
            if tool_call.status == ToolStatus::Error {
                "✗"
            } else if tool_call.status == ToolStatus::Running {
                "▶"
            } else {
                "✓"
            },
            tool_call.tool_name,
        );
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![],
            agent_id: agent_id.to_string(),
            role: ChatRole::Tool,
            text,
            thinking: None,
            tool_call: Some(tool_call),
            timestamp: now_millis(),
            meta: None,
        }
    }

    /// 创建系统消息
    pub fn system(agent_id: &str, text: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![],
            agent_id: agent_id.to_string(),
            role: ChatRole::System,
            text: text.to_string(),
            thinking: None,
            tool_call: None,
            timestamp: now_millis(),
            meta: None,
        }
    }

    /// 创建错误消息
    pub fn error(agent_id: &str, text: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![],
            agent_id: agent_id.to_string(),
            role: ChatRole::Error,
            text: text.to_string(),
            thinking: None,
            tool_call: None,
            timestamp: now_millis(),
            meta: None,
        }
    }
}

/// 进入详情视图类型
#[derive(Debug, Clone)]
pub enum EnteredView {
    /// Diff 视图
    Diff {
        /// 文件路径
        path: String,
        /// diff 行
        diff_lines: Vec<DiffLine>,
    },
    /// 完整输出视图
    FullOutput {
        /// 标题
        title: String,
        /// 完整内容
        content: String,
        /// 滚动偏移（行数）
        scroll: usize,
    },
    /// Subagent 对话视图
    Subagent {
        /// 子代理 ID
        agent_id: String,
        /// 子代理消息列表
        messages: Vec<ChatMessage>,
    },
}

/// pi 消息中的内容块（对应 assistant 消息的复杂内容结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// 普通文本块
    #[serde(rename = "text")]
    Text {
        /// 文本内容
        text: String,
    },
    /// 模型思考块
    #[serde(rename = "thinking")]
    Thinking {
        /// 思考内容
        thinking: String,
    },
    /// 工具调用块
    #[serde(rename = "toolCall")]
    ToolCall {
        /// 工具调用 ID
        id: String,
        /// 工具名称
        name: String,
        /// 调用参数
        arguments: Value,
        /// 执行结果（ToolExecutionEnd 后回填）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        /// 是否出错
        #[serde(default)]
        is_error: bool,
    },
    /// 图片块（预留，终端 TUI 暂不支持展示）
    #[allow(unused)]
    #[serde(rename = "image")]
    Image {
        /// base64 编码的图片数据
        data: String,
        /// MIME 类型
        mime_type: String,
    },
}

/// pi 消息原始数据摘要（从 JSON 事件中提取的公共字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    /// 消息角色（user / assistant / toolResult）
    pub role: String,
    /// 消息内容块列表
    pub content: Vec<ContentBlock>,
    /// 停止原因（仅 assistant 消息有）
    pub stop_reason: Option<String>,
    /// 错误信息（仅错误时有）
    pub error_message: Option<String>,
}

/// 获取当前时间戳（毫秒）
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("agent-1", "Hello, world!");
        assert_eq!(msg.agent_id, "agent-1");
        assert_eq!(msg.role, ChatRole::User);
        assert_eq!(msg.text, "Hello, world!");
        assert!(msg.thinking.is_none());
        assert!(msg.tool_call.is_none());
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn test_chat_message_assistant() {
        let msg = ChatMessage::assistant("agent-1", "Hi there!");
        assert_eq!(msg.role, ChatRole::Assistant);
        assert_eq!(msg.text, "Hi there!");
    }

    #[test]
    fn test_chat_message_tool() {
        let info = ToolCallInfo {
            tool_name: "read".to_string(),
            tool_call_id: "call-1".to_string(),
            status: ToolStatus::Done,
            args: serde_json::json!({"path": "src/main.rs"}),
            result: Some(serde_json::json!({"content": "fn main() {}"})),
            detail_text: "Read src/main.rs (12 bytes)".to_string(),
        };
        let msg = ChatMessage::tool("agent-1", info);
        assert_eq!(msg.role, ChatRole::Tool);
        assert!(msg.text.contains("read"));
        assert!(msg.tool_call.is_some());
        assert_eq!(msg.tool_call.as_ref().unwrap().tool_name, "read");
    }

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("agent-1", "Connected to pi");
        assert_eq!(msg.role, ChatRole::System);
        assert_eq!(msg.text, "Connected to pi");
    }

    #[test]
    fn test_chat_message_error() {
        let msg = ChatMessage::error("agent-1", "Connection failed");
        assert_eq!(msg.role, ChatRole::Error);
        assert_eq!(msg.text, "Connection failed");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let msg = ChatMessage::user("agent-1", "test");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.id, deserialized.id);
        assert_eq!(msg.text, deserialized.text);
        assert_eq!(msg.role, deserialized.role);
    }

    #[test]
    fn test_content_block_serialize_text() {
        let block = ContentBlock::Text {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "Hello");
    }

    #[test]
    fn test_content_block_serialize_thinking() {
        let block = ContentBlock::Thinking {
            thinking: "I need to...".to_string(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "thinking");
        assert_eq!(json["thinking"], "I need to...");
    }

    #[test]
    fn test_content_block_tool_call() {
        let block = ContentBlock::ToolCall {
            id: "tc-1".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
            result: None,
            is_error: false,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "toolCall");
        assert_eq!(json["name"], "bash");
        assert_eq!(json["arguments"]["command"], "ls");
        assert!(json.get("result").is_none());
        assert_eq!(json["is_error"], false);
    }

    #[test]
    fn test_content_block_tool_call_with_result() {
        let block = ContentBlock::ToolCall {
            id: "tc-2".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({"path": "src/main.rs"}),
            result: Some(serde_json::json!({"content": "fn main() {}"})),
            is_error: false,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["result"]["content"], "fn main() {}");
        assert_eq!(json["is_error"], false);
    }

    #[test]
    fn test_message_data() {
        let data = MessageData {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
            }],
            stop_reason: Some("endTurn".to_string()),
            error_message: None,
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["stop_reason"], "endTurn");
    }

    #[test]
    fn test_unique_ids() {
        let msg1 = ChatMessage::user("agent-1", "a");
        let msg2 = ChatMessage::user("agent-1", "b");
        assert_ne!(msg1.id, msg2.id);
    }
}
