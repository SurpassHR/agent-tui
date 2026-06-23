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

## 文件路径

`providers.json` 存储在 TUI 的配置目录：`~/.config/agent-tui/providers.json`

## 读写并发保护

`providers.json` 由 `Arc<RwLock<ProviderConfig>>` 在内存中持有。TUI 启动时一次性读取，之后所有读写操作基于内存中的 `Arc<RwLock>`：
- HTTP Router 每请求读 `providers.json` 的内存快照（`read().unwrap()`）
- TUI 界面切换模型时写内存（`write().unwrap()`），并异步写回磁盘文件
- 磁盘写入使用原子写（`write_atomic`：先写 `.tmp` 再 `rename`），避免文件损坏

## 核心原则

1. **单一 endpoint** — pi 只注册一个 provider `"local"`，所有请求发到 `http://127.0.0.1:8001/v1/*`
2. **按 model 路由** — 路由器从 `body.model` 字段提取模型名，匹配到对应后端
3. **body 透传** — 请求体原样转发，不做任何修改。只替换 `Authorization` header（注入真实 API key）
4. **热切换** — 换 model 不需要重启 pi，下一个请求自动路由到新 backend
5. **统一 OpenAI 格式** — 所有后端 provider 使用 OpenAI 兼容格式（deepseek、openrouter 原生支持；anthropic 通过 `/v1/messages` 端点走自己的格式——见下方说明）

## 关于 API 格式

pi 注册 provider 时通过 `api` 字段决定请求格式。注册为 `api: "openai-completions"` 则 pi 发送 OpenAI 格式。

各后端的格式策略（MVP 阶段）：

| 后端类型 | 路由方式 | 说明 |
|----------|----------|------|
| deepseek / openrouter / groq / openai 等 | body.model | OpenAI 原生，透传零处理 |
| anthropic（通过 openrouter） | body.model | OpenRouter 负责格式转换，TUI 透传 |
| gemini（通过 openrouter） | body.model | OpenRouter 负责格式转换，TUI 透传 |

**MVP 只对接 OpenAI 兼容的 provider**。anthropic、gemini 等非 OpenAI 格式的后端通过 `baseUrl` 指向格式转换网关（如 OpenRouter / litellm / one-api）来接入，TUI 不感知格式差异。格式转换层作为后续迭代。

### Bridge 模式

某些本地代理工具（如 `cc-switch`、`one-api`、`litellm proxy`）已自包含路由和鉴权逻辑。此时 TUI 应退化为**纯透传桥接**，不做任何请求检查、model 解析或 header 注入：

```json
{
  "id": "cc-switch",
  "name": "CC Switch",
  "bridge": true,
  "baseUrl": "http://127.0.0.1:5000",
  "models": []  // bridge 模式下忽略
}
```

当 `bridge: true` 时：
- 所有 `POST /v1/chat/completions` → `POST http://127.0.0.1:5000/v1/chat/completions`
- 所有 `GET /v1/models` → `GET http://127.0.0.1:5000/v1/models`
- **不解析 body**，**不替换 header**，**不按 model 路由**
- 完整的请求/响应字节流透传

一个 `providers.json` 中只能有一个 bridge provider 处于活跃状态。当 bridge 启用时，按 model 路由的非 bridge provider 被忽略（视图上可读，但流量走 bridge）。

示例：要使用 Claude，在 `providers.json` 中配置 OpenRouter：

```json
{
  "id": "openrouter",
  "baseUrl": "https://openrouter.ai/api/v1",
  "models": [{ "id": "anthropic/claude-sonnet-4", ... }]
}
```

TUI 转发请求到 OpenRouter，OpenRouter 负责将 OpenAI 格式转换为 Anthropic Messages 格式。

## 路由逻辑

### POST /v1/chat/completions

```
Request: POST http://127.0.0.1:8001/v1/chat/completions
Headers: { Authorization: "Bearer LOCAL_API_KEY", Content-Type: "application/json" }
Body: { model: "deepseek-v4-flash", messages: [...], stream: true }

处理流程:

**Bridge 模式**（当 active provider 设置了 `bridge: true`）：
1. 跳过 body 解析，跳过 model 匹配
2. 直接 POST 到 `{baseUrl}/v1/chat/completions`
3. 请求/响应字节流透传，不注入任何 header，不读 body

**标准模式**（按 model 路由）：
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
  // 默认选中的模型，仅用于 TUI 界面初始状态
  // 实际路由完全依赖请求中的 body.model 字段
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
    },
    {
      "id": "cc-switch",
      "name": "CC Switch",
      "bridge": true,
      "baseUrl": "http://127.0.0.1:5000",
      "models": []
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
      // `!` 前缀是 pi extension 的 shell 命令语法
      // 要求：pi 版本 ≥ 0.77（@earendil-works/pi-coding-agent）
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

1. TUI 更新 `providers.json` 的 `currentModel`（仅用于 UI 初始选中）
2. TUI 内部记录当前模型，通过 RPC 通知 pi 切换模型（`/model` 命令或 `set_config` 通知）
3. pi 的下一次请求会自动使用新的 `body.model` 值
4. 路由器根据 body.model 值自动路由到对应的 backend

**关键**：路由器不依赖 `currentModel`。路由决策完全基于请求中的 `body.model`，这是设计原则：「路由器无状态」。

如果 pi 不支持运行时通过 RPC 切换模型，TUI 重启 pi 子进程并传入 `--model <新模型>`。这是降级行为，正常路径是 RPC 切换。

## TUI 界面

左侧栏底部的 MODEL 区块**替换为 PROVIDER 区块**：

```
┌─ LEFT SIDEBAR (40) ─────────────────────────────────┐
│ ACTIVE SESSION                                       │
│ ● 首页重构讨论                                       │
│                                                      │
│ 工作区                                               │
│ ▼ 📁 agent-tui (5)                                   │
│   ├─ ● 首页重构讨论                                  │
│   …                                                  │
│──────────────────────────────────────────────────────│
│ PROVIDER (1) ◈ 路由在线 :8001                        │
│ ◆ deepseek  (当前)                                   │
│ ○ openrouter                                         │
│ ○ cc-switch 🔗 桥接                                  │
│ MODEL  deepseek-v4-flash  ←                         │
│        deepseek-v4-pro                               │
└──────────────────────────────────────────────────────┘
```

- `◆` / `○` 表示 provider 启用/未使用
- `◈ 路由在线 :8001` 指示 HTTP Router 运行状态
- 焦点系统新增 `SidebarSubsection::Provider`，替代原有的 `Model`
- 焦点在 Provider 子区时 ↑/↓ 选择 provider，Enter 展开模型列表，再 ↑/↓ 选模型
- 选中模型即时生效，通过 RPC 通知 pi

## 进程生命周期

HTTP Router 作为 TUI 进程内的一个 axum server 运行，与 ratatui 共享同一个 tokio runtime。

```
TUI 启动流程:
1. 解析 CLI 参数
2. 读取 providers.json → 初始化 Arc<RwLock<ProviderConfig>>
3. 启动 axum HTTP server（spawn 到现有 tokio runtime）
4. 生成并写入 local-provider.ts 到 pi 的 extensions 目录
5. 启动 pi RPC 子进程（pi 自动加载 local-provider.ts）
6. 进入 TUI 事件循环
┌─────────────────────────────────────────────────────────┐
│  tokio::select! 同时驱动:                                │
│  ├─ ticker（TUI 渲染）                                   │
│  ├─ event_stream（crossterm 键盘/鼠标）                  │
│  ├─ pi event_rx（RPC 事件）                              │
│  └─ axum Server（Provider Router）                       │
└─────────────────────────────────────────────────────────┘

TUI 退出时:
1. pi RPC 子进程停止
2. axum server graceful shutdown（tokio signal + timeout）
3. 可选：将 local-provider.ts 保留（供下次启动时沿用）或移除
```

## 错误处理

| 场景 | 行为 | HTTP 状态码 |
|------|------|------------|
| body.model 不在任何 provider 的 models 中 | 返回错误 JSON `{ error: "unknown model: xxx" }` | 404 |
| 目标后端不可达 / 超时 | `reqwest` 返回错误，路由器透传为 502 | 502 |
| JSON body 解析失败 / 缺少 model 字段 | 返回错误 JSON `{ error: "invalid request: ..." }` | 400 |
| provider 的 apiKey 为空/未配置 | 启动时跳过该 provider（打印警告），不 panic | — |
| 端口 8001 已被占用 | 尝试 8002、8003……直到可用，打印日志 | — |
| providers.json 文件损坏 / 不存在 | 使用默认配置（port 8001, providers 为空列表），不 panic | — |
| bridge 模式 + body 解析 | bridge 模式下不解析 body，无此错误路径 | — |
| 多个 bridge provider 同时存在 | 启动时警告，只取第一个 bridge | — |

## MVP 范围声明

以下特性不在 MVP 范围内，文档中仅为占位或后续迭代预留：
- **Cost 统计**：`cost: { input:0, output:0, ... }` 为占位值，MVP 不做费用计算
- **Session ID 管理**：`session_start` 事件监听和 session 文件写入沿用现有 `local-provider.ts` 行为，MVP 结束后可移除
- **非 OpenAI 格式转换**：anthropic / gemini 等非 OpenAI 兼容 provider 需通过格式转换网关（如 OpenRouter）接入

## 性能保证

- 同一 tokio runtime，零 IPC
- axum + reqwest stream-to-stream 转发，不缓冲完整 body
- 连接池复用（reqwest 内建）
- 额外内存开销 < 5MB
- 单次请求延迟：+0.5~1ms（JSON body 解析 + 本地转发）
