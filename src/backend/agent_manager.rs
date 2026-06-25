//! 多 Agent 进程管理器
//!
//! 管理多个 pi agent 子进程的生命周期：spawn、kill、事件转发。
//!
//! # 架构
//!
//! 每个 `spawn()` 调用创建一个 `PiRpcBackend` 实例，启动 pi 子进程，
//! 取得 `PiRpcClient` 后将 `event_rx` 取出并转发到统一的 `event_tx` channel，
//! 由主循环 `tokio::select!` 统一消费。

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc;

use super::event::PiEvent;
use super::rpc::PiRpcBackend;
use super::rpc_client::PiRpcClient;
use crate::errors::Result;

/// 单个 agent 进程的完整句柄
///
/// 包含会话标识、文件路径、子进程后端和 RPC 客户端。
pub struct AgentProcess {
    /// 会话 ID（工作区树中的 session id）
    pub session_id: String,
    /// JSONL 会话文件路径
    pub session_path: PathBuf,
    /// pi 子进程后端
    pub backend: PiRpcBackend,
    /// RPC 通信客户端
    pub client: PiRpcClient,
}

/// 多 agent 进程管理器
///
/// 生命周期：
/// - `spawn()` → 启动 pi 子进程，注册事件转发
/// - `kill()` → 停止进程并移除
/// - `kill_all()` → 退出时停止所有进程
///
/// 通过 `event_tx` channel 将每个 agent 的 RPC 事件汇入统一的 event stream，
/// 主循环从中消费处理。
pub struct AgentManager {
    agents: HashMap<String, AgentProcess>,
}

impl AgentManager {
    /// 创建空的管理器实例
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// 为指定会话启动 pi agent 进程
    ///
    /// `event_tx` 用于将 agent 的 RPC 事件转发到主循环的统一事件流。
    /// `cwd` 是 pi 子进程的工作目录（通常为项目根目录）。
    pub async fn spawn(
        &mut self,
        session_id: String,
        session_path: PathBuf,
        cwd: PathBuf,
        event_tx: mpsc::UnboundedSender<(String, PiEvent)>,
    ) -> Result<()> {
        let mut backend = PiRpcBackend::new("pi", &cwd);
        let session_path_str = session_path.to_string_lossy().to_string();
        let mut client = backend.start(Some(&session_path_str)).await?;

        // event_rx 是 mpsc::UnboundedReceiver，不支持广播/重订阅。
        // 取出所有权的 event_rx，替换为哑接收器（不会被使用）。
        let (dummy_tx, dummy_rx) = mpsc::unbounded_channel();
        // 丢弃 dummy_tx 以关闭 dummy_rx，防止泄漏
        let forwarded_rx = std::mem::replace(&mut client.event_rx, dummy_rx);
        drop(dummy_tx);

        // 将 event_rx 转发到统一 event stream
        let sid = session_id.clone();
        tokio::spawn(async move {
            let mut rx = forwarded_rx;
            while let Some(event) = rx.recv().await {
                if event_tx.send((sid.clone(), event)).is_err() {
                    // event_tx 关闭 = 主循环退出，停止转发
                    break;
                }
            }
        });

        self.agents.insert(
            session_id.clone(),
            AgentProcess {
                session_id,
                session_path,
                backend,
                client,
            },
        );

        Ok(())
    }

    /// 停止指定会话的 pi agent 进程
    ///
    /// 移除进程句柄并调用 `stop()` 依次执行 kill + wait。
    pub async fn kill(&mut self, session_id: &str) {
        if let Some(mut proc) = self.agents.remove(session_id) {
            let _ = proc.backend.stop().await;
        }
    }

    /// 退出时停止所有 agent 进程
    pub async fn kill_all(&mut self) {
        for (_sid, mut proc) in self.agents.drain() {
            let _ = proc.backend.stop().await;
        }
    }

    /// 检查指定会话是否有活跃 agent
    pub fn is_active(&self, session_id: &str) -> bool {
        self.agents.contains_key(session_id)
    }

    /// 获取指定 agent 的 RPC client
    ///
    /// 用于状态轮询时向特定 agent 发送 request（如 get_state、get_session_stats）。
    pub fn client_mut(&mut self, session_id: &str) -> Option<&mut PiRpcClient> {
        self.agents.get_mut(session_id).map(|p| &mut p.client)
    }

    /// 遍历所有活跃 agent 的可变引用
    ///
    /// 用于批量状态轮询等需要遍历所有 agent 的场景。
    pub fn agents_mut(&mut self) -> impl Iterator<Item = &mut AgentProcess> {
        self.agents.values_mut()
    }
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_manager_new_is_empty() {
        let mgr = AgentManager::new();
        assert!(!mgr.is_active("any-session"));
    }

    #[test]
    fn test_agent_manager_default_is_empty() {
        let mgr = AgentManager::default();
        assert!(!mgr.is_active("test"));
    }

    #[test]
    fn test_agents_mut_empty() {
        let mut mgr = AgentManager::new();
        assert_eq!(mgr.agents_mut().count(), 0);
    }

    #[test]
    fn test_kall_all_empty_is_noop() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut mgr = AgentManager::new();
            mgr.kill_all().await;
            assert!(!mgr.is_active("any"));
        });
    }
}
