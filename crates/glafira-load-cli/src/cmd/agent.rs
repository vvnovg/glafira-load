//! `glafira-load agent`: адрес, токен, папка работы, запуск сервера до Ctrl+C (спека §4.1).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use super::startup_error;
use crate::agent::runs::{AgentConfig, AgentState, secrets_from_vars};
use crate::agent::server;
use crate::exit::{Exit, OK};

pub const TOKEN_ENV: &str = "GLAFIRA_LOAD_AGENT_TOKEN";

pub struct AgentArgs {
    pub listen: String,
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
    pub work: Option<PathBuf>,
    pub keep_runs: usize,
    pub drain_ms: u64,
}

/// Токен: `--token`, затем `--token-file`, затем переменная окружения; пустые значения — как отсутствующие.
pub fn resolve_token(
    token: Option<String>,
    token_file: Option<&Path>,
    env: Option<String>,
) -> Result<String, String> {
    if let Some(t) = token.filter(|t| !t.trim().is_empty()) {
        return Ok(t.trim().to_string());
    }
    if let Some(file) = token_file {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read token file {}: {e}", file.display()))?;
        let t = text.trim();
        if t.is_empty() {
            return Err(format!("token file {} is empty", file.display()));
        }
        return Ok(t.to_string());
    }
    if let Some(t) = env.filter(|t| !t.trim().is_empty()) {
        return Ok(t.trim().to_string());
    }
    Err(format!(
        "agent token is required: pass --token, --token-file or set {TOKEN_ENV}"
    ))
}

pub fn default_work_dir() -> PathBuf {
    std::env::temp_dir().join("glafira-load-agent")
}

pub async fn agent(args: AgentArgs) -> Exit {
    let token = match resolve_token(
        args.token,
        args.token_file.as_deref(),
        std::env::var(TOKEN_ENV).ok(),
    ) {
        Ok(t) => t,
        Err(e) => return startup_error(&e),
    };
    let addr: SocketAddr = match args.listen.parse() {
        Ok(a) => a,
        Err(e) => return startup_error(&format!("--listen {}: {e}", args.listen)),
    };
    let vars = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
    let config = AgentConfig {
        work_dir: args.work.unwrap_or_else(default_work_dir),
        keep_runs: args.keep_runs,
        drain: Duration::from_millis(args.drain_ms),
        token,
        secrets_env: secrets_from_vars(vars),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let state = match AgentState::new(config) {
        Ok(s) => s,
        Err(e) => return startup_error(&format!("work dir: {e}")),
    };
    // Свой глобальный подписчик: `main` не зовёт `cmd::console_log` для `agent`, поэтому ставится
    // ровно он. Писатель — `LogSink`: stderr и `agent.log` текущего прогона (файл меняет `AgentState`).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(state.log_sink.clone())
        .try_init();
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => return startup_error(&format!("cannot listen on {addr}: {e}")),
    };
    let bound = listener.local_addr().unwrap_or(addr);
    tracing::info!(
        "glafira-load agent {} listening on {bound}, work dir {}, {} secret(s)",
        state.version,
        state.work_dir.display(),
        state.secrets_env.len()
    );
    // Ctrl+C: текущий прогон останавливается, чтобы его SSE закончился `done` и мягкое завершение
    // сервера не ждало конца профиля.
    let on_shutdown = state.clone();
    let shutdown = async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
        if let Some(run_id) = on_shutdown.busy() {
            let _ = on_shutdown.stop(&run_id).await;
        }
    };
    if let Err(e) = server::serve(listener, state, shutdown).await {
        return startup_error(&format!("server: {e}"));
    }
    Exit(OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_sources_in_order() {
        assert!(
            resolve_token(None, None, None)
                .unwrap_err()
                .contains(TOKEN_ENV)
        );
        assert!(resolve_token(Some("  ".into()), None, None).is_err());
        assert_eq!(
            resolve_token(Some(" a ".into()), None, Some("c".into())).unwrap(),
            "a"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("token");
        std::fs::write(&file, "b\n").unwrap();
        assert_eq!(
            resolve_token(None, Some(&file), Some("c".into())).unwrap(),
            "b"
        );
        std::fs::write(&file, "\n").unwrap();
        assert!(
            resolve_token(None, Some(&file), None)
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            resolve_token(None, Some(&dir.path().join("none")), None)
                .unwrap_err()
                .contains("cannot read")
        );
        assert_eq!(resolve_token(None, None, Some("c".into())).unwrap(), "c");
    }
}
