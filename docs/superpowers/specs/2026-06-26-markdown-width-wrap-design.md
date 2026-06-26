# Markdown 渲染宽度感知与行折行 — 设计文档

> 日期：2026-06-26 | 状态：草稿

## 概述

为 `markdown::render()` 引入可用宽度参数，使 `emit_code_block()` 和 `emit_table()` 在已知终端宽度内对超长行做行内折行，避免因自然行宽超出终端导致 Ratatui Paragraph 自动折行破坏代码块/表格的固定格式视觉结构。

## 问题背景

当前 `markdown::render(text, theme)` → `Vec<Line<'static>>` 没有宽度参数。代码块和表格按内容自然宽度生成固定格式行（边框、行号、列对齐），然后由调用方通过 `Paragraph::wrap(Wrap)` 显示。

当代码块某行或表格某单元格长度超过终端可用宽度时：

- `emit_code_block()` 以最长行计算 `content_max_w`，所有行 padding 到该宽度，生成超宽代码块
- `emit_table()` 以最宽单元格计算列宽，生成超宽表格
- Ratatui Paragraph 的字符级自动折行打破固定格式（边框/行号/列对齐全乱）

## 目标范围

| 元素 | 当前行为 | 改动后 |
|------|---------|--------|
| 代码块 | 按最长行计算 `content_max_w`，padding 到统一宽度 | 在 `max_width` 内折行，续行行号留空 |
| 表格 | 按最宽单元格计算每列宽度 | 超宽时按比例收缩列宽，单元格折行 |

不涉及：普通段落、标题、列表、引用块等——这些元素没有固定格式，已有 Paragraph wrap 正常工作。

## 接口变更

```diff
-pub fn render(text: &str, theme: &Theme) -> Vec<Line<'static>>
+pub fn render(text: &str, theme: &Theme, max_width: u16) -> Vec<Line<'static>>
```

- `max_width` 为终端可用列数（即 main_view 中 `inner.width`）
- 该值存储在 `MarkdownRenderer` 中，仅在 `emit_code_block()` 和 `emit_table()` 中使用
- 其他元素（标题、段落、列表等）不受影响

## 代码块折行

### 格式分析

代码块每行格式由 `format!("│ {} │ {}", num, code_line)` 生成，续行加 `{gap}│`：
```
│ {num} │ {code_line}{gap}│
```

**前缀开销**（`│ {num} │ ` 部分）：
- `│ `（左侧边框+空格）= 2
- `{num}`（右对齐行号，含前导空格）= num_width
- ` │ `（分隔符）= 3
- **总计** = `num_width + 5`

尾部 `│`（右侧边框）占 1 字符。

**可用 content 宽度**（保留最少 1 字符间隙）：
```
可用 content 宽度 = max_width - num_width - 7
```
实现时以 `max_width - num_width - 6` 为硬上限（gap=0），但尽量留 1 字符空隙。
对于典型 num_width=3 的终端（>9 行代码块）：可用内容 = max_width - 10。

### 折行规则

- 超过可用 content 宽度的行在**字符边界**切分为 N 段（不尝试 word-level、不破坏缩进）
- 首行输出完整行号
- 续行行号位置留空（`│    │`），行号宽度对齐保证边框连续
- 续行内容缩进到与首行缩进对齐宽度（保留前导空白对齐）

**示例**（`max_width=60`, num_width=3）：
```
│  6 │       "evidence": "Added `scroll: usize` to EnteredView::Diff
│    │ and EnteredView::Subagent variants in message.rs; fixed the
│    │ single construction site in app.rs:1339; added `..` to match
│    │ compilation clean (Task 3 will replace them)."
```

### 续行前缀

```
│    │ {segment}
```
其中 `{spaces}` 填充到 `num_width` 宽度（保持行号区对齐）。

### 尾部边框对齐

- 非最后一段：用空格填充到 `max_width - 1` 再输出 `│`，保持右边框对齐
- 最后一段：正常输出尾部 `│` + 填充空格，与当前 `emit_code_block()` 行为一致

## 表格折行

### 宽度分析

表格行格式（由 `format!("│ {} │", cells.join(" │ "))` 生成）：
```
│ cell1 │ cell2 │ ... │ cellN │
```

宽度公式：`row_width = 3n + 1 + Σcol_widths`，其中 n = 列数。

验证（n=3, col_widths=[1,1,1]）：`│ A │ B │ C │` = 3×3+1+3 = 13 ✓

### 收缩策略

1. 计算表格自然宽度：`natural = 3n + 1 + Σcol_widths`
2. 若 `natural ≤ max_width`：不处理，保持原有渲染
3. 若 `natural > max_width`：
   - 可用内容宽度 = `max_width - 3n - 1`
   - 按比例收缩各列：`col_i = max(3, round(col_i * available / Σcol_widths))`
   - 不足整数分配时，剩余宽度轮流分配给收缩率最高的列

### 单元格折行

- 某单元格内容超出 `col_i` 宽度时，在字符边界切分
- 同一行各列取最大折行数 N
- 不足 N 行的列补空行（对齐填充）
- 每行之后输出行间分隔线（已有逻辑）

**示例**（收缩前 col_widths=[10, 40]，收缩后 available=35 → [7, 28]）：
```
│ short    │ very long text that wraps to next │
│          │ line here, continuing the content │
│          │ until it ends somewhere           │
├──────────┼───────────────────────────────────┤
│ another  │ short                             │
```

### 对齐保持

折行段的左/中/右对齐与首行一致。

## 调用链路变更

```
main_view.rs: render() → inner.width (u16)
                   ↓
render_message(msg, theme, self, i, inner.width)
                   ↓
markdown::render(text, theme, inner.width)
                   ↓
MarkdownRenderer { max_width }
                   ↓
        emit_code_block() / emit_table()
```

共约 5 处 `markdown::render()` 调用点（user 消息 1 处、assistant 纯 text 1 处、assistant Text block 1 处、entered_view 2 处），均已持有或可获取 `inner.width`。

## 变更范围

| 文件 | 改动 |
|------|------|
| `src/components/markdown.rs` | `render()` 签名 + `MarkdownRenderer` 新增 `max_width` 字段 + `emit_code_block()` 折行 + `emit_table()` 收缩折行；新增辅助函数 `split_at_width()` 用于通用字符级按宽度切分 |
| `src/components/main_view.rs` | `render_message` 参数已有 `max_width: u16`，改为传入 `render()`；`render_entered` 中补充传入 |
| 测试 | 更新 `render()` 调用处 + 新增代码块/表格折行用例 |

## 边界情况

- **窄终端**：每列下限 3 字符，代码块前缀开销固定，折行后可能单列显示
- **Unicode/CJK**：继续使用 `unicode-width` 计宽，与现有逻辑一致
- **空代码块/空表格**：已有守卫（`code_block_lines.is_empty()`, `table_rows.is_empty()`）
- **行号位数变化**：1/2/3 位时 `num_width` 自动调整，续行行号区对齐跟随
- **表格仅一行**：该行可视作表头，收缩逻辑不变
- **对齐方式**：右对齐/居中对齐的折行段保持原有对齐方向
- **零宽字符/ANSI**：代码块行已由 `strip_ansi` 预处理；计数用 `unicode-width` 而非 `len()`
- **表格行数变化后折行数变少**：`split_at_width` 复用同一函数，确保各列折行段数一致
