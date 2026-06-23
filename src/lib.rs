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
