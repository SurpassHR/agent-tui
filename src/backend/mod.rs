use async_trait::async_trait;
use tokio::sync::mpsc;

pub mod event;
pub mod rpc;
pub mod rpc_client;

use crate::action::Action;
use crate::errors::Result;

/// 智能体后端抽象接口
///
/// 所有子进程（pi、claude-code、标准 shell 等）的拉起和交互均通过此 trait 抽象。
/// 实现必须满足 `Send + Sync`，以便跨线程传递。
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// 启动智能体子进程
    ///
    /// `tx` 用于将 PTY 输出和退出事件推回主循环。
    /// 实现应在此方法中创建 PTY pair、spawn 子进程、启动读取循环。
    async fn start(&mut self, tx: mpsc::Sender<Action>) -> Result<()>;

    /// 向该智能体的 stdin 写入用户指令
    async fn send_input(&mut self, input: &str) -> Result<()>;

    /// 强制终止该智能体进程
    ///
    /// 必须依次调用 kill() 和 wait()，防止产生僵尸进程。
    async fn terminate(&mut self) -> Result<()>;
}
