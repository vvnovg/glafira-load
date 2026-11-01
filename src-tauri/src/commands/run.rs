//! Команды прогона: запуск с каналом событий, остановка, список, карточка, подтверждение
//! закрытия окна при идущем прогоне (спека §7.1–§7.2).

use std::sync::atomic::{AtomicBool, Ordering};

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

/// Пользователь подтвердил закрытие при идущем прогоне: агенты стенда доработают сами
/// (спека §6.3), локальный прогон останавливается вместе с процессом и не успевает дописать файлы.
#[tauri::command]
pub fn confirm_close(app: AppHandle, state: State<'_, AppState>) {
    let stopped = state.runs.stop_all();
    tracing::info!(stopped, "closing on user confirmation");
    app.exit(0);
}
