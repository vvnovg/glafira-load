//! Агент: сервер на свободном порту внутри теста, клиент reqwest, стенд — заглушка axum (спека §4.3, §9.1).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use axum::http::StatusCode as AxStatus;
use common::{TOKEN, stand_stub};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use glafira_load_cli::agent::runs::{AgentConfig, AgentState};
use glafira_load_cli::agent::server;
use glafira_load_engine::agent_api::{
    AgentInfo, ApiError, EnvAddresses, Mode, RunListItem, RunRequest, RunStatus,
};
use glafira_load_model::profile::Point;
use glafira_load_model::suite::{self, Arrival};
use predicates::prelude::*;
use reqwest::multipart::{Form, Part};
use reqwest::{Client, StatusCode};
use tempfile::TempDir;

const SUITE: &str = "format: 1\nname: orders-http\nscenarios:\n  - id: create-get\n    steps:\n      - id: create\n        http:\n          operation: \"http:orders:POST /v1/orders\"\n          body: { customerId: \"{{uuid}}\" }\n        expect: { status: 201 }\n        extract: { order_id: \"$.id\" }\n      - id: get\n        http:\n          operation: \"http:orders:GET /v1/orders/{id}\"\n          path: { id: \"{{steps.create.order_id}}\" }\n        expect: { status: 200 }\n";

struct Agent {
    base: String,
    _work: TempDir,
    state: Arc<AgentState>,
}

async fn start_agent(secrets: BTreeMap<String, String>, keep_runs: usize) -> Agent {
    let work = tempfile::tempdir().unwrap();
    let config = AgentConfig {
        work_dir: work.path().to_path_buf(),
        keep_runs,
        drain: Duration::from_millis(1500),
        token: TOKEN.into(),
        secrets_env: secrets,
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let state = AgentState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::serve(
        listener,
        state.clone(),
        std::future::pending(),
    ));
    Agent {
        base: format!("http://{addr}"),
        _work: work,
        state,
    }
}

fn run_id() -> String {
    format!(
        "run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn request(
    mode: Mode,
    minutes: f64,
    stand: &str,
    secrets: Vec<String>,
    controller_version: &str,
) -> RunRequest {
    RunRequest {
        run_id: run_id(),
        agent_id: "gen-1".into(),
        suite_name: "orders-http".into(),
        controller_version: controller_version.into(),
        suite_format: suite::FORMAT,
        env: EnvAddresses {
            http: Some(stand.into()),
            kafka: None,
            schema_registry: None,
        },
        secrets,
        targets: BTreeMap::from([("http:orders:POST /v1/orders".to_string(), 20.0)]),
        timeline: vec![
            Point {
                minute: 0.0,
                pct: 100.0,
            },
            Point {
                minute: minutes,
                pct: 100.0,
            },
        ],
        total_minutes: minutes,
        max_in_flight: 100,
        arrival: Arrival::Constant,
        mode,
        kafka: BTreeMap::new(),
        response_schemas: BTreeMap::new(),
        avro_schemas: BTreeMap::new(),
    }
}

fn bundle(req: &RunRequest, suite_text: &str) -> Form {
    Form::new()
        .part(
            "run.json",
            Part::bytes(serde_json::to_vec(req).unwrap())
                .file_name("run.json")
                .mime_str("application/json")
                .unwrap(),
        )
        .part(
            "suite.yaml",
            Part::text(suite_text.to_string()).file_name("suite.yaml"),
        )
}

async fn post_run(agent: &Agent, req: &RunRequest, suite_text: &str) -> reqwest::Response {
    Client::new()
        .post(format!("{}/v1/runs", agent.base))
        .bearer_auth(TOKEN)
        .multipart(bundle(req, suite_text))
        .send()
        .await
        .unwrap()
}

async fn get_json<T: serde::de::DeserializeOwned>(agent: &Agent, path: &str) -> T {
    Client::new()
        .get(format!("{}{path}", agent.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

struct Sse {
    id: Option<u64>,
    event: String,
    data: String,
}

/// Читает SSE до закрытия потока (агент закрывает его после `done`) или до `limit` событий.
async fn read_sse(agent: &Agent, run_id: &str, last_id: Option<u64>, limit: usize) -> Vec<Sse> {
    let mut rb = Client::new()
        .get(format!("{}/v1/runs/{run_id}/events", agent.base))
        .bearer_auth(TOKEN);
    if let Some(id) = last_id {
        rb = rb.header("Last-Event-ID", id.to_string());
    }
    let resp = rb.send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.bytes_stream().eventsource();
    let mut out = Vec::new();
    while let Some(ev) = stream.next().await {
        let ev = ev.unwrap();
        out.push(Sse {
            id: ev.id.parse().ok(),
            event: ev.event,
            data: ev.data,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_without_token_and_reports_info() {
    let agent = start_agent(BTreeMap::new(), 10).await;
    let c = Client::new();
    let r = c
        .get(format!("{}/v1/info", agent.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(r.json::<ApiError>().await.unwrap().code, "UNAUTHORIZED");
    let r = c
        .get(format!("{}/v1/info", agent.base))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let info: AgentInfo = get_json(&agent, "/v1/info").await;
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert!(info.cpus >= 1);
    assert_eq!(info.busy, None);
    let r = c
        .get(format!("{}/v1/runs/nope/events", agent.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = c
        .get(format!("{}/v1/runs/nope/files", agent.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn smoke_run_streams_done_and_files_contain_summary() {
    let stand = stand_stub(AxStatus::CREATED).await;
    let agent = start_agent(BTreeMap::new(), 10).await;
    let req = request(
        Mode::Smoke,
        0.0,
        &format!("http://{stand}"),
        vec![],
        env!("CARGO_PKG_VERSION"),
    );
    let r = post_run(&agent, &req, SUITE).await;
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    assert_eq!(
        r.json::<serde_json::Value>().await.unwrap()["runId"],
        req.run_id
    );
    let events = read_sse(&agent, &req.run_id, None, usize::MAX).await;
    let last = events.last().unwrap();
    assert_eq!(last.event, "done");
    assert!(last.data.contains("completed"), "{}", last.data);
    assert!(events.iter().all(|e| e.id.is_some()));
    // история и файлы
    let list: Vec<RunListItem> = get_json(&agent, "/v1/runs").await;
    assert!(
        list.iter()
            .any(|r| r.run_id == req.run_id && matches!(r.status, RunStatus::Completed))
    );
    let resp = Client::new()
        .get(format!("{}/v1/runs/{}/files", agent.base, req.run_id))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers()["content-type"], "application/x-tar");
    let bytes = resp.bytes().await.unwrap();
    let mut names = Vec::new();
    for entry in tar::Archive::new(&bytes[..]).entries().unwrap() {
        names.push(
            entry
                .unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
    }
    assert!(names.contains(&"summary.yaml".to_string()), "{names:?}");
    assert!(names.contains(&"agent.log".to_string()), "{names:?}");
    assert!(names.contains(&"run.json".to_string()), "{names:?}");
    assert!(
        names.contains(&"hist.bin".to_string()),
        "LocalAgent files unpacked from tar: {names:?}"
    );
    assert_eq!(agent.state.busy(), None);
    // поток завершённого прогона отдаётся из буфера и снова заканчивается `done`
    let again = read_sse(&agent, &req.run_id, None, usize::MAX).await;
    assert_eq!(again.last().unwrap().event, "done");
}

#[tokio::test(flavor = "multi_thread")]
async fn busy_then_stop_with_last_event_id() {
    let stand = stand_stub(AxStatus::CREATED).await;
    let agent = start_agent(BTreeMap::new(), 10).await;
    let stand = format!("http://{stand}");
    let req = request(Mode::Run, 1.0, &stand, vec![], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        post_run(&agent, &req, SUITE).await.status(),
        StatusCode::ACCEPTED
    );
    // второй старт во время прогона — занят
    let second = request(Mode::Run, 1.0, &stand, vec![], env!("CARGO_PKG_VERSION"));
    let r = post_run(&agent, &second, SUITE).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let err: ApiError = r.json().await.unwrap();
    assert_eq!(err.code, "AGENT_BUSY");
    assert!(err.message.contains(&req.run_id));
    let info: AgentInfo = get_json(&agent, "/v1/info").await;
    assert_eq!(info.busy.as_deref(), Some(req.run_id.as_str()));
    // три события, обрыв, переподключение с Last-Event-ID
    let first = read_sse(&agent, &req.run_id, None, 3).await;
    assert_eq!(first.len(), 3);
    let last_id = first.last().unwrap().id.unwrap();
    let again = read_sse(&agent, &req.run_id, Some(last_id - 1), 1).await;
    assert_eq!(again[0].id, Some(last_id));
    // стоп → done со статусом stopped
    let r = Client::new()
        .post(format!("{}/v1/runs/{}/stop", agent.base, req.run_id))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let rest = read_sse(&agent, &req.run_id, Some(last_id), usize::MAX).await;
    let done = rest.last().unwrap();
    assert_eq!(done.event, "done");
    assert!(done.data.contains("stopped"), "{}", done.data);
    let list: Vec<RunListItem> = get_json(&agent, "/v1/runs").await;
    assert!(matches!(
        list.iter().find(|r| r.run_id == req.run_id).unwrap().status,
        RunStatus::Stopped
    ));
    assert_eq!(agent.state.busy(), None);
    // стоп завершённого прогона — 200, неизвестного — 404
    let r = Client::new()
        .post(format!("{}/v1/runs/{}/stop", agent.base, req.run_id))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = Client::new()
        .post(format!("{}/v1/runs/nope/stop", agent.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_missing_version_and_invalid_suite() {
    let stand = format!("http://{}", stand_stub(AxStatus::CREATED).await);
    let agent = start_agent(BTreeMap::new(), 10).await;
    let with_secret = SUITE.replacen(
        "name: orders-http\n",
        "name: orders-http\nsecrets: [X]\n",
        1,
    );
    let req = request(
        Mode::Smoke,
        0.0,
        &stand,
        vec!["X".into()],
        env!("CARGO_PKG_VERSION"),
    );
    let r = post_run(&agent, &req, &with_secret).await;
    assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
    let err: ApiError = r.json().await.unwrap();
    assert_eq!(err.code, "SECRET_MISSING");
    assert_eq!(err.names, vec!["X"]);
    // с секретом в окружении агента прогон принимается
    let agent2 = start_agent(BTreeMap::from([("X".to_string(), "v".to_string())]), 10).await;
    let req2 = request(
        Mode::Smoke,
        0.0,
        &stand,
        vec!["X".into()],
        env!("CARGO_PKG_VERSION"),
    );
    assert_eq!(
        post_run(&agent2, &req2, &with_secret).await.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        read_sse(&agent2, &req2.run_id, None, usize::MAX)
            .await
            .last()
            .unwrap()
            .event,
        "done"
    );
    // версия контроллера другой minor
    let req3 = request(Mode::Smoke, 0.0, &stand, vec![], "9.9.0");
    let r = post_run(&agent, &req3, SUITE).await;
    assert_eq!(r.status(), StatusCode::UPGRADE_REQUIRED);
    let err: ApiError = r.json().await.unwrap();
    assert_eq!(err.code, "AGENT_VERSION");
    assert_eq!(err.controller.as_deref(), Some("9.9.0"));
    assert_eq!(err.agent.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    // невалидный сьют
    let req4 = request(Mode::Smoke, 0.0, &stand, vec![], env!("CARGO_PKG_VERSION"));
    let r = post_run(
        &agent,
        &req4,
        "format: 1\nname: x\nscenarios: []\nbogus: 1\n",
    )
    .await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.json::<ApiError>().await.unwrap().code, "SUITE_INVALID");
    // бандл без run.json
    let r = Client::new()
        .post(format!("{}/v1/runs", agent.base))
        .bearer_auth(TOKEN)
        .multipart(Form::new().part("suite.yaml", Part::text(SUITE)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(agent.state.busy(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_runs_removes_old_folders() {
    let stand = format!("http://{}", stand_stub(AxStatus::CREATED).await);
    let agent = start_agent(BTreeMap::new(), 1).await;
    let a = request(Mode::Smoke, 0.0, &stand, vec![], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        post_run(&agent, &a, SUITE).await.status(),
        StatusCode::ACCEPTED
    );
    read_sse(&agent, &a.run_id, None, usize::MAX).await;
    let dir_a = agent.state.run_dir(&a.run_id).unwrap();
    assert!(dir_a.is_dir());
    let b = request(Mode::Smoke, 0.0, &stand, vec![], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        post_run(&agent, &b, SUITE).await.status(),
        StatusCode::ACCEPTED
    );
    read_sse(&agent, &b.run_id, None, usize::MAX).await;
    assert!(!dir_a.is_dir());
    assert!(agent.state.run_dir(&b.run_id).unwrap().is_dir());
    let list: Vec<RunListItem> = get_json(&agent, "/v1/runs").await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].run_id, b.run_id);
}

#[test]
fn agent_without_token_exits_with_3() {
    Command::from_std(std::process::Command::new(env!(
        "CARGO_BIN_EXE_glafira-load"
    )))
    .args(["agent", "--listen", "127.0.0.1:0"])
    .env_remove("GLAFIRA_LOAD_AGENT_TOKEN")
    .assert()
    .code(3)
    .stderr(predicate::str::contains("token"));
    Command::from_std(std::process::Command::new(env!(
        "CARGO_BIN_EXE_glafira-load"
    )))
    .args(["agent", "--listen", "not-an-address", "--token", "t"])
    .assert()
    .code(3)
    .stderr(predicate::str::contains("--listen"));
}
