# Phase 3: 流式 ANSI 解码与动态打字机渲染

> 日期: 2026-06-23 | 状态: 设计评审 | 阶段: 三

## 1. 目标

在阶段二（三栏静态布局）的基础上，将阶段一的 Headless PTY 管道接入 TUI 中间视窗
（MainView），实现流式 ANSI 解码与实时渲染。

### 1.1 范围

**包含：**
- `ansitok` crate 集成，将 ANSI 转义码解析为 Ratatui `Span` / `Style`
- `src/ansi.rs` — ANSI→Ratatui 转换模块
- `MainView` 持有输出缓冲区，实时追加 PTY chunk
- `run_tui()` 处理 `Action::PtyStdout` 推入 MainView
- 缓冲区上限管理（100KB 裁剪）
- PTY 进程退出显示

**不包含（后续阶段）：**
- 键盘事件 → PTY stdin 管道（阶段四）
- 滚动/翻页（阶段四）
- `OutputInterceptor` 特征拦截（阶段四）
- 会话切换（阶段五）

> **注意：** 用户选择了「手动启动」策略，但阶段三尚未实现键盘事件处理。
> 临时方案：`--tui --target /bin/sh` 时自动 `app.start()`（否则无输出可看）；
> 纯 `--tui`（无 `--target`）时停留在等待状态。阶段四添加键盘事件后，
> 统一改为交互式启动。

### 1.2 验收标准

- `cargo run -- --tui` 启动后显示等待状态
- 手动启动后端后，PTY 彩色输出实时显示在 MainView
- ANSI 颜色/修饰在终端中正确复现
- 打字机效果由 PTY chunk 到达速率自然驱动，无额外调速
- 缓冲区超过 100KB 时自动裁剪
- 子进程退出后显示结束标记
- 6 个已有集成测试全部通过
- 无 clippy 警告、格式正确

## 2. 数据流

```
子进程 stdout
    ↓
portable-pty MasterPty::try_clone_reader()
    ↓ (spawn_blocking 死循环读取)
MPSC channel → Action::PtyStdout(chunk)
    ↓
run_tui() 收到 chunk
    ↓
MainView.push_output(&chunk)
    ├── buffer.push_str(chunk)      — 追加到总缓冲区
    └── display_pos = buffer.len()  — 推进显示位置
    ↓
--- 每 250ms tick ---
    ↓
MainView.render()
    ├── 取 buffer[..display_pos]
    ├── parse_to_lines() 用 ansitok 解析
    └── Paragraph::new(lines) → Frame::render_widget
```

## 3. 文件变更清单

| 操作 | 文件 | 说明 |
|------|------|------|
| 修改 | `Cargo.toml` | 添加 `ansitok` 依赖 |
| 新建 | `src/ansi.rs` | ANSI→Ratatui 转换器 |
| 修改 | `src/lib.rs` | 导出 `pub mod ansi` |
| 修改 | `src/components/main_view.rs` | 添加 buffer / display_pos 字段，替换静态占位 |
| 修改 | `src/tui.rs` | `run_tui()` 改为 `mut app`，处理 PtyStdout/PtyExit |
| 修改 | `src/main.rs` | `--tui --target` 时自动 `app.start()`（阶段三临时行为） |
| 修改 | `src/backend/pi.rs` | 检查：PTY chunk 发送前已正确编码 |

## 4. 模块详细设计

### 4.1 `src/ansi.rs`

```rust
/// 将含 ANSI 转义码的文本解析为 Ratatui 行
///
/// 使用 ansitok crate 逐 token 解析：
/// - 内部维护 AnsiStyleState 状态机追踪当前样式
/// - 每个 SGR 代码更新状态机，Text 片段用当前样式生成 Span
/// - 同行的连续 span 合并为 Line
/// - 无 ANSI 码的纯文本直接转 Span::raw
pub fn parse_to_lines(text: &str) -> Vec<Line<'static>>
```

**ANSI SGR 是有状态协议** — 核心设计：在遍历 ansitok `Element` 时维护状态机：

```rust
struct AnsiStyleState {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
}

impl AnsiStyleState {
    /// 解析 VisualAttribute 列表，更新状态
    fn apply(&mut self, attrs: &[VisualAttribute])
    /// 将当前状态转为 Ratatui Style
    fn to_ratatui_style(&self) -> Style
    /// 全部重置（对应 SGR 0）
    fn reset(&mut self)
}
```

处理逻辑：
1. 遍历 `parse_ansi(text)` 返回的 `Element`
2. `ElementKind::Sgr` → 解析 `VisualAttribute` 列表 → `state.apply(attrs)`
3. `ElementKind::Text` → 使用 `state.to_ratatui_style()` 生成 `Span`
4. 未识别的 SGR 片段（`Output::Text`）跳过
5. `\n` 触发换行：当前 `Line` push 到输出，新建空行

**ANSIColor → Ratatui Color 映射：**

| ANSI | Ratatui |
|------|---------|
| Black | `Color::Black` |
| Red | `Color::Red` |
| Green | `Color::Green` |
| Yellow | `Color::Yellow` |
| Blue | `Color::Blue` |
| Magenta | `Color::Magenta` |
| Cyan | `Color::Cyan` |
| White | `Color::White` |
| Bright variants | `Color::Light*` |
| 256-color (38;5;n) | `Color::Indexed(n)` |
| RGB (38;2;r;g;b) | `Color::Rgb(r, g, b)` |

**ANSI Modifier → Ratatui Modifier 映射：**

| ANSI | Ratatui |
|------|---------|
| Bold (1) | `Modifier::BOLD` |
| Dim (2) | `Modifier::DIM` |
| Italic (3) | `Modifier::ITALIC` |
| Underline (4) | `Modifier::UNDERLINED` |
| Reverse (7) | `Modifier::REVERSED` |

> 已知限制（阶段三）：每帧全量重新解析，不缓存。100KB 缓冲区在 250ms 帧间隔下
> 实测可接受。阶段四/五可加入 dirty-flag 增量解析优化。

### 4.2 `src/components/main_view.rs`

```rust
pub struct MainView {
    /// 累计的 PTY 原始输出（含 ANSI 转义码）
    pub buffer: String,
    /// 当前显示到的字符位置
    pub display_pos: usize,
}

impl MainView {
    /// PTY chunk 到达时调用
    pub fn push_output(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);
        self.display_pos = self.buffer.len();
        self.trim_buffer();
    }

    /// 缓冲区上限 100KB，超限时裁到 50KB（留出缓冲空间）
    ///
    /// 注意：drain 必须在 UTF-8 字符边界上，使用 ceil_char_boundary 确保安全。
    fn trim_buffer(&mut self) {
        const MAX_BYTES: usize = 100 * 1024;
        const TRIM_TARGET: usize = 50 * 1024;
        if self.buffer.len() > MAX_BYTES {
            let remove = self.buffer.len() - TRIM_TARGET;
            // 确保截断在 UTF-8 字符边界上，防止 panic
            let boundary = self.buffer[..remove].ceil_char_boundary(remove);
            self.buffer.drain(..boundary);
            // Phase 3: 始终显示末尾
            // Phase 4 需区分 scroll_pos vs write_pos
            self.display_pos = self.buffer.len();
        }
    }
}

impl MainView {
    /// 无数据时的等待状态
    fn render_empty(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(theme.border_dim);

        let lines = vec![
            Line::from(""),
            Line::from(" Meta-TUI Agent Orchestrator".to_string().fg(theme.text)),
            Line::from(""),
            Line::from("  等待 PTY 会话输出...".to_string().dim()),
        ];

        f.render_widget(
            Paragraph::new(lines).block(block).fg(theme.text_dim).wrap(Wrap { trim: false }),
            area,
        );
    }
}

impl Component for MainView {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        if self.buffer.is_empty() {
            return self.render_empty(f, area, theme);
        }

        let visible_text = &self.buffer[..self.display_pos];
        let lines = ansi::parse_to_lines(visible_text);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_dim);

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);
    }
}

/// 本模块在 Phase 3 中的已知局限：
/// - 每帧全量重解析 buffer（无缓存），100KB 量级实测可接受
/// - display_pos 未区分 scroll 偏移（Phase 4 实现滚动时需重构）
/// - 无增量 diff 解析优化（Phase 4/5 可引入）
```

### 4.3 `src/tui.rs`

阶段三在 `tokio::select!` 中添加一个 crossterm 按键监听分支，仅监听 `q` 键退出。
这不是完整的键盘事件系统（留到阶段四），只是确保用户不会被困在 TUI 中。

```rust
use crossterm::event::{self, Event, KeyCode};

/// 非阻塞式检查 q 键是否被按下
async fn check_quit_key() -> bool {
    // 非阻塞：只在有事件就绪时检查
    if event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read() {
            return key.code == KeyCode::Char('q');
        }
    }
    false
}

pub async fn run_tui(mut app: App, mut action_rx: mpsc::Receiver<Action>) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut ticker = tokio::time::interval(Duration::from_millis(250));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let _ = terminal.try_draw(|f| {
                    app.render_tui(f);
                    Ok::<_, std::io::Error>(())
                });
            }
            _ = check_quit_key() => {
                if check_quit_key().await {
                    break;
                }
            }
            Some(action) = action_rx.recv() => {
                match action {
                    Action::PtyStdout(chunk) => {
                        app.tui.main_view.push_output(&chunk);
                    }
                    Action::PtyExit => {
                        app.tui.main_view.push_output("\n[进程已退出]\n");
                        // 继续显示，等待用户按 q 退出
                    }
                    Action::UserSubmitInput(input) => {
                        app.handle_action(action).await?;
                    }
                    _ => {
                        tracing::debug!("TUI 收到未处理的 action: {:?}", action);
                    }
                }
            }
            else => break,
        }
    }

    let _ = ratatui::try_restore();
    Ok(())
}
```

## 5. 边界情况

| 场景 | 处理方式 |
|------|---------|
| 无 PTY 输出 | MainView 显示等待提示 |
| ANSI 码不完整（跨 chunk） | ansitok 处理；若仍乱码，console 输出 raw bytes |
| 缓冲区过大（>100KB） | 裁掉前半，保留最新 50KB |
| 子进程退出 | 追加 `[进程已退出]` 标记，继续展示，等待用户按 q |
| 终端 resize | Ratatui 的 `draw()` 自动处理 |
| 空 chunk 或纯空格 | 正常追加，不影响显示 |
| Headless 模式 | 完全不改动，`run_headless()` 不受影响 |

## 6. 测试策略

### 6.1 集成测试（新增 3 个）

- `test_tui_pty_roundtrip`：启动 TUI + PTY，验证 PtyStdout 被 MainView 接收
- `test_ansi_parsing`：发送含 ANSI 码的文本，验证 parse_to_lines 输出风格正确
- `test_buffer_trim`：写入 150KB 数据，验证裁剪到 100KB

### 6.2 手动测试

```bash
# 启动 TUI + backend
cargo run -- --tui --target /bin/sh

# 在另一个终端发送数据
echo "Hello \x1b[31mRed\x1b[0m World" > /dev/pts/...

# 验证颜色正确显示
# 验证打字机流式效果
# 验证退出后显示结束标记
```

## 7. 依赖变更

```toml
# Cargo.toml 新增
ansitok = "0.3"
```

`ansitok` ≥ 0.3，MIT 许可，纯 Rust，零不安全代码，无额外依赖树增长。

## 8. 风险和注意事项

| 风险 | 缓解措施 |
|------|---------|
| ansitok API 与文档不符 | 查阅 crate 源码确认最新 API 签名 |
| ANSI 跨 chunk 截断 | ansitok 内部状态机容错；若仍残片，在下个 chunk 补全 |
| TUI 模式下没有 stdin→PTY 输入通道 | 阶段四实现 crossterm event → send_input；阶段三可提供简单的 readline 绕过 |
| buffer.drain(..remove) 在 String 上是 O(n) | 100KB 量级可接受；后续可改为 VecDeque<Line> |

## 9. 实施顺序

```
Step 1: 添加 ansitok 依赖到 Cargo.toml
Step 2: 创建 src/ansi.rs 模块
Step 3: 重构 MainView（buffer + display_pos + push_output + trim_buffer）
Step 4: 修改 run_tui() 为 mut app 并处理 PtyStdout/PtyExit
Step 5: 更新 lib.rs 导出 ansi 模块
Step 6: cargo build + cargo test + cargo clippy + cargo fmt
Step 7: 手动验证（cargo run -- --tui --target /bin/sh）
```
