//! `glafira-load run`: прогон по профилю локально или на агентах стенда (спека §4.5, §5.4):
//! строка в секунду, итоговая таблица, коды возврата 0/2/3.

use std::path::PathBuf;
use std::sync::Arc;

use glafira_load_engine::agent_api::{Mode, RunStatus};
use glafira_load_engine::agent_link::{
    AgentLink, ControllerEvent, EngineError, HttpAgent, RunResult,
};
use glafira_load_engine::controller::plan::{PlanArgs, load_plan};
use glafira_load_engine::controller::{CONTROLLER_VERSION, Controller};
use glafira_load_engine::local::{LocalRun, run_local};
use glafira_load_model::project::Environment;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{cancel_on_ctrl_c, engine_error, startup_error};
use crate::console;
use crate::exit::{Exit, OK, STARTUP, STOPPED};
use crate::project::{project_root, resolve_suite};

pub struct RunArgs {
    pub project: PathBuf,
    pub profile: String,
    pub env: String,
    pub suite: Option<String>,
    /// `--agents`: прогон на агентах стенда из манифеста вместо своего процесса.
    pub agents: bool,
    pub max_in_flight: Option<u32>,
    pub out: Option<PathBuf>,
}

/// 0 — завершён (в том числе `incomplete`), 2 — остановлен, 3 — упал.
pub fn exit_for(status: &RunStatus) -> Exit {
    match status {
        RunStatus::Completed | RunStatus::Incomplete => Exit(OK),
        RunStatus::Stopped => Exit(STOPPED),
        RunStatus::Failed => Exit(STARTUP),
    }
}

/// Имя переменной окружения с токеном агента: `GLAFIRA_LOAD_AGENT_TOKEN_<ID>`, ID в верхнем
/// регистре, `-` → `_`.
pub fn token_var(id: &str) -> String {
    format!(
        "GLAFIRA_LOAD_AGENT_TOKEN_{}",
        id.to_ascii_uppercase().replace('-', "_")
    )
}

/// `HttpAgent` по агентам стенда с токенами из `lookup` (имя переменной → значение). Стенд без
/// агентов — `AGENT_UNREACHABLE`; нет токена (или он пустой) — `TOKEN_MISSING` с агентом и
/// именем переменной.
pub fn agents_from(
    env: &Environment,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Vec<Arc<dyn AgentLink>>, EngineError> {
    if env.agents.is_empty() {
        return Err(EngineError::new(
            "AGENT_UNREACHABLE",
            format!(
                "environment `{}` has no agents; add them to the manifest",
                env.id
            ),
        ));
    }
    let mut out: Vec<Arc<dyn AgentLink>> = Vec::new();
    for a in &env.agents {
        let var = token_var(&a.id);
        let token = lookup(&var)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                EngineError::new(
                    "TOKEN_MISSING",
                    format!("no token for agent {}: set {var}", a.id),
                )
                .with_names(vec![var.clone()])
                .with_agent(&a.id)
            })?;
        out.push(Arc::new(HttpAgent::new(
            &a.id,
            &a.url,
            &token,
            CONTROLLER_VERSION,
        )));
    }
    Ok(out)
}

/// Агенты стенда с токенами из окружения процесса.
pub fn agents_of(env: &Environment) -> Result<Vec<Arc<dyn AgentLink>>, EngineError> {
    agents_from(env, |var| std::env::var(var).ok())
}

/// Прогон на агентах: план из проекта (`check` сьюта, цели профиля), `HttpAgent` по стенду,
/// общий контроллер (тот же, что у окна прогона).
pub async fn run_agents(
    root: PathBuf,
    args: &RunArgs,
    events: mpsc::Sender<ControllerEvent>,
    cancel: CancellationToken,
) -> Result<RunResult, EngineError> {
    let plan_args = PlanArgs {
        root,
        suite: args.suite.clone(),
        profile: args.profile.clone(),
        env: args.env.clone(),
        mode: Mode::Run,
        max_in_flight: args.max_in_flight,
        out_dir: args.out.clone(),
    };
    // чтение проекта и `check` сьюта — файловый ввод-вывод, вне потоков рантайма
    let plan = tokio::task::spawn_blocking(move || load_plan(plan_args))
        .await
        .map_err(|e| EngineError::io(format!("reading the project failed: {e}")))??;
    let agents = agents_of(&plan.env)?;
    Controller::run(plan, agents, events, cancel).await
}

/// Консоль: старт, строка в секунду, фазы; `show_agents` — ещё и смена состояний агентов (у
/// локального прогона агент один и встроенный, его состояния не печатаются).
async fn print_events(mut rx: mpsc::Receiver<ControllerEvent>, show_agents: bool) {
    while let Some(event) = rx.recv().await {
        match event {
            ControllerEvent::Started { run_id, dir } => println!("run {run_id} -> {dir}"),
            ControllerEvent::Tick(t) => println!("{}", console::tick_line(&t)),
            ControllerEvent::Phase(p) => {
                println!("t={}s phase {} target {:.0}%", p.t, p.index + 1, p.pct)
            }
            ControllerEvent::Agent { id, state } if show_agents => {
                println!("agent {id}: {}", format!("{state:?}").to_lowercase())
            }
            // примеры ошибок уходят в errors.jsonl; в консоли их считает строка секунды
            ControllerEvent::Agent { .. }
            | ControllerEvent::Error(_)
            | ControllerEvent::Done(_) => {}
        }
    }
}

pub async fn run(args: RunArgs) -> Exit {
    let root = match project_root(&args.project) {
        Ok(r) => r,
        Err(e) => return startup_error(&e),
    };
    let (tx, rx) = mpsc::channel(1024);
    let cancel = CancellationToken::new();
    cancel_on_ctrl_c(cancel.clone());
    let result = if args.agents {
        let (result, ()) =
            tokio::join!(run_agents(root, &args, tx, cancel), print_events(rx, true));
        result
    } else {
        let suite = match resolve_suite(&root, args.suite.as_deref()) {
            Ok(s) => s,
            Err(e) => return startup_error(&e),
        };
        let req = LocalRun {
            root,
            suite,
            profile: args.profile.clone(),
            env: args.env.clone(),
            mode: Mode::Run,
            max_in_flight: args.max_in_flight,
            out: args.out.clone(),
        };
        let (result, ()) = tokio::join!(run_local(req, tx, cancel), print_events(rx, false));
        result
    };
    match result {
        Ok(r) => {
            println!();
            print!("{}", console::summary_table(&r.summary));
            println!(
                "status: {}  folder: {}",
                console::status_label(&r.status),
                r.dir
            );
            exit_for(&r.status)
        }
        Err(e) => {
            engine_error(&e);
            Exit(STARTUP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::project::{Agent, EnvKind};

    fn env(agents: Vec<Agent>) -> Environment {
        Environment {
            id: "test".into(),
            kind: EnvKind::Test,
            http: None,
            kafka: None,
            grpc: None,
            sql: None,
            schema_registry: None,
            agents,
            extra: Default::default(),
        }
    }

    fn agent(id: &str, port: u16, weight: u32) -> Agent {
        Agent {
            id: id.into(),
            url: format!("http://127.0.0.1:{port}"),
            weight,
        }
    }

    #[test]
    fn token_var_uppercases_and_replaces_dashes() {
        assert_eq!(token_var("gen-1"), "GLAFIRA_LOAD_AGENT_TOKEN_GEN_1");
        assert_eq!(token_var("a"), "GLAFIRA_LOAD_AGENT_TOKEN_A");
    }

    #[test]
    fn agents_from_requires_a_token_per_agent() {
        let e = env(vec![agent("gen-1", 1, 1), agent("gen-2", 2, 2)]);
        let both =
            |v: &str| (v.ends_with("_GEN_1") || v.ends_with("_GEN_2")).then(|| "t".to_string());
        let links = agents_from(&e, both).unwrap();
        assert_eq!(
            links.iter().map(|l| l.id().to_string()).collect::<Vec<_>>(),
            vec!["gen-1", "gen-2"]
        );

        let first_only = |v: &str| v.ends_with("_GEN_1").then(|| "t".to_string());
        let err = agents_from(&e, first_only).err().expect("error");
        assert_eq!(
            (err.code.as_str(), err.agent.as_deref()),
            ("TOKEN_MISSING", Some("gen-2"))
        );
        assert_eq!(
            err.names,
            vec!["GLAFIRA_LOAD_AGENT_TOKEN_GEN_2".to_string()]
        );

        let blank = agents_from(&e, |_| Some("  ".into())).err().expect("error");
        assert_eq!(blank.code, "TOKEN_MISSING", "пустой токен не считается");
        assert_eq!(blank.agent.as_deref(), Some("gen-1"));
    }

    #[test]
    fn environment_without_agents_is_a_startup_error() {
        let err = agents_from(&env(vec![]), |_| Some("t".into()))
            .err()
            .expect("error");
        assert_eq!(err.code, "AGENT_UNREACHABLE");
        assert!(err.message.contains("test"), "{}", err.message);
    }

    #[test]
    fn exit_codes_follow_the_status() {
        assert_eq!(exit_for(&RunStatus::Completed), Exit(OK));
        assert_eq!(exit_for(&RunStatus::Incomplete), Exit(OK));
        assert_eq!(exit_for(&RunStatus::Stopped), Exit(STOPPED));
        assert_eq!(exit_for(&RunStatus::Failed), Exit(STARTUP));
    }
}
