# Meta-TUI: Agent Orchestration Workbench

English | [简体中文](README.zh.md)

> A high-density Terminal (TUI) workbench built with **Rust + Ratatui + Tokio**,
> orchestrating CLI AI agents through RPC technology.

## Overview

Meta-TUI connects to `pi` (or other CLI agents) via **RPC**, providing a three-panel TUI interface:

- **Left panel**: Active session + workspace tree (expandable session list)
- **Center**: Conversation/message area + bottom input box
- **Right panel**: Subagent list (discovered from `~/.pi/agent/agents/*.md`)
- **Bottom bar**: Keyboard shortcuts + status hints + Context/Token/Cost info

Text selection is confined within each panel boundary; releasing the mouse automatically copies the selection to the system clipboard.

## Quick Start

```bash
# Build
cargo build

# Print configuration info (without starting pi)
cargo run -- --dry-run

# Run TUI (automatically starts pi RPC backend)
cargo run -- --tui

# Test
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust (edition 2024, MSRV 1.85) |
| TUI Framework | Ratatui 0.30.2 + Crossterm 0.29 |
| Async Runtime | Tokio |
| RPC | Custom JSONL RPC Protocol |
| Error Handling | color-eyre + thiserror |
| Logging | tracing + tracing-subscriber |

## Project Structure

```
meta-tui/
├── src/
│   ├── main.rs              # Entry point + --dry-run
│   ├── lib.rs               # Library root
│   ├── action.rs            # Action enum
│   ├── app.rs               # App state machine + TuiState
│   ├── tui.rs               # TUI event loop (RPC + mouse/keyboard)
│   ├── selection.rs         # Mouse selection highlighting + text collection
│   ├── config.rs            # CLI argument parsing
│   ├── errors.rs            # Error types
│   ├── logging.rs           # Log initialization
│   ├── message.rs           # ChatMessage model
│   ├── theme.rs             # Cyan industrial theme
│   ├── backend/
│   │   ├── mod.rs           # AgentBackend trait
│   │   ├── rpc.rs           # PiRpcBackend (process management)
│   │   ├── rpc_client.rs    # JSONL RPC client
│   │   └── event.rs         # PiEvent types + parsing
│   └── components/
│       ├── mod.rs           # Component trait (&mut self)
│       ├── sidebar.rs       # Left: session + workspace tree + MODEL
│       ├── main_view.rs     # Center: conversation + input
│       ├── agent_panel.rs   # Right: subagent list
│       ├── bottom_bar.rs    # Bottom: shortcuts + status + Token
│       └── popup.rs         # Popup overlay
└── tests/
    └── integration_test.rs
```

## License

MIT
