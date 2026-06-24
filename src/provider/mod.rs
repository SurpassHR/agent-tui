//! Provider 路由系统 — HTTP 路由器 + 配置管理
//!
//! TUI 内嵌 axum HTTP 服务器，将 pi 的请求按 model 路由到对应后端。
//! 支持标准模式（按 model 字段路由）和桥接模式（透传）。

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
    /// Provider 列表
    pub providers: Vec<ProviderInfo>,
}

fn default_port() -> u16 {
    8001
}

/// Provider 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// 唯一标识
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 桥接模式：透传不做任何处理
    #[serde(default)]
    pub bridge: bool,
    /// 目标 base URL
    pub base_url: String,
    /// API key（标准模式使用）
    #[serde(default)]
    pub api_key: String,
    /// 模型列表（标准模式使用）
    #[serde(default)]
    pub models: Vec<ModelInfo>,
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
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_tier() -> String {
    "T2".to_string()
}

fn default_context_window() -> u32 {
    128000
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

    /// 获取活跃 provider（bridge 优先，否则按 current_model 匹配）
    pub fn active_provider<'a>(&'a self) -> Option<&'a ProviderInfo> {
        // bridge provider 优先
        if let Some(bridge) = self.providers.iter().find(|p| p.bridge) {
            return Some(bridge);
        }
        // 按 current_model 匹配
        if let Some(ref model) = self.current_model {
            for p in &self.providers {
                if p.models.iter().any(|m| m.id == *model) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// 根据 model id 查找 provider
    pub fn find_provider_by_model(&self, model: &str) -> Option<&ProviderInfo> {
        // bridge 模式直接返回第一个 bridge provider
        if let Some(bridge) = self.providers.iter().find(|p| p.bridge) {
            return Some(bridge);
        }
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

    // 注册 provider
    s.push_str(&format!("  pi.registerProvider(\"local\", {{\n    baseUrl: \"http://127.0.0.1:{}/v1\",\n    apiKey: \"LOCAL_API_KEY\",\n    api: \"openai-completions\",\n    headers: {{\n      \"X-Session-Id\": \"!cat /tmp/pi-session-id\",\n    }},\n    compat: {{\n      supportsDeveloperRole: true,\n      supportsReasoningEffort: true,\n    }},\n    models: [\n", port));

    for p in &config.providers {
        for m in &p.models {
            s.push_str(&format!(
                r#"      {{ id: "{}", name: "{}", reasoning: {}, contextWindow: {}, cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }} }},"#,
                m.id, m.name, m.reasoning, m.context_window
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
