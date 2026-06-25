# Provider 端点类型配置 — 设计文档

> 日期：2026-06-25 | 状态：设计确认

## 背景

当前 Provider 路由（`router.rs`）在标准模式下统一走 `POST /v1/chat/completions`（OpenAI 兼容格式），`generate_local_provider_ts` 硬编码 `api: "openai-completions"`。用户需要在 provider 配置面板中为每个 provider 指定端点类型，以支持不同 API 协议（OpenAI Responses、Anthropic Messages、Google Gemini）。

## 目标

1. ProviderInfo 新增 `endpoint_type` 字段，支持四种端点类型
2. 编辑表单中用端点类型选择器替换当前 mode 字段，bridge 降为独立开关
3. Router 注册对应路由，透传转发
4. `generate_local_provider_ts` 根据端点类型输出正确的 pi `api` 值
5. 向后兼容：旧配置（无 endpoint_type）默认 `"openai_compat"`

---

## 1. 端点类型定义

| 用户面标签 | `endpoint_type` 值 | pi `KnownApi` 值 | Router 路径 |
|-----------|-------------------|-----------------|------------|
| `openai_compat` | `"openai_compat"` | `"openai-completions"` | `/v1/chat/completions` |
| `openai_responses` | `"openai_responses"` | `"openai-responses"` | `/v1/responses` |
| `anthropic_messages` | `"anthropic_messages"` | `"anthropic-messages"` | `/v1/messages` |
| `gemini` | `"gemini"` | `"google-generative-ai"` | `/v1/models/:model:generateContent` |

---

## 2. 数据模型变更

### 2.1 `ProviderInfo`（`src/provider/mod.rs:34-54`）

新增字段：

```rust
/// 端点类型，默认 "openai_compat"
#[serde(default = "default_endpoint_type")]
pub endpoint_type: String,
```

默认值函数：

```rust
fn default_endpoint_type() -> String {
    "openai_compat".to_string()
}
```

`bridge` 字段保留不变，作为独立开关。

### 2.2 端点类型工具函数

```rust
impl ProviderInfo {
    /// endpoint_type → pi KnownApi 映射
    pub fn pi_api(&self) -> &str {
        match self.endpoint_type.as_str() {
            "openai_responses" => "openai-responses",
            "anthropic_messages" => "anthropic-messages",
            "gemini" => "google-generative-ai",
            _ => "openai-completions",  // 默认 + openai_compat
        }
    }

    /// Router 路径前缀
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

---

## 3. UI 变更

### 3.1 编辑表单（`app.rs:2242-2480`）

**当前焦点顺序（`FIELD_ORDER`，`app.rs:382`）：**
```
[4=mode, 0=ID, 1=name, 2=base_url, 3=api_key, 5=models]
```

**新焦点顺序：**
```
[6=endpoint_type, 0=ID, 1=name, 2=base_url, 3=api_key, 5=models, 4=bridge]
```

- 字段 6（旧字段 4 换内容）：端点类型选择器，Space 循环切换四种值
- 字段 4（旧 mode → bridge）：独立开关，Space 切换 bool
- bridge=true 时，端点类型选择器不在调用用户交互的画面上渲染（桥接模式不关心端点类型）
- `FIELD_ORDER` 数组大小从 `[usize; 6]` 变为 `[usize; 7]`，`next_field()` / `prev_field()` 的取模逻辑相应调整

**表单布局：**
```
┌──────── Provider Editor ───────┐
│  端点类型                      │
│    ○ openai_compat             │
│    ◆ anthropic_messages  ▸     │
│    ○ gemini                    │
│    ○ openai_responses          │
│  桥接模式  [ ] 启用            │
│  ───────────────────────       │
│  Provider ID: deepseek         │
│  显示名称: DeepSeek            │
│  Base URL: https://api...      │
│  API Key: sk-xxxx              │
│  模型（id:tier:ctx）           │
│  deepseek-chat:T2:128000       │
│  ───────────────────────       │
│  [✓ 保存] [取消] [Tab 字段]    │
└────────────────────────────────┘
```

### 3.2 Sidebar 列表（`sidebar.rs:345-370`）

每行尾部追加端点类型标签：

```
  ◆ deepseek  (2 models) [openai_compat]
  ○ anthropic (3 models) [anthropic_messages] 🔗
```

### 3.3 详情弹窗（`app.rs:2181-2239`）

新增一行显示 `Endpoint:` + 端点类型标签。

---

## 4. Router 变更（`src/provider/router.rs`）

### 4.1 新增路由

```rust
let app = Router::new()
    .route("/v1/chat/completions", post(handle_chat_completions))
    .route("/v1/responses", post(handle_responses))
    .route("/v1/messages", post(handle_messages))
    .route("/v1/models/{model}:generateContent", post(handle_gemini))
    .route("/v1/models", get(handle_models))
    .with_state(state);
```

### 4.2 Handler 统一逻辑

所有 handler 执行相同流程：

1. 读 `active_provider()`
2. bridge=true → `bridge_proxy(client, base_url, path, headers, body)`
3. bridge=false → 拼 `{base_url}{path}` → 注入 API key → 透传代理

**注意**：当前 `standard_chat_proxy`（`router.rs:226-306`）将上游路径硬编码为 `/v1/chat/completions`。需要先重构该函数，接受 `path: &str` 参数，使其可复用于其他端点类型。或者，每个端点类型编写独立的 handler，内部调用同一个泛化的代理函数。

### 4.3 Gemini 特殊处理

Gemini 路径 `/v1/models/{model}:generateContent` 中的 `{model}` 通过 axum `Path` extractor 从路由参数提取。

**方案：** `handle_gemini(Path(model): Path<String>, ...)` 提取 model，构造 `{base_url}/v1/models/{model}:generateContent`。axum 的 matchit 路由器会将 `{model}` 正确解析（路径中的 `:` 不是特殊字符）。

---

## 5. `generate_local_provider_ts` 变更

`src/provider/mod.rs:173-218`

当前硬编码 `api: "openai-completions"`（第 186 行）。改为读取 `p.pi_api()`：

```rust
s.push_str(&format!(
    "  pi.registerProvider(\"local\", {{\n    baseUrl: \"http://127.0.0.1:{}/v1\",\n    apiKey: \"LOCAL_API_KEY\",\n    api: \"{}\",\n    ...",
    port,
    active_provider.pi_api(),  // 根据 endpoint_type 动态输出
));
```

---

## 6. 向后兼容

- 旧 `providers.json` 无 `endpoint_type` 字段 → serde default 为 `"openai_compat"`，行为不变
- `bridge` 字段保留，语义不变
- `mode` 字段焦点（旧 FIELD_ORDER[0]=4）的内容从 bridge/standard 切换改为端点类型选择

---

## 7. 涉及文件

| 文件 | 变更 |
|------|------|
| `src/provider/mod.rs` | ProviderInfo 加 `endpoint_type` + `pi_api()` + `endpoint_path()` + `generate_local_provider_ts` 改用 `pi_api()` |
| `src/provider/router.rs` | 新增 3 条路由 + 3 个 handler |
| `src/app.rs` | FIELD_ORDER 调整 + 表单渲染 + 键盘处理 + 详情弹窗 |
| `src/components/sidebar.rs` | 列表行追加端点类型标签 |

---

## 8. 不涉及

- **格式转换**：Router 不做请求体格式转换，pi 根据 `api` 字段自行格式化
- **Bridge 模式**：不受影响，始终透传
- **模型拉取**：`fetch_models_list` 仍走 `/v1/models`，不变
