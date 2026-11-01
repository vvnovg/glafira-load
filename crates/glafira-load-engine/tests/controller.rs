//! Контроллер с фиктивными агентами: сбор, отказы до старта, потеря агента, отмена (спека §4.5).

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use glafira_load_engine::agent_api::{
    AgentInfo, ErrorSample, Event, Mode, OpTick, PhaseEvent, RunRequest, RunStatus, Tick,
};
use glafira_load_engine::agent_link::{AgentLink, LinkError};
use glafira_load_engine::controller::{AgentState, Controller, ControllerEvent, RunPlan};
use glafira_load_engine::metrics::{OpTotals, Totals, new_histogram, summary};
use glafira_load_engine::run_files::hist_log;
use glafira_load_model::profile::{Basis, Computed, Point, Target};
use glafira_load_model::project::{Agent, EnvKind, Environment};
use glafira_load_model::registry::OpKind;
use glafira_load_model::suite::parse;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const OP: &str = "http:orders:GET /ping";

const SUITE: &str = "format: 1
name: orders
scenarios:
  - id: ping
    steps:
      - http: { operation: \"http:orders:GET /ping\" }
        expect: { status: 200 }
";

fn plan(root: &Path) -> RunPlan {
    RunPlan {
        root: root.to_path_buf(),
        suite_name: "orders".into(),
        suite: parse(SUITE).unwrap(),
        suite_text: SUITE.into(),
        profile: "short".into(),
        computed: Computed {
            profile: "short".into(),
            basis: Basis::Avg,
            targets: vec![Target {
                id: OP.into(),
                service: "orders".into(),
                kind: OpKind::Http,
                basis_value: 100.0,
                k_env: 1.0,
                k_growth: 1.0,
                target: 100.0,
                share: 1.0,
            }],
            total: 100.0,
            uncovered: vec![],
            timeline: vec![
                Point {
                    minute: 0.0,
                    pct: 0.0,
                },
                Point {
                    minute: 0.05,
                    pct: 100.0,
                },
            ],
            total_minutes: 0.05,
        },
        env: Environment {
            id: "test".into(),
            kind: EnvKind::Test,
            http: Some("http://127.0.0.1:1".into()),
            kafka: None,
            grpc: None,
            sql: None,
            schema_registry: None,
            agents: vec![
                Agent {
                    id: "a".into(),
                    url: "http://a".into(),
                    weight: 1,
                },
                Agent {
                    id: "b".into(),
                    url: "http://b".into(),
                    weight: 1,
                },
            ],
            extra: Default::default(),
        },
        mode: Mode::Run,
        max_in_flight: None,
        out_dir: None,
        response_schemas: BTreeMap::new(),
        avro_schemas: BTreeMap::new(),
    }
}

fn tick(t: u64, n: u64) -> Event {
    Event::Tick(Tick {
        t,
        ops: vec![OpTick {
            id: OP.into(),
            n,
            err: BTreeMap::new(),
            dropped: 0,
            p50: 10.0,
            p90: 20.0,
            p95: 30.0,
            p99: 40.0,
            max: 50.0,
            bytes_in: n,
            bytes_out: n,
            lag: None,
            hist: None,
        }],
        scenarios: vec![],
    })
}

/// Итог агента на `n` замеров по 10 мс за 2 секунды.
fn totals(n: u64) -> Totals {
    let mut totals = Totals {
        ops: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        seconds: 2,
    };
    let mut hist = new_histogram();
    for _ in 0..n {
        hist.record(10_000).unwrap();
    }
    totals.ops.insert(
        OP.into(),
        OpTotals {
            hist,
            n,
            err: BTreeMap::new(),
            dropped: 0,
            bytes_in: n,
            bytes_out: n,
            per_second_max: n,
        },
    );
    totals
}

fn done(n: u64) -> Event {
    let mut targets = BTreeMap::new();
    targets.insert(OP.to_string(), 50.0);
    Event::Done(summary(&totals(n), &targets, RunStatus::Completed))
}

fn error(agent: &str) -> Event {
    Event::Error(ErrorSample {
        t: 0,
        code: "HTTP_5xx".into(),
        operation: OP.into(),
        scenario: "ping".into(),
        step: "step1".into(),
        agent: agent.into(),
        message: "boom".into(),
        request: None,
        response: None,
    })
}

/// Построитель tar с `summary.yaml` и `hist.bin` (писатель Task 8) на `n` замеров по 10 мс.
fn tar_builder(n: u64) -> tar::Builder<Vec<u8>> {
    let hist = hist_log(&totals(n)).unwrap();
    let summary = match done(n) {
        Event::Done(s) => glafira_load_model::to_yaml(&s),
        _ => unreachable!(),
    };
    let mut tar = tar::Builder::new(Vec::new());
    for (name, body) in [("summary.yaml", summary.into_bytes()), ("hist.bin", hist)] {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, name, &body[..]).unwrap();
    }
    tar
}

fn files_tar(n: u64) -> Vec<u8> {
    tar_builder(n).into_inner().unwrap()
}

/// Очередь потоков событий: по элементу на вызов `events`.
type Streams = Mutex<VecDeque<Result<Vec<(u64, Event)>, LinkError>>>;

/// Фиктивный агент: настраиваемые ответы, потоки событий из векторов по порядку вызовов `events`.
struct MockAgent {
    id: String,
    info: Result<AgentInfo, LinkError>,
    start: Result<(), LinkError>,
    streams: Streams,
    files: Vec<u8>,
    /// Сколько первых вызовов `files` ответят `AGENT_BUSY`.
    files_busy: AtomicUsize,
    files_calls: AtomicUsize,
    starts: AtomicUsize,
    stops: AtomicUsize,
    events_calls: Mutex<Vec<Option<u64>>>,
    /// Темп потока событий: пауза перед первым событием и между событиями (по умолчанию — сразу).
    pace: Option<(Duration, Duration)>,
}

impl MockAgent {
    fn new(id: &str, events: Vec<(u64, Event)>) -> Self {
        Self {
            id: id.into(),
            info: Ok(AgentInfo {
                version: "0.2.0".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                cpus: 2,
                busy: None,
            }),
            start: Ok(()),
            streams: Mutex::new(VecDeque::from([Ok(events)])),
            files: files_tar(20),
            files_busy: AtomicUsize::new(0),
            files_calls: AtomicUsize::new(0),
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            events_calls: Mutex::new(vec![]),
            pace: None,
        }
    }

    fn healthy(id: &str) -> Self {
        Self::new(
            id,
            vec![
                (
                    1,
                    Event::Phase(PhaseEvent {
                        t: 0,
                        index: 0,
                        pct: 0.0,
                    }),
                ),
                (2, tick(1, 10)),
                (3, tick(2, 10)),
                (4, done(20)),
            ],
        )
    }
}

#[async_trait]
impl AgentLink for MockAgent {
    fn id(&self) -> &str {
        &self.id
    }
    async fn info(&self) -> Result<AgentInfo, LinkError> {
        self.info.clone()
    }
    async fn start(
        &self,
        _req: &RunRequest,
        _suite: &str,
        _data: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), LinkError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.start.clone()
    }
    async fn events(
        &self,
        _run_id: &str,
        last_id: Option<u64>,
    ) -> Result<BoxStream<'static, (u64, Event)>, LinkError> {
        self.events_calls.lock().unwrap().push(last_id);
        let next = self.streams.lock().unwrap().pop_front();
        match next {
            Some(Ok(items)) => match self.pace {
                None => Ok(stream::iter(items).boxed()),
                Some((first, every)) => Ok(stream::iter(items.into_iter().enumerate())
                    .then(move |(i, item)| async move {
                        tokio::time::sleep(if i == 0 { first } else { every }).await;
                        item
                    })
                    .boxed()),
            },
            Some(Err(e)) => Err(e),
            None => Err(LinkError::new("AGENT_UNREACHABLE", "no more streams")),
        }
    }
    async fn stop(&self, _run_id: &str) -> Result<(), LinkError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn files(&self, _run_id: &str) -> Result<Vec<u8>, LinkError> {
        self.files_calls.fetch_add(1, Ordering::SeqCst);
        let busy = self.files_busy.load(Ordering::SeqCst);
        if busy > 0 {
            self.files_busy.store(busy - 1, Ordering::SeqCst);
            return Err(LinkError::new("AGENT_BUSY", "run is still in progress"));
        }
        Ok(self.files.clone())
    }
}

fn link(a: MockAgent) -> (Arc<MockAgent>, Arc<dyn AgentLink>) {
    let a = Arc::new(a);
    (a.clone(), a)
}

async fn drain(mut rx: mpsc::Receiver<ControllerEvent>) -> Vec<ControllerEvent> {
    let mut out = Vec::new();
    while let Some(e) = rx.recv().await {
        out.push(e);
    }
    out
}

#[tokio::test]
async fn two_agents_complete_and_summary_sums_n() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, la) = link(MockAgent::healthy("a"));
    let (b, lb) = link(MockAgent::healthy("b"));
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap();
    let events = events.await.unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    let op = result.summary.ops.iter().find(|o| o.id == OP).unwrap();
    assert_eq!(op.n, 40, "n is the sum of both agents' ticks");
    assert_eq!(op.target, Some(100.0));
    assert!(
        (op.p50 - 10.0).abs() < 1.0,
        "percentile from merged hist.bin: {}",
        op.p50
    );
    assert_eq!(
        (
            a.starts.load(Ordering::SeqCst),
            b.starts.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    assert_eq!(
        (
            a.stops.load(Ordering::SeqCst),
            b.stops.load(Ordering::SeqCst)
        ),
        (0, 0),
        "finished agents need no stop"
    );
    let dir = Path::new(&result.dir);
    assert!(dir.starts_with(tmp.path().join("runs")));
    for id in ["a", "b"] {
        assert!(dir.join("agents").join(id).join("summary.yaml").is_file());
        assert!(dir.join("agents").join(id).join("hist.bin").is_file());
    }
    assert!(dir.join("summary.yaml").is_file() && dir.join("run.yaml").is_file());
    let run_yaml = std::fs::read_to_string(dir.join("run.yaml")).unwrap();
    assert!(run_yaml.contains("status: completed"), "{run_yaml}");
    assert!(run_yaml.contains("url: http://a"), "{run_yaml}");
    assert!(matches!(
        events.first(),
        Some(ControllerEvent::Agent {
            state: AgentState::Ready,
            ..
        })
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ControllerEvent::Started { .. }))
    );
    let ticks: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::Tick(t) => Some(t.t),
            _ => None,
        })
        .collect();
    assert_eq!(ticks, vec![1, 2], "one merged second per t");
    let merged_n: u64 = events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::Tick(t) => Some(t.ops[0].n),
            _ => None,
        })
        .sum();
    assert_eq!(merged_n, 40);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ControllerEvent::Phase(_)))
            .count(),
        1,
        "phases come from the first agent only"
    );
    assert!(dir.join("phases.yaml").is_file());
    let metrics = std::fs::read_to_string(dir.join("metrics.jsonl")).unwrap();
    assert_eq!(metrics.lines().count(), 2, "{metrics}");
    for id in ["a", "b"] {
        assert!(events.iter().any(|e| matches!(
            e,
            ControllerEvent::Agent { id: x, state: AgentState::Done } if x == id
        )));
    }
    assert!(matches!(events.last(), Some(ControllerEvent::Done(r)) if r.run_id == result.run_id));
}

#[tokio::test]
async fn busy_agent_prevents_any_start() {
    let tmp = tempfile::tempdir().unwrap();
    let mut busy = MockAgent::healthy("b");
    busy.info = Ok(AgentInfo {
        version: "0.2.0".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cpus: 2,
        busy: Some("other-run".into()),
    });
    let (a, la) = link(MockAgent::healthy("a"));
    let (b, lb) = link(busy);
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.agent.as_deref(), e.names.as_slice()),
        ("AGENT_BUSY", Some("b"), &["other-run".to_string()][..])
    );
    assert_eq!(
        (
            a.starts.load(Ordering::SeqCst),
            b.starts.load(Ordering::SeqCst)
        ),
        (0, 0)
    );
    assert!(
        !tmp.path().join("runs").exists(),
        "no run folder is created"
    );

    let mut down = MockAgent::healthy("c");
    down.info = Err(LinkError::new("AGENT_UNREACHABLE", "connection refused"));
    let (_, lc) = link(down);
    let (_, la2) = link(MockAgent::healthy("a"));
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(
        plan(tmp.path()),
        vec![la2, lc],
        tx,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.agent.as_deref()),
        ("AGENT_UNREACHABLE", Some("c"))
    );

    let mut old = MockAgent::healthy("d");
    old.info = Ok(AgentInfo {
        version: "0.1.4".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cpus: 2,
        busy: None,
    });
    let (d, ld) = link(old);
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(plan(tmp.path()), vec![ld], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.agent.as_deref(), e.names.as_slice()),
        (
            "AGENT_VERSION",
            Some("d"),
            &["0.1.4".to_string(), "0.2.0".to_string()][..]
        )
    );
    assert_eq!(d.starts.load(Ordering::SeqCst), 0);

    let (_, l1) = link(MockAgent::healthy("a"));
    let (_, l2) = link(MockAgent::healthy("a"));
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(plan(tmp.path()), vec![l1, l2], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(e.code, "INVALID_INPUT", "repeated agent id");
    let (_, bad) = link(MockAgent::healthy("../x"));
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(plan(tmp.path()), vec![bad], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        e.code, "INVALID_INPUT",
        "agent id is the agents/<id> folder name"
    );
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(plan(tmp.path()), vec![], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(e.code, "AGENT_UNREACHABLE");
    assert!(!tmp.path().join("runs").exists());
}

#[tokio::test]
async fn failed_start_stops_the_agent_that_started() {
    let tmp = tempfile::tempdir().unwrap();
    let mut bad = MockAgent::healthy("b");
    bad.start =
        Err(LinkError::new("SECRET_MISSING", "missing").with_names(vec!["API_TOKEN".into()]));
    let (a, la) = link(MockAgent::healthy("a"));
    let (b, lb) = link(bad);
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let e = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap_err();
    let events = events.await.unwrap();
    assert_eq!(
        (e.code.as_str(), e.agent.as_deref(), e.names.as_slice()),
        ("SECRET_MISSING", Some("b"), &["API_TOKEN".to_string()][..])
    );
    assert_eq!(
        a.stops.load(Ordering::SeqCst),
        1,
        "the started agent gets stop"
    );
    assert_eq!(b.stops.load(Ordering::SeqCst), 0);
    assert!(!tmp.path().join("runs").exists());
    assert!(events.iter().any(|e| matches!(
        e,
        ControllerEvent::Agent { id, state: AgentState::Failed } if id == "b"
    )));
}

#[tokio::test]
async fn existing_run_folder_stops_the_started_agents() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("taken");
    std::fs::create_dir_all(&out).unwrap();
    let mut p = plan(tmp.path());
    p.out_dir = Some(out);
    let (a, la) = link(MockAgent::healthy("a"));
    let (b, lb) = link(MockAgent::healthy("b"));
    let (tx, _rx) = mpsc::channel(256);
    let e = Controller::run(p, vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(e.code, "IO");
    assert_eq!(
        (
            a.stops.load(Ordering::SeqCst),
            b.stops.load(Ordering::SeqCst)
        ),
        (1, 1),
        "no load is left running without a run folder"
    );
}

#[tokio::test(start_paused = true)]
async fn lost_agent_makes_the_run_incomplete_and_keeps_other_files() {
    let tmp = tempfile::tempdir().unwrap();
    let flaky = MockAgent::new("a", vec![(1, tick(1, 10))]);
    flaky
        .streams
        .lock()
        .unwrap()
        .push_back(Err(LinkError::new("AGENT_UNREACHABLE", "down")));
    let (a, la) = link(flaky);
    let (_, lb) = link(MockAgent::healthy("b"));
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap();
    let events = events.await.unwrap();
    assert_eq!(result.status, RunStatus::Incomplete);
    let calls = a.events_calls.lock().unwrap().clone();
    assert_eq!(calls[0], None);
    assert!(
        calls.len() >= 2 && calls[1..].iter().all(|c| *c == Some(1)),
        "retries carry Last-Event-ID: {calls:?}"
    );
    assert!(
        events.iter().any(
            |e| matches!(e, ControllerEvent::Agent { id, state: AgentState::Lost } if id == "a")
        )
    );
    assert!(
        a.stops.load(Ordering::SeqCst) >= 1,
        "stop is repeated for the lost agent when collecting files"
    );
    let dir = Path::new(&result.dir);
    assert!(dir.join("agents/b/summary.yaml").is_file());
    assert_eq!(result.summary.ops[0].n, 30, "ticks: 10 from a + 20 from b");
    let ticks: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::Tick(t) => Some(t.t),
            _ => None,
        })
        .collect();
    assert_eq!(ticks, vec![1, 2], "the series has no repeated t");
}

#[tokio::test]
async fn cancel_stops_both_agents_and_reports_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, la) = link(MockAgent::healthy("a"));
    let (b, lb) = link(MockAgent::healthy("b"));
    let (tx, rx) = mpsc::channel(256);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la, lb], tx, cancel)
        .await
        .unwrap();
    let _ = events.await.unwrap();
    assert_eq!(result.status, RunStatus::Stopped);
    assert_eq!(
        (
            a.stops.load(Ordering::SeqCst),
            b.stops.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    assert!(Path::new(&result.dir).join("summary.yaml").is_file());
}

#[tokio::test]
async fn gap_and_errors_are_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, la) = link(MockAgent::new(
        "a",
        vec![
            (1, tick(1, 5)),
            (2, error("a")),
            (3, Event::Gap),
            (4, done(5)),
        ],
    ));
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la], tx, CancellationToken::new())
        .await
        .unwrap();
    let events = events.await.unwrap();
    assert_eq!(result.status, RunStatus::Incomplete, "gap → incomplete");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ControllerEvent::Error(e) if e.code == "HTTP_5xx"))
    );
    let errors = std::fs::read_to_string(Path::new(&result.dir).join("errors.jsonl")).unwrap();
    assert!(errors.contains("HTTP_5xx"), "{errors}");
    let out = tmp.path().join("custom-out");
    let mut p = plan(tmp.path());
    p.out_dir = Some(out.clone());
    let (_, la) = link(MockAgent::healthy("a"));
    let (tx, _rx) = mpsc::channel(256);
    let r = Controller::run(p, vec![la], tx, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(Path::new(&r.dir), out.as_path());
}

#[tokio::test]
async fn error_samples_are_capped_across_agents() {
    let tmp = tempfile::tempdir().unwrap();
    let errors = |id: &str| {
        let mut ev: Vec<(u64, Event)> = (1..=4).map(|i| (i, error(id))).collect();
        ev.push((5, tick(1, 4)));
        ev.push((6, done(4)));
        MockAgent::new(id, ev)
    };
    let (_, la) = link(errors("a"));
    let (_, lb) = link(errors("b"));
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap();
    let events = events.await.unwrap();
    let forwarded = events
        .iter()
        .filter(|e| matches!(e, ControllerEvent::Error(_)))
        .count();
    let written = std::fs::read_to_string(Path::new(&result.dir).join("errors.jsonl"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(
        (forwarded, written),
        (5, 5),
        "five samples per code and operation for the whole run"
    );
}

/// tar с вложенным путём и выходом наверх: такие записи не распаковываются.
fn hostile_tar() -> Vec<u8> {
    let mut tar = tar_builder(20);
    let body = b"x";
    let mut nested = tar::Header::new_gnu();
    nested.set_size(1);
    nested.set_mode(0o644);
    tar.append_data(&mut nested, "sub/nested.txt", &body[..])
        .unwrap();
    // `append_data` не пускает `..` в путь — имя пишется в заголовок напрямую.
    let mut up = tar::Header::new_gnu();
    up.set_size(1);
    up.set_mode(0o644);
    let name = b"../escape.txt";
    up.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
    up.set_cksum();
    tar.append(&up, &body[..]).unwrap();
    tar.into_inner().unwrap()
}

#[tokio::test(start_paused = true)]
async fn busy_files_are_retried_and_only_flat_entries_are_unpacked() {
    let tmp = tempfile::tempdir().unwrap();
    let mut agent = MockAgent::healthy("a");
    agent.files = hostile_tar();
    agent.files_busy = AtomicUsize::new(2);
    let (a, la) = link(agent);
    let mut p = plan(tmp.path());
    let out = tmp.path().join("out/run");
    p.out_dir = Some(out.clone());
    let (tx, _rx) = mpsc::channel(256);
    let result = Controller::run(p, vec![la], tx, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(
        a.files_calls.load(Ordering::SeqCst),
        3,
        "two AGENT_BUSY then success"
    );
    let agent_dir = out.join("agents/a");
    assert!(agent_dir.join("hist.bin").is_file());
    assert!(
        !agent_dir.join("sub").exists(),
        "nested paths are not unpacked"
    );
    assert!(!out.join("agents/escape.txt").exists());
    assert!(!tmp.path().join("out/escape.txt").exists());
}

/// Агент, который отдаёт `seconds` тиков по `n` раз в секунду, начиная через `start` (имитация
/// позднего старта: вход в группу Kafka, загрузка среза), затем `done`.
fn paced(id: &str, start: Duration, seconds: u64, n: u64) -> MockAgent {
    let mut events: Vec<(u64, Event)> = (1..=seconds).map(|t| (t, tick(t, n))).collect();
    events.push((seconds + 1, done(seconds * n)));
    let mut agent = MockAgent::new(id, events);
    agent.pace = Some((start, Duration::from_secs(1)));
    agent
}

#[tokio::test(start_paused = true)]
async fn agents_started_seconds_apart_are_merged_into_every_second() {
    let tmp = tempfile::tempdir().unwrap();
    // b начинает на 5 с позже a: при старом сроке слияния 2 с весь ряд b уходил в «опоздавшие».
    let (_, la) = link(paced("a", Duration::from_secs(1), 10, 10));
    let (_, lb) = link(paced("b", Duration::from_secs(6), 10, 7));
    let (tx, rx) = mpsc::channel(256);
    let events = tokio::spawn(drain(rx));
    let result = Controller::run(plan(tmp.path()), vec![la, lb], tx, CancellationToken::new())
        .await
        .unwrap();
    let events = events.await.unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    let series: Vec<(u64, u64)> = events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::Tick(t) => Some((t.t, t.ops[0].n)),
            _ => None,
        })
        .collect();
    assert_eq!(
        series,
        (1..=10).map(|t| (t, 17)).collect::<Vec<_>>(),
        "every second holds both agents"
    );
    let metrics = std::fs::read_to_string(Path::new(&result.dir).join("metrics.jsonl")).unwrap();
    let rows: Vec<serde_json::Value> = metrics
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(rows.len(), 10, "{metrics}");
    assert!(rows.iter().all(|r| r["n"] == 17), "{metrics}");
    assert_eq!(result.summary.ops[0].n, 170, "summary and series agree");
}
