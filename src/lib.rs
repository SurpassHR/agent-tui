//! Meta-TUI: 智能体编排工作台
//!
//! 基于 Rust + Ratatui 的硬核高密度终端（TUI）工作台。
//! 通过 PTY 技术包装各种独立的 CLI 智能体，在不破坏其独立性的前提下，
//! 为其外挂高级能力。
//!
//! # 架构
//!
//! 异步事件驱动模型（Event-Driven MPSC Architecture）：
//!
//! ```text
//! 【真实终端】 ◄── (Crossterm) ──► 【Ratatui TUI 主线程】
//!       ◄── (MPSC Channel) ── 【异步事件/拦截器层】
//!             ◄── (PTY Master) ──► 【子进程: pi agent】
//! ```
//!
//! # 阶段一
//!
//! Headless PTY 隧道拓扑与接口闭环 — 无 UI 渲染，仅验证 I/O 拓扑。

pub mod action;
pub mod app;
pub mod backend;
pub mod components;
pub mod config;
pub mod errors;
pub mod logging;
pub mod message;
pub mod persistence;
pub mod provider;
pub mod selection;
pub mod theme;
pub mod tui;

// 重新导出常用类型
pub use action::Action;
pub use app::App;
pub use backend::AgentBackend;
pub use config::Config;
pub use errors::Error;

/// 剥离 ANSI 转义序列（CSI 序列: ESC [ ... m）
///
/// 用于清理 pi thinking 输出中的终端颜色代码，
/// 避免在 TUI 中直接显示 ANSI 控制字符导致渲染异常。
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // 消费 ESC [...] m 序列
            if let Some(&'[') = chars.peek() {
                chars.next(); // 消费 '['
                              // 消费参数部分（数字和分号）
                while let Some(&nc) = chars.peek() {
                    if nc.is_ascii_digit() || nc == ';' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // 消费终止字符（m 或其它 CSI 终止符）
                if let Some(&nc) = chars.peek() {
                    if nc.is_ascii_alphabetic() {
                        chars.next();
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 将含 ANSI SGR 转义序列的文本行解析为 Ratatui Span 列表
///
/// 返回的每个 Span 携带对应的颜色/样式，可直接用于 `Line::from(spans)` 渲染。
/// `base_style` 作为初始样式（无 ANSI 区域使用此样式）。
pub fn parse_ansi_spans(
    line: &str,
    base_style: ratatui::style::Style,
) -> Vec<ratatui::text::Span<'static>> {
    use ratatui::text::Span;

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = base_style;
    let mut chars = line.chars().peekable();

    let flush = |text: &mut String, style| -> Option<Span<'static>> {
        if text.is_empty() {
            None
        } else {
            let s = Span::styled(std::mem::take(text), style);
            Some(s)
        }
    };

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if let Some(&'[') = chars.peek() {
                chars.next(); // 消费 '['
                              // 刷新当前文本
                if let Some(span) = flush(&mut current_text, current_style) {
                    spans.push(span);
                }
                // 收集 SGR 参数
                let mut params = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc.is_ascii_digit() || nc == ';' {
                        params.push(nc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // 消费终止字符
                if let Some(&nc) = chars.peek() {
                    if nc == 'm' {
                        chars.next();
                        current_style = apply_sgr(&params, current_style, base_style);
                    }
                }
            }
        } else {
            current_text.push(c);
        }
    }

    if let Some(span) = flush(&mut current_text, current_style) {
        spans.push(span);
    }

    spans
}

/// 应用 SGR 参数到当前样式，返回新样式
fn apply_sgr(
    params: &str,
    current: ratatui::style::Style,
    base: ratatui::style::Style,
) -> ratatui::style::Style {
    use ratatui::style::{Color, Modifier};

    let mut style = current;
    // 收集所有参数
    let nums: Vec<u8> = params
        .split(';')
        .filter_map(|s| s.parse::<u8>().ok())
        .collect();

    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => style = base, // reset
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            30..=37 => {
                // 标准前景色
                let color = ansi_16_color(nums[i] - 30);
                style = style.fg(color);
            }
            38 => {
                // 扩展前景色: 38;5;<n> 或 38;2;<r>;<g>;<b>
                if i + 2 < nums.len() && nums[i + 1] == 5 {
                    style = style.fg(ansi_256_color(nums[i + 2]));
                    i += 2;
                } else if i + 4 < nums.len() && nums[i + 1] == 2 {
                    style = style.fg(Color::Rgb(nums[i + 2], nums[i + 3], nums[i + 4]));
                    i += 4;
                }
            }
            39 => style = style.fg(base.fg.unwrap_or(Color::Reset)),
            40..=47 => {
                let color = ansi_16_color(nums[i] - 40);
                style = style.bg(color);
            }
            48 => {
                // 扩展背景色
                if i + 2 < nums.len() && nums[i + 1] == 5 {
                    style = style.bg(ansi_256_color(nums[i + 2]));
                    i += 2;
                } else if i + 4 < nums.len() && nums[i + 1] == 2 {
                    style = style.bg(Color::Rgb(nums[i + 2], nums[i + 3], nums[i + 4]));
                    i += 4;
                }
            }
            49 => style = style.bg(Color::Reset),
            90..=97 => {
                // 亮前景色
                let color = ansi_16_bright_color(nums[i] - 90);
                style = style.fg(color);
            }
            100..=107 => {
                // 亮背景色
                let color = ansi_16_bright_color(nums[i] - 100);
                style = style.bg(color);
            }
            _ => {} // 未知参数忽略
        }
        i += 1;
    }

    // 叠加 dim 效果以统一色调
    style.add_modifier(Modifier::DIM)
}

/// ANSI 16 标准色
fn ansi_16_color(n: u8) -> ratatui::style::Color {
    use ratatui::style::Color;
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        _ => Color::Reset,
    }
}

/// ANSI 16 亮色
fn ansi_16_bright_color(n: u8) -> ratatui::style::Color {
    use ratatui::style::Color;
    match n {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}

/// ANSI 256 色调色板近似
fn ansi_256_color(n: u8) -> ratatui::style::Color {
    use ratatui::style::Color;
    match n {
        0..=7 => ansi_16_color(n),
        8..=15 => ansi_16_bright_color(n - 8),
        16..=231 => {
            // 6×6×6 颜色立方体
            let n = n - 16;
            let r = (n / 36) * 51;
            let g = ((n / 6) % 6) * 51;
            let b = (n % 6) * 51;
            Color::Rgb(r, g, b)
        }
        232..=255 => {
            // 灰度
            let g = 8 + (n - 232) * 10;
            Color::Rgb(g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_colors() {
        let input = "\u{1b}[38;2;138;190;183mThinking:\u{1b}[39m hello";
        let output = strip_ansi(input);
        assert_eq!(output, "Thinking: hello");
    }

    #[test]
    fn test_strip_ansi_bold() {
        let input = "\u{1b}[1mbold\u{1b}[0m text";
        let output = strip_ansi(input);
        assert_eq!(output, "bold text");
    }

    #[test]
    fn test_strip_no_ansi() {
        let input = "plain text";
        let output = strip_ansi(input);
        assert_eq!(output, "plain text");
    }

    #[test]
    fn test_strip_incomplete_escape() {
        // 孤立的 ESC（不是 CSI 序列）
        let input = "text \u{1b} after";
        let output = strip_ansi(input);
        assert_eq!(output, "text  after");
    }

    #[test]
    fn test_parse_ansi_truecolor() {
        use ratatui::style::{Color, Modifier, Style};
        let input = "\u{1b}[38;2;138;190;183mThinking:\u{1b}[39m hello";
        let base = Style::default().fg(Color::White);
        let spans = parse_ansi_spans(input, base);
        assert_eq!(spans.len(), 2, "should have 2 spans");
        // 第一个 span: "Thinking:" 带 TrueColor + DIM
        assert_eq!(spans[0].content, "Thinking:");
        let expected_fg = Color::Rgb(138, 190, 183);
        assert_eq!(spans[0].style.fg, Some(expected_fg));
        assert!(spans[0].style.add_modifier.contains(Modifier::DIM));
        // 第二个 span: " hello" — \u{1b}[39m 后回退到 base fg + DIM
        assert_eq!(spans[1].content, " hello");
        assert_eq!(spans[1].style.fg, Some(Color::White));
    }

    #[test]
    fn test_parse_ansi_reset() {
        use ratatui::style::{Color, Style};
        let input = "\u{1b}[1;31merror\u{1b}[0m ok";
        let base = Style::default().fg(Color::White);
        let spans = parse_ansi_spans(input, base);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "error");
        assert_eq!(spans[1].content, " ok");
        // reset 后应回到 base 样式
        assert_eq!(spans[1].style.fg, Some(Color::White));
    }

    #[test]
    fn test_parse_no_ansi() {
        use ratatui::style::{Color, Style};
        let input = "plain text";
        let base = Style::default().fg(Color::White);
        let spans = parse_ansi_spans(input, base);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "plain text");
        assert_eq!(spans[0].style.fg, Some(Color::White));
    }
}
