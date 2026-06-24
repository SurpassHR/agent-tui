//! pi agent 的 JSONL RPC 事件类型和解析逻辑
//!
//! pi 以 `--mode rpc` 模式运行时，通过 stdout 输出 JSON Lines。
//! 每行可能是 `response`（同步回复）或 `event`（异步推送）。
//! 本模块专注于事件类型定义和 JSON → `PiEvent` 的转换。
//!
//! # 事件流示例
//!
//! ```json
//! {"type":"agent_start"}
//! {"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hello"}}
//! {"type":"tool_execution_start","toolName":"read","toolCallId":"...","args":{"path":"..."}}
//! {"type":"agent_end","stopReason":"endTurn","messages":[...]}
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::MessageData;

/// pi 事件类型枚举
///
/// 所有来自 pi agent 的 JSONL 事件都被解析为 `PiEvent`，
/// 随后由 `PiEventProcessor` 翻译为 `Action` 供 App 状态机消费。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiEvent {
    /// agent 开始处理新一轮任务
    AgentStart,

    /// agent 结束处理
    AgentEnd {
        /// 停止原因（endTurn / stop / error 等）
        #[serde(rename = "stopReason")]
        stop_reason: String,
        /// 错误信息（如果有）
        #[serde(default)]
        error: Option<String>,
        /// 是否将自动重试
        #[serde(rename = "willRetry", default)]
        will_retry: Option<bool>,
    },

    /// 新消息开始
    MessageStart {
        /// 消息角色（user / assistant）
        role: String,
    },

    /// 消息内容更新（流式）
    /// pi 发送的原始 JSON 中，`event_type` 嵌套在 `assistantMessageEvent` 对象中。
    MessageUpdate {
        /// 助理消息事件数据（嵌套对象，包含 type + delta）
        #[serde(rename = "assistantMessageEvent")]
        assistant_event: AssistantMessageEvent,
        /// 增量文本（部分事件携带）
        #[serde(default)]
        delta: Option<String>,
        /// 完整消息数据快照（部分事件携带）
        #[serde(default)]
        message: Option<MessageData>,
    },

    /// 消息结束定型
    MessageEnd {
        /// 最终消息数据
        #[serde(default)]
        message: Option<MessageData>,
    },

    /// 工具调用开始
    ToolExecutionStart {
        /// 工具名称
        #[serde(rename = "toolName")]
        tool_name: String,
        /// 工具调用唯一标识
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// 调用参数
        args: Value,
    },

    /// 工具调用进度更新
    ToolExecutionUpdate {
        /// 工具名称
        #[serde(rename = "toolName")]
        tool_name: String,
        /// 工具调用唯一标识
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// 部分结果
        #[serde(rename = "partialResult", default)]
        partial_result: Option<Value>,
    },

    /// 工具调用结束
    ToolExecutionEnd {
        /// 工具名称
        #[serde(rename = "toolName")]
        tool_name: String,
        /// 工具调用唯一标识
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// 执行结果
        result: Value,
        /// 是否出错
        #[serde(rename = "isError", default)]
        is_error: bool,
    },

    /// 扩展错误
    ExtensionError {
        /// 错误信息
        error: String,
    },

    /// 自动重试开始
    AutoRetryStart {
        /// 当前重试次数
        attempt: u32,
        /// 最大重试次数
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        /// 延迟时间（毫秒）
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        /// 错误信息
        #[serde(rename = "errorMessage", default)]
        error_message: Option<String>,
    },

    /// 自动重试结束
    AutoRetryEnd {
        /// 是否成功
        success: bool,
        /// 最终错误（如果失败）
        #[serde(rename = "finalError", default)]
        final_error: Option<String>,
    },

    /// 扩展 UI 请求（pi 向前端发送的 UI 交互请求，如设置状态、通知等）
    ExtensionUiRequest {
        /// 请求 ID
        id: String,
        /// 方法名
        method: String,
        /// 方法参数
        #[serde(default)]
        params: Value,
    },
}

/// pi `assistantMessageEvent` 嵌套对象的完整结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessageEvent {
    /// 子事件类型
    #[serde(rename = "type")]
    pub event_type: AssistantEventType,
    /// 增量文本
    #[serde(default)]
    pub delta: Option<String>,
}

/// 助手消息子事件类型
///
/// 用于 `MessageUpdate` 事件的 `assistantMessageEvent.type` 字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantEventType {
    /// 文本增量
    TextDelta,
    /// 文本开始
    TextStart,
    /// 文本结束
    TextEnd,
    /// 思考增量
    ThinkingDelta,
    /// 思考结束
    ThinkingEnd,
    /// 思考块开始
    #[serde(rename = "thinking_start")]
    ThinkingStart,
    /// 消息开始
    MessageStart,
    /// 消息结束
    MessageEnd,
    /// 工具调用块开始
    #[serde(rename = "toolcall_start")]
    ToolCallStart,
    /// 工具调用参数流式追加
    #[serde(rename = "toolcall_delta")]
    ToolCallDelta,
    /// 工具调用块完成
    #[serde(rename = "toolcall_end")]
    ToolCallEnd,
    /// 处理完成
    Done,
    /// 出错
    Error,
}

/// 将原始 JSON Value 解析为 PiEvent
///
/// 返回 `Ok(None)` 表示该 JSON 应作为 response 由 `PiRpcClient` 的 pending 机制处理。
/// 返回 `Ok(Some(event))` 表示该 JSON 是一个需要 App 消费的事件。
///
/// # 错误
///
/// 如果 JSON 结构无法匹配任何已知事件类型，返回错误。
pub fn parse_pi_event(raw: &Value) -> crate::errors::Result<Option<PiEvent>> {
    let type_field = raw
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::errors::Error::Channel("PiEvent missing 'type' field".to_string()))?;

    match type_field {
        // response 不在此处理，由 PiRpcClient 的 pending 机制消费
        "response" => {
            if raw.get("id").and_then(|v| v.as_str()).is_some() {
                return Ok(None);
            }
            // 没有 id 的 response 可能是异常格式，当作事件处理
            Err(crate::errors::Error::Channel(
                "response without id field".to_string(),
            ))
        }
        _ => {
            // 尝试反序列化为 PiEvent
            let event: PiEvent = serde_json::from_value(raw.clone()).map_err(|e| {
                crate::errors::Error::Channel(format!(
                    "failed to parse PiEvent '{}': {}",
                    type_field, e
                ))
            })?;
            Ok(Some(event))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_agent_start() {
        let raw = json!({"type": "agent_start"});
        let event = parse_pi_event(&raw).unwrap();
        assert!(matches!(event, Some(PiEvent::AgentStart)));
    }

    #[test]
    fn test_parse_agent_end_endturn() {
        let raw = json!({"type": "agent_end", "stopReason": "endTurn"});
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AgentEnd { stop_reason, .. }) => {
                assert_eq!(stop_reason, "endTurn");
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_agent_end_error() {
        let raw = json!({
            "type": "agent_end",
            "stopReason": "error",
            "error": "API rate limit exceeded"
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AgentEnd {
                stop_reason, error, ..
            }) => {
                assert_eq!(stop_reason, "error");
                assert_eq!(error, Some("API rate limit exceeded".to_string()));
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_agent_end_will_retry() {
        let raw = json!({
            "type": "agent_end",
            "stopReason": "error",
            "willRetry": true
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AgentEnd { will_retry, .. }) => {
                assert_eq!(will_retry, Some(true));
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_message_update_text_delta() {
        let raw = json!({
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "delta": "Hello"},
            "delta": "Hello"
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::MessageUpdate {
                assistant_event,
                delta,
                ..
            }) => {
                assert_eq!(assistant_event.event_type, AssistantEventType::TextDelta);
                assert_eq!(delta, Some("Hello".to_string()));
            }
            other => panic!("expected MessageUpdate, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_message_update_thinking_delta() {
        let raw = json!({
            "type": "message_update",
            "assistantMessageEvent": {
                "type": "thinking_delta",
                "delta": "I need to analyze..."
            }
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::MessageUpdate {
                assistant_event, ..
            }) => {
                assert_eq!(
                    assistant_event.event_type,
                    AssistantEventType::ThinkingDelta
                );
                assert_eq!(
                    assistant_event.delta,
                    Some("I need to analyze...".to_string())
                );
            }
            other => panic!("expected MessageUpdate, got {:?}", other),
        }
    }

    #[test]
    fn test_assistant_event_tool_call_start() {
        let raw = json!({
            "type": "message_update",
            "assistantMessageEvent": {
                "type": "toolcall_start",
                "contentIndex": 2,
                "toolName": "bash"
            }
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::MessageUpdate {
                assistant_event, ..
            }) => {
                assert_eq!(
                    assistant_event.event_type,
                    AssistantEventType::ToolCallStart
                );
            }
            other => panic!("expected MessageUpdate, got {:?}", other),
        }
    }

    #[test]
    fn test_assistant_event_thinking_start() {
        let raw = json!({
            "type": "message_update",
            "assistantMessageEvent": {
                "type": "thinking_start"
            }
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::MessageUpdate {
                assistant_event, ..
            }) => {
                assert_eq!(
                    assistant_event.event_type,
                    AssistantEventType::ThinkingStart
                );
            }
            other => panic!("expected MessageUpdate, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_execution_start() {
        let raw = json!({
            "type": "tool_execution_start",
            "toolName": "read",
            "toolCallId": "call-123",
            "args": {"path": "src/main.rs"}
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::ToolExecutionStart {
                tool_name,
                tool_call_id,
                args,
            }) => {
                assert_eq!(tool_name, "read");
                assert_eq!(tool_call_id, "call-123");
                assert_eq!(args["path"], "src/main.rs");
            }
            other => panic!("expected ToolExecutionStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_execution_end() {
        let raw = json!({
            "type": "tool_execution_end",
            "toolName": "bash",
            "toolCallId": "call-456",
            "result": {"exitCode": 0, "output": "hello"},
            "is_error": false
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::ToolExecutionEnd {
                tool_name,
                tool_call_id,
                is_error,
                ..
            }) => {
                assert_eq!(tool_name, "bash");
                assert_eq!(tool_call_id, "call-456");
                assert!(!is_error);
            }
            other => panic!("expected ToolExecutionEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_auto_retry_start() {
        let raw = json!({
            "type": "auto_retry_start",
            "attempt": 1,
            "maxAttempts": 3,
            "delayMs": 5000
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AutoRetryStart {
                attempt,
                max_attempts,
                delay_ms,
                ..
            }) => {
                assert_eq!(attempt, 1);
                assert_eq!(max_attempts, 3);
                assert_eq!(delay_ms, 5000);
            }
            other => panic!("expected AutoRetryStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_auto_retry_end() {
        let raw = json!({
            "type": "auto_retry_end",
            "success": true
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AutoRetryEnd { success, .. }) => {
                assert!(success);
            }
            other => panic!("expected AutoRetryEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_auto_retry_end_with_error() {
        let raw = json!({
            "type": "auto_retry_end",
            "success": false,
            "finalError": "API error"
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::AutoRetryEnd {
                success,
                final_error,
            }) => {
                assert!(!success);
                assert_eq!(final_error, Some("API error".to_string()));
            }
            other => panic!("expected AutoRetryEnd, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_extension_error() {
        let raw = json!({"type": "extension_error", "error": "Failed to load"});
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::ExtensionError { error }) => {
                assert_eq!(error, "Failed to load");
            }
            other => panic!("expected ExtensionError, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_response_returns_none() {
        let raw = json!({
            "id": "test-123",
            "type": "response",
            "command": "get_state",
            "success": true,
            "data": {}
        });
        let event = parse_pi_event(&raw);
        assert!(matches!(event, Ok(None)));
    }

    #[test]
    fn test_parse_response_without_id_error() {
        let raw = json!({"type": "response"});
        let event = parse_pi_event(&raw);
        assert!(event.is_err());
    }

    #[test]
    fn test_parse_unknown_type() {
        let raw = json!({"type": "unknown_event_type"});
        let event = parse_pi_event(&raw);
        assert!(event.is_err());
    }

    #[test]
    fn test_parse_missing_type() {
        let raw = json!({"data": "no type field"});
        let event = parse_pi_event(&raw);
        assert!(event.is_err());
    }

    #[test]
    fn test_parse_extension_ui_request() {
        let raw = json!({
            "type": "extension_ui_request",
            "id": "req-1",
            "method": "setStatus",
            "params": {"statusKey": "mcp", "statusText": "online"}
        });
        let event = parse_pi_event(&raw).unwrap();
        match event {
            Some(PiEvent::ExtensionUiRequest { id, method, params }) => {
                assert_eq!(id, "req-1");
                assert_eq!(method, "setStatus");
                assert_eq!(params["statusKey"], "mcp");
            }
            other => panic!("expected ExtensionUiRequest, got {:?}", other),
        }
    }
}
