//! Контроллер прогона: опрашивает агентов, раздаёт бандлы, сводит события и собирает `runs/<дата>/`
//! (спека §4.5, §6). Один код для приложения и `glafira-load run --agents` (отступление плана 8).

pub mod bundle;
pub mod merge;
pub mod plan;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::future::join_all;
use glafira_load_model::profile::Computed;
use glafira_load_model::project::{Environment, is_id};
use glafira_load_model::registry::{REGISTRY_FILE, Registry};
use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::Suite;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::agent_api::{AgentSummary, ErrorSample, Event, Mode, PhaseEvent, RunStatus, Tick};
use crate::agent_link::{AgentLink, same_minor};
use crate::errors::SampleCap;
use crate::metrics::{Totals, merge_totals, summary};
use crate::run_files::{
    AGENTS_DIR, ERRORS_FILE, HIST_FILE, METRICS_FILE, PHASES_FILE, RUNS_DIR, RunAgent, RunMeta,
    RunWriter, now_rfc3339, run_dir_name, totals_from_hist_log, write_atomic,
};
use merge::{Accumulator, TickMerger};

pub use crate::agent_link::{AgentState, ControllerEvent, EngineError, RunResult};
pub use crate::local::project_commit;
pub use bundle::{slice_rows, split_targets};

/// Версия контроллера — версия крейта; сравнивается с версией агента по `major.minor`.
pub const CONTROLLER_VERSION: &str = crate::agent_link::VERSION;
/// Сколько ждать восстановления потока событий агента, прежде чем считать его потерянным.
pub const LOST_AFTER: Duration = Duration::from_secs(30);
/// Пауза между повторными подключениями к потоку событий.
pub const RECONNECT_PAUSE: Duration = Duration::from_secs(1);
/// Запасной срок буфера слияния (`TickMerger`): секунда, которую кто-то из живых агентов ещё не
/// прошёл, закрывается без него через столько после её первого тика. Обычно секунда закрывается
/// раньше — как только все живые агенты прислали тики с этой `t` или старше; срок нужен только для
/// зависшего агента. Больше разницы в старте агентов (ожидание назначения партиций Kafka до 10 с
/// плюс загрузка среза), чтобы поздно стартовавший агент не выпадал из ряда, и меньше `LOST_AFTER`.
pub const MERGE_HOLD: Duration = Duration::from_secs(15);
const _: () = assert!(
    MERGE_HOLD.as_secs() > crate::kafka::ASSIGNMENT_WAIT.as_secs()
        && MERGE_HOLD.as_secs() < LOST_AFTER.as_secs()
);
/// Как часто проверять буфер слияния на просроченные секунды.
const EXPIRE_EVERY: Duration = Duration::from_millis(500);
/// Сколько раз спрашивать файлы у агента, пока он отвечает `AGENT_BUSY` (прогон ещё не ушёл
/// в историю: потерянный агент дорабатывает после `stop`), и пауза между попытками.
const FILES_ATTEMPTS: u32 = 10;
const FILES_RETRY_PAUSE: Duration = Duration::from_secs(1);

/// Всё, что нужно для прогона: проект уже прочитан и проверен (`plan::load_plan`).
#[derive(Debug, Clone)]
pub struct RunPlan {
    pub root: PathBuf,
    pub suite_name: String,
    pub suite: Suite,
    pub suite_text: String,
    pub profile: String,
    pub computed: Computed,
    pub env: Environment,
    pub mode: Mode,
    pub max_in_flight: Option<u32>,
    pub out_dir: Option<PathBuf>,
    /// Схемы ответов операций реестра (`expect.schema: response`) — уходят агентам в `RunRequest`.
    pub response_schemas: BTreeMap<String, SchemaSummary>,
    /// Тексты `.avsc` операций `kafka_produce` с `encoding: avro` — уходят агентам в `RunRequest`.
    pub avro_schemas: BTreeMap<String, String>,
}

/// Вес агента по манифесту стенда; агент вне манифеста (например, `LocalAgent`) — 1.
pub fn weight_of(env: &Environment, id: &str) -> u32 {
    env.agents
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.weight.max(1))
        .unwrap_or(1)
}

/// Версии сервисов из `registry/operations.yaml` проекта (`local::service_versions` по прочитанному
/// реестру); нет файла или он не разбирается — пусто.
pub fn service_versions(root: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join(REGISTRY_FILE)) else {
        return BTreeMap::new();
    };
    let Ok(registry) = glafira_load_model::from_yaml::<Registry>(&text) else {
        return BTreeMap::new();
    };
    crate::local::service_versions(&registry)
}

/// Сообщение задачи агента сводному циклу.
enum AgentMsg {
    Event(Event),
    /// Поток событий не восстановился за `LOST_AFTER`.
    Lost,
}

/// Читает поток событий агента до `done`; при обрыве — повтор с `Last-Event-ID` раз в
/// `RECONNECT_PAUSE`, пока поток молчит меньше `LOST_AFTER`, затем `Lost`.
async fn agent_task(
    link: Arc<dyn AgentLink>,
    run_id: String,
    tx: mpsc::Sender<(String, AgentMsg)>,
) {
    let id = link.id().to_string();
    let mut last_id: Option<u64> = None;
    let mut broken_since: Option<Instant> = None;
    loop {
        match link.events(&run_id, last_id).await {
            Ok(mut stream) => {
                while let Some((event_id, event)) = stream.next().await {
                    broken_since = None;
                    last_id = Some(event_id);
                    let done = matches!(event, Event::Done(_));
                    if tx.send((id.clone(), AgentMsg::Event(event))).await.is_err() || done {
                        return;
                    }
                }
                tracing::warn!(agent = %id, "event stream ended before done, reconnecting");
            }
            Err(e) => tracing::warn!(agent = %id, code = %e.code, "events failed: {}", e.message),
        }
        let since = *broken_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= LOST_AFTER {
            let _ = tx.send((id, AgentMsg::Lost)).await;
            return;
        }
        tokio::time::sleep(RECONNECT_PAUSE).await;
    }
}

async fn stop_all(agents: &[Arc<dyn AgentLink>], run_id: &str) {
    let results = join_all(agents.iter().map(|a| a.stop(run_id))).await;
    for (a, r) in agents.iter().zip(results) {
        if let Err(e) = r {
            tracing::warn!(agent = a.id(), code = %e.code, "stop failed: {}", e.message);
        }
    }
}

/// Id агентов — имена папок `agents/<id>/` и ключи сведения: допустимые id манифеста, без повторов.
fn check_agent_ids(agents: &[Arc<dyn AgentLink>]) -> Result<(), EngineError> {
    if agents.is_empty() {
        return Err(EngineError::new("AGENT_UNREACHABLE", "no agents to run on"));
    }
    let mut seen = BTreeSet::new();
    for a in agents {
        let id = a.id();
        if !is_id(id) {
            return Err(EngineError::new(
                "INVALID_INPUT",
                format!("agent id `{id}` must match [a-z0-9-]"),
            )
            .with_agent(id));
        }
        if !seen.insert(id) {
            return Err(
                EngineError::new("INVALID_INPUT", format!("agent id `{id}` is repeated"))
                    .with_agent(id),
            );
        }
    }
    Ok(())
}

/// Распаковывает в `dir` только обычные файлы верхнего уровня tar: вложенные пути, `..`, ссылки
/// и каталоги пропускаются — архив пришёл от агента по сети. Каждый файл пишется атомарно.
fn unpack_flat(tar: &[u8], dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut archive = tar::Archive::new(tar);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut parts = path.components();
        let name = match (parts.next(), parts.next()) {
            (Some(Component::Normal(name)), None) if entry.header().entry_type().is_file() => {
                name.to_owned()
            }
            _ => {
                tracing::warn!(dir = %dir.display(), "skipping agent tar entry {}", path.display());
                continue;
            }
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        write_atomic(&dir.join(name), &bytes)?;
    }
    Ok(())
}

/// Файлы агента в `out` и его итог из `hist.bin`. Потерянному агенту `stop` повторяется
/// (спека §4.5); `AGENT_BUSY` — прогон ещё не в истории агента, спрашиваем снова.
async fn collect_files(
    link: Arc<dyn AgentLink>,
    run_id: String,
    lost: bool,
    out: PathBuf,
) -> Result<Totals, String> {
    if lost && let Err(e) = link.stop(&run_id).await {
        tracing::warn!(agent = link.id(), code = %e.code, "stop retry failed: {}", e.message);
    }
    let mut attempt = 1;
    let tar = loop {
        match link.files(&run_id).await {
            Ok(bytes) => break bytes,
            Err(e) if e.code == "AGENT_BUSY" && attempt < FILES_ATTEMPTS => {
                attempt += 1;
                tokio::time::sleep(FILES_RETRY_PAUSE).await;
            }
            Err(e) => return Err(format!("files: {e}")),
        }
    };
    tokio::task::spawn_blocking(move || {
        unpack_flat(&tar, &out)
            .map_err(|e| format!("cannot unpack files into {}: {e}", out.display()))?;
        let bytes = std::fs::read(out.join(HIST_FILE)).map_err(|e| format!("{HIST_FILE}: {e}"))?;
        totals_from_hist_log(&bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Сводный ряд и файлы прогона: сведение тиков, запись, пересылка событий, сумма счётчиков.
struct Collector {
    writer: RunWriter,
    merger: TickMerger,
    acc: Accumulator,
    /// Предел примеров ошибок на код и операцию на весь прогон (все агенты вместе).
    samples: SampleCap,
    events: Sender<ControllerEvent>,
    /// Первая ошибка записи: дальше файлы не пишутся, прогон останавливается.
    write_err: Option<EngineError>,
}

impl Collector {
    fn write(&mut self, file: &str, r: std::io::Result<()>) {
        if let Err(e) = r
            && self.write_err.is_none()
        {
            tracing::error!("cannot write {file}: {e}");
            self.write_err = Some(EngineError::io(format!("{file}: {e}")));
        }
    }

    /// Закрытые секунды — в счётчики, `metrics.jsonl` и события; опоздавшие тики — только
    /// в счётчики итога (ряд по `t` уже ушёл дальше).
    async fn emit(&mut self, closed: Vec<Tick>) {
        for t in closed {
            self.acc.add(&t);
            if self.write_err.is_none() {
                let r = self.writer.tick(&t);
                self.write(METRICS_FILE, r);
            }
            let _ = self.events.send(ControllerEvent::Tick(t)).await;
        }
        for t in self.merger.take_late() {
            self.acc.add(&t);
        }
    }

    async fn push(&mut self, agent: &str, tick: Tick) {
        let closed = self.merger.push(agent, tick, Instant::now());
        self.emit(closed).await;
    }

    async fn expire(&mut self) {
        let closed = self.merger.expire(Instant::now());
        self.emit(closed).await;
    }

    async fn drop_agent(&mut self, agent: &str) {
        let closed = self.merger.drop_agent(agent);
        self.emit(closed).await;
    }

    async fn flush(&mut self) {
        let closed = self.merger.flush();
        self.emit(closed).await;
    }

    async fn phase(&mut self, p: PhaseEvent) {
        if self.write_err.is_none() {
            let r = self.writer.phase(&p);
            self.write(PHASES_FILE, r);
        }
        let _ = self.events.send(ControllerEvent::Phase(p)).await;
    }

    async fn error(&mut self, e: ErrorSample) {
        if !self.samples.allow(&e.code, &e.operation) {
            return;
        }
        if self.write_err.is_none() {
            let r = self.writer.error(&e);
            self.write(ERRORS_FILE, r);
        }
        let _ = self.events.send(ControllerEvent::Error(e)).await;
    }
}

/// Контроллер прогона на агентах `AgentLink` (HTTP-агенты или встроенный `LocalAgent`).
pub struct Controller;

impl Controller {
    /// Прогон по спеке §4.5: `info` всех → старт всех (отказ одного — `stop` запущенным) →
    /// папка прогона → сведение потоков событий → по `done` всех или после отмены — файлы агентов
    /// в `agents/<id>/`, сводка из их `hist.bin` и счётчиков тиков → `ControllerEvent::Done`.
    /// Ошибка до создания папки — папки нет и нагрузка не идёт.
    pub async fn run(
        plan: RunPlan,
        agents: Vec<Arc<dyn AgentLink>>,
        events: Sender<ControllerEvent>,
        cancel: CancellationToken,
    ) -> Result<RunResult, EngineError> {
        check_agent_ids(&agents)?;
        let send = |ev: ControllerEvent| {
            let events = events.clone();
            async move {
                let _ = events.send(ev).await;
            }
        };

        // 1. `info` всех агентов параллельно: недоступный, занятый или другой версии — отказ до старта.
        let infos = join_all(agents.iter().map(|a| a.info())).await;
        let mut versions = BTreeMap::new();
        for (a, r) in agents.iter().zip(infos) {
            let id = a.id();
            let info = r.map_err(|e| EngineError::from_link(e, id))?;
            if let Some(run) = info.busy {
                return Err(EngineError::new(
                    "AGENT_BUSY",
                    format!("agent {id} is busy with run {run}"),
                )
                .with_names(vec![run])
                .with_agent(id));
            }
            if !same_minor(&info.version, CONTROLLER_VERSION) {
                return Err(EngineError::new(
                    "AGENT_VERSION",
                    format!(
                        "agent {id} runs version {}, controller is {CONTROLLER_VERSION}",
                        info.version
                    ),
                )
                .with_names(vec![info.version, CONTROLLER_VERSION.into()])
                .with_agent(id));
            }
            versions.insert(id.to_string(), info.version);
        }
        for a in &agents {
            send(ControllerEvent::Agent {
                id: a.id().into(),
                state: AgentState::Ready,
            })
            .await;
        }

        // 2. Бандлы и старт всем параллельно; любой отказ — `stop` уже запущенным.
        let run_id = uuid::Uuid::new_v4().to_string();
        let weights: Vec<(String, u32)> = agents
            .iter()
            .map(|a| (a.id().to_string(), weight_of(&plan.env, a.id())))
            .collect();
        let bundles = bundle::bundles(&plan, &run_id, &weights)?;
        let starts = join_all(
            agents
                .iter()
                .zip(&bundles)
                .map(|(a, b)| a.start(&b.request, &plan.suite_text, &b.data)),
        )
        .await;
        if starts.iter().any(|r| r.is_err()) {
            let started: Vec<Arc<dyn AgentLink>> = agents
                .iter()
                .zip(&starts)
                .filter(|(_, r)| r.is_ok())
                .map(|(a, _)| a.clone())
                .collect();
            stop_all(&started, &run_id).await;
            let mut first = None;
            for (a, r) in agents.iter().zip(starts) {
                if let Err(e) = r {
                    send(ControllerEvent::Agent {
                        id: a.id().into(),
                        state: AgentState::Failed,
                    })
                    .await;
                    first.get_or_insert(EngineError::from_link(e, a.id()));
                }
            }
            return Err(first.expect("at least one start failed"));
        }

        // 3. Папка прогона и `run.yaml`; не создалась — нагрузку останавливаем.
        let started_at = OffsetDateTime::now_utc();
        let dir = plan.out_dir.clone().unwrap_or_else(|| {
            plan.root
                .join(RUNS_DIR)
                .join(run_dir_name(started_at, plan.mode == Mode::Smoke))
        });
        let meta = RunMeta {
            run_id: run_id.clone(),
            suite: plan.suite_name.clone(),
            profile: plan.profile.clone(),
            env: plan.env.id.clone(),
            mode: plan.mode,
            agents: agents
                .iter()
                .map(|a| RunAgent {
                    id: a.id().into(),
                    url: plan
                        .env
                        .agents
                        .iter()
                        .find(|x| x.id == a.id())
                        .map(|x| x.url.clone()),
                    weight: weight_of(&plan.env, a.id()),
                    version: versions.get(a.id()).cloned(),
                })
                .collect(),
            project_commit: project_commit(&plan.root),
            services: service_versions(&plan.root),
            started_at: started_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| now_rfc3339()),
            ended_at: None,
            status: None,
        };
        let writer = match RunWriter::create(&dir, &meta) {
            Ok(w) => w,
            Err(e) => {
                stop_all(&agents, &run_id).await;
                return Err(EngineError::io(format!(
                    "cannot create the run folder: {e}"
                )));
            }
        };
        send(ControllerEvent::Started {
            run_id: run_id.clone(),
            dir: dir.display().to_string(),
        })
        .await;
        for a in &agents {
            send(ControllerEvent::Agent {
                id: a.id().into(),
                state: AgentState::Running,
            })
            .await;
        }

        // 4. Задача на агента и сводный цикл. `JoinSet` снимает задачи, если прогон бросили.
        let (tx, mut rx) = mpsc::channel::<(String, AgentMsg)>(1024);
        let mut tasks = JoinSet::new();
        for a in &agents {
            tasks.spawn(agent_task(a.clone(), run_id.clone(), tx.clone()));
        }
        drop(tx);
        let ids: Vec<String> = agents.iter().map(|a| a.id().to_string()).collect();
        let mut col = Collector {
            writer,
            merger: TickMerger::new(ids.clone(), MERGE_HOLD),
            acc: Accumulator::default(),
            samples: SampleCap::new(),
            events: events.clone(),
            write_err: None,
        };
        let mut states: BTreeMap<String, AgentState> = ids
            .iter()
            .map(|id| (id.clone(), AgentState::Running))
            .collect();
        let mut summaries: BTreeMap<String, AgentSummary> = BTreeMap::new();
        let mut gap = false;
        let mut cancelled = false;
        let mut stop_sent = false;
        let mut expire = tokio::time::interval(EXPIRE_EVERY);
        loop {
            if col.write_err.is_some() && !stop_sent {
                // Писать некуда — нагрузку без записи не продолжаем.
                stop_sent = true;
                stop_all(&agents, &run_id).await;
            }
            // `biased`: отмена проверяется первой, затем события агентов, затем таймер буфера.
            tokio::select! {
                biased;
                _ = cancel.cancelled(), if !stop_sent => {
                    stop_sent = true;
                    cancelled = true;
                    let running: Vec<Arc<dyn AgentLink>> = agents
                        .iter()
                        .filter(|a| states.get(a.id()) == Some(&AgentState::Running))
                        .cloned()
                        .collect();
                    stop_all(&running, &run_id).await;
                }
                msg = rx.recv() => {
                    let Some((id, msg)) = msg else { break };
                    match msg {
                        AgentMsg::Lost => {
                            states.insert(id.clone(), AgentState::Lost);
                            col.drop_agent(&id).await;
                            send(ControllerEvent::Agent { id, state: AgentState::Lost }).await;
                        }
                        AgentMsg::Event(Event::Tick(t)) => col.push(&id, t).await,
                        AgentMsg::Event(Event::Phase(p)) => {
                            // Фазы — от первого агента списка, а если он потерян — от первого живого.
                            let source = ids.iter().find(|x| states.get(*x) != Some(&AgentState::Lost));
                            if source == Some(&id) {
                                col.phase(p).await;
                            }
                        }
                        AgentMsg::Event(Event::Error(e)) => col.error(e).await,
                        AgentMsg::Event(Event::Gap) => {
                            tracing::warn!(agent = %id, "events were lost: the run is incomplete");
                            gap = true;
                        }
                        AgentMsg::Event(Event::Done(s)) => {
                            states.insert(id.clone(), AgentState::Done);
                            summaries.insert(id.clone(), s);
                            col.drop_agent(&id).await;
                            send(ControllerEvent::Agent { id, state: AgentState::Done }).await;
                        }
                    }
                }
                _ = expire.tick() => col.expire().await,
            }
        }
        col.flush().await;
        drop(tasks);
        if let Some(e) = col.write_err {
            return Err(e);
        }

        // 5. Файлы агентов, слияние гистограмм `hist.bin`, сводка.
        let agents_dir = dir.join(AGENTS_DIR);
        let collected = join_all(agents.iter().map(|a| {
            let lost = states.get(a.id()) == Some(&AgentState::Lost);
            collect_files(a.clone(), run_id.clone(), lost, agents_dir.join(a.id()))
        }))
        .await;
        let mut incomplete = gap
            || states.values().any(|s| *s == AgentState::Lost)
            || summaries
                .values()
                .any(|s| s.status == RunStatus::Incomplete);
        let mut totals = Totals::default();
        for (a, part) in agents.iter().zip(collected) {
            match part.and_then(|p| merge_totals(&[totals.clone(), p])) {
                Ok(merged) => totals = merged,
                Err(e) => {
                    tracing::warn!(agent = a.id(), "agent files are not collected: {e}");
                    incomplete = true;
                }
            }
        }
        col.acc.fill_counters(&mut totals);
        let stopped = cancelled || summaries.values().any(|s| s.status == RunStatus::Stopped);
        let failed = summaries.values().any(|s| s.status == RunStatus::Failed);
        let status = if incomplete {
            RunStatus::Incomplete
        } else if stopped {
            RunStatus::Stopped
        } else if failed {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        };
        let summary = summary(&totals, &bundle::targets_of(&plan), status);
        col.writer
            .finish(&summary, status)
            .map_err(|e| EngineError::io(format!("cannot finish the run files: {e}")))?;
        let result = RunResult {
            run_id,
            dir: dir.display().to_string(),
            status,
            summary,
        };
        send(ControllerEvent::Done(result.clone())).await;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_of_reads_the_manifest_and_defaults_to_one() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = bundle::tests::plan(tmp.path().to_path_buf());
        assert_eq!(weight_of(&plan.env, "a"), 1);
        assert_eq!(weight_of(&plan.env, "b"), 2);
        assert_eq!(
            weight_of(&plan.env, "local"),
            1,
            "agent outside the manifest"
        );
        let mut env = plan.env.clone();
        env.agents[1].weight = 0;
        assert_eq!(weight_of(&env, "b"), 1, "weight 0 counts as 1");
    }

    #[test]
    fn service_versions_without_registry_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(service_versions(tmp.path()).is_empty());
        std::fs::create_dir_all(tmp.path().join("registry")).unwrap();
        std::fs::write(tmp.path().join(REGISTRY_FILE), ": not yaml [").unwrap();
        assert!(service_versions(tmp.path()).is_empty());
    }
}
