//! Команды CLI. Каждая возвращает `Exit`; текст ошибок запуска — в stderr.

pub mod agent;
pub mod check;
pub mod run;
pub mod smoke;

use glafira_load_engine::agent_link::EngineError;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::exit::{Exit, STARTUP};

/// Ctrl+C → отмена: локальный `run` равен `stop`, текущие итерации дорабатывают (спека §5.4).
pub fn cancel_on_ctrl_c(cancel: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("stopping: waiting for in-flight iterations");
            cancel.cancel();
        }
    });
}

/// Журнал движка (например, испорченный файл данных у `check`) — в stderr без цветов; уровень —
/// `RUST_LOG`, по умолчанию `warn`. Вызывается из `main` для `check`, `smoke` и `run`: у агента
/// (Task 10) свой журнал, глобальный подписчик ставится один раз.
pub fn console_log() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}

pub fn startup_error(message: &str) -> Exit {
    eprintln!("error: {message}");
    Exit(STARTUP)
}

/// Ошибка движка в stderr: `error CODE: message`, затем `names` по строке (находки `SUITE_INVALID`,
/// имена отсутствующих секретов) и виновный агент, если он известен.
pub fn engine_error(e: &EngineError) {
    eprintln!("error {}: {}", e.code, e.message);
    for name in &e.names {
        eprintln!("  {name}");
    }
    if let Some(agent) = &e.agent {
        eprintln!("  agent: {agent}");
    }
}
