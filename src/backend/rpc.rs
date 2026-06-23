//! pi agent 的 RPC 后端实现
//!
//! 通过 `std::process::Command` spawn `pi --mode rpc`，
//! 将 stdin/stdout 移交给 `PiRpcClient` 进行 JSONL 通信。

use std::process::Stdio;
use std::sync::Mutex;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};

use super::rpc_client::{PiRpcClient, RpcReadState};
use crate::errors::Result;

/// pi RPC 后端 — 进程管理器
///
/// 负责 spawn pi 子进程、移交 stdin/stdout 给 PiRpcClient、管理进程生命周期。
/// **不实现旧的 AgentBackend trait** — RPC 后端接口完全不同。
pub struct PiRpcBackend {
    /// pi CLI 命令路径
    command: String,
    /// 工作目录
    cwd: std::path::PathBuf,
    /// 子进程句柄（用于 kill/wait）
    child: Mutex<Option<Child>>,
}

impl PiRpcBackend {
    /// 创建新的 PiRpcBackend
    pub fn new(command: impl Into<String>, cwd: impl Into<std::path::PathBuf>) -> Self {
        Self {
            command: command.into(),
            cwd: cwd.into(),
            child: Mutex::new(None),
        }
    }

    /// 检测 pi CLI 是否已安装
    ///
    /// 尝试执行 `pi --version`，成功返回 true 和版本号。
    pub async fn check_installed() -> (bool, Option<String>) {
        let result = tokio::process::Command::new("pi")
            .arg("--version")
            .output()
            .await;
        match result {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        String::from_utf8_lossy(&output.stderr)
                            .lines()
                            .next()
                            .map(|s| s.to_string())
                    });
                (true, version)
            }
            _ => (false, None),
        }
    }

    /// 启动 pi 子进程并返回 PiRpcClient
    ///
    /// spawn `pi --mode rpc`，移交 stdin/stdout，启动读取循环。
    /// session_path 为 None 时创建新 session，为 Some 时恢复已有 session。
    pub async fn start(&mut self, session_path: Option<&str>) -> Result<PiRpcClient> {
        let mut cmd = Command::new(&self.command);
        cmd.arg("--mode");
        cmd.arg("rpc");

        if let Some(path) = session_path {
            // 指定 session 文件：持久化，不用 --no-session
            cmd.arg("--session");
            cmd.arg(path);
        } else {
            // 无 session 时用 --no-session 避免 pi 初始化扩展卡住
            cmd.arg("--no-session");
        }

        cmd.current_dir(&self.cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| crate::errors::Error::Subprocess(format!("failed to spawn pi: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| crate::errors::Error::Subprocess("failed to take stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| crate::errors::Error::Subprocess("failed to take stdout".to_string()))?;
        let _stderr = child.stderr.take();

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (protocol_err_tx, protocol_err_rx) = tokio::sync::mpsc::unbounded_channel();
        let state = std::sync::Arc::new(std::sync::Mutex::new(RpcReadState {
            pending: std::collections::HashMap::new(),
            event_tx,
            protocol_err_tx,
        }));

        // 启动异步 stdout 读取循环
        let read_state = state.clone();
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let trimmed = line.trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        process_line(&trimmed, &read_state);
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        let guard = read_state.lock().unwrap();
                        let _ = guard.protocol_err_tx.send(format!("read error: {}", e));
                        break;
                    }
                }
            }
        });

        let writer = tokio::io::BufWriter::new(stdin);

        *self.child.lock().unwrap() = Some(child);

        Ok(PiRpcClient::new(
            writer,
            event_rx,
            protocol_err_rx,
            state.clone(),
        ))
    }

    /// 停止进程
    pub async fn stop(&mut self) -> Result<()> {
        let mut child_to_stop = self.child.lock().unwrap().take();
        if let Some(ref mut child) = child_to_stop {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    /// 检查进程是否存活
    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().unwrap();
        guard
            .as_mut()
            .map(|c| matches!(c.try_wait(), Ok(None)))
            .unwrap_or(false)
    }
}

/// 处理单行 JSON 输出
fn process_line(line: &str, state: &std::sync::Arc<std::sync::Mutex<RpcReadState>>) {
    if line.is_empty() {
        return;
    }

    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(val) => {
            let is_response = val.get("type").and_then(|v| v.as_str()) == Some("response");
            if is_response {
                if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                    let mut guard = state.lock().unwrap();
                    if let Some(pending) = guard.pending.remove(id) {
                        let response = super::rpc_client::RpcResponse {
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
                        return;
                    }
                }
            }

            // 尝试解析为 PiEvent
            match crate::backend::event::parse_pi_event(&val) {
                Ok(Some(event)) => {
                    let guard = state.lock().unwrap();
                    let _ = guard.event_tx.send(event);
                }
                Ok(None) => {
                    // response without pending match — ignore
                }
                Err(e) => {
                    let guard = state.lock().unwrap();
                    let _ = guard
                        .protocol_err_tx
                        .send(format!("parse error: {} | raw: {}", e, line));
                }
            }
        }
        Err(e) => {
            let guard = state.lock().unwrap();
            let _ = guard
                .protocol_err_tx
                .send(format!("JSON parse error: {} | raw: {}", e, line));
        }
    }
}
