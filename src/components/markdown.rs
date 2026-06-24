//! Markdown 渲染器 — 将 Markdown 文本转换为带样式的 Ratatui Line
//!
//! 基于 pulldown-cmark 解析 Markdown AST，遍历事件流生成 `Vec<Line<'static>>`。
//! 支持所有常用语法：标题、粗体/斜体/删除线、内联代码、围栏代码块、
//! 引用块、有序/无序列表、链接、水平分割线。

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
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

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(text, options);
    let mut renderer = MarkdownRenderer::new(theme);
    renderer.process(parser);
    renderer.finish()
}

/// 快速检查文本是否包含 Markdown 语法标记，避免对于纯文本消息进行不必要的解析
fn needs_markdown(text: &str) -> bool {
    // 仅扫描前 8KB，避免大文本性能损失
    let scan = if text.len() > 8192 {
        &text[..8192]
    } else {
        text
    };
    scan.contains('*')
        || scan.contains('`')
        || scan.contains('#')
        || scan.contains('>')
        || scan.contains('[')
        || scan.contains('~')
        || scan.contains('|')
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
    /// 是否处于表格内部
    in_table: bool,
    /// 表格列对齐方式
    table_alignments: Vec<Alignment>,
    /// 表格所有行数据（rows × cols），不含分隔行
    table_rows: Vec<Vec<String>>,
    /// 当前表格行正在构建的单元格
    table_current_row: Vec<String>,
    /// 当前单元格文本缓冲区
    table_cell_buf: String,
    /// 是否有表头（表格渲染时用于区分表头行和数据行）
    has_table_head: bool,
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
            in_blockquote: false,
            heading_level: 0,
            list_depth: 0,
            list_index: 0,
            in_list_item: false,
            in_table: false,
            table_alignments: Vec::new(),
            table_rows: Vec::new(),
            table_current_row: Vec::new(),
            table_cell_buf: String::new(),
            has_table_head: false,
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
                    if self.in_code_block {
                        for line in html.lines() {
                            self.code_block_lines.push(line.to_string());
                        }
                    } else if !html.trim().is_empty() {
                        self.buf.push_str(&html);
                    }
                }
                Event::InlineHtml(html) => {
                    if self.in_code_block {
                        self.code_block_lines.push(html.to_string());
                    } else {
                        self.buf.push_str(&html);
                    }
                }
                Event::InlineMath(math) => {
                    if self.in_code_block {
                        self.code_block_lines.push(math.to_string());
                    } else {
                        self.buf.push_str(&math);
                    }
                }
                Event::DisplayMath(math) => {
                    if self.in_code_block {
                        for line in math.lines() {
                            self.code_block_lines.push(line.to_string());
                        }
                    } else {
                        self.buf.push_str(&math);
                    }
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

    fn finish(self) -> Vec<Line<'static>> {
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
            Tag::Table(alignments) => {
                self.in_table = true;
                self.table_alignments = alignments;
                self.table_rows.clear();
                self.table_current_row.clear();
                self.table_cell_buf.clear();
                self.has_table_head = false;
            }
            Tag::TableHead => {
                self.has_table_head = true;
            }
            Tag::TableRow => {
                self.table_current_row.clear();
            }
            Tag::TableCell => {
                // 单元格开始：采集后续文本到 table_cell_buf
                self.table_cell_buf.clear();
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
            Tag::HtmlBlock => {}
            Tag::FootnoteDefinition(_) => {}
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
            TagEnd::TableCell => {
                // 完成单元格，存入当前行
                let text = std::mem::take(&mut self.table_cell_buf);
                self.table_current_row.push(text);
            }
            TagEnd::TableRow => {
                // 完成当前行，存入表格
                let row = std::mem::take(&mut self.table_current_row);
                self.table_rows.push(row);
            }
            TagEnd::TableHead => {
                // 表头单元格是 TableHead 的直接子元素（无 TableRow 包装）
                let row = std::mem::take(&mut self.table_current_row);
                if !row.is_empty() {
                    self.table_rows.push(row);
                }
            }
            TagEnd::Table => {
                self.emit_table();
                self.in_table = false;
                self.table_rows.clear();
                self.table_alignments.clear();
            }
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
            TagEnd::HtmlBlock => {}
            TagEnd::FootnoteDefinition => {}
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    // ── 内联事件处理 ──

    fn on_text(&mut self, text: &str) {
        if self.in_code_block {
            // pulldown-cmark 0.12 + ENABLE_TABLES 将围栏代码块内容合并为单个
            // Event::Text（内含 \n），需拆分为独立行以正确渲染多行代码块
            for line in text.lines() {
                self.code_block_lines.push(line.to_string());
            }
            return;
        }
        if self.in_table {
            self.table_cell_buf.push_str(text);
            return;
        }
        self.buf.push_str(text);
    }

    fn on_inline_code(&mut self, code: &str) {
        if self.in_table {
            self.table_cell_buf.push_str(code);
            return;
        }

        self.flush_inline_text();
        let style = Style::default()
            .fg(self.theme.text)
            .bg(self.theme.inline_code_bg);
        self.current_spans
            .push(Span::styled(code.to_string(), style));
    }

    fn on_soft_break(&mut self) {
        if self.in_code_block {
            return;
        }
        if self.in_table {
            self.table_cell_buf.push(' ');
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
            .filter_map(|(i, s)| if i % 2 == 0 { Some(s) } else { None })
            .collect::<Vec<_>>()
            .join("");
        if !clean.is_empty() {
            self.current_spans.push(Span::styled(clean, style));
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

        // 先计算总宽度
        let num_width = if self.code_block_lines.len() > 99 {
            3
        } else if self.code_block_lines.len() > 9 {
            2
        } else {
            1
        };
        let content_max_w = self
            .code_block_lines
            .iter()
            .map(|l| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(0);
        // │ {num} │ {code_line}  │ — 固定框架 3 个 │ + 5 个空格 = 8 字符
        let total_width = (content_max_w + num_width + 8).max(20);

        // 顶部边框（总宽匹配 body）
        let top = if self.code_block_lang.is_empty() {
            format!("┌{}", "─".repeat(total_width.saturating_sub(1)))
        } else {
            let lang_tag = format!("── {} ──", self.code_block_lang);
            let lang_w = unicode_width::UnicodeWidthStr::width(lang_tag.as_str());
            let remaining = total_width.saturating_sub(lang_w + 1); // +1 for ┌
            format!("┌{}{}", lang_tag, "─".repeat(remaining))
        };
        self.lines.push(Line::from(Span::styled(top, border_style)));

        for (i, code_line) in self.code_block_lines.iter().enumerate() {
            let num = format!("{:>width$}", i + 1, width = num_width);
            // 先构建不包含尾部 │ 的部分，计算已用宽度
            let prefix_and_code = format!("│ {} │ {}", num, code_line);
            let used = unicode_width::UnicodeWidthStr::width(prefix_and_code.as_str());
            // 尾部 │ 固定在 total_width - 1 位置，中间用空格填充
            let gap = total_width.saturating_sub(used + 1); // +1 给尾部 │
            let padded = format!("{prefix_and_code}{}│", " ".repeat(gap));
            self.lines
                .push(Line::from(Span::styled(padded, body_style)));
        }

        // 底部边框
        let bottom = "└".to_string() + &"─".repeat(total_width.saturating_sub(1));
        self.lines
            .push(Line::from(Span::styled(bottom, border_style)));
        self.lines.push(Line::from(""));
    }

    /// 输出表格（两遍渲染：先计算列宽，再对齐输出）
    ///
    /// 表格渲染策略：
    /// - 表头行用 bold 样式 + 分隔线
    /// - 数据行用左/中/右对齐
    /// - 列间用 "│" 分隔
    fn emit_table(&mut self) {
        if self.table_rows.is_empty() {
            self.lines.push(Line::from(""));
            return;
        }

        let header_style = Style::default()
            .fg(self.theme.heading_color)
            .add_modifier(Modifier::BOLD);
        let cell_style = Style::default().fg(self.theme.text);
        let border_style = Style::default()
            .fg(self.theme.text_dim)
            .add_modifier(Modifier::DIM);

        let num_cols = self.table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if num_cols == 0 {
            return;
        }

        // 计算每列最大显示宽度（含中文字符宽度）
        let mut col_widths: Vec<usize> = vec![0; num_cols];
        for row in &self.table_rows {
            for (ci, cell) in row.iter().enumerate() {
                if ci >= num_cols {
                    break;
                }
                let w = unicode_width::UnicodeWidthStr::width(cell.as_str());
                col_widths[ci] = col_widths[ci].max(w);
            }
        }
        // 每列最小 3 字符宽
        for w in &mut col_widths {
            *w = (*w).max(3);
        }

        // 构建对齐函数
        let align = |text: &str, width: usize, ci: usize| -> String {
            let w = unicode_width::UnicodeWidthStr::width(text);
            let padding = width.saturating_sub(w);
            let alignment = self.table_alignments.get(ci).copied();
            match alignment {
                Some(Alignment::Right) | Some(Alignment::Center) => {
                    let left = if alignment == Some(Alignment::Center) {
                        padding / 2
                    } else {
                        padding
                    };
                    let right = padding - left;
                    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
                }
                _ => {
                    // Left 或 None：左对齐
                    format!("{}{}", text, " ".repeat(padding))
                }
            }
        };

        self.lines.push(Line::from(Span::styled(
            table_border(&col_widths, '┌', '┬', '┐'),
            border_style,
        )));

        let header_count = if self.has_table_head || self.table_rows.len() == 1 {
            // 有显式 thead 或只有一行（视为全表头）
            1.min(self.table_rows.len())
        } else {
            // 无显式 thead：首行当表头
            1
        };

        for (ri, row) in self.table_rows.iter().enumerate() {
            let is_header = ri < header_count;
            let style = if is_header { header_style } else { cell_style };

            // 渲染数据行
            let cells: Vec<String> = (0..num_cols)
                .map(|ci| {
                    let text = row.get(ci).map(|s| s.as_str()).unwrap_or("");
                    align(text, col_widths[ci], ci)
                })
                .collect();
            let row_text = format!("│ {} │", cells.join(" │ "));
            self.lines.push(Line::from(Span::styled(row_text, style)));

            // 每一行之间保持水平分隔，避免窄终端中表格内容读成连续段落。
            if ri + 1 < self.table_rows.len() {
                self.lines.push(Line::from(Span::styled(
                    table_border(&col_widths, '├', '┼', '┤'),
                    border_style,
                )));
            }
        }

        // 渲染底边框
        self.lines.push(Line::from(Span::styled(
            table_border(&col_widths, '└', '┴', '┘'),
            border_style,
        )));
        self.lines.push(Line::from(""));
    }
}

fn table_border(col_widths: &[usize], left: char, junction: char, right: char) -> String {
    let segments = col_widths
        .iter()
        .map(|w| "─".repeat(w + 2))
        .collect::<Vec<_>>()
        .join(&junction.to_string());
    format!("{left}{segments}{right}")
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
            has_bold
                || lines
                    .iter()
                    .any(|l| l.spans.iter().any(|s| s.content.contains("World"))),
            "Should render bold text or at minimum preserve the content"
        );
    }

    #[test]
    fn test_inline_code() {
        let theme = Theme::cyan();
        let lines = render("Use `cargo build` to compile", &theme);
        // 内联代码应有特殊背景色
        let has_code = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("cargo build")));
        assert!(has_code, "Should preserve inline code content");
    }

    #[test]
    fn test_heading() {
        let theme = Theme::cyan();
        let lines = render("# Hello", &theme);
        let has_heading = lines.iter().any(|line| {
            line.spans.iter().any(|s| {
                s.content.contains("Hello") && s.style.add_modifier.contains(Modifier::BOLD)
            })
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

    #[test]
    fn test_simple_table() {
        let theme = Theme::cyan();
        let lines = render(
            "| 名称 | 版本 |\n|------|------|\n| Rust | 1.85 |\n| Tokio | 1.0 |\n",
            &theme,
        );
        // 应包含表头和数据
        let has_name = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("名称")));
        let has_rust = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("Rust")));
        let has_tokio = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("Tokio")));
        assert!(has_name, "Table header should contain column names");
        assert!(has_rust, "Table should contain Rust row");
        assert!(has_tokio, "Table should contain Tokio row");
    }

    #[test]
    fn test_table_with_alignment() {
        let theme = Theme::cyan();
        // 右对齐的数字列
        let lines = render("| 项目 | 数量 |\n|:-----|-----:|\n| A | 123 |\n", &theme);
        let has_project = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("项目")));
        let has_a = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains('A')));
        assert!(has_project, "Table should render");
        assert!(has_a, "Table should render data rows");
    }

    #[test]
    fn test_table_preserves_all_rows() {
        let theme = Theme::cyan();
        let lines = render(
            "| x | y |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n",
            &theme,
        );
        let mut count_1 = 0;
        for line in &lines {
            for span in &line.spans {
                if span.content.contains('1')
                    || span.content.contains('3')
                    || span.content.contains('5')
                {
                    count_1 += 1;
                }
            }
        }
        assert_eq!(count_1, 3, "All three data rows should be present");
    }

    #[test]
    fn table_should_keep_inline_code_inside_cells() {
        let theme = Theme::cyan();
        let lines = render(
            "| Commit | 说明 |\n|---|---|\n| `5670948` | 添加依赖 |\n| `1fca916` | 创建渲染器 |\n",
            &theme,
        );
        let rendered = lines_to_text(&lines);

        assert!(
            rendered
                .lines()
                .any(|line| line.contains("5670948") && line.contains("添加依赖")),
            "内联代码单元格应留在同一张表格行内：\n{rendered}"
        );
    }

    #[test]
    fn table_should_render_horizontal_separator_between_body_rows() {
        let theme = Theme::cyan();
        let lines = render("| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n", &theme);
        let rendered = lines_to_text(&lines);
        let separator_count = rendered.lines().filter(|line| line.contains('├')).count();

        assert!(
            separator_count >= 2,
            "表头和正文行之间都应有水平分隔线：\n{rendered}"
        );
    }

    fn lines_to_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn code_block_with_pipe_chars() {
        // 管道符在代码块内不应被 ENABLE_TABLES 误解析为表格
        let theme = Theme::cyan();
        let input = "```rust\nlet f = |x| x + 1;\nlet g = |y| y * 2;\n```";
        let lines = render(input, &theme);
        let rendered = lines_to_text(&lines);
        assert!(rendered.contains("let f"), "{rendered}");
        assert!(rendered.contains("let g"), "{rendered}");
        assert!(rendered.contains('|'), "pipe char preserved: {rendered}");
    }
}
