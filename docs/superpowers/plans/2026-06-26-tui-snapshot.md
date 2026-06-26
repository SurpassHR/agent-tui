# TUI 快照功能 Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在输入框输入 `/snapshot` 时将当前 TUI 屏幕内容保存到文本文件。

**Architecture:** 在 `tui.rs` Enter 处理器中拦截 `/snapshot`，调用 `terminal.buffer()` 获取当前帧字符网格，逐行拼接后写入 `~/.config/agent-tui/snapshots/`。

**Tech Stack:** Ratatui 0.30.2, Rust edition 2024

## Global Constraints

- 不新增 Action 变体
- 不修改持久化 / 配置
- 不发送给 agent（本地拦截）
- 文档注释：中文

---

### Task 1: 拦截 `/snapshot` 并实现 Buffer 捕获

**Files:**
- Modify: `src/tui.rs`

**Interfaces:**
- Consumes: `Terminal::buffer()` → `&Buffer`, `std::fs::create_dir_all()`, `chrono::Local::now()`
- Produces: 文本文件 + `bottom_bar.status` 反馈

- [ ] **Step 1: 添加 snapshot 保存函数**
  - 在 `tui.rs` 中添加 `fn save_snapshot(terminal: &Terminal<...>) -> Result<String>` 函数
  - 遍历 `terminal.buffer()` 逐行拼接
  - 拼入时间戳头行
  - 调用 `create_dir_all` + `write`

- [ ] **Step 2: 拦截 `/snapshot`，跳过 agent 路由**
  - 在 Enter 处理中判断 `trimmed == "/snapshot"`
  - 调用 `save_snapshot(&terminal)`
  - 设置底部栏状态提示
  - 清空 input_buffer 并 `continue`（不走 agent）

- [ ] **Step 3: 编译验证**

  ```bash
  cd /home/hr/Projects/agent-tui && cargo build 2>&1
  ```

- [ ] **Step 4: 运行集成测试验证编译不影响现有功能**

  ```bash
  cd /home/hr/Projects/agent-tui && cargo test 2>&1
  ```
