use thiserror::Error;

/// Meta-TUI 的自定义错误类型
#[derive(Debug, Error)]
pub enum Error {
    /// PTY 操作错误
    #[error("PTY error: {0}")]
    Pty(String),
    /// I/O 错误
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// 配置错误
    #[error("Config error: {0}")]
    Config(String),
    /// 子进程错误
    #[error("Subprocess error: {0}")]
    Subprocess(String),
    /// 内部通道错误
    #[error("Channel error: {0}")]
    Channel(String),
}

/// 便捷类型别名
pub type Result<T> = std::result::Result<T, Error>;
