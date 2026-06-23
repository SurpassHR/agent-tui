use color_eyre::Result;
use tracing_subscriber::EnvFilter;

/// 初始化 tracing 日志系统
pub fn init() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    Ok(())
}
