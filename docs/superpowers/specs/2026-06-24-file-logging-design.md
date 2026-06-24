# 文件日志系统设计

> 状态：设计定稿 | 日期：2026-06-24

## 目标

取消 tracing 控制台（stderr）输出，改为写入结构化 JSON 文件日志。

## 需求

| 维度 | 决策 |
|------|------|
| 输出目标 | 仅文件，不输出到 stderr |
| 日志目录 | `$XDG_DATA_HOME/meta-tui/logs/`（Linux 默认 `~/.local/share/meta-tui/logs/`） |
| 文件命名 | `meta-tui.YYYY-MM-DD.jsonl` |
| 滚动策略 | 按日滚动（`tracing-appender::rolling::daily`） |
| 保留策略 | 最近 7 天，启动时自动清理 |
| 格式 | 结构化 JSON Lines（每行一个 JSON 对象） |
| 级别控制 | 默认 `info`，`RUST_LOG` 环境变量覆盖 |

## 架构

```
main()
  └─ logging::init()
       ├── dirs::data_dir() → 计算 $XDG_DATA_HOME/meta-tui/logs/
       ├── create_dir_all() → 确保目录存在
       ├── cleanup_old_logs() → 清理 7 天前的 .jsonl 文件
       ├── RollingFileAppender::builder() … .filename_suffix("jsonl") → 创建每日滚动 FileAppender
       ├── tracing_appender::non_blocking() → 包装为非阻塞 writer + WorkerGuard
       └── tracing_subscriber::fmt()
            .json()              → JSON Lines 格式
            .with_writer(...)    → 写入非阻塞 appender
            .with_env_filter()   → RUST_LOG / 默认 info
            .init()              → 注册全局 subscriber
```

## 改动清单

### `Cargo.toml`

```diff
+dirs = "6"
+tracing-appender = "0.2"
-tracing-subscriber = { version = "0.3", features = ["env-filter"] }
+tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

### `src/logging.rs` — 重写

```rust
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// 持有 WorkerGuard，确保程序退出前日志缓冲区全部刷新到磁盘。
pub struct LogGuard {
    _guard: WorkerGuard,
}

/// 初始化文件日志系统。
pub fn init() -> io::Result<LogGuard> {
    let log_dir = xdg_log_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    cleanup_old_logs(&log_dir, Duration::from_secs(7 * 24 * 3600));

    let file_appender = tracing_appender::rolling::Builder::new()
        .directory(&log_dir)
        .file_prefix("meta-tui")
        .filename_suffix("jsonl")
        .build();
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

fn xdg_log_dir() -> io::Result<std::path::PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法获取 XDG 数据目录"))?;
    Ok(base.join("meta-tui").join("logs"))
}

fn cleanup_old_logs(log_dir: &Path, max_age: Duration) {
    let cutoff = SystemTime::now() - max_age;
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            // 只清理 .jsonl 后缀的普通文件
            let is_jsonl = entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "jsonl");
            if !is_jsonl {
                continue;
            }
            let is_old = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|t| t < cutoff);
            if is_old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
```

### `src/main.rs`

```diff
- meta_tui::logging::init()?;
+ let _log_guard = meta_tui::logging::init()?;
```

### 不修改的文件

- `src/lib.rs` — 无变更
- 所有使用 `tracing::*!` 宏的模块 — 宏签名不变，透明切换

## 日志输出示例

```jsonl
{"timestamp":"2026-06-24T14:32:05.123456Z","level":"INFO","target":"meta_tui::tui","message":"pi 已安装: pi v2.4.0"}
{"timestamp":"2026-06-24T14:32:06.234567Z","level":"DEBUG","target":"meta_tui::app","message":"切换到会话: abc123-session"}
{"timestamp":"2026-06-24T14:32:10.345678Z","level":"WARN","target":"meta_tui::tui","message":"事件流结束"}
{"timestamp":"2026-06-24T14:32:12.456789Z","level":"ERROR","target":"meta_tui::tui","message":"pi CLI 未安装或不在 PATH 中"}
```

查询示例：

```bash
# 只看错误
jq 'select(.level=="ERROR")' ~/.local/share/meta-tui/logs/meta-tui.2026-06-24.jsonl

# 统计各模块日志量
jq -r '.target' ~/.local/share/meta-tui/logs/*.jsonl | sort | uniq -c | sort -rn
```

## 设计决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 返回 `LogGuard` | 是 | `non_blocking` 的 `WorkerGuard` 必须存活，否则日志丢失 |
| 7 天清理在启动时 | 是 | 简单，无需后台计时器 |
| `io::Result` 而非 `color_eyre::Result` | 是 | logging 模块不依赖 `color-eyre` |
| 不保留 stderr 输出 | 是 | 需求明确 |
| `.jsonl` 后缀 | 是 | 准确描述 JSON Lines 格式，编辑器友好 |

## 风险与边界

- `dirs::data_dir()` 在非标准 Linux 环境可能返回 `None` → `init()` 返回 `Err`，`main()` 以错误退出
- 日志目录创建失败 → 同上
- `cleanup_old_logs()` 基于文件修改时间，非精确语义（系统时间调整可能跳过某些旧文件）→ 影响极小，可接受
- 无文件大小告警 —— 阶段后续按需添加
