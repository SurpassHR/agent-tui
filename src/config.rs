use clap::Parser;

/// Meta-TUI 智能体编排工作台
///
/// 基于 PTY 的无侵入式 AI Agent 多进程宿主与编排器
#[derive(Debug, Clone, Parser)]
#[command(name = "meta-tui", version, about)]
pub struct Config {
    /// 要启动的目标命令（如 pi、/bin/sh）
    #[arg(short = 'T', long = "target", default_value = "/bin/sh")]
    pub target_command: String,

    /// 目标命令的参数（默认空，需自行指定，如 --args -i）
    #[arg(short = 'a', long = "args", num_args = 0..)]
    pub target_args: Vec<String>,

    /// pi CLI 命令路径（仅 TUI RPC 模式使用，默认 "pi"）
    #[arg(long = "pi-path", default_value = "pi")]
    pub pi_command: String,

    /// 是否启用 TUI 模式（阶段二起生效）
    #[arg(long = "tui", default_value_t = false)]
    pub tui_mode: bool,

    /// 诊断模式：查询 pi 状态并打印（不启动 TUI）
    #[arg(long = "diagnose", default_value_t = false)]
    pub diagnose: bool,

    /// 仅打印配置信息（sessions/extensions/skills/mcps），不启动 pi 进程
    #[arg(long = "dry-run", default_value_t = false)]
    pub dry_run: bool,
    /// 独立启动 Provider Router（带配置管理 TUI）
    #[arg(long = "router", default_value_t = false)]
    pub router: bool,
}

impl Config {
    /// 从命令行参数解析配置
    pub fn from_args() -> Self {
        Self::parse()
    }
}
