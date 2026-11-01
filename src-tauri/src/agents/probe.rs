//! Опрос агентов стенда: `GET /v1/info` параллельно, таймаут 5 с, сравнение `major.minor`
//! (спека §7.1, таблица стендов; §7.3).

use std::time::Duration;

use futures::future::join_all;
use glafira_load_engine::agent_api::AgentInfo;
use glafira_load_engine::agent_link::{AgentLink, HttpAgent, same_minor};
use glafira_load_model::project::Agent;
use serde::Serialize;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Версия приложения; агент с другими `major.minor` — состояние `version`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Ready,
    Busy,
    Unreachable,
    Version,
    NoToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: String,
    pub url: String,
    pub state: ProbeState,
    pub info: Option<AgentInfo>,
    /// Причина (`unreachable`), `run_id` (`busy`), обе версии (`version`). Технический текст.
    pub message: Option<String>,
}

/// Один агент: без токена — `no_token` без запроса; иначе `/v1/info` с таймаутом.
pub async fn probe_one(agent: &Agent, token: Option<String>) -> AgentStatus {
    let status = |state, info, message| AgentStatus {
        id: agent.id.clone(),
        url: agent.url.clone(),
        state,
        info,
        message,
    };
    let Some(token) = token else {
        return status(ProbeState::NoToken, None, None);
    };
    let link = HttpAgent::new(&agent.id, &agent.url, &token, APP_VERSION);
    match tokio::time::timeout(PROBE_TIMEOUT, link.info()).await {
        Err(_) => status(
            ProbeState::Unreachable,
            None,
            Some(format!("no response within {} s", PROBE_TIMEOUT.as_secs())),
        ),
        Ok(Err(e)) if e.code == "AGENT_VERSION" => {
            status(ProbeState::Version, None, Some(e.message))
        }
        Ok(Err(e)) => status(
            ProbeState::Unreachable,
            None,
            Some(format!("{}: {}", e.code, e.message)),
        ),
        Ok(Ok(info)) => {
            // `HttpAgent::info` уже сравнивает версии; повтор — на случай другой реализации.
            if !same_minor(&info.version, APP_VERSION) {
                let message = format!("agent {} vs app {APP_VERSION}", info.version);
                status(ProbeState::Version, Some(info), Some(message))
            } else if let Some(run) = info.busy.clone() {
                status(ProbeState::Busy, Some(info), Some(run))
            } else {
                status(ProbeState::Ready, Some(info), None)
            }
        }
    }
}

/// Все агенты стенда параллельно; порядок ответа — порядок `agents`.
pub async fn probe_all(agents: &[Agent], tokens: Vec<Option<String>>) -> Vec<AgentStatus> {
    join_all(
        agents
            .iter()
            .zip(tokens)
            .map(|(agent, token)| probe_one(agent, token)),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::testing::{agent, closed_port_url, info_router, serve};

    #[tokio::test]
    async fn probe_states() {
        let ready = serve(info_router(APP_VERSION, None, "t1")).await;
        let busy = serve(info_router(APP_VERSION, Some("run-7"), "t2")).await;
        let old = serve(info_router("0.1.0", None, "t3")).await;
        let agents = vec![
            agent("ready", &ready),
            agent("busy", &busy),
            agent("old", &old),
            agent("closed", &closed_port_url()),
            agent("no-token", &ready),
            agent("wrong-token", &ready),
        ];
        let tokens = vec![
            Some("t1".to_string()),
            Some("t2".into()),
            Some("t3".into()),
            Some("t4".into()),
            None,
            Some("bad".into()),
        ];
        let out = probe_all(&agents, tokens).await;
        let states: Vec<(&str, ProbeState)> =
            out.iter().map(|s| (s.id.as_str(), s.state)).collect();
        assert_eq!(
            states,
            [
                ("ready", ProbeState::Ready),
                ("busy", ProbeState::Busy),
                ("old", ProbeState::Version),
                ("closed", ProbeState::Unreachable),
                ("no-token", ProbeState::NoToken),
                ("wrong-token", ProbeState::Unreachable),
            ]
        );
        assert_eq!(out[0].info.as_ref().unwrap().cpus, 4);
        assert_eq!(out[1].message.as_deref(), Some("run-7"));
        assert!(out[2].message.as_deref().unwrap_or("").contains("0.1.0"));
        assert!(out[3].message.is_some() && out[3].info.is_none());
        assert!(
            out[5]
                .message
                .as_deref()
                .unwrap_or("")
                .starts_with("UNAUTHORIZED: "),
            "{:?}",
            out[5].message
        );
        assert_eq!(out[4].url, ready);
        let json = serde_json::to_value(&out[4]).unwrap();
        assert_eq!(json["state"], "no_token");
    }
}
