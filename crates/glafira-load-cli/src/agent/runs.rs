//! Состояние агента: текущий прогон, история, запуск через `LocalAgent`, остановка с дренажом,
//! обработчики `/v1/info`, `/v1/runs`, `/v1/runs/{id}/stop` (спека §4.1, §4.3, §4.4).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::{Multipart, Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use glafira_load_engine::agent_api::{
    AgentInfo, AgentSummary, ApiError, Event, RunListItem, RunRequest, RunStatus,
};
use glafira_load_engine::agent_link::{
    AgentLink, LinkError, LocalAgent, SECRET_PREFIX, same_minor,
};
use glafira_load_engine::local::executor_factory;
use glafira_load_engine::summary::sorted_summary;
use glafira_load_model::suite;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::task::JoinHandle;
use tracing_subscriber::fmt::MakeWriter;

use super::sse::EventLog;
use super::{api_error, api_error_full, not_found};
use crate::console::status_label;

pub const AGENT_LOG: &str = "agent.log";
pub const SUMMARY_FILE: &str = "summary.yaml";
pub const REQUEST_FILE: &str = "run.json";

/// Запас сторожа сверх `--drain-ms`: `LocalAgent` дренирует ровно `drain` (`with_drain`), затем
/// до 1 с ждёт прерванные итерации (`ABORT_GRACE` планировщика), сводит итог и шлёт `done`.
/// Сторож не должен обгонять движок, иначе файлы остановленного прогона теряются.
const DRAIN_GRACE: Duration = Duration::from_secs(3);
/// Сколько насос ждёт файлы `LocalAgent` после `done` (задача прогона к этому моменту кончается).
const FILES_TIMEOUT: Duration = Duration::from_secs(30);
/// Шаг опроса при ожидании конца прогона на выключении агента.
const SHUTDOWN_POLL: Duration = Duration::from_millis(50);

pub struct AgentConfig {
    pub work_dir: PathBuf,
    pub keep_runs: usize,
    pub drain: Duration,
    pub token: String,
    /// Секреты стенда: имя без префикса `GLAFIRA_LOAD_SECRET_` → значение.
    pub secrets_env: BTreeMap<String, String>,
    pub version: String,
}

pub struct RunningRun {
    pub run_id: String,
    pub started_at: String,
    pub dir: PathBuf,
    pub log: Arc<EventLog>,
    pub agent: Arc<LocalAgent>,
    pub pump: Option<JoinHandle<()>>,
    /// Насос получил `done` и собирает файлы: сторож дренажа прогон уже не закрывает.
    pub collecting: bool,
}

pub struct FinishedRun {
    pub run_id: String,
    pub started_at: String,
    pub status: RunStatus,
    pub dir: PathBuf,
    /// Буфер событий для переподключений после `done`; у прогонов до рестарта агента его нет.
    pub log: Option<Arc<EventLog>>,
}

/// Порядок замков: `current`, затем `history` (только в `finish` и `launch`); остальные места берут по одному.
pub struct AgentState {
    pub current: Mutex<Option<RunningRun>>,
    pub history: Mutex<Vec<FinishedRun>>,
    pub work_dir: PathBuf,
    pub keep_runs: usize,
    pub drain: Duration,
    pub token: String,
    pub secrets_env: BTreeMap<String, String>,
    pub version: String,
    pub log_sink: LogSink,
}

/// Писатель `tracing`: stderr всегда, плюс `agent.log` текущего прогона.
#[derive(Clone, Default)]
pub struct LogSink(Arc<Mutex<Option<File>>>);

impl LogSink {
    pub fn set(&self, file: Option<File>) {
        *self.0.lock().unwrap() = file;
    }
}

pub struct SinkWriter(LogSink);

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        if let Some(file) = self.0.0.lock().unwrap().as_mut() {
            let _ = file.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.0.0.lock().unwrap().as_mut() {
            file.flush()?;
        }
        io::stderr().flush()
    }
}

impl<'a> MakeWriter<'a> for LogSink {
    type Writer = SinkWriter;

    fn make_writer(&'a self) -> SinkWriter {
        SinkWriter(self.clone())
    }
}

/// `GLAFIRA_LOAD_SECRET_<NAME>=value` → `NAME → value` (карта для `LocalAgent::with_secrets`).
pub fn secrets_from_vars(vars: impl Iterator<Item = (String, String)>) -> BTreeMap<String, String> {
    vars.filter_map(|(k, v)| {
        k.strip_prefix(SECRET_PREFIX)
            .map(|name| (name.to_string(), v))
    })
    .collect()
}

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

fn busy_response(run_id: &str) -> Response {
    api_error(
        StatusCode::CONFLICT,
        "AGENT_BUSY",
        format!("agent is running {run_id}"),
    )
}

fn link_error_response(e: LinkError, agent_version: &str, controller_version: &str) -> Response {
    let status = match e.code.as_str() {
        "AGENT_BUSY" => StatusCode::CONFLICT,
        "SECRET_MISSING" => StatusCode::PRECONDITION_FAILED,
        "SUITE_INVALID" => StatusCode::UNPROCESSABLE_ENTITY,
        "AGENT_VERSION" => StatusCode::UPGRADE_REQUIRED,
        "INVALID_INPUT" => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let (agent, controller) = if e.code == "AGENT_VERSION" {
        (Some(agent_version.into()), Some(controller_version.into()))
    } else {
        (None, None)
    };
    api_error_full(
        status,
        ApiError {
            code: e.code,
            message: e.message,
            names: e.names,
            agent,
            controller,
        },
    )
}

/// `summary.yaml` атомарно; текст тот же, что у файла из tar `LocalAgent` (`sorted_summary`).
fn write_summary(dir: &Path, summary: &AgentSummary) {
    let tmp = dir.join(format!("{SUMMARY_FILE}.tmp"));
    let text = glafira_load_model::to_yaml(&sorted_summary(summary));
    let result =
        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, dir.join(SUMMARY_FILE)));
    if let Err(e) = result {
        tracing::warn!("cannot write {SUMMARY_FILE}: {e}");
    }
}

fn empty_summary(status: RunStatus) -> AgentSummary {
    AgentSummary {
        status,
        ops: vec![],
        scenarios: vec![],
        seconds: 0,
    }
}

/// Файлы `LocalAgent` (`metrics.jsonl`, `errors.jsonl`, `summary.yaml`, `hist.bin`) из tar в папку прогона.
fn unpack_agent_files(dir: &Path, tar: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    tar::Archive::new(tar).unpack(dir)
}

/// `files()` встроенного агента → tar → `work/<run_id>/`; ошибки только в журнал.
async fn collect_files(agent: &LocalAgent, run_id: &str, dir: &Path) {
    match tokio::time::timeout(FILES_TIMEOUT, agent.files(run_id)).await {
        Ok(Ok(tar)) => {
            let dir = dir.to_path_buf();
            let unpacked = tokio::task::spawn_blocking(move || unpack_agent_files(&dir, &tar))
                .await
                .unwrap_or_else(|e| Err(io::Error::other(e)));
            if let Err(e) = unpacked {
                tracing::warn!(run_id, "cannot unpack agent files: {e}");
            }
        }
        Ok(Err(e)) => tracing::warn!(run_id, "files: {}", e.message),
        Err(_) => tracing::warn!(run_id, "files: no result in {} s", FILES_TIMEOUT.as_secs()),
    }
}

/// Насос: события `LocalAgent` → буфер SSE. `done` придерживается: сначала файлы агента
/// распаковываются в `work/<run_id>/`, затем `finish` переносит прогон в историю и только потом
/// кладёт `done` в буфер — клиент, увидевший `done`, находит прогон в `/v1/runs` и все файлы.
/// Поток без `done` — `failed`.
async fn pump_events(state: Arc<AgentState>, agent: Arc<LocalAgent>, run_id: String, dir: PathBuf) {
    let Some(log) = state.find_log(&run_id) else {
        return;
    };
    let mut finished: Option<AgentSummary> = None;
    match agent.events(&run_id, None).await {
        Ok(mut stream) => {
            while let Some((_, event)) = stream.next().await {
                if let Event::Done(summary) = event {
                    finished = Some(summary);
                    break;
                }
                log.push(event);
            }
        }
        Err(e) => tracing::error!(run_id, "events: {}", e.message),
    }
    let summary = match finished {
        Some(summary) => {
            if !state.mark_collecting(&run_id) {
                return;
            }
            collect_files(&agent, &run_id, &dir).await;
            summary
        }
        None => {
            tracing::error!(run_id, "event stream ended without done");
            empty_summary(RunStatus::Failed)
        }
    };
    state.finish(&run_id, summary);
}

/// Бронь слота на время `LocalAgent::start`: ошибка или сброшенный запрос (клиент отключился)
/// освобождают агент и убирают папку несостоявшегося прогона.
struct Launch<'a> {
    state: &'a AgentState,
    run_id: String,
    dir: PathBuf,
    armed: bool,
}

impl Drop for Launch<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.state.clear_current(&self.run_id).is_some() {
            self.state.log_sink.set(None);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

impl AgentState {
    /// Создаёт состояние, перечитывает папки прошлых прогонов в `work_dir` и удаляет лишние по `keep_runs`.
    pub fn new(config: AgentConfig) -> io::Result<Arc<Self>> {
        std::fs::create_dir_all(&config.work_dir)?;
        let mut history = Vec::new();
        for entry in std::fs::read_dir(&config.work_dir)? {
            let entry = entry?;
            let dir = entry.path();
            // Прогон — только папка с `run.json`: чужие папки в `--work` не трогаются.
            if !dir.is_dir() || !dir.join(REQUEST_FILE).is_file() {
                continue;
            }
            let started_at = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| OffsetDateTime::from(t).format(&Rfc3339).unwrap_or_default())
                .unwrap_or_default();
            let status = std::fs::read_to_string(dir.join(SUMMARY_FILE))
                .ok()
                .and_then(|text| glafira_load_model::from_yaml::<AgentSummary>(&text).ok())
                .map(|s| s.status)
                .unwrap_or(RunStatus::Incomplete);
            history.push(FinishedRun {
                run_id: entry.file_name().to_string_lossy().into_owned(),
                started_at,
                status,
                dir,
                log: None,
            });
        }
        history.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        let state = Arc::new(Self {
            current: Mutex::new(None),
            history: Mutex::new(history),
            work_dir: config.work_dir,
            keep_runs: config.keep_runs.max(1),
            drain: config.drain,
            token: config.token,
            secrets_env: config.secrets_env,
            version: config.version,
            log_sink: LogSink::default(),
        });
        state.prune(0);
        Ok(state)
    }

    /// Удаляет старые прогоны так, чтобы вместе с `reserve` новыми их было не больше `keep_runs`.
    fn prune(&self, reserve: usize) {
        let mut history = self.history.lock().unwrap();
        while history.len() + reserve > self.keep_runs && !history.is_empty() {
            let old = history.remove(0);
            if let Err(e) = std::fs::remove_dir_all(&old.dir)
                && e.kind() != io::ErrorKind::NotFound
            {
                tracing::warn!("cannot remove {}: {e}", old.dir.display());
            }
        }
    }

    pub fn busy(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.run_id.clone())
    }

    pub fn find_log(&self, run_id: &str) -> Option<Arc<EventLog>> {
        if let Some(r) = self.current.lock().unwrap().as_ref()
            && r.run_id == run_id
        {
            return Some(r.log.clone());
        }
        self.history
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.run_id == run_id)
            .and_then(|r| r.log.clone())
    }

    pub fn run_dir(&self, run_id: &str) -> Option<PathBuf> {
        if let Some(r) = self.current.lock().unwrap().as_ref()
            && r.run_id == run_id
        {
            return Some(r.dir.clone());
        }
        self.history
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.run_id == run_id)
            .map(|r| r.dir.clone())
    }

    /// Папка завершённого прогона (из истории); текущий прогон сюда не входит.
    pub fn history_dir(&self, run_id: &str) -> Option<PathBuf> {
        self.history
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.run_id == run_id)
            .map(|r| r.dir.clone())
    }

    pub fn list(&self) -> Vec<RunListItem> {
        self.history
            .lock()
            .unwrap()
            .iter()
            .map(|r| RunListItem {
                run_id: r.run_id.clone(),
                status: r.status,
                started_at: r.started_at.clone(),
            })
            .collect()
    }

    pub fn info(&self) -> AgentInfo {
        AgentInfo {
            version: self.version.clone(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            cpus: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            busy: self.busy(),
        }
    }

    fn prepare_dir(&self, dir: &Path, req: &RunRequest) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(REQUEST_FILE), serde_json::to_vec_pretty(req)?)?;
        self.log_sink.set(Some(File::create(dir.join(AGENT_LOG))?));
        Ok(())
    }

    fn clear_current(&self, run_id: &str) -> Option<RunningRun> {
        self.current.lock().unwrap().take_if(|r| r.run_id == run_id)
    }

    /// Насос получил `done`: сторож дренажа больше не вмешивается. `false` — прогон уже закрыт.
    fn mark_collecting(&self, run_id: &str) -> bool {
        match self.current.lock().unwrap().as_mut() {
            Some(r) if r.run_id == run_id => {
                r.collecting = true;
                true
            }
            _ => false,
        }
    }

    // Ошибка — готовый ответ axum: он сразу уходит клиенту, упаковка в `Box` ничего не даёт.
    #[allow(clippy::result_large_err)]
    /// Запуск: папка `work/<run_id>/`, `LocalAgent::start`, насос событий. Слот занимается до первого `await`.
    pub async fn launch(
        self: &Arc<Self>,
        req: RunRequest,
        suite_text: String,
        data: BTreeMap<String, Vec<u8>>,
        secrets: BTreeMap<String, String>,
    ) -> Result<(), Response> {
        let run_id = req.run_id.clone();
        if run_id.is_empty() || run_id.starts_with('.') || run_id.contains(['/', '\\', ':']) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "run_id must be a plain folder name",
            ));
        }
        if self
            .history
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.run_id == run_id)
        {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                format!("run {run_id} already exists on this agent"),
            ));
        }
        let dir = self.work_dir.join(&run_id);
        let log = Arc::new(EventLog::new());
        let agent = Arc::new(
            LocalAgent::new(&req.agent_id, executor_factory())
                .with_secrets(secrets)
                .with_drain(self.drain),
        );
        {
            let mut current = self.current.lock().unwrap();
            if let Some(r) = current.as_ref() {
                return Err(busy_response(&r.run_id));
            }
            *current = Some(RunningRun {
                run_id: run_id.clone(),
                started_at: now_rfc3339(),
                dir: dir.clone(),
                log,
                agent: agent.clone(),
                pump: None,
                collecting: false,
            });
        }
        let mut launch = Launch {
            state: self,
            run_id: run_id.clone(),
            dir: dir.clone(),
            armed: true,
        };
        if let Err(e) = self.prepare_dir(&dir, &req) {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "IO",
                format!("cannot prepare {}: {e}", dir.display()),
            ));
        }
        tracing::info!(run_id, suite = %req.suite_name, mode = ?req.mode, "run accepted");
        if let Err(e) = agent.start(&req, &suite_text, &data).await {
            tracing::error!(run_id, code = %e.code, "run rejected: {}", e.message);
            return Err(link_error_response(
                e,
                &self.version,
                &req.controller_version,
            ));
        }
        launch.armed = false;
        {
            // Насос запускается под замком: его `finish` не обгонит запись `pump`.
            let mut current = self.current.lock().unwrap();
            if let Some(r) = current.as_mut()
                && r.run_id == run_id
            {
                // Старые прогоны удаляются только после принятого старта (отказ ничего не
                // стирает) и до насоса: под замком `current` новый прогон в историю не попадёт.
                self.prune(1);
                r.pump = Some(tokio::spawn(pump_events(
                    self.clone(),
                    agent,
                    run_id.clone(),
                    dir,
                )));
                return Ok(());
            }
        }
        // Пока шёл `start`, прогон закрыли (остановка и сторож): он уже в истории,
        // а запущенный `LocalAgent` без насоса никто бы не дослушал — останавливаем его.
        tracing::warn!(run_id, "run was closed while starting, stopping it");
        if let Err(e) = agent.stop(&run_id).await {
            tracing::warn!(run_id, "stop: {}", e.message);
        }
        self.prune(0);
        Ok(())
    }

    /// Переносит прогон в историю, пишет `summary.yaml` и только затем кладёт `done` в буфер.
    /// Второй вызов для того же прогона (насос и сторож) ничего не делает.
    pub fn finish(&self, run_id: &str, summary: AgentSummary) {
        let log = {
            let mut current = self.current.lock().unwrap();
            let Some(run) = current.take_if(|r| r.run_id == run_id) else {
                return;
            };
            write_summary(&run.dir, &summary);
            self.history.lock().unwrap().push(FinishedRun {
                run_id: run.run_id,
                started_at: run.started_at,
                status: summary.status,
                dir: run.dir,
                log: Some(run.log.clone()),
            });
            run.log
        };
        tracing::info!(
            run_id,
            status = status_label(&summary.status),
            "run finished"
        );
        self.log_sink.set(None);
        log.push(Event::Done(summary));
    }

    // Ошибка — готовый ответ axum: он сразу уходит клиенту, упаковка в `Box` ничего не даёт.
    #[allow(clippy::result_large_err)]
    /// Мягкая остановка: отмена в `LocalAgent`, затем сторож на `drain` (с запасом `DRAIN_GRACE`) —
    /// если `done` не пришёл, прогон закрываем сами. Завершённый прогон — `Ok`, неизвестный — 404.
    pub async fn stop(self: &Arc<Self>, run_id: &str) -> Result<(), Response> {
        let agent = {
            let current = self.current.lock().unwrap();
            match current.as_ref() {
                Some(r) if r.run_id == run_id => Some(r.agent.clone()),
                _ => None,
            }
        };
        let Some(agent) = agent else {
            return if self.run_dir(run_id).is_some() {
                Ok(())
            } else {
                Err(not_found(run_id))
            };
        };
        tracing::info!(run_id, "stop requested");
        if let Err(e) = agent.stop(run_id).await {
            tracing::warn!(run_id, "stop: {}", e.message);
        }
        let state = self.clone();
        let id = run_id.to_string();
        let wait = self.drain + DRAIN_GRACE;
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            state.force_done(&id, RunStatus::Stopped);
        });
        Ok(())
    }

    /// Выключение агента: текущий прогон останавливается, и выключение ждёт, пока он не уйдёт
    /// в историю с файлами (не дольше дренажа, запаса сторожа и сбора файлов).
    pub async fn shutdown(self: &Arc<Self>) {
        let Some(run_id) = self.busy() else {
            return;
        };
        let _ = self.stop(&run_id).await;
        let limit = self.drain + DRAIN_GRACE + FILES_TIMEOUT;
        let waited = tokio::time::timeout(limit, async {
            while self.busy().is_some() {
                tokio::time::sleep(SHUTDOWN_POLL).await;
            }
        })
        .await;
        if waited.is_err() {
            tracing::warn!(run_id, "run did not finish in {} s", limit.as_secs());
        }
    }

    /// Принудительное завершение: насос прерывается, в буфер идёт `done` с пустой сводкой
    /// (она же пишется в `summary.yaml`). Если насос уже получил `done`, прогон закроет он.
    pub fn force_done(&self, run_id: &str, status: RunStatus) {
        let pump = {
            let mut current = self.current.lock().unwrap();
            match current.as_mut() {
                Some(r) if r.run_id == run_id && !r.collecting => r.pump.take(),
                _ => return,
            }
        };
        if let Some(pump) = pump {
            pump.abort();
        }
        tracing::warn!(
            run_id,
            "drain timeout: closing the run as {}",
            status_label(&status)
        );
        self.finish(run_id, empty_summary(status));
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub run_id: String,
}

#[derive(Default)]
struct Bundle {
    run: Option<RunRequest>,
    suite: Option<String>,
    data: BTreeMap<String, Vec<u8>>,
}

// Ошибка — готовый ответ axum: он сразу уходит клиенту, упаковка в `Box` ничего не даёт.
#[allow(clippy::result_large_err)]
/// Части бандла: `run.json`, `suite.yaml`, `data/<набор>`; прочие части игнорируются.
async fn read_bundle(mut multipart: Multipart) -> Result<Bundle, Response> {
    let bad = |m: String| api_error(StatusCode::BAD_REQUEST, "BAD_REQUEST", m);
    let mut bundle = Bundle::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(format!("multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| bad(format!("multipart part {name}: {e}")))?;
        if name == "run.json" {
            bundle.run =
                Some(serde_json::from_slice(&bytes).map_err(|e| bad(format!("run.json: {e}")))?);
        } else if name == "suite.yaml" {
            bundle.suite = Some(
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| bad("suite.yaml is not UTF-8".into()))?,
            );
        } else if let Some(set) = name.strip_prefix("data/") {
            bundle.data.insert(set.to_string(), bytes.to_vec());
        }
    }
    Ok(bundle)
}

/// `GET /v1/info`.
pub async fn info(State(state): State<Arc<AgentState>>) -> Json<AgentInfo> {
    Json(state.info())
}

/// `GET /v1/runs` — история завершённых прогонов.
pub async fn list(State(state): State<Arc<AgentState>>) -> Json<Vec<RunListItem>> {
    Json(state.list())
}

/// `POST /v1/runs`: 409 занят, 426 версия или формат сьюта, 422 сьют, 412 секреты, 202 принят.
pub async fn start(State(state): State<Arc<AgentState>>, multipart: Multipart) -> Response {
    let bundle = match read_bundle(multipart).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(req) = bundle.run else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "run.json is missing",
        );
    };
    let Some(suite_text) = bundle.suite else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "suite.yaml is missing",
        );
    };
    if let Some(id) = state.busy() {
        return busy_response(&id);
    }
    if !same_minor(&req.controller_version, &state.version) || req.suite_format != suite::FORMAT {
        return api_error_full(
            StatusCode::UPGRADE_REQUIRED,
            ApiError {
                code: "AGENT_VERSION".into(),
                message: format!(
                    "agent {} (suite format {}) does not match controller {} (suite format {})",
                    state.version,
                    suite::FORMAT,
                    req.controller_version,
                    req.suite_format
                ),
                names: vec![],
                agent: Some(state.version.clone()),
                controller: Some(req.controller_version.clone()),
            },
        );
    }
    let parsed = match suite::parse(&suite_text) {
        Ok(s) => s,
        Err(e) => {
            let message = match e.line {
                Some(line) => format!("suite.yaml:{line}: {}", e.message),
                None => e.message,
            };
            return api_error(StatusCode::UNPROCESSABLE_ENTITY, "SUITE_INVALID", message);
        }
    };
    // секреты из запроса и из сьюта — на агенте должны быть все
    let mut wanted = req.secrets.clone();
    for name in &parsed.secrets {
        if !wanted.contains(name) {
            wanted.push(name.clone());
        }
    }
    let missing: Vec<String> = wanted
        .iter()
        .filter(|n| !state.secrets_env.contains_key(n.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return api_error_full(
            StatusCode::PRECONDITION_FAILED,
            ApiError {
                code: "SECRET_MISSING".into(),
                message: format!("missing secrets on the agent: {}", missing.join(", ")),
                names: missing,
                agent: None,
                controller: None,
            },
        );
    }
    let secrets: BTreeMap<String, String> = wanted
        .iter()
        .filter_map(|n| state.secrets_env.get(n).map(|v| (n.clone(), v.clone())))
        .collect();
    let run_id = req.run_id.clone();
    match state.launch(req, suite_text, bundle.data, secrets).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(StartResponse { run_id })).into_response(),
        Err(r) => r,
    }
}

/// `POST /v1/runs/{id}/stop`: 200 и для уже завершённого прогона, 404 для неизвестного.
pub async fn stop(
    State(state): State<Arc<AgentState>>,
    AxPath(run_id): AxPath<String>,
) -> Response {
    match state.stop(&run_id).await {
        Ok(()) => (StatusCode::OK, Json(StartResponse { run_id })).into_response(),
        Err(r) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_secrets_helpers() {
        // `same_minor` — из движка (Task 8); здесь проверяется только карта секретов.
        assert!(
            same_minor("0.2.0", "0.2.7")
                && !same_minor("0.2.0", "0.3.0")
                && !same_minor("0.2.0", "garbage")
        );
        let vars = vec![
            ("GLAFIRA_LOAD_SECRET_API_TOKEN".to_string(), "t".to_string()),
            ("PATH".to_string(), "/bin".to_string()),
        ];
        assert_eq!(
            secrets_from_vars(vars.into_iter()),
            BTreeMap::from([("API_TOKEN".to_string(), "t".to_string())])
        );
    }

    #[test]
    fn new_state_reads_history_and_prunes() {
        let work = tempfile::tempdir().unwrap();
        for (name, summary) in [
            (
                "a",
                Some("status: completed\nops: []\nscenarios: []\nseconds: 1\n"),
            ),
            ("b", None),
            (
                "c",
                Some("status: stopped\nops: []\nscenarios: []\nseconds: 1\n"),
            ),
        ] {
            let dir = work.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(REQUEST_FILE), "{}").unwrap();
            if let Some(text) = summary {
                std::fs::write(dir.join(SUMMARY_FILE), text).unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let config = AgentConfig {
            work_dir: work.path().to_path_buf(),
            keep_runs: 2,
            drain: Duration::from_secs(1),
            token: "t".into(),
            secrets_env: BTreeMap::new(),
            version: "0.2.0".into(),
        };
        let state = AgentState::new(config).unwrap();
        let list = state.list();
        assert_eq!(
            list.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            ["b", "c"],
            "самый старый удалён"
        );
        assert!(matches!(list[0].status, RunStatus::Incomplete));
        assert!(matches!(list[1].status, RunStatus::Stopped));
        assert!(!work.path().join("a").exists());
        assert_eq!(state.busy(), None);
        assert!(
            state.find_log("b").is_none(),
            "у прогонов до рестарта буфера нет"
        );
        assert_eq!(state.run_dir("c"), Some(work.path().join("c")));
    }

    #[test]
    fn startup_keeps_folders_that_are_not_runs() {
        for keep_runs in [0, 1] {
            let work = tempfile::tempdir().unwrap();
            let notes = work.path().join("notes");
            std::fs::create_dir_all(&notes).unwrap();
            std::fs::write(notes.join("readme.txt"), "mine").unwrap();
            std::fs::write(work.path().join("loose.txt"), "mine").unwrap();
            for name in ["r1", "r2"] {
                let dir = work.path().join(name);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(REQUEST_FILE), "{}").unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
            let state = AgentState::new(AgentConfig {
                work_dir: work.path().to_path_buf(),
                keep_runs,
                drain: Duration::from_secs(1),
                token: "t".into(),
                secrets_env: BTreeMap::new(),
                version: "0.2.0".into(),
            })
            .unwrap();
            assert_eq!(
                state
                    .list()
                    .iter()
                    .map(|r| r.run_id.as_str())
                    .collect::<Vec<_>>(),
                ["r2"],
                "keep_runs {keep_runs}: папка без run.json — не прогон"
            );
            assert!(!work.path().join("r1").exists());
            assert!(notes.join("readme.txt").is_file(), "чужая папка цела");
            assert!(work.path().join("loose.txt").is_file());
            assert_eq!(state.run_dir("notes"), None);
        }
    }

    #[test]
    fn finish_moves_to_history_before_done_and_only_once() {
        let work = tempfile::tempdir().unwrap();
        let config = AgentConfig {
            work_dir: work.path().to_path_buf(),
            keep_runs: 5,
            drain: Duration::from_secs(1),
            token: "t".into(),
            secrets_env: BTreeMap::new(),
            version: "0.2.0".into(),
        };
        let state = AgentState::new(config).unwrap();
        let dir = work.path().join("r1");
        std::fs::create_dir_all(&dir).unwrap();
        let log = Arc::new(EventLog::new());
        let agent = Arc::new(LocalAgent::new("gen-1", executor_factory()));
        *state.current.lock().unwrap() = Some(RunningRun {
            run_id: "r1".into(),
            started_at: now_rfc3339(),
            dir: dir.clone(),
            log: log.clone(),
            agent,
            pump: None,
            collecting: false,
        });
        state.force_done("r1", RunStatus::Stopped);
        assert_eq!(state.busy(), None);
        assert!(matches!(log.status(), Some(RunStatus::Stopped)));
        assert!(matches!(state.list()[0].status, RunStatus::Stopped));
        let text = std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap();
        assert!(text.contains("stopped"), "{text}");
        // повторное закрытие (насос после сторожа) ничего не меняет
        state.finish("r1", empty_summary(RunStatus::Completed));
        assert_eq!(state.list().len(), 1);
        assert_eq!(log.last_id(), 1);
    }
}
