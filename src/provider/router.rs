//! axum HTTP 路由器 — 启动代理服务

use axum::{
    body::Body,
    extract::State,
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

    tracing::info!(
        "收到 POST /v1/chat/completions | provider={} mode={}",
        provider.id,
        if provider.bridge {
            "bridge"
        } else {
            "standard"
        }
    );

    if provider.bridge {
        // 桥接模式：纯透传
        bridge_proxy(
            &state.http,
            &provider.base_url,
            "/v1/chat/completions",
            headers,
            body,
        )
        .await
    } else {
        // 标准模式：读 model 字段路由
        standard_chat_proxy(&state.http, &provider, headers, body, &config).await
    }
}

/// GET /v1/models
async fn handle_models(State(state): State<Arc<AppState>>) -> Response {
    tracing::debug!("收到 GET /v1/models");
    let config = state.config.read().await;

    // 桥接模式：返回空列表（bridge 自己管理模型）
    if config.providers.iter().any(|p| p.bridge) {
        return (
            StatusCode::OK,
            [(
                HeaderName::from_static("content-type"),
                HeaderValue::from_static("application/json"),
            )],
            "{\"object\":\"list\",\"data\":[]}",
        )
            .into_response();
    }

    // 标准模式：聚合所有 provider 的模型
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

/// 桥接透传代理
async fn bridge_proxy(
    client: &Client,
    base_url: &str,
    path: &str,
    _headers: HeaderMap,
    body: Body,
) -> Response {
    let target_url = format!("{}{}", base_url.trim_end_matches('/'), path);
    tracing::debug!("POST {} → bridge 透传", target_url);
    let response = proxy_request(client, &target_url, body).await;
    tracing::debug!("POST {} — 上游响应完成", target_url);
    response
}

/// 标准模式代理：读 model 字段 → 匹配 provider → 转发 + 注入 API key
async fn standard_chat_proxy(
    client: &Client,
    _provider: &super::ProviderInfo,
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

    // 查找 model 对应的 provider
    let target = config.find_provider_by_model(&model);
    let target = match target {
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
    };

    let target_url = format!(
        "{}/v1/chat/completions",
        target.base_url.trim_end_matches('/')
    );

    tracing::info!(
        "POST /v1/chat/completions — model={} → provider={} → {}",
        model,
        target.id,
        target_url,
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
        if target.api_key.is_empty() { "none" } else { "Bearer ***" }
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

/// 通用转发（bridge 模式用）
async fn proxy_request(client: &Client, target_url: &str, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "{\"error\":\"failed to read body\"}",
            )
                .into_response()
        }
    };

    let req_builder = client
        .post(target_url)
        .header("Content-Type", "application/json")
        .body(bytes.to_vec());

    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            tracing::info!("POST {} — 上游响应 status={}", target_url, status.as_u16(),);
            let headers = resp.headers().clone();
            let stream = resp.bytes_stream();

            let body_stream =
                tokio_stream::StreamExt::map(stream, |chunk| chunk.map_err(std::io::Error::other));

            let mut response_headers = HeaderMap::new();
            for (key, value) in headers.iter() {
                if key != "transfer-encoding" {
                    response_headers.insert(key.clone(), value.clone());
                }
            }

            (
                status,
                response_headers,
                axum::body::Body::from_stream(body_stream),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("POST {} — 上游请求失败: {}", target_url, e);
            (
                StatusCode::BAD_GATEWAY,
                format!("{{\"error\":\"upstream error: {}\"}}", e),
            )
                .into_response()
        }
    }
}
