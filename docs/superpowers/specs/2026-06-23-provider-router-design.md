# Provider 路由系统设计

## 架构

```
┌─ TUI (Rust + axum + tokio) ────────────────────────────┐
│                                                          │
│  HTTP Router (127.0.0.1:8001/v1/*)                      │
│                                                          │
│  POST /v1/chat/completions                               │
│    ├─→ 解析 body.model 字段                              │
│    ├─→ 匹配到目标 provider（providers.json）              │
│    ├─→ 替换 Authorization header（注入真实 API key）     │
│    ├─→ body 原封不动转发到目标后端                        │
│    └─→ stream response 逐 chunk 回传                     │
│                                                          │
│  GET /v1/models                                          │
│    └─→ 聚合所有 provider 的 model 列表                   │
│                                                          │
│  Provider Config (providers.json)                        │
│  ┌──────────────────────────────────────────────────┐   │
│  │  currentModel, providers[] → id/name/baseUrl/    │   │
│  │  apiKey/models                                    │   │
│  └──────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
           ↑ 注册一个 provider "local"
           │ api: "openai-completions"
┌─ pi agent ──────────────────────────────────────────┐
│  pi --model <任意> --mode rpc                        │
│  → 统一用 OpenAI 格式发送请求                         │
│  → http://127.0.0.1:8001/v1/chat/completions         │
│  → body.model 字段由 pi 自己填充                      │
│  → 换模型 = 只改 body.model，pi 不需要重启            │
└──────────────────────────────────────────────────────┘
```

## 核心原则

1. **单一 endpoint** — pi 只注册一个 provider `"local"`，所有请求发到 `http://127.0.0.1:8001/v1/*`
2. **按 model 路由** — 路由器从 `body.model` 字段提取模型名，匹配到对应后端
3. **body 透传** — 请求体原样转发，不做任何修改。只替换 `Authorization` header（注入真实 API key）
4. **热切换** — 换 model 不需要重启 pi，下一个请求自动路由到新 backend
5. **统一 OpenAI 格式** — 所有后端 provider 使用 OpenAI 兼容格式（deepseek、openrouter 原生支持；anthropic 通过 `/v1/messages` 端点走自己的格式——见下方说明）

## 关于 API 格式

pi 注册 provider 时通过 `api` 字段决定请求格式。注册为 `api: "openai-completions"` 则 pi 发送 OpenAI 格式。

各后端的格式策略：

| 后端类型 | 路由方式 | 格式 | 说明 |
|----------|----------|------|------|
| deepseek / openrouter / groq 等 | body.model | OpenAI 原生 | 透传，零处理 |
| anthropic | body.model → header 格式检测 | Anthropic Messages | Anthropic 不走 OpenAI 格式，需在路由器端做单方向适配 |
| gemini | body.model | OpenAI 兼容 | Gemini 有自己的 `/v1/models` 和 `/v1/chat/completions` |

对于 **Anthropic** 这类不原生支持 OpenAI 格式的后端，有两种处理路径：

### 路径 A：注册为独立 provider（当前方案）

在 `local-provider.ts` 中注册两个 provider：

```ts
// 聚合 provider — 全走 OpenAI 格式
pi.registerProvider("local", {
  baseUrl: "http://127.0.0.1:8001/v1",
  apiKey: "LOCAL_API_KEY",
  api: "openai-completions",
  models: [
    { id: "deepseek-v4-flash", ... },
    { id: "deepseek-v4-pro", ... },
    // anthropic 的 model 也注册到这里
    // 但路由转发时需要处理格式差异
  ],
});
```

当 `body.model` 匹配到 anthropic 的 model 时，路由器将 OpenAI 格式的请求转换为 Anthropic Messages 格式再转发。

### 路径 B：纯透传 + 用户侧保证

路由器只转发 OpenAI 格式的请求，anthropic 用户需通过 OSS 代理（如 `litellm` / `one-api`）在其目标 `baseUrl` 层完成格式转换。TUI 不感知格式细节。

### 推荐：先做路径 B（MVP），路径 A 后续迭代

MVP 阶段只支持 OpenAI 兼容的 provider（deepseek、openrouter、groq、openai 等），anthropic、gemini 用户自建适配网关。等核心流程稳定后再加格式转换层。

## 路由逻辑

### POST /v1/chat/completions

```
Request: POST http://127.0.0.1:8001/v1/chat/completions
Headers: { Authorization: "Bearer LOCAL_API_KEY", Content-Type: "application/json" }
Body: { model: "deepseek-v4-flash", messages: [...], stream: true }

处理流程:
1. 解析 JSON body，提取 model = "deepseek-v4-flash"
2. 遍历 providers.json，找到包含此 model 的 provider → deepseek
3. 构造转发请求:
   URL: https://api.deepseek.com/v1/chat/completions
   Headers: { Authorization: "Bearer sk-real-key-...", Content-Type: "application/json" }
   Body: 原样（stream: true）
4. 发起 HTTP 请求
5. 根据是否 stream:
   - stream=true: axum 逐 chunk 流式转发 SSE 响应
   - stream=false: 完整响应转发
```

### GET /v1/models

```
1. 从 providers.json 中读取每个 provider 的 models 列表
2. 合并为 OpenAI 兼容的 models 列表格式
3. 返回 { object: "list", data: [ { id, object: "model", ... }, ... ] }
```

## Provider 配置格式 (providers.json)

```json
{
  "port": 8001,
  "currentModel": "deepseek-v4-flash",
  "providers": [
    {
      "id": "deepseek",
      "name": "DeepSeek",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "sk-...",
      "models": [
        {
          "id": "deepseek-v4-flash",
          "name": "DeepSeek Flash",
          "contextWindow": 128000,
          "reasoning": true
        },
        {
          "id": "deepseek-v4-pro",
          "name": "DeepSeek Pro",
          "contextWindow": 128000,
          "reasoning": true
        }
      ]
    },
    {
      "id": "openrouter",
      "name": "OpenRouter",
      "baseUrl": "https://openrouter.ai/api/v1",
      "apiKey": "sk-or-...",
      "models": [
        {
          "id": "anthropic/claude-sonnet-4",
          "name": "Claude Sonnet 4",
          "contextWindow": 200000
        }
      ]
    }
  ]
}
```

## local-provider.ts 注入

TUI 启动 / provider 配置变更时，生成 `~/.pi/agent/extensions/local-provider.ts`：

```ts
// 由 Meta-TUI Provider Router 管理 — 每次启动自动生成
export default async function (pi: ExtensionAPI) {
  pi.registerProvider("local", {
    baseUrl: "http://127.0.0.1:8001/v1",
    apiKey: "LOCAL_API_KEY",
    api: "openai-completions",
    headers: {
      "X-Session-Id": "!cat /tmp/pi-session-id",
    },
    compat: {
      supportsDeveloperRole: true,
      supportsReasoningEffort: true,
    },
    models: [
      { id: "deepseek-v4-flash", name: "DeepSeek Flash",
        reasoning: true, contextWindow: 128000,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } },
      { id: "deepseek-v4-pro", name: "DeepSeek Pro",
        reasoning: true, contextWindow: 128000,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } },
    ],
  });

  // 启动时如果没有 session 文件，初始化一个
  const SESSION_FILE = "/tmp/pi-session-id";
  if (!existsSync(SESSION_FILE)) {
    writeFileSync(SESSION_FILE, `startup:${randomUUID()}`);
  }

  // 监听 session 切换
  pi.on("session_start", async (event) => {
    let sessionId = event.reason === "resume"
      ? `resume:${event.previousSessionFile?.replace(/[^a-zA-Z0-9]/g, "-")}`
      : `session:${randomUUID()}`;
    writeFileSync(SESSION_FILE, sessionId);
  });
}
```

## 切换模型

**不需要重启 pi**。用户通过 TUI 界面选择模型后：

1. TUI 更新 `providers.json` 的 `currentModel`
2. TUI 显示侧边栏模型变更
3. pi 的下一次请求（用户发送新消息）会自动使用新的 `body.model` 值
4. 路由器根据新 model 值自动路由到对应的 backend

## TUI 界面

右侧栏新增 PROVIDER 区块：

```
PROVIDERS (2)
◆ deepseek  (当前)
○ openrouter

MODEL
  deepseek-v4-flash  ← 当前选中
  deepseek-v4-pro

Status: ● 路由在线 (127.0.0.1:8001)
```

- `◆` / `○` 表示 provider 启用/未使用
- MODEL 区展示当前可用模型列表
- 底部显示路由状态

## 性能保证

- 同一 tokio runtime，零 IPC
- axum + reqwest stream-to-stream 转发，不缓冲完整 body
- 连接池复用（reqwest 内建）
- 额外内存开销 < 5MB
- 单次请求延迟：+0.5~1ms（JSON body 解析 + 本地转发）
