# TUI 快照功能设计

## 概述

在输入框中输入 `/snapshot` 命令，将当前 TUI 界面完整画面保存为文本文件。

## 用户流程

1. 用户在输入框输入 `/snapshot` 并按 Enter
2. 不发送给 agent，直接捕获当前终端屏幕内容
3. 保存到 `~/.config/agent-tui/snapshots/snapshot-YYYYMMDD-HHMMSS.txt`
4. 底部栏中央显示保存路径提示

## 实现方案

### 关键技术：Ratatui Buffer 捕获

每次 `terminal.try_draw()` 渲染完成后，terminal 的 viewport 中保存了完整的字符网格（Buffer）。通过 `terminal.buffer()` 可在任意时刻访问上一帧的渲染结果。

### 截获点

在 `tui.rs` 的 Enter 键盘处理器（约第 1920 行）中，当 `input_buffer.trim() == "/snapshot"` 时，不走 agent 路由，直接执行快照逻辑。

### Buffer → 文本转换

```rust
let buf = terminal.buffer();
let mut text = String::new();
text.push_str(&format!("─── Snapshot: {} ───\n\n", now_str));
for y in 0..buf.area.height {
    for x in 0..buf.area.width {
        text.push_str(buf[(x, y)].symbol());
    }
    text.push('\n');
}
```

### 文件保存

- **目录**：`~/.config/agent-tui/snapshots/`
- **命名**：`snapshot-YYYYMMDD-HHMMSS.txt`
- 自动创建目录（`std::fs::create_dir_all`）

### UI 反馈

- 设置 `bottom_bar.status = "📷 Snapshot: <path>"`
- 后续用户操作后自动被覆盖

### 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `src/tui.rs` | Enter 处理中拦截 `/snapshot`，捕获 buffer 写文件 |

### 无需修改

- `src/action.rs` — 不新增 Action 变体
- `src/components/bottom_bar.rs` — `status` 字段已支持
- `src/components/main_view.rs` — 无改动
- 持久化 / 配置 — 无改动

## 边界情况

- **终端尺寸为 0**：不保存，静默忽略
- **权限不足**：`create_dir_all` 失败时打印 `tracing::error`
- **两次连续 `/snapshot`**：文件名含时间戳，不会覆盖
- **CJK 字符**：`cell.symbol()` 返回完整 Unicode 符号，拼接正确
