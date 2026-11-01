//! Общее для интеграционных тестов: временный проект из фикстур и заглушка стенда на axum.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::extract::Path as AxPath;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use glafira_load_model::project::{Environment, MANIFEST_FILE, Manifest};
use glafira_load_model::{from_yaml, to_yaml};
use tempfile::TempDir;

pub const TOKEN: &str = "test-token";

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dev/fixtures/suites")
}

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// Проект: фикстура `project` + сьюты `orders` и `orders-http` с данными; стенд `test` смотрит на `stand_url`.
pub fn temp_project(stand_url: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    copy_dir(&fixtures().join("project"), root);
    for name in ["orders", "orders-http"] {
        let src = fixtures().join(name);
        std::fs::create_dir_all(root.join("suites").join(name)).unwrap();
        std::fs::copy(
            src.join("suite.yaml"),
            root.join("suites").join(name).join("suite.yaml"),
        )
        .unwrap();
        if src.join("data").is_dir() {
            copy_dir(&src.join("data"), &root.join("data"));
        }
    }
    let mut manifest: Manifest =
        from_yaml(&std::fs::read_to_string(root.join(MANIFEST_FILE)).unwrap()).unwrap();
    let env: Environment =
        from_yaml(&format!("id: test\nkind: test\nhttp: {stand_url}\n")).unwrap();
    manifest.environments.retain(|e| e.id != "test");
    manifest.environments.push(env);
    std::fs::write(root.join(MANIFEST_FILE), to_yaml(&manifest)).unwrap();
    dir
}

/// Профиль `name`: сразу 100 % и полка `minutes` минут.
pub fn write_profile(root: &Path, name: &str, minutes: f64) {
    let text = format!(
        "format: 1\nprofiles:\n  {name}:\n    basis: peak\n    phases:\n      - {{ kind: ramp, minutes: 0, to_pct: 100 }}\n      - {{ kind: steady, minutes: {minutes} }}\n"
    );
    std::fs::write(root.join("load/profile.yaml"), text).unwrap();
}

/// Цифры ПРОМ для HTTP-операций `orders`: по 20 RPS.
pub fn write_prod_load(root: &Path) {
    let text = "format: 1\noperations:\n  \"http:orders:POST /v1/orders\": { avg: 20, peak: 20 }\n  \"http:orders:GET /v1/orders/{id}\": { avg: 20, peak: 20 }\n";
    std::fs::write(root.join("load/prod-load.yaml"), text).unwrap();
}

/// Заглушка стенда: `POST /v1/orders` → `create_status` с `{"id":"o-1"}`, `GET /v1/orders/{id}` → 200, остальное → 200 `{}`.
pub async fn stand_stub(create_status: StatusCode) -> SocketAddr {
    let app = Router::new()
        .route(
            "/v1/orders",
            post(move || async move {
                (
                    create_status,
                    Json(serde_json::json!({"id": "o-1", "status": "created"})),
                )
            }),
        )
        .route(
            "/v1/orders/{id}",
            get(|AxPath(id): AxPath<String>| async move {
                Json(serde_json::json!({"id": id, "status": "created"}))
            }),
        )
        .fallback(|| async { (StatusCode::OK, Json(serde_json::json!({}))) });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}
