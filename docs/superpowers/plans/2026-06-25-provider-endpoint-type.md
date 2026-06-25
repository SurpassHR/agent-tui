# Provider 端点类型配置 — 实现计划

> **Goal:** 为 ProviderInfo 新增 endpoint_type 字段，编辑表单支持端点类型选择，Router 注册对应路由

**Architecture:** 数据模型加字段 → 表单调整焦点顺序和渲染 → Router 注册新路由 → TS 生成改用动态 api

**Tech Stack:** Rust + Ratatui + axum + serde

## 全局约束

- Rust edition 2024，MSRV 1.85
- 禁止 `unwrap()`，使用 `?` 操作符
- 公开 API 必须有 `///` 文档注释
- 向后兼容：serde `default` 保证旧配置不报错

---

### Task 1: ProviderInfo 数据模型 — 新增 endpoint_type 字段

**Files:**
- Modify: `src/provider/mod.rs`

- [ ] **Step 1: 在 ProviderInfo 加字段和默认函数**

```rust
fn default_endpoint_type() -> String {
    "openai_compat".to_string()
}

// 在 ProviderInfo 中新增：
/// 端点类型："openai_compat" | "openai_responses" | "anthropic_messages" | "gemini"
#[serde(default = "default_endpoint_type")]
pub endpoint_type: String,
```

- [ ] **Step 2: 添加 ProviderInfo 工具方法 pi_api() 和 endpoint_path()**

```rust
impl ProviderInfo {
    /// endpoint_type → pi KnownApi 值
    pub fn pi_api(&self) -> &str {
        match self.endpoint_type.as_str() {
            "openai_responses" => "openai-responses",
            "anthropic_messages" => "anthropic-messages",
            "gemini" => "google-generative-ai",
            _ => "openai-completions",
        }
    }

    /// endpoint_type → Router 路径
    pub fn endpoint_path(&self) -> &str {
        match self.endpoint_type.as_str() {
            "openai_responses" => "/v1/responses",
            "anthropic_messages" => "/v1/messages",
            "gemini" => "/v1/models/:model:generateContent",
            _ => "/v1/chat/completions",
        }
    }
}
```

- [ ] **Step 3: generate_local_provider_ts 改用 pi_api()**

将 `api: "openai-completions"` 改为 `api: "{}", provider.pi_api()`

- [ ] **Step 4: cargo check 验证通过**
- [ ] **Step 5: Commit**

---

### Task 2: Editor 表单 — 端点类型选择器

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: 调整 FIELD_ORDER**

```rust
const FIELD_ORDER: [usize; 7] = [6, 0, 1, 2, 3, 5, 4];
// 6=endpoint_type, 0=ID, 1=name, 2=base_url, 3=api_key, 5=models, 4=bridge
const FIELD_COUNT: usize = 7;
```

- [ ] **Step 2: 更新 next_field/prev_field 的取模计算，适配 7 个字段**

- [ ] **Step 3: 更新表单渲染（render provider editor 区域）**

将字段 4（原 mode）改为端点类型选择器：渲染四种选项，当前值高亮，Space 循环切换。bridge 独立渲染为字段 4 的 bool 开关。bridge=true 时跳过字段 6 的渲染。

- [ ] **Step 4: 更新键盘处理 handle_provider_editor_key**

字段 6 按 Space → 循环 endpoint_type（"openai_compat" → "openai_responses" → "anthropic_messages" → "gemini" → "openai_compat"）。其他字段编辑逻辑不变。bridge=true 时 Tab 跳过字段 6。

- [ ] **Step 5: 更新 open_edit_provider_editor，draft.endpoint_type 设默认值**

- [ ] **Step 6: 更新详情弹窗，显示 endpoint_type**

- [ ] **Step 7: 更新 sidebar provider 列表行，尾部追加端点类型标签**

Modify: `src/components/sidebar.rs`

```rust
// 在 provider 行尾部追加：
let mut text = format!("  {} {}  ({} models)", icon, p.name, cnt);
if !p.endpoint_type.is_empty() && p.endpoint_type != "openai_compat" {
    text.push_str(&format!(" [{}]", p.endpoint_type));
}
if p.bridge {
    text.push_str(" 🔗");
}
```

- [ ] **Step 8: cargo check + cargo clippy**
- [ ] **Step 9: Commit**

---

### Task 3: Router — 新增端点路由

**Files:**
- Modify: `src/provider/router.rs`

- [ ] **Step 1: 重构 standard_chat_proxy，接受 path 参数**

将函数签名从 `standard_chat_proxy(client, provider, headers, body, config)` 改为 `standard_chat_proxy(client, provider, path, headers, body, config)`，target_url 使用传入的 path 而非硬编码。

- [ ] **Step 2: 注册新路由**

```rust
let app = Router::new()
    .route("/v1/chat/completions", post(handle_chat_completions))
    .route("/v1/responses", post(handle_responses))
    .route("/v1/messages", post(handle_messages))
    .route("/v1/models/{model}:generateContent", post(handle_gemini))
    .route("/v1/models", get(handle_models))
    .with_state(state);
```

- [ ] **Step 3: 实现 handle_responses / handle_messages / handle_gemini**

三个 handler 结构一致：读 active_provider → bridge 走 bridge_proxy(p) / 标准走 standard_chat_proxy(p, path)。`handle_gemini` 用 `Path(model): Path<String>` 提取模型名构造路径。

- [ ] **Step 4: cargo check + cargo clippy**
- [ ] **Step 5: Commit**
