//! axum HTTP 路由器 — 启动代理服务

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;

use super::{ProviderConfig, SharedConfig};

/// 共享 HTTP 客户端
struct AppState {
    config: SharedConfig,
    http: Client,
}

/// 启动 axum HTTP 服务器，返回实际绑定的端口
pub async fn start_router(config: SharedConfig) -> Result<u16, crate::errors::Error> {
    let port = {
        let cfg = config.read().await;
        cfg.port
    };

    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| crate::errors::Error::Config(format!("reqwest client: {}", e)))?;

    let state = Arc::new(AppState { config, http });

    // 尝试启动，端口被占用时递增
    let mut try_port = port;
    let app = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/responses", post(handle_responses))
        .route("/v1/messages", post(handle_messages))
        .route("/v1/models/{*path}", post(handle_gemini))
        .route("/v1/models", get(handle_models))
        .with_state(state);

    loop {
        let addr = SocketAddr::from(([127, 0, 0, 1], try_port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                tracing::info!("Provider router listening on :{}", try_port);
                axum::serve(listener, app)
                    .await
                    .map_err(|e| crate::errors::Error::Config(format!("axum serve: {}", e)))?;
                return Ok(try_port);
            }
            Err(_e) if try_port < port + 10 => {
                tracing::warn!("Port :{} busy, trying :{}", try_port, try_port + 1);
                try_port += 1;
            }
            Err(e) => {
                return Err(crate::errors::Error::Config(format!(
                    "cannot bind port {}: {}",
                    port, e
                )));
            }
        }
    }
}

/// POST /v1/chat/completions
async fn handle_chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let config = state.config.read().await;
    let provider = config.active_provider();

    let provider = match provider {
        Some(p) => p.clone(),
        None => {
            tracing::warn!("收到 POST /v1/chat/completions — 无可用 provider");
            return (StatusCode::NOT_FOUND, "{\"error\":\"no active provider\"}").into_response();
        }
    };

    tracing::info!("收到 POST /v1/chat/completions | provider={}", provider.id,);

    standard_chat_proxy(&state.http, &provider, headers, body, &config).await
}

/// POST /v1/responses
async fn handle_responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let config = state.config.read().await;
    let provider = match config.active_provider() {
        Some(p) => p.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "{\"error\":\"no active provider\"}").into_response();
        }
    };
    standard_chat_proxy(&state.http, &provider, headers, body, &config).await
}

/// POST /v1/messages
async fn handle_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let config = state.config.read().await;
    let provider = match config.active_provider() {
        Some(p) => p.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "{\"error\":\"no active provider\"}").into_response();
        }
    };
    standard_chat_proxy(&state.http, &provider, headers, body, &config).await
}

/// POST /v1/models/{*path}（Gemini generateContent 等操作）
async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    Path(_path): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let config = state.config.read().await;
    let provider = match config.active_provider() {
        Some(p) => p.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "{\"error\":\"no active provider\"}").into_response();
        }
    };
    standard_chat_proxy(&state.http, &provider, headers, body, &config).await
}

/// GET /v1/models
async fn handle_models(State(state): State<Arc<AppState>>) -> Response {
    tracing::debug!("收到 GET /v1/models");
    let config = state.config.read().await;

    // 聚合所有 provider 的模型
    let mut models = Vec::new();
    for p in &config.providers {
        for m in &p.models {
            models.push(serde_json::json!({
                "id": m.id,
                "object": "model",
                "owned_by": p.id,
                "context_window": m.context_window,
            }));
        }
    }

    let resp = serde_json::json!({
        "object": "list",
        "data": models,
    });

    (
        StatusCode::OK,
        [(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        )],
        serde_json::to_string(&resp).unwrap_or_default(),
    )
        .into_response()
}

/// 标准模式代理：读 model 字段 → 优先用 handler 传入的 provider → 根据 endpoint_type 构造目标路径 → 转发 + 注入 API key
async fn standard_chat_proxy(
    client: &Client,
    provider: &super::ProviderInfo,
    _headers: HeaderMap,
    body: Body,
    config: &ProviderConfig,
) -> Response {
    // 从 body 中提取 model 字段
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "{\"error\":\"failed to read body\"}",
            )
                .into_response()
        }
    };

    // 尝试解析 JSON 提取 model
    let model = match extract_model_from_body(&bytes) {
        Some(m) => m,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "{\"error\":\"invalid request: missing 'model' field\"}".to_string(),
            )
                .into_response();
        }
    };

    // 优先用 handler 传入的 provider（UI 侧选中的），验证它是否包含请求的 model
    // 回退到 config.find_provider_by_model（也会优先 current_provider 索引）
    let target = if provider.enabled && provider.models.iter().any(|m| m.id == model) {
        provider.clone()
    } else {
        match config.find_provider_by_model(&model) {
            Some(p) => p.clone(),
            None => {
                tracing::warn!(
                    "POST /v1/chat/completions — model={} 未匹配到 provider",
                    model
                );
                return (
                    StatusCode::NOT_FOUND,
                    format!("{{\"error\":\"unknown model: {}\"}}", model),
                )
                    .into_response();
            }
        }
    };

    // 根据 provider 的 endpoint_type 构造目标路径
    let path = match target.endpoint_type.as_str() {
        "openai_responses" => "/v1/responses".to_string(),
        "anthropic_messages" => "/v1/messages".to_string(),
        "gemini" => format!("/v1/models/{}:generateContent", model),
        _ => "/v1/chat/completions".to_string(),
    };

    let target_url = format!("{}{}", target.base_url.trim_end_matches('/'), path);

    tracing::info!(
        "POST {} — model={} → provider={} → {} (endpoint={})",
        path,
        model,
        target.id,
        target_url,
        target.endpoint_type,
    );

    // 构造转发请求
    let mut req_builder = client
        .post(&target_url)
        .header("Content-Type", "application/json");

    // 注入 API key
    if !target.api_key.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {}", target.api_key));
    }

    // pi 固定使用 developer 角色，但多数 OpenAI 兼容 API（如 DeepSeek）不支持
    // 转发前统一转换为 system
    let bytes = rewrite_developer_to_system(bytes.to_vec());

    // 临时诊断：打印发送给上游的请求体
    let body_str = String::from_utf8_lossy(&bytes);
    tracing::info!(
        "POST {} | body={} | auth={}",
        target_url,
        &body_str[..body_str.len().min(500)],
        if target.api_key.is_empty() {
            "none"
        } else {
            "Bearer ***"
        }
    );

    req_builder = req_builder.body(bytes.to_vec());

    let upstream_start = std::time::Instant::now();
    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let elapsed = upstream_start.elapsed();
            tracing::info!(
                "POST {} — 上游响应 status={} 耗时={}ms",
                target_url,
                status.as_u16(),
                elapsed.as_millis(),
            );
            let headers = resp.headers().clone();
            let stream = resp.bytes_stream();

            // 流式转发
            let body_stream =
                tokio_stream::StreamExt::map(stream, |chunk| chunk.map_err(std::io::Error::other));

            let streaming_body = axum::body::Body::from_stream(body_stream);

            let mut response_headers = HeaderMap::new();
            for (key, value) in headers.iter() {
                if key != "transfer-encoding" {
                    response_headers.insert(key.clone(), value.clone());
                }
            }

            (status, response_headers, streaming_body).into_response()
        }
        Err(e) => {
            let elapsed = upstream_start.elapsed();
            tracing::error!(
                "POST {} — 上游请求失败 耗时={}ms: {}",
                target_url,
                elapsed.as_millis(),
                e,
            );
            (
                StatusCode::BAD_GATEWAY,
                format!("{{\"error\":\"upstream error: {}\"}}", e),
            )
                .into_response()
        }
    }
}

/// 从请求 body 中提取 model 字段
fn extract_model_from_body(bytes: &[u8]) -> Option<String> {
    let val: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    val.get("model")?.as_str().map(|s| s.to_string())
}

/// 将 messages 中的 developer 角色转换为 system（DeepSeek 等 API 不支持 developer）
fn rewrite_developer_to_system(bytes: Vec<u8>) -> Vec<u8> {
    let mut val: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return bytes,
    };
    if let Some(messages) = val.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for msg in messages {
            if msg.get("role").and_then(|r| r.as_str()) == Some("developer") {
                if let Some(obj) = msg.as_object_mut() {
                    obj.insert("role".to_string(), serde_json::json!("system"));
                }
            }
        }
    }
    serde_json::to_vec(&val).unwrap_or(bytes)
}
