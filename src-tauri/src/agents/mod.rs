//! Агенты стенда: ключ токена в хранилище ОС, стенд из манифеста, опрос `/v1/info`.

pub mod probe;

use std::path::Path;

use glafira_load_model::project::{Environment, Manifest};

use crate::error::{AppError, AppResult, ErrorCode};

/// Ключ токена: `<канонический корень проекта>/<стенд>/<агент>` (спека §4.2).
pub fn token_key(root: &Path, env_id: &str, agent_id: &str) -> String {
    format!("{}/{env_id}/{agent_id}", root.display())
}

/// Стенд манифеста по `id`; нет — `NOT_FOUND`.
pub fn env_of(m: &Manifest, env_id: &str) -> AppResult<Environment> {
    m.environments
        .iter()
        .find(|e| e.id == env_id)
        .cloned()
        .ok_or_else(|| {
            app_err!(
                ErrorCode::NotFound,
                "errors.project.noEnvironment",
                "Environment {{id}} is not in the manifest",
                id = env_id
            )
        })
}

pub fn no_agent(env_id: &str, agent_id: &str) -> AppError {
    app_err!(
        ErrorCode::NotFound,
        "errors.agent.notFound",
        "Agent {{id}} is not in environment {{env}}",
        id = agent_id,
        env = env_id
    )
}

/// Заглушки агента на `axum` для тестов команд и прогонов.
#[cfg(test)]
pub(crate) mod testing {
    use std::future::IntoFuture;

    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use glafira_load_engine::agent_api::AgentInfo;
    use glafira_load_model::project::{Agent, EnvKind, Environment, Manifest};

    /// Поднимает маршрутизатор на свободном порту; адрес вида `http://127.0.0.1:NNNN`.
    pub(crate) async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, router).into_future());
        format!("http://{addr}")
    }

    /// Адрес закрытого порта: соединение отвергается сразу.
    pub(crate) fn closed_port_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    pub(crate) fn bearer_ok(headers: &HeaderMap, token: &str) -> bool {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == format!("Bearer {token}"))
    }

    /// `GET /v1/info` с проверкой токена.
    pub(crate) fn info_router(version: &str, busy: Option<&str>, token: &str) -> Router {
        let info = AgentInfo {
            version: version.into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            cpus: 4,
            busy: busy.map(String::from),
        };
        let token = token.to_string();
        Router::new().route(
            "/v1/info",
            get(move |headers: HeaderMap| {
                let (info, token) = (info.clone(), token.clone());
                async move {
                    if !bearer_ok(&headers, &token) {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(info))
                }
            }),
        )
    }

    pub(crate) fn agent(id: &str, url: &str) -> Agent {
        Agent {
            id: id.into(),
            url: url.into(),
            weight: 1,
        }
    }

    /// Манифест с одним стендом `test` и заданными агентами.
    pub(crate) fn manifest_with_agents(agents: Vec<Agent>) -> Manifest {
        let mut m = Manifest::new("t");
        m.environments.push(Environment {
            id: "test".into(),
            kind: EnvKind::Test,
            http: Some("http://127.0.0.1:1".into()),
            kafka: None,
            grpc: None,
            sql: None,
            schema_registry: None,
            agents,
            extra: Default::default(),
        });
        m
    }
}
