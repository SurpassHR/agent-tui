//! pi agent 的 JSONL RPC 协议客户端
//!
//! 通过 stdin/stdout 与 `pi --mode rpc` 子进程通信。
//! 支持请求/响应模式和事件推送模式。

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use super::event::PiEvent;
use crate::errors::Result;

/// 待处理的请求
pub(crate) struct PendingRequest {
    /// 响应发送端
    pub responder: oneshot::Sender<RpcResponse>,
}

/// 读取循环的共享状态
pub(crate) struct RpcReadState {
    /// 待处理的请求
    pub pending: HashMap<String, PendingRequest>,
    /// 事件发送端
    pub event_tx: mpsc::UnboundedSender<PiEvent>,
    /// 协议错误发送端
    pub protocol_err_tx: mpsc::UnboundedSender<String>,
}

/// RPC 响应
#[derive(Debug, Clone)]
pub struct RpcResponse {
    /// 请求 ID
    pub id: String,
    /// 命令名称
    pub command: String,
    /// 是否成功
    pub success: bool,
    /// 响应数据
    pub data: Option<Value>,
    /// 错误信息
    pub error: Option<String>,
}

/// JSONL RPC 客户端
///
/// 使用 boxed writer 避免泛型参数传播到整个调用链。
pub struct PiRpcClient {
    /// stdin writer — 发送 JSONL 请求
    pub(crate) writer: BufWriter<Box<dyn AsyncWrite + Unpin + Send>>,
    /// 事件接收端 — App 从中消费 PiEvent
    pub event_rx: mpsc::UnboundedReceiver<PiEvent>,
    /// 协议错误接收端
    pub protocol_err_rx: mpsc::UnboundedReceiver<String>,
    /// 读取循环共享状态（用于 request 注册 pending）
    pub(crate) read_state: Arc<Mutex<RpcReadState>>,
}

#[allow(dead_code)]
impl PiRpcClient {
    /// 创建新的 PiRpcClient（内部使用，外部请用 from_stdin）
    pub(crate) fn new(
        writer: impl AsyncWrite + Unpin + Send + 'static,
        event_rx: mpsc::UnboundedReceiver<PiEvent>,
        protocol_err_rx: mpsc::UnboundedReceiver<String>,
        read_state: Arc<Mutex<RpcReadState>>,
    ) -> Self {
        Self {
            writer: BufWriter::new(Box::new(writer)),
            event_rx,
            protocol_err_rx,
            read_state,
        }
    }

    /// 发送请求并等待响应
    pub async fn request(
        &mut self,
        cmd: Value,
        timeout: std::time::Duration,
    ) -> Result<RpcResponse> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut payload = cmd.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.clone()));
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.read_state.lock().unwrap();
            guard
                .pending
                .insert(id.clone(), PendingRequest { responder: tx });
        }

        let line = serde_json::to_string(&payload)
            .map_err(|e| crate::errors::Error::Channel(format!("serialization error: {}", e)))?;
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(crate::errors::Error::Io)?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(crate::errors::Error::Io)?;
        self.writer
            .flush()
            .await
            .map_err(crate::errors::Error::Io)?;

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => {
                // 正常响应：从 pending 中移除（理论上 read_loop 已移除，但稳妥起见再做一次）
                let mut guard = self.read_state.lock().unwrap();
                guard.pending.remove(&id);
                Ok(response)
            }
            Ok(Err(_)) => {
                // oneshot sender 被丢弃（read_loop 出错退出）
                let mut guard = self.read_state.lock().unwrap();
                guard.pending.remove(&id);
                Err(crate::errors::Error::Channel(
                    "RPC channel closed".to_string(),
                ))
            }
            Err(_) => {
                // 超时：清理 pending，否则 sender 永远不释放
                let mut guard = self.read_state.lock().unwrap();
                guard.pending.remove(&id);
                Err(crate::errors::Error::Channel(
                    "RPC request timed out".to_string(),
                ))
            }
        }
    }

    /// 发送通知（不需要等待响应）
    pub async fn notify(&mut self, cmd: Value) -> Result<()> {
        let msg_type = cmd.get("type").and_then(|v| v.as_str()).unwrap_or("?");
        let msg_preview = if msg_type == "prompt" {
            cmd.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };
        tracing::info!(
            "RPC notify → pi: type={} {}",
            msg_type,
            if msg_preview.is_empty() {
                String::new()
            } else {
                format!("message=\"{}\"", msg_preview)
            }
        );
        let line = serde_json::to_string(&cmd)
            .map_err(|e| crate::errors::Error::Channel(format!("serialization error: {}", e)))?;
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(crate::errors::Error::Io)?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(crate::errors::Error::Io)?;
        self.writer
            .flush()
            .await
            .map_err(crate::errors::Error::Io)?;
        Ok(())
    }

    /// 获取事件接收端引用
    pub fn event_rx(&mut self) -> &mut mpsc::UnboundedReceiver<PiEvent> {
        &mut self.event_rx
    }

    /// 获取协议错误接收端引用
    pub fn protocol_err_rx(&mut self) -> &mut mpsc::UnboundedReceiver<String> {
        &mut self.protocol_err_rx
    }

    /// 从原始 stdin 创建 PiRpcClient（主要用于测试）
    ///
    /// 自动创建内部 channel 和 RpcReadState。不启动读取循环，仅用于写入测试。
    pub fn from_stdin(stdin: impl AsyncWrite + Unpin + Send + 'static) -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (protocol_err_tx, protocol_err_rx) = tokio::sync::mpsc::unbounded_channel();
        let read_state = std::sync::Arc::new(std::sync::Mutex::new(RpcReadState {
            pending: std::collections::HashMap::new(),
            event_tx,
            protocol_err_tx,
        }));
        Self::new(
            tokio::io::BufWriter::new(stdin),
            event_rx,
            protocol_err_rx,
            read_state,
        )
    }
}

/// 读取循环 — 在 spawn_blocking 中运行
///
/// 从 stdout 读取 JSON Lines，解析后：
/// - response 匹配 pending 请求 → 通过 oneshot 发送响应
/// - 事件 → 通过 event_tx 发送 PiEvent
/// - 解析失败 → 通过 protocol_err_tx 发送原始行
#[allow(dead_code)]
pub(crate) fn read_loop(
    mut reader: impl std::io::BufRead + Send + 'static,
    state: Arc<Mutex<RpcReadState>>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match serde_json::from_str::<Value>(trimmed) {
                    Ok(val) => {
                        let is_response =
                            val.get("type").and_then(|v| v.as_str()) == Some("response");
                        if is_response {
                            if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                                let mut guard = state.lock().unwrap();
                                if let Some(pending) = guard.pending.remove(id) {
                                    let response = RpcResponse {
                                        id: id.to_string(),
                                        command: val
                                            .get("command")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        success: val
                                            .get("success")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false),
                                        data: val.get("data").cloned(),
                                        error: val
                                            .get("error")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                    };
                                    let _ = pending.responder.send(response);
                                    continue;
                                }
                            }
                        }

                        match crate::backend::event::parse_pi_event(&val) {
                            Ok(Some(event)) => {
                                let guard = state.lock().unwrap();
                                let _ = guard.event_tx.send(event);
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let guard = state.lock().unwrap();
                                let _ = guard
                                    .protocol_err_tx
                                    .send(format!("parse error: {} | raw: {}", e, trimmed));
                            }
                        }
                    }
                    Err(e) => {
                        let guard = state.lock().unwrap();
                        let _ = guard
                            .protocol_err_tx
                            .send(format!("JSON parse error: {} | raw: {}", e, trimmed));
                    }
                }
            }
            Err(e) => {
                let guard = state.lock().unwrap();
                let _ = guard.protocol_err_tx.send(format!("read error: {}", e));
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[tokio::test]
    async fn test_rpc_event_stream() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (protocol_err_tx, _protocol_err_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(RpcReadState {
            pending: HashMap::new(),
            event_tx: event_tx.clone(),
            protocol_err_tx,
        }));

        let output = br#"{"type":"agent_start"}
{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hi"}}
{"type":"agent_end","stopReason":"endTurn"}
"#;
        let reader = BufReader::new(&output[..]);
        let read_state = state.clone();
        let handle = tokio::task::spawn_blocking(move || {
            read_loop(reader, read_state);
        });

        // 等待读取循环完成后再收集事件
        handle.await.unwrap();

        // 丢弃所有 sender 以关闭 channel
        drop(event_tx);
        drop(state);

        let mut events = Vec::new();
        while let Some(event) = event_rx.recv().await {
            events.push(event);
        }

        assert_eq!(events.len(), 3, "expected 3 events, got {}", events.len());
        assert!(matches!(events[0], PiEvent::AgentStart));
        assert!(matches!(events[1], PiEvent::MessageUpdate { .. }));
        assert!(matches!(events[2], PiEvent::AgentEnd { .. }));
    }

    #[tokio::test]
    async fn test_rpc_timeout() {
        let (stdin_writer, _stdin) = tokio::io::duplex(4096);
        let mut client = PiRpcClient::from_stdin(stdin_writer);

        let payload = serde_json::json!({"type": "get_state"});
        let result = client
            .request(payload, std::time::Duration::from_millis(50))
            .await;

        assert!(result.is_err(), "expected timeout error");
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("timed out"),
            "expected timeout-related error message, got: {}",
            err_msg
        );
    }
}
