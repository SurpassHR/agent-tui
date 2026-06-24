use std::io;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::EnvFilter;

/// 持有 WorkerGuard，确保程序退出前日志缓冲区全部刷新到磁盘。
pub struct LogGuard {
    _guard: WorkerGuard,
}

/// 初始化文件日志系统。
///
/// - 目录：`$XDG_DATA_HOME/meta-tui/logs/`
/// - 文件：`meta-tui.YYYY-MM-DD.jsonl`，每日滚动
/// - 保留：最近 7 个文件，内置自动清理
/// - 格式：结构化 JSON，每行一条记录
/// - 级别：默认 `info`，通过 `RUST_LOG` 环境变量覆盖
pub fn init() -> io::Result<LogGuard> {
    let log_dir = xdg_log_dir()?;
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("meta-tui")
        .filename_suffix("jsonl")
        .max_log_files(7)
        .build(&log_dir)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .json()
        .with_writer(non_blocking)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    Ok(LogGuard { _guard: guard })
}

/// 计算 XDG 数据目录下的日志路径
fn xdg_log_dir() -> io::Result<std::path::PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法获取 XDG 数据目录"))?;
    Ok(base.join("meta-tui").join("logs"))
}
