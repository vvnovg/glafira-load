//! `HttpAgent` движка против настоящего агента `glafira-load agent` (в процессе, на свободном порту):
//! имена частей multipart, коды ошибок, SSE с `Last-Event-ID`, остановка и файлы после `done`.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode as AxStatus;
use common::{TOKEN, stand_stub};
use futures::StreamExt;
use glafira_load_cli::agent::runs::{AgentConfig, AgentState};
use glafira_load_cli::agent::server;
use glafira_load_engine::agent_api::{EnvAddresses, Event, Mode, RunRequest, RunStatus};
use glafira_load_engine::agent_link::{AgentLink, HttpAgent};
use glafira_load_model::profile::Point;
use glafira_load_model::suite::{self, Arrival};
use tempfile::TempDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SUITE: &str = "format: 1\nname: orders-http\nscenarios:\n  - id: create-get\n    steps:\n      - id: create\n        http:\n          operation: \"http:orders:POST /v1/orders\"\n          body: { customerId: \"{{uuid}}\" }\n        expect: { status: 201 }\n        extract: { order_id: \"$.id\" }\n      - id: get\n        http:\n          operation: \"http:orders:GET /v1/orders/{id}\"\n          path: { id: \"{{steps.create.order_id}}\" }\n        expect: { status: 200 }\n";

struct Agent {
    base: String,
    _work: TempDir,
    _state: Arc<AgentState>,
}

async fn start_agent(secrets: BTreeMap<String, String>) -> Agent {
    let work = tempfile::tempdir().unwrap();
    let state = AgentState::new(AgentConfig {
        work_dir: work.path().to_path_buf(),
        keep_runs: 10,
        drain: Duration::from_millis(1500),
        token: TOKEN.into(),
        secrets_env: secrets,
        version: VERSION.into(),
    })
    .unwrap();
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
        _state: state,
    }
}

fn request(
    run_id: &str,
    mode: Mode,
    minutes: f64,
    stand: &str,
    secrets: Vec<String>,
) -> RunRequest {
    RunRequest {
        run_id: run_id.into(),
        agent_id: "gen-1".into(),
        suite_name: "orders-http".into(),
        controller_version: VERSION.into(),
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

fn tar_names(bytes: &[u8]) -> Vec<String> {
    tar::Archive::new(bytes)
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// Все события потока до его закрытия агентом (после `done`).
async fn all_events(a: &HttpAgent, run_id: &str, last_id: Option<u64>) -> Vec<(u64, Event)> {
    tokio::time::timeout(
        Duration::from_secs(60),
        a.events(run_id, last_id).await.unwrap().collect(),
    )
    .await
    .expect("the agent closes the stream after done")
}

#[tokio::test(flavor = "multi_thread")]
async fn info_start_with_data_events_and_files() {
    let stand = format!("http://{}", stand_stub(AxStatus::CREATED).await);
    let agent = start_agent(BTreeMap::new()).await;
    let a = HttpAgent::new("gen-1", &agent.base, TOKEN, VERSION);

    let info = a.info().await.unwrap();
    assert_eq!((info.version.as_str(), info.busy), (VERSION, None));
    let wrong = HttpAgent::new("gen-1", &agent.base, "wrong", VERSION);
    assert_eq!(wrong.info().await.unwrap_err().code, "UNAUTHORIZED");
    let e = HttpAgent::new("gen-1", &agent.base, TOKEN, "9.9.0")
        .info()
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.names.as_slice()),
        (
            "AGENT_VERSION",
            &[VERSION.to_string(), "9.9.0".to_string()][..]
        )
    );

    // набор данных доходит до агента частью `data/users`: без неё агент отвечает 422
    let with_data = SUITE.replacen(
        "name: orders-http\n",
        "name: orders-http\ndata:\n  users: data/users.csv\n",
        1,
    );
    let e = a
        .start(
            &request("r-nodata", Mode::Smoke, 0.0, &stand, vec![]),
            &with_data,
            &BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "SUITE_INVALID");
    let data = BTreeMap::from([("users".to_string(), b"id\n1\n2\n".to_vec())]);
    a.start(
        &request("r-smoke", Mode::Smoke, 0.0, &stand, vec![]),
        &with_data,
        &data,
    )
    .await
    .unwrap();

    let events = all_events(&a, "r-smoke", None).await;
    let ids: Vec<u64> = events.iter().map(|(id, _)| *id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    match &events.last().unwrap().1 {
        Event::Done(summary) => assert_eq!(summary.status, RunStatus::Completed),
        other => panic!("last event is not done: {other:?}"),
    }
    let names = tar_names(&a.files("r-smoke").await.unwrap());
    for name in ["summary.yaml", "hist.bin", "run.json", "agent.log"] {
        assert!(names.contains(&name.to_string()), "{name} in {names:?}");
    }
    // старт с тем же run id — 400 агента, код из тела ответа
    let e = a
        .start(
            &request("r-smoke", Mode::Smoke, 0.0, &stand, vec![]),
            SUITE,
            &BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "BAD_REQUEST");
    assert_eq!(a.files("nope").await.unwrap_err().code, "NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread")]
async fn busy_resume_stop_and_secret_missing() {
    let stand = format!("http://{}", stand_stub(AxStatus::CREATED).await);
    let agent = start_agent(BTreeMap::new()).await;
    let a = HttpAgent::new("gen-1", &agent.base, TOKEN, VERSION);

    let with_secret = SUITE.replacen(
        "name: orders-http\n",
        "name: orders-http\nsecrets: [X]\n",
        1,
    );
    let e = a
        .start(
            &request("r-secret", Mode::Smoke, 0.0, &stand, vec!["X".into()]),
            &with_secret,
            &BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.names.as_slice()),
        ("SECRET_MISSING", &["X".to_string()][..])
    );

    a.start(
        &request("r-long", Mode::Run, 1.0, &stand, vec![]),
        SUITE,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(a.info().await.unwrap().busy.as_deref(), Some("r-long"));
    let e = a
        .start(
            &request("r-second", Mode::Run, 1.0, &stand, vec![]),
            SUITE,
            &BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "AGENT_BUSY");
    assert!(e.message.contains("r-long"), "{}", e.message);
    assert_eq!(a.files("r-long").await.unwrap_err().code, "AGENT_BUSY");

    // два события, обрыв (поток брошен), продолжение с `Last-Event-ID`
    let first: Vec<(u64, Event)> = a
        .events("r-long", None)
        .await
        .unwrap()
        .take(2)
        .collect()
        .await;
    let last_id = first[1].0;
    let mut resumed = a.events("r-long", Some(last_id)).await.unwrap();
    let (next_id, _) = resumed.next().await.unwrap();
    assert_eq!(next_id, last_id + 1);
    drop(resumed);

    a.stop("r-long").await.unwrap();
    let rest = all_events(&a, "r-long", Some(last_id)).await;
    assert!(rest.iter().all(|(id, _)| *id > last_id));
    match &rest.last().unwrap().1 {
        Event::Done(summary) => assert_eq!(summary.status, RunStatus::Stopped),
        other => panic!("last event is not done: {other:?}"),
    }
    assert!(tar_names(&a.files("r-long").await.unwrap()).contains(&"summary.yaml".to_string()));
    assert_eq!(a.info().await.unwrap().busy, None);
    // остановка завершённого и неизвестного прогона — не ошибка
    a.stop("r-long").await.unwrap();
    a.stop("nope").await.unwrap();
}
