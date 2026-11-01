//! Локальный прогон (спека L2 §4.6): проект → `RunRequest` → `LocalAgent` → `runs/<дата>/`.
//! Путь тот же, что у контроллера с HTTP-агентами: start, events, stop, files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use glafira_load_model::prod_load::{PROD_LOAD_FILE, ProdLoad};
use glafira_load_model::profile::{Computed, PROFILE_FILE, ProfileError, ProfileFile, compute};
use glafira_load_model::project::{Environment, MANIFEST_FILE, Manifest};
use glafira_load_model::registry::{REGISTRY_FILE, Registry, Status};
use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::check::{DataInfo, Level, check};
use glafira_load_model::suite::{self, Action, Encoding, SUITE_FILE, SUITES_DIR, Suite};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

use crate::agent_api::{
    AgentSummary, EnvAddresses, ErrorSample, Event, Mode, RunRequest, RunStatus,
};
use crate::agent_link::{
    AgentLink, AgentState, ControllerEvent, DRAIN, EngineError, ExecutorFactory, LocalAgent,
    RunResult, VERSION, secrets_from_env,
};
use crate::context::{RunContext, data_info, is_inside_project};
use crate::executor::{Executors, StepExecutor};
use crate::http::HttpExecutor;
use crate::kafka::{KafkaExecutor, topic_of};
use crate::run_files::{
    AGENTS_DIR, RUNS_DIR, RunAgent, RunMeta, RunWriter, now_rfc3339, run_dir_name,
};
use crate::summary::SmokeReport;

/// Id встроенного агента в `run.yaml` и `agents/<id>/`.
pub const LOCAL_AGENT_ID: &str = "local";

#[derive(Debug, Clone)]
pub struct LocalRun {
    pub root: PathBuf,
    pub suite: String,
    /// Пусто в режиме `smoke`: цели не считаются.
    pub profile: String,
    pub env: String,
    pub mode: Mode,
    pub max_in_flight: Option<u32>,
    /// Папка результата; по умолчанию `runs/<дата>/` в проекте.
    pub out: Option<PathBuf>,
}

/// Файлы проекта, нужные прогону.
#[derive(Debug, Clone)]
pub struct Project {
    pub manifest: Manifest,
    pub registry: Registry,
    pub prod: ProdLoad,
    pub profiles: ProfileFile,
}

fn read_yaml<T: serde::de::DeserializeOwned>(root: &Path, rel: &str) -> Result<T, EngineError> {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| EngineError::new("IO", format!("{}: {e}", path.display())))?;
    glafira_load_model::from_yaml(&text).map_err(|e| {
        let at = e.line.map(|l| format!(":{l}")).unwrap_or_default();
        EngineError::new("INVALID_INPUT", format!("{rel}{at}: {}", e.message))
    })
}

fn read_yaml_or_default<T: serde::de::DeserializeOwned + Default>(
    root: &Path,
    rel: &str,
) -> Result<T, EngineError> {
    if root.join(rel).is_file() {
        read_yaml(root, rel)
    } else {
        Ok(T::default())
    }
}

pub fn load_project(root: &Path) -> Result<Project, EngineError> {
    if !root.join(MANIFEST_FILE).is_file() {
        return Err(EngineError::new(
            "NOT_FOUND",
            format!("{} has no {MANIFEST_FILE}", root.display()),
        ));
    }
    Ok(Project {
        manifest: read_yaml(root, MANIFEST_FILE)?,
        registry: read_yaml_or_default(root, REGISTRY_FILE)?,
        prod: read_yaml_or_default(root, PROD_LOAD_FILE)?,
        profiles: read_yaml_or_default(root, PROFILE_FILE)?,
    })
}

/// Сьют после `check` и байты его наборов данных.
#[derive(Debug, Clone)]
pub struct LoadedSuite {
    pub name: String,
    pub text: String,
    pub suite: Suite,
    pub data: BTreeMap<String, Vec<u8>>,
}

/// Имя сьюта — имя папки в `suites/`: `[a-z0-9-]{1,64}`, так что `..` и `/` из `suites/` не выводят.
fn valid_suite_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Читает `suites/<name>/suite.yaml`, данные с диска и выполняет `check`; ошибки — `SUITE_INVALID`
/// со списком `file:line: message` в `names`, недопустимое имя — `INVALID_INPUT`.
pub fn load_suite(
    root: &Path,
    name: &str,
    registry: &Registry,
    env: Option<&Environment>,
) -> Result<LoadedSuite, EngineError> {
    if !valid_suite_name(name) {
        return Err(EngineError::new(
            "INVALID_INPUT",
            format!("suite name `{name}` must match [a-z0-9-]{{1,64}}"),
        ));
    }
    let rel = format!("{SUITES_DIR}/{name}/{SUITE_FILE}");
    let path = root.join(&rel);
    if !path.is_file() {
        return Err(EngineError::new(
            "NOT_FOUND",
            format!("suite `{name}` not found: {}", path.display()),
        ));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| EngineError::new("IO", format!("{}: {e}", path.display())))?;
    let mut data = BTreeMap::new();
    let mut infos: BTreeMap<String, Option<DataInfo>> = BTreeMap::new();
    // Ошибки разбора покажет `check`; здесь нужны только пути наборов.
    if let Ok(parsed) = suite::parse(&text) {
        for (set, spec) in &parsed.data {
            // Путь вне проекта — `PATH_OUTSIDE_PROJECT` сообщит `check`; файл не читаем.
            // Испорченный файл `data_info` отдаёт как `None`: `check` сообщит о нём как об
            // отсутствующем наборе, и прогон не начнётся.
            infos.insert(set.clone(), data_info(root, &spec.path));
            if is_inside_project(&spec.path)
                && let Ok(bytes) = std::fs::read(root.join(&spec.path))
            {
                data.insert(set.clone(), bytes);
            }
        }
    }
    let (parsed, findings) = check(&text, &rel, registry, &infos, env);
    let errors: Vec<String> = findings
        .iter()
        .filter(|f| f.level == Level::Error)
        .map(|f| match f.line {
            Some(l) => format!("{}:{l}: {}", f.file, f.message),
            None => format!("{}: {}", f.file, f.message),
        })
        .collect();
    let Some(suite) = parsed.filter(|_| errors.is_empty()) else {
        let count = errors.len().max(1);
        return Err(
            EngineError::new("SUITE_INVALID", format!("{count} error(s) in {rel}"))
                .with_names(errors),
        );
    };
    Ok(LoadedSuite {
        name: name.into(),
        text,
        suite,
        data,
    })
}

/// Расчёт профиля `name` по реестру и прод-нагрузке проекта; неизвестный профиль — `NOT_FOUND`,
/// прочие ошибки расчёта — `INVALID_INPUT`.
pub fn compute_profile(project: &Project, name: &str) -> Result<Computed, EngineError> {
    compute(&project.profiles, name, &project.registry, &project.prod).map_err(|e| {
        let code = match e {
            ProfileError::UnknownProfile(_) => "NOT_FOUND",
            _ => "INVALID_INPUT",
        };
        EngineError::new(code, e.to_string()).with_names(vec![name.to_string()])
    })
}

/// Цели операций из расчёта профиля.
pub fn targets_of(computed: &Computed) -> BTreeMap<String, f64> {
    computed
        .targets
        .iter()
        .map(|t| (t.id.clone(), t.target))
        .collect()
}

/// Тексты `.avsc` для `kafka_produce` с `encoding: avro`: путь из `kafka.topics` сервиса в манифесте,
/// иначе `source.path` операции; относительно клона сервиса.
pub fn avro_schema_texts(
    root: &Path,
    manifest: &Manifest,
    registry: &Registry,
    suite: &Suite,
) -> Result<BTreeMap<String, String>, EngineError> {
    let mut out = BTreeMap::new();
    for step in suite.scenarios.iter().flat_map(|s| s.steps.iter()) {
        let Action::KafkaProduce(p) = &step.action else {
            continue;
        };
        if p.encoding != Encoding::Avro || out.contains_key(&p.operation) {
            continue;
        }
        let op = registry.get(&p.operation).ok_or_else(|| {
            EngineError::new(
                "SUITE_INVALID",
                format!("operation `{}` is not in the registry", p.operation),
            )
        })?;
        let service = manifest.service(&op.draft.service).ok_or_else(|| {
            EngineError::new(
                "NOT_FOUND",
                format!(
                    "service `{}` of `{}` is not in the manifest",
                    op.draft.service, p.operation
                ),
            )
        })?;
        let topic = topic_of(&p.operation).unwrap_or_default();
        let rel = service
            .kafka
            .topics
            .get(topic)
            .map(|t| t.schema().to_string())
            .unwrap_or_else(|| op.draft.source.path.clone());
        let base = if Path::new(&service.path).is_absolute() {
            PathBuf::from(&service.path)
        } else {
            root.join(&service.path)
        };
        let path = base.join(&rel);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            EngineError::new(
                "IO",
                format!(
                    "Avro schema of `{}` at {}: {e}",
                    p.operation,
                    path.display()
                ),
            )
        })?;
        out.insert(p.operation.clone(), text);
    }
    Ok(out)
}

/// Коммит проекта из `.git` без запуска git: `HEAD` → ссылка → файл ссылки или `packed-refs`.
pub fn project_commit(root: &Path) -> Option<String> {
    let git = root.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref: ") else {
        return Some(head.to_string());
    };
    if let Ok(hash) = std::fs::read_to_string(git.join(reference)) {
        return Some(hash.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (hash, name) = line.split_once(' ')?;
        (name == reference).then(|| hash.to_string())
    })
}

/// Сервис → версия из `last_seen` активных операций реестра.
pub fn service_versions(registry: &Registry) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for op in registry
        .operations
        .iter()
        .filter(|o| o.status == Status::Active)
    {
        if !op.last_seen.version.is_empty() {
            out.insert(op.draft.service.clone(), op.last_seen.version.clone());
        }
    }
    out
}

/// Схемы ответов операций реестра (`id → SchemaSummary`) — поле `RunRequest.response_schemas`.
pub fn response_schemas_of(registry: &Registry) -> BTreeMap<String, SchemaSummary> {
    registry
        .operations
        .iter()
        .filter_map(|o| o.draft.response.clone().map(|s| (o.draft.id.clone(), s)))
        .collect()
}

/// Фабрика исполнителей для `LocalAgent` (локальный прогон и агент Task 10): HTTP со схемами ответов
/// из `RunRequest.response_schemas`; Kafka — если в сьюте есть шаги Kafka, со схемами `.avsc` из
/// `RunRequest.avro_schemas`, значениями секретов из контекста (маскирование) и схемами сообщений
/// из `RunRequest.response_schemas` (`expect.schema: response` у `kafka_consume`; ключи — id операций,
/// так что карта передаётся целиком). Реестр проекта фабрике не нужен — всё приходит в запросе.
pub fn executor_factory() -> ExecutorFactory {
    Arc::new(|req: RunRequest, suite: Suite, ctx: Arc<RunContext>| {
        Box::pin(async move {
            let steps = || suite.scenarios.iter().flat_map(|s| s.steps.iter());
            let topics: Vec<String> = steps()
                .filter_map(|st| match &st.action {
                    Action::KafkaConsume(c) => topic_of(&c.operation).map(str::to_string),
                    _ => None,
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let needs_kafka = steps()
                .any(|st| matches!(st.action, Action::KafkaProduce(_) | Action::KafkaConsume(_)));
            let secrets = ctx.secret_values();
            let http = HttpExecutor::new(
                req.env.http.as_deref().unwrap_or(""),
                &suite.defaults.http,
                &secrets,
            )
            .map_err(|m| EngineError::new("INVALID_INPUT", m))?
            .with_schemas(req.response_schemas.clone());
            let kafka: Option<Box<dyn StepExecutor>> = if needs_kafka {
                let bootstrap = req.env.kafka.clone().ok_or_else(|| {
                    EngineError::new("INVALID_INPUT", "the environment has no kafka address")
                })?;
                // Маскирование и схемы сообщений `kafka_consume` включаются только явными вызовами.
                let executor = KafkaExecutor::new(
                    &bootstrap,
                    &req.kafka,
                    &topics,
                    &req.run_id,
                    &req.agent_id,
                    req.env.schema_registry.clone(),
                    &req.avro_schemas,
                )
                .await
                .map_err(|m| EngineError::new("KAFKA_CONSUME", m))?
                .with_secrets(&secrets)
                .with_response_schemas(req.response_schemas.clone());
                Some(Box::new(executor))
            } else {
                None
            };
            Ok(Executors { http, kafka })
        })
    })
}

/// Запуск агента, чтение событий, запись тиков и сбор файлов; остановка по `cancel`.
async fn drive(
    agent: &LocalAgent,
    request: &RunRequest,
    loaded: &LoadedSuite,
    writer: &mut RunWriter,
    events: &Sender<ControllerEvent>,
    cancel: &CancellationToken,
) -> Result<(AgentSummary, Vec<ErrorSample>), EngineError> {
    let smoke = request.mode == Mode::Smoke;
    agent
        .start(request, &loaded.text, &loaded.data)
        .await
        .map_err(|e| EngineError::from(e).with_agent(LOCAL_AGENT_ID))?;
    let _ = events
        .send(ControllerEvent::Agent {
            id: LOCAL_AGENT_ID.into(),
            state: AgentState::Running,
        })
        .await;
    let mut stream = agent.events(&request.run_id, None).await?;
    let mut errors = Vec::new();
    let mut stopped = false;
    let summary = loop {
        let next = tokio::select! {
            _ = cancel.cancelled(), if !stopped => {
                stopped = true;
                agent.stop(&request.run_id).await?;
                continue;
            }
            next = stream.next() => next,
        };
        let Some((_, event)) = next else {
            return Err(
                EngineError::new("AGENT_UNREACHABLE", "local agent ended without `done`")
                    .with_agent(LOCAL_AGENT_ID),
            );
        };
        match event {
            Event::Tick(t) => {
                // Смок — без графиков и metrics.jsonl (решение плана 5).
                if !smoke {
                    writer.tick(&t)?;
                    let _ = events.send(ControllerEvent::Tick(t)).await;
                }
            }
            Event::Phase(p) => {
                writer.phase(&p)?;
                let _ = events.send(ControllerEvent::Phase(p)).await;
            }
            Event::Error(e) => {
                // Предел примеров держит агент (`SampleCap`), писатель — вторая защита.
                writer.error(&e)?;
                // Примеры в памяти нужны только отчёту смока.
                if smoke {
                    errors.push(e.clone());
                }
                let _ = events.send(ControllerEvent::Error(e)).await;
            }
            Event::Done(s) => break s,
            Event::Gap => {}
        }
    };
    let tar = agent.files(&request.run_id).await?;
    let agent_dir = writer.dir().join(AGENTS_DIR).join(LOCAL_AGENT_ID);
    std::fs::create_dir_all(&agent_dir)?;
    tar::Archive::new(&tar[..]).unpack(&agent_dir)?;
    let _ = events
        .send(ControllerEvent::Agent {
            id: LOCAL_AGENT_ID.into(),
            state: AgentState::Done,
        })
        .await;
    Ok((summary, errors))
}

/// Локальный прогон или смок проекта. Ошибки до создания папки — коды `NOT_FOUND`, `IO`,
/// `INVALID_INPUT`, `SUITE_INVALID`, `SECRET_MISSING`; после — папка получает `status: failed`.
pub async fn run_local(
    req: LocalRun,
    events: Sender<ControllerEvent>,
    cancel: CancellationToken,
) -> Result<RunResult, EngineError> {
    let root = req.root.clone();
    let project = load_project(&root)?;
    let env = project
        .manifest
        .environments
        .iter()
        .find(|e| e.id == req.env)
        .cloned()
        .ok_or_else(|| {
            EngineError::new(
                "NOT_FOUND",
                format!("environment `{}` is not in the manifest", req.env),
            )
        })?;
    let loaded = load_suite(&root, &req.suite, &project.registry, Some(&env))?;
    let smoke = req.mode == Mode::Smoke;
    let computed = if smoke && req.profile.is_empty() {
        None
    } else {
        Some(compute_profile(&project, &req.profile)?)
    };
    let secret_names = loaded.suite.secrets.clone();
    secrets_from_env(&secret_names).map_err(|names| {
        EngineError::new(
            "SECRET_MISSING",
            "secrets are missing in the environment (GLAFIRA_LOAD_SECRET_*)",
        )
        .with_names(names)
    })?;
    let avro = avro_schema_texts(&root, &project.manifest, &project.registry, &loaded.suite)?;

    let run_id = uuid::Uuid::new_v4().to_string();
    let request = RunRequest {
        run_id: run_id.clone(),
        agent_id: LOCAL_AGENT_ID.into(),
        suite_name: loaded.name.clone(),
        controller_version: VERSION.into(),
        suite_format: loaded.suite.format,
        env: EnvAddresses {
            http: env.http.clone(),
            kafka: env.kafka.clone(),
            schema_registry: env.schema_registry.clone(),
        },
        secrets: secret_names,
        targets: computed.as_ref().map(targets_of).unwrap_or_default(),
        timeline: computed
            .as_ref()
            .map(|c| c.timeline.clone())
            .unwrap_or_default(),
        total_minutes: computed.as_ref().map(|c| c.total_minutes).unwrap_or(0.0),
        max_in_flight: req
            .max_in_flight
            .unwrap_or(loaded.suite.defaults.max_in_flight),
        arrival: loaded.suite.defaults.arrival,
        mode: req.mode,
        kafka: loaded.suite.defaults.kafka.clone(),
        response_schemas: response_schemas_of(&project.registry),
        avro_schemas: avro,
    };
    let started = OffsetDateTime::now_utc();
    let dir = req
        .out
        .clone()
        .unwrap_or_else(|| root.join(RUNS_DIR).join(run_dir_name(started, smoke)));
    let meta = RunMeta {
        run_id: run_id.clone(),
        suite: loaded.name.clone(),
        profile: req.profile.clone(),
        env: env.id.clone(),
        mode: req.mode,
        agents: vec![RunAgent {
            id: LOCAL_AGENT_ID.into(),
            url: None,
            weight: 1,
            version: Some(VERSION.into()),
        }],
        project_commit: project_commit(&root),
        services: service_versions(&project.registry),
        // Тот же момент, что в имени папки: `started_at` и папка не расходятся на границе секунды.
        started_at: started.format(&Rfc3339).unwrap_or_else(|_| now_rfc3339()),
        ended_at: None,
        status: None,
    };
    let mut writer = RunWriter::create(&dir, &meta)?;
    let dir_text = dir.display().to_string();
    let _ = events
        .send(ControllerEvent::Started {
            run_id: run_id.clone(),
            dir: dir_text.clone(),
        })
        .await;

    let agent = LocalAgent::new(LOCAL_AGENT_ID, executor_factory());
    match drive(&agent, &request, &loaded, &mut writer, &events, &cancel).await {
        Ok((summary, errors)) => {
            if smoke {
                writer.smoke(&SmokeReport::from_errors(&loaded.suite, &errors))?;
            }
            let status = summary.status;
            writer.finish(&summary, status)?;
            let result = RunResult {
                run_id,
                dir: dir_text,
                status,
                summary,
            };
            let _ = events.send(ControllerEvent::Done(result.clone())).await;
            Ok(result)
        }
        Err(e) => {
            // Нагрузка не должна идти дальше без слушателя: остановить и дождаться конца
            // (не дольше доработки и запаса), прежде чем вернуть ошибку.
            if agent.stop(&run_id).await.is_ok() {
                let wait = DRAIN + std::time::Duration::from_secs(2);
                if tokio::time::timeout(wait, agent.files(&run_id))
                    .await
                    .is_err()
                {
                    tracing::error!("local run {run_id} did not stop within {wait:?}");
                }
            }
            let failed = AgentSummary {
                status: RunStatus::Failed,
                ops: vec![],
                scenarios: vec![],
                seconds: 0,
            };
            writer.finish(&failed, RunStatus::Failed)?;
            let _ = events
                .send(ControllerEvent::Agent {
                    id: LOCAL_AGENT_ID.into(),
                    state: AgentState::Failed,
                })
                .await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::extract::Path as AxPath;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use glafira_load_model::prod_load::Figures;
    use glafira_load_model::profile::{Basis, Phase, Profile};
    use glafira_load_model::project::{EnvKind, Service};
    use glafira_load_model::registry::{
        Address, Manual, OpKind, Operation, OperationDraft, Seen, Source,
    };
    use glafira_load_model::to_yaml;
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::*;

    const SUITE: &str = include_str!("../../../dev/fixtures/suites/orders-http/suite.yaml");

    /// Стенд-заглушка: `POST /v1/orders` → 201 с id, `GET /v1/orders/{id}` → 200.
    async fn stub() -> String {
        let app = Router::new()
            .route(
                "/v1/orders",
                post(|| async {
                    (
                        StatusCode::CREATED,
                        Json(json!({"id": "o-1", "status": "new"})),
                    )
                }),
            )
            .route(
                "/v1/orders/{id}",
                get(|AxPath(id): AxPath<String>| async move {
                    Json(json!({"id": id, "status": "new"}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Проект во временной папке: стенд `test` на заглушку, две HTTP-операции по 20 RPS, профиль
    /// `quick` — скачок до 100 % и полка `steady_minutes`.
    fn project(http: &str, steady_minutes: f64) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut manifest = Manifest::new("orders-platform");
        manifest.services.push(Service {
            id: "orders".into(),
            name: String::new(),
            path: ".".into(),
            contracts: vec![],
            kafka: Default::default(),
            extra: Default::default(),
        });
        manifest.environments.push(Environment {
            id: "test".into(),
            kind: EnvKind::Test,
            http: Some(http.into()),
            kafka: None,
            grpc: None,
            sql: None,
            schema_registry: None,
            agents: vec![],
            extra: Default::default(),
        });
        write(root, MANIFEST_FILE, &to_yaml(&manifest));
        let mut registry = Registry::default();
        for (method, path) in [("POST", "/v1/orders"), ("GET", "/v1/orders/{id}")] {
            let draft = OperationDraft::new(
                "orders",
                OpKind::Http,
                Address::Http {
                    method: method.into(),
                    path: path.into(),
                },
                Source::manual(),
            );
            registry.operations.push(Operation {
                fingerprint: draft.fingerprint(),
                draft,
                status: Status::Active,
                first_seen: Seen::default(),
                last_seen: Seen {
                    version: "1.2.0".into(),
                    ..Default::default()
                },
                manual: Manual::default(),
            });
        }
        write(root, REGISTRY_FILE, &to_yaml(&registry));
        let mut prod = ProdLoad::default();
        for id in [
            "http:orders:POST /v1/orders",
            "http:orders:GET /v1/orders/{id}",
        ] {
            prod.operations.insert(
                id.into(),
                Figures {
                    avg: 20.0,
                    peak: 40.0,
                    ..Default::default()
                },
            );
        }
        write(root, PROD_LOAD_FILE, &to_yaml(&prod));
        let mut profiles = ProfileFile::default();
        profiles.profiles.insert(
            "quick".into(),
            Profile {
                basis: Basis::Avg,
                phases: vec![
                    Phase::Ramp {
                        minutes: 0.0,
                        to_pct: 100.0,
                    },
                    Phase::Steady {
                        minutes: steady_minutes,
                    },
                ],
            },
        );
        write(root, PROFILE_FILE, &to_yaml(&profiles));
        write(root, "suites/orders-http/suite.yaml", SUITE);
        dir
    }

    fn request(root: &Path, profile: &str, mode: Mode, out: Option<PathBuf>) -> LocalRun {
        LocalRun {
            root: root.to_path_buf(),
            suite: "orders-http".into(),
            profile: profile.into(),
            env: "test".into(),
            mode,
            max_in_flight: None,
            out,
        }
    }

    #[test]
    fn project_loading_and_helpers() {
        let tmp = project("http://127.0.0.1:1", 1.0);
        let p = load_project(tmp.path()).unwrap();
        assert_eq!(
            (
                p.manifest.name.as_str(),
                p.registry.operations.len(),
                p.prod.operations.len()
            ),
            ("orders-platform", 2, 2)
        );
        assert!(p.profiles.profiles.contains_key("quick"));
        assert_eq!(
            service_versions(&p.registry),
            BTreeMap::from([("orders".to_string(), "1.2.0".to_string())])
        );
        assert_eq!(
            load_project(&tmp.path().join("nope")).unwrap_err().code,
            "NOT_FOUND"
        );

        let env = p.manifest.environments[0].clone();
        let loaded = load_suite(tmp.path(), "orders-http", &p.registry, Some(&env)).unwrap();
        assert_eq!(
            (
                loaded.name.as_str(),
                loaded.suite.scenarios.len(),
                loaded.data.len()
            ),
            ("orders-http", 1, 0)
        );
        assert_eq!(
            load_suite(tmp.path(), "missing", &p.registry, Some(&env))
                .unwrap_err()
                .code,
            "NOT_FOUND"
        );
        write(
            tmp.path(),
            "suites/broken/suite.yaml",
            &SUITE.replace("http:orders:GET /v1/orders/{id}", "http:orders:GET /nope"),
        );
        let invalid = load_suite(tmp.path(), "broken", &p.registry, Some(&env)).unwrap_err();
        assert_eq!(invalid.code, "SUITE_INVALID");
        assert!(
            invalid
                .names
                .iter()
                .any(|n| n.contains("suites/broken/suite.yaml")),
            "{invalid:?}"
        );
        // Имя сьюта — только `[a-z0-9-]{1,64}`: из `suites/` не выйти.
        write(tmp.path(), "x/suite.yaml", SUITE);
        for bad in ["../x", "a/b", "Orders", "", &"a".repeat(65)] {
            assert_eq!(
                load_suite(tmp.path(), bad, &p.registry, Some(&env))
                    .unwrap_err()
                    .code,
                "INVALID_INPUT",
                "{bad}"
            );
        }
        // Испорченный файл данных `check` видит как отсутствующий набор.
        write(
            tmp.path(),
            "suites/bad-data/suite.yaml",
            &SUITE.replace(
                "name: orders-http\n",
                "name: bad-data\ndata:\n  users: data/users.csv\n",
            ),
        );
        write(tmp.path(), "data/users.csv", "id,name\n1\n");
        let bad_data = load_suite(tmp.path(), "bad-data", &p.registry, Some(&env)).unwrap_err();
        assert_eq!(bad_data.code, "SUITE_INVALID", "{bad_data:?}");
        write(tmp.path(), "data/users.csv", "id,name\n1,Ann\n");
        let good = load_suite(tmp.path(), "bad-data", &p.registry, Some(&env)).unwrap();
        assert_eq!(good.data["users"], b"id,name\n1,Ann\n");

        let computed = compute(&p.profiles, "quick", &p.registry, &p.prod).unwrap();
        assert_eq!(targets_of(&computed)["http:orders:POST /v1/orders"], 20.0);
        assert!(
            avro_schema_texts(tmp.path(), &p.manifest, &p.registry, &loaded.suite)
                .unwrap()
                .is_empty()
        );
        assert!(
            response_schemas_of(&p.registry).is_empty(),
            "у операций реестра теста нет схем ответов"
        );
        let mut with_schema = p.registry.clone();
        with_schema.operations[0].draft.response =
            Some(glafira_load_model::schema::SchemaSummary::default());
        assert_eq!(
            response_schemas_of(&with_schema).keys().collect::<Vec<_>>(),
            [&with_schema.operations[0].draft.id]
        );

        std::fs::create_dir_all(tmp.path().join(".git/refs/heads")).unwrap();
        std::fs::write(tmp.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(tmp.path().join(".git/refs/heads/main"), "abc123\n").unwrap();
        assert_eq!(project_commit(tmp.path()).as_deref(), Some("abc123"));
        std::fs::write(tmp.path().join(".git/HEAD"), "deadbeef\n").unwrap();
        assert_eq!(project_commit(tmp.path()).as_deref(), Some("deadbeef"));
        assert_eq!(project_commit(&tmp.path().join("nope")), None);
    }

    /// Запрос для фабрики исполнителей: стенд без брокера (`127.0.0.1:1`), секрет `API_TOKEN`.
    fn factory_request(kafka: Option<&str>, http: &str) -> RunRequest {
        RunRequest {
            run_id: "r1".into(),
            agent_id: LOCAL_AGENT_ID.into(),
            suite_name: "s".into(),
            controller_version: VERSION.into(),
            suite_format: suite::FORMAT,
            env: EnvAddresses {
                http: Some(http.into()),
                kafka: kafka.map(str::to_string),
                schema_registry: None,
            },
            secrets: vec!["API_TOKEN".into()],
            targets: BTreeMap::new(),
            timeline: vec![],
            total_minutes: 0.0,
            max_in_flight: 10,
            arrival: suite::Arrival::Constant,
            mode: Mode::Smoke,
            kafka: BTreeMap::new(),
            response_schemas: BTreeMap::new(),
            avro_schemas: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn executor_factory_wires_secrets_into_kafka_and_checks_addresses() {
        let suite = suite::parse(
            "format: 1\nname: s\nsecrets: [API_TOKEN]\nscenarios:\n  - id: a\n    steps:\n      - id: send\n        kafka_produce:\n          operation: \"kafka_produce:orders.paid\"\n          key: k\n          value_text: \"token {{secret API_TOKEN}}\"\n          encoding: avro\n",
        )
        .unwrap();
        let ctx = Arc::new(RunContext {
            run_id: "r1".into(),
            agent_id: LOCAL_AGENT_ID.into(),
            secrets: BTreeMap::from([("API_TOKEN".to_string(), "s3cr3t".to_string())]),
            ..RunContext::default()
        });
        let factory = executor_factory();
        let executors = factory(
            factory_request(Some("127.0.0.1:1"), "http://127.0.0.1:1"),
            suite.clone(),
            ctx.clone(),
        )
        .await
        .unwrap();
        assert!(executors.kafka.is_some(), "в сьюте есть шаг Kafka");
        let mut it = crate::context::IterationCtx::new(&ctx, 0);
        it.begin_step("send");
        // Avro-схемы нет: ошибка до отправки, пример — отрендеренный `value_text` с секретом.
        let e = executors
            .execute(&suite.scenarios[0].steps[0], &mut it)
            .await
            .unwrap_err();
        assert_eq!(e.code, crate::errors::StepCode::AvroEncode);
        let request = e.request.unwrap_or_default();
        assert!(
            !request.contains("s3cr3t") && request.contains("token ***"),
            "фабрика передаёт секреты в Kafka: {request}"
        );

        let no_kafka = factory(
            factory_request(None, "http://127.0.0.1:1"),
            suite.clone(),
            ctx.clone(),
        )
        .await
        .err()
        .expect("no kafka address");
        assert_eq!(no_kafka.code, "INVALID_INPUT");
        let bad_http = factory(factory_request(None, "not a url"), suite, ctx)
            .await
            .err()
            .expect("bad base url");
        assert_eq!(bad_http.code, "INVALID_INPUT");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn smoke_writes_run_and_smoke_yaml_without_metrics() {
        let http = stub().await;
        let tmp = project(&http, 0.05);
        let (tx, mut rx) = mpsc::channel(256);
        let res = run_local(
            request(tmp.path(), "", Mode::Smoke, None),
            tx,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(res.status, RunStatus::Completed);
        let dir = PathBuf::from(&res.dir);
        assert!(dir.starts_with(tmp.path().join(RUNS_DIR)));
        assert!(
            dir.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("smoke-")
        );
        assert!(dir.join("run.yaml").is_file() && dir.join("smoke.yaml").is_file());
        assert!(
            !dir.join("metrics.jsonl").exists(),
            "смок не пишет metrics.jsonl"
        );
        let smoke: SmokeReport = glafira_load_model::from_yaml(
            &std::fs::read_to_string(dir.join("smoke.yaml")).unwrap(),
        )
        .unwrap();
        assert!(smoke.is_ok(), "{smoke:?}");
        assert_eq!(
            smoke.scenarios[0]
                .steps
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["create", "get"]
        );
        let run: RunMeta =
            glafira_load_model::from_yaml(&std::fs::read_to_string(dir.join("run.yaml")).unwrap())
                .unwrap();
        assert_eq!(run.status, Some(RunStatus::Completed));
        assert!(run.ended_at.is_some() && run.profile.is_empty());
        assert_eq!(
            run.services.get("orders").map(String::as_str),
            Some("1.2.0")
        );
        assert_eq!(run.agents[0].id, LOCAL_AGENT_ID);
        assert!(dir.join("agents/local/summary.yaml").is_file());
        let mut kinds = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            kinds.push(match ev {
                ControllerEvent::Started { .. } => "started",
                ControllerEvent::Agent { .. } => "agent",
                ControllerEvent::Done(_) => "done",
                ControllerEvent::Tick(_) => "tick",
                ControllerEvent::Phase(_) | ControllerEvent::Error(_) => "other",
            });
        }
        assert_eq!(kinds.first().copied(), Some("started"));
        assert_eq!(kinds.last().copied(), Some("done"));
        assert!(!kinds.contains(&"tick"), "в смоке графиков нет: {kinds:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_profile_for_three_seconds_writes_metrics_and_summary() {
        let http = stub().await;
        let tmp = project(&http, 0.05);
        let out = tmp.path().join("out");
        let (tx, mut rx) = mpsc::channel(4096);
        let res = run_local(
            request(tmp.path(), "quick", Mode::Run, Some(out.clone())),
            tx,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(res.status, RunStatus::Completed);
        assert_eq!(PathBuf::from(&res.dir), out);
        let metrics = std::fs::read_to_string(out.join("metrics.jsonl")).unwrap();
        assert!(metrics.lines().count() >= 2, "{metrics}");
        assert!(
            metrics.contains("\"op\":\"http:orders:POST /v1/orders\"")
                && metrics.contains("\"scenario\":\"order-flow\""),
            "{metrics}"
        );
        let summary: AgentSummary = glafira_load_model::from_yaml(
            &std::fs::read_to_string(out.join("summary.yaml")).unwrap(),
        )
        .unwrap();
        let post = summary
            .ops
            .iter()
            .find(|o| o.id == "http:orders:POST /v1/orders")
            .expect("summary of the driving operation");
        assert!(post.n > 0 && post.achieved_pct.is_some(), "{post:?}");
        assert_eq!(post.target, Some(20.0));
        assert_eq!(res.summary, summary);
        assert!(
            out.join("agents/local/hist.bin").is_file()
                && out.join("agents/local/metrics.jsonl").is_file()
        );
        let run: RunMeta =
            glafira_load_model::from_yaml(&std::fs::read_to_string(out.join("run.yaml")).unwrap())
                .unwrap();
        assert_eq!(
            (run.status, run.profile.as_str(), run.mode),
            (Some(RunStatus::Completed), "quick", Mode::Run)
        );
        let mut ticks = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, ControllerEvent::Tick(_)) {
                ticks += 1;
            }
        }
        assert!(ticks >= 2, "ticks = {ticks}");
    }

    /// Ровная полка 6 с на 20 RPS: `summary.yaml` даёт `rps_avg` ≈ цели и `achieved_pct` ≈ 100
    /// (хвостовой срез после доработки секундой нагрузки не считается).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_run_summary_rate_matches_the_target() {
        let http = stub().await;
        let tmp = project(&http, 0.1);
        let out = tmp.path().join("out");
        let (tx, mut rx) = mpsc::channel(4096);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let res = run_local(
            request(tmp.path(), "quick", Mode::Run, Some(out.clone())),
            tx,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        drain.await.unwrap();
        assert_eq!(res.status, RunStatus::Completed);
        let summary: AgentSummary = glafira_load_model::from_yaml(
            &std::fs::read_to_string(out.join("summary.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(summary.seconds, 6);
        let post = summary
            .ops
            .iter()
            .find(|o| o.id == "http:orders:POST /v1/orders")
            .unwrap();
        assert!((post.rps_avg - 20.0).abs() <= 0.4, "{post:?}");
        let achieved = post.achieved_pct.unwrap();
        assert!((achieved - 100.0).abs() <= 2.0, "{achieved}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_stops_the_run_with_status_stopped() {
        let http = stub().await;
        let tmp = project(&http, 10.0);
        let (tx, mut rx) = mpsc::channel(4096);
        let cancel = CancellationToken::new();
        let req = LocalRun {
            max_in_flight: Some(50),
            ..request(tmp.path(), "quick", Mode::Run, None)
        };
        let handle = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_local(req, tx, cancel).await }
        });
        tokio::time::sleep(Duration::from_secs(2)).await;
        cancel.cancel();
        let res = tokio::time::timeout(Duration::from_secs(15), handle)
            .await
            .expect("stops within 15 s")
            .unwrap()
            .unwrap();
        assert_eq!(res.status, RunStatus::Stopped);
        let run: RunMeta = glafira_load_model::from_yaml(
            &std::fs::read_to_string(PathBuf::from(&res.dir).join("run.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(run.status, Some(RunStatus::Stopped));
        rx.close();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn startup_errors_have_codes_and_names() {
        let http = stub().await;
        let tmp = project(&http, 0.05);
        let (tx, _rx) = mpsc::channel(16);
        let missing_env = run_local(
            LocalRun {
                env: "prod".into(),
                ..request(tmp.path(), "quick", Mode::Run, None)
            },
            tx.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_env.code, "NOT_FOUND");
        let missing_profile = run_local(
            request(tmp.path(), "nope", Mode::Run, None),
            tx.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_profile.code, "NOT_FOUND");
        write(
            tmp.path(),
            "suites/secret/suite.yaml",
            &SUITE.replace(
                "name: orders-http\n",
                "name: secret\nsecrets: [ORDERS_TOKEN]\n",
            ),
        );
        let missing_secret = run_local(
            LocalRun {
                suite: "secret".into(),
                ..request(tmp.path(), "quick", Mode::Run, None)
            },
            tx,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (missing_secret.code.as_str(), missing_secret.names),
            ("SECRET_MISSING", vec!["ORDERS_TOKEN".to_string()])
        );
        assert!(
            !tmp.path().join(RUNS_DIR).exists(),
            "папка прогона не создаётся при ошибке запуска"
        );
    }
}
