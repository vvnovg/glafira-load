//! Связь контроллера с агентом: трейт `AgentLink`, ошибки и встроенный агент `LocalAgent`
//! (решение плана 8): тот же путь «start → events → stop → files», что у HTTP-агента, но в процессе.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use glafira_load_model::suite::{self, Suite};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::agent_api::{
    AgentInfo, AgentSummary, ErrorSample, Event, PhaseEvent, RunRequest, RunStatus, Tick,
};
use crate::context::{DataTable, RunContext, parse_data};
use crate::executor::{Executors, StepExecutor};
use crate::metrics::{Recorder, SharedRecorder, summary};
use crate::run_files::{ERRORS_FILE, HIST_FILE, METRICS_FILE, SUMMARY_FILE, hist_log, tick_lines};
use crate::scheduler::{Scheduler, SchedulerConfig};
use crate::summary::sorted_summary;

/// Версия движка: сравнивается с версией контроллера по `major.minor` (Global Constraints).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Префикс переменных окружения с секретами стенда.
pub const SECRET_PREFIX: &str = "GLAFIRA_LOAD_SECRET_";
/// Сколько встроенный агент ждёт активные итерации после конца таймлайна или остановки.
pub const DRAIN: Duration = Duration::from_secs(5);

/// Ошибка связи с агентом; коды: `AGENT_UNREACHABLE`, `AGENT_BUSY`, `AGENT_VERSION`,
/// `SECRET_MISSING`, `SUITE_INVALID`, `UNAUTHORIZED`, `NOT_FOUND`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct LinkError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub names: Vec<String>,
}

impl LinkError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            names: Vec::new(),
        }
    }

    pub fn with_names(mut self, names: Vec<String>) -> Self {
        self.names = names;
        self
    }
}

/// Ошибка контроллера и локального прогона; `agent` — виновный агент, если есть.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct EngineError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl EngineError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            names: Vec::new(),
            agent: None,
        }
    }

    pub fn with_names(mut self, names: Vec<String>) -> Self {
        self.names = names;
        self
    }

    pub fn with_agent(mut self, agent: &str) -> Self {
        self.agent = Some(agent.into());
        self
    }

    /// Ошибка файловой системы: код `IO`.
    pub fn io(message: impl Into<String>) -> Self {
        Self::new("IO", message)
    }

    /// Ошибка связи с конкретным агентом: код, текст и имена — из `LinkError`, `agent` — виновник.
    pub fn from_link(e: LinkError, agent: &str) -> Self {
        Self {
            code: e.code,
            message: e.message,
            names: e.names,
            agent: Some(agent.into()),
        }
    }
}

impl From<LinkError> for EngineError {
    fn from(e: LinkError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            names: e.names,
            agent: None,
        }
    }
}

impl From<io::Error> for EngineError {
    fn from(e: io::Error) -> Self {
        Self::new("IO", e.to_string())
    }
}

/// Итог прогона; поля — snake_case, как `run.yaml` (фронтенд Task 14 читает `run_id`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunResult {
    pub run_id: String,
    /// Абсолютный путь папки прогона.
    pub dir: String,
    pub status: RunStatus,
    pub summary: AgentSummary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Ready,
    Running,
    Lost,
    Done,
    Failed,
}

/// События контроллера для CLI и окна прогона: тег `type`, поля без переименования (`run_id`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerEvent {
    Started { run_id: String, dir: String },
    Tick(Tick),
    Phase(PhaseEvent),
    Agent { id: String, state: AgentState },
    Error(ErrorSample),
    Done(RunResult),
}

#[async_trait]
pub trait AgentLink: Send + Sync {
    fn id(&self) -> &str;
    async fn info(&self) -> Result<AgentInfo, LinkError>;
    /// `data`: имя набора из `suite.data` → байты файла (для `slice` — строки этого агента).
    async fn start(
        &self,
        req: &RunRequest,
        suite_text: &str,
        data: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), LinkError>;
    async fn events(
        &self,
        run_id: &str,
        last_id: Option<u64>,
    ) -> Result<BoxStream<'static, (u64, Event)>, LinkError>;
    async fn stop(&self, run_id: &str) -> Result<(), LinkError>;
    /// tar с `metrics.jsonl`, `errors.jsonl`, `summary.yaml`, `hist.bin` (и `agent.log` у HTTP-агента).
    async fn files(&self, run_id: &str) -> Result<Vec<u8>, LinkError>;
}

/// `major.minor` версий совпадают.
pub fn same_minor(a: &str, b: &str) -> bool {
    let head = |v: &str| v.split('.').take(2).map(str::to_string).collect::<Vec<_>>();
    head(a) == head(b)
}

/// Секреты по именам через `get(GLAFIRA_LOAD_SECRET_<ИМЯ>)`; ошибка — отсортированные имена отсутствующих.
pub fn secrets_from(
    names: &[String],
    get: impl Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, String>, Vec<String>> {
    let mut out = BTreeMap::new();
    let mut missing = Vec::new();
    for name in names {
        match get(&format!("{SECRET_PREFIX}{name}")) {
            Some(v) => {
                out.insert(name.clone(), v);
            }
            None => missing.push(name.clone()),
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        missing.sort();
        Err(missing)
    }
}

pub fn secrets_from_env(names: &[String]) -> Result<BTreeMap<String, String>, Vec<String>> {
    secrets_from(names, |k| std::env::var(k).ok())
}

/// Таблицы наборов сьюта из байтов бандла; формат — по расширению `data[*].path`.
pub fn data_tables(
    suite: &Suite,
    data: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, DataTable>, String> {
    let mut out = BTreeMap::new();
    for (name, set) in &suite.data {
        let bytes = data
            .get(name)
            .ok_or_else(|| format!("data set `{name}` was not provided"))?;
        let table = parse_data(&set.path, bytes)
            .map_err(|e| format!("data set `{name}` ({}): {e}", set.path))?;
        out.insert(name.clone(), table);
    }
    Ok(out)
}

/// tar в памяти из именованных файлов.
pub fn agent_tar(files: &[(&str, &[u8])]) -> io::Result<Vec<u8>> {
    let mtime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(mtime);
        builder.append_data(&mut header, name, *bytes)?;
    }
    builder.into_inner()
}

/// Фабрика исполнителей прогона: HTTP всегда, Kafka — если в сьюте есть шаги Kafka.
/// Контекст даёт значения секретов (маскирование в примерах ошибок).
pub type ExecutorFactory = Arc<
    dyn Fn(RunRequest, Suite, Arc<RunContext>) -> BoxFuture<'static, Result<Executors, EngineError>>
        + Send
        + Sync,
>;

struct ActiveRun {
    run_id: String,
    cancel: CancellationToken,
    events: Option<mpsc::UnboundedReceiver<(u64, Event)>>,
    task: Option<JoinHandle<Vec<u8>>>,
}

/// Встроенный агент: исполняет прогон в текущем процессе.
pub struct LocalAgent {
    id: String,
    factory: ExecutorFactory,
    /// Секреты `имя → значение` вместо окружения процесса (агент Task 10 собирает их сам).
    secrets: Option<BTreeMap<String, String>>,
    active: Mutex<Option<ActiveRun>>,
}

/// Файлы агента, собираемые из потока событий.
#[derive(Default)]
struct AgentFiles {
    metrics: Vec<u8>,
    errors: Vec<u8>,
    summary: Option<AgentSummary>,
    hist: Vec<u8>,
    next_id: u64,
}

impl AgentFiles {
    fn tar(&self) -> Vec<u8> {
        let summary = self
            .summary
            .as_ref()
            .map(|s| glafira_load_model::to_yaml(&sorted_summary(s)))
            .unwrap_or_default();
        agent_tar(&[
            (METRICS_FILE, self.metrics.as_slice()),
            (ERRORS_FILE, self.errors.as_slice()),
            (SUMMARY_FILE, summary.as_bytes()),
            (HIST_FILE, self.hist.as_slice()),
        ])
        .unwrap_or_default()
    }
}

/// Нумерует события, копит `metrics.jsonl`/`errors.jsonl` агента и отдаёт их наружу.
async fn forward_events(
    mut rx: mpsc::Receiver<Event>,
    out: mpsc::UnboundedSender<(u64, Event)>,
) -> AgentFiles {
    let mut files = AgentFiles {
        next_id: 1,
        ..Default::default()
    };
    while let Some(ev) = rx.recv().await {
        match &ev {
            Event::Tick(t) => {
                for line in tick_lines(t) {
                    if serde_json::to_writer(&mut files.metrics, &line).is_ok() {
                        files.metrics.push(b'\n');
                    }
                }
            }
            Event::Error(e) => {
                if serde_json::to_writer(&mut files.errors, e).is_ok() {
                    files.errors.push(b'\n');
                }
            }
            Event::Done(s) => files.summary = Some(s.clone()),
            Event::Phase(_) | Event::Gap => {}
        }
        let id = files.next_id;
        files.next_id += 1;
        // Получатель мог отключиться: события дальше не нужны, но планировщик должен дожить до конца.
        let _ = out.send((id, ev));
    }
    files
}

async fn run_task(
    req: RunRequest,
    suite: Arc<Suite>,
    run_ctx: Arc<RunContext>,
    executors: Arc<Executors>,
    out: mpsc::UnboundedSender<(u64, Event)>,
    cancel: CancellationToken,
) -> Vec<u8> {
    let recorder: SharedRecorder = Arc::new(Mutex::new(Recorder::new()));
    let cfg = SchedulerConfig {
        targets: req.targets.clone(),
        timeline: req.timeline.clone(),
        total_minutes: req.total_minutes,
        arrival: req.arrival,
        max_in_flight: req.max_in_flight,
        mode: req.mode,
        drain: DRAIN,
    };
    let (tx, rx) = mpsc::channel::<Event>(1024);
    let forward = tokio::spawn(forward_events(rx, out.clone()));
    let executor: Arc<dyn StepExecutor> = executors;
    let status = Scheduler::run(suite, cfg, run_ctx, executor, recorder.clone(), tx, cancel).await;
    // Отправитель планировщика сброшен: пересылка доходит до конца.
    let mut files = forward.await.unwrap_or_default();
    let totals = recorder.lock().expect("recorder").finish();
    files.hist = hist_log(&totals).unwrap_or_default();
    if files.summary.is_none() {
        let s = summary(&totals, &req.targets, status);
        files.summary = Some(s.clone());
        let id = files.next_id;
        files.next_id += 1;
        let _ = out.send((id, Event::Done(s)));
    }
    files.tar()
}

impl LocalAgent {
    pub fn new(id: &str, factory: ExecutorFactory) -> Self {
        Self {
            id: id.into(),
            factory,
            secrets: None,
            active: Mutex::new(None),
        }
    }

    /// Секреты из карты `имя → значение` (без префикса) вместо переменных окружения процесса.
    pub fn with_secrets(mut self, secrets: BTreeMap<String, String>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    fn secrets_for(&self, names: &[String]) -> Result<BTreeMap<String, String>, Vec<String>> {
        match &self.secrets {
            Some(map) => secrets_from(names, |key| {
                key.strip_prefix(SECRET_PREFIX)
                    .and_then(|name| map.get(name).cloned())
            }),
            None => secrets_from_env(names),
        }
    }

    /// `run_id` идущего прогона.
    pub fn is_busy(&self) -> Option<String> {
        let guard = self.active.lock().expect("active run");
        guard
            .as_ref()
            .filter(|r| r.task.as_ref().is_some_and(|t| !t.is_finished()))
            .map(|r| r.run_id.clone())
    }
}

#[async_trait]
impl AgentLink for LocalAgent {
    fn id(&self) -> &str {
        &self.id
    }

    async fn info(&self) -> Result<AgentInfo, LinkError> {
        Ok(AgentInfo {
            version: VERSION.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            cpus: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            busy: self.is_busy(),
        })
    }

    async fn start(
        &self,
        req: &RunRequest,
        suite_text: &str,
        data: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), LinkError> {
        if let Some(run_id) = self.is_busy() {
            return Err(
                LinkError::new("AGENT_BUSY", format!("agent is running {run_id}"))
                    .with_names(vec![run_id]),
            );
        }
        if !same_minor(&req.controller_version, VERSION) {
            return Err(LinkError::new(
                "AGENT_VERSION",
                format!(
                    "controller {} and agent {VERSION} differ",
                    req.controller_version
                ),
            ));
        }
        if req.suite_format != suite::FORMAT {
            return Err(LinkError::new(
                "AGENT_VERSION",
                format!(
                    "suite format {} is not supported, agent expects {}",
                    req.suite_format,
                    suite::FORMAT
                ),
            ));
        }
        let parsed = suite::parse(suite_text).map_err(|e| {
            let at = e.line.map(|l| format!("line {l}: ")).unwrap_or_default();
            LinkError::new("SUITE_INVALID", format!("{at}{}", e.message))
        })?;
        let secrets = self.secrets_for(&req.secrets).map_err(|names| {
            LinkError::new(
                "SECRET_MISSING",
                format!("missing {SECRET_PREFIX}* variables"),
            )
            .with_names(names)
        })?;
        let tables = data_tables(&parsed, data).map_err(|m| LinkError::new("SUITE_INVALID", m))?;
        let run_ctx = Arc::new(RunContext {
            run_id: req.run_id.clone(),
            agent_id: req.agent_id.clone(),
            env: req.env.clone(),
            secrets,
            data: tables,
        });
        let executors = (self.factory)(req.clone(), parsed.clone(), run_ctx.clone())
            .await
            .map_err(|e| LinkError {
                code: e.code,
                message: e.message,
                names: e.names,
            })?;
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_task(
            req.clone(),
            Arc::new(parsed),
            run_ctx,
            Arc::new(executors),
            tx,
            cancel.clone(),
        ));
        *self.active.lock().expect("active run") = Some(ActiveRun {
            run_id: req.run_id.clone(),
            cancel,
            events: Some(rx),
            task: Some(task),
        });
        Ok(())
    }

    /// Поток читается один раз; `last_id` не нужен — события в процессе не теряются.
    async fn events(
        &self,
        run_id: &str,
        _last_id: Option<u64>,
    ) -> Result<BoxStream<'static, (u64, Event)>, LinkError> {
        let mut guard = self.active.lock().expect("active run");
        let run = guard
            .as_mut()
            .filter(|r| r.run_id == run_id)
            .ok_or_else(|| {
                LinkError::new("NOT_FOUND", format!("run {run_id} is not on this agent"))
            })?;
        let rx = run
            .events
            .take()
            .ok_or_else(|| LinkError::new("RUN_BUSY", "events of a local run are read once"))?;
        Ok(UnboundedReceiverStream::new(rx).boxed())
    }

    async fn stop(&self, run_id: &str) -> Result<(), LinkError> {
        let guard = self.active.lock().expect("active run");
        let run = guard
            .as_ref()
            .filter(|r| r.run_id == run_id)
            .ok_or_else(|| {
                LinkError::new("NOT_FOUND", format!("run {run_id} is not on this agent"))
            })?;
        run.cancel.cancel();
        Ok(())
    }

    /// Ждёт конца прогона и отдаёт tar; после этого агент свободен.
    async fn files(&self, run_id: &str) -> Result<Vec<u8>, LinkError> {
        let task = {
            let mut guard = self.active.lock().expect("active run");
            let run = guard
                .as_mut()
                .filter(|r| r.run_id == run_id)
                .ok_or_else(|| {
                    LinkError::new("NOT_FOUND", format!("run {run_id} is not on this agent"))
                })?;
            run.task.take().ok_or_else(|| {
                LinkError::new(
                    "NOT_FOUND",
                    format!("files of run {run_id} were already taken"),
                )
            })?
        };
        let bytes = task.await.map_err(|e| {
            LinkError::new("AGENT_UNREACHABLE", format!("local run task failed: {e}"))
        })?;
        let mut guard = self.active.lock().expect("active run");
        if guard.as_ref().is_some_and(|r| r.run_id == run_id) {
            *guard = None;
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_api::{EnvAddresses, Mode};

    #[test]
    fn same_minor_compares_major_and_minor_only() {
        assert!(same_minor("0.2.0", "0.2.7"));
        assert!(same_minor("1.4.0-beta.1", "1.4.3"));
        assert!(!same_minor("0.2.0", "0.3.0"));
        assert!(!same_minor("1.2.0", "0.2.0"));
        assert!(same_minor(VERSION, VERSION));
    }

    #[test]
    fn secrets_come_from_prefixed_variables_and_missing_are_listed() {
        let vars = BTreeMap::from([
            (
                "GLAFIRA_LOAD_SECRET_API_TOKEN".to_string(),
                "t0k".to_string(),
            ),
            ("API_TOKEN".to_string(), "wrong".to_string()),
        ]);
        let get = |k: &str| vars.get(k).cloned();
        let ok = secrets_from(&["API_TOKEN".to_string()], get).unwrap();
        assert_eq!(ok.get("API_TOKEN").map(String::as_str), Some("t0k"));
        let missing = secrets_from(
            &[
                "API_TOKEN".to_string(),
                "DB_PASS".to_string(),
                "AWS".to_string(),
            ],
            get,
        )
        .unwrap_err();
        assert_eq!(
            missing,
            vec!["AWS".to_string(), "DB_PASS".to_string()],
            "отсортированы"
        );
        assert!(secrets_from(&[], get).unwrap().is_empty());
    }

    #[test]
    fn data_tables_follow_suite_paths() {
        let suite = glafira_load_model::suite::parse(
            "format: 1\nname: s\ndata:\n  users: data/users.csv\n  skus: { path: data/skus.jsonl, mode: slice }\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:o:GET /\" }\n",
        )
        .unwrap();
        let data = BTreeMap::from([
            ("users".to_string(), b"id\n1\n2\n".to_vec()),
            ("skus".to_string(), b"{\"sku\":\"A\"}\n".to_vec()),
        ]);
        let tables = data_tables(&suite, &data).unwrap();
        assert_eq!(tables["users"].rows.len(), 2);
        assert_eq!(tables["skus"].columns, ["sku"]);
        let err = data_tables(&suite, &BTreeMap::new()).unwrap_err();
        assert!(err.contains("`skus`") || err.contains("`users`"), "{err}");
    }

    #[test]
    fn agent_tar_packs_named_files() {
        let bytes = agent_tar(&[
            ("metrics.jsonl", b"{}\n".as_slice()),
            ("summary.yaml", b"status: completed\n".as_slice()),
        ])
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tar::Archive::new(&bytes[..]).unpack(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("metrics.jsonl")).unwrap(),
            "{}\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("summary.yaml")).unwrap(),
            "status: completed\n"
        );
    }

    #[tokio::test]
    async fn local_agent_rejects_bad_version_format_suite_and_secrets() {
        let factory: ExecutorFactory =
            Arc::new(|_req: RunRequest, _suite: Suite, _ctx: Arc<RunContext>| {
                Box::pin(async {
                    Err::<Executors, EngineError>(EngineError::new(
                        "INVALID_INPUT",
                        "factory must not be called",
                    ))
                })
            });
        let agent = LocalAgent::new("local", factory);
        assert_eq!(agent.id(), "local");
        let info = agent.info().await.unwrap();
        assert_eq!((info.version.as_str(), info.busy), (VERSION, None));
        assert!(info.cpus >= 1 && !info.os.is_empty() && !info.arch.is_empty());

        let suite = "format: 1\nname: s\nsecrets: [NOPE_SECRET]\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:o:GET /\" }\n";
        let mut req = RunRequest {
            run_id: "r1".into(),
            agent_id: "local".into(),
            suite_name: "s".into(),
            controller_version: VERSION.into(),
            suite_format: glafira_load_model::suite::FORMAT,
            env: EnvAddresses {
                http: None,
                kafka: None,
                schema_registry: None,
            },
            secrets: vec![],
            targets: BTreeMap::new(),
            timeline: vec![],
            total_minutes: 0.0,
            max_in_flight: 10,
            arrival: glafira_load_model::suite::Arrival::Constant,
            mode: Mode::Smoke,
            kafka: BTreeMap::new(),
            response_schemas: BTreeMap::new(),
            avro_schemas: BTreeMap::new(),
        };
        req.controller_version = "9.9.0".into();
        assert_eq!(
            agent
                .start(&req, suite, &BTreeMap::new())
                .await
                .unwrap_err()
                .code,
            "AGENT_VERSION"
        );
        req.controller_version = VERSION.into();
        req.suite_format = 99;
        assert_eq!(
            agent
                .start(&req, suite, &BTreeMap::new())
                .await
                .unwrap_err()
                .code,
            "AGENT_VERSION"
        );
        req.suite_format = glafira_load_model::suite::FORMAT;
        let bad = agent
            .start(&req, "format: 1\nname: s\nscenarios: [", &BTreeMap::new())
            .await
            .unwrap_err();
        assert_eq!(bad.code, "SUITE_INVALID");
        req.secrets = vec!["NOPE_SECRET".into()];
        let missing = agent
            .start(&req, suite, &BTreeMap::new())
            .await
            .unwrap_err();
        assert_eq!(
            (missing.code.as_str(), missing.names),
            ("SECRET_MISSING", vec!["NOPE_SECRET".to_string()])
        );
        req.secrets = vec![];
        let factory_failed = agent
            .start(&req, suite, &BTreeMap::new())
            .await
            .unwrap_err();
        assert_eq!(factory_failed.code, "INVALID_INPUT");
        assert!(agent.events("r1", None).await.is_err(), "прогона нет");
        assert!(agent.stop("r1").await.is_err());
        assert!(agent.files("r1").await.is_err());

        // Секреты из карты `with_secrets` (агент Task 10): окружение процесса не трогается.
        let factory: ExecutorFactory =
            Arc::new(|_req: RunRequest, _suite: Suite, ctx: Arc<RunContext>| {
                Box::pin(async move {
                    Err::<Executors, EngineError>(EngineError::new(
                        "SEEN",
                        ctx.secrets.get("NOPE_SECRET").cloned().unwrap_or_default(),
                    ))
                })
            });
        let with_map = LocalAgent::new("local", factory).with_secrets(BTreeMap::from([(
            "NOPE_SECRET".to_string(),
            "v4lue".to_string(),
        )]));
        req.secrets = vec!["NOPE_SECRET".into()];
        let seen = with_map
            .start(&req, suite, &BTreeMap::new())
            .await
            .unwrap_err();
        assert_eq!(
            (seen.code.as_str(), seen.message.as_str()),
            ("SEEN", "v4lue")
        );
    }

    /// Исполнитель Kafka, у которого падает опрос лага: планировщик вернёт `Failed`.
    struct PanickingLag;

    #[async_trait]
    impl StepExecutor for PanickingLag {
        async fn execute(
            &self,
            _step: &glafira_load_model::suite::Step,
            _ctx: &mut crate::context::IterationCtx<'_>,
        ) -> Result<crate::executor::StepOutcome, crate::errors::StepError> {
            Ok(crate::executor::StepOutcome::default())
        }

        async fn lag(&self) -> BTreeMap<String, i64> {
            panic!("lag probe failed")
        }
    }

    #[tokio::test]
    async fn failed_scheduler_ends_the_run_with_failed_done() {
        let factory: ExecutorFactory =
            Arc::new(|req: RunRequest, suite: Suite, ctx: Arc<RunContext>| {
                Box::pin(async move {
                    let http = crate::http::HttpExecutor::new(
                        req.env.http.as_deref().unwrap_or(""),
                        &suite.defaults.http,
                        &ctx.secret_values(),
                    )
                    .map_err(|m| EngineError::new("INVALID_INPUT", m))?;
                    Ok(Executors {
                        http,
                        kafka: Some(Box::new(PanickingLag) as Box<dyn StepExecutor>),
                    })
                })
            });
        let agent = LocalAgent::new("local", factory);
        let suite = "format: 1\nname: s\nscenarios:\n  - id: a\n    steps:\n      - kafka_produce: { operation: \"kafka_produce:t\" }\n";
        let req = RunRequest {
            run_id: "r1".into(),
            agent_id: "local".into(),
            suite_name: "s".into(),
            controller_version: VERSION.into(),
            suite_format: glafira_load_model::suite::FORMAT,
            env: EnvAddresses {
                http: Some("http://127.0.0.1:1".into()),
                kafka: None,
                schema_registry: None,
            },
            secrets: vec![],
            targets: BTreeMap::from([("kafka_produce:t".to_string(), 5.0)]),
            timeline: vec![
                glafira_load_model::profile::Point {
                    minute: 0.0,
                    pct: 100.0,
                },
                glafira_load_model::profile::Point {
                    minute: 0.03,
                    pct: 100.0,
                },
            ],
            total_minutes: 0.03,
            max_in_flight: 10,
            arrival: glafira_load_model::suite::Arrival::Constant,
            mode: Mode::Run,
            kafka: BTreeMap::new(),
            response_schemas: BTreeMap::new(),
            avro_schemas: BTreeMap::new(),
        };
        agent.start(&req, suite, &BTreeMap::new()).await.unwrap();
        assert_eq!(agent.is_busy().as_deref(), Some("r1"));
        let mut events = agent.events("r1", None).await.unwrap();
        let mut done = None;
        while let Some((_, ev)) = events.next().await {
            if let Event::Done(s) = ev {
                done = Some(s);
            }
        }
        assert_eq!(done.expect("done event").status, RunStatus::Failed);
        let tar = agent.files("r1").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tar::Archive::new(&tar[..]).unpack(tmp.path()).unwrap();
        assert!(
            std::fs::read_to_string(tmp.path().join(SUMMARY_FILE))
                .unwrap()
                .contains("status: failed")
        );
        assert_eq!(agent.is_busy(), None, "после files агент свободен");
    }

    #[test]
    fn engine_error_helpers_keep_code_names_and_agent() {
        let io = EngineError::io("disk full");
        assert_eq!(
            (io.code.as_str(), io.message.as_str(), io.agent),
            ("IO", "disk full", None)
        );
        let link = LinkError::new("SECRET_MISSING", "missing").with_names(vec!["API_TOKEN".into()]);
        let e = EngineError::from_link(link, "gen-1");
        assert_eq!(
            (e.code.as_str(), e.names.as_slice(), e.agent.as_deref()),
            (
                "SECRET_MISSING",
                &["API_TOKEN".to_string()][..],
                Some("gen-1")
            )
        );
        let result = RunResult {
            run_id: "r1".into(),
            dir: "/tmp/r1".into(),
            status: RunStatus::Completed,
            summary: AgentSummary {
                status: RunStatus::Completed,
                ops: vec![],
                scenarios: vec![],
                seconds: 1,
            },
        };
        let json = serde_json::to_string(&ControllerEvent::Done(result)).unwrap();
        assert!(
            json.contains("\"type\":\"done\"") && json.contains("\"run_id\":\"r1\""),
            "{json}"
        );
        let started = serde_json::to_string(&ControllerEvent::Started {
            run_id: "r1".into(),
            dir: "/tmp/r1".into(),
        })
        .unwrap();
        assert_eq!(
            started,
            r#"{"type":"started","run_id":"r1","dir":"/tmp/r1"}"#
        );
    }
}
