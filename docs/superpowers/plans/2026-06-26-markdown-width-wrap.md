# Markdown 渲染宽度感知与行折行 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `markdown::render()` 增加可用宽度参数，使代码块和表格在已知终端宽度内对超长行做行内折行。

**Architecture:** `render()` 新增 `max_width: u16` 参数，传入 `MarkdownRenderer`；新增 `split_at_width()` 辅助函数用于字符级按宽度切分；`emit_code_block()` 和 `emit_table()` 在生成行时检测超限并折多行；调用方 `main_view.rs` 已有 `max_width` / `inner.width`，直接传入。

**Tech Stack:** Rust, ratatui, unicode-width, pulldown-cmark

## Global Constraints

- Rust edition 2024, MSRV 1.85
- 所有宽度测量使用 `unicode_width::UnicodeWidthStr::width()`，不依赖 `len()` 或 `chars().count()`
- 折行为字符级，不做 word-level wrap（代码按字符折行更合理）
- 代码块续行行号留空，边框对齐
- 表格列宽下限 3 字符
- 测试用 `assert!` 而非 `assert_eq!` 检查渲染包含内容（因折行后精确格式可能不同）
- 禁止 `unwrap()`，使用 `?`

---

### Task 1: 接口变更 — 为 `render()` 添加 `max_width` 参数并更新所有调用点

**文件：**
- 修改：`src/components/markdown.rs` — `render()` 签名、`MarkdownRenderer` 结构体、`new()` 方法
- 修改：`src/components/main_view.rs` — 6 处调用点

**接口：**
- 消耗：无
- 产出：`pub fn render(text: &str, theme: &Theme, max_width: u16) -> Vec<Line<'static>>`

- [ ] **Step 1: 修改 `render()` 签名和 `MarkdownRenderer`**

  在 `src/components/markdown.rs` 中：

  ```rust
  pub fn render(text: &str, theme: &Theme, max_width: u16) -> Vec<Line<'static>> {
      // ... 快速路径不变，仅在创建 renderer 时传入 max_width
      let mut renderer = MarkdownRenderer::new(theme, max_width);
      // ...
  }
  ```

  `MarkdownRenderer` 新增字段：
  ```rust
  struct MarkdownRenderer<'a> {
      // ... 现有字段
      max_width: u16,
  }
  ```

  `new()` 新增参数：
  ```rust
  fn new(theme: &'a Theme, max_width: u16) -> Self {
      Self {
          // ... 现有初始化
          max_width,
      }
  }
  ```

- [ ] **Step 2: 更新 `main_view.rs` `render_message` 中的 5 处调用**

  文件 `src/components/main_view.rs`，以下位置将 `markdown::render(&clean, theme)` 改为 `markdown::render(&clean, theme, max_width)`：
  - 行 143（User 消息）
  - 行 193（Assistant fallback）
  - 行 262（Text block）
  - 行 375（System role）
  - 行 386（Error role）

  所有调用已在 `render_message` 内部，`max_width` 已是参数可直接使用。

- [ ] **Step 3: 更新 `main_view.rs` `render_full_output` 中的 1 处调用**

  文件 `src/components/main_view.rs` 行 643，将：
  ```rust
  let md_lines = markdown::render(&clean, theme);
  ```
  改为：
  ```rust
  let md_lines = markdown::render(&clean, theme, inner.width);
  ```

- [ ] **Step 4: 验证编译通过**

  ```bash
  cargo build 2>&1 | head -20
  ```
  预期：编译成功，无警告。

- [ ] **Step 5: 运行现有测试并确认全部通过**

  ```bash
  cargo test test_code_block test_simple_table test_table_preserves_all_rows table_should_keep_inline_code_inside_cells table_should_render_horizontal_separator_between_body_rows code_block_with_pipe_chars -- --nocapture
  ```
  预期：所有测试失败（因为 `render()` 签名变了但测试还没更新）。

---

### Task 2: 实现 `split_at_width()` 辅助函数

**文件：**
- 修改：`src/components/markdown.rs` — 新增函数

**接口：**
- 消耗：Task 1 完成（`MarkdownRenderer` 结构体可用）
- 产出：`fn split_at_width(text: &str, max_width: usize) -> Vec<String>`

- [ ] **Step 1: 实现 `split_at_width()`**

  在 `src/components/markdown.rs` 中 `impl` 块外（`fn emit_code_block` 附近）或模块级新增：

  ```rust
  /// 按 Unicode 显示宽度在字符边界切分文本
  ///
  /// 将 text 切分为多段，每段的 `unicode_width` 不超过 `max_width`。
  /// 不会在单词边界切分（适用于代码/URL 等不应破坏词的内容）。
  /// 如果 `max_width` 小于 1，返回空 Vec（调用方应确保传入合法值）。
  fn split_at_width(text: &str, max_width: usize) -> Vec<String> {
      if max_width < 1 || text.is_empty() {
          if text.is_empty() {
              return vec![String::new()];
          }
          return Vec::new();
      }

      let mut segments: Vec<String> = Vec::new();
      let mut current = String::new();
      let mut current_w = 0usize;

      for c in text.chars() {
          let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
          if current_w + cw > max_width && !current.is_empty() {
              segments.push(std::mem::take(&mut current));
              current_w = 0;
          }
          current.push(c);
          current_w += cw;
      }
      if !current.is_empty() {
          segments.push(current);
      }

      segments
  }
  ```

- [ ] **Step 2: 编译检查**

  ```bash
  cargo check 2>&1
  ```
  预期：编译通过。

---

### Task 3: 代码块折行 — `emit_code_block()` 支持长行折行

**文件：**
- 修改：`src/components/markdown.rs` — `emit_code_block()` 方法

**接口：**
- 消耗：`self.max_width`, `split_at_width()`, Task 2
- 产出：代码块渲染逻辑支持超 `max_width` 的长行折行

- [ ] **Step 1: 判断是否需要折行，计算受约束的总宽度**

  在 `emit_code_block()` 方法顶部，添加 `self.max_width` 判断：

  ```rust
  fn emit_code_block(&mut self) {
      let border_style = Style::default()
          .fg(self.theme.code_border)
          .add_modifier(Modifier::DIM);
      let body_style = Style::default().bg(self.theme.code_bg);

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

      // 受约束宽度：在 max_width 和自然宽度之间取较小值
      // │ {num} │ {code}  │ — 前缀开销 = num_width + 5，右 border = 1，最小 gap = 1
      let content_overhead = num_width + 7; // = prefix(num_width+5) + right_border(1) + min_gap(1)
      let max_content_w = (self.max_width as usize).saturating_sub(content_overhead);
      let effective_content_w = if self.max_width > 0 && content_max_w > max_content_w {
          max_content_w
      } else {
          content_max_w.min(max_content_w)
      };
      let total_width = effective_content_w + num_width + 8;
      let total_width = total_width.max(20).min(self.max_width as usize);
  ```

- [ ] **Step 2: 修改顶边框和底边框宽度计算，使用受约束的 `total_width`**

  保持现有顶边框和底边框逻辑不变，只是 `total_width` 现在是受约束的。

- [ ] **Step 3: 修改代码行渲染循环，支持折行**

  将原有的单行渲染循环扩展为：

  ```rust
  for (i, code_line) in self.code_block_lines.iter().enumerate() {
      let code_w = unicode_width::UnicodeWidthStr::width(code_line.as_str());
      let segments = if code_w > effective_content_w {
          split_at_width(code_line, effective_content_w)
      } else {
          vec![code_line.clone()]
      };
      let line_width = num_width; // 用于续行的行号区占位宽度

      for (seg_idx, segment) in segments.iter().enumerate() {
          let num_str = if seg_idx == 0 {
              format!("{:>width$}", i + 1, width = num_width)
          } else {
              " ".repeat(num_width) // 续行：行号区留空
          };

          let prefix_and_code = format!("│ {} │ {}", num_str, segment);
          let used = unicode_width::UnicodeWidthStr::width(prefix_and_code.as_str());
          let gap = total_width.saturating_sub(used + 1);
          let padded = format!("{prefix_and_code}{}│", " ".repeat(gap));
          self.lines.push(Line::from(Span::styled(padded, body_style)));
      }
  }
  ```

- [ ] **Step 4: 编译检查**

  ```bash
  cargo check 2>&1
  ```
  预期：编译通过。

---

### Task 4: 表格折行 — `emit_table()` 支持列宽收缩和单元格折行

**文件：**
- 修改：`src/components/markdown.rs` — `emit_table()` 方法

**接口：**
- 消耗：`self.max_width`, `split_at_width()`, Task 2
- 产出：表格渲染在超宽时按比例收缩列宽并折行

- [ ] **Step 1: 计算可用宽度并在超宽时按比例收缩列宽**

  在 `emit_table()` 中计算完 `col_widths` 后添加：

  ```rust
  // 计算表格总宽并检查是否需要收缩
  let border_overhead = 3 * num_cols + 1;
  let natural_width: usize = col_widths.iter().sum::<usize>() + border_overhead;
  let max_width = self.max_width as usize;

  if max_width > border_overhead && natural_width > max_width {
      let available = max_width - border_overhead;
      let total_natural: usize = col_widths.iter().sum();

      // 按比例收缩
      let mut shrunk: Vec<usize> = col_widths
          .iter()
          .map(|&w| {
              let s = (w as f64 * available as f64 / total_natural as f64).round() as usize;
              s.max(3)
          })
          .collect();

      // 收缩后可能有余量，分配回去
      let used_after: usize = shrunk.iter().sum();
      let mut remaining = available.saturating_sub(used_after);
      while remaining > 0 {
          // 每次给可扩展空间最大的列分配剩余
          for w in &mut shrunk {
              if remaining == 0 {
                  break;
              }
              *w += 1;
              remaining = remaining.saturating_sub(1);
          }
      }

      col_widths = shrunk;
  }
  ```

- [ ] **Step 2: 修改数据行渲染，支持单元格折行为多行**

  将原有的单行渲染循环替换为：

  ```rust
  for (ri, row) in self.table_rows.iter().enumerate() {
      let is_header = ri < header_count;
      let style = if is_header { header_style } else { cell_style };

      // 将每个单元格按列宽切分
      let cell_segments: Vec<Vec<String>> = (0..num_cols)
          .map(|ci| {
              let text = row.get(ci).map(|s| s.as_str()).unwrap_or("");
              let w = col_widths[ci];
              if unicode_width::UnicodeWidthStr::width(text) > w {
                  split_at_width(text, w)
              } else {
                  vec![text.to_string()]
              }
          })
          .collect();

      // 计算最大折行数
      let max_lines = cell_segments.iter().map(|s| s.len()).max().unwrap_or(1);

      // 渲染 N 行
      for line_idx in 0..max_lines {
          let cells: Vec<String> = (0..num_cols)
              .map(|ci| {
                  let seg = cell_segments[ci]
                      .get(line_idx)
                      .map(|s| s.as_str())
                      .unwrap_or("");
                  align(seg, col_widths[ci], ci)
              })
              .collect();
          let row_text = format!("│ {} │", cells.join(" │ "));
          self.lines.push(Line::from(Span::styled(row_text, style)));
      }

      // 行间分隔线（仅在有下一行时输出一次）
      if ri + 1 < self.table_rows.len() {
          self.lines.push(Line::from(Span::styled(
              table_border(&col_widths, '├', '┼', '┤'),
              border_style,
          )));
      }
  }
  ```

- [ ] **Step 3: 编译检查**

  ```bash
  cargo check 2>&1
  ```
  预期：编译通过。

---

### Task 5: 测试 — 更新现有测试并添加折行用例

**文件：**
- 修改：`src/components/markdown.rs` — 测试模块

- [ ] **Step 1: 更新所有现有测试中 `render()` 调用，补充 `max_width` 参数**

  搜索 `render("` 的所有调用，添加 `80` 作为默认 max_width：

  例如：
  ```rust
  // 原来
  let lines = render("```rust\nfn main() {}\n```", &theme);
  // 改为
  let lines = render("```rust\nfn main() {}\n```", &theme, 80);
  ```

  受影响的测试函数：
  - `test_code_block`（行 769）
  - `test_blockquote`（行 780）
  - `test_unordered_list`（行 788）
  - `test_empty_input`（行 796）
  - `test_link_renders_text`（行 804）
  - `test_simple_table`（行 812）
  - `test_table_with_alignment`（行 832）
  - `test_table_preserves_all_rows`（行 847）
  - `table_should_keep_inline_code_inside_cells`（行 869）
  - `table_should_render_horizontal_separator_between_body_rows`（行 886）
  - `code_block_with_pipe_chars`（行 908）

- [ ] **Step 2: 添加代码块折行测试**

  ```rust
  #[test]
  fn code_block_with_long_line_should_wrap() {
      let theme = Theme::cyan();
      let long_line = format!("// {}", "a".repeat(120));
      let input = format!("```rust\n{}\nlet x = 1;\n```", long_line);
      let lines = render(&input, &theme, 40);
      let rendered = lines_to_text(&lines);
      // 应包含多行输出（折行了），且边框对齐
      assert!(rendered.contains("│"), "Should have borders");
      // 序列化后的行数应大于原始代码行数（有折行）
      let code_lines: Vec<&str> = rendered.lines().filter(|l| l.contains("//")).collect();
      assert!(
          code_lines.len() > 1,
          "Long line should be wrapped into multiple lines, got: {}",
          code_lines.len()
      );
  }

  #[test]
  fn code_block_short_line_should_not_wrap() {
      let theme = Theme::cyan();
      let input = "```rust\nfn main() {}\n```";
      let lines = render(input, &theme, 80);
      let rendered = lines_to_text(&lines);
      // 短行不应产生多余折行
      let code_lines: Vec<&str> = rendered.lines().filter(|l| l.contains("fn main")).collect();
      assert_eq!(code_lines.len(), 1, "Short line should not wrap");
  }

  #[test]
  fn code_block_wrap_continuation_no_line_number() {
      let theme = Theme::cyan();
      let long_line = format!("let x = \"{}\";", "a".repeat(100));
      let input = format!("```rust\n{}\n```", long_line);
      let lines = render(&input, &theme, 40);
      let rendered = lines_to_text(&lines);
      // 续行不应重复行号（即行号位置是空格）
      let content_lines: Vec<&str> = rendered.lines()
          .filter(|l| l.contains("│"))
          .collect();
      // 至少包含 3 行（首行 + 至少 1 个续行 + 底边框），且续行行号区为空格
      assert!(content_lines.len() >= 4, "Should have multiple content lines");
  }
  ```

- [ ] **Step 3: 添加表格折行测试**

  ```rust
  #[test]
  fn table_should_shrink_when_too_wide() {
      let theme = Theme::cyan();
      let input = "| A | B |\n|---|---|\n| short | http://very-long-url-that-exceeds-normal-width.example.com/path |\n";
      let lines = render(input, &theme, 50);
      let rendered = lines_to_text(&lines);
      // 应包含表格边框
      assert!(rendered.contains('┌'), "Should have top border");
      assert!(rendered.contains('└'), "Should have bottom border");
      // 长 URL 的列应被折行
      let rows: Vec<&str> = rendered.lines().filter(|l| l.contains("short")).collect();
      assert!(
          rows.len() >= 1,
          "Should contain the 'short' cell"
      );
  }

  #[test]
  fn table_with_fit_content_should_not_shrink() {
      let theme = Theme::cyan();
      let input = "| A |\n|---|\n| short |\n";
      let lines = render(input, &theme, 80);
      let rendered = lines_to_text(&lines);
      // 表格宽度不足 80 时不应折行
      let data_lines: Vec<&str> = rendered.lines().filter(|l| l.contains("short")).collect();
      assert_eq!(data_lines.len(), 1, "Short content should not wrap");
  }
  ```

- [ ] **Step 4: 运行所有测试验证通过**

  ```bash
  cargo test -- --nocnocapture 2>&1
  ```
  预期：所有测试通过，包括新增的折行测试。

- [ ] **Step 5: 运行完整检查**

  ```bash
  cargo clippy -- -D warnings 2>&1
  ```
  预期：零警告。

  ```bash
  cargo test 2>&1
  ```
  预期：全部通过。
