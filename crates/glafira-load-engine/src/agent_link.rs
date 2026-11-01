//! Связь контроллера с агентом: трейт `AgentLink`, ошибки и встроенный агент `LocalAgent`
//! (решение плана 8): тот же путь «start → events → stop → files», что у HTTP-агента, но в процессе.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use glafira_load_model::suite::{self, Suite};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::agent_api::{
    AgentInfo, AgentSummary, ErrorSample, Event, Mode, PhaseEvent, RunRequest, RunStatus, Tick,
};
use crate::context::{DataTable, RunContext, parse_data};
use crate::errors::SampleCap;
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
/// `SECRET_MISSING`, `SUITE_INVALID`, `UNAUTHORIZED`, `NOT_FOUND`, `IO` (файлы прогона не собраны).
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

/// Прогон на встроенном агенте. Сброс (`files` забрал файлы, агент уничтожен) отменяет токен
/// прогона через `DropGuard`: брошенный прогон не продолжает нагрузку без слушателя.
struct ActiveRun {
    run_id: String,
    cancel: CancellationToken,
    _cancel_on_drop: DropGuard,
    events: Option<mpsc::UnboundedReceiver<(u64, Event)>>,
    /// Сигнал конца задачи прогона: `true` после записи итога в слот. Отправитель закрыт без
    /// `true` — задача прервана (остановка рантайма), итога не будет.
    done: watch::Receiver<bool>,
}

/// Итог задачи прогона: tar файлов агента или ошибка `IO`/`AGENT_UNREACHABLE`.
type RunOutcome = Result<Vec<u8>, LinkError>;

/// Состояние слота встроенного агента; агент свободен только в `Idle`.
enum Slot {
    Idle,
    /// `start` занял слот и ждёт фабрику исполнителей; `stop` отменяет будущий прогон.
    Starting {
        run_id: String,
        cancel: CancellationToken,
    },
    Running(ActiveRun),
    /// Задача прогона завершилась и записала итог, но файлы ещё не забраны (`files`).
    Finished {
        run: ActiveRun,
        outcome: RunOutcome,
    },
}

impl Slot {
    fn run_id(&self) -> Option<&str> {
        match self {
            Slot::Idle => None,
            Slot::Starting { run_id, .. } => Some(run_id),
            Slot::Running(r) | Slot::Finished { run: r, .. } => Some(&r.run_id),
        }
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut ActiveRun> {
        match self {
            Slot::Running(r) | Slot::Finished { run: r, .. } if r.run_id == run_id => Some(r),
            _ => None,
        }
    }
}

type SharedSlot = Arc<Mutex<Slot>>;

fn not_found(run_id: &str) -> LinkError {
    LinkError::new("NOT_FOUND", format!("run {run_id} is not on this agent"))
}

/// Бронь слота на время ожидания фабрики: если `start` вернул ошибку или его будущее сброшено,
/// слот снова свободен.
struct Reservation<'a> {
    slot: &'a Mutex<Slot>,
    run_id: String,
    armed: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut guard = self.slot.lock().expect("agent slot");
        if matches!(&*guard, Slot::Starting { run_id, .. } if *run_id == self.run_id) {
            *guard = Slot::Idle;
        }
    }
}

/// Встроенный агент: исполняет прогон в текущем процессе.
pub struct LocalAgent {
    id: String,
    factory: ExecutorFactory,
    /// Секреты `имя → значение` вместо окружения процесса (агент Task 10 собирает их сам).
    secrets: Option<BTreeMap<String, String>>,
    slot: SharedSlot,
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
    fn tar(&self) -> Result<Vec<u8>, String> {
        let summary = self
            .summary
            .as_ref()
            .map(|s| glafira_load_model::to_yaml(&sorted_summary(s)))
            .ok_or("the run has no summary")?;
        agent_tar(&[
            (METRICS_FILE, self.metrics.as_slice()),
            (ERRORS_FILE, self.errors.as_slice()),
            (SUMMARY_FILE, summary.as_bytes()),
            (HIST_FILE, self.hist.as_slice()),
        ])
        .map_err(|e| format!("agent files tar: {e}"))
    }
}

/// Нумерует события, копит `metrics.jsonl`/`errors.jsonl` агента и отдаёт их наружу.
/// `cap` — предел примеров ошибок (режим `run`): лишние примеры не пишутся в `errors.jsonl`
/// и не уходят в поток; в смоке (`None`) отчёту нужен пример каждого шага.
async fn forward_events(
    mut rx: mpsc::Receiver<Event>,
    out: mpsc::UnboundedSender<(u64, Event)>,
    mut cap: Option<SampleCap>,
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
                if let Some(cap) = cap.as_mut()
                    && !cap.allow(&e.code, &e.operation)
                {
                    continue;
                }
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

/// Прогон до конца и tar файлов агента; ошибка — файлы собрать не удалось (`files` отдаст её).
async fn run_task(
    req: RunRequest,
    suite: Arc<Suite>,
    run_ctx: Arc<RunContext>,
    executors: Arc<Executors>,
    out: mpsc::UnboundedSender<(u64, Event)>,
    cancel: CancellationToken,
) -> Result<Vec<u8>, String> {
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
    let cap = (req.mode == Mode::Run).then(SampleCap::new);
    let forward = tokio::spawn(forward_events(rx, out.clone(), cap));
    let executor: Arc<dyn StepExecutor> = executors;
    let status = Scheduler::run(suite, cfg, run_ctx, executor, recorder.clone(), tx, cancel).await;
    // Отправитель планировщика сброшен: пересылка доходит до конца. Без её файлов прогон не
    // собрать: поток закончится без `done`, и контроллер отметит прогон как `failed`.
    let mut files = forward
        .await
        .map_err(|e| format!("event forwarding of the local run failed: {e}"))?;
    let totals = recorder.lock().expect("recorder").finish();
    if files.summary.is_none() {
        let s = summary(&totals, &req.targets, status);
        files.summary = Some(s.clone());
        let id = files.next_id;
        files.next_id += 1;
        let _ = out.send((id, Event::Done(s)));
    }
    files.hist = hist_log(&totals)?;
    files.tar()
}

impl LocalAgent {
    pub fn new(id: &str, factory: ExecutorFactory) -> Self {
        Self {
            id: id.into(),
            factory,
            secrets: None,
            slot: Arc::new(Mutex::new(Slot::Idle)),
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

    /// `run_id` прогона, занимающего агент: от брони в `start` до того, как `files` отдал файлы.
    pub fn is_busy(&self) -> Option<String> {
        self.slot
            .lock()
            .expect("agent slot")
            .run_id()
            .map(str::to_string)
    }

    /// Проверки запроса до брони слота; ошибка — код `LinkError`.
    fn prepare(
        &self,
        req: &RunRequest,
        suite_text: &str,
        data: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(Suite, RunContext), LinkError> {
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
        let run_ctx = RunContext {
            run_id: req.run_id.clone(),
            agent_id: req.agent_id.clone(),
            env: req.env.clone(),
            secrets,
            data: tables,
        };
        Ok((parsed, run_ctx))
    }
}

/// Текст паники задачи прогона.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

/// Задача прогона: по выходе кладёт итог в слот (`Running` → `Finished`, если агент ещё жив)
/// и подаёт сигнал `done`. Итог живёт в слоте, а не в `JoinHandle`: сброшенное ожидание
/// `files` ничего не забирает, и следующий `files` получит тот же итог.
async fn run_and_mark(
    slot: Weak<Mutex<Slot>>,
    run_id: String,
    done: watch::Sender<bool>,
    run: impl Future<Output = Result<Vec<u8>, String>>,
) {
    let outcome = match std::panic::AssertUnwindSafe(run).catch_unwind().await {
        Ok(result) => result.map_err(|m| LinkError::new("IO", m)),
        Err(panic) => Err(LinkError::new(
            "AGENT_UNREACHABLE",
            format!("local run task failed: {}", panic_message(&*panic)),
        )),
    };
    if let Some(slot) = slot.upgrade() {
        let mut guard = slot.lock().expect("agent slot");
        if matches!(&*guard, Slot::Running(r) if r.run_id == run_id)
            && let Slot::Running(run) = std::mem::replace(&mut *guard, Slot::Idle)
        {
            *guard = Slot::Finished { run, outcome };
        }
    }
    // Сигнал после записи итога: проснувшийся `files` уже видит `Finished`.
    done.send_replace(true);
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
        let busy = |run_id: &str| {
            LinkError::new("AGENT_BUSY", format!("agent is running {run_id}"))
                .with_names(vec![run_id.to_string()])
        };
        if let Some(run_id) = self.is_busy() {
            return Err(busy(&run_id));
        }
        let (parsed, run_ctx) = self.prepare(req, suite_text, data)?;
        let run_ctx = Arc::new(run_ctx);
        // Бронь под замком до ожидания фабрики (подключение к Kafka идёт до 10 с): второй
        // `start` в это время получает `AGENT_BUSY`, а не затирает первый прогон.
        let cancel = {
            let mut guard = self.slot.lock().expect("agent slot");
            if let Some(run_id) = guard.run_id() {
                return Err(busy(run_id));
            }
            let cancel = CancellationToken::new();
            *guard = Slot::Starting {
                run_id: req.run_id.clone(),
                cancel: cancel.clone(),
            };
            cancel
        };
        let mut reservation = Reservation {
            slot: &self.slot,
            run_id: req.run_id.clone(),
            armed: true,
        };
        let executors = (self.factory)(req.clone(), parsed.clone(), run_ctx.clone())
            .await
            .map_err(|e| LinkError {
                code: e.code,
                message: e.message,
                names: e.names,
            })?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = watch::channel(false);
        // Задача запускается под замком: её отметка `Finished` не обгонит запись `Running`.
        // `JoinHandle` не нужен: итог задача кладёт в слот сама.
        let mut guard = self.slot.lock().expect("agent slot");
        tokio::spawn(run_and_mark(
            Arc::downgrade(&self.slot),
            req.run_id.clone(),
            done_tx,
            run_task(
                req.clone(),
                Arc::new(parsed),
                run_ctx,
                Arc::new(executors),
                tx,
                cancel.clone(),
            ),
        ));
        *guard = Slot::Running(ActiveRun {
            run_id: req.run_id.clone(),
            _cancel_on_drop: cancel.clone().drop_guard(),
            cancel,
            events: Some(rx),
            done: done_rx,
        });
        reservation.armed = false;
        Ok(())
    }

    /// Поток читается один раз; `last_id` не нужен — события в процессе не теряются.
    async fn events(
        &self,
        run_id: &str,
        _last_id: Option<u64>,
    ) -> Result<BoxStream<'static, (u64, Event)>, LinkError> {
        let mut guard = self.slot.lock().expect("agent slot");
        let run = guard.run_mut(run_id).ok_or_else(|| not_found(run_id))?;
        let rx = run
            .events
            .take()
            .ok_or_else(|| LinkError::new("RUN_BUSY", "events of a local run are read once"))?;
        Ok(UnboundedReceiverStream::new(rx).boxed())
    }

    async fn stop(&self, run_id: &str) -> Result<(), LinkError> {
        let guard = self.slot.lock().expect("agent slot");
        match &*guard {
            Slot::Starting { run_id: id, cancel } if id == run_id => cancel.cancel(),
            Slot::Running(r) | Slot::Finished { run: r, .. } if r.run_id == run_id => {
                r.cancel.cancel()
            }
            _ => return Err(not_found(run_id)),
        }
        Ok(())
    }

    /// Ждёт конца прогона и отдаёт tar; агент занят, пока файлы не отданы, и свободен после.
    /// Ожидание слот не меняет: сброшенный `files` (таймаут, отключение клиента) оставляет
    /// прогон на месте, и следующий `files` отдаст его итог.
    async fn files(&self, run_id: &str) -> Result<Vec<u8>, LinkError> {
        let mut done = {
            let guard = self.slot.lock().expect("agent slot");
            match &*guard {
                Slot::Running(r) | Slot::Finished { run: r, .. } if r.run_id == run_id => {
                    r.done.clone()
                }
                _ => return Err(not_found(run_id)),
            }
        };
        // Ошибка — отправитель закрыт без итога: задача прогона прервана.
        let ended = done.wait_for(|d| *d).await.is_ok();
        // Итог забирается и слот освобождается одним шагом под замком.
        let mut guard = self.slot.lock().expect("agent slot");
        match std::mem::replace(&mut *guard, Slot::Idle) {
            Slot::Finished { run, outcome } if run.run_id == run_id => outcome,
            Slot::Running(run) if run.run_id == run_id && !ended => Err(LinkError::new(
                "AGENT_UNREACHABLE",
                format!("local run task of {run_id} ended without a result"),
            )),
            other => {
                // Итог уже забрал параллельный `files`, слот занят другим прогоном.
                *guard = other;
                Err(LinkError::new(
                    "NOT_FOUND",
                    format!("files of run {run_id} were already taken"),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_api::EnvAddresses;

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
        assert_eq!(agent.is_busy(), None, "бронь снята после ошибки фабрики");
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

    const KAFKA_SUITE: &str = "format: 1\nname: s\nscenarios:\n  - id: a\n    steps:\n      - kafka_produce: { operation: \"kafka_produce:t\" }\n";

    /// Запрос на сьют `KAFKA_SUITE`: 5 RPS на `kafka_produce:t` в течение `minutes`.
    fn kafka_request(run_id: &str, mode: Mode, minutes: f64) -> RunRequest {
        RunRequest {
            run_id: run_id.into(),
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
                    minute: minutes,
                    pct: 100.0,
                },
            ],
            total_minutes: minutes,
            max_in_flight: 10,
            arrival: glafira_load_model::suite::Arrival::Constant,
            mode,
            kafka: BTreeMap::new(),
            response_schemas: BTreeMap::new(),
            avro_schemas: BTreeMap::new(),
        }
    }

    /// Исполнитель Kafka-шагов: считает вызовы; `fail` — каждый шаг падает с `KAFKA_PRODUCE`.
    struct Counting {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl StepExecutor for Counting {
        async fn execute(
            &self,
            _step: &glafira_load_model::suite::Step,
            _ctx: &mut crate::context::IterationCtx<'_>,
        ) -> Result<crate::executor::StepOutcome, crate::errors::StepError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                return Err(crate::errors::StepError::new(
                    crate::errors::StepCode::KafkaProduce,
                    "broker is down",
                ));
            }
            Ok(crate::executor::StepOutcome::default())
        }

        async fn lag(&self) -> BTreeMap<String, i64> {
            BTreeMap::new()
        }
    }

    /// Фабрика с исполнителем `Counting`; `gate` — фабрика сначала сообщает `entered` и ждёт `gate`.
    fn counting_factory(
        calls: Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
        gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> ExecutorFactory {
        Arc::new(
            move |req: RunRequest, suite: Suite, _ctx: Arc<RunContext>| {
                let calls = calls.clone();
                let gate = gate.clone();
                Box::pin(async move {
                    if let Some((entered, open)) = gate {
                        entered.notify_one();
                        open.notified().await;
                    }
                    let http = crate::http::HttpExecutor::new(
                        req.env.http.as_deref().unwrap_or(""),
                        &suite.defaults.http,
                        &[],
                    )
                    .map_err(|m| EngineError::new("INVALID_INPUT", m))?;
                    Ok(Executors {
                        http,
                        kafka: Some(Box::new(Counting { calls, fail }) as Box<dyn StepExecutor>),
                    })
                })
            },
        )
    }

    fn error_sample(code: &str, operation: &str, t: u64) -> ErrorSample {
        ErrorSample {
            t,
            code: code.into(),
            operation: operation.into(),
            scenario: "a".into(),
            step: "step1".into(),
            agent: "local".into(),
            message: "boom".into(),
            request: None,
            response: None,
        }
    }

    /// Пересылает 20 примеров двух кодов одной операции; возвращает коды отправленных событий,
    /// их id и коды строк `errors.jsonl` агента.
    async fn forward_twenty(cap: Option<SampleCap>) -> (Vec<String>, Vec<u64>, Vec<String>) {
        let (tx, rx) = mpsc::channel(128);
        let (out, mut got) = mpsc::unbounded_channel();
        let forward = tokio::spawn(forward_events(rx, out, cap));
        for t in 0..20 {
            for code in ["KAFKA_PRODUCE", "TIMEOUT"] {
                tx.send(Event::Error(error_sample(code, "kafka_produce:t", t)))
                    .await
                    .unwrap();
            }
        }
        drop(tx);
        let files = forward.await.unwrap();
        let mut codes = Vec::new();
        let mut ids = Vec::new();
        while let Ok((id, ev)) = got.try_recv() {
            ids.push(id);
            if let Event::Error(e) = ev {
                codes.push(e.code);
            }
        }
        let written = String::from_utf8(files.errors)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<ErrorSample>(l).unwrap().code)
            .collect();
        (codes, ids, written)
    }

    #[tokio::test]
    async fn forward_keeps_five_samples_per_code_and_operation_in_run_mode() {
        let (codes, ids, written) = forward_twenty(Some(SampleCap::new())).await;
        let count = |v: &[String], code: &str| v.iter().filter(|c| *c == code).count();
        assert_eq!(codes.len(), 10, "{codes:?}");
        assert_eq!(
            (count(&codes, "KAFKA_PRODUCE"), count(&codes, "TIMEOUT")),
            (5, 5)
        );
        assert_eq!(ids, (1..=10).collect::<Vec<u64>>(), "id без пропусков");
        assert_eq!(
            (count(&written, "KAFKA_PRODUCE"), count(&written, "TIMEOUT")),
            (5, 5),
            "errors.jsonl агента"
        );
        // Смок не ограничивает: отчёту нужен пример каждого шага.
        let (codes, _, written) = forward_twenty(None).await;
        assert_eq!((codes.len(), written.len()), (40, 40));
    }

    #[tokio::test(start_paused = true)]
    async fn run_mode_sends_and_keeps_at_most_five_samples_per_code_and_operation() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = LocalAgent::new("local", counting_factory(calls.clone(), true, None));
        // 30 с отказов: планировщик пропускает по примеру в секунду, предел — пять.
        agent
            .start(
                &kafka_request("r1", Mode::Run, 0.5),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        let mut events = agent.events("r1", None).await.unwrap();
        let mut samples = 0;
        while let Some((_, ev)) = events.next().await {
            if matches!(ev, Event::Error(_)) {
                samples += 1;
            }
        }
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst) > 20,
            "отказов больше предела"
        );
        assert_eq!(samples, 5);
        let tar = agent.files("r1").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tar::Archive::new(&tar[..]).unpack(tmp.path()).unwrap();
        let errors = std::fs::read_to_string(tmp.path().join(ERRORS_FILE)).unwrap();
        assert_eq!(errors.lines().count(), 5, "{errors}");
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_agent_cancels_its_run() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = LocalAgent::new("local", counting_factory(calls.clone(), false, None));
        agent
            .start(
                &kafka_request("r1", Mode::Run, 10.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let before = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(before > 0, "нагрузка идёт");
        drop(agent);
        // Доработка не дольше `DRAIN`; дальше исполнитель не вызывается.
        tokio::time::sleep(DRAIN + Duration::from_secs(2)).await;
        let stopped = calls.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            stopped,
            "после сброса агента нагрузка остановлена"
        );
    }

    #[tokio::test]
    async fn agent_stays_busy_from_reservation_until_files_are_taken() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let open = Arc::new(tokio::sync::Notify::new());
        let agent = Arc::new(LocalAgent::new(
            "local",
            counting_factory(calls, false, Some((entered.clone(), open.clone()))),
        ));
        let first = tokio::spawn({
            let agent = agent.clone();
            async move {
                agent
                    .start(
                        &kafka_request("r1", Mode::Smoke, 0.0),
                        KAFKA_SUITE,
                        &BTreeMap::new(),
                    )
                    .await
            }
        });
        // Фабрика первого прогона ещё ждёт: слот уже занят.
        entered.notified().await;
        assert_eq!(agent.is_busy().as_deref(), Some("r1"));
        let second = agent
            .start(
                &kafka_request("r2", Mode::Smoke, 0.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (second.code.as_str(), second.names),
            ("AGENT_BUSY", vec!["r1".to_string()])
        );
        open.notify_one();
        first.await.unwrap().unwrap();

        let mut events = agent.events("r1", None).await.unwrap();
        let mut done = false;
        while let Some((_, ev)) = events.next().await {
            done |= matches!(ev, Event::Done(_));
        }
        assert!(done);
        // Задача прогона отмечает конец сама; файлы ещё не забраны — агент занят.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(&*agent.slot.lock().unwrap(), Slot::Finished { .. }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("run task marks the slot finished");
        assert_eq!(agent.is_busy().as_deref(), Some("r1"));
        open.notify_one();
        let third = agent
            .start(
                &kafka_request("r3", Mode::Smoke, 0.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(third.code, "AGENT_BUSY", "незабранный прогон не заменяется");

        agent.files("r1").await.unwrap();
        assert_eq!(agent.is_busy(), None, "после files агент свободен");
        assert!(agent.files("r1").await.is_err(), "файлы отдаются один раз");
        agent
            .start(
                &kafka_request("r4", Mode::Smoke, 0.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        agent.files("r4").await.unwrap();
        assert_eq!(agent.is_busy(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn dropped_files_wait_keeps_the_run_and_a_later_files_succeeds() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = LocalAgent::new("local", counting_factory(calls, false, None));
        // Прогон на 30 с: ожидание `files` сбрасывается по таймауту задолго до конца.
        agent
            .start(
                &kafka_request("r1", Mode::Run, 0.5),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        let dropped = tokio::time::timeout(Duration::from_secs(1), agent.files("r1")).await;
        assert!(dropped.is_err(), "ожидание сброшено до конца прогона");
        assert_eq!(agent.is_busy().as_deref(), Some("r1"), "прогон на месте");
        assert!(matches!(&*agent.slot.lock().unwrap(), Slot::Running(_)));
        let busy = agent
            .start(
                &kafka_request("r2", Mode::Smoke, 0.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(busy.code, "AGENT_BUSY");

        let tar = agent.files("r1").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tar::Archive::new(&tar[..]).unpack(tmp.path()).unwrap();
        assert!(tmp.path().join(SUMMARY_FILE).exists());
        assert_eq!(agent.is_busy(), None, "после files агент свободен");
        agent
            .start(
                &kafka_request("r3", Mode::Smoke, 0.0),
                KAFKA_SUITE,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        agent.files("r3").await.unwrap();
        assert_eq!(agent.is_busy(), None);
    }

    /// Слот `Running` с сигналом конца, как после `start`.
    fn running_slot(run_id: &str) -> (SharedSlot, watch::Sender<bool>) {
        let (done_tx, done_rx) = watch::channel(false);
        let cancel = CancellationToken::new();
        let slot = Arc::new(Mutex::new(Slot::Running(ActiveRun {
            run_id: run_id.into(),
            _cancel_on_drop: cancel.clone().drop_guard(),
            cancel,
            events: None,
            done: done_rx,
        })));
        (slot, done_tx)
    }

    fn agent_with_slot(slot: SharedSlot) -> LocalAgent {
        let mut agent = LocalAgent::new("local", counting_factory(Arc::default(), false, None));
        agent.slot = slot;
        agent
    }

    #[tokio::test]
    async fn run_task_panic_and_files_error_end_in_their_codes_and_free_the_agent() {
        // Паника задачи прогона — `AGENT_UNREACHABLE`.
        let (slot, done) = running_slot("r1");
        let agent = agent_with_slot(slot.clone());
        let task = tokio::spawn(run_and_mark(
            Arc::downgrade(&slot),
            "r1".into(),
            done,
            async { panic!("scheduler exploded") },
        ));
        let err = agent.files("r1").await.unwrap_err();
        assert_eq!(err.code, "AGENT_UNREACHABLE");
        assert!(
            err.message.contains("scheduler exploded"),
            "{}",
            err.message
        );
        assert_eq!(agent.is_busy(), None);
        task.await.unwrap();

        // Файлы не собраны — `IO`.
        let (slot, done) = running_slot("r2");
        let agent = agent_with_slot(slot.clone());
        tokio::spawn(run_and_mark(
            Arc::downgrade(&slot),
            "r2".into(),
            done,
            async { Err("agent files tar: disk full".to_string()) },
        ));
        let err = agent.files("r2").await.unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            ("IO", "agent files tar: disk full")
        );
        assert_eq!(agent.is_busy(), None);

        // Задача прервана без итога (отправитель сигнала закрыт) — `AGENT_UNREACHABLE`.
        let (slot, done) = running_slot("r3");
        let agent = agent_with_slot(slot);
        drop(done);
        let err = agent.files("r3").await.unwrap_err();
        assert_eq!(err.code, "AGENT_UNREACHABLE");
        assert_eq!(agent.is_busy(), None);
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
