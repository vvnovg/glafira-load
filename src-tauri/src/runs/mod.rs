//! Прогоны из приложения: план (`load_plan` движка после `check` приложения), агенты стенда или
//! встроенный `LocalAgent`, запуск контроллера в фоновой задаче с событиями в канал, смок через
//! `run_local`, остановка, список и карточка прогона (спека §4.5, §4.6, §6, §7.2).

pub mod rebuild;
pub mod registry;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glafira_load_engine::agent_api::{AgentSummary, Mode};
use glafira_load_engine::agent_link::{AgentLink, HttpAgent, LocalAgent};
use glafira_load_engine::controller::plan::{PlanArgs, load_plan};
use glafira_load_engine::controller::{
    CONTROLLER_VERSION, Controller, ControllerEvent, EngineError, RunPlan, RunResult,
};
use glafira_load_engine::local::{LOCAL_AGENT_ID, LocalRun, executor_factory, run_local};
pub use glafira_load_engine::run_files::{
    AGENTS_DIR, RUN_FILE, RUNS_DIR, RunMeta, SMOKE_FILE, SUMMARY_FILE,
};
use glafira_load_engine::summary::SmokeReport;
use glafira_load_model::from_yaml;
use glafira_load_model::project::Environment;
use glafira_load_model::suite::check::{Finding, Level};
use serde::{Deserialize, Serialize};

use crate::agents::{env_of, token_key};
use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::load::profile::calculate;
use crate::project::manifest::ProjectDir;
use crate::project::paths::{inside, relative, resolve};
use crate::state::AppState;
use crate::suites::files::{has_errors, load_checked, suite_file};

/// Ёмкость канала событий контроллера.
pub const EVENT_BUFFER: usize = 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub suite: String,
    /// В режиме `smoke` не используется: цели смоку не нужны (как `glafira-load smoke`).
    pub profile: String,
    pub env: String,
    pub mode: Mode,
    #[serde(default)]
    pub max_in_flight: Option<u32>,
}

/// Строка списка прогонов: `RunMeta` плюс папка для `run_get` и «Открыть папку».
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListEntry {
    /// `runs/<папка>` относительно проекта.
    pub dir: String,
    #[serde(flatten)]
    pub meta: RunMeta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetails {
    /// `runs/<папка>` относительно проекта.
    pub dir: String,
    pub meta: RunMeta,
    pub summary: Option<AgentSummary>,
    pub smoke: Option<SmokeReport>,
}

/// `check` нашёл ошибки: находки — в `details` как JSON `Vec<Finding>`.
pub fn suite_invalid(name: &str, findings: &[Finding]) -> AppError {
    let count = findings
        .iter()
        .filter(|f| matches!(f.level, Level::Error))
        .count();
    app_err!(
        ErrorCode::SuiteInvalid,
        "errors.suite.invalid",
        "Suite {{name}} has {{count}} error(s)",
        name = name,
        count = count
    )
    .with_details(serde_json::to_string(findings).unwrap_or_default())
}

pub fn token_missing(agent_id: &str) -> AppError {
    app_err!(
        ErrorCode::TokenMissing,
        "errors.agent.tokenMissing",
        "Agent {{id}} has no token in the secret store",
        id = agent_id
    )
}

/// Ошибка движка → ошибка приложения по коду (спека §7.3); неизвестный код — `INTERNAL`
/// с кодом и текстом движка в `details`.
pub fn engine_err(e: EngineError) -> AppError {
    let id = e.agent.clone().unwrap_or_default();
    match e.code.as_str() {
        "SUITE_INVALID" => app_err!(
            ErrorCode::SuiteInvalid,
            "errors.suite.rejected",
            "Suite rejected by agent {{id}}: {{message}}",
            id = id,
            message = e.message
        ),
        "AGENT_UNREACHABLE" => app_err!(
            ErrorCode::AgentUnreachable,
            "errors.agent.unreachable",
            "Agent {{id}} is unreachable: {{message}}",
            id = id,
            message = e.message
        ),
        "UNAUTHORIZED" => app_err!(
            ErrorCode::AgentUnreachable,
            "errors.agent.unauthorized",
            "Agent {{id}} rejected the token",
            id = id
        ),
        "AGENT_BUSY" => app_err!(
            ErrorCode::AgentBusy,
            "errors.agent.busy",
            "Agent {{id}} is busy with another run: {{message}}",
            id = id,
            message = e.message
        ),
        "AGENT_VERSION" => app_err!(
            ErrorCode::AgentVersion,
            "errors.agent.version",
            "Agent {{id}} version does not match the app: {{message}}",
            id = id,
            message = e.message
        ),
        // Локальный прогон (смок, `run_local`) берёт секреты из окружения приложения.
        "SECRET_MISSING" if e.agent.is_none() => app_err!(
            ErrorCode::SecretMissing,
            "errors.run.secretMissing",
            "Set environment variables for secrets before starting the app: {{names}}",
            names = e.names.join(", ")
        ),
        "SECRET_MISSING" => app_err!(
            ErrorCode::SecretMissing,
            "errors.agent.secretMissing",
            "Agent {{id}} has no secrets: {{names}}",
            id = id,
            names = e.names.join(", ")
        ),
        "TOKEN_MISSING" => token_missing(&id),
        _ => AppError::internal_with(format!("{}: {}", e.code, e.message)),
    }
}

/// Ошибка прогона сьюта `suite`: `SUITE_INVALID` без агента — `check` движка в `load_plan` или
/// `run_local`; его находки (`file:line: message` в `names`) уходят в `details` как `Vec<Finding>`,
/// как у `check` приложения. Остальное — `engine_err`.
pub fn local_err(suite: &str, e: EngineError) -> AppError {
    if e.code == "SUITE_INVALID" && e.agent.is_none() {
        let file = suite_file(suite);
        let names = if e.names.is_empty() {
            vec![e.message.clone()]
        } else {
            e.names.clone()
        };
        let findings: Vec<Finding> = names
            .into_iter()
            .map(|message| Finding {
                level: Level::Error,
                file: file.clone(),
                line: None,
                message,
            })
            .collect();
        return suite_invalid(suite, &findings);
    }
    engine_err(e)
}

/// `check` приложения со стендом: ошибки — `SUITE_INVALID` с находками.
fn check_suite(d: &ProjectDir, suite: &str, env: &Environment) -> AppResult<()> {
    let checked = load_checked(d, suite, Some(env))?;
    if checked.suite.is_none() || has_errors(&checked.findings) {
        return Err(suite_invalid(suite, &checked.findings));
    }
    Ok(())
}

/// План прогона: стенд, сьют через `check` приложения (находки в ошибке), профиль (`calculate`,
/// локализованные ошибки), затем `load_plan` движка — те же чтение и проверка, что у
/// `glafira-load run --agents`, со схемами ответов и `.avsc` для агентов.
pub fn build_plan(d: &ProjectDir, req: StartRequest) -> AppResult<RunPlan> {
    let env = env_of(&d.manifest()?, &req.env)?;
    check_suite(d, &req.suite, &env)?;
    calculate(d, &req.profile)?;
    let suite = req.suite.clone();
    load_plan(PlanArgs {
        root: d.root().to_path_buf(),
        suite: Some(req.suite),
        profile: req.profile,
        env: req.env,
        mode: req.mode,
        max_in_flight: req.max_in_flight,
        out_dir: None,
    })
    .map_err(|e| local_err(&suite, e))
}

/// Смок: всегда локальный (спека §4.6), без профиля; проверки — как у `build_plan`.
fn smoke_request(d: &ProjectDir, req: StartRequest) -> AppResult<LocalRun> {
    let env = env_of(&d.manifest()?, &req.env)?;
    check_suite(d, &req.suite, &env)?;
    Ok(LocalRun {
        root: d.root().to_path_buf(),
        suite: req.suite,
        profile: String::new(),
        env: req.env,
        mode: Mode::Smoke,
        max_in_flight: req.max_in_flight,
        out: None,
    })
}

/// Агенты стенда как `HttpAgent` с токенами из хранилища (`TOKEN_MISSING`); стенд без агентов —
/// встроенный `LocalAgent` (решение плана 8): файлы держит в памяти и отдаёт контроллеру tar-ом,
/// секреты — из окружения приложения (`GLAFIRA_LOAD_SECRET_*`).
pub fn agents(st: &AppState, root: &Path, env: &Environment) -> AppResult<Vec<Arc<dyn AgentLink>>> {
    if env.agents.is_empty() {
        let local: Arc<dyn AgentLink> =
            Arc::new(LocalAgent::new(LOCAL_AGENT_ID, executor_factory()));
        return Ok(vec![local]);
    }
    env.agents
        .iter()
        .map(|a| {
            let token = st
                .secrets
                .get(&token_key(root, &env.id, &a.id))?
                .ok_or_else(|| token_missing(&a.id))?;
            let link: Arc<dyn AgentLink> =
                Arc::new(HttpAgent::new(&a.id, &a.url, &token, CONTROLLER_VERSION));
            Ok(link)
        })
        .collect()
}

fn project_key(path: &str) -> AppResult<String> {
    Ok(ProjectDir::open(path)?
        .root()
        .to_string_lossy()
        .into_owned())
}

/// Что запускать: контроллер на агентах или локальный смок.
enum Launch {
    Controller(Box<RunPlan>, Vec<Arc<dyn AgentLink>>),
    Smoke(LocalRun),
}

/// Запуск: регистрация (`RUN_BUSY`), план и агенты в блокирующей задаче, прогон — в фоновой
/// задаче; события уходят в `on_event` по мере поступления. Возвращает итог прогона.
/// Регистрация живёт в фоновой задаче: прогон не прерывается, если ответ команды никто не ждёт.
pub async fn start(
    st: &AppState,
    path: String,
    req: StartRequest,
    on_event: impl Fn(ControllerEvent) + Send + Sync + 'static,
) -> AppResult<RunResult> {
    let root = blocking({
        let p = path.clone();
        move || project_key(&p)
    })
    .await?;
    let guard = st.runs.guard(root.clone())?;
    let cancel = guard.cancel();
    let suite = req.suite.clone();
    let st2 = st.clone();
    let launch = blocking(move || {
        let d = ProjectDir::open(&path)?;
        match req.mode {
            Mode::Smoke => Ok(Launch::Smoke(smoke_request(&d, req)?)),
            Mode::Run => {
                let plan = build_plan(&d, req)?;
                let links = agents(&st2, d.root(), &plan.env)?;
                Ok(Launch::Controller(Box::new(plan), links))
            }
        }
    })
    .await?;
    // Встроенный агент (смок или стенд без агентов) берёт секреты из окружения приложения:
    // его ошибки — без агента, чтобы текст звал задать переменные, а не чинить агента.
    let local = match &launch {
        Launch::Smoke(_) => true,
        Launch::Controller(plan, _) => plan.env.agents.is_empty(),
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ControllerEvent>(EVENT_BUFFER);
    let (registry, project) = (st.runs.clone(), root);
    let forward = tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if let ControllerEvent::Started { run_id, .. } = &ev {
                registry.set_run_id(&project, run_id);
            }
            on_event(ev);
        }
    });
    let run = tauri::async_runtime::spawn(async move {
        // Регистрация снимается, когда прогон закончился, а не когда фронтенд перестал ждать.
        let _guard = guard;
        match launch {
            Launch::Controller(plan, links) => Controller::run(*plan, links, tx, cancel).await,
            Launch::Smoke(req) => run_local(req, tx, cancel).await,
        }
    });
    let result = run
        .await
        .map_err(|e| AppError::internal_with(e.to_string()))?
        .map_err(|mut e| {
            if local {
                e.agent = None;
            }
            local_err(&suite, e)
        });
    let _ = forward.await;
    result
}

/// Запрашивает остановку прогона проекта; прогона нет — ничего не делает.
pub async fn stop(st: &AppState, path: String) -> AppResult<()> {
    let st = st.clone();
    blocking(move || {
        // Проекта уже нет на диске — ключ по пути как есть (так он и не найдётся).
        let root = project_key(&path).unwrap_or(path);
        st.runs.stop(&root);
        Ok(())
    })
    .await
}

/// Останавливает все прогоны приложения и ждёт, пока контроллеры остановят агентов и допишут
/// папки, но не дольше `timeout`. Возвращает (сколько остановлено, дождались ли).
pub async fn stop_all(st: &AppState, timeout: std::time::Duration) -> (usize, bool) {
    let stopped = st.runs.stop_all();
    let idle = st.runs.wait_idle(timeout).await;
    (stopped, idle)
}

fn invalid_file(file: &str, message: &str) -> AppError {
    app_err!(
        ErrorCode::ProjectInvalid,
        "errors.project.invalidFile",
        "{{file}}: {{error}}",
        file = file,
        error = message
    )
}

/// YAML-файл прогона; нет файла — `None`, не разбирается — `PROJECT_INVALID`.
fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> AppResult<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(text) => from_yaml(&text)
            .map(Some)
            .map_err(|e| invalid_file(label, &e.message)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::from(e).with_details(path.display().to_string())),
    }
}

/// Папки `runs/*` с `run.yaml`: (`runs/<папка>`, метаданные). Нечитаемый `run.yaml` пропускается.
pub fn read_runs(d: &ProjectDir) -> AppResult<Vec<(String, RunMeta)>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(d.file(RUNS_DIR)) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let dir = format!("{RUNS_DIR}/{name}");
        match read_yaml::<RunMeta>(&e.path().join(RUN_FILE), &format!("{dir}/{RUN_FILE}")) {
            Ok(Some(meta)) => out.push((dir, meta)),
            Ok(None) => {}
            Err(err) => tracing::warn!("run {name} skipped: {}", err.message),
        }
    }
    Ok(out)
}

/// Список прогонов по дате убыв.; незавершённые (`run.yaml` без `ended_at`) достраиваются,
/// если в проекте сейчас нет прогона. Сбой достраивания одной папки список не валит.
pub async fn list(st: &AppState, path: String) -> AppResult<Vec<RunListEntry>> {
    let (root, runs) = blocking(move || {
        let d = ProjectDir::open(&path)?;
        Ok((d.root().to_path_buf(), read_runs(&d)?))
    })
    .await?;
    let busy = st.runs.current(&root.to_string_lossy()).is_some();
    let mut out = Vec::with_capacity(runs.len());
    for (dir, meta) in runs {
        let meta = if meta.ended_at.is_none() && !busy {
            match rebuild::rebuild(st, &root, &dir, meta.clone()).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(dir, "run was not rebuilt: {}", e.message);
                    meta
                }
            }
        } else {
            meta
        };
        out.push(RunListEntry { dir, meta });
    }
    out.sort_by(|a, b| (&b.meta.started_at, &b.dir).cmp(&(&a.meta.started_at, &a.dir)));
    Ok(out)
}

/// Папка прогона: `runs/<папка>` относительно проекта или абсолютный путь внутри `runs/`.
pub fn run_dir(d: &ProjectDir, dir: &str) -> AppResult<PathBuf> {
    let not_found = || {
        app_err!(
            ErrorCode::NotFound,
            "errors.run.notFound",
            "Run {{dir}} not found",
            dir = dir
        )
    };
    let runs = d.file(RUNS_DIR);
    if !runs.is_dir() {
        return Err(not_found());
    }
    let p = inside(&runs, &resolve(d.root(), dir))?;
    if !p.join(RUN_FILE).is_file() {
        return Err(not_found());
    }
    Ok(p)
}

/// Карточка прогона: `run.yaml`, `summary.yaml` и `smoke.yaml`, если есть; `dir` в ответе —
/// `runs/<папка>`, даже если спросили абсолютным путём.
pub fn details(d: &ProjectDir, dir: &str) -> AppResult<RunDetails> {
    let p = run_dir(d, dir)?;
    let dir = relative(d.root(), &p);
    let meta = read_yaml::<RunMeta>(&p.join(RUN_FILE), &format!("{dir}/{RUN_FILE}"))?
        .ok_or_else(|| AppError::internal_with(format!("{dir}/{RUN_FILE} disappeared")))?;
    let summary = read_yaml(&p.join(SUMMARY_FILE), &format!("{dir}/{SUMMARY_FILE}"))?;
    let smoke = read_yaml(&p.join(SMOKE_FILE), &format!("{dir}/{SMOKE_FILE}"))?;
    Ok(RunDetails {
        dir,
        meta,
        summary,
        smoke,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agents::testing::{agent, closed_port_url, serve};
    use crate::commands::suite::tests::{copy_fixture_project, fixture, point_env_to};
    use crate::storage::secrets::MemoryStore;
    use axum::extract::Path as AxumPath;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use glafira_load_engine::agent_api::RunStatus;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// Заглушка стенда: `POST /v1/orders` → 201, `GET /v1/orders/{id}` → 200.
    pub(crate) fn orders_router() -> Router {
        Router::new()
            .route(
                "/v1/orders",
                post(|| async {
                    (
                        StatusCode::CREATED,
                        Json(json!({ "id": "A-1", "status": "NEW" })),
                    )
                }),
            )
            .route(
                "/v1/orders/{id}",
                get(|AxumPath(id): AxumPath<String>| async move {
                    Json(json!({ "id": id, "status": "NEW" }))
                }),
            )
    }

    pub(crate) fn state(dir: &std::path::Path) -> AppState {
        AppState::new(dir)
            .unwrap()
            .with_secrets(Arc::new(MemoryStore::default()))
    }

    /// Проект из фикстуры со сьютом `orders-http` (фикстура движка) и стендом `test` без агентов,
    /// смотрящим на заглушку.
    pub(crate) async fn project_with_stub() -> (tempfile::TempDir, String) {
        let (tmp, path) = copy_fixture_project();
        let url = serve(orders_router()).await;
        point_env_to(&path, &url, vec![]);
        let text = std::fs::read_to_string(fixture("orders-http").join("suite.yaml")).unwrap();
        ProjectDir::open(&path)
            .unwrap()
            .write_text("suites/orders-http/suite.yaml", &text)
            .unwrap();
        (tmp, path)
    }

    pub(crate) fn request(suite: &str, mode: Mode) -> StartRequest {
        StartRequest {
            suite: suite.into(),
            profile: "nominal".into(),
            env: "test".into(),
            mode,
            max_in_flight: None,
        }
    }

    fn kind(ev: &ControllerEvent) -> &'static str {
        match ev {
            ControllerEvent::Started { .. } => "started",
            ControllerEvent::Tick(_) => "tick",
            ControllerEvent::Phase(_) => "phase",
            ControllerEvent::Agent { .. } => "agent",
            ControllerEvent::Error(_) => "error",
            ControllerEvent::Done(_) => "done",
        }
    }

    pub(crate) fn run_dirs(d: &ProjectDir) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(d.file(RUNS_DIR))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    #[tokio::test]
    async fn smoke_runs_locally_and_writes_a_run_folder() {
        let (tmp, path) = project_with_stub().await;
        // Смок всегда локальный (спека §4.6): агенты стенда не опрашиваются и токены не нужны.
        let d = ProjectDir::open(&path).unwrap();
        let url = closed_port_url();
        let mut m = d.manifest().unwrap();
        m.environments[0].agents = vec![agent("gen-1", &url)];
        d.save_manifest(&m).unwrap();
        let st = state(&tmp.path().join("data"));
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let sink = events.clone();
        let result = start(
            &st,
            path.clone(),
            request("orders-http", Mode::Smoke),
            move |ev| {
                sink.lock().unwrap().push(kind(&ev));
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(result.status, RunStatus::Completed),
            "{:?}",
            result.status
        );
        assert!(!st.runs.is_busy(), "регистрация снята");
        let dirs = run_dirs(&d);
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        assert!(dirs[0].starts_with("smoke-"), "{dirs:?}");
        let run_dir = d.file(RUNS_DIR).join(&dirs[0]);
        assert!(run_dir.join(RUN_FILE).is_file() && run_dir.join(SMOKE_FILE).is_file());
        let got = details(&d, &format!("{RUNS_DIR}/{}", dirs[0])).unwrap();
        assert_eq!(got.dir, format!("{RUNS_DIR}/{}", dirs[0]));
        let smoke = got.smoke.expect("smoke.yaml");
        assert!(smoke.scenarios[0].steps.iter().all(|s| s.ok), "{smoke:?}");
        assert!(got.meta.ended_at.is_some());
        assert!(matches!(got.meta.mode, Mode::Smoke));
        assert_eq!(got.meta.suite, "orders-http");
        // Абсолютный путь внутри `runs/` тоже принимается и даёт ту же папку.
        let abs = details(&d, &run_dir.to_string_lossy()).unwrap();
        assert_eq!(abs.dir, got.dir);
        let listed = list(&st, path.clone()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].dir, format!("{RUNS_DIR}/{}", dirs[0]));
        let seen = events.lock().unwrap().clone();
        assert!(
            seen.contains(&"started") && seen.contains(&"done"),
            "{seen:?}"
        );
        assert!(!seen.contains(&"tick"), "смок без графиков: {seen:?}");
        assert_eq!(
            details(&d, "runs/none").unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            details(&d, "../outside").unwrap_err().code,
            ErrorCode::PathOutsideProject
        );
    }

    #[tokio::test]
    async fn run_on_the_local_agent_is_stopped_through_the_registry() {
        let (tmp, path) = project_with_stub().await;
        let st = state(&tmp.path().join("data"));
        let root = ProjectDir::open(&path)
            .unwrap()
            .root()
            .to_string_lossy()
            .into_owned();
        let (reg, key) = (st.runs.clone(), root.clone());
        let stopped = Arc::new(Mutex::new(None::<bool>));
        let seen = stopped.clone();
        let result = start(
            &st,
            path.clone(),
            request("orders-http", Mode::Run),
            move |ev| {
                if matches!(ev, ControllerEvent::Started { .. }) {
                    // Прогон зарегистрирован к моменту `Started`, и `run_id` уже известен.
                    let known = reg.current(&key).flatten().is_some();
                    *seen.lock().unwrap() = Some(known && reg.stop(&key));
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(*stopped.lock().unwrap(), Some(true));
        assert!(
            matches!(result.status, RunStatus::Stopped),
            "{:?}",
            result.status
        );
        let d = ProjectDir::open(&path).unwrap();
        let dirs = run_dirs(&d);
        assert_eq!(dirs.len(), 1);
        assert!(!dirs[0].starts_with("smoke-"));
        let dir = format!("{RUNS_DIR}/{}", dirs[0]);
        let got = details(&d, &dir).unwrap();
        assert!(matches!(got.meta.status, Some(RunStatus::Stopped)));
        assert!(got.meta.ended_at.is_some());
        assert!(got.summary.is_some());
        assert_eq!(got.meta.agents.len(), 1);
        assert_eq!(got.meta.agents[0].id, LOCAL_AGENT_ID);
        assert!(
            d.file(&dir)
                .join(AGENTS_DIR)
                .join(LOCAL_AGENT_ID)
                .join(SUMMARY_FILE)
                .is_file()
        );
        // Остановка без прогона — не ошибка.
        stop(&st, path).await.unwrap();
    }

    #[tokio::test]
    async fn stop_all_waits_until_the_run_folder_is_complete() {
        let (tmp, path) = project_with_stub().await;
        let st = state(&tmp.path().join("data"));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Mutex::new(Some(tx));
        let run = tokio::spawn({
            let (st, path) = (st.clone(), path.clone());
            async move {
                start(&st, path, request("orders-http", Mode::Run), move |ev| {
                    if matches!(ev, ControllerEvent::Started { .. })
                        && let Some(tx) = tx.lock().unwrap().take()
                    {
                        let _ = tx.send(());
                    }
                })
                .await
            }
        });
        rx.await.unwrap();
        // Как `confirm_close(stop = true)`: остановить всё и дождаться, не дожидаясь ответа `run_start`.
        let (stopped, idle) = stop_all(&st, std::time::Duration::from_secs(30)).await;
        assert_eq!((stopped, idle), (1, true));
        assert!(!st.runs.is_busy());
        let d = ProjectDir::open(&path).unwrap();
        let dirs = run_dirs(&d);
        assert_eq!(dirs.len(), 1);
        let got = details(&d, &format!("{RUNS_DIR}/{}", dirs[0])).unwrap();
        assert!(matches!(got.meta.status, Some(RunStatus::Stopped)));
        assert!(got.meta.ended_at.is_some(), "папка дописана до выхода");
        let result = run.await.unwrap().unwrap();
        assert!(matches!(result.status, RunStatus::Stopped));
        // Прогонов нет — сразу.
        assert_eq!(stop_all(&st, std::time::Duration::ZERO).await, (0, true));
    }

    #[tokio::test]
    async fn second_run_in_the_project_is_busy() {
        let (tmp, path) = project_with_stub().await;
        let st = state(&tmp.path().join("data"));
        let root = ProjectDir::open(&path)
            .unwrap()
            .root()
            .to_string_lossy()
            .into_owned();
        let guard = st.runs.guard(root).unwrap();
        let e = start(
            &st,
            path.clone(),
            request("orders-http", Mode::Smoke),
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::RunBusy);
        drop(guard);
        assert!(
            start(&st, path, request("orders-http", Mode::Smoke), |_| {})
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn invalid_suite_and_missing_token_block_the_start() {
        let (tmp, path) = project_with_stub().await;
        let st = state(&tmp.path().join("data"));
        let d = ProjectDir::open(&path).unwrap();
        d.write_text(
            "suites/bad/suite.yaml",
            "format: 1\nname: bad\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:GET /nope\" }\n",
        )
        .unwrap();
        for mode in [Mode::Smoke, Mode::Run] {
            let e = start(&st, path.clone(), request("bad", mode), |_| {})
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::SuiteInvalid);
            let findings: Vec<Finding> =
                serde_json::from_str(e.details.as_deref().unwrap()).unwrap();
            assert!(has_errors(&findings));
            assert!(!st.runs.is_busy());
        }

        let url = closed_port_url();
        point_env_to(&path, &url, vec![agent("gen-1", &url)]);
        let e = start(&st, path.clone(), request("orders-http", Mode::Run), |_| {})
            .await
            .unwrap_err();
        assert_eq!(
            (e.code, e.params["id"].as_str()),
            (ErrorCode::TokenMissing, "gen-1")
        );
        st.secrets
            .set(&token_key(d.root(), "test", "gen-1"), "t")
            .unwrap();
        let e = start(&st, path.clone(), request("orders-http", Mode::Run), |_| {})
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::AgentUnreachable, "{e:?}");
        assert_eq!(e.params["id"], "gen-1");
        assert!(!st.runs.is_busy());
        assert!(run_dirs(&d).is_empty(), "до старта папки нет");
        assert_eq!(
            start(
                &st,
                path.clone(),
                request("orders-http", Mode::Run).with_env("prod"),
                |_| {}
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            start(
                &st,
                path,
                StartRequest {
                    profile: "nope".into(),
                    ..request("orders-http", Mode::Run)
                },
                |_| {}
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn local_runs_ask_for_secrets_in_the_app_environment() {
        let (tmp, path) = project_with_stub().await;
        let st = state(&tmp.path().join("data"));
        let d = ProjectDir::open(&path).unwrap();
        let text = d
            .read_text("suites/orders-http/suite.yaml")
            .unwrap()
            .unwrap();
        let name = "GLAFIRA_TEST_TASK13_ABSENT";
        d.write_text(
            "suites/orders-http/suite.yaml",
            &text.replace(
                "name: orders-http\n",
                &format!("name: orders-http\nsecrets: [{name}]\n"),
            ),
        )
        .unwrap();
        for mode in [Mode::Smoke, Mode::Run] {
            let e = start(&st, path.clone(), request("orders-http", mode), |_| {})
                .await
                .unwrap_err();
            assert_eq!(
                (e.code, e.key.as_str()),
                (ErrorCode::SecretMissing, "errors.run.secretMissing"),
                "{mode:?}: {e:?}"
            );
            assert_eq!(e.params["names"], name);
            assert!(!st.runs.is_busy());
        }
    }

    impl StartRequest {
        fn with_env(mut self, env: &str) -> Self {
            self.env = env.into();
            self
        }
    }

    #[test]
    fn engine_errors_map_to_app_codes() {
        let err = |code: &str, agent: Option<&str>, names: &[&str]| EngineError {
            code: code.into(),
            message: "m".into(),
            names: names.iter().map(|s| s.to_string()).collect(),
            agent: agent.map(String::from),
        };
        let cases = [
            (
                "AGENT_UNREACHABLE",
                ErrorCode::AgentUnreachable,
                "errors.agent.unreachable",
            ),
            (
                "UNAUTHORIZED",
                ErrorCode::AgentUnreachable,
                "errors.agent.unauthorized",
            ),
            ("AGENT_BUSY", ErrorCode::AgentBusy, "errors.agent.busy"),
            (
                "AGENT_VERSION",
                ErrorCode::AgentVersion,
                "errors.agent.version",
            ),
            (
                "SECRET_MISSING",
                ErrorCode::SecretMissing,
                "errors.agent.secretMissing",
            ),
            (
                "SUITE_INVALID",
                ErrorCode::SuiteInvalid,
                "errors.suite.rejected",
            ),
            (
                "TOKEN_MISSING",
                ErrorCode::TokenMissing,
                "errors.agent.tokenMissing",
            ),
            ("WHATEVER", ErrorCode::Internal, "errors.internal"),
        ];
        for (code, expected, key) in cases {
            let e = engine_err(err(code, Some("gen-1"), &["API_TOKEN", "DB"]));
            assert_eq!((e.code, e.key.as_str()), (expected, key), "{code}");
        }
        let e = engine_err(err("SECRET_MISSING", Some("gen-1"), &["API_TOKEN", "DB"]));
        assert_eq!(e.params["names"], "API_TOKEN, DB");
        assert_eq!(e.params["id"], "gen-1");
        let e = engine_err(err("WHATEVER", None, &[]));
        assert_eq!(e.details.as_deref(), Some("WHATEVER: m"));
        // Локальный прогон: секреты берутся из окружения приложения — агента в тексте нет.
        let e = engine_err(err("SECRET_MISSING", None, &["API_TOKEN"]));
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::SecretMissing, "errors.run.secretMissing")
        );
        // `check` движка без агента — находки в `details`, как у `check` приложения.
        let e = local_err(
            "orders",
            err("SUITE_INVALID", None, &["suites/orders/suite.yaml:3: x"]),
        );
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::SuiteInvalid, "errors.suite.invalid")
        );
        let findings: Vec<Finding> = serde_json::from_str(e.details.as_deref().unwrap()).unwrap();
        assert_eq!(findings[0].message, "suites/orders/suite.yaml:3: x");
        assert!(has_errors(&findings));
    }

    #[test]
    fn dto_shapes_for_the_frontend() {
        let meta: RunMeta = glafira_load_model::from_yaml(
            "run_id: r1\nsuite: s\nprofile: p\nenv: test\nmode: run\nagents: [{ id: local }]\nstarted_at: \"2026-11-01T10:00:00Z\"\n",
        )
        .unwrap();
        let entry = serde_json::to_value(RunListEntry {
            dir: "runs/2026-11-01T10-00-00".into(),
            meta,
        })
        .unwrap();
        assert_eq!(entry["dir"], "runs/2026-11-01T10-00-00");
        assert_eq!(entry["run_id"], "r1");
        assert_eq!(entry["started_at"], "2026-11-01T10:00:00Z");
        assert!(entry.get("meta").is_none(), "RunMeta развёрнут в строку");
        let ev = serde_json::to_value(ControllerEvent::Started {
            run_id: "r1".into(),
            dir: "/p/runs/x".into(),
        })
        .unwrap();
        assert_eq!(
            ev,
            json!({ "type": "started", "run_id": "r1", "dir": "/p/runs/x" })
        );
        let req: StartRequest = serde_json::from_value(json!({
            "suite": "s", "profile": "p", "env": "test", "mode": "smoke", "maxInFlight": 5
        }))
        .unwrap();
        assert_eq!((req.mode, req.max_in_flight), (Mode::Smoke, Some(5)));
    }
}
