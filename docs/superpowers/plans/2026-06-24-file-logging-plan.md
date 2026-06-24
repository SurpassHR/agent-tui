# 文件日志系统 — 实现计划

> 状态：待执行 | 基于 spec: `2026-06-24-file-logging-design.md`

## 任务清单

### Task 1: 依赖与 logging 模块重写

**文件：**
- `Cargo.toml` — 加 `dirs`、`tracing-appender`，`tracing-subscriber` 开 `json` feature
- `src/logging.rs` — 完整重写（文件输出 + JSON + 滚动 + 7 天清理）

**参考：** spec 文档中 "改动清单" 节里的完整代码，照搬即可。

### Task 2: main.rs 适配

**文件：**
- `src/main.rs` — `logging::init()?;` → `let _log_guard = logging::init()?;`

---

## 全局约束

- Rust edition 2024
- 禁止 `unwrap()`，使用 `?`
- 改完后 `cargo build` 和 `cargo clippy -- -D warnings` 必须通过
- 不修改 spec 中 "不修改的文件" 节列出的模块
