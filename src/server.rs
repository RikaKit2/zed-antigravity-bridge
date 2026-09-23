use crate::adapter::Adapter;
use crate::auth::TokenManager;
use crate::types::{CompletionRequest, GeminiSseEvent};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct AppState {
    pub client: reqwest::Client,
    pub token_manager: TokenManager,
    pub default_model: String,
    pub upstream_base_url: String,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/completions", post(handle_completions))
        .route("/completions", post(handle_completions))
        .with_state(Arc::new(state))
}

async fn health_check() -> &'static str {
    "OK"
}

async fn handle_completions(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CompletionRequest>,
) -> Response {
    let project_id = state.token_manager.get_project_id().await;
    let (model, envelope, prompt_prefix) = Adapter::build_antigravity_envelope(&payload, &state.default_model, project_id);
    debug!(model = %model, "Processing completions request");

    // Try sending request with token, retry once on 401
    for attempt in 0..2 {
        let token = match state.token_manager.get_token().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to acquire token: {:?}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": {
                            "message": format!("Token acquisition failed: {}", e),
                            "type": "auth_error"
                        }
                    })),
                )
                    .into_response();
            }
        };

        let url = format!(
            "{}/v1internal:streamGenerateContent?alt=sse",
            state.upstream_base_url.trim_end_matches('/')
        );

        let send_res = state
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .header("User-Agent", "antigravity")
            .json(&envelope)
            .send()
            .await;

        let response = match send_res {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to contact upstream Antigravity API: {:?}", e);
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": {
                            "message": format!("Upstream connection error: {}", e),
                            "type": "upstream_error"
                        }
                    })),
                )
                    .into_response();
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            warn!("Upstream returned 401 Unauthorized; invalidating token cache");
            state.token_manager.invalidate().await;
            if attempt == 0 {
                info!("Retrying with refreshed token...");
                continue;
            } else {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": {
                            "message": "Upstream authentication failed after refresh",
                            "type": "auth_error"
                        }
                    })),
                )
                    .into_response();
            }
        }

        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            error!(status = %status, body = %error_body, "Upstream returned error");
            return (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(serde_json::json!({
                    "error": {
                        "message": format!("Upstream API error ({}): {}", status, error_body),
                        "type": "upstream_api_error"
                    }
                })),
            )
                .into_response();
        }

        // Parse SSE stream
        let mut stream = response.bytes_stream();
        let mut aggregated_text = String::new();
        let mut finish_reason = None;
        let mut line_buffer = String::new();

        while let Some(chunk_res) = stream.next().await {
            let chunk = match chunk_res {
                Ok(c) => c,
                Err(e) => {
                    warn!("Error reading SSE chunk: {:?}", e);
                    break;
                }
            };

            let text = String::from_utf8_lossy(&chunk);
            line_buffer.push_str(&text);

            while let Some(newline_pos) = line_buffer.find('\n') {
                let line = line_buffer[..newline_pos].trim().to_string();
                line_buffer = line_buffer[newline_pos + 1..].to_string();

                if let Some(json_str) = line.strip_prefix("data: ") {
                    let json_str = json_str.trim();
                    if json_str.is_empty() || json_str == "[DONE]" {
                        continue;
                    }

                    if let Ok(gemini_event) = serde_json::from_str::<GeminiSseEvent>(json_str) {
                        let (part_text, reason) = Adapter::extract_text_from_sse_event(&gemini_event);
                        aggregated_text.push_str(&part_text);
                        if reason.is_some() {
                            finish_reason = reason;
                        }
                    }
                }
            }
        }

        debug!(len = aggregated_text.len(), "Successfully generated completion text");

        let openai_resp = Adapter::build_openai_response(model, aggregated_text, &prompt_prefix, finish_reason);
        return Json(openai_resp).into_response();
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": { "message": "Failed to complete request", "type": "internal_error" }
        })),
    )
        .into_response()
}
