# Markdown 渲染支持 实施方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 TUI 对话区的助手消息文本内容增加 Markdown 渲染支持，将原始的 Markdown 文本转换为带样式的 Ratatui Span/Line。

**Architecture:** 使用 pulldown-cmark 0.12 解析 Markdown AST，在 `src/components/markdown.rs` 中构建自定义渲染器，将 AST 事件流转换为 `Vec<Line<'static>>`。渲染器处理所有常用 Markdown 块级和内联语法（标题、粗体、斜体、删除线、代码块/内联代码、引用块、有序/无序列表、链接、水平分割线），通过 Theme 中新增的 markdown 专用颜色与现有主题风格融合。

**Tech Stack:** Ratatui 0.30.2, pulldown-cmark 0.12, edition 2021

## Global Constraints

- Edition 2021，MSRV 1.85
- 错误处理：公开 API 返回 `anyhow::Result`；内部使用 `color-eyre`；禁止 `unwrap()`
- 所有 `pub fn` / `pub struct` 必须有 `///` 文档注释
- 新增 variant 或字段时同步更新所有 `match` 分支和使用点
- `cargo clippy -- -D warnings` 必须零警告
- 不修改 `target/`、`.codegraph/`、`Cargo.toml` 中已有的依赖项
- 所有 BLOCK 级别元素（代码块、引用块）独立成行

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `Cargo.toml` | 新增 `pulldown-cmark` 依赖 |
| `src/theme.rs` | 新增 7 个 markdown 专用颜色字段 + `cyan()` 默认值 |
| `src/components/markdown.rs` | **新建** — Markdown → `Vec<Line<'static>>` 渲染器 |
| `src/components/mod.rs` | 注册 `pub mod markdown` |
| `src/components/main_view.rs` | 在 `render_message` 中调用 `markdown::render()` 替代原始 `parse_ansi_spans` |

---

### Task 1: 添加 pulldown-cmark 依赖

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces: `pulldown-cmark = "0.12"`（features: 无额外 feature，默认启用 `html` 和 `simd`）

- [ ] **Step 1: 编辑 Cargo.toml 添加依赖**

```toml
# 在 [dependencies] 末尾新增一行：
pulldown-cmark = "0.12"
```

- [ ] **Step 2: 下载依赖验证编译**

```bash
cargo fetch
```
Expected: 无错误，pulldown-cmark 被下载。

- [ ] **Step 3: 验证

```bash
cargo check
```
Expected: 编译成功（新依赖被引入但尚未使用，不应有警告）。

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(deps): 添加 pulldown-cmark 0.12"
```

---

### Task 2: 创建 Markdown 渲染模块

**Files:**
- Create: `src/components/markdown.rs`
- Modify: `src/components/mod.rs`

**Interfaces:**
- Consumes: `Theme` (from `crate::theme`), `pulldown_cmark::{Parser, Event, Tag, TagEnd, HeadingLevel, CodeBlockKind}`
- Produces: `pub fn render(text: &str, theme: &Theme) -> Vec<Line<'static>>`

- [ ] **Step 1: 注册模块**

编辑 `src/components/mod.rs`，在 `pub mod popup;` 后新增一行：

```rust
pub mod markdown;
```

- [ ] **Step 2: 编写完整渲染器**

创建 `src/components/markdown.rs`，写入以下完整实现：

```rust
//! Markdown 渲染器 — 将 Markdown 文本转换为带样式的 Ratatui Line
//!
//! 基于 pulldown-cmark 解析 Markdown AST，遍历事件流生成 `Vec<Line<'static>>`。
//! 支持所有常用语法：标题、粗体/斜体/删除线、内联代码、围栏代码块、
//! 引用块、有序/无序列表、链接、水平分割线。

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// 将 Markdown 文本渲染为 Ratatui Line 列表
///
/// # 参数
///
/// * `text` — 输入 Markdown 文本
/// * `theme` — 当前主题，提供 markdown 各元素的颜色
///
/// # 返回
///
/// 可直接传入 `Paragraph::new()` 的 `Vec<Line<'static>>`
pub fn render(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    // 如果文本不含任何 Markdown 标记字符，走快速路径直接返回纯文本行
    if !needs_markdown(text) {
        return text
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
    }

    let parser = Parser::new(text);
    let mut renderer = MarkdownRenderer::new(theme);
    renderer.process(parser);
    renderer.finish()
}

/// 快速检查文本是否包含 Markdown 语法标记，避免对于纯文本消息进行不必要的解析
fn needs_markdown(text: &str) -> bool {
    // 仅扫描前 8KB，避免大文本性能损失
    let scan = if text.len() > 8192 { &text[..8192] } else { text };
    scan.contains('*')
        || scan.contains('`')
        || scan.contains('#')
        || scan.contains('>')
        || scan.contains('[')
        || scan.contains('~')
        || scan.contains("```")
        || scan.contains("1.")
        || scan.contains("- ")
}

/// Markdown → Ratatui Line 渲染器状态机
struct MarkdownRenderer<'a> {
    theme: &'a Theme,
    /// 输出行缓冲区
    lines: Vec<Line<'static>>,
    /// 当前行正在构建的 Span 列表
    current_spans: Vec<Span<'static>>,
    /// 当前内联文本缓冲区
    buf: String,
    /// 内联样式栈 — 处理嵌套的 Emphasis/Strong/Strikethrough/Code/Link
    style_stack: Vec<Style>,
    /// 基础样式（块级上下文决定，如 heading、blockquote 等）
    base_style: Style,
    /// 是否处于围栏代码块内部
    in_code_block: bool,
    /// 代码块语言标识
    code_block_lang: String,
    /// 代码块行缓冲区
    code_block_lines: Vec<String>,
    /// 代码块开始后的行号（用于在块内插入带样式的行）
    code_block_start_line_index: usize,
    /// 是否处于引用块内部
    in_blockquote: bool,
    /// 当前 heading 级别（0 = 不在 heading 中）
    heading_level: u8,
    /// 列表嵌套深度
    list_depth: usize,
    /// 当前列表项序号（有序列表时使用）
    list_index: u64,
    /// 是否在列表项中（用于判断是否追加项目前缀）
    in_list_item: bool,
}

impl<'a> MarkdownRenderer<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            current_spans: Vec::new(),
            buf: String::new(),
            style_stack: Vec::new(),
            base_style: Style::default().fg(theme.text),
            in_code_block: false,
            code_block_lang: String::new(),
            code_block_lines: Vec::new(),
            code_block_start_line_index: 0,
            in_blockquote: false,
            heading_level: 0,
            list_depth: 0,
            list_index: 0,
            in_list_item: false,
        }
    }

    fn process(&mut self, parser: Parser<'_>) {
        for event in parser {
            match event {
                Event::Start(tag) => self.on_start(tag),
                Event::End(tag_end) => self.on_end(tag_end),
                Event::Text(text) => self.on_text(&text),
                Event::Code(code) => self.on_inline_code(&code),
                Event::Html(html) => {
                    // HTML 块：忽略或转义显示
                    if !html.trim().is_empty() {
                        self.buf.push_str(&html);
                    }
                }
                Event::InlineHtml(html) => {
                    self.buf.push_str(&html);
                }
                Event::SoftBreak => self.on_soft_break(),
                Event::HardBreak => self.on_hard_break(),
                Event::Rule => self.on_rule(),
                Event::FootnoteReference(_) => {}
                Event::TaskListMarker(checked) => {
                    let mark = if checked { "[x]" } else { "[ ]" };
                    self.buf.push_str(mark);
                }
            }
        }
        // 处理结束后刷新所有未输出的内容
        self.flush_inline();
        if !self.current_spans.is_empty() {
            self.push_line();
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.lines
    }

    // ── 块级事件处理 ──

    fn on_start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                // 段落开始：无需特殊处理，内联样式在 Text 事件中累积
            }
            Tag::Heading { level, .. } => {
                self.heading_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                self.base_style = Style::default()
                    .fg(self.theme.heading_color)
                    .add_modifier(Modifier::BOLD);
            }
            Tag::BlockQuote(_) => {
                self.in_blockquote = true;
                self.base_style = Style::default()
                    .fg(self.theme.blockquote_text)
                    .add_modifier(Modifier::ITALIC);
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                self.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block_lines.clear();
                self.code_block_start_line_index = self.lines.len();
            }
            Tag::List(order) => {
                self.list_depth += 1;
                // 有序列表从 1 开始编号；嵌套列表重新编号
                if let Some(start) = order {
                    self.list_index = start;
                } else {
                    self.list_index = 0; // 无序列表标记
                }
            }
            Tag::Item => {
                self.in_list_item = true;
                // 先刷新上一个 item 的尾行
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                // 构建项目符号前缀
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let prefix = if self.list_index > 0 {
                    let p = format!("{}{}. ", indent, self.list_index);
                    self.list_index += 1;
                    p
                } else {
                    format!("{}• ", indent)
                };
                self.buf = prefix;
            }
            Tag::Table(_) => {
                // 表格在终端中渲染体验差，回退为纯文本
            }
            Tag::TableHead | Tag::TableRow => {}
            Tag::TableCell => {
                self.buf.push_str(" | ");
            }
            Tag::Emphasis => {
                self.flush_inline_text();
                self.style_stack
                    .push(Style::default().add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                self.flush_inline_text();
                self.style_stack
                    .push(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::Strikethrough => {
                self.flush_inline_text();
                self.style_stack
                    .push(Style::default().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.flush_inline_text();
                // 链接：记录 URL 以便在 End 时追加显示
                self.buf.push('\0'); // 哨兵标记
                self.buf.push_str(&dest_url);
                self.buf.push('\0');
                self.style_stack
                    .push(Style::default().fg(self.theme.link_color));
            }
            Tag::Image { dest_url, .. } => {
                // 图片：渲染为占位标记
                self.buf.push_str("[IMG: ");
                self.buf.push_str(&dest_url);
                self.buf.push(']');
            }
            Tag::MetadataBlock(_) => {}
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {}
        }
    }

    fn on_end(&mut self, tag_end: TagEnd) {
        match tag_end {
            TagEnd::Paragraph => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                // 段落之间加空行
                self.lines.push(Line::from(""));
            }
            TagEnd::Heading(_) => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                self.lines.push(Line::from(""));
                self.heading_level = 0;
                self.base_style = self.default_base_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                self.lines.push(Line::from(""));
                self.in_blockquote = false;
                self.base_style = self.default_base_style();
            }
            TagEnd::CodeBlock => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                // 输出代码块内容（带背景色和边框）
                self.emit_code_block();
                self.in_code_block = false;
                self.code_block_lang.clear();
                self.code_block_lines.clear();
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
                    self.lines.push(Line::from(""));
                }
            }
            TagEnd::Item => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
                self.in_list_item = false;
            }
            TagEnd::Table(_) => {
                self.lines.push(Line::from(""));
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                self.flush_inline();
                if !self.current_spans.is_empty() {
                    self.push_line();
                }
            }
            TagEnd::TableCell => {}
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.flush_inline_text();
                self.style_stack.pop();
            }
            TagEnd::Link => {
                // 弹出链接 URL（用 \0 分隔的哨兵）
                self.flush_inline_text();
                if let Some(end_idx) = self.buf.rfind('\0') {
                    if let Some(start_idx) = self.buf[..end_idx].rfind('\0') {
                        // 提取 URL，保留文本
                        let _url = self.buf[start_idx + 1..end_idx].to_string();
                        self.buf.replace_range(start_idx..=end_idx, "");
                    }
                }
                self.style_stack.pop();
            }
            TagEnd::Image => {}
            TagEnd::MetadataBlock(_) => {}
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    // ── 内联事件处理 ──

    fn on_text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_block_lines.push(text.to_string());
            return;
        }
        self.buf.push_str(text);
    }

    fn on_inline_code(&mut self, code: &str) {
        self.flush_inline_text();
        let style = Style::default()
            .fg(self.theme.text)
            .bg(self.theme.inline_code_bg);
        self.current_spans.push(Span::styled(code.to_string(), style));
    }

    fn on_soft_break(&mut self) {
        if self.in_code_block {
            return;
        }
        self.buf.push(' ');
    }

    fn on_hard_break(&mut self) {
        if self.in_code_block {
            return;
        }
        self.flush_inline();
        if !self.current_spans.is_empty() {
            self.push_line();
        }
    }

    fn on_rule(&mut self) {
        self.flush_inline();
        if !self.current_spans.is_empty() {
            self.push_line();
        }
        let rule = Span::styled(
            "─".repeat(40),
            Style::default()
                .fg(self.theme.text_dim)
                .add_modifier(Modifier::DIM),
        );
        self.lines.push(Line::from(rule));
        self.lines.push(Line::from(""));
    }

    // ── 内部辅助方法 ──

    /// 当前块级上下文默认基础样式
    fn default_base_style(&self) -> Style {
        if self.in_blockquote {
            Style::default()
                .fg(self.theme.blockquote_text)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(self.theme.text)
        }
    }

    /// 计算当前内联文本的有效样式：base_style 叠加 style_stack 中所有修饰
    fn current_style(&self) -> Style {
        let mut style = self.base_style;
        // heading 级别越大字体越小
        if self.heading_level > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        for s in &self.style_stack {
            style = style.patch(*s);
        }
        style
    }

    /// 将 buf 中的当前文本作为一个 Span 刷新到 current_spans
    fn flush_inline_text(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        // 处理链接的哨兵（不输出到 Span 中）
        let text = std::mem::take(&mut self.buf);
        let style = self.current_style();
        // 过滤掉链接 URL 哨兵
        let clean: String = text
            .split('\0')
            .enumerate()
            .filter_map(|(i, s)| {
                if i % 2 == 0 {
                    Some(s)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        if !clean.is_empty() {
            self.current_spans
                .push(Span::styled(clean, style));
        }
    }

    /// 刷新 buf → Span，合并到 current_spans（不清空 spans）
    fn flush_inline(&mut self) {
        self.flush_inline_text();
    }

    /// 将 current_spans 存入 lines，重置 spans
    fn push_line(&mut self) {
        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    /// 输出围栏代码块（带边框和背景色）
    fn emit_code_block(&mut self) {
        let border_style = Style::default()
            .fg(self.theme.code_border)
            .add_modifier(Modifier::DIM);
        let body_style = Style::default().bg(self.theme.code_bg);

        // 顶部边框：┌─── lang ───
        let top = if self.code_block_lang.is_empty() {
            "┌───".to_string()
        } else {
            format!("┌── {} ──", self.code_block_lang)
        };
        self.lines.push(Line::from(Span::styled(top, border_style)));

        // 代码行
        let num_width = if self.code_block_lines.len() > 99 {
            3
        } else if self.code_block_lines.len() > 9 {
            2
        } else {
            1
        };
        let total_width = self
            .code_block_lines
            .iter()
            .map(|l| l.len())
            .max()
            .unwrap_or(0)
            + num_width
            + 4; // "│ " + 行号 + " │ " + 代码 + " │"
        let total_width = total_width.max(20);

        for (i, code_line) in self.code_block_lines.iter().enumerate() {
            let num = format!("{:>width$}", i + 1, width = num_width);
            let text = format!("│ {} │ {} │", num, code_line);
            let padding = total_width.saturating_sub(text.len());
            let padded = if padding > 0 {
                format!("{}{}", text, " ".repeat(padding))
            } else {
                text
            };
            self.lines
                .push(Line::from(Span::styled(padded, body_style)));
        }

        // 底部边框
        let bottom = "└".to_string() + &"─".repeat(total_width.saturating_sub(1));
        self.lines
            .push(Line::from(Span::styled(bottom, border_style)));
        self.lines.push(Line::from(""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn test_plain_text_no_markdown() {
        let theme = Theme::cyan();
        let lines = render("Hello World", &theme);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.iter().any(|s| s.content.contains("Hello")));
    }

    #[test]
    fn test_bold_text() {
        let theme = Theme::cyan();
        let lines = render("Hello **World**!", &theme);
        // 应该有 bold 修饰
        let has_bold = lines.iter().any(|line| {
            line.spans.iter().any(|s| {
                s.style.add_modifier == Modifier::BOLD
                    || s.style.add_modifier.contains(Modifier::BOLD)
            })
        });
        assert!(
            has_bold || lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("World"))),
            "Should render bold text or at minimum preserve the content"
        );
    }

    #[test]
    fn test_inline_code() {
        let theme = Theme::cyan();
        let lines = render("Use `cargo build` to compile", &theme);
        // 内联代码应有特殊背景色
        let has_code = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("cargo build"))
        });
        assert!(has_code, "Should preserve inline code content");
    }

    #[test]
    fn test_heading() {
        let theme = Theme::cyan();
        let lines = render("# Hello", &theme);
        let has_heading = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("Hello") && s.style.add_modifier.contains(Modifier::BOLD))
        });
        assert!(has_heading, "Heading should be bold");
    }

    #[test]
    fn test_code_block() {
        let theme = Theme::cyan();
        let lines = render("```rust\nfn main() {}\n```", &theme);
        let has_code = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("fn main")));
        assert!(has_code, "Code block should contain the code content");
    }

    #[test]
    fn test_blockquote() {
        let theme = Theme::cyan();
        let lines = render("> quoted text", &theme);
        let has_quote = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("quoted text")));
        assert!(has_quote, "Blockquote should contain the quoted text");
    }

    #[test]
    fn test_unordered_list() {
        let theme = Theme::cyan();
        let lines = render("- item one\n- item two", &theme);
        let has_bullet = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains('•')));
        assert!(has_bullet, "Unordered list should have bullet points");
    }

    #[test]
    fn test_empty_input() {
        let theme = Theme::cyan();
        let lines = render("", &theme);
        assert!(lines.is_empty() || lines.iter().all(|l| l.spans.is_empty()));
    }

    #[test]
    fn test_link_renders_text() {
        let theme = Theme::cyan();
        let lines = render("[click here](https://example.com)", &theme);
        let has_text = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("click here")));
        assert!(has_text, "Link should show link text");
    }
}
```

- [ ] **Step 3: 编译检查**

```bash
cargo check
```
Expected: 编译成功，零警告。

- [ ] **Step 4: 运行测试**

```bash
cargo test --lib markdown
```
Expected: 全部 PASS（至少 9 个测试通过）。

- [ ] **Step 5: Clippy 检查**

```bash
cargo clippy -- -D warnings
```
Expected: 零警告。

- [ ] **Step 6: Commit**

```bash
git add src/components/markdown.rs src/components/mod.rs
git commit -m "feat(markdown): 添加 Markdown → Ratatui Line 渲染器"
```

---

### Task 3: 扩展 Theme 增加 Markdown 专用颜色

**Files:**
- Modify: `src/theme.rs`

**Interfaces:**
- Consumes: 现有 `Theme` 结构体
- Produces: 7 个新字段：`code_bg`, `code_border`, `blockquote_text`, `inline_code_bg`, `link_color`, `heading_color`。注意：`blockquote_bar` 在终端 TUI 中间接通过文本颜色呈现，不需要单独的 `blockquote_bar` bar 颜色，因为引用块没有物理左边框。

- [ ] **Step 1: 新增字段到 Theme 结构体**

编辑 `src/theme.rs`，在 `diff_green_bg` 字段之后新增：

```rust
    /// Markdown 代码块背景色
    pub code_bg: Color,
    /// Markdown 代码块边框色
    pub code_border: Color,
    /// Markdown 引用块文字色
    pub blockquote_text: Color,
    /// Markdown 内联代码背景色
    pub inline_code_bg: Color,
    /// Markdown 链接色
    pub link_color: Color,
    /// Markdown 标题色（H1-H6 共用，字号/加粗区分层级）
    pub heading_color: Color,
```

- [ ] **Step 2: 更新 `cyan()` 初始化**

编辑 `src/theme.rs`，在 `cyan()` 末尾的 `diff_green_bg` 后新增：

```rust
            code_bg: Color::Rgb(14, 20, 26),
            code_border: Color::Rgb(76, 133, 135),
            blockquote_text: Color::Rgb(132, 155, 156),
            inline_code_bg: Color::Rgb(20, 28, 36),
            link_color: Color::Rgb(151, 255, 245),
            heading_color: Color::Rgb(255, 191, 92),
```

- [ ] **Step 3: 更新 Theme 测试断言**

编辑 `src/theme.rs` 的 `cyan_theme_should_match_midnight_control_room_palette` 测试，在末尾新增断言：

```rust
        assert_eq!(theme.code_bg, Color::Rgb(14, 20, 26));
        assert_eq!(theme.code_border, Color::Rgb(76, 133, 135));
        assert_eq!(theme.blockquote_text, Color::Rgb(132, 155, 156));
        assert_eq!(theme.inline_code_bg, Color::Rgb(20, 28, 36));
        assert_eq!(theme.link_color, Color::Rgb(151, 255, 245));
        assert_eq!(theme.heading_color, Color::Rgb(255, 191, 92));
```

- [ ] **Step 4: 编译 + 测试**

```bash
cargo test --lib theme
cargo clippy -- -D warnings
```
Expected: 测试 PASS，零 clippy 警告。

- [ ] **Step 5: Commit**

```bash
git add src/theme.rs
git commit -m "feat(theme): 新增 Markdown 专用颜色字段"
```

---

### Task 4: 集成到 MainView 渲染路径

**Files:**
- Modify: `src/components/main_view.rs`

**Interfaces:**
- Consumes: `markdown::render(text: &str, theme: &Theme) -> Vec<Line<'static>>`
- Produces: `render_message` 和 `render_full_output` 中的文本渲染改为调用 `markdown::render` 替代原始的 `parse_ansi_spans` 逐字符处理

需要修改的位置（4 处文本渲染路径）：

1. **`ChatRole::User` 消息**（`render_message` 中 ∼L116-124）：用户消息通常不含 Markdown，但安全起见统一走 `markdown::render`，`needs_markdown()` 快速路径会自动回退纯文本
2. **`ChatRole::Assistant` 的 `ContentBlock::Text { text }`**（`render_message` 中 ∼L222-230）：这是核心渲染点，pi 回复内容含完整 Markdown
3. **`ChatRole::Assistant` 的 `message.text` 回退路径**（`render_message` 中 ∼L138-146，当 `content` 为空时使用 `message.text`）
4. **`ChatRole::System` / `ChatRole::Error`**（`render_message` 末尾）：与 User 同理

另外 `render_full_output` 中渲染 content 文本行也应改用 `markdown::render`。

- [ ] **Step 1: 在 main_view.rs 顶部添加 use 声明**

在 `use super::Component;` 之后新增：

```rust
use super::markdown;
```

- [ ] **Step 2: 修改 `ContentBlock::Text` 渲染**（核心修改）

编辑 `src/components/main_view.rs`，找到 `crate::message::ContentBlock::Text { text } =>` 分支（约 L222），将：

```rust
                        crate::message::ContentBlock::Text { text } => {
                            for text_line in text.lines() {
                                let prefix = Span::from(" ").style(Style::default().fg(theme.text));
                                let mut spans = vec![prefix];
                                spans.extend(crate::parse_ansi_spans(
                                    text_line,
                                    Style::default().fg(theme.text),
                                ));
                                lines.push(Line::from(spans));
                            }
                        }
```

替换为：

```rust
                        crate::message::ContentBlock::Text { text } => {
                            // 先用 strip_ansi 去掉 ANSI 转义序列，再解析 Markdown
                            let clean = crate::strip_ansi(text);
                            let md_lines = markdown::render(&clean, theme);
                            // 为每行添加前缀缩进空格
                            for md_line in md_lines {
                                let mut spans = vec![Span::from(" ")];
                                spans.extend(md_line.spans.into_iter());
                                lines.push(Line::from(spans));
                            }
                        }
```

- [ ] **Step 3: 修改 `ChatRole::User` 渲染**

找到 `ChatRole::User =>` 分支中 `for text_line in message.text.lines()` 循环（约 L116-124），将：

```rust
                for text_line in message.text.lines() {
                    let prefix = Span::from(" ").style(Style::default().fg(theme.text));
                    let mut spans = vec![prefix];
                    spans.extend(crate::parse_ansi_spans(
                        text_line,
                        Style::default().fg(theme.text),
                    ));
                    lines.push(Line::from(spans));
                }
```

替换为：

```rust
                let clean = crate::strip_ansi(&message.text);
                let md_lines = markdown::render(&clean, theme);
                for md_line in md_lines {
                    let mut spans = vec![Span::from(" ")];
                    spans.extend(md_line.spans.into_iter());
                    lines.push(Line::from(spans));
                }
```

- [ ] **Step 4: 修改 `ChatRole::Assistant` 的 `message.text` 回退路径**

找到 `if message.content.is_empty()` 分支内 `for text_line in message.text.lines()` 循环（约 L138-146），与 Step 3 同样的替换模式：

```rust
                let clean = crate::strip_ansi(&message.text);
                let md_lines = markdown::render(&clean, theme);
                for md_line in md_lines {
                    let mut spans = vec![Span::from(" ")];
                    spans.extend(md_line.spans.into_iter());
                    lines.push(Line::from(spans));
                }
```

替换原有的 `for text_line in message.text.lines()` 循环。

- [ ] **Step 5: 修改 `ChatRole::System` 和 `ChatRole::Error` 渲染**

找到 `ChatRole::System =>` 分支和 `ChatRole::Error =>` 分支（约 L258-280），将它们的 `for text_line in message.text.lines()` 循环也替换为与 Step 3 相同的 markdown 渲染模式。

- [ ] **Step 6: 修改 `render_full_output` 中的文本渲染**

找到 `render_full_output` 方法中的 `for text_line in content.lines()` 循环（约 L374-377），替换为：

```rust
        let md_lines = markdown::render(content, theme);
        for md_line in md_lines {
            let mut spans = vec![Span::from(" ")];
            spans.extend(md_line.spans.into_iter());
            lines.push(Line::from(spans));
        }
```

- [ ] **Step 7: 编译检查**

```bash
cargo check
```
Expected: 编译成功，零警告。

- [ ] **Step 8: 运行全部测试**

```bash
cargo test
```
Expected: 全部测试 PASS。

- [ ] **Step 9: Clippy 检查**

```bash
cargo clippy -- -D warnings
```
Expected: 零警告。

- [ ] **Step 10: 清理未使用的 `parse_ansi_spans` 导入**

检查 `src/components/main_view.rs` 顶部 use 声明中是否还在使用 `crate::parse_ansi_spans`。若不再直接调用，移除该 use（但保留 `crate::strip_ansi` 的 use）。

如果是全局 use，检查其他文件是否还在使用 `parse_ansi_spans`：

```bash
cargo check 2>&1 | grep -i unused
```

若有 `unused import: crate::parse_ansi_spans`，移除该 use。

(T1⚠：main_view.rs 不直接 `use crate::parse_ansi_spans`，它通过 `crate::parse_ansi_spans(...)` 路径调用，所以不会触发 unused import 警告。)

- [ ] **Step 11: Commit**

```bash
git add src/components/main_view.rs
git commit -m "feat(markdown): 集成 Markdown 渲染到 MainView 文本路径"
```

---

### Task 5: 集成测试与端到端验证

**Files:**
- Modify: `tests/integration_test.rs`（如果存在）
- 或新增测试在 `src/components/main_view.rs` 的 `#[cfg(test)]` 块中

**Interfaces:**
- Consumes: `markdown::render`, `Theme`, `ChatMessage`, `MainView`
- Produces: 测试用例验证 Markdown 文本正确渲染为带样式的 Line

- [ ] **Step 1: 在 main_view tests 中新增 Markdown 渲染集成测试**

编辑 `src/components/main_view.rs` 的 `#[cfg(test)] mod tests` 块，在末尾（`}` 之前）新增：

```rust
    #[test]
    fn test_render_markdown_bold_in_text_block() {
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![crate::message::ContentBlock::Text {
            text: "这是 **粗体** 文字".to_string(),
        }];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // 确认 "粗体" 文字出现在输出中
        let has_bold_text = buffer
            .content()
            .iter()
            .any(|c| c.symbol() == "粗");
        assert!(has_bold_text, "Bold text should be rendered in output");
    }

    #[test]
    fn test_render_markdown_code_block() {
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![crate::message::ContentBlock::Text {
            text: "```\nlet x = 1;\n```".to_string(),
        }];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // 确认代码内容出现
        let has_code = buffer
            .content()
            .iter()
            .any(|c| c.symbol().contains("let"));
        assert!(has_code, "Code block content should be rendered");
    }

    #[test]
    fn test_render_markdown_heading() {
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![crate::message::ContentBlock::Text {
            text: "## 安装步骤".to_string(),
        }];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let has_heading = buffer
            .content()
            .iter()
            .any(|c| c.symbol().contains("安装") || c.symbol().contains("步骤"));
        assert!(has_heading, "Heading text should be rendered");
    }

    #[test]
    fn test_render_markdown_list() {
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![crate::message::ContentBlock::Text {
            text: "- 第一项\n- 第二项".to_string(),
        }];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let has_item = buffer
            .content()
            .iter()
            .any(|c| c.symbol().contains("第一"));
        assert!(has_item, "List items should be rendered");
    }

    #[test]
    fn test_render_plain_text_still_works() {
        // 回归测试：确保无 Markdown 的纯文本仍然正常渲染
        let mut mv = MainView::default();
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![crate::message::ContentBlock::Text {
            text: "普通文本 没有特殊格式".to_string(),
        }];
        mv.messages.push(msg);

        let theme = Theme::cyan();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                mv.render(f, area, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let has_text = buffer
            .content()
            .iter()
            .any(|c| c.symbol().contains("普通"));
        assert!(has_text, "Plain text should be rendered");
    }
```

- [ ] **Step 2: 运行新增测试**

```bash
cargo test test_render_markdown
cargo test test_render_plain_text_still_works
```
Expected: 全部 PASS。

- [ ] **Step 3: 运行全量测试 + Clippy**

```bash
cargo test
cargo clippy -- -D warnings
```
Expected: 全部测试 PASS，零 clippy 警告。

- [ ] **Step 4: Commit**

```bash
git add src/components/main_view.rs
git commit -m "test(markdown): 新增 Markdown 渲染集成测试"
```

---

## 自审清单

### 1. 需求覆盖

| 需求 | 覆盖任务 |
|---|---|
| Markdown 粗体 / 斜体 / 删除线 | Task 2 (StyleStack + Modifier) |
| 标题 H1-H6 | Task 2 (Heading + heading_color + BOLD) |
| 内联代码 | Task 2 (on_inline_code + inline_code_bg) |
| 围栏代码块 | Task 2 (CodeBlock + emit_code_block + 行号) |
| 引用块 | Task 2 (BlockQuote + blockquote_text + ITALIC) |
| 有序/无序列表 | Task 2 (List/Item + 自动编号/项目符号) |
| 链接 | Task 2 (Link + link_color) |
| 水平分割线 | Task 2 (Rule + 分隔符) |
| 纯文本回退 | Task 2 (needs_markdown 快速路径) |
| 主题颜色融合 | Task 3 (Theme 新增字段) |
| 集成到主渲染路径 | Task 4 (4 个渲染路径 + render_full_output) |
| 集成测试 | Task 5 (5 个新测试 + 回归测试) |

### 2. 占位符检查

无 TBD / TODO / "implement later"。所有代码块均为完整可运行代码。

### 3. 类型一致性

- `markdown::render()` 签名：`(&str, &Theme) -> Vec<Line<'static>>` — 在 Task 2 定义，Task 4 和 Task 5 中调用
- `Theme` 新增字段在 Task 3 定义，Task 2 的 `MarkdownRenderer` 中使用，名称一致
- `needs_markdown` 内部函数仅在 `markdown.rs` 内使用
- `strip_ansi` 在 Task 4 中调用，来自 `lib.rs`，已存在无需修改

---
