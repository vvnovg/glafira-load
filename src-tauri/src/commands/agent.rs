//! Команды агентов стенда: токен в хранилище ОС и опрос `/v1/info` (спека §7.2).

use tauri::State;

use super::blocking;
use crate::agents::probe::{AgentStatus, ProbeState, probe_all};
use crate::agents::{env_of, no_agent, token_key};
use crate::error::{AppError, AppResult};
use crate::project::manifest::ProjectDir;
use crate::state::AppState;

/// Сохраняет токен агента; `None` или пустая строка — удаляет. Агент должен быть в стенде.
pub async fn set_token(
    st: &AppState,
    path: String,
    env_id: String,
    agent_id: String,
    token: Option<String>,
) -> AppResult<()> {
    let st = st.clone();
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let env = env_of(&d.manifest()?, &env_id)?;
        if !env.agents.iter().any(|a| a.id == agent_id) {
            return Err(no_agent(&env_id, &agent_id));
        }
        let key = token_key(d.root(), &env_id, &agent_id);
        match token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        {
            Some(t) => st.secrets.set(&key, &t),
            None => st.secrets.delete(&key),
        }
    })
    .await
}

#[tauri::command]
pub async fn agent_set_token(
    state: State<'_, AppState>,
    path: String,
    env_id: String,
    agent_id: String,
    token: Option<String>,
) -> AppResult<()> {
    set_token(state.inner(), path, env_id, agent_id, token).await
}

/// Опрос всех агентов стенда: токены читаются в блокирующей задаче, запросы идут параллельно.
/// Ошибка хранилища при чтении токена одного агента не валит опрос: этот агент —
/// `unreachable` с причиной в `message`, остальные опрашиваются как обычно.
pub async fn probe(st: &AppState, path: String, env_id: String) -> AppResult<Vec<AgentStatus>> {
    let st = st.clone();
    let (agents, tokens) = blocking(move || {
        let d = ProjectDir::open(&path)?;
        let env = env_of(&d.manifest()?, &env_id)?;
        let tokens = env
            .agents
            .iter()
            .map(|a| st.secrets.get(&token_key(d.root(), &env_id, &a.id)))
            .collect::<Vec<_>>();
        Ok((env.agents, tokens))
    })
    .await?;
    // Агент с ошибкой хранилища идёт в опрос как «без токена» (запроса не будет), затем его
    // статус заменяется на `unreachable` с причиной.
    let failures: Vec<Option<String>> = tokens
        .iter()
        .map(|t| t.as_ref().err().map(store_error_text))
        .collect();
    let tokens = tokens.into_iter().map(|t| t.ok().flatten()).collect();
    let mut out = probe_all(&agents, tokens).await;
    for (status, failure) in out.iter_mut().zip(failures) {
        if let Some(message) = failure {
            status.state = ProbeState::Unreachable;
            status.info = None;
            status.message = Some(message);
        }
    }
    Ok(out)
}

/// Технический текст ошибки хранилища токенов для `AgentStatus.message`.
fn store_error_text(e: &AppError) -> String {
    match &e.details {
        Some(d) => format!("{}: {d}", e.message),
        None => e.message.clone(),
    }
}

#[tauri::command]
pub async fn agent_probe(
    state: State<'_, AppState>,
    path: String,
    env_id: String,
) -> AppResult<Vec<AgentStatus>> {
    probe(state.inner(), path, env_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::probe::{APP_VERSION, ProbeState};
    use crate::agents::testing::{
        agent, closed_port_url, info_router, manifest_with_agents, serve,
    };
    use crate::error::ErrorCode;
    use crate::project::manifest::create;
    use crate::storage::secrets::{MemoryStore, SecretStore};
    use std::sync::Arc;

    /// Хранилище, которое не читает ключи, оканчивающиеся на `fail`; остальное — как `MemoryStore`.
    #[derive(Default)]
    struct FlakyStore {
        inner: MemoryStore,
    }

    impl SecretStore for FlakyStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            if key.ends_with("fail") {
                return Err(app_err!(
                    ErrorCode::Internal,
                    "errors.secrets.keychain",
                    "Cannot access the system secret store"
                )
                .with_details("locked"));
            }
            self.inner.get(key)
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.inner.set(key, value)
        }

        fn delete(&self, key: &str) -> AppResult<()> {
            self.inner.delete(key)
        }
    }

    fn state(dir: &std::path::Path) -> AppState {
        AppState::new(dir)
            .unwrap()
            .with_secrets(Arc::new(MemoryStore::default()))
    }

    #[tokio::test]
    async fn token_is_stored_under_the_project_key_and_probe_reports_states() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(&tmp.path().join("data"));
        let path = tmp.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let ready = serve(info_router(APP_VERSION, None, "secret-1")).await;
        let d = ProjectDir::open(&path).unwrap();
        d.save_manifest(&manifest_with_agents(vec![
            agent("gen-1", &ready),
            agent("gen-2", &closed_port_url()),
            agent("gen-3", &ready),
        ]))
        .unwrap();
        set_token(
            &st,
            path.clone(),
            "test".into(),
            "gen-1".into(),
            Some(" secret-1\n".into()),
        )
        .await
        .unwrap();
        set_token(
            &st,
            path.clone(),
            "test".into(),
            "gen-2".into(),
            Some("x".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            st.secrets
                .get(&token_key(d.root(), "test", "gen-1"))
                .unwrap()
                .as_deref(),
            Some("secret-1"),
            "токен обрезан и лежит под ключом проект/стенд/агент"
        );
        assert_eq!(
            token_key(d.root(), "test", "gen-1"),
            format!("{}/test/gen-1", d.root().display())
        );
        let out = probe(&st, path.clone(), "test".into()).await.unwrap();
        let states: Vec<ProbeState> = out.iter().map(|s| s.state).collect();
        assert_eq!(
            states,
            [
                ProbeState::Ready,
                ProbeState::Unreachable,
                ProbeState::NoToken
            ]
        );
        // Пустой токен удаляет запись.
        set_token(
            &st,
            path.clone(),
            "test".into(),
            "gen-1".into(),
            Some("  ".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            st.secrets
                .get(&token_key(d.root(), "test", "gen-1"))
                .unwrap(),
            None
        );
        set_token(&st, path.clone(), "test".into(), "gen-2".into(), None)
            .await
            .unwrap();
        let out = probe(&st, path.clone(), "test".into()).await.unwrap();
        assert!(out.iter().all(|s| s.state == ProbeState::NoToken));
        assert_eq!(
            set_token(
                &st,
                path.clone(),
                "test".into(),
                "nope".into(),
                Some("t".into())
            )
            .await
            .unwrap_err()
            .key,
            "errors.agent.notFound"
        );
        assert_eq!(
            probe(&st, path, "prod".into()).await.unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn probe_on_the_fixture_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(&tmp.path().join("data"));
        let (_fixture, path) = crate::commands::suite::tests::copy_fixture_project();
        let ready = serve(info_router(APP_VERSION, Some("r-1"), "t")).await;
        crate::commands::suite::tests::point_env_to(
            &path,
            "http://127.0.0.1:1",
            vec![agent("gen-1", &ready), agent("gen-2", &closed_port_url())],
        );
        for id in ["gen-1", "gen-2"] {
            set_token(
                &st,
                path.clone(),
                "test".into(),
                id.into(),
                Some("t".into()),
            )
            .await
            .unwrap();
        }
        let out = probe(&st, path, "test".into()).await.unwrap();
        let states: Vec<(&str, ProbeState)> =
            out.iter().map(|s| (s.id.as_str(), s.state)).collect();
        assert_eq!(
            states,
            [
                ("gen-1", ProbeState::Busy),
                ("gen-2", ProbeState::Unreachable)
            ]
        );
        assert!(
            out[1]
                .message
                .as_deref()
                .unwrap_or("")
                .starts_with("AGENT_UNREACHABLE: "),
            "{:?}",
            out[1].message
        );
    }

    #[tokio::test]
    async fn probe_survives_a_keystore_error_for_one_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let st = AppState::new(&tmp.path().join("data"))
            .unwrap()
            .with_secrets(Arc::new(FlakyStore::default()));
        let path = tmp.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let ready = serve(info_router(APP_VERSION, None, "secret-1")).await;
        let d = ProjectDir::open(&path).unwrap();
        d.save_manifest(&manifest_with_agents(vec![
            agent("gen-1", &ready),
            agent("fail", &ready),
            agent("gen-3", &ready),
        ]))
        .unwrap();
        set_token(
            &st,
            path.clone(),
            "test".into(),
            "gen-1".into(),
            Some("secret-1".into()),
        )
        .await
        .unwrap();
        let out = probe(&st, path, "test".into()).await.unwrap();
        let states: Vec<ProbeState> = out.iter().map(|s| s.state).collect();
        assert_eq!(
            states,
            [
                ProbeState::Ready,
                ProbeState::Unreachable,
                ProbeState::NoToken
            ]
        );
        let message = out[1].message.as_deref().unwrap_or("");
        assert!(
            message.contains("secret store") && message.contains("locked"),
            "{message}"
        );
    }
}
