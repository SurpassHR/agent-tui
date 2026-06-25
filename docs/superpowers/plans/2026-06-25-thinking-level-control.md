# Thinking Level 控制实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 agent-tui 中添加思考级别（thinking level）的显示和切换功能，支持通过 UI 和快捷键控制 pi agent 的推理强度。

**Architecture:** 扩展现有 `ModelInfo` 结构以支持 `thinkingLevelMap`，在 `AgentRuntimeState` 中同步 pi 的思考级别状态，在侧边栏添加思考级别显示，并通过 Shift+Tab 快捷键循环切换级别。

**Tech Stack:** Rust 2024, ratatui 0.30.2, tokio, serde_json

## Global Constraints

- Rust edition 2024, MSRV 1.85
- 禁止 `unwrap()`，使用 `?` 操作符
- 所有 pub API 必须有 `///` 文档注释
- 使用 `tokio` 异步，禁止 `std::thread::spawn`
- 阻塞 I/O 必须在 `tokio::task::spawn_blocking` 中执行
- Commit message 使用中文，遵循 Conventional Commits 格式

## 背景知识

### Pi 的思考级别系统

Pi 支持 6 个标准化思考级别：`off`, `minimal`, `low`, `medium`, `high`, `xhigh`

各提供商的映射关系：

| 提供商 | API 参数 | 原生级别 |
|--------|---------|---------|
| OpenAI | `reasoning.effort` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh` |
| Anthropic | `effort` + adaptive thinking | `low`, `medium`, `high`, `xhigh`, `max` |
| Google Gemini 3 | `thinking_level` | `low`, `medium`, `high` |
| DeepSeek V4 | `reasoning_effort` | `high`, `max` |

Pi 通过 `thinkingLevelMap` 将标准化级别映射到各提供商的原生值。

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `src/provider/mod.rs` | `ModelInfo` 添加 `thinking_level_map` 字段 |
| `src/app.rs` | `AgentRuntimeState` 同步思考级别；添加 `SetThinkingLevel` Action 处理 |
| `src/action.rs` | 新增 `SetThinkingLevel` Action 变体 |
| `src/tui.rs` | 从 `get_state` 读取 `thinkingLevel`；处理 Shift+Tab 快捷键；发送 `set_thinking_level` RPC |
| `src/components/sidebar.rs` | 显示当前思考级别；支持点击/快捷键切换 |

---

### Task 1: 扩展 ModelInfo 支持 thinkingLevelMap

**Files:**
- Modify: `src/provider/mod.rs:58-76`

**Interfaces:**
- Consumes: 无
- Produces: `ModelInfo::thinking_level_map` 字段，供后续任务使用

- [ ] **Step 1: 在 ModelInfo 结构体中添加 thinking_level_map 字段**

```rust
/// 模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    #[serde(default)]
    pub reasoning: bool,
    /// 模型分级：T1 / T2 / T3
    #[serde(default = "default_tier")]
    pub tier: String,
    pub enabled: bool,
    /// 思考级别映射：将 pi 标准级别映射到提供商原生值
    /// `null` 表示该级别不支持，应在 UI 中隐藏
    #[serde(default)]
    pub thinking_level_map: Option<std::collections::HashMap<String, Option<String>>>,
}
```

- [ ] **Step 2: 更新所有 ModelInfo 构造点**

在 `src/app.rs` 中搜索所有 `ModelInfo { ... }` 构造，添加 `thinking_level_map: None` 字段。

涉及位置：
- `src/app.rs:342` (`merge_models_text` 函数)
- `src/app.rs:4877` (测试中的 ModelInfo 构造)
- `src/app.rs:4893` (测试中的 ModelInfo 构造)
- 其他测试中的 ModelInfo 构造

```rust
Some(crate::provider::ModelInfo {
    id,
    name: String::new(),
    context_window: ctx,
    reasoning: tier != "T1",
    tier,
    enabled,
    thinking_level_map: None,
})
```

- [ ] **Step 3: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add src/provider/mod.rs src/app.rs
git commit -m "feat(provider): 为 ModelInfo 添加 thinking_level_map 字段"
```

---

### Task 2: 添加 SetThinkingLevel Action

**Files:**
- Modify: `src/action.rs:1-143`

**Interfaces:**
- Consumes: 无
- Produces: `Action::SetThinkingLevel` 变体，供 Task 4 处理

- [ ] **Step 1: 在 Action 枚举中添加 SetThinkingLevel 变体**

在 `src/action.rs` 的 `// --- UI 交互控制 ---` 部分之前添加：

```rust
    // --- 思考级别控制 ---
    /// 设置思考级别（通过 UI 或快捷键触发）
    SetThinkingLevel(String),
```

- [ ] **Step 2: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add src/action.rs
git commit -m "feat(action): 添加 SetThinkingLevel Action 变体"
```

---

### Task 3: 从 pi get_state 读取 thinkingLevel

**Files:**
- Modify: `src/tui.rs:622-641`

**Interfaces:**
- Consumes: `AgentRuntimeState.thinking_level` 字段（已存在）
- Produces: `app.runtime.thinking_level` 被正确填充

- [ ] **Step 1: 在 get_state 响应处理中读取 thinkingLevel**

在 `src/tui.rs` 的状态轮询部分（约第 629 行），在读取 `model` 信息后添加：

```rust
                        if let Some(model) = data.get("model") {
                            app.runtime.model_name = model.get("name").and_then(|v| v.as_str()).map(String::from);
                            app.runtime.provider = model.get("provider").and_then(|v| v.as_str()).map(String::from);
                        }
                        // 读取思考级别
                        app.runtime.thinking_level = data
                            .get("thinkingLevel")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let sid = data.get("sessionId").and_then(|v| v.as_str()).map(String::from);
```

- [ ] **Step 2: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): 从 pi get_state 读取 thinkingLevel"
```

---

### Task 4: 处理 SetThinkingLevel Action 并发送 RPC

**Files:**
- Modify: `src/tui.rs:670-750` (键盘事件处理)
- Modify: `src/tui.rs:79-120` (translate_pi_events 附近)

**Interfaces:**
- Consumes: `Action::SetThinkingLevel(String)` (Task 2)
- Produces: 向 pi 发送 `set_thinking_level` RPC 请求

- [ ] **Step 1: 在 tui.rs 中添加处理 SetThinkingLevel 的逻辑**

在键盘事件处理部分（`Event::Key` match）中添加 Shift+Tab 快捷键处理：

```rust
                    // Shift+Tab: 循环切换思考级别
                    crossterm::event::KeyModifiers::SHIFT
                        if key.code == KeyCode::Tab =>
                    {
                        let current = app.runtime.thinking_level
                            .as_deref()
                            .unwrap_or("high");
                        let next = cycle_thinking_level(current);
                        let agent_id = app.active_agent.clone().unwrap_or_default();
                        if let Some(session_client) = app.agent_manager.client_mut(&agent_id) {
                            let _ = session_client
                                .notify(serde_json::json!({
                                    "type": "set_thinking_level",
                                    "level": next,
                                }))
                                .await;
                        }
                        app.runtime.thinking_level = Some(next.to_string());
                        app.tui.bottom_bar.status = format!("思考级别: {}", next);
                    }
```

- [ ] **Step 2: 添加 cycle_thinking_level 辅助函数**

在 `src/tui.rs` 文件顶部（`translate_pi_events` 函数附近）添加：

```rust
/// 循环切换思考级别
fn cycle_thinking_level(current: &str) -> &'static str {
    const LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];
    let idx = LEVELS.iter().position(|&l| l == current).unwrap_or(4); // 默认 high
    LEVELS[(idx + 1) % LEVELS.len()]
}
```

- [ ] **Step 3: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): 实现 Shift+Tab 循环切换思考级别"
```

---

### Task 5: 在侧边栏显示思考级别

**Files:**
- Modify: `src/components/sidebar.rs:26-63` (Sidebar 结构体)
- Modify: `src/components/sidebar.rs:272-310` (render_footer 函数)

**Interfaces:**
- Consumes: `AgentRuntimeState.thinking_level` (Task 3)
- Produces: 侧边栏显示当前思考级别

- [ ] **Step 1: 在 Sidebar 结构体中添加 thinking_level 字段**

```rust
pub struct Sidebar {
    // ... 现有字段 ...
    /// 当前思考级别（从 AgentRuntimeState 同步）
    pub thinking_level: String,
}
```

更新 `Default` 实现：

```rust
impl Default for Sidebar {
    fn default() -> Self {
        Self {
            // ... 现有字段 ...
            thinking_level: "high".into(),
        }
    }
}
```

- [ ] **Step 2: 在 render_footer 中添加思考级别显示**

在 `render_footer` 函数中，PROVIDER 部分之后、MODEL 部分之前添加：

```rust
    // ── THINKING LEVEL ──
    lines.push(Line::from(vec![
        Span::from("THINKING").fg(theme.heading).bold(),
    ]));
    
    let thinking_level = &self.thinking_level;
    let thinking_icon = match thinking_level.as_str() {
        "off" => "○",
        "minimal" | "low" => "◦",
        "medium" => "●",
        "high" => "◉",
        "xhigh" => "★",
        _ => "?",
    };
    
    lines.push(
        Line::from(Span::from(format!(
            "  {} {}",
            thinking_icon,
            thinking_level.to_uppercase(),
        )))
        .style(Style::default().fg(theme.text_dim)),
    );
    
    lines.push(Line::from(""));
```

- [ ] **Step 3: 在 sync_components 中同步 thinking_level**

在 `src/app.rs` 的 `sync_components` 函数中添加：

```rust
    // 同步思考级别到侧边栏
    self.tui.sidebar.thinking_level = self.runtime.thinking_level
        .clone()
        .unwrap_or_else(|| "high".into());
```

- [ ] **Step 4: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git add src/components/sidebar.rs src/app.rs
git commit -m "feat(sidebar): 显示当前思考级别"
```

---

### Task 6: 支持 thinkingLevelMap 过滤可用级别

**Files:**
- Modify: `src/tui.rs:79-120` (cycle_thinking_level 函数)

**Interfaces:**
- Consumes: `ModelInfo.thinking_level_map` (Task 1)
- Produces: `cycle_thinking_level` 只循环当前模型支持的级别

- [ ] **Step 1: 修改 cycle_thinking_level 函数**

```rust
/// 循环切换思考级别（只循环当前模型支持的级别）
fn cycle_thinking_level(current: &str, thinking_level_map: Option<&std::collections::HashMap<String, Option<String>>>) -> &'static str {
    const ALL_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];
    
    // 根据 thinking_level_map 过滤可用级别
    let available: Vec<&str> = if let Some(map) = thinking_level_map {
        ALL_LEVELS.iter()
            .filter(|&&level| {
                match map.get(level) {
                    Some(Some(_)) => true,  // 映射到具体值
                    None => true,           // 未配置，使用默认
                    Some(None) => false,    // 显式禁用
                }
            })
            .copied()
            .collect()
    } else {
        // 没有 thinking_level_map，使用默认（不含 xhigh）
        vec!["off", "minimal", "low", "medium", "high"]
    };
    
    if available.is_empty() {
        return "high";
    }
    
    let idx = available.iter().position(|&l| l == current).unwrap_or(available.len() - 1);
    available[(idx + 1) % available.len()]
}
```

- [ ] **Step 2: 更新调用点**

在 Task 4 中添加的 Shift+Tab 处理代码中，获取当前模型的 thinking_level_map：

```rust
                    // Shift+Tab: 循环切换思考级别
                    crossterm::event::KeyModifiers::SHIFT
                        if key.code == KeyCode::Tab =>
                    {
                        let current = app.runtime.thinking_level
                            .as_deref()
                            .unwrap_or("high");
                        
                        // 获取当前模型的 thinking_level_map
                        let tl_map = app.tui.sidebar.providers
                            .iter()
                            .find(|p| Some(&p.id) == app.runtime.provider.as_ref())
                            .and_then(|p| p.models.iter().find(|m| m.id == app.tui.sidebar.current_model))
                            .and_then(|m| m.thinking_level_map.as_ref());
                        
                        let next = cycle_thinking_level(current, tl_map.as_ref());
                        // ... 后续 RPC 发送代码不变 ...
                    }
```

- [ ] **Step 3: 运行测试验证编译通过**

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add src/tui.rs
git commit -m "feat(tui): 根据 thinking_level_map 过滤可用思考级别"
```

---

### Task 7: 添加思考级别切换测试

**Files:**
- Modify: `src/tui.rs:2312` (文件末尾测试模块)

**Interfaces:**
- Consumes: `cycle_thinking_level` 函数 (Task 6)
- Produces: 单元测试验证级别循环逻辑

- [ ] **Step 1: 添加 cycle_thinking_level 测试**

在 `src/tui.rs` 的 `#[cfg(test)] mod tests` 中添加：

```rust
    #[test]
    fn test_cycle_thinking_level_default() {
        // 无 thinking_level_map 时，不包含 xhigh
        assert_eq!(cycle_thinking_level("off", None), "minimal");
        assert_eq!(cycle_thinking_level("high", None), "off");
    }

    #[test]
    fn test_cycle_thinking_level_with_map() {
        let mut map = std::collections::HashMap::new();
        map.insert("off".to_string(), None);  // 禁用 off
        map.insert("xhigh".to_string(), Some("max".to_string()));
        
        assert_eq!(cycle_thinking_level("low", Some(&map)), "medium");
        assert_eq!(cycle_thinking_level("high", Some(&map)), "xhigh");
        assert_eq!(cycle_thinking_level("xhigh", Some(&map)), "minimal");
    }

    #[test]
    fn test_cycle_thinking_level_xhigh_not_in_default() {
        // 默认不支持 xhigh
        assert_eq!(cycle_thinking_level("high", None), "off");
        // 有 map 且 xhigh 有值时支持
        let mut map = std::collections::HashMap::new();
        map.insert("xhigh".to_string(), Some("max".to_string()));
        assert_eq!(cycle_thinking_level("high", Some(&map)), "xhigh");
    }
```

- [ ] **Step 2: 运行测试**

Run: `cargo test -- tui::tests::test_cycle_thinking_level --nocapture`
Expected: 3 个测试通过

- [ ] **Step 3: Commit**

```bash
git add src/tui.rs
git commit -m "test(tui): 添加 cycle_thinking_level 单元测试"
```

---

### Task 8: 集成验证

**Files:**
- 无新文件

**Interfaces:**
- Consumes: 所有前序 Task
- Produces: 完整功能可用

- [ ] **Step 1: 运行完整测试套件**

Run: `cargo test`
Expected: 所有测试通过

- [ ] **Step 2: 运行 clippy 检查**

Run: `cargo clippy -- -D warnings`
Expected: 零警告

- [ ] **Step 3: 运行格式检查**

Run: `cargo fmt --check`
Expected: 格式正确

- [ ] **Step 4: 手动测试**

```bash
cargo run -- --tui
```

验证：
1. 侧边栏显示 THINKING 区块和当前级别
2. Shift+Tab 可循环切换级别
3. 切换后底部状态栏显示新级别
4. 级别变化后 pi agent 响应质量有差异

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 完成思考级别控制功能"
```

---

## 自检清单

1. **Spec 覆盖** ✅
   - 显示思考级别：Task 5
   - 切换思考级别：Task 4, 6
   - thinkingLevelMap 支持：Task 1, 6
   - 测试覆盖：Task 7

2. **占位符扫描** ✅
   - 无 TBD/TODO
   - 所有代码块完整

3. **类型一致性** ✅
   - `thinking_level: Option<String>` (AgentRuntimeState)
   - `thinking_level_map: Option<HashMap<String, Option<String>>>` (ModelInfo)
   - `cycle_thinking_level(&str, Option<&HashMap<...>>) -> &'static str`
