//! Provider 路由系统 — HTTP 路由器 + 配置管理
//!
//! TUI 内嵌 axum HTTP 服务器，将 pi 的请求按 model 路由到对应后端。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod router;

/// Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// HTTP 监听端口
    #[serde(default = "default_port")]
    pub port: u16,
    /// 当前选中模型（仅 UI 初始态）
    pub current_model: Option<String>,
    /// 当前选中 provider 索引（对应 TuiState.active_provider_idx）
    #[serde(default)]
    pub current_provider: Option<usize>,
    /// Provider 列表
    pub providers: Vec<ProviderInfo>,
}

fn default_port() -> u16 {
    8001
}

fn default_true() -> bool {
    true
}

/// Provider 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// 唯一标识
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 目标 base URL
    pub base_url: String,
    /// API key（标准模式使用）
    #[serde(default)]
    pub api_key: String,
    /// 模型列表（标准模式使用）
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    /// 端点类型："openai_compat" | "openai_responses" | "anthropic_messages" | "gemini"
    #[serde(default = "default_endpoint_type")]
    pub endpoint_type: String,
}

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

fn default_tier() -> String {
    "T2".to_string()
}

fn default_context_window() -> u32 {
    128000
}

fn default_endpoint_type() -> String {
    "openai_compat".to_string()
}

impl Default for ProviderInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            base_url: String::new(),
            api_key: String::new(),
            models: Vec::new(),
            endpoint_type: "openai_compat".into(),
        }
    }
}

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

impl ProviderConfig {
    /// 加载配置文件
    pub fn load(path: &PathBuf) -> Self {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(cfg) => return cfg,
                    Err(e) => tracing::warn!("providers.json parse error: {} — 使用默认配置", e),
                },
                Err(e) => tracing::warn!("providers.json read error: {} — 使用默认配置", e),
            }
        }
        ProviderConfig {
            port: default_port(),
            current_model: None,
            current_provider: None,
            providers: Vec::new(),
        }
    }

    /// 保存配置文件（原子写）
    pub fn save(path: &PathBuf, config: &Self) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        if let Ok(content) = serde_json::to_string_pretty(config) {
            if std::fs::write(&tmp, &content).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// 获取活跃 provider（优先按 current_provider 索引，回退按 current_model 匹配，跳过 disabled）
    pub fn active_provider(&self) -> Option<&ProviderInfo> {
        // 优先按 current_provider 索引
        if let Some(idx) = self.current_provider {
            if let Some(p) = self.providers.get(idx) {
                if p.enabled {
                    // 验证 current_model 确实属于此 provider
                    if let Some(ref model) = self.current_model {
                        if p.models.iter().any(|m| m.id == *model) {
                            return Some(p);
                        }
                        // current_model 不属于此 provider，继续回退
                    } else {
                        return Some(p);
                    }
                }
            }
        }
        // 回退：按 current_model 匹配
        if let Some(ref model) = self.current_model {
            for p in &self.providers {
                if p.enabled && p.models.iter().any(|m| m.id == *model) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// 根据 model id 查找 provider（优先按 current_provider 索引）
    pub fn find_provider_by_model(&self, model: &str) -> Option<&ProviderInfo> {
        // 优先按 current_provider 索引
        if let Some(idx) = self.current_provider {
            if let Some(p) = self.providers.get(idx) {
                if p.enabled && p.models.iter().any(|m| m.id == model) {
                    return Some(p);
                }
            }
        }
        // 回退：遍历查找
        self.providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.id == model))
    }
}

/// 共享配置状态
pub type SharedConfig = Arc<RwLock<ProviderConfig>>;

/// 获取配置目录路径
fn config_dir() -> PathBuf {
    // XDG_CONFIG_HOME 或 ~/.config
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(val).join("agent-tui")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("agent-tui")
    } else {
        PathBuf::from(".agent-tui")
    }
}

/// 获取 providers.json 路径
pub fn config_path() -> PathBuf {
    config_dir().join("providers.json")
}

/// 默认配置
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            current_model: None,
            current_provider: None,
            providers: Vec::new(),
        }
    }
}

/// 生成 local-provider.ts 内容
pub fn generate_local_provider_ts(config: &ProviderConfig, port: u16) -> String {
    let mut s = String::new();
    s.push_str(
        r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { randomUUID } from "node:crypto";
import { writeFileSync, existsSync } from "node:fs";

export default async function (pi: ExtensionAPI) {
"#,
    );

    // 注册 provider — api 取第一个启用 provider 的端点类型
    let api = config
        .providers
        .iter()
        .find(|p| p.enabled)
        .map(|p| p.pi_api())
        .unwrap_or("openai-completions");
    s.push_str(&format!("  pi.registerProvider(\"local\", {{\n    baseUrl: \"http://127.0.0.1:{}/v1\",\n    apiKey: \"LOCAL_API_KEY\",\n    api: \"{}\",\n    headers: {{\n      \"X-Session-Id\": \"!cat /tmp/pi-session-id\",\n    }},\n    compat: {{\n      supportsDeveloperRole: true,\n      supportsReasoningEffort: true,\n    }},\n    models: [\n", port, api));

    for p in &config.providers {
        for m in &p.models {
            let display_name = if m.name.is_empty() { &m.id } else { &m.name };
            s.push_str(&format!(
                r#"      {{ id: "{}", name: "{}", reasoning: {}, contextWindow: {}, input: ["text"], cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }} }},"#,
                m.id, display_name, m.reasoning, m.context_window
            ));
            s.push('\n');
        }
    }

    s.push_str(
        r#"    ],
  });

  const SESSION_FILE = "/tmp/pi-session-id";
  if (!existsSync(SESSION_FILE)) {
    writeFileSync(SESSION_FILE, `startup:${randomUUID()}`);
  }

  pi.on("session_start", async (event) => {
    let sessionId = event.reason === "resume"
      ? `resume:${event.previousSessionFile?.replace(/[^a-zA-Z0-9]/g, "-")}`
      : `session:${randomUUID()}`;
    writeFileSync(SESSION_FILE, sessionId);
  });
}
"#,
    );

    s
}

/// 重新生成 local-provider.ts 写入 pi 扩展目录
///
/// 每次 provider 配置变更时调用，确保下次 pi 启动使用正确的端点类型。
pub fn regenerate_local_provider_ts(config: &ProviderConfig) {
    let ts_content = generate_local_provider_ts(config, config.port);
    let pi_home = std::env::var("PI_CODING_AGENT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".pi").join("agent"))
                .unwrap_or_default()
        });
    let ext_dir = pi_home.join("extensions");
    let _ = std::fs::create_dir_all(&ext_dir);
    if let Err(e) = std::fs::write(ext_dir.join("local-provider.ts"), &ts_content) {
        tracing::warn!("重新生成 local-provider.ts 失败: {}", e);
    } else {
        let model_count: usize = config.providers.iter().map(|p| p.models.len()).sum();
        tracing::info!(
            "local-provider.ts 已更新（{} providers, {} models）",
            config.providers.len(),
            model_count
        );
    }
}

/// 从 /v1/models 接口拉取模型列表，返回 id:tier:contextWindow 格式文本
pub async fn fetch_models_list(base_url: &str, api_key: &str) -> Option<String> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; agent-tui/0.1)")
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("创建 HTTP 客户端失败: {}", e);
            return None;
        }
    };
    let resp = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("请求 /v1/models 失败: {}", e);
            return None;
        }
    };
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct.contains("json") {
        tracing::warn!(
            "/v1/models 返回非 JSON 响应 (Content-Type: {})，可能是反爬/认证页面",
            ct
        );
        return None;
    }
    let body: serde_json::Value = match resp.text().await {
        Ok(t) => match serde_json::from_str(&t) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "解析 /v1/models 响应失败: {} — 原始: {}",
                    e,
                    &t[..t.len().min(200)]
                );
                return None;
            }
        },
        Err(e) => {
            tracing::warn!("读取 /v1/models 响应失败: {}", e);
            return None;
        }
    };
    let models: Option<&Vec<serde_json::Value>> = body
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| body.get("models").and_then(|m| m.as_array()))
        .or_else(|| body.as_array());
    let models = match models {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            tracing::warn!("/v1/models 响应无法解析模型列表");
            return None;
        }
    };
    let lines: Vec<String> = models
        .iter()
        .filter_map(|m| {
            m.get("id")
                .and_then(|v| v.as_str())
                .or_else(|| m.get("name").and_then(|v| v.as_str()))
                .or_else(|| m.get("model").and_then(|v| v.as_str()))
        })
        .map(|id| format!("{}:T2:128000", id))
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}
