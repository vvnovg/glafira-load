//! Достраивание незавершённого прогона (спека §6.3): агенты из `run.yaml` опрашиваются
//! (`GET /v1/runs` → статус, `GET /v1/runs/{id}/files` → tar), файлы ложатся в `agents/<id>/`,
//! `summary.yaml` и `run.yaml` дописываются; кого-то не дозвались, нет токена, прогон удалён по
//! `--keep-runs` или агент локальный — статус `incomplete`. Агент, который ещё исполняет этот прогон
//! (`/v1/info.busy`), откладывает достраивание: запись остаётся незавершённой. Сводный
//! `metrics.jsonl` не пересобирается: живые графики закрывшегося окна уже не нужны.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures::future::join_all;
use glafira_load_engine::agent_api::{
    AgentSummary, OpSummary, RunListItem, RunStatus, ScenarioSummary,
};
use glafira_load_engine::agent_link::{AgentLink, HttpAgent};
use glafira_load_engine::run_files::{RunAgent, RunMeta};
use glafira_load_model::project::is_id;
use glafira_load_model::{from_yaml, to_yaml};

use crate::agents::probe::APP_VERSION;
use crate::agents::token_key;
use crate::commands::blocking;
use crate::error::AppResult;
use crate::runs::{AGENTS_DIR, RUN_FILE, SUMMARY_FILE};
use crate::scan::run::now_rfc3339;
use crate::state::AppState;
use crate::storage::fs::atomic_write_shared;

/// Таймаут `GET /v1/runs` и `GET /v1/info`; на скачивание tar — шестикратный.
pub const AGENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Что удалось забрать у агента.
pub struct Fetched {
    pub status: RunStatus,
    pub summary: Option<AgentSummary>,
}

/// Ответ агента о прогоне.
pub enum Fetch {
    /// Прогон у агента завершён: статус из `GET /v1/runs`, файлы распакованы в `agents/<id>/`.
    Got(Fetched),
    /// Агент ещё исполняет этот прогон (`/v1/info.busy == run_id`): достраивать рано.
    Running,
    /// Не дозвались, нет токена, прогона у агента нет (удалён по `--keep-runs`) или агент локальный.
    Missing,
}

/// `GET /v1/runs` агента с bearer-токеном; ошибка — текст для журнала.
async fn agent_runs(url: &str, token: &str) -> Result<Vec<RunListItem>, String> {
    let client = reqwest::Client::builder()
        .timeout(AGENT_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{}/v1/runs", url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<RunListItem>>()
        .await
        .map_err(|e| e.to_string())
}

/// Раскладывает tar агента в `into`: только обычные файлы верхнего уровня (имя без каталогов,
/// без `..` и ссылок) — архив пришёл по сети; возвращает имена в порядке архива.
pub fn unpack(tar: &[u8], into: &Path) -> AppResult<Vec<String>> {
    std::fs::create_dir_all(into)?;
    let mut names = Vec::new();
    let mut archive = tar::Archive::new(tar);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut normal = path
            .components()
            .filter(|c| !matches!(c, Component::CurDir));
        let name = match (normal.next(), normal.next()) {
            (Some(Component::Normal(name)), None) if entry.header().entry_type().is_file() => {
                name.to_string_lossy().into_owned()
            }
            _ => {
                tracing::warn!(dir = %into.display(), "skipping agent tar entry {}", path.display());
                continue;
            }
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        atomic_write_shared(&into.join(&name), &bytes)?;
        names.push(name);
    }
    Ok(names)
}

/// Один агент: `Missing` — не дозвались, нет токена, прогона у агента нет или агент локальный;
/// `Running` — агент ещё исполняет этот прогон.
async fn fetch(run_dir: PathBuf, run_id: String, agent: RunAgent, token: Option<String>) -> Fetch {
    let Some(url) = agent.url.clone() else {
        return Fetch::Missing;
    };
    // Id — имя папки `agents/<id>/`: из `run.yaml` берём только допустимые.
    if !is_id(&agent.id) {
        tracing::warn!(agent = agent.id, "bad agent id in run.yaml");
        return Fetch::Missing;
    }
    let Some(token) = token else {
        tracing::warn!(
            agent = agent.id,
            "no token: run cannot be rebuilt from this agent"
        );
        return Fetch::Missing;
    };
    let runs = match agent_runs(&url, &token).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(agent = agent.id, "GET /v1/runs failed: {e}");
            return Fetch::Missing;
        }
    };
    let link = HttpAgent::new(&agent.id, &url, &token, APP_VERSION);
    let Some(item) = runs.into_iter().find(|r| r.run_id == run_id) else {
        // В списке только завершённые прогоны (спека §4.3): текущий виден в `/v1/info.busy`.
        return match tokio::time::timeout(AGENT_TIMEOUT, link.info()).await {
            Ok(Ok(info)) if info.busy.as_deref() == Some(run_id.as_str()) => {
                tracing::info!(agent = agent.id, "run is still in progress on the agent");
                Fetch::Running
            }
            _ => Fetch::Missing,
        };
    };
    let tar = match tokio::time::timeout(AGENT_TIMEOUT * 6, link.files(&run_id)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            tracing::warn!(agent = agent.id, "files failed: {}: {}", e.code, e.message);
            return Fetch::Missing;
        }
        Err(_) => {
            tracing::warn!(agent = agent.id, "files timed out");
            return Fetch::Missing;
        }
    };
    let dir = run_dir.join(AGENTS_DIR).join(&agent.id);
    let summary = match blocking(move || {
        unpack(&tar, &dir)?;
        Ok(std::fs::read_to_string(dir.join(SUMMARY_FILE))
            .ok()
            .and_then(|t| from_yaml::<AgentSummary>(&t).ok()))
    })
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(agent = agent.id, "unpack failed: {}", e.message);
            return Fetch::Missing;
        }
    };
    Fetch::Got(Fetched {
        status: item.status,
        summary,
    })
}

fn add_err(acc: &mut BTreeMap<String, u64>, more: &BTreeMap<String, u64>) {
    for (k, v) in more {
        *acc.entry(k.clone()).or_default() += v;
    }
}

fn add_op(acc: &mut OpSummary, o: &OpSummary) {
    acc.n += o.n;
    add_err(&mut acc.err, &o.err);
    acc.dropped += o.dropped;
    acc.rps_avg += o.rps_avg;
    acc.rps_peak += o.rps_peak;
    acc.p50 = acc.p50.max(o.p50);
    acc.p90 = acc.p90.max(o.p90);
    acc.p95 = acc.p95.max(o.p95);
    acc.p99 = acc.p99.max(o.p99);
    acc.max = acc.max.max(o.max);
    acc.target = match (acc.target, o.target) {
        (Some(a), Some(b)) => Some(a + b),
        (a, b) => a.or(b),
    };
}

fn add_scenario(acc: &mut ScenarioSummary, s: &ScenarioSummary) {
    acc.n += s.n;
    add_err(&mut acc.err, &s.err);
    acc.p50 = acc.p50.max(s.p50);
    acc.p95 = acc.p95.max(s.p95);
    acc.p99 = acc.p99.max(s.p99);
    acc.max = acc.max.max(s.max);
}

/// Сводка достроенного прогона из агентских: счётчики, RPS и цели суммируются, перцентили —
/// наибольшие по агентам (верхняя оценка; точные даёт только живой контроллер из слитых
/// гистограмм), доля ошибок и достигнутый процент пересчитываются.
pub fn merge_summaries(parts: &[AgentSummary], status: RunStatus) -> AgentSummary {
    let mut ops: BTreeMap<String, OpSummary> = BTreeMap::new();
    let mut scenarios: BTreeMap<String, ScenarioSummary> = BTreeMap::new();
    for p in parts {
        for o in &p.ops {
            match ops.get_mut(&o.id) {
                Some(acc) => add_op(acc, o),
                None => {
                    ops.insert(o.id.clone(), o.clone());
                }
            }
        }
        for s in &p.scenarios {
            match scenarios.get_mut(&s.id) {
                Some(acc) => add_scenario(acc, s),
                None => {
                    scenarios.insert(s.id.clone(), s.clone());
                }
            }
        }
    }
    for o in ops.values_mut() {
        let errors: u64 = o.err.values().sum();
        o.err_pct = if o.n > 0 {
            errors as f64 / o.n as f64 * 100.0
        } else {
            0.0
        };
        o.achieved_pct = o.target.filter(|t| *t > 0.0).map(|t| o.rps_avg / t * 100.0);
    }
    AgentSummary {
        status,
        ops: ops.into_values().collect(),
        scenarios: scenarios.into_values().collect(),
        seconds: parts.iter().map(|p| p.seconds).max().unwrap_or(0),
    }
}

/// Статус достроенного прогона в том же порядке, что у `Controller::run`: кого-то нет →
/// `incomplete`; иначе `stopped` > `failed` > `completed`.
pub fn combined_status(lost: bool, parts: &[RunStatus]) -> RunStatus {
    if lost || parts.is_empty() || parts.contains(&RunStatus::Incomplete) {
        RunStatus::Incomplete
    } else if parts.contains(&RunStatus::Stopped) {
        RunStatus::Stopped
    } else if parts.contains(&RunStatus::Failed) {
        RunStatus::Failed
    } else {
        RunStatus::Completed
    }
}

/// Достраивает прогон `dir` (`runs/<папка>`): файлы агентов, `summary.yaml`, `run.yaml`; если
/// какой-то агент ещё исполняет прогон — возвращает `meta` без изменений.
pub async fn rebuild(
    st: &AppState,
    root: &Path,
    dir: &str,
    mut meta: RunMeta,
) -> AppResult<RunMeta> {
    let run_dir = root.join(dir);
    let tokens = {
        let (st, root, env, agents) = (
            st.clone(),
            root.to_path_buf(),
            meta.env.clone(),
            meta.agents.clone(),
        );
        blocking(move || {
            agents
                .iter()
                .map(|a| match &a.url {
                    Some(_) => st.secrets.get(&token_key(&root, &env, &a.id)),
                    None => Ok(None),
                })
                .collect::<AppResult<Vec<_>>>()
        })
        .await?
    };
    let fetched = join_all(
        meta.agents
            .iter()
            .cloned()
            .zip(tokens)
            .map(|(a, t)| fetch(run_dir.clone(), meta.run_id.clone(), a, t)),
    )
    .await;
    if fetched.iter().any(|f| matches!(f, Fetch::Running)) {
        tracing::info!(
            run = meta.run_id,
            "an agent is still running this run: rebuild postponed"
        );
        return Ok(meta);
    }
    let lost = fetched.iter().any(|f| matches!(f, Fetch::Missing));
    let got: Vec<Fetched> = fetched
        .into_iter()
        .filter_map(|f| match f {
            Fetch::Got(x) => Some(x),
            Fetch::Running | Fetch::Missing => None,
        })
        .collect();
    let statuses: Vec<RunStatus> = got.iter().map(|f| f.status).collect();
    let status = combined_status(lost, &statuses);
    let summaries: Vec<AgentSummary> = got.into_iter().filter_map(|f| f.summary).collect();
    meta.ended_at = Some(now_rfc3339());
    meta.status = Some(status);
    let out = meta.clone();
    blocking(move || {
        if !summaries.is_empty() {
            atomic_write_shared(
                &run_dir.join(SUMMARY_FILE),
                to_yaml(&merge_summaries(&summaries, status)).as_bytes(),
            )?;
        }
        atomic_write_shared(&run_dir.join(RUN_FILE), to_yaml(&meta).as_bytes())
    })
    .await?;
    tracing::info!(run = out.run_id, status = ?out.status, "run rebuilt");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::testing::{bearer_ok, closed_port_url, serve};
    use crate::project::manifest::create;
    use crate::runs::tests::state;
    use crate::runs::{RUNS_DIR, list};
    use axum::extract::Path as AxumPath;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::get;
    use axum::{Json, Router};
    use glafira_load_engine::agent_api::{AgentInfo, Mode};
    use std::sync::Arc;

    fn op(id: &str, n: u64, p95: f64, target: f64) -> OpSummary {
        OpSummary {
            id: id.into(),
            n,
            rps_avg: n as f64 / 10.0,
            rps_peak: n as f64 / 8.0,
            err: BTreeMap::from([("HTTP_5xx".to_string(), 1)]),
            err_pct: 100.0 / n as f64,
            dropped: 1,
            p50: p95 / 2.0,
            p90: p95 * 0.9,
            p95,
            p99: p95 * 1.5,
            max: p95 * 2.0,
            target: Some(target),
            achieved_pct: Some(n as f64 / 10.0 / target * 100.0),
        }
    }

    fn summary(status: RunStatus, n: u64, p95: f64) -> AgentSummary {
        AgentSummary {
            status,
            ops: vec![op("http:orders:POST /v1/orders", n, p95, 5.0)],
            scenarios: vec![ScenarioSummary {
                id: "flow".into(),
                n,
                err: BTreeMap::new(),
                p50: p95 / 2.0,
                p95,
                p99: p95 * 1.5,
                max: p95 * 2.0,
            }],
            seconds: 10,
        }
    }

    /// tar с `summary.yaml`, `metrics.jsonl` и вложенным путём, который должен быть пропущен.
    fn tar_of(summary: &AgentSummary) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        let mut add = |name: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, data).unwrap();
        };
        add("summary.yaml", to_yaml(summary).as_bytes());
        add("metrics.jsonl", b"{\"t\":1}\n");
        add("nested/evil.txt", b"no");
        b.into_inner().unwrap()
    }

    /// Заглушка агента: `GET /v1/info` (с `busy`), `GET /v1/runs` и `GET /v1/runs/{id}/files`
    /// с проверкой токена.
    fn agent_router(
        token: &str,
        run_id: &str,
        summary: AgentSummary,
        busy: Option<&str>,
    ) -> Router {
        let tar = Arc::new(tar_of(&summary));
        let (token_a, token_b, token_c) = (token.to_string(), token.to_string(), token.to_string());
        let (run_a, run_b) = (run_id.to_string(), run_id.to_string());
        let busy = busy.map(String::from);
        let status = summary.status;
        Router::new()
            .route(
                "/v1/info",
                get(move |headers: HeaderMap| {
                    let (token, busy) = (token_c.clone(), busy.clone());
                    async move {
                        if !bearer_ok(&headers, &token) {
                            return Err(StatusCode::UNAUTHORIZED);
                        }
                        Ok(Json(AgentInfo {
                            version: APP_VERSION.into(),
                            os: "linux".into(),
                            arch: "x86_64".into(),
                            cpus: 2,
                            busy,
                        }))
                    }
                }),
            )
            .route(
                "/v1/runs",
                get(move |headers: HeaderMap| {
                    let (token, run_id) = (token_a.clone(), run_a.clone());
                    async move {
                        if !bearer_ok(&headers, &token) {
                            return Err(StatusCode::UNAUTHORIZED);
                        }
                        Ok(Json(vec![RunListItem {
                            run_id,
                            status,
                            started_at: "2026-11-01T10:00:00Z".into(),
                        }]))
                    }
                }),
            )
            .route(
                "/v1/runs/{id}/files",
                get(move |AxumPath(id): AxumPath<String>, headers: HeaderMap| {
                    let (token, run_id, tar) = (token_b.clone(), run_b.clone(), tar.clone());
                    async move {
                        if !bearer_ok(&headers, &token) {
                            return Err(StatusCode::UNAUTHORIZED);
                        }
                        if id != run_id {
                            return Err(StatusCode::NOT_FOUND);
                        }
                        Ok((
                            [(header::CONTENT_TYPE, "application/x-tar")],
                            tar.as_ref().clone(),
                        ))
                    }
                }),
            )
    }

    fn meta(run_id: &str, started_at: &str, agents: Vec<RunAgent>) -> RunMeta {
        RunMeta {
            run_id: run_id.into(),
            suite: "orders".into(),
            profile: "peak".into(),
            env: "test".into(),
            mode: Mode::Run,
            agents,
            project_commit: None,
            services: BTreeMap::new(),
            started_at: started_at.into(),
            ended_at: None,
            status: None,
        }
    }

    fn run_agent(id: &str, url: Option<&str>) -> RunAgent {
        RunAgent {
            id: id.into(),
            url: url.map(String::from),
            weight: 1,
            version: Some("0.2.0".into()),
        }
    }

    #[test]
    fn merge_sums_counters_and_takes_the_largest_percentiles() {
        let a = summary(RunStatus::Completed, 100, 40.0);
        let b = summary(RunStatus::Completed, 300, 80.0);
        let m = merge_summaries(&[a, b], RunStatus::Completed);
        assert_eq!(m.ops.len(), 1);
        let o = &m.ops[0];
        assert_eq!((o.n, o.dropped, o.err["HTTP_5xx"]), (400, 2, 2));
        assert_eq!((o.rps_avg, o.rps_peak), (40.0, 50.0));
        assert_eq!((o.p50, o.p95, o.max), (40.0, 80.0, 160.0));
        assert_eq!(o.target, Some(10.0));
        assert_eq!(o.achieved_pct, Some(400.0));
        assert!((o.err_pct - 0.5).abs() < 1e-9);
        assert_eq!((m.scenarios[0].n, m.scenarios[0].p95), (400, 80.0));
        assert_eq!(m.seconds, 10);
        assert!(matches!(m.status, RunStatus::Completed));
    }

    #[test]
    fn combined_status_follows_the_controller() {
        use RunStatus::*;
        assert_eq!(combined_status(true, &[Completed]), Incomplete);
        assert_eq!(combined_status(false, &[]), Incomplete);
        assert_eq!(combined_status(false, &[Completed, Incomplete]), Incomplete);
        assert_eq!(combined_status(false, &[Completed, Stopped]), Stopped);
        // Как `Controller::run`: остановка важнее отказа агента.
        assert_eq!(combined_status(false, &[Failed, Stopped]), Stopped);
        assert_eq!(combined_status(false, &[Completed, Failed]), Failed);
        assert_eq!(combined_status(false, &[Completed]), Completed);
    }

    #[test]
    fn unpack_keeps_only_top_level_files() {
        let tmp = tempfile::tempdir().unwrap();
        let names = unpack(&tar_of(&summary(RunStatus::Completed, 1, 1.0)), tmp.path()).unwrap();
        assert_eq!(names, ["summary.yaml", "metrics.jsonl"]);
        assert!(tmp.path().join("summary.yaml").is_file());
        assert!(!tmp.path().join("nested").exists());
    }

    #[tokio::test]
    async fn run_list_rebuilds_unfinished_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(&tmp.path().join("data"));
        let path = tmp.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let d = crate::project::manifest::ProjectDir::open(&path).unwrap();
        let ok = serve(agent_router(
            "tok-1",
            "r1",
            summary(RunStatus::Completed, 100, 40.0),
            None,
        ))
        .await;
        let stopped = serve(agent_router(
            "tok-2",
            "r2",
            summary(RunStatus::Stopped, 50, 20.0),
            None,
        ))
        .await;
        // Агент ещё исполняет r5: в списке его нет, он виден в `info.busy`.
        let running = serve(agent_router(
            "tok-5",
            "other",
            summary(RunStatus::Completed, 1, 1.0),
            Some("r5"),
        ))
        .await;
        let closed = closed_port_url();
        for (id, token) in [
            ("gen-1", "tok-1"),
            ("gen-2", "tok-2"),
            ("gen-3", "x"),
            ("gen-5", "tok-5"),
        ] {
            st.secrets
                .set(&token_key(d.root(), "test", id), token)
                .unwrap();
        }
        let write = |dir: &str, m: &RunMeta| {
            d.write_text(&format!("{RUNS_DIR}/{dir}/{RUN_FILE}"), &to_yaml(m))
                .unwrap();
        };
        // r1: один агент отвечает, другой на закрытом порту → incomplete.
        write(
            "2026-11-01T10-00-00",
            &meta(
                "r1",
                "2026-11-01T10:00:00Z",
                vec![
                    run_agent("gen-1", Some(&ok)),
                    run_agent("gen-3", Some(&closed)),
                ],
            ),
        );
        // r2: агент отвечает, прогон был остановлен → stopped.
        write(
            "2026-11-01T11-00-00",
            &meta(
                "r2",
                "2026-11-01T11:00:00Z",
                vec![run_agent("gen-2", Some(&stopped))],
            ),
        );
        // r3: агент без токена и локальный → incomplete без сети.
        write(
            "2026-11-01T09-00-00",
            &meta(
                "r3",
                "2026-11-01T09:00:00Z",
                vec![run_agent("gen-9", Some(&ok)), run_agent("local", None)],
            ),
        );
        // r4: уже завершён — не трогается.
        let mut done = meta("r4", "2026-11-01T12:00:00Z", vec![]);
        done.ended_at = Some("2026-11-01T12:30:00Z".into());
        done.status = Some(RunStatus::Completed);
        write("smoke-2026-11-01T12-00-00", &done);
        // r5: агент ещё исполняет прогон → достраивание отложено, запись остаётся незавершённой.
        write(
            "2026-11-01T08-00-00",
            &meta(
                "r5",
                "2026-11-01T08:00:00Z",
                vec![run_agent("gen-5", Some(&running))],
            ),
        );
        // r6: агент отвечает, но прогона у него уже нет (удалён по `--keep-runs`) → incomplete.
        write(
            "2026-11-01T07-00-00",
            &meta(
                "r6",
                "2026-11-01T07:00:00Z",
                vec![run_agent("gen-1", Some(&ok))],
            ),
        );

        let out = list(&st, path.clone()).await.unwrap();
        let dirs: Vec<&str> = out.iter().map(|e| e.dir.as_str()).collect();
        assert_eq!(
            dirs,
            [
                "runs/smoke-2026-11-01T12-00-00",
                "runs/2026-11-01T11-00-00",
                "runs/2026-11-01T10-00-00",
                "runs/2026-11-01T09-00-00",
                "runs/2026-11-01T08-00-00",
                "runs/2026-11-01T07-00-00",
            ],
            "по дате убыв."
        );
        let by_id = |id: &str| out.iter().find(|e| e.meta.run_id == id).unwrap();
        assert_eq!(by_id("r1").meta.status, Some(RunStatus::Incomplete));
        assert_eq!(by_id("r2").meta.status, Some(RunStatus::Stopped));
        assert_eq!(by_id("r3").meta.status, Some(RunStatus::Incomplete));
        assert_eq!(
            by_id("r4").meta.ended_at.as_deref(),
            Some("2026-11-01T12:30:00Z")
        );
        assert_eq!(by_id("r6").meta.status, Some(RunStatus::Incomplete));
        assert!(
            by_id("r5").meta.status.is_none() && by_id("r5").meta.ended_at.is_none(),
            "агент занят r5: не трогаем"
        );
        assert!(
            !d.file("runs/2026-11-01T08-00-00")
                .join(SUMMARY_FILE)
                .exists()
        );
        assert!(
            out.iter()
                .filter(|e| e.meta.run_id != "r5")
                .all(|e| e.meta.ended_at.is_some())
        );
        let r1 = d.file("runs/2026-11-01T10-00-00");
        assert!(
            r1.join(AGENTS_DIR)
                .join("gen-1")
                .join(SUMMARY_FILE)
                .is_file()
        );
        assert!(
            r1.join(AGENTS_DIR)
                .join("gen-1")
                .join("metrics.jsonl")
                .is_file()
        );
        assert!(!r1.join(AGENTS_DIR).join("gen-1").join("nested").exists());
        assert!(!r1.join(AGENTS_DIR).join("gen-3").exists());
        let merged: AgentSummary =
            from_yaml(&std::fs::read_to_string(r1.join(SUMMARY_FILE)).unwrap()).unwrap();
        assert_eq!(merged.ops[0].n, 100);
        assert_eq!(merged.status, RunStatus::Incomplete);
        let r2: AgentSummary = from_yaml(
            &std::fs::read_to_string(d.file("runs/2026-11-01T11-00-00").join(SUMMARY_FILE))
                .unwrap(),
        )
        .unwrap();
        assert_eq!((r2.status, r2.ops[0].n), (RunStatus::Stopped, 50));
        assert!(
            !d.file("runs/2026-11-01T09-00-00")
                .join(SUMMARY_FILE)
                .exists(),
            "ни одного агента не достали"
        );
        let again: RunMeta =
            from_yaml(&std::fs::read_to_string(r1.join(RUN_FILE)).unwrap()).unwrap();
        assert!(again.ended_at.is_some(), "run.yaml дописан");
        // Повторный список ничего не достраивает (ended_at уже есть; r5 по-прежнему занят)
        // и даёт тот же результат.
        let out2 = list(&st, path.clone()).await.unwrap();
        assert_eq!(out2.len(), 6);
        assert_eq!(out2[1].meta.ended_at, by_id("r2").meta.ended_at);
        assert!(out2[4].meta.ended_at.is_none());

        // Пока в проекте идёт прогон, список ничего не достраивает.
        write(
            "2026-11-01T06-00-00",
            &meta("r7", "2026-11-01T06:00:00Z", vec![run_agent("local", None)]),
        );
        let guard = st
            .runs
            .guard(d.root().to_string_lossy().into_owned())
            .unwrap();
        let out3 = list(&st, path.clone()).await.unwrap();
        assert!(
            out3.iter()
                .find(|e| e.meta.run_id == "r7")
                .unwrap()
                .meta
                .ended_at
                .is_none()
        );
        drop(guard);
        let out4 = list(&st, path).await.unwrap();
        let r7 = out4.iter().find(|e| e.meta.run_id == "r7").unwrap();
        assert_eq!(r7.meta.status, Some(RunStatus::Incomplete));
    }
}
