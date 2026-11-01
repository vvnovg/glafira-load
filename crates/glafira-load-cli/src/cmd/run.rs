//! `glafira-load run`: локальный прогон по профилю, строка в секунду и итоговая таблица (спека §5.4).

use std::path::PathBuf;

use glafira_load_engine::agent_api::{Mode, RunStatus};
use glafira_load_engine::agent_link::ControllerEvent;
use glafira_load_engine::local::{LocalRun, run_local};
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

pub async fn run(args: RunArgs) -> Exit {
    let root = match project_root(&args.project) {
        Ok(r) => r,
        Err(e) => return startup_error(&e),
    };
    let suite = match resolve_suite(&root, args.suite.as_deref()) {
        Ok(s) => s,
        Err(e) => return startup_error(&e),
    };
    let (tx, mut rx) = mpsc::channel(1024);
    let cancel = CancellationToken::new();
    cancel_on_ctrl_c(cancel.clone());
    let req = LocalRun {
        root,
        suite,
        profile: args.profile,
        env: args.env,
        mode: Mode::Run,
        max_in_flight: args.max_in_flight,
        out: args.out,
    };
    let print = async {
        while let Some(event) = rx.recv().await {
            match event {
                ControllerEvent::Started { run_id, dir } => println!("run {run_id} -> {dir}"),
                ControllerEvent::Tick(t) => println!("{}", console::tick_line(&t)),
                ControllerEvent::Phase(p) => {
                    println!("t={}s phase {} target {:.0}%", p.t, p.index + 1, p.pct)
                }
                // примеры ошибок уходят в errors.jsonl; в консоли их считает строка секунды
                ControllerEvent::Error(_)
                | ControllerEvent::Agent { .. }
                | ControllerEvent::Done(_) => {}
            }
        }
    };
    let (result, ()) = tokio::join!(run_local(req, tx, cancel), print);
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
