//! Команды прогона: запуск с каналом событий, остановка, список, карточка, подтверждение
//! закрытия окна при идущем прогоне (спека §7.1–§7.2).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use glafira_load_engine::agent_api::Mode;
use glafira_load_engine::controller::{ControllerEvent, RunResult};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::blocking;
use crate::error::AppResult;
use crate::project::manifest::ProjectDir;
use crate::runs::{self, RunDetails, RunListEntry, StartRequest};
use crate::state::AppState;

/// Прогон по профилю на агентах стенда (стенд без агентов — встроенный агент) или смок локально;
/// события — в канал по мере поступления, ответ — итог прогона. Второй прогон в проекте —
/// `RUN_BUSY`. Параметры — по контракту IPC (спека §7.2), отсюда их число.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    path: String,
    suite: String,
    profile: String,
    env: String,
    mode: Mode,
    max_in_flight: Option<u32>,
    on_event: Channel<ControllerEvent>,
) -> AppResult<RunResult> {
    let st = state.inner().clone();
    let req = StartRequest {
        suite,
        profile,
        env,
        mode,
        max_in_flight,
    };
    // Webview перезагрузили — канал закрыт, а прогон идёт дальше: предупреждение один раз.
    let warned = AtomicBool::new(false);
    runs::start(&st, path, req, move |ev| {
        if let Err(e) = on_event.send(ev)
            && !warned.swap(true, Ordering::Relaxed)
        {
            tracing::warn!("run events are not delivered: {e}");
        }
    })
    .await
}

/// Мягкая остановка прогона проекта (прогон в проекте один).
#[tauri::command]
pub async fn run_stop(state: State<'_, AppState>, path: String) -> AppResult<()> {
    runs::stop(state.inner(), path).await
}

/// Папки `runs/` по дате убыв.; незавершённые достраиваются.
#[tauri::command]
pub async fn run_list(state: State<'_, AppState>, path: String) -> AppResult<Vec<RunListEntry>> {
    runs::list(state.inner(), path).await
}

/// Карточка прогона: `dir` — `runs/<папка>` или абсолютный путь внутри `runs/`.
#[tauri::command]
pub async fn run_get(path: String, dir: String) -> AppResult<RunDetails> {
    blocking(move || runs::details(&ProjectDir::open(&path)?, &dir)).await
}

/// Сколько `confirm_close(stop = true)` ждёт, пока остановленные прогоны допишут папки.
pub const CLOSE_STOP_WAIT: Duration = Duration::from_secs(30);

/// Пользователь подтвердил закрытие при идущем прогоне (спека §7.1). `stop = true` — остановить
/// все прогоны и дождаться, пока контроллеры остановят агентов и допишут папки (не дольше
/// `CLOSE_STOP_WAIT`), затем выйти. `stop = false` — ничего не отменять: агенты стенда доработают
/// сами, папка прогона достроится при следующем открытии проекта (спека §6.3); встроенный агент
/// (стенд без агентов, смок) завершается вместе с процессом.
#[tauri::command]
pub async fn confirm_close(
    app: AppHandle,
    state: State<'_, AppState>,
    stop: bool,
) -> AppResult<()> {
    if stop {
        let (stopped, idle) = runs::stop_all(state.inner(), CLOSE_STOP_WAIT).await;
        if !idle {
            tracing::warn!(stopped, "runs did not finish in time, closing anyway");
        }
        tracing::info!(stopped, idle, "closing after stopping the runs");
    } else {
        tracing::info!("closing on user confirmation, runs are left to the agents");
    }
    // Решение принято: обработчик `RunEvent::Exit` в `lib.rs` прогоны больше не трогает
    // (при `stop = false` они нарочно оставлены агентам).
    state.runs.confirm_exit();
    // `ExitRequested { code: Some(0) }`: обработчик в `lib.rs` такой выход не задерживает.
    app.exit(0);
    Ok(())
}
