//! `HttpAgent` против axum-заглушки агента: коды ошибок, multipart, SSE с `Last-Event-ID`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use glafira_load_engine::agent_api::{AgentInfo, ApiError, EnvAddresses, Event, Mode, RunRequest};
use glafira_load_engine::agent_link::{AgentLink, HttpAgent, same_minor};
use glafira_load_model::suite::Arrival;

#[derive(Clone)]
struct Stub {
    info_status: StatusCode,
    start_status: StatusCode,
    busy: Option<String>,
    version: String,
    /// Поток начинается с `gap` без `id:` — так реальный агент сообщает о вытесненных событиях.
    gap_first: bool,
    seen_parts: Arc<std::sync::Mutex<Vec<(String, usize)>>>,
    last_event_id: Arc<std::sync::Mutex<Option<String>>>,
    stops: Arc<AtomicUsize>,
}

fn authorized(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer t0k")
}

async fn info(State(s): State<Stub>, h: HeaderMap) -> Response {
    if !authorized(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if s.info_status != StatusCode::OK {
        return s.info_status.into_response();
    }
    Json(AgentInfo {
        version: s.version.clone(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cpus: 4,
        busy: s.busy.clone(),
    })
    .into_response()
}

async fn start(State(s): State<Stub>, h: HeaderMap, mut mp: Multipart) -> Response {
    if !authorized(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    while let Ok(Some(field)) = mp.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        let bytes = field.bytes().await.unwrap();
        s.seen_parts.lock().unwrap().push((name, bytes.len()));
    }
    match s.start_status.as_u16() {
        202 => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "runId": "r1" })),
        )
            .into_response(),
        412 => (
            StatusCode::PRECONDITION_FAILED,
            Json(ApiError {
                code: "SECRET_MISSING".into(),
                message: "missing secrets".into(),
                names: vec!["API_TOKEN".into()],
                agent: None,
                controller: None,
            }),
        )
            .into_response(),
        426 => (
            StatusCode::UPGRADE_REQUIRED,
            Json(ApiError {
                code: "AGENT_VERSION".into(),
                message: "version mismatch".into(),
                names: vec![],
                agent: Some("0.9.0".into()),
                controller: Some("0.2.0".into()),
            }),
        )
            .into_response(),
        409 => (StatusCode::CONFLICT, Body::from("not json")).into_response(),
        code => StatusCode::from_u16(code).unwrap().into_response(),
    }
}

async fn events(State(s): State<Stub>, h: HeaderMap, Path(id): Path<String>) -> Response {
    if !authorized(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let sse = |body: Body| {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(body)
            .unwrap()
    };
    // Агент «замолкает»: одно событие, затем соединение висит без байтов.
    if id == "silent" {
        let first =
            "id: 1\nevent: phase\ndata: {\"type\":\"phase\",\"t\":1,\"index\":0,\"pct\":5.0}\n\n";
        let chunks =
            futures::stream::once(
                async move { Ok::<_, std::convert::Infallible>(first.to_string()) },
            )
            .chain(futures::stream::pending());
        return sse(Body::from_stream(chunks));
    }
    let last = h
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    *s.last_event_id.lock().unwrap() = last.clone();
    let from: u64 = last
        .and_then(|v| v.parse().ok())
        .map(|v: u64| v + 1)
        .unwrap_or(1);
    let mut body = String::new();
    if s.gap_first {
        body += "event: gap\ndata: {\"type\":\"gap\"}\n\n";
    }
    for id in from..=3 {
        let data = if id == 3 {
            "{\"type\":\"gap\"}".to_string()
        } else {
            format!("{{\"type\":\"phase\",\"t\":{id},\"index\":0,\"pct\":50.0}}")
        };
        body += &format!("id: {id}\nevent: x\ndata: {data}\n\n");
    }
    sse(Body::from(body))
}

async fn stop(State(s): State<Stub>, h: HeaderMap) -> StatusCode {
    if !authorized(&h) {
        return StatusCode::UNAUTHORIZED;
    }
    s.stops.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

async fn files(h: HeaderMap) -> Response {
    if !authorized(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Body::from(vec![1u8, 2, 3]).into_response()
}

async fn serve(stub: Stub) -> String {
    let app = Router::new()
        .route("/v1/info", get(info))
        .route("/v1/runs", post(start))
        .route("/v1/runs/{id}/events", get(events))
        .route("/v1/runs/{id}/stop", post(stop))
        .route("/v1/runs/{id}/files", get(files))
        .with_state(stub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn stub() -> Stub {
    Stub {
        info_status: StatusCode::OK,
        start_status: StatusCode::ACCEPTED,
        busy: None,
        version: "0.2.0".into(),
        gap_first: false,
        seen_parts: Default::default(),
        last_event_id: Default::default(),
        stops: Default::default(),
    }
}

fn request() -> RunRequest {
    RunRequest {
        run_id: "r1".into(),
        agent_id: "a".into(),
        suite_name: "orders".into(),
        controller_version: "0.2.0".into(),
        suite_format: 1,
        env: EnvAddresses {
            http: Some("http://x".into()),
            kafka: None,
            schema_registry: None,
        },
        secrets: vec![],
        targets: BTreeMap::new(),
        timeline: vec![],
        total_minutes: 0.0,
        max_in_flight: 10,
        arrival: Arrival::Poisson,
        mode: Mode::Run,
        kafka: BTreeMap::new(),
        response_schemas: BTreeMap::new(),
        avro_schemas: BTreeMap::new(),
    }
}

#[test]
fn version_compare_uses_major_minor() {
    assert!(same_minor("0.2.0", "0.2.7"));
    assert!(!same_minor("0.3.0", "0.2.7"));
    assert!(!same_minor("1.2.0", "0.2.0"));
    assert!(!same_minor("junk", "0.2.0"));
}

#[tokio::test]
async fn info_maps_network_auth_busy_and_version() {
    let url = serve(stub()).await;
    let a = HttpAgent::new("a", &url, "t0k", "0.2.9");
    let info = a.info().await.unwrap();
    assert_eq!(
        (info.version.as_str(), info.cpus, info.busy),
        ("0.2.0", 4, None)
    );

    let bad = HttpAgent::new("a", &url, "wrong", "0.2.0");
    assert_eq!(bad.info().await.unwrap_err().code, "UNAUTHORIZED");

    let down = HttpAgent::new("a", "http://127.0.0.1:1", "t0k", "0.2.0");
    assert_eq!(down.info().await.unwrap_err().code, "AGENT_UNREACHABLE");

    let old = serve(Stub {
        version: "0.1.0".into(),
        ..stub()
    })
    .await;
    let e = HttpAgent::new("a", &old, "t0k", "0.2.0")
        .info()
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.names.as_slice()),
        (
            "AGENT_VERSION",
            &["0.1.0".to_string(), "0.2.0".to_string()][..]
        )
    );

    let busy = serve(Stub {
        busy: Some("r9".into()),
        ..stub()
    })
    .await;
    assert_eq!(
        HttpAgent::new("a", &busy, "t0k", "0.2.0")
            .info()
            .await
            .unwrap()
            .busy
            .as_deref(),
        Some("r9")
    );

    let broken = serve(Stub {
        info_status: StatusCode::INTERNAL_SERVER_ERROR,
        ..stub()
    })
    .await;
    assert_eq!(
        HttpAgent::new("a", &broken, "t0k", "0.2.0")
            .info()
            .await
            .unwrap_err()
            .code,
        "AGENT_UNREACHABLE"
    );
}

#[tokio::test]
async fn start_sends_multipart_and_maps_statuses() {
    let s = stub();
    let url = serve(s.clone()).await;
    let a = HttpAgent::new("a", &url, "t0k", "0.2.0");
    let mut data = BTreeMap::new();
    data.insert("users".to_string(), b"id\n1\n".to_vec());
    a.start(&request(), "format: 1\n", &data).await.unwrap();
    let parts = s.seen_parts.lock().unwrap().clone();
    let names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["run.json", "suite.yaml", "data/users"]);
    assert_eq!(parts[2].1, 5);

    let url = serve(Stub {
        start_status: StatusCode::PRECONDITION_FAILED,
        ..stub()
    })
    .await;
    let e = HttpAgent::new("a", &url, "t0k", "0.2.0")
        .start(&request(), "", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.names.as_slice()),
        ("SECRET_MISSING", &["API_TOKEN".to_string()][..])
    );

    let url = serve(Stub {
        start_status: StatusCode::UPGRADE_REQUIRED,
        ..stub()
    })
    .await;
    let e = HttpAgent::new("a", &url, "t0k", "0.2.0")
        .start(&request(), "", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.names.as_slice()),
        (
            "AGENT_VERSION",
            &["0.9.0".to_string(), "0.2.0".to_string()][..]
        )
    );

    let url = serve(Stub {
        start_status: StatusCode::CONFLICT,
        ..stub()
    })
    .await;
    let e = HttpAgent::new("a", &url, "t0k", "0.2.0")
        .start(&request(), "", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        e.code, "AGENT_BUSY",
        "409 without a JSON body maps by status"
    );

    let url = serve(Stub {
        start_status: StatusCode::UNPROCESSABLE_ENTITY,
        ..stub()
    })
    .await;
    let e = HttpAgent::new("a", &url, "t0k", "0.2.0")
        .start(&request(), "", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(e.code, "SUITE_INVALID");
}

#[tokio::test]
async fn events_parse_ids_and_resume_with_last_event_id_then_stop_and_files() {
    let s = stub();
    let url = serve(s.clone()).await;
    let a = HttpAgent::new("a", &url, "t0k", "0.2.0");
    let got: Vec<(u64, Event)> = a.events("r1", None).await.unwrap().collect().await;
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].0, 1);
    assert!(matches!(&got[0].1, Event::Phase(p) if p.t == 1));
    assert!(matches!(got[2].1, Event::Gap));
    assert_eq!(*s.last_event_id.lock().unwrap(), None);

    let got: Vec<(u64, Event)> = a.events("r1", Some(2)).await.unwrap().collect().await;
    assert_eq!(got.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![3]);
    assert_eq!(s.last_event_id.lock().unwrap().as_deref(), Some("2"));

    a.stop("r1").await.unwrap();
    assert_eq!(s.stops.load(Ordering::SeqCst), 1);
    assert_eq!(a.files("r1").await.unwrap(), vec![1, 2, 3]);
    assert_eq!(a.id(), "a");

    let bad = HttpAgent::new("a", &url, "nope", "0.2.0");
    assert_eq!(
        bad.events("r1", None)
            .await
            .err()
            .expect("events without a valid token fail")
            .code,
        "UNAUTHORIZED"
    );
    assert_eq!(bad.files("r1").await.unwrap_err().code, "UNAUTHORIZED");
}

#[tokio::test]
async fn gap_without_id_keeps_the_resume_id() {
    let url = serve(Stub {
        gap_first: true,
        ..stub()
    })
    .await;
    let a = HttpAgent::new("a", &url, "t0k", "0.2.0");
    let got: Vec<(u64, Event)> = a.events("r1", Some(2)).await.unwrap().collect().await;
    assert_eq!(got.len(), 2, "gap without id is not dropped: {got:?}");
    assert!(matches!(got[0], (2, Event::Gap)), "{got:?}");
    assert!(matches!(got[1], (3, Event::Gap)), "{got:?}");

    let got: Vec<(u64, Event)> = a.events("r1", None).await.unwrap().collect().await;
    assert!(matches!(got[0], (0, Event::Gap)), "{got:?}");
    assert_eq!(got.len(), 4);
}

#[tokio::test]
async fn silent_agent_ends_the_event_stream() {
    let url = serve(stub()).await;
    let a = HttpAgent::new("a", &url, "t0k", "0.2.0").with_idle_timeout(Duration::from_millis(300));
    let got: Vec<(u64, Event)> = tokio::time::timeout(
        Duration::from_secs(5),
        a.events("silent", None).await.unwrap().collect(),
    )
    .await
    .expect("stream ends after the idle timeout");
    assert_eq!(got.len(), 1);
    assert!(matches!(&got[0], (1, Event::Phase(p)) if p.t == 1));
}
