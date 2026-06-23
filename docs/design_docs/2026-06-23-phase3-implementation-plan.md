# Phase 3 实施计划

## 总览

将阶段一的 PTY 输出管道接入阶段二的 TUI MainView，用 ansitok 解析 ANSI 颜色，
以真实 pi agent 验证流式渲染。

## 任务分解

```
Task 1: 依赖 + 新模块
  - Cargo.toml: 添加 ansitok
  - src/ansi.rs: 新建 ANSI→Ratatui 转换器（AnsiStyleState 状态机 + parse_to_lines）
  - src/lib.rs: 导出 pub mod ansi

Task 2: MainView 改造
  - 添加 buffer: String / display_pos: usize 字段
  - push_output(chunk): 追加 chunk，推进 display_pos
  - trim_buffer(): 100KB 上限，裁到 50KB（UTF-8 安全边界）
  - render(): 无数据→render_empty, 有数据→ansitok 解析→Paragraph

Task 3: TUI 主循环 + main.rs 启动
  - src/tui.rs: run_tui(mut app) 处理 PtyStdout/PtyExit/q 键退出
  - src/main.rs: TUI 模式 + --target → app.start().await
```

## 边界条件

- 无 PTY 输出: 等待状态
- PTY 退出: 显示 [进程已退出]，按 q 退出
- q 键: 唯一键盘事件，从 TUI 退出
- Headless 模式: 不变
- 6 个集成测试: 全部通过

## 验证

```bash
# 1. 编译 + 测试
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check

# 2. 真实 agent 验证
cargo run -- --tui --target /bin/sh
# 在 shell 里输入: echo "Hello \x1b[31mRed\x1b[0m World"
# 验证 ANSI 彩色输出正确显示
# 按 q 退出

# 3. 验证 headless 不变
cargo run -- --target /bin/sh
```
