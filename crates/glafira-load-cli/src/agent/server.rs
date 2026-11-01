//! Маршруты агента и запуск сервера (спека §4.3).

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use tokio::net::TcpListener;

use super::runs::{self, AgentState};
use super::{auth, files, sse};

/// Бандл несёт сьют и файлы данных: потолок тела — 1 ГиБ.
pub const BODY_LIMIT: usize = 1 << 30;

pub fn router(state: Arc<AgentState>) -> Router {
    Router::new()
        .route("/v1/info", get(runs::info))
        .route("/v1/runs", get(runs::list).post(runs::start))
        .route("/v1/runs/{id}/events", get(sse::events))
        .route("/v1/runs/{id}/stop", post(runs::stop))
        .route("/v1/runs/{id}/files", get(files::files))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .with_state(state)
}

pub async fn serve(
    listener: TcpListener,
    state: Arc<AgentState>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
