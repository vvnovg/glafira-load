//! Команды разбора: автопоиск контрактов, разбор сервисов с прогрессом, отмена.

use glafira_load_model::project::ContractRef;
use tauri::State;
use tauri::ipc::Channel;

use super::blocking;
use crate::error::AppResult;
use crate::git::version::GitVersion;
use crate::project::manifest::ProjectDir;
use crate::scan::discover::discover;
use crate::scan::run::{ScanProgress, ScanRegistry, ScanReport, scan_service_blocking};
use crate::state::AppState;

#[tauri::command]
pub async fn service_discover_contracts(
    path: String,
    service_id: String,
) -> AppResult<Vec<ContractRef>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let m = d.manifest()?;
        Ok(discover(&d.service_root(&m, &service_id)?))
    })
    .await
}

/// Канонический корень проекта — ключ реестра разборов (один проект, разные написания пути).
fn project_key(path: &str) -> AppResult<String> {
    Ok(ProjectDir::open(path)?
        .root()
        .to_string_lossy()
        .into_owned())
}

/// Разбор одного сервиса или всех (`service_id = None`), по очереди; прогресс — в канал.
/// Отмена останавливает и текущий сервис, и следующие.
#[tauri::command]
pub async fn service_scan(
    state: State<'_, AppState>,
    path: String,
    service_id: Option<String>,
    on_progress: Channel<ScanProgress>,
) -> AppResult<Vec<ScanReport>> {
    let st = state.inner().clone();
    let (root, ids) = blocking(move || {
        let d = ProjectDir::open(&path)?;
        let ids = match service_id {
            Some(id) => vec![id],
            None => d
                .manifest()?
                .services
                .iter()
                .map(|s| s.id.clone())
                .collect(),
        };
        Ok((d.root().to_string_lossy().into_owned(), ids))
    })
    .await?;
    let mut reports = Vec::new();
    for id in ids {
        // Снимается при выходе из итерации, в том числе по ошибке.
        let running = st.scans.guard(ScanRegistry::key(&root, &id))?;
        let (r, sid) = (root.clone(), id.clone());
        let clone = blocking(move || {
            let d = ProjectDir::open(&r)?;
            let m = d.manifest()?;
            Ok(d.service_root(&m, &sid).ok())
        })
        .await?;
        // Нет папки клона — разбор сам вернёт предупреждение и не тронет реестр.
        let git = match clone {
            Some(c) => crate::git::version::describe(&c).await,
            None => GitVersion::unversioned("service folder not found"),
        };
        let (r, ch, cancel) = (root.clone(), on_progress.clone(), running.flag());
        let report = blocking(move || {
            let d = ProjectDir::open(&r)?;
            let progress = move |pr: ScanProgress| {
                let _ = ch.send(pr);
            };
            scan_service_blocking(&d, &id, git, &progress, &cancel)
        })
        .await?;
        drop(running);
        let stop = report.cancelled;
        reports.push(report);
        if stop {
            break;
        }
    }
    Ok(reports)
}

/// Ставит флаг отмены; разбора нет — ничего не делает.
#[tauri::command]
pub async fn scan_cancel(
    state: State<'_, AppState>,
    path: String,
    service_id: Option<String>,
) -> AppResult<()> {
    let st = state.inner().clone();
    blocking(move || {
        // Проекта уже нет на диске — ключ по пути как есть (так он и не найдётся).
        let root = project_key(&path).unwrap_or(path);
        match service_id {
            Some(id) => {
                st.scans.cancel(&ScanRegistry::key(&root, &id));
            }
            None => {
                st.scans.cancel_all(&root);
            }
        }
        Ok(())
    })
    .await
}
