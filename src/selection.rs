//! 鼠标拖拽选中工具
//!
//! 提供对 Lines 应用选中高亮并收集选中文本的通用函数。
//! 高亮只打在选中范围内的字符上，而非整行或整个 Span。

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::app::SelectionState;
use crate::theme::Theme;

/// 对准备渲染的 Lines 应用选中高亮，并将选中文本写入 `selection.selected_text`。
///
/// `lines` 将被直接修改：只有选中范围内的字符会获得 highlight_bg 背景。
/// `area` 是这些 Lines 的渲染区域（左上角坐标）。
/// `selection` 是当前的选区状态（终端坐标）。
pub fn apply_selection(
    lines: &mut [Line<'static>],
    area: Rect,
    selection: &mut SelectionState,
    theme: &Theme,
) {
    let (a_col, a_row) = match selection.anchor {
        Some(v) => v,
        None => return,
    };
    let (f_col, f_row) = match selection.focus {
        Some(v) => v,
        None => return,
    };
    if a_col == f_col && a_row == f_row {
        return;
    }

    let sel_y1 = a_row.min(f_row);
    let sel_y2 = a_row.max(f_row);

    // 选区完全不在此区域，跳过
    if sel_y2 < area.y || sel_y1 >= area.y + area.height {
        return;
    }

    selection.selected_text.clear();
    let sel_x1 = a_col.min(f_col);
    let sel_x2 = a_col.max(f_col);

    let mut first_line = true;

    for (i, line) in lines.iter_mut().enumerate() {
        let line_y = area.y + i as u16;
        if line_y < sel_y1 || line_y > sel_y2 {
            continue;
        }

        if !first_line {
            selection.selected_text.push('\n');
        }
        first_line = false;

        // 展平该行所有字符（char + 显示宽度 + 原始样式）
        struct CharInfo {
            ch: char,
            width: u16,
            style: Style,
        }
        let mut chars: Vec<CharInfo> = Vec::new();
        for span in &line.spans {
            for ch in span.content.chars() {
                let w = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
                chars.push(CharInfo {
                    ch,
                    width: w,
                    style: span.style,
                });
            }
        }
        // 收集选中文本
        let mut line_text = String::new();
        let mut col = area.x;
        for ci in &chars {
            let col_start = col;
            let col_end = col + ci.width.saturating_sub(1);
            if col_end >= sel_x1 && col_start <= sel_x2 {
                line_text.push(ci.ch);
            }
            col += ci.width;
        }
        selection.selected_text.push_str(&line_text);

        // 重建该行 spans：每个字符一个 Span，选中部分加背景
        let mut new_spans: Vec<Span<'static>> = Vec::new();
        let mut col = area.x;
        for ci in &chars {
            let col_start = col;
            let col_end = col + ci.width.saturating_sub(1);
            let in_sel = col_end >= sel_x1 && col_start <= sel_x2;
            let style = if in_sel {
                ci.style.bg(theme.highlight_bg)
            } else {
                ci.style
            };
            new_spans.push(Span::styled(ci.ch.to_string(), style));
            col += ci.width;
        }
        line.spans = new_spans;
    }
}
