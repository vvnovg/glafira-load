//! Бинарник `glafira-load`: разбор аргументов и выбор команды (спека §5.4, §4.1).

use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use glafira_load_cli::cmd;
use glafira_load_cli::cmd::agent::AgentArgs;
use glafira_load_cli::cmd::run::RunArgs;
use glafira_load_cli::cmd::smoke::SmokeArgs;
use glafira_load_cli::exit::{Exit, OK, STARTUP};

#[derive(Parser)]
#[command(
    name = "glafira-load",
    version,
    about = "Glafira for Load Testing: check and smoke suites, run load profiles, serve as a load agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check one suite or all suites of the project without network (exit 4 on errors)
    Check {
        /// Project folder
        project: PathBuf,
        /// Suite name; all suites when omitted
        #[arg(long)]
        suite: Option<String>,
    },
    /// Run every scenario of a suite once against an environment (exit 4 on failed steps)
    Smoke {
        /// Project folder
        project: PathBuf,
        #[arg(long)]
        suite: String,
        /// Environment id from the project manifest
        #[arg(long)]
        env: String,
        /// Run folder; default `runs/smoke-<timestamp>` inside the project
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run a load profile locally or on the environment's agents (exit 2 when stopped with Ctrl+C)
    Run {
        /// Project folder
        project: PathBuf,
        #[arg(long)]
        profile: String,
        /// Environment id from the project manifest
        #[arg(long)]
        env: String,
        /// Suite name; the only suite of the project when omitted
        #[arg(long)]
        suite: Option<String>,
        /// Run on the agents of the environment (tokens in GLAFIRA_LOAD_AGENT_TOKEN_<ID>) instead of this process
        #[arg(long)]
        agents: bool,
        /// Cap of concurrent iterations; overrides `defaults.max_in_flight`
        #[arg(long = "max-in-flight")]
        max_in_flight: Option<u32>,
        /// Run folder; default `runs/<timestamp>` inside the project
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Serve as a load agent for a controller
    Agent {
        /// Address to listen on, e.g. 0.0.0.0:7420
        #[arg(long)]
        listen: String,
        /// Bearer token; alternatively --token-file or GLAFIRA_LOAD_AGENT_TOKEN
        #[arg(long)]
        token: Option<String>,
        /// File with the bearer token
        #[arg(long = "token-file")]
        token_file: Option<PathBuf>,
        /// Folder for run bundles and files; default <temp>/glafira-load-agent
        #[arg(long)]
        work: Option<PathBuf>,
        /// How many finished runs to keep on disk
        #[arg(long = "keep-runs", default_value_t = 10)]
        keep_runs: usize,
        /// How long in-flight iterations may finish after stop
        #[arg(long = "drain-ms", default_value_t = 5000)]
        drain_ms: u64,
    },
}

#[tokio::main]
async fn main() -> Exit {
    // clap завершал бы процесс кодом 2, а он занят значением «остановлено»: ошибки разбора → 3, help/version → 0
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                Exit(OK)
            } else {
                Exit(STARTUP)
            };
        }
    };
    match cli.command {
        Command::Check { project, suite } => {
            cmd::console_log();
            cmd::check::check(project, suite).await
        }
        Command::Smoke {
            project,
            suite,
            env,
            out,
        } => {
            cmd::console_log();
            cmd::smoke::smoke(SmokeArgs {
                project,
                suite,
                env,
                out,
            })
            .await
        }
        Command::Run {
            project,
            profile,
            env,
            suite,
            agents,
            max_in_flight,
            out,
        } => {
            cmd::console_log();
            cmd::run::run(RunArgs {
                project,
                profile,
                env,
                suite,
                agents,
                max_in_flight,
                out,
            })
            .await
        }
        // журнал агента ставит сама команда (stderr + `agent.log` прогона), `console_log` не нужен
        Command::Agent {
            listen,
            token,
            token_file,
            work,
            keep_runs,
            drain_ms,
        } => {
            cmd::agent::agent(AgentArgs {
                listen,
                token,
                token_file,
                work,
                keep_runs,
                drain_ms,
            })
            .await
        }
    }
}
