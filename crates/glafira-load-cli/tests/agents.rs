//! `glafira-load run --agents`: два настоящих агента (в процессе, на свободных портах) + заглушка
//! стенда на axum + временная копия проекта из фикстур (спека §4.5, §5.4, §9.1).

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use axum::http::StatusCode;
use common::{TOKEN, stand_stub, temp_project, write_prod_load, write_profile};
use glafira_load_cli::agent::runs::{AgentConfig, AgentState};
use glafira_load_cli::agent::server;
use glafira_load_engine::agent_api::AgentSummary;
use glafira_load_model::project::{Agent, MANIFEST_FILE, Manifest};
use glafira_load_model::{from_yaml, to_yaml};
use tempfile::TempDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SUITE: &str = "orders-http";
const CREATE: &str = "http:orders:POST /v1/orders";

struct RunningAgent {
    url: String,
    _work: TempDir,
    _state: Arc<AgentState>,
}

/// Агент `glafira-load agent` в процессе теста: свободный порт, токен `TOKEN`, своя рабочая папка.
async fn start_agent() -> RunningAgent {
    let work = tempfile::tempdir().unwrap();
    let state = AgentState::new(AgentConfig {
        work_dir: work.path().to_path_buf(),
        keep_runs: 10,
        drain: Duration::from_millis(1500),
        token: TOKEN.into(),
        secrets_env: BTreeMap::new(),
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
    RunningAgent {
        url: format!("http://{addr}"),
        _work: work,
        _state: state,
    }
}

/// Стенд `test` проекта получает агентов `a` (вес 1) и `b` (вес 2).
fn add_agents(root: &Path, a: &str, b: &str) {
    let path = root.join(MANIFEST_FILE);
    let mut manifest: Manifest = from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let env = manifest
        .environments
        .iter_mut()
        .find(|e| e.id == "test")
        .unwrap();
    env.agents = vec![
        Agent {
            id: "a".into(),
            url: a.into(),
            weight: 1,
        },
        Agent {
            id: "b".into(),
            url: b.into(),
            weight: 2,
        },
    ];
    std::fs::write(&path, to_yaml(&manifest)).unwrap();
}

fn op_n(summary: &AgentSummary, op: &str) -> u64 {
    summary
        .ops
        .iter()
        .find(|o| o.id == op)
        .map(|o| o.n)
        .unwrap_or(0)
}

fn read_summary(path: &Path) -> AgentSummary {
    from_yaml(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `glafira-load run <project> --agents` с токенами `envs` (остальные токены агентов убраны).
async fn run_agents(
    project: PathBuf,
    envs: Vec<(&'static str, &'static str)>,
) -> std::process::Output {
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_glafira-load"));
        cmd.arg("run")
            .arg(&project)
            .args([
                "--profile",
                "short",
                "--env",
                "test",
                "--suite",
                SUITE,
                "--agents",
            ])
            .env_remove("GLAFIRA_LOAD_AGENT_TOKEN_A")
            .env_remove("GLAFIRA_LOAD_AGENT_TOKEN_B")
            .timeout(Duration::from_secs(90));
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_with_two_agents_collects_files_and_sums_n() {
    let stand = stand_stub(StatusCode::CREATED).await;
    let project = temp_project(&format!("http://{stand}"));
    write_profile(project.path(), "short", 0.05);
    write_prod_load(project.path());
    let a = start_agent().await;
    let b = start_agent().await;
    add_agents(project.path(), &a.url, &b.url);

    let output = run_agents(
        project.path().to_path_buf(),
        vec![
            ("GLAFIRA_LOAD_AGENT_TOKEN_A", TOKEN),
            ("GLAFIRA_LOAD_AGENT_TOKEN_B", TOKEN),
        ],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("status: completed"), "{stdout}");

    let runs: Vec<PathBuf> = std::fs::read_dir(project.path().join("runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let dir = &runs[0];
    let mut per_agent = BTreeMap::new();
    for id in ["a", "b"] {
        let agent_dir = dir.join("agents").join(id);
        assert!(
            agent_dir.join("summary.yaml").is_file(),
            "{}",
            agent_dir.display()
        );
        per_agent.insert(
            id,
            op_n(&read_summary(&agent_dir.join("summary.yaml")), CREATE),
        );
    }
    assert!(per_agent["a"] > 0 && per_agent["b"] > 0, "{per_agent:?}");
    let total = read_summary(&dir.join("summary.yaml"));
    assert_eq!(
        op_n(&total, CREATE),
        per_agent["a"] + per_agent["b"],
        "total n = sum of agents n: {per_agent:?}"
    );
    assert!(dir.join("run.yaml").is_file());

    // Без токена агента `b` — ошибка запуска (код 3), в stderr код и имя агента.
    let output = run_agents(
        project.path().to_path_buf(),
        vec![("GLAFIRA_LOAD_AGENT_TOKEN_A", TOKEN)],
    )
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(
        stderr.contains("TOKEN_MISSING") && stderr.contains("GLAFIRA_LOAD_AGENT_TOKEN_B"),
        "{stderr}"
    );
    assert!(stderr.contains("agent: b"), "{stderr}");
    let runs = std::fs::read_dir(project.path().join("runs"))
        .unwrap()
        .count();
    assert_eq!(runs, 1, "a run without a token creates no folder");
}
