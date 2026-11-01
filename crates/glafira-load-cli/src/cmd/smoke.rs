//! `glafira-load smoke`: каждый сценарий один раз, отчёт `smoke.yaml` (спека §5.4, §6.2).

use std::path::PathBuf;

use glafira_load_engine::agent_api::{Mode, RunStatus};
use glafira_load_engine::agent_link::ControllerEvent;
use glafira_load_engine::local::{LocalRun, run_local};
use glafira_load_engine::summary::SmokeReport;
use glafira_load_model::suite;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{cancel_on_ctrl_c, engine_error, startup_error};
use crate::console;
use crate::exit::{Exit, FINDINGS, OK, STARTUP, STOPPED};
use crate::project::{project_root, read_suite_text, suite_file};

pub struct SmokeArgs {
    pub project: PathBuf,
    pub suite: String,
    pub env: String,
    pub out: Option<PathBuf>,
}

pub async fn smoke(args: SmokeArgs) -> Exit {
    let root = match project_root(&args.project) {
        Ok(r) => r,
        Err(e) => return startup_error(&e),
    };
    let text = match read_suite_text(&root, &args.suite) {
        Ok(t) => t,
        Err(e) => return startup_error(&e),
    };
    // отчёт строится по сценариям сьюта: разбор нужен до прогона
    let parsed = match suite::parse(&text) {
        Ok(s) => s,
        Err(e) => {
            let line = e.line.map(|l| format!(":{l}")).unwrap_or_default();
            println!("error    {}{line}  {}", suite_file(&args.suite), e.message);
            return Exit(FINDINGS);
        }
    };
    let (tx, mut rx) = mpsc::channel(1024);
    let cancel = CancellationToken::new();
    cancel_on_ctrl_c(cancel.clone());
    // профиль пуст: в смоке цели не считаются, и профиль проекта смоку не нужен
    let req = LocalRun {
        root,
        suite: args.suite,
        profile: String::new(),
        env: args.env,
        mode: Mode::Smoke,
        max_in_flight: None,
        out: args.out,
    };
    let collect = async {
        let mut errors = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ControllerEvent::Started { run_id, dir } => println!("smoke {run_id} -> {dir}"),
                ControllerEvent::Error(e) => errors.push(e),
                _ => {}
            }
        }
        errors
    };
    let (result, errors) = tokio::join!(run_local(req, tx, cancel), collect);
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            engine_error(&e);
            return if e.code == "SUITE_INVALID" {
                Exit(FINDINGS)
            } else {
                Exit(STARTUP)
            };
        }
    };
    // `smoke.yaml` в папке прогона уже записал `run_local`; для консоли отчёт строится из тех же примеров ошибок.
    let report = SmokeReport::from_errors(&parsed, &errors);
    print!("{}", console::smoke_table(&report));
    println!(
        "status: {}  folder: {}",
        console::status_label(&result.status),
        result.dir
    );
    smoke_exit(&result.status, report.failed_steps())
}

/// 2 — остановлен, 3 — прогон упал (например, не создался исполнитель: примеров ошибок может
/// не быть, и все шаги выглядели бы `ok`), 4 — есть упавшие шаги, иначе 0.
pub fn smoke_exit(status: &RunStatus, failed_steps: usize) -> Exit {
    match status {
        RunStatus::Stopped => Exit(STOPPED),
        RunStatus::Failed => Exit(STARTUP),
        _ if failed_steps > 0 => Exit(FINDINGS),
        _ => Exit(OK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_run_is_a_startup_error_even_without_failed_steps() {
        assert_eq!(smoke_exit(&RunStatus::Completed, 0), Exit(OK));
        assert_eq!(smoke_exit(&RunStatus::Completed, 2), Exit(FINDINGS));
        assert_eq!(smoke_exit(&RunStatus::Stopped, 1), Exit(STOPPED));
        assert_eq!(smoke_exit(&RunStatus::Failed, 0), Exit(STARTUP));
    }
}
