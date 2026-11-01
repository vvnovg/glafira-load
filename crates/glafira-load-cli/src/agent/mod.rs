//! Агент `glafira-load agent`: HTTP API на axum, SSE, файлы прогона (спека §4).

pub mod auth;
pub mod files;
pub mod runs;
pub mod server;
pub mod sse;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use glafira_load_engine::agent_api::ApiError;

/// JSON-ошибка протокола без имён и версий.
pub fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    api_error_full(
        status,
        ApiError {
            code: code.into(),
            message: message.into(),
            names: vec![],
            agent: None,
            controller: None,
        },
    )
}

pub fn api_error_full(status: StatusCode, error: ApiError) -> Response {
    (status, Json(error)).into_response()
}

pub fn not_found(run_id: &str) -> Response {
    api_error(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        format!("run {run_id} is unknown"),
    )
}
